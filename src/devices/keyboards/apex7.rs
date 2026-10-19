//! Apex 7 and the boards of its legacy "macho" platform, including the `[EXPERIMENTAL]` on-board
//! profile read.
//!
//! Frame construction, reply parsing, payload geometry and decoding live in
//! [`crate::devices::keyboards::apex_legacy_profile`]; every wire fact the read depends on is stated
//! in that module's doc, once. This module is the transfer machinery: it sends the firmware poll and the
//! pull frames, the pacing read that follows each pull and the payload reads, collects the replies and
//! assembles a
//! [`ProfileImage`](crate::devices::keyboards::apex_legacy_profile::ProfileImage).
//!
//! The transfers run on the handle this device already owns - the collection the device manager opened,
//! held open for the whole read, which is what the packet methods below use. `usbhid` polls a collection's
//! interrupt endpoint only while a descriptor on that node is open, so the interrupt reports the read
//! consumes (the `0x90` answer and the packet after each pull) can only arrive on a handle that stays open
//! for the whole read: measured on an Apex 7 in the Linux run capture,
//! where a reader that opened the node per report collected nothing
//! (`docs/development/apex-7-onboard-profile-read.md`, "Input reports: the node must be open when the
//! report arrives"). Apex 7 wired works through hidapi: the same `SET_REPORT wValue 0x0200` and
//! `wValue 0x0300` shapes the reference capture shows, on the collection hidapi opened.
//!
//! `[EXPERIMENTAL]`: the read completes on hardware - a full profile slot has been read from an Apex 7
//! under Linux (reported 2026-10-18; model, firmware and the decoded bytes are not recorded in this repo
//! yet). What is still unproven is the decode: slot layout, checksum words and the per-slot compare
//! against GG (`ONBOARD.md` steps 3-5). This module never writes a profile.

use std::ops::{Deref, DerefMut};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::apex_legacy_profile::{
    ChunkRecord, DEVICE_RESOURCE_BUFFER_LEN, GG_GAP_MS, GG_INPUT_READ_BYTES, LegacyApexOnboard, MAX_READ_LEN,
    PAYLOAD_READ_LEN, PROFILE_FILE_BYTES, PULLS_PER_FILE, ProfileImage, READ_REPLY_FRAMING_LEN, READ_REPORT_LEN,
    READS_PER_PULL, ReadReply, SCHEMA_B, hex_of, parse_firmware_version, parse_read_reply_at, profile_file_id,
    reply_mismatch, schema_of, validate_profile,
};
use super::onboard::{OnboardDevice, OnboardProfileCodec};
use super::{GenericKeyboard, Keyboard};
use crate::devices::hid_reports::{
    FirmwareSource, HidCommand, ONBOARD_FEATURE_REPORT_SIZE, ONBOARD_OUTPUT_REPORT_SIZE, OnboardFirmwareVersionCommand,
    OnboardPullCommand, OnboardReadCommand,
};
use crate::devices::key_mapping::{KeyId, KeyMapping};
use crate::devices::product_ids;
use crate::devices::zone_mapping::{ZoneEffect, ZoneMapping};
use crate::devices::{Device, DeviceType, device_name_from_product_id, known_product_id};
use crate::error::{Error, Result};
use crate::rgb::{Color, PerKeyEffect};

