//! [EXPERIMENTAL] On-board profile read for the legacy/"macho" Apex platform.
//!
//! Frame construction, reply parsing, payload geometry and decoding live in [`crate::devices::keyboards::apex_legacy_profile`].
//! This module only moves bytes: it sends GG's opening chain (which starts with the firmware poll), the
//! pull frames, the pacing read that follows
//! each pull and the payload reads, collects the replies and assembles a
//! [`crate::devices::keyboards::apex_legacy_profile::ProfileImage`].
//!
//! Wire facts confirmed from the 2026-10-15 Windows USBPcap capture of official GG on an Apex 7
//! (VID `0x1038`, PID `0x1612`, firmware `4.16.8`), decoded from
//! `ssgg-capture-1612-20261015-004400/hub-2.pcap` URB by URB:
//!
//! - every profile transfer runs on USB interface 1 (`wIndex 1`), the usage-page-`0xffc0` collection;
//! - the read opens with GG's six-frame chain, `90 00`, `90 01`, `f4 00`, `6c 00 01`, then `90 00` and
//!   `90 01` again, 11 ms apart, each frame `SET_REPORT wValue 0x0200 wIndex 1 wLength 64` (report number 0,
//!   output type, 64 data bytes); `90 00` and `90 01` are answered on the interrupt endpoint with the ASCII
//!   firmware version (`34 2e 31 36 2e 38 00…` = `4.16.8`) and answer identically (measured 2026-10-17),
//!   `f4 00` answers `00 01 00…` and `6c 00 01` is never answered, so the meanings of both are unknown.
//!   **This crate sends the whole chain, one interrupt read after every frame**: on hardware a read that
//!   sends only the `90 00` poll is answered on the wire and the payload replies come back scrambled, and a
//!   chain run that drained only the first answer saw `90 00` and `90 01` answered and nothing from frame 3
//!   on (Apex 7, 2026-10-18). Which frame arms the resource buffer, and why an undrained answer silences the
//!   frames after it, are not known. The capture contains no 13-byte transfer at all, so the pacing frame this
//!   module sent before is not on GG's wire;
//! - the pull frame is `SET_REPORT wValue 0x0200 wLength 64` carrying
//!   `[85][00][file selector u16 LE][file offset u32 LE][size u16 LE]`, 10 frame bytes. GG pairs every pull
//!   with one input read (`read-chunk HIDIO 65`) and keeps **no bytes** of its byte specifier `(0 0 0)`:
//!   the packet that answers it is the keyboard's ordinary input report `0x01` (all-zero on an idle
//!   keyboard, 1.7-3.7 ms after the pull in all 121 pulls). That read is pacing, not a data transfer - but
//!   it is load-bearing: `read_pull_ack` runs it for every chunk, and a 2026-10-17 run that skipped it
//!   failed every payload read-back;
//! - the payload-read request is `SET_REPORT wValue 0x0300 wLength 642` — a **feature** report, not an
//!   output report — carrying `[83][00][resource-buffer offset u16 LE][size u16 LE]`, 6 frame bytes;
//! - the answer is a **feature-report reply** (`GET_REPORT wValue 0x0300 wIndex 1 wLength 642`): 643
//!   bytes, report ID `0x03` and the requested bytes from data byte 0 — the 2-byte probe of file
//!   `0x0201` answers `03 0b 00` and the 512-byte read at buffer offset 0 answers
//!   `03 0b 00 43 6f 6e 66 69 67 20 31 …` (schema word, then the ASCII profile name at payload offset
//!   2). GG issued 241 of these; 180 answers were all zero, the rest carried profile data, and the
//!   submit-to-completion latency was 1-3 ms.
//! - GG read 122 file groups: 12 pulls of 1024 bytes per file at offsets `0, 1024, …, 11264` for each
//!   of the ten files `0x0201`-`0x0205` and `0x0301`-`0x0305`, plus a 2-byte probe on the profile file
//!   of slot 0 and on the lighting file; the opening chain runs once, in front of the first pull.
//!
//! GG's own read-back is unusable, which is why this command reports checksum words as
//! "stored/computed" pairs and never claims a profile is verified: in the capture the assembled
//! profile name never decodes (it reads `` `1 `` for slot 0) and the stored tail word of slot 0
//! (`0x25EE1762`) is not the CRC of the bytes GG assembled. The schema word GG reads at payload
//! offset 0 is `0x0000` on this device, i.e. the base schema of the platform.
//!
//! The transfers run on the handle [`GenericKeyboard`] already owns - the collection the device manager
//! opened, held open for the whole read (see the packet methods named below). `usbhid` polls a
//! collection's interrupt endpoint only
//! while a descriptor on that node is open, so the interrupt reports the read consumes (the `0x90`
//! answer and the packet after each pull) can
//! only arrive on a handle that stays open for the whole read - measured on an Apex 7 in the 2026-10-16
//! Linux run capture (`linux-read.pcapng`), where a reader that opened the node per report collected
//! nothing; see `docs/development/apex-7-onboard-profile-read.md`, "Input reports: the node must be open
//! when the report arrives". Apex 7 wired works through hidapi: the same `SET_REPORT wValue 0x0200` and
//! `wValue 0x0300` shapes the reference capture shows, on the collection hidapi opened.
//! `[EXPERIMENTAL]`: the read completes on hardware - a full profile slot has been read from an Apex 7
//! under Linux (reported 2026-10-18; model, firmware and the decoded bytes are not recorded in this repo
//! yet). What is still unproven is the decode: slot layout, checksum words and the per-slot compare
//! against GG (`ONBOARD.md` steps 3-5). This module never writes a profile.

