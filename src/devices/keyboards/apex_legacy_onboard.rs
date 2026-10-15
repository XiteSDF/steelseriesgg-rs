//! [EXPERIMENTAL] Hardware read loop for the legacy Apex on-board profile.
//!
//! Everything in this file talks to a keyboard. The wire format, the read plan, reply parsing, decode
//! and JSON live in [`crate::onboard_profile`], and the notes, including what the reference capture
//! does and does not prove, live in docs/development/apex-7-onboard-profile-read.md.
//!
//! Status: reverse-engineered, never replayed on hardware from this crate. The sequence is read-only:
//! the pacing frame, pulls and payload reads. Nothing in this path writes a profile, and the write
//! command of this protocol family is deliberately not implemented here.

use crate::devices::{
    list_hidraw_paths, read_feature_report_raw, read_input_report_raw, send_feature_report_raw, write_output_report_raw,
};
use crate::onboard_profile::{
    ChunkRecord, PULL_REPLY_REPORT_ID, ProfileImage, READ_REPLY_HEADER_LEN, READ_REPLY_REPORT_ID, READ_REPORT_LEN,
    ReadConfig, Step, StepKind, hex_of, parse_read_reply, reply_mismatch,
};
use crate::{Error, Result};
use std::time::Duration;
use tokio::time::sleep;

/// Attempts for one transfer on one device path before that path is given up on.
pub const STEP_ATTEMPTS: u8 = 3;
/// Device paths tried for one read: discovery's control collection first, then the other collections
/// of the same vendor/product pair.
pub const PATH_ATTEMPTS: usize = 4;
/// Time to wait for a pull answer. GG's own pacing round-trips in about 5 ms; this is the ceiling, not
/// the target.
pub const ACK_TIMEOUT_MS: u64 = 300;

/// Read the vendor and product of a hidraw node from its sysfs `uevent` (`HID_ID=0003:000001038:000001612`).
///
/// Used when `--device` names a hidraw node directly, so the fallback list can be built.
#[cfg(unix)]
pub fn hidraw_vendor_product(hidraw_path: &str) -> Option<(u16, u16)> {
    let name = std::path::Path::new(hidraw_path).file_name()?.to_str()?;
    let uevent = std::fs::read_to_string(format!("/sys/class/hidraw/{name}/device/uevent")).ok()?;
    parse_hid_id_uevent(&uevent)
}

/// Extract vendor and product from a sysfs `uevent` text's `HID_ID` line.
pub fn parse_hid_id_uevent(text: &str) -> Option<(u16, u16)> {
    let line = text.lines().find(|line| line.starts_with("HID_ID="))?;
    let value = line.trim_start_matches("HID_ID=");
    let mut parts = value.split(':');
    let _bus = parts.next()?;
    let vendor = u32::from_str_radix(parts.next()?, 16).ok()?;
    let product = u32::from_str_radix(parts.next()?, 16).ok()?;
    Some((vendor as u16, product as u16))
}