/// Report number to ask for when reading a payload read back: **0**, which is the
/// `GET_REPORT wValue 0x0300 wIndex 1` control read the reference capture shows. The kernel writes the
/// device's own report-ID byte into byte 0 of the buffer, so the reply's leading `03` arrives there
/// whatever we ask for. Earlier builds asked `0x83` (the command byte as a report number), which
/// produced `wValue 0x0383` in the Linux capture - a report number the keyboard never
/// declared - and that run answered nothing.
/// `[EXPERIMENTAL]`
const READ_REPLY_ID: u8 = 0x00;
/// Buffer size to ask the transport for when reading a payload read back: [`READ_REPORT_LEN`] (642), the
/// capture's `GET_REPORT wValue 0x0300 wIndex 1 wLength 642`. The read side forwards this number to the
/// device as `wLength` verbatim, so 643 is the ask that never completes (`Connection timed out`, measured
/// 2026-10-15). The device's report-ID byte lands in buffer[0], so a full answer is `[report ID][641 bytes]`.
/// `[EXPERIMENTAL]` The 642-byte ask answered on hardware (Apex 7, 2026-10-16 and 2026-10-18).
const READ_REPLY_BUFFER_LEN: usize = READ_REPORT_LEN + 1;
/// Poll acknowledges and status reports the reply collector skips.
const ACK_REPORT_IDS: [u8; 2] = [0x0B, 0x12];
/// Bytes to read for the pacing packet that follows a pull. The pull has no answer of its own: the device
/// keeps streaming its ordinary input reports on the interrupt endpoint (report `0x01`, all-zero on an idle
/// keyboard, 1.7-3.7 ms after the pull in the capture) and GG consumes one packet per chunk as pacing, taking
/// no bytes from it (GG's `read-chunk HIDIO 65`: report-ID slot plus 64 data bytes, see
/// [`GG_INPUT_READ_BYTES`], which is what this asks hidapi for).
const PULL_ACK_LEN: usize = GG_INPUT_READ_BYTES;
/// Polls per reply collection round, in milliseconds.
const REPLY_POLL_MS: u64 = 50;
/// Reply budget for one payload read, in milliseconds: the feature-report poll loop runs until this
/// expires, then the input-endpoint fallback starts. A fixed budget, not a caller-set deadline (the
/// `--timeout` knob is gone).
const REPLY_BUDGET_MS: u64 = 2_000;
/// Input-report polls allowed per reply before giving up.
const REPLY_INPUT_ATTEMPTS: u32 = 20;
/// Reads allowed per pull before giving up (the device streams input reports, so one answer is expected).
const PULL_ACK_ATTEMPTS: u32 = 4;
/// Bytes to read for the firmware poll's answer: GG's input read size, [`GG_INPUT_READ_BYTES`] (a
/// report-ID slot plus 64 data bytes), which the answer fits (`0x6E` with the ASCII version).
const FIRMWARE_REPLY_LEN: usize = GG_INPUT_READ_BYTES;
/// Reads allowed for the firmware poll's answer. One: the version is informational, so the read must not
/// spend time waiting on a keyboard that answers nothing.
const FIRMWARE_REPLY_ATTEMPTS: u32 = 1;
/// Size of the schema probe pull, in bytes: the 2-byte probe on the profile file of slot 0 the reference
/// capture shows in front of the file read (`[85][00][0201][0][2]`, answered `03 0b 00`).
const PROBE_PULL_LEN: u16 = 2;
/// Largest input report a profile collection can return (Apex 7 profile collections report 1024).
const INPUT_REPORT_MAX_LEN: usize = 1024;

/// A payload read reply arrives as `[report ID][641 bytes]`, and a header-echoing reply spends
/// [`READ_REPLY_FRAMING_LEN`] of those bytes on its header: that is why a read frame never asks for more
/// than [`MAX_READ_LEN`] (636) - a larger ask could not be carried by any reply.
const _: () = assert!(MAX_READ_LEN + READ_REPLY_FRAMING_LEN < READ_REPLY_BUFFER_LEN);

// The transfers the profile protocol needs are the packet I/O of the keyboard this device owns:
// `GenericKeyboard::write_profile_packet` for output frames (not `send_report`: its write goes through
// `write_padded_report`, whose global hid optimizer drops a report byte-identical to one sent in the last 50 ms
// - see [`Apex7::write_output_command`]), the keyboard's `send_feature` for the feature-report read request, and
// `read_profile_feature_packet` / `read_profile_input_packet` for the two reply shapes - all on the handle the
// device manager opened, open for the whole read.
//
// `[EXPERIMENTAL]`: on hardware the firmware poll, the pull, the pacing read and the payload read-back all
// complete on an Apex 7 (2026-10-17/18), and the read request is answered by the `GET_REPORT` control-IN reply
// the capture shows. Measured 2026-10-19: GG's opening chain and its closing sequence are **not needed** - the
// read collects a profile slot without either, so this crate sends neither.

