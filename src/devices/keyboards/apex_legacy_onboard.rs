//! [EXPERIMENTAL] On-board profile transport for the legacy/"macho" Apex platform.
//!
//! Frame construction, reply parsing, payload geometry and decoding live in [`crate::onboard_profile`].
//! This module only moves bytes: it opens the collection that carries profile traffic, sends the
//! opening-chain, pull and read frames the read plan describes, collects replies and assembles a
//! [`crate::onboard_profile::ProfileImage`].
//!
//! Wire facts confirmed from the 2026-10-15 Windows USBPcap capture of official GG on an Apex 7
//! (VID `0x1038`, PID `0x1612`, firmware `4.16.8`), decoded from
//! `ssgg-capture-1612-20261015-004400/hub-2.pcap` URB by URB:
//!
//! - every profile transfer runs on USB interface 1 (`wIndex 1`), the usage-page-`0xffc0` collection;
//! - the read opens with a six-frame chain, each one `SET_REPORT wValue 0x0200 wIndex 1 wLength 64`
//!   (report number 0, output type, 64 data bytes): `90 00`, `90 01`, `f4 00`, `6c 00 01`, then
//!   `90 00` and `90 01` again, 11 ms apart. The two `90` polls are answered on the interrupt endpoint
//!   with the ASCII firmware version (`34 2e 31 36 2e 38 00…` = `4.16.8`), `f4 00` with
//!   `00 01 00…`, `6c 00 01` with nothing. The meanings of `f4` and `6c` are unknown. The capture
//!   contains no 13-byte transfer at all, so the pacing frame this module sent before is not on GG's
//!   wire;
//! - the pull frame is `SET_REPORT wValue 0x0200 wLength 64` carrying
//!   `[85][00][file selector u16 LE][file offset u32 LE][size u16 LE]`, 10 frame bytes;
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
//! Linux: the request goes out through the raw hidraw paths in [`crate::devices`] (the hidapi
//! feature-report ioctl has the wrong direction bits); the reply is collected with
//! [`crate::devices::read_feature_report_raw`] first and with the interrupt endpoint as a fallback,
//! because the kernel's `HID_REPORT_GET` path on this collection returns `ETIMEDOUT`.
//!
//! Nothing in this path has been replayed on hardware. This module never writes a profile.

use std::time::{Duration, Instant};

use crate::devices::find_hidraw_for_interface;
#[cfg(unix)]
use crate::devices::{
    read_feature_report_raw, read_input_report_raw, send_feature_report_raw, write_output_report_raw,
};

/// Windows/macOS transport: hidapi has the same control transfers, so the frames and replies are
/// identical on the wire. Each call opens the collection by path.
#[cfg(not(unix))]
mod wire_shim {
    use super::*;

    fn open(path: &str) -> Result<hidapi::HidDevice> {
        let api = hidapi::HidApi::new().map_err(|e| Error::DeviceCommunication(e.to_string()))?;
        let c_path = std::ffi::CString::new(path)
            .map_err(|_| Error::InvalidConfig("device path contains a NUL byte".to_string()))?;
        api.open_path(&c_path)
            .map_err(|e| Error::DeviceCommunication(format!("cannot open {path}: {e}")))
    }

    pub fn write_output_report_raw(path: &str, buffer: &[u8]) -> Result<()> {
        open(path)?
            .send_output_report(buffer)
            .map(|_| ())
            .map_err(|e| Error::DeviceCommunication(e.to_string()))
    }

    pub fn send_feature_report_raw(path: &str, buffer: &[u8], _report_len: usize) -> Result<()> {
        open(path)?
            .send_feature_report(buffer)
            .map_err(|e| Error::DeviceCommunication(e.to_string()))
    }

    pub fn read_feature_report_raw(path: &str, report_id: u8, report_len: usize) -> Result<Vec<u8>> {
        let mut report = vec![report_id; report_len];
        let read = open(path)?
            .get_feature_report(&mut report)
            .map_err(|e| Error::DeviceCommunication(e.to_string()))?;
        report.truncate(read);
        Ok(report)
    }

    pub fn read_input_report_raw(path: &str, max_len: usize, poll_ms: u64) -> Result<Vec<u8>> {
        let mut report = vec![0u8; max_len];
        let timeout = i32::try_from(poll_ms).unwrap_or(i32::MAX);
        let read = open(path)?
            .read_timeout(&mut report, timeout)
            .map_err(|e| Error::DeviceCommunication(e.to_string()))?;
        report.truncate(read);
        Ok(report)
    }
}