use std::time::{Duration, Instant};

use super::GenericKeyboard;
use super::apex_legacy_profile::{
    ChunkRecord, DEVICE_RESOURCE_BUFFER_LEN, GG_GAP_MS, GG_INPUT_READ_BYTES, PAYLOAD_READ_LEN, PROFILE_FILE_BYTES,
    PULLS_PER_FILE, ProfileImage, READ_REPLY_FRAMING_LEN, READS_PER_PULL, ReadReply, hex_of, parse_firmware_version,
    parse_read_reply_at, profile_file_id, reply_mismatch, validate_profile,
};
use crate::devices::hid_reports::{
    HidCommand, HidDeviceType, HidReportBuilder, ONBOARD_FEATURE_REPORT_SIZE, ONBOARD_INIT_CHAIN,
    ONBOARD_INIT_FRAME_COUNT, ONBOARD_OUTPUT_REPORT_SIZE, ONBOARD_REPLY_REPORT_SIZE, OnboardPullCommand,
    OnboardReadCommand,
};
use crate::error::{Error, Result};

/// Report number to ask for when reading a payload read back: **0**, which is the
/// `GET_REPORT wValue 0x0300 wIndex 1` control read the reference capture shows. The kernel writes the
/// device's own report-ID byte into byte 0 of the buffer, so the reply's leading `03` arrives there
/// whatever we ask for. Earlier builds asked `0x83` (the command byte as a report number), which
/// produced `wValue 0x0383` in the 2026-10-15 Linux capture - a report number the keyboard never
/// declared - and that run answered nothing.
/// `[EXPERIMENTAL]`
const READ_REPLY_ID: u8 = 0x00;
/// Length to request for the payload reply (see [`ONBOARD_REPLY_REPORT_SIZE`]). The kernel forwards
/// this number to the device as `wLength`: 642 answers, asking for more makes the control read never
/// complete, which is what `Connection timed out` meant on 2026-10-15.
const READ_REPLY_WIRE_LEN: usize = ONBOARD_REPLY_REPORT_SIZE;
/// Poll acknowledges and status reports the reply collector skips.
const ACK_REPORT_IDS: [u8; 2] = [0x0B, 0x12];
/// Bytes to read for the pacing packet that follows a pull. The pull has no answer of its own: the device
/// keeps streaming its ordinary input reports on the interrupt endpoint (report `0x01`, all-zero on an idle
/// keyboard, 1.7-3.7 ms after the pull in the capture) and GG consumes one packet per chunk as pacing, taking
/// no bytes from it (GG's `read-chunk HIDIO 65`: report-ID slot plus 64 data bytes, see
/// [`GG_INPUT_READ_BYTES`], which is what this asks hidapi for).
const PULL_ACK_LEN: usize = GG_INPUT_READ_BYTES;
/// Polls per reply collection round, in milliseconds.
const REPLY_POLL_MS: i32 = 50;
/// Reply budget for one payload read, in milliseconds: the feature-report poll loop runs until this
/// expires, then the input-endpoint fallback starts. A fixed budget, not a caller-set deadline (the
/// `--timeout` knob is gone).
const REPLY_BUDGET_MS: u64 = 2_000;
/// Input-report polls allowed per reply before giving up.
const REPLY_INPUT_ATTEMPTS: u32 = 20;
/// Reads allowed per pull before giving up (the device streams input reports, so one answer is expected).
const PULL_ACK_ATTEMPTS: u32 = 4;
/// Bytes to read for one opening-chain answer: GG's input read size, [`GG_INPUT_READ_BYTES`] (a report-ID
/// slot plus 64 data bytes), which every chain answer fits (`0x6E` with the ASCII version, `0x12` with
/// `00 01 …`).
const CHAIN_REPLY_LEN: usize = GG_INPUT_READ_BYTES;
/// Reads per chain frame. One: the frame that follows must not consume this frame's answer, and a frame
/// GG leaves unanswered (`6c 00 01`) must not stall the chain.
const CHAIN_REPLY_ATTEMPTS: u32 = 1;
/// Largest input report a profile collection can return (Apex 7 profile collections report 1024).
const INPUT_REPORT_MAX_LEN: usize = 1024;