/// Apex 7 and the boards of its platform that take the on-board profile protocol.
///
/// Only `0x1612` (Apex 7) appears in the reference capture; `0x1614` (Apex Pro TKL, 2016), `0x1616`
/// (`apex_150`) and `0x1618` (`apex_7_tkl`) are the same legacy platform in GG's firmware registry and
/// are unverified here.
#[must_use]
pub fn onboard_family(product_id: u16) -> bool {
    matches!(
        product_id,
        product_ids::APEX_7 | product_ids::APEX_PRO_TKL | product_ids::APEX_150 | product_ids::APEX_7_TKL
    )
}

/// Apex 7 keyboard device: standard lighting control through [`GenericKeyboard`], plus the
/// `[EXPERIMENTAL]` on-board profile read on its own handle.
///
/// Discovery builds this device for every [`onboard_family`] product ID: the device class is part of the
/// device tree, and the read path is part of the core build.
pub struct Apex7 {
    inner: GenericKeyboard,
}

impl Apex7 {
    /// Product ID of the Apex 7 the reference capture was taken on.
    pub const PRODUCT_ID: u16 = product_ids::APEX_7;

    /// Create from a generic keyboard.
    pub fn new(keyboard: GenericKeyboard) -> Self {
        Self { inner: keyboard }
    }

    /// Whether this device's product ID is on the platform the read was reverse-engineered from.
    #[must_use]
    pub fn supports_onboard(&self) -> bool {
        onboard_family(self.inner.info().product_id)
    }

    /// Read one on-board profile slot (slots `0..=4`; everything else about the read is fixed geometry).
    /// GG names the same slots `Config 1` .. `Config 5`, and `ssgg onboard read --profile N` passes
    /// `N - 1` here.
    ///
    /// The read plan polls the device and sleeps for GG's inter-frame pacing, so it runs on a scoped
    /// thread and blocks this task's thread until the read finishes (about 2 s for a slot on the
    /// reference capture's cadence). Nothing in it changes the keyboard's state.
    ///
    /// # Errors
    ///
    /// Returns an error when the slot is out of range, the collection rejects a frame, it answers a schema
    /// this read cannot decode, or it never answers a payload read.
    pub async fn read_profile(&self, profile: u8) -> Result<ProfileImage> {
        // The read plan blocks on reply polls and GG's inter-frame pacing, so it runs on its own thread. A
        // scoped thread borrows this device and the scope is joined before this returns, which is the whole
        // reason to use `std::thread::scope`: the borrow is checked, not asserted.
        std::thread::scope(|scope| {
            scope
                .spawn(|| self.read_image(profile))
                .join()
                .map_err(|_| Error::Other("onboard read worker panicked".to_string()))?
        })
    }