use crate::devices::hid_reports::{
    CommandCode, HidCommand, HidDeviceType, HidReportBuilder, ONBOARD_FEATURE_REPORT_SIZE, ONBOARD_INIT_CHAIN,
    ONBOARD_OUTPUT_REPORT_SIZE, ONBOARD_REPLY_REPORT_SIZE, OnboardPullCommand, OnboardReadCommand,
};
use crate::error::{Error, Result};
use crate::onboard_profile::{
    ChunkRecord, INIT_REPLY_LEN, MAX_PROFILE_INDEX, PROFILE_FS_BASE, ProfileImage, ReadConfig, ReadReply,
    SCHEMA_READ_LEN, hex_of, parse_firmware_version, parse_read_reply_at, reply_mismatch, schema_of,
};
#[cfg(not(unix))]
use wire_shim::{read_feature_report_raw, read_input_report_raw, send_feature_report_raw, write_output_report_raw};

/// Highest profile slot this read supports.
pub const MAX_PROFILE_INDEX_FOR_READ: u8 = MAX_PROFILE_INDEX;
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
/// Wire length of the pull answer the capture shows (64 bytes on report `0x85`).
const PULL_ACK_LEN: usize = 64;
/// Polls per reply collection round, in milliseconds.
const REPLY_POLL_MS: i32 = 50;
/// Reply budget per payload read when the caller set no deadline, in milliseconds.
const REPLY_BUDGET_MS: u64 = 2_000;
/// Input-report polls allowed per reply before giving up.
const REPLY_INPUT_ATTEMPTS: u32 = 20;
/// Polls allowed for one pull answer.
const PULL_ACK_ATTEMPTS: u32 = 4;
/// Polls allowed for one opening-chain answer.
const CHAIN_REPLY_ATTEMPTS: u32 = 3;
/// Largest input report a profile collection can return (Apex 7 profile collections report 1024).
const INPUT_REPORT_MAX_LEN: usize = 1024;

/// Reads one profile slot from the first candidate collection that answers.
///
/// # Errors
///
/// Returns an error when no candidate path answers, a frame is rejected, or the read plan cannot be
/// validated.
pub async fn read_profile(paths: &[String], config: &ReadConfig, strict: bool) -> Result<(ProfileImage, String)> {
    let paths = paths.to_vec();
    let config = *config;
    tokio::task::spawn_blocking(move || read_on_first_path(&paths, &config, strict, None))
        .await
        .map_err(|e| Error::Other(format!("onboard read task failed: {e}")))?
}

/// Reads one profile slot twice and reports the first byte where the two reads disagree.
///
/// # Errors
///
/// Returns an error when the slot cannot be read twice.
pub async fn read_profile_verify(
    paths: &[String],
    config: &ReadConfig,
    strict: bool,
) -> Result<(ProfileImage, ProfileImage, Option<usize>, String)> {
    let (first, path) = read_profile(paths, config, strict).await?;
    let (second, _) = read_profile(paths, config, strict).await?;
    let diff = first
        .data
        .iter()
        .zip(second.data.iter())
        .enumerate()
        .find(|(_, (a, b))| a != b)
        .map(|(offset, _)| offset);
    Ok((first, second, diff, path))
}

/// Reads the schema word of profile slot `slot`, i.e. [`PROFILE_FS_BASE`] + slot.
///
/// # Errors
///
/// Returns an error when no candidate path answers a 2-byte schema read.
pub async fn read_schema(
    paths: &[String],
    slot: u8,
    gap_ms: u64,
    strict: bool,
    deadline: Option<Duration>,
) -> Result<(u16, Vec<u8>, String)> {
    let config = schema_config(PROFILE_FS_BASE, slot, gap_ms);
    read_schema_words(paths, &config, strict, deadline).await
}

/// Reads the schema word with an explicit reply framing length, the knob `ssgg onboard schema
/// --sweep` uses to find where this keyboard puts the payload behind the report-ID byte.
///
/// # Errors
///
/// Returns an error when no candidate path answers a 2-byte schema read.
pub async fn read_schema_probe(
    paths: &[String],
    fs_id: u16,
    slot: u8,
    reply_header: usize,
    gap_ms: u64,
    strict: bool,
    deadline: Option<Duration>,
) -> Result<(u16, Vec<u8>, String)> {
    let mut config = schema_config(fs_id, slot, gap_ms);
    config.reply_header = reply_header;
    read_schema_words(paths, &config, strict, deadline).await
}