// The transfers the profile protocol needs are the packet I/O of the keyboard the read runs on:
// `GenericKeyboard::write_profile_packet` for output frames (not `send_report`, whose hid optimizer
// dedup would drop the repeated frames of a read), the keyboard's existing `send_feature` for the
// feature-report read request, and `read_profile_feature_packet` / `read_profile_input_packet` for the
// two reply shapes, all on the handle that device already owns - the collection the device manager
// opened, open for the whole read.
//
// `[EXPERIMENTAL]`: on hardware the opening chain, the pull, the pacing read and the payload read-back all
// complete on an Apex 7 (2026-10-17/18); the read request is answered by the `GET_REPORT` control-IN reply
// the capture shows. The chain is what makes the payload readable: without it the replies come back
// scrambled.

/// Reads one profile slot on one collection a keyboard device holds open, which is how
/// [`super::apex7::Apex7`] reads its own on-board profiles.
///
/// # Errors
///
/// Returns an error when the collection rejects a frame, never answers a payload read, or `profile` is
/// not a slot this read supports.
pub async fn read_profile(keyboard: &GenericKeyboard, profile: u8) -> Result<ProfileImage> {
    // The read plan blocks on reply polls and GG's inter-frame pacing, so it runs on its own thread. A
    // scoped thread borrows the keyboard and the scope is joined before this returns, which is the whole
    // reason to use `std::thread::scope`: the borrow is checked, not asserted.
    std::thread::scope(|scope| {
        scope
            .spawn(move || read_image(keyboard, profile))
            .join()
            .map_err(|_| Error::Other("onboard read worker panicked".to_string()))?
    })
}