    /// Runs one read: the firmware poll, the schema probe, then [`PULLS_PER_FILE`] pulls, each with its
    /// pacing read and payload reads.
    ///
    /// The read **changes nothing on the keyboard**: GG's opening chain and closing sequence are not sent -
    /// measured 2026-10-19, the read works without them (see
    /// `docs/development/apex-7-onboard-profile-read.md` §Sequences the read does not need).
    fn read_image(&self, profile: u8) -> Result<ProfileImage> {
        validate_profile(profile)?;
        let fs_id = profile_file_id(profile);
        let read_len = PAYLOAD_READ_LEN as u16;

        // The geometry this platform's read always has; `ssgg --debug onboard read ...` shows it before
        // the first byte moves.
        tracing::info!(
            "onboard read geometry: profile slot {profile} = GG config {} (file {fs_id:04X}) is              {PULLS_PER_FILE} pulls of \
             {DEVICE_RESOURCE_BUFFER_LEN} bytes, {} payload reads of {PAYLOAD_READ_LEN} bytes, {} of the {} \
             payload bytes covered",
            profile + 1,
            PULLS_PER_FILE * READS_PER_PULL,
            PULLS_PER_FILE * READS_PER_PULL * PAYLOAD_READ_LEN,
            PROFILE_FILE_BYTES,
        );

        // GG polls the firmware version in front of a profile session. The version is informational, so a
        // refused or unanswered poll is recorded and the read continues (see [`Self::firmware_version`]).
        let firmware = match self.firmware_version(FirmwareSource::Keyboard) {
            Ok(version) => version,
            Err(e) => {
                tracing::debug!("onboard read: firmware poll failed: {e}");
                None
            }
        };

        // GG's 2-byte probe follows, and the schema word it answers decides whether this read can decode
        // what it is about to pull (see [`Self::probe_schema`]).
        self.probe_schema(fs_id)?;

        let mut chunks: Vec<ChunkRecord> = Vec::with_capacity(PULLS_PER_FILE);
        for chunk in 0..PULLS_PER_FILE {
            let file_offset = (chunk * DEVICE_RESOURCE_BUFFER_LEN) as u32;
            let (ack_report_id, ack_hex) = self.pull_window(fs_id, file_offset, DEVICE_RESOURCE_BUFFER_LEN as u16)?;
            let mut reads: Vec<ReadReply> = Vec::with_capacity(READS_PER_PULL);
            for read in 0..READS_PER_PULL {
                let buffer_offset = (read * PAYLOAD_READ_LEN) as u16;
                reads.push(self.read_window(&format!("chunk {chunk}"), buffer_offset, read_len)?);
            }
            chunks.push(ChunkRecord {
                index: chunk,
                fs_id,
                file_offset,
                ack_report_id,
                ack_hex,
                reads,
            });
        }

        let mut image = ProfileImage::assemble(profile, chunks)?;
        image.firmware = firmware;
        // The bytes are stamped with the keyboard they came off: the layout, the encodable fields and
        // the feature set are all per device, so a profile file has to name its model.
        let info = self.info();
        image.device = Some(OnboardDevice {
            vendor_id: info.vendor_id,
            product_id: info.product_id,
            product: device_name_from_product_id(info.product_id).to_string(),
            known: known_product_id(info.product_id),
        });
        Ok(image)
    }

    /// Runs GG's 2-byte probe in front of the file read and checks the schema word it answers.
    ///
    /// The probe is a pull of [`PROBE_PULL_LEN`] bytes at file offset 0 - the shape the capture shows for
    /// the profile file of slot 0 - followed by one payload read at buffer offset 0, whose answer starts
    /// with the schema word (`03 0b 00 …` on the reference device, i.e. [`SCHEMA_B`]).
    ///
    /// The read decodes [`SCHEMA_B`] only, so any other answer stops the read here: it is the cheapest
    /// place to find out (nothing has been written, the device has only been asked to pull), and a schema
    /// this decoder cannot lay out would surface later as a profile whose checksum never matches.
    /// `[EXPERIMENTAL]`: the schema word has been read on hardware (Apex 7, 2026-10-18).
    fn probe_schema(&self, fs_id: u16) -> Result<u16> {
        self.pull_window(fs_id, 0, PROBE_PULL_LEN)?;
        let reply = self.read_window("schema probe", 0, PAYLOAD_READ_LEN as u16)?;
        let schema = schema_of(&reply.data);
        if schema != SCHEMA_B {
            return Err(Error::Profile(format!(
                "device answers schema {schema:#06x} at payload offset 0; this read is implemented for \
                 {SCHEMA_B:#06x} (schema B) and measured on hardware with that schema only"
            )));
        }
        Ok(schema)
    }