/// Reads the schema word of an arbitrary file selector (the lighting files `0x0102` and `0x0301`).
///
/// # Errors
///
/// Returns an error when no candidate path answers a 2-byte schema read for that file.
pub async fn read_schema_file(
    paths: &[String],
    fs_id: u16,
    gap_ms: u64,
    strict: bool,
    deadline: Option<Duration>,
) -> Result<(u16, Vec<u8>, String)> {
    let config = schema_config(fs_id, 0, gap_ms);
    read_schema_words(paths, &config, strict, deadline).await
}

async fn read_schema_words(
    paths: &[String],
    config: &ReadConfig,
    strict: bool,
    deadline: Option<Duration>,
) -> Result<(u16, Vec<u8>, String)> {
    let paths = paths.to_vec();
    let config = *config;
    let (image, path) = tokio::task::spawn_blocking(move || read_on_first_path(&paths, &config, strict, deadline))
        .await
        .map_err(|e| Error::Other(format!("onboard schema probe task failed: {e}")))??;
    let raw = image.data;
    Ok((schema_of(&raw), raw, path))
}

/// Read plan for a schema-only probe: the opening chain, one pull of 2 bytes, drained by one 2-byte
/// payload read.
#[must_use]
pub fn schema_config(fs_base: u16, profile: u8, gap_ms: u64) -> ReadConfig {
    ReadConfig {
        profile,
        fs_base,
        chunks: 1,
        resource_len: SCHEMA_READ_LEN,
        read_len: SCHEMA_READ_LEN,
        payload_len: SCHEMA_READ_LEN,
        gap_ms,
        send_init_chain: true,
        reply_header: crate::onboard_profile::READ_REPLY_FRAMING_LEN,
    }
}

/// Runs the read plan on the first candidate path that answers, logging every path that did not.
fn read_on_first_path(
    paths: &[String],
    config: &ReadConfig,
    strict: bool,
    deadline: Option<Duration>,
) -> Result<(ProfileImage, String)> {
    if paths.is_empty() {
        return Err(Error::DeviceNotFound(
            "no candidate device path for the on-board profile read".to_string(),
        ));
    }
    let mut last = String::from("no candidate path answered");
    for path in paths {
        let started = Instant::now();
        match read_image(path, config, strict, deadline) {
            Ok(image) => return Ok((image, path.clone())),
            Err(e) => {
                tracing::debug!("onboard read on {path} failed after {:?}: {e}", started.elapsed());
                last = format!("{path}: {e}");
            }
        }
    }
    Err(Error::DeviceCommunication(format!(
        "no profile collection answered the read ({last})"
    )))
}