/// Paths to try for a profile read: `preferred` (discovery's control collection) first, then every
/// other collection of the same vendor/product pair, in the order [`list_hidraw_paths`] ranks them.
pub fn candidate_paths(preferred: Option<&str>, vendor_id: u16, product_id: u16) -> Vec<String> {
    let mut paths: Vec<String> = Vec::with_capacity(PATH_ATTEMPTS);
    if let Some(path) = preferred {
        paths.push(path.to_string());
    }
    for path in list_hidraw_paths(vendor_id, product_id) {
        if paths.len() >= PATH_ATTEMPTS {
            break;
        }
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

/// Report buffer for a request: report ID byte, frame bytes, zero padding to the wire length.
fn request_buffer(step: &Step) -> Vec<u8> {
    let mut report = vec![0u8; step.wire_len];
    let take = step.frame.len().min(step.wire_len - 1);
    report[1..1 + take].copy_from_slice(&step.frame[..take]);
    report
}

/// Send one transfer and read its reply when the protocol defines one, on one path.
///
/// Returns the reply report (report ID byte kept) for `Pull` and `Read` steps and an empty vector for
/// the pacing frame, which the official engine sends without reading anything back.
fn exchange(path: &str, step: &Step) -> Result<Vec<u8>> {
    let report = request_buffer(step);

    match step.kind {
        StepKind::Config => {
            send_feature_report_raw(path, &report, step.wire_len)?;
            Ok(Vec::new())
        }
        StepKind::Pull => {
            // The pull is an output report: report ID byte plus 64 payload bytes.
            write_output_report_raw(path, &report)?;
            let want = step.reply_len.unwrap_or(64);
            let reply = read_input_report_raw(path, want, ACK_TIMEOUT_MS)?;
            if reply.is_empty() {
                return Err(Error::DeviceCommunication(format!(
                    "no pull answer on {path} within {ACK_TIMEOUT_MS} ms"
                )));
            }
            Ok(reply)
        }
        StepKind::Read => {
            send_feature_report_raw(path, &report, step.wire_len)?;
            let want = step.reply_len.unwrap_or(READ_REPORT_LEN);
            let reply = read_feature_report_raw(path, READ_REPLY_REPORT_ID, want)?;
            if reply.len() <= 1 + READ_REPLY_HEADER_LEN {
                return Err(Error::DeviceCommunication(format!(
                    "payload read on {path} returned {} bytes, no payload behind the reply header",
                    reply.len()
                )));
            }
            Ok(reply)
        }
    }
}

/// One transfer with retries.
///
/// In `strict` mode a payload-read reply whose header does not echo the request, and a pull answer on
/// the wrong report ID, count as failed attempts. In lenient mode both are accepted and only tallied
/// by [`ProfileImage::assemble`].
async fn step_with_retries(path: &str, step: &Step, gap_ms: u64, strict: bool) -> Result<Vec<u8>> {
    let mut last_error = None::<String>;

    for attempt in 1..=STEP_ATTEMPTS {
        let task_path = path.to_string();
        let task_step = step.clone();

        let outcome = tokio::task::spawn_blocking(move || exchange(&task_path, &task_step))
            .await
            .map_err(|e| Error::DeviceCommunication(format!("HID transfer task failed: {e}")))?;

        match outcome {
            Ok(reply) => {
                let rejected = if step.kind == StepKind::Read && strict {
                    let parsed = parse_read_reply(&reply, step.size)?;
                    reply_mismatch(&parsed, step.buffer_offset.unwrap_or(0), step.size)
                } else if step.kind == StepKind::Pull && strict && reply[0] != PULL_REPLY_REPORT_ID {
                    Some(format!(
                        "pull answer arrived on report 0x{:02X}, expected 0x{PULL_REPLY_REPORT_ID:02X}",
                        reply[0]
                    ))
                } else {
                    None
                };

                match rejected {
                    None => return Ok(reply),
                    Some(mismatch) => {
                        tracing::warn!("chunk {} on {path}: {mismatch} (attempt {attempt})", step.chunk);
                        last_error = Some(mismatch);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "chunk {} {} on {path} failed (attempt {attempt}): {e}",
                    step.chunk,
                    step.kind
                );
                last_error = Some(e.to_string());
            }
        }

        sleep(Duration::from_millis(gap_ms)).await;
    }

    Err(Error::DeviceCommunication(format!(
        "chunk {} on {path} did not yield a usable reply after {} attempts: {}",
        step.chunk,
        STEP_ATTEMPTS,
        last_error.unwrap_or_else(|| "no error recorded".to_string())
    )))
}

/// Read one profile file from one device path.
async fn read_on_path(path: &str, config: &ReadConfig, strict: bool) -> Result<ProfileImage> {
    let steps = config.steps();
    let mut records: Vec<ChunkRecord> = Vec::with_capacity(config.chunks);

    for chunk in 0..config.chunks {
        let mut ack: Vec<u8> = Vec::new();
        let mut reads: Vec<crate::onboard_profile::ReadReply> =
            Vec::with_capacity(config.resource_len / config.read_len);

        for step in steps.iter().filter(|s| s.chunk == chunk) {
            let reply = step_with_retries(path, step, config.gap_ms, strict).await?;
            match step.kind {
                StepKind::Pull => {
                    ack = reply;
                }
                StepKind::Read => {
                    reads.push(parse_read_reply(&reply, step.size)?);
                }
                StepKind::Config => {}
            }
            sleep(Duration::from_millis(config.gap_ms)).await;
        }

        records.push(ChunkRecord {
            index: chunk,
            fs_id: config.fs_id(),
            file_offset: (chunk * config.resource_len) as u32,
            ack_report_id: ack.first().copied().unwrap_or(0),
            ack_hex: hex_of(&ack[..ack.len().min(16)]),
            reads,
        });
    }

    ProfileImage::assemble(records, *config)
}

/// Read one profile, trying each candidate path until one answers the protocol.
///
/// Returns the assembled image and the path it came from.
pub async fn read_profile(paths: &[String], config: &ReadConfig, strict: bool) -> Result<(ProfileImage, String)> {
    if paths.is_empty() {
        return Err(Error::DeviceNotFound(
            "no device path available for the on-board profile read".to_string(),
        ));
    }
    config.validate()?;

    let mut last_error = None::<Error>;
    for path in paths.iter().take(PATH_ATTEMPTS) {
        match read_on_path(path, config, strict).await {
            Ok(image) => {
                tracing::info!("read the on-board profile from {path}");
                return Ok((image, path.clone()));
            }
            Err(e) => {
                tracing::warn!("on-board profile read on {path} failed: {e}");
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| Error::DeviceCommunication("the on-board profile read never ran".to_string())))
}

/// Read a profile twice and compare the profile bytes.
///
/// Returns both images, the offset of the first differing byte (`None` when identical) and the path
/// they were read from.
pub async fn read_profile_verify(
    paths: &[String],
    config: &ReadConfig,
    strict: bool,
) -> Result<(ProfileImage, ProfileImage, Option<usize>, String)> {
    let (first, path) = read_profile(paths, config, strict).await?;
    sleep(Duration::from_millis(config.gap_ms)).await;
    let (second, _) = read_profile(std::slice::from_ref(&path), config, strict).await?;
    let diff = crate::onboard_profile::image_diff(&first, &second);
    if diff.is_some() {
        tracing::warn!("the two reads of the on-board profile differ");
    }
    Ok((first, second, diff, path))
}

/// Highest profile slot this command probes: GG's `profileCount` for the legacy family is 5.
pub const MAX_PROFILE_INDEX_FOR_READ: u8 = crate::onboard_profile::MAX_PROFILE_INDEX;

/// Read only the schema word of one profile slot.
///
/// Returns the schema word, the raw two bytes and the path they came from.
pub async fn read_schema(paths: &[String], profile: u8, gap_ms: u64, strict: bool) -> Result<(u16, [u8; 2], String)> {
    if profile > MAX_PROFILE_INDEX_FOR_READ {
        return Err(Error::InvalidConfig(format!(
            "profile slot {profile} is above the highest slot {MAX_PROFILE_INDEX_FOR_READ}"
        )));
    }
    read_schema_file(
        paths,
        crate::onboard_profile::PROFILE_FS_BASE + u16::from(profile),
        gap_ms,
        strict,
    )
    .await
}

/// Read only the schema word of any file: one 2-byte pull and one 2-byte payload read.
///
/// The schema word is a length in GG's code, so a 2-byte read is the smallest thing the protocol
/// defines; this is what `ssgg onboard schema` uses to learn the file map of a keyboard.
pub async fn read_schema_file(
    paths: &[String],
    fs_id: u16,
    gap_ms: u64,
    strict: bool,
) -> Result<(u16, [u8; 2], String)> {
    if paths.is_empty() {
        return Err(Error::DeviceNotFound(
            "no device path available for the on-board profile schema read".to_string(),
        ));
    }

    let steps = crate::onboard_profile::schema_read_steps(fs_id);
    let mut last_error = None::<Error>;

    for path in paths.iter().take(PATH_ATTEMPTS) {
        let mut schema_bytes = [0u8; 2];
        let mut ok = true;
        for step in &steps {
            match step_with_retries(path, step, gap_ms, strict).await {
                Ok(reply) => {
                    if step.kind == StepKind::Read {
                        let parsed = parse_read_reply(&reply, step.size)?;
                        schema_bytes.copy_from_slice(&parsed.data[..2]);
                    }
                }
                Err(e) => {
                    tracing::warn!("schema read on {path} failed: {e}");
                    last_error = Some(e);
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return Ok((u16::from_le_bytes(schema_bytes), schema_bytes, path.clone()));
        }
    }

    Err(last_error.unwrap_or_else(|| Error::DeviceCommunication("the schema read never completed".to_string())))
}