    /// Fills the device's resource buffer with one pull frame and drains the pacing packet that follows it.
    ///
    /// Returns the pacing packet's report ID and hex text for the chunk record (`(0, "")` when nothing
    /// arrived, which is logged, not an error - see [`Self::read_pull_ack`]).
    fn pull_window(&self, fs_id: u16, file_offset: u32, size: u16) -> Result<(u8, String)> {
        let path = self.collection_label();
        self.write_output_command(OnboardPullCommand {
            fs_id,
            file_offset,
            size,
        })
        .map_err(|e| {
            Error::DeviceCommunication(format!(
                "onboard pull frame (file {fs_id:04X} offset {file_offset} size {size}) on {path} rejected: {e}"
            ))
        })?;
        match self.read_pull_ack() {
            Some(report) if !report.is_empty() => Ok((report[0], hex_of(&report))),
            _ => {
                tracing::debug!(
                    "no input report arrived after the pull for file {fs_id:04X} offset {file_offset} on {path}"
                );
                Ok((0, String::new()))
            }
        }
    }

    /// Requests one payload window and collects its reply, then paces the way GG's read spec paces
    /// ([`GG_GAP_MS`] between transfers).
    fn read_window(&self, label: &str, buffer_offset: u16, read_len: u16) -> Result<ReadReply> {
        let path = self.collection_label();
        self.write_feature_command(OnboardReadCommand {
            buffer_offset,
            size: read_len,
        })
        .map_err(|e| {
            Error::DeviceCommunication(format!(
                "onboard read request for {label} buffer offset {buffer_offset} on {path} rejected: {e}"
            ))
        })?;
        // If there's no wait there's a race of whether the device prepares the buffer in time, resulting
        // in garbage reads
        std::thread::sleep(Duration::from_millis(1));
        let reply = self.read_reply_report(PAYLOAD_READ_LEN)?;
        let parsed = parse_read_reply_at(&reply, read_len, READ_REPLY_FRAMING_LEN)?;
        if let Some(msg) = reply_mismatch(&parsed, buffer_offset, read_len) {
            tracing::debug!("onboard read: {msg}");
        }
        if GG_GAP_MS > 0 {
            std::thread::sleep(Duration::from_millis(GG_GAP_MS));
        }
        Ok(parsed)
    }

    /// Polls GG's firmware version ([`OnboardFirmwareVersionCommand`]) and decodes its answer.
    ///
    /// The source byte selects the answer: [`FirmwareSource::Keyboard`] (`90 00`) is the firmware running
    /// the keyboard, [`FirmwareSource::Led`] (`90 01`) the LED controller's. Both answer on hardware (Apex
    /// 7, 2026-10-19; the GG capture has both forms on the wire). `[EXPERIMENTAL]`.
    ///
    /// The read calls this with [`FirmwareSource::Keyboard`] for the version line of a
    /// [`ProfileImage`](crate::devices::keyboards::apex_legacy_profile::ProfileImage); a keyboard that
    /// accepts the frame and answers nothing is not an error, `None` is returned.
    ///
    /// # Errors
    ///
    /// Returns an error when the device refuses the frame.
    pub fn firmware_version(&self, source: FirmwareSource) -> Result<Option<String>> {
        let path = self.collection_label();
        let frame = OnboardFirmwareVersionCommand { source };
        self.write_output_command(frame).map_err(|e| {
            Error::DeviceCommunication(format!(
                "onboard firmware poll {} on {path} rejected: {e}",
                frame.description()
            ))
        })?;
        // The answer lands 1-3 ms after the frame; GG's own cadence is one frame per GG_GAP_MS.
        if GG_GAP_MS > 0 {
            std::thread::sleep(Duration::from_millis(GG_GAP_MS));
        }
        let answer = self.read_interrupt_report(FIRMWARE_REPLY_LEN, FIRMWARE_REPLY_ATTEMPTS);
        if answer.is_none() {
            tracing::debug!("onboard firmware poll on {path} answered nothing");
        }
        Ok(answer.as_deref().and_then(parse_firmware_version))
    }