/// Runs one read (GG's opening chain, then [`PULLS_PER_FILE`] pulls, each with its pacing read and payload
/// reads) on one collection.
fn read_image(keyboard: &GenericKeyboard, profile: u8) -> Result<ProfileImage> {
    let path = collection_label(keyboard);
    validate_profile(profile)?;
    let fs_id = profile_file_id(profile);
    let read_len = PAYLOAD_READ_LEN as u16;

    // The geometry this platform's read always has; `ssgg --debug onboard read ...` shows it before
    // the first byte moves.
    tracing::info!(
        "onboard read geometry: profile slot {profile} (file {fs_id:04X}) is {PULLS_PER_FILE} pulls of \
         {DEVICE_RESOURCE_BUFFER_LEN} bytes, {} payload reads of {PAYLOAD_READ_LEN} bytes, {} of the {} payload bytes covered",
        PULLS_PER_FILE * READS_PER_PULL,
        PULLS_PER_FILE * READS_PER_PULL * PAYLOAD_READ_LEN,
        PROFILE_FILE_BYTES,
    );

    // GG's opening chain runs in front of the first pull, and the firmware version line comes from its
    // first poll; the read does not produce a clean payload without it (see [`send_init_chain`]).
    let firmware = send_init_chain(keyboard)?;

    let mut chunks: Vec<ChunkRecord> = Vec::with_capacity(PULLS_PER_FILE);
    for chunk in 0..PULLS_PER_FILE {
        let file_offset = (chunk * DEVICE_RESOURCE_BUFFER_LEN) as u32;
        let pull = OnboardPullCommand {
            fs_id,
            file_offset,
            size: DEVICE_RESOURCE_BUFFER_LEN as u16,
        };
        write_output_command(keyboard, pull).map_err(|e| {
            Error::DeviceCommunication(format!(
                "onboard pull frame for chunk {chunk} (file {fs_id:04X} offset {file_offset}) on \
                 {path} rejected: {e}"
            ))
        })?;
        let ack = read_pull_ack(keyboard);
        let (ack_report_id, ack_hex) = match ack.as_deref() {
            Some(report) if !report.is_empty() => (report[0], hex_of(report)),
            _ => {
                tracing::debug!("no input report arrived after the pull for chunk {chunk} on {path}");
                (0, String::new())
            }
        };

        let mut reads: Vec<ReadReply> = Vec::with_capacity(READS_PER_PULL);
        for read in 0..READS_PER_PULL {
            let buffer_offset = (read * PAYLOAD_READ_LEN) as u16;
            let command = OnboardReadCommand {
                buffer_offset,
                size: read_len,
            };
            write_feature_command(keyboard, command).map_err(|e| {
                Error::DeviceCommunication(format!(
                    "onboard read request for chunk {chunk} buffer offset {buffer_offset} on \
                     {path} rejected: {e}"
                ))
            })?;
            let reply = read_reply_report(keyboard, PAYLOAD_READ_LEN)?;
            let parsed = parse_read_reply_at(&reply, read_len, READ_REPLY_FRAMING_LEN)?;
            if let Some(msg) = reply_mismatch(&parsed, buffer_offset, read_len) {
                tracing::debug!("onboard read: {msg}");
            }
            reads.push(parsed);
            if GG_GAP_MS > 0 {
                std::thread::sleep(Duration::from_millis(GG_GAP_MS));
            }
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

    ProfileImage::assemble(profile, chunks).map(|image| image.with_firmware(firmware))
}

/// Sends GG's opening chain: [`ONBOARD_INIT_CHAIN`], six 64-byte output frames
/// ([`crate::devices::hid_reports::OnboardInitCommand`]) [`GG_GAP_MS`] apart - `90 00`, `90 01`, `f4 00`,
/// `6c 00 01`, `90 00`, `90 01` - draining one interrupt report after **every** frame, and returning the
/// firmware version the first `90 00` answers.
///
/// The chain is not optional. On hardware a read that sends only the `90 00` poll
/// ([`crate::devices::hid_reports::OnboardFirmwareVersionCommand`]) is answered on the wire and the payload
/// replies come back as scrambled bytes; with this chain the same read collects a profile slot (Apex 7,
/// 2026-10-18). Which frame arms the resource buffer is unknown - `f4 00` answers `00 01 00 …` in the
/// capture and `6c 00 01` answers nothing - so all six frames go, the way GG sends them. A refused frame
/// fails the read: a keyboard that drops the chain cannot be read cleanly.
///
/// **Each frame is answered only after the previous answer has been read.** Measured 2026-10-18: a chain
/// run that drained just the first answer saw the answers of `90 00` and `90 01` and nothing from frame 3
/// on, with the frames on the wire. GG's session keeps the interrupt stream read throughout (its read spec
/// issues a `read-chunk HIDIO 65` per transfer), so a read per frame matches GG's pattern; the flow-control
/// reading (the device's command queue keeps one answer pending) is inference, not measured.
fn send_init_chain(keyboard: &GenericKeyboard) -> Result<Option<String>> {
    let path = collection_label(keyboard);
    let mut firmware = None;
    for (index, frame) in ONBOARD_INIT_CHAIN.iter().enumerate() {
        write_output_command(keyboard, *frame).map_err(|e| {
            Error::DeviceCommunication(format!(
                "onboard opening chain frame {} of {ONBOARD_INIT_FRAME_COUNT} ({}) on {path} rejected: {e}",
                index + 1,
                frame.description()
            ))
        })?;
        if GG_GAP_MS > 0 {
            // The answer lands 1-3 ms after the frame, and GG's own cadence is one frame per GG_GAP_MS.
            std::thread::sleep(Duration::from_millis(GG_GAP_MS));
        }
        let answer = read_interrupt_report(keyboard, CHAIN_REPLY_LEN, CHAIN_REPLY_ATTEMPTS);
        match answer.as_deref() {
            Some(report) if index == 0 => firmware = parse_firmware_version(report),
            Some(report) => {
                tracing::debug!(
                    "onboard opening chain frame {} on {path} answered {}",
                    index + 1,
                    hex_of(&report[..report.len().min(8)])
                );
            }
            None => {
                tracing::debug!("onboard opening chain frame {} on {path} answered nothing", index + 1);
            }
        }
    }
    if firmware.is_none() {
        tracing::debug!("onboard opening chain on {path} answered no firmware version");
    }
    Ok(firmware)
}

/// Collects one payload reply: the feature-report reply the capture shows, then the interrupt
/// endpoint. Returns `[report ID][payload…]`, the shape [`parse_read_reply`] consumes.
fn read_reply_report(keyboard: &GenericKeyboard, payload_len: usize) -> Result<Vec<u8>> {
    let path = collection_label(keyboard);
    let stop = Instant::now() + Duration::from_millis(REPLY_BUDGET_MS);
    let mut empty_answers = 0u32;
    let mut last_error: Option<Error> = None;

    while Instant::now() < stop {
        match keyboard.read_profile_feature_packet(READ_REPLY_ID, READ_REPLY_WIRE_LEN) {
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
        std::thread::sleep(Duration::from_millis(REPLY_POLL_MS.max(0).try_into().unwrap_or(1u64)));
    }

    let mut input_attempts = 0u32;
    while input_attempts < REPLY_INPUT_ATTEMPTS {
        input_attempts = input_attempts.saturating_add(1);
        match keyboard.read_profile_input_packet(INPUT_REPORT_MAX_LEN, REPLY_POLL_MS as u64) {
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

/// Reads the pacing packet that follows a pull. Its content is not looked at - GG's read specifiers keep zero
/// bytes of it - and the pull has no answer of its own: the device keeps streaming its ordinary input reports
/// on the interrupt endpoint (report `0x01`, all-zero on an idle keyboard, 1.7-3.7 ms after the pull in the
/// capture). The drain is load-bearing on hidraw: a run on 2026-10-17 that skipped it failed every payload
/// read-back with `io::read_exact: unexpected end of file`.
fn read_pull_ack(keyboard: &GenericKeyboard) -> Option<Vec<u8>> {
    read_interrupt_report(keyboard, PULL_ACK_LEN, PULL_ACK_ATTEMPTS)
}

/// Identity of the collection the read runs on, for logs and read errors.
fn collection_label(keyboard: &GenericKeyboard) -> String {
    let info = &keyboard.info;
    format!(
        "vid {:04x}:pid {:04x} interface {} ({})",
        info.vendor_id, info.product_id, info.interface_number, info.path
    )
}

/// Polls the node for one non-empty report of at most `len` bytes.
fn read_interrupt_report(keyboard: &GenericKeyboard, len: usize, attempts: u32) -> Option<Vec<u8>> {
    let path = collection_label(keyboard);
    for _ in 0..attempts {
        match keyboard.read_profile_input_packet(len, REPLY_POLL_MS as u64) {
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

/// Serializes a command through [`HidReportBuilder`] and sends it as an output report: report number
/// 0 plus the 64-byte data stage, i.e. `SET_REPORT wValue 0x0200 wLength 64` as the reference capture
/// shows (the kernel takes the report number out of the data stage, so the buffer is one byte longer
/// than the wire).
fn write_output_command<C: HidCommand>(keyboard: &GenericKeyboard, command: C) -> Result<()> {
    let mut buffer = [0u8; ONBOARD_OUTPUT_REPORT_SIZE];
    let len = HidReportBuilder::new(HidDeviceType::Keyboard).build_report(command, &mut buffer)?;
    if len != buffer.len() {
        return Err(Error::DeviceCommunication(format!(
            "onboard output report built {len} of {} bytes",
            buffer.len()
        )));
    }
    keyboard.write_profile_packet(&buffer)
}

/// Serializes a command through [`HidReportBuilder`] and sends it as a feature report on report number
/// 0 (`SET_REPORT wValue 0x0300 wLength 642` in the capture).
fn write_feature_command<C: HidCommand>(keyboard: &GenericKeyboard, command: C) -> Result<()> {
    let mut buffer = [0u8; ONBOARD_FEATURE_REPORT_SIZE];
    let len = HidReportBuilder::new(HidDeviceType::Keyboard).build_report(command, &mut buffer)?;
    if len != buffer.len() {
        return Err(Error::DeviceCommunication(format!(
            "onboard feature report built {len} of {} bytes",
            buffer.len()
        )));
    }
    keyboard.send_feature(&buffer, ONBOARD_FEATURE_REPORT_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::keyboards::apex_legacy_profile::{
        DEVICE_RESOURCE_BUFFER_LEN, MAX_PROFILE_INDEX, PAYLOAD_READ_LEN, PROFILE_FS_BASE, PULLS_PER_FILE,
        READS_PER_PULL, profile_file_id, validate_profile,
    };

    #[test]
    fn reply_carries_payload_needs_room_behind_the_report_id() {
        assert!(reply_carries_payload(&[0x03, 0x01, 0x02], 2));
        assert!(!reply_carries_payload(&[0x03, 0x01], 2));
        assert!(!reply_carries_payload(&[0x0B, 0x0a, 0x00, 0x00], 2));
        assert!(!reply_carries_payload(&[0x12, 0x00], 2));
        assert!(!reply_carries_payload(&[], 2));
    }

    #[test]
    fn default_read_config_matches_the_captured_geometry() {
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
}