/// Runs one read plan (the opening chain, then pulls with their payload reads) on one path.
fn read_image(path: &str, config: &ReadConfig, strict: bool, deadline: Option<Duration>) -> Result<ProfileImage> {
    config.validate()?;
    let fs_id = config.fs_id();
    let reads_per_pull = config.resource_len / config.read_len;
    let read_len = u16::try_from(config.read_len).unwrap_or(u16::MAX);
    let overall = deadline.map(|d| Instant::now() + d);

    let firmware = if config.send_init_chain {
        send_init_chain(path, config.gap_ms)
    } else {
        None
    };

    let mut chunks: Vec<ChunkRecord> = Vec::with_capacity(config.chunks);
    for chunk in 0..config.chunks {
        let file_offset = (chunk * config.resource_len) as u32;
        let pull = OnboardPullCommand {
            fs_id,
            file_offset,
            size: u16::try_from(config.resource_len).unwrap_or(OnboardPullCommand::MAX_SIZE),
        };
        write_output_command(path, pull).map_err(|e| {
            Error::DeviceCommunication(format!(
                "onboard pull frame for chunk {chunk} (file {fs_id:04X} offset {file_offset}) on \
                 {path} rejected: {e}"
            ))
        })?;
        let ack = read_pull_ack(path);
        let (ack_report_id, ack_hex) = match ack.as_deref() {
            Some(report) if !report.is_empty() => (report[0], hex_of(report)),
            _ => {
                tracing::debug!("onboard pull answer missing for chunk {chunk} on {path}");
                (0, String::new())
            }
        };

        let mut reads: Vec<ReadReply> = Vec::with_capacity(reads_per_pull);
        for read in 0..reads_per_pull {
            let buffer_offset = (read * config.read_len) as u16;
            let command = OnboardReadCommand {
                buffer_offset,
                size: read_len,
            };
            write_feature_command(path, command).map_err(|e| {
                Error::DeviceCommunication(format!(
                    "onboard read request for chunk {chunk} buffer offset {buffer_offset} on \
                     {path} rejected: {e}"
                ))
            })?;
            let reply = read_reply_report(path, config.read_len, overall)?;
            let parsed = parse_read_reply_at(&reply, read_len, config.reply_header)?;
            if let Some(msg) = reply_mismatch(&parsed, buffer_offset, read_len) {
                tracing::debug!("onboard read: {msg}");
                if strict {
                    return Err(Error::DeviceCommunication(format!(
                        "strict mode: {msg} (drop --strict to accept replies that do not echo the \
                         request header)"
                    )));
                }
            }
            reads.push(parsed);
            if config.gap_ms > 0 {
                std::thread::sleep(Duration::from_millis(config.gap_ms));
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
        if overall.is_some_and(|d| Instant::now() >= d) {
            return Err(Error::DeviceCommunication(format!(
                "reply budget expired after chunk {chunk} on {path}"
            )));
        }
    }

    ProfileImage::assemble(chunks, *config).map(|image| image.with_firmware(firmware))
}

/// Sends GG's opening chain and returns the firmware version the `0x90` poll answered with.
///
/// Every frame is a 64-byte output report, in the capture's order: `90 00`, `90 01`, `f4 00`,
/// `6c 00 01`, `90 00`, `90 01`. The answers arrive on the interrupt endpoint and carry no report-ID
/// byte, so a frame that expects one is polled briefly; `6c 00 01` is not answered at all in the
/// capture. A collection that rejects a frame is logged and the chain continues: the chain is GG's,
/// and we have not measured on hardware which of its frames a keyboard requires.
fn send_init_chain(path: &str, gap_ms: u64) -> Option<String> {
    let mut version: Option<String> = None;
    for frame in ONBOARD_INIT_CHAIN {
        if let Err(e) = write_output_command(path, frame) {
            tracing::debug!("onboard opening-chain frame {} on {path} rejected: {e}", frame.command);
            continue;
        }
        if frame.expects_reply {
            if let Some(report) = read_interrupt_report(path, INIT_REPLY_LEN, CHAIN_REPLY_ATTEMPTS) {
                tracing::debug!(
                    "onboard opening-chain answer on {path}: {}",
                    hex_of(&report[..report.len().min(8)])
                );
                if matches!(frame.command, CommandCode::FirmwareVersion) {
                    version = version.or_else(|| parse_firmware_version(&report));
                }
            }
        }
        if gap_ms > 0 {
            std::thread::sleep(Duration::from_millis(gap_ms));
        }
    }
    version
}

/// Collects one payload reply: the feature-report reply the capture shows, then the interrupt
/// endpoint. Returns `[report ID][payload…]`, the shape [`parse_read_reply`] consumes.
fn read_reply_report(path: &str, payload_len: usize, overall: Option<Instant>) -> Result<Vec<u8>> {
    let budget = overall
        .map(|d| d.saturating_duration_since(Instant::now()))
        .filter(|d| !d.is_zero())
        .unwrap_or(Duration::from_millis(REPLY_BUDGET_MS));
    let stop = Instant::now() + budget;
    let mut empty_answers = 0u32;
    let mut last_error: Option<Error> = None;

    while Instant::now() < stop {
        match read_feature_report_raw(path, READ_REPLY_ID, READ_REPLY_WIRE_LEN) {
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
                last_error = Some(e);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(REPLY_POLL_MS.max(0).try_into().unwrap_or(1u64)));
    }

    let mut input_attempts = 0u32;
    while input_attempts < REPLY_INPUT_ATTEMPTS && overall.is_none_or(|d| Instant::now() < d) {
        input_attempts = input_attempts.saturating_add(1);
        match read_input_report_raw(path, INPUT_REPORT_MAX_LEN, REPLY_POLL_MS as u64) {
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

/// Reads the pull answer (the capture shows a 64-byte report on report `0x85` after each pull).
fn read_pull_ack(path: &str) -> Option<Vec<u8>> {
    read_interrupt_report(path, PULL_ACK_LEN, PULL_ACK_ATTEMPTS)
}

/// Polls the interrupt endpoint for one non-empty report of at most `len` bytes.
fn read_interrupt_report(path: &str, len: usize, attempts: u32) -> Option<Vec<u8>> {
    for _ in 0..attempts {
        match read_input_report_raw(path, len + 1, REPLY_POLL_MS as u64) {
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
fn write_output_command<C: HidCommand>(path: &str, command: C) -> Result<()> {
    let mut buffer = [0u8; ONBOARD_OUTPUT_REPORT_SIZE];
    let len = HidReportBuilder::new(HidDeviceType::Keyboard).build_report(command, &mut buffer)?;
    if len != buffer.len() {
        return Err(Error::DeviceCommunication(format!(
            "onboard output report built {len} of {} bytes",
            buffer.len()
        )));
    }
    write_output_report_raw(path, &buffer)
}

/// Serializes a command through [`HidReportBuilder`] and sends it as a feature report on report number
/// 0 (`SET_REPORT wValue 0x0300 wLength 642` in the capture).
fn write_feature_command<C: HidCommand>(path: &str, command: C) -> Result<()> {
    let mut buffer = [0u8; ONBOARD_FEATURE_REPORT_SIZE];
    let len = HidReportBuilder::new(HidDeviceType::Keyboard).build_report(command, &mut buffer)?;
    if len != buffer.len() {
        return Err(Error::DeviceCommunication(format!(
            "onboard feature report built {len} of {} bytes",
            buffer.len()
        )));
    }
    send_feature_report_raw(path, &buffer, ONBOARD_FEATURE_REPORT_SIZE)
}

/// USB interface the profile transfers run on: every `SET_REPORT` and `GET_REPORT` of the reference
/// capture carries `wIndex 1`, the control collection (`0xFFC0`) GG opens for profile traffic.
const PROFILE_INTERFACE: usize = 1;

/// Device paths to try for a profile read: the caller's preferred collection first, then the
/// [`PROFILE_INTERFACE`] collection of the same device, which is where the capture shows the profile
/// transfers running. On Windows the interface number cannot pick a collection, so
/// [`find_hidraw_for_interface`] ranks by usage page there and returns the control collection.
#[must_use]
pub fn candidate_paths(preferred: Option<&str>, vendor_id: u16, product_id: u16) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    if let Some(path) = preferred {
        paths.push(path.to_string());
    }
    if let Some(path) = find_hidraw_for_interface(vendor_id, product_id, PROFILE_INTERFACE) {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

/// `vendor:pid` of a `/dev/hidrawN` node, read from its sysfs `uevent`.
#[must_use]
pub fn hidraw_vendor_product(path: &str) -> Option<(u16, u16)> {
    let name = path.rsplit('/').next()?;
    let text = std::fs::read_to_string(format!("/sys/class/hidraw/{name}/device/uevent")).ok()?;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("HID_ID=") {
            let mut parts = value.split(':');
            let (_bus, vendor, product) = (parts.next()?, parts.next()?, parts.next()?);
            let vendor = u16::from_str_radix(vendor, 16).ok()?;
            let product = u16::from_str_radix(product, 16).ok()?;
            return Some((vendor, product));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboard_profile::{PROFILE_FS_BASE, RESOURCE_BUFFER_LEN};

    #[test]
    fn schema_config_covers_two_bytes_in_one_pull() {
        let config = schema_config(PROFILE_FS_BASE, 2, 5);
        assert_eq!(config.fs_id(), PROFILE_FS_BASE + 2);
        assert_eq!(config.total_resource_len(), SCHEMA_READ_LEN);
        // opening chain + one pull + one payload read
        assert_eq!(config.steps().len(), 8);
        config.validate().expect("schema probe plan is valid");
    }

    #[test]
    fn lighting_schema_config_addresses_the_shared_file() {
        let config = schema_config(0x0102, 0, 5);
        assert_eq!(config.fs_id(), 0x0102);
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
    fn default_read_config_matches_the_captured_geometry() {
        let config = ReadConfig::default();
        assert_eq!(config.resource_len, RESOURCE_BUFFER_LEN);
        assert_eq!(config.fs_id(), PROFILE_FS_BASE);
        assert_eq!(config.total_resource_len(), 12_288);
        let steps = config.steps();
        let pulls = steps.iter().filter(|s| s.kind.to_string() == "pull").count();
        assert_eq!(pulls, config.chunks);
    }
}