    /// Collects one payload reply: the feature-report reply the capture shows, then the interrupt
    /// endpoint. Returns `[report ID][payload…]`, the shape `parse_read_reply_at` consumes.
    fn read_reply_report(&self, payload_len: usize) -> Result<Vec<u8>> {
        let path = self.collection_label();
        let stop = Instant::now() + Duration::from_millis(REPLY_BUDGET_MS);
        let mut empty_answers = 0u32;
        let mut last_error: Option<Error> = None;

        while Instant::now() < stop {
            match self.read_profile_feature_packet(READ_REPLY_ID, READ_REPLY_BUFFER_LEN) {
                Ok(report) if reply_carries_payload(&report, payload_len) => return Ok(report),
                Ok(report) => {
                    empty_answers = empty_answers.saturating_add(1);
                    tracing::debug!(
                        "onboard read on {path}: feature reply of {} bytes (report {:#04X}) carries no \
                         payload of {payload_len} bytes",
                        report.len(),
                        report.first().copied().unwrap_or(0),
                    );
                }
                Err(e) => {
                    last_error = Some(Error::DeviceCommunication(format!("payload read on {path}: {e}")));
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(REPLY_POLL_MS));
        }

        let mut input_attempts = 0u32;
        while input_attempts < REPLY_INPUT_ATTEMPTS {
            input_attempts = input_attempts.saturating_add(1);
            match self.read_profile_input_packet(INPUT_REPORT_MAX_LEN, REPLY_POLL_MS) {
                Ok(report) if reply_carries_payload(&report, payload_len) => {
                    tracing::debug!(
                        "onboard read: reply arrived on {path} as a {}-byte input report",
                        report.len()
                    );
                    return Ok(report);
                }
                Ok(report) if !is_ack(&report) => {
                    tracing::debug!(
                        "onboard read on {path}: ignored a {}-byte input report (report {:#04X})",
                        report.len(),
                        report.first().copied().unwrap_or(0),
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!("onboard read on {path}: input report read failed: {e}");
                    break;
                }
            }
        }

        let reason = match last_error {
            Some(e) => e.to_string(),
            None => format!("{empty_answers} feature reply answer(s) carried no payload of {payload_len} bytes"),
        };
        Err(Error::DeviceCommunication(format!(
            "no profile payload reply from {path}: {reason}"
        )))
    }

    /// Reads the pacing packet that follows a pull. Its content is not looked at - GG's read specifiers keep
    /// zero bytes of it - and the pull has no answer of its own: the device keeps streaming its ordinary
    /// input reports on the interrupt endpoint (report `0x01`, all-zero on an idle keyboard, 1.7-3.7 ms after
    /// the pull in the capture). The drain is load-bearing: a run on 2026-10-17 that skipped it failed every
    /// payload read-back.
    fn read_pull_ack(&self) -> Option<Vec<u8>> {
        self.read_interrupt_report(PULL_ACK_LEN, PULL_ACK_ATTEMPTS)
    }

    /// Identity of the collection the read runs on, for logs and read errors.
    fn collection_label(&self) -> String {
        let info = self.inner.info();
        format!(
            "vid {:04x}:pid {:04x} interface {} ({})",
            info.vendor_id, info.product_id, info.interface_number, info.path
        )
    }

    /// Polls the collection for one non-empty report of at most `len` bytes.
    fn read_interrupt_report(&self, len: usize, attempts: u32) -> Option<Vec<u8>> {
        let path = self.collection_label();
        for _ in 0..attempts {
            match self.read_profile_input_packet(len, REPLY_POLL_MS) {
                Ok(report) if !report.is_empty() => return Some(report),
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!("onboard input-report poll on {path}: {e}");
                    break;
                }
            }
        }
        None
    }

    /// Serializes a command through the keyboard's [`HidReportBuilder`] and sends it as an output report:
    /// report number 0 plus the 64-byte data stage, i.e. `SET_REPORT wValue 0x0200 wLength 64` as the
    /// reference capture shows (the kernel takes the report number out of the data stage, so the buffer is
    /// one byte longer than the wire).
    ///
    /// This is [`GenericKeyboard::write_profile_packet`], deliberately not [`GenericKeyboard::send_report`]:
    /// `send_report` writes through [`crate::devices::write_padded_report`], whose global
    /// [`crate::devices::HidOptimizer`] skips any report byte-identical to one sent in the last 50 ms, and
    /// profile-protocol transfers repeat identical frames (GG's session polls `90 00` and `90 01` over and
    /// over, 5 ms apart), which that optimizer would swallow.
    fn write_output_command<C: HidCommand>(&self, command: C) -> Result<()> {
        let mut buffer = [0u8; ONBOARD_OUTPUT_REPORT_SIZE];
        let len = self.inner.report_builder.build_report(command, &mut buffer)?;
        if len != buffer.len() {
            return Err(Error::DeviceCommunication(format!(
                "onboard output report built {len} of {} bytes",
                buffer.len()
            )));
        }
        self.inner.write_profile_packet(&buffer)
    }

    /// Serializes a command through the keyboard's [`HidReportBuilder`] and sends it as a feature report on
    /// report number 0 (`SET_REPORT wValue 0x0300 wLength 642` in the capture).
    fn write_feature_command<C: HidCommand>(&self, command: C) -> Result<()> {
        let mut buffer = [0u8; ONBOARD_FEATURE_REPORT_SIZE];
        let len = self.inner.report_builder.build_report(command, &mut buffer)?;
        if len != buffer.len() {
            return Err(Error::DeviceCommunication(format!(
                "onboard feature report built {len} of {} bytes",
                buffer.len()
            )));
        }
        self.inner.send_feature(&buffer, ONBOARD_FEATURE_REPORT_SIZE)
    }
}

// Delegate Device trait.
impl Deref for Apex7 {
    type Target = GenericKeyboard;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Apex7 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

// Delegate Device trait.
impl Device for Apex7 {
    fn info(&self) -> &crate::devices::DeviceInfo {
        self.inner.info()
    }

    fn device_type(&self) -> DeviceType {
        self.inner.device_type()
    }

    fn initialize(&mut self) -> Result<()> {
        self.inner.initialize()
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    fn send_raw(&mut self, data: &[u8]) -> Result<()> {
        self.inner.send_raw(data)
    }

    fn receive_raw(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.inner.receive_raw(buf)
    }
}

// Delegate Keyboard trait; the on-board profile methods are this device's own.
crate::impl_keyboard_with_delegation!(Apex7, {
    fn onboard_codec(&self) -> Option<&'static dyn OnboardProfileCodec> {
        self.supports_onboard().then_some(&LegacyApexOnboard)
    }

    async fn read_onboard_profile(&self, profile: u8) -> Result<ProfileImage> {
        self.read_profile(profile).await
    }
});

/// A reply carries a payload when it is long enough to hold `payload_len` bytes behind its report-ID
/// byte and its report ID is not a poll acknowledge.
fn reply_carries_payload(report: &[u8], payload_len: usize) -> bool {
    if report.is_empty() {
        return false;
    }
    !ACK_REPORT_IDS.contains(&report[0]) && report.len() > payload_len
}

fn is_ack(report: &[u8]) -> bool {
    report.first().is_some_and(|id| ACK_REPORT_IDS.contains(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::keyboards::apex_legacy_profile::{
        MAX_PROFILE_INDEX, PROFILE_FS_BASE, profile_file_id, validate_profile,
    };

    #[test]
    fn the_family_predicate_selects_the_device() {
        assert!(onboard_family(product_ids::APEX_7));
        for pid in [product_ids::APEX_3, product_ids::APEX_PRO, product_ids::APEX_7_2024] {
            assert!(
                !onboard_family(pid),
                "PID {pid:#06x} is not the legacy platform discovery wraps in Apex7"
            );
        }
    }

    #[test]
    fn reply_carries_payload_needs_room_behind_the_report_id() {
        assert!(reply_carries_payload(&[0x03, 0x01, 0x02], 2));
        assert!(!reply_carries_payload(&[0x03, 0x01], 2));
        assert!(!reply_carries_payload(&[0x0B, 0x0a, 0x00, 0x00], 2));
        assert!(!reply_carries_payload(&[0x12, 0x00], 2));
        assert!(!reply_carries_payload(&[], 2));
    }

    #[test]
    fn the_read_geometry_is_the_captured_one() {
        assert_eq!(profile_file_id(0), PROFILE_FS_BASE);
        assert_eq!(
            PULLS_PER_FILE * DEVICE_RESOURCE_BUFFER_LEN,
            12_288,
            "the fixed read covers the payload of the reference capture"
        );
        assert_eq!(
            READS_PER_PULL,
            DEVICE_RESOURCE_BUFFER_LEN / PAYLOAD_READ_LEN,
            "the fixed read drains the buffer in 512-byte windows"
        );
        assert!(validate_profile(MAX_PROFILE_INDEX).is_ok());
        assert!(validate_profile(MAX_PROFILE_INDEX + 1).is_err());
    }

    mod onboard {
        use super::super::*;
        use crate::devices::{DeviceInfo, DeviceType};

        fn device_without_handle(product_id: u16) -> Apex7 {
            let info = DeviceInfo {
                name: std::borrow::Cow::Borrowed("Test Apex 7"),
                device_type: DeviceType::Keyboard,
                vendor_id: crate::STEELSERIES_VENDOR_ID,
                product_id,
                interface_number: 1,
                usage_page: 0xFFC0,
                usage: 0x01,
                serial_number: None,
                manufacturer: Some("SteelSeries".to_string()),
                path: "/test/device/path".to_string(),
            };
            Apex7::new(GenericKeyboard::new_without_device(info))
        }

        #[test]
        fn only_the_legacy_family_gets_the_read() {
            let apex_7 = device_without_handle(product_ids::APEX_7);
            assert!(apex_7.supports_onboard());
            assert!(Apex7::supports_onboard_profile(&apex_7));
            // The codec answers which product IDs its protocol covers: a profile JSON that names a
            // board outside this list was written by a family this codec is not.
            assert!(LegacyApexOnboard.knows_product(product_ids::APEX_7));
            assert!(LegacyApexOnboard.knows_product(product_ids::APEX_PRO_TKL));
            assert!(LegacyApexOnboard.knows_product(product_ids::APEX_150));
            assert!(LegacyApexOnboard.knows_product(product_ids::APEX_7_TKL));
            assert!(
                !LegacyApexOnboard.knows_product(product_ids::APEX_PRO_TKL_2023),
                "the 2023 board is not on the legacy platform"
            );
            assert!(
                !LegacyApexOnboard.knows_product(0xFFFF),
                "an ID the registry does not list"
            );
            for pid in [product_ids::APEX_3, product_ids::APEX_PRO, product_ids::APEX_7_2024] {
                assert!(
                    !onboard_family(pid),
                    "PID {pid:#06x} is not the platform the read was reverse-engineered from"
                );
            }
        }

        #[test]
        fn a_device_without_a_handle_says_so() {
            let keyboard = device_without_handle(product_ids::APEX_7);
            // report number 0 and a 642-byte ask: the payload read-back shape of the reference capture,
            // which the read side sends to the device verbatim as wLength 642
            match keyboard
                .inner
                .read_profile_feature_packet(0x00, crate::devices::keyboards::apex_legacy_profile::READ_REPORT_LEN)
            {
                Ok(_) => panic!("a device without a handle must not answer a profile packet"),
                Err(error) => assert!(
                    error.to_string().contains("Device not connected"),
                    "the error is the shared handle-less message: {error}"
                ),
            }
        }

        #[test]
        fn a_read_asks_for_the_slot_it_names() {
            // The device carries no slot state: the slot is an argument, and slot 0 is the file the
            // capture read.
            assert_eq!(
                crate::devices::keyboards::apex_legacy_profile::profile_file_id(0),
                crate::devices::keyboards::apex_legacy_profile::PROFILE_FS_BASE
            );
            assert_eq!(
                crate::devices::keyboards::apex_legacy_profile::profile_file_id(4),
                0x0205
            );
        }
    }
}
