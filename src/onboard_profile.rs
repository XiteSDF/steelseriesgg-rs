//! [EXPERIMENTAL] On-board profile read-back for the legacy/"macho" Apex platform.
//!
//! This module is the pure half of the read path: frame builders, the read sequence plan, reply
//! parsing, payload assembly, struct decode, checksum reporting and JSON. It performs no I/O. The
//! hardware loop is [`crate::devices::keyboards::apex_legacy_onboard`] (feature `onboard`).
//!
//! Protocol status: reverse-engineered. The file ids, chunk geometry, frame layouts and the payload
//! struct layout below are reproduced from the Windows engine's device descriptors, and every one of
//! them is visible in one Windows USBPcap capture of the official GG talking to an Apex 7 (firmware
//! 4.16.8, 2026-10-15, `ssgg-capture-1612-20261015-004400`): the capture contains 121 pulls
//! (report `0x85`, 64-byte replies) and 241 payload reads (report `0x83`, 642-byte replies) that form
//! the `pull, read, read` pattern described here. On 2026-10-15 an Apex 7 accepted the pacing frame, the
//! pull and the payload-read request on its vendor collection, and answered on the interrupt endpoint; the
//! full read has not completed on hardware, and nothing here writes a profile. Treat every constant as
//! unverified until the first Linux read-back.
//!
//! # Wire format
//!
//! All transfers run on the keyboard's vendor collection (the Apex 7 exposes it as usage page `0xffc1` on
//! USB interface 4, and usage page `0xffc0` on interface 1 holds the same protocol family). Report numbers
//! are embedded in the payload, so in `hidapi` terms the first byte of every buffer is the report ID.
//!
//! * pacing (`HIDCONFIG`, feature report ID `0x00`, 12 bytes on the wire):
//!   `00 | time_between_commands u32 LE | file_access u32 LE | max_usb_timeout u32 LE`,
//!   GG reads a profile with `1 | 2 | 500`.
//! * pull (`HIDIO`, output report ID `0x00`, 65 bytes on the wire, i.e. a 64-byte output report):
//!   `00 | 85 | 00 | file_id u16 LE | offset u32 LE | size u16 LE | 00…`
//!   — copies `size` bytes of the profile file into the device's resource buffer at buffer offset 0.
//!   The keyboard answers with input report `0x85` (64 bytes, contents not interpreted here).
//! * payload read (`HIDFEATURE`, feature report ID `0x00`, 643 bytes on the wire):
//!   `00 | 83 | 00 | offset u16 LE | size u16 LE | 00…`
//!   — returns `size` bytes of the resource buffer. The keyboard answers with a report that carries
//!   `offset` and `size` back in its header, payload from byte 6: on Linux that answer arrives as one
//!   input report (the collection declares a 1024-byte maximum input report); the reference capture shows
//!   it as a 642-byte control answer.
//!
//! A profile file is read as `ceil((payload_len + 1023) / 1024)` resource-buffer fills
//! ([`RESOURCE_BUFFER_LEN`] each); every fill is drained with two [`READ_LEN`]-byte payload reads at
//! buffer offsets 0 and 512. The profile files are addressed by id: [`PROFILE_FS_BASE`] plus the
//! profile index (`0..=4`).
//!
//! The first two payload bytes are the profile schema, so a schema-only read is one pull of 2 bytes
//! followed by one payload read of 2 bytes ([`schema_read_steps`]). Firmware `4.16.3` selects schema
//! [`SCHEMA_B`]; older firmware uses [`SCHEMA_A`].
//!
//! The payload starts at file offset 0 with the schema word; the engine's payload structs carry one
//! leading constant id byte that is not part of the file, so `dest` in [`Step`] is the file offset and
//! the assembled image is [`ReadConfig::payload_len`] bytes with no id byte.

use crate::devices::keyboards::apex_pro_tkl_2023_profile::profile_checksum;
use crate::error::{Error, Result};
use serde::Serialize;
use serde_json::json;

/// Schema the pre-4.16.3 firmware uses.
pub const SCHEMA_A: u16 = 0x000A;
/// Schema used from firmware `4.16.3` (adds the Rapid Tap block).
pub const SCHEMA_B: u16 = 0x000B;
/// Payload length of schema [`SCHEMA_A`] profiles, in bytes.
pub const PAYLOAD_LEN_A: usize = 11_648;
/// Payload length of schema [`SCHEMA_B`] profiles, in bytes.
pub const PAYLOAD_LEN_B: usize = 11_696;
/// Firmware that switches the on-board profile schema to [`SCHEMA_B`].
pub const SCHEMA_B_FIRMWARE: &str = "4.16.3";
/// On-board resource buffer the pull command fills, in bytes.
pub const RESOURCE_BUFFER_LEN: usize = 1024;
/// Bytes taken from the resource buffer per payload read.
pub const READ_LEN: usize = 512;
/// Pulls needed to cover one schema [`SCHEMA_A`] payload.
pub const DEFAULT_CHUNKS: usize = 12;
/// Full on-board profile region in flash, in bytes.
pub const PROFILE_FS_LEN: usize = 0xA000;
/// File id of profile slot 0; slot `n` is this plus `n`.
pub const PROFILE_FS_BASE: u16 = 0x0201;
/// Highest profile slot.
pub const MAX_PROFILE_INDEX: u8 = 4;
/// File id of the per-profile lighting config of slot 0.
pub const LIGHTING_FS_BASE: u16 = 0x0301;
/// Shared lighting config file id.
pub const LIGHTING_FS_SHARED: u16 = 0x0102;
/// Macro data block inside a profile payload, in bytes.
pub const MACRO_DATA_LEN: usize = 9600;

/// Command byte of the payload read (`HIDFEATURE`).
pub const CMD_READ_PAYLOAD: u8 = 0x83;
/// Command byte of the pull (`HIDIO`).
pub const CMD_PULL_PAYLOAD: u8 = 0x85;
/// Command byte of the push (profile write); defined for completeness, nothing here sends it.
pub const CMD_PUSH_PAYLOAD: u8 = 0x05;
/// Report ID of every request frame.
pub const REQUEST_REPORT_ID: u8 = 0x00;
/// Report ID of a payload-read reply.
pub const READ_REPLY_REPORT_ID: u8 = 0x83;
/// Report ID of a pull reply.
pub const PULL_REPLY_REPORT_ID: u8 = 0x85;
/// Report ID of a lighting-config payload-read reply.
pub const LIGHTING_READ_REPLY_REPORT_ID: u8 = 0x03;
/// Feature report length of a payload read, report ID byte included.
pub const READ_REPORT_LEN: usize = 643;
/// Output report length of a pull, report ID byte included.
pub const PULL_REPORT_LEN: usize = 65;
/// Feature report length of the pacing frame.
pub const CONFIG_REPORT_LEN: usize = 12;
/// Header of a payload-read reply, report ID byte excluded: `reserved, offset u16, size u16`.
pub const READ_REPLY_HEADER_LEN: usize = 5;
/// Frame length of a pull command, report ID byte excluded.
pub const PULL_FRAME_LEN: usize = 11;
/// Frame length of a payload-read command, report ID byte excluded.
pub const READ_FRAME_LEN: usize = 7;
/// `file_access` value GG uses for a profile read (read/write file access).
pub const FILE_ACCESS_READWRITE: u32 = 2;
/// `max_usb_timeout` value GG uses for a profile read, in milliseconds.
pub const MAX_USB_TIMEOUT_MS: u32 = 500;
/// `time_between_commands` value GG uses for a profile read, in milliseconds.
pub const TIME_BETWEEN_COMMANDS_MS: u32 = 1;
/// Median gap between two device replies during a GG profile read in the reference capture, in
/// milliseconds. Used as the default pacing between our own transfers.
pub const GG_GAP_MS: u64 = 5;

/// Largest payload a payload-read reply can carry: the reply report minus the report ID byte and the
/// reply header.
pub const MAX_READ_LEN: usize = READ_REPORT_LEN - 1 - READ_REPLY_HEADER_LEN;

/// One entry of the read sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    /// Transfer kind.
    pub kind: StepKind,
    /// Pull this step belongs to (`0` for the pacing frame).
    pub chunk: usize,
    /// Frame bytes after the report ID byte.
    pub frame: Vec<u8>,
    /// Total report length on the wire, report ID byte included.
    pub wire_len: usize,
    /// Report ID expected in the reply, `None` for transfers GG does not read back.
    pub reply_report_id: Option<u8>,
    /// Reply report length to read, report ID byte included.
    pub reply_len: Option<usize>,
    /// File offset the payload of a read lands at; `None` for pulls and the pacing frame.
    pub dest: Option<usize>,
    /// Resource-buffer offset a read asks for; `None` for pulls and the pacing frame.
    pub buffer_offset: Option<u16>,
    /// Payload size requested by a read.
    pub size: u16,
}

/// Transfer kind of a [`Step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Pacing frame (feature report ID `0x00`, 12 bytes).
    Config,
    /// Pull into the resource buffer (output report, 65 bytes).
    Pull,
    /// Payload read out of the resource buffer (feature report, 643 bytes).
    Read,
}

impl std::fmt::Display for StepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            StepKind::Config => "config",
            StepKind::Pull => "pull",
            StepKind::Read => "read",
        };
        f.write_str(name)
    }
}

/// Parameters of one profile read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReadConfig {
    /// Profile slot `0..=4`.
    pub profile: u8,
    /// Base file id; [`PROFILE_FS_BASE`] for profiles.
    pub fs_base: u16,
    /// Pulls to issue.
    pub chunks: usize,
    /// Resource buffer size per pull.
    pub resource_len: usize,
    /// Bytes per payload read.
    pub read_len: usize,
    /// Payload length to assemble.
    pub payload_len: usize,
    /// Pacing between our transfers, in milliseconds.
    pub gap_ms: u64,
    /// `time_between_commands` for the pacing frame, in milliseconds.
    pub time_between_commands_ms: u32,
    /// `file_access` for the pacing frame.
    pub file_access: u32,
    /// `max_usb_timeout` for the pacing frame, in milliseconds.
    pub max_usb_timeout_ms: u32,
    /// Send the pacing frame before the pull loop. GG always sends it; turning it off is a probe.
    pub send_config: bool,
}

impl Default for ReadConfig {
    fn default() -> Self {
        Self {
            profile: 0,
            fs_base: PROFILE_FS_BASE,
            chunks: DEFAULT_CHUNKS,
            resource_len: RESOURCE_BUFFER_LEN,
            read_len: READ_LEN,
            payload_len: PAYLOAD_LEN_A,
            gap_ms: GG_GAP_MS,
            time_between_commands_ms: TIME_BETWEEN_COMMANDS_MS,
            file_access: FILE_ACCESS_READWRITE,
            max_usb_timeout_ms: MAX_USB_TIMEOUT_MS,
            send_config: true,
        }
    }
}

impl ReadConfig {
    /// File id of the profile being read.
    pub fn fs_id(&self) -> u16 {
        self.fs_base.wrapping_add(u16::from(self.profile))
    }

    /// File bytes covered by the planned pulls.
    pub fn total_resource_len(&self) -> usize {
        self.chunks * self.resource_len
    }

    /// Reject configurations that cannot produce a complete payload or cannot fit a reply.
    pub fn validate(&self) -> Result<()> {
        if self.profile > MAX_PROFILE_INDEX {
            return Err(Error::InvalidConfig(format!(
                "profile slot {} is above the highest slot {MAX_PROFILE_INDEX}",
                self.profile
            )));
        }
        if self.resource_len == 0 || self.read_len == 0 {
            return Err(Error::InvalidConfig(
                "resource length and read length must be non-zero".to_string(),
            ));
        }
        if self.read_len > self.resource_len {
            return Err(Error::InvalidConfig(format!(
                "read length {} exceeds the resource buffer length {}",
                self.read_len, self.resource_len
            )));
        }
        let per_pull = self.resource_len / self.read_len;
        if per_pull * self.read_len != self.resource_len {
            return Err(Error::InvalidConfig(format!(
                "the {} resource buffer is not a whole number of {} reads",
                self.resource_len, self.read_len
            )));
        }
        if self.read_len > MAX_READ_LEN {
            return Err(Error::InvalidConfig(format!(
                "read length {} exceeds the {} bytes a reply report can carry",
                self.read_len, MAX_READ_LEN
            )));
        }
        if self.payload_len == 0 {
            return Err(Error::InvalidConfig("payload length must be non-zero".to_string()));
        }
        if self.total_resource_len() < self.payload_len {
            return Err(Error::InvalidConfig(format!(
                "{} pulls of {} bytes cover {}, short of the {} payload bytes",
                self.chunks,
                self.resource_len,
                self.total_resource_len(),
                self.payload_len
            )));
        }
        Ok(())
    }

    /// Full transfer sequence for one profile read, in wire order.
    pub fn steps(&self) -> Vec<Step> {
        let mut steps = Vec::with_capacity(1 + self.chunks * (1 + self.resource_len / self.read_len));
        if self.send_config {
            steps.push(Step {
                kind: StepKind::Config,
                chunk: 0,
                frame: config_frame(self.time_between_commands_ms, self.file_access, self.max_usb_timeout_ms).to_vec(),
                wire_len: CONFIG_REPORT_LEN,
                reply_report_id: None,
                reply_len: None,
                dest: None,
                buffer_offset: None,
                size: 0,
            });
        }
        let reads_per_pull = self.resource_len / self.read_len;
        for chunk in 0..self.chunks {
            let file_offset = chunk * self.resource_len;
            steps.push(Step {
                kind: StepKind::Pull,
                chunk,
                frame: pull_frame(self.fs_id(), file_offset as u32, self.resource_len as u16).to_vec(),
                wire_len: PULL_REPORT_LEN,
                reply_report_id: Some(PULL_REPLY_REPORT_ID),
                reply_len: Some(64),
                dest: None,
                buffer_offset: None,
                size: self.resource_len as u16,
            });
            for read in 0..reads_per_pull {
                let buffer_offset = read * self.read_len;
                steps.push(Step {
                    kind: StepKind::Read,
                    chunk,
                    frame: read_frame(buffer_offset as u16, self.read_len as u16).to_vec(),
                    wire_len: READ_REPORT_LEN,
                    reply_report_id: Some(READ_REPLY_REPORT_ID),
                    reply_len: Some(READ_REPORT_LEN),
                    dest: Some(file_offset + buffer_offset),
                    buffer_offset: Some(buffer_offset as u16),
                    size: self.read_len as u16,
                });
            }
        }
        steps
    }
}

/// Transfer sequence for a schema-only read: one pull of 2 bytes, one payload read of 2 bytes.
pub fn schema_read_steps(fs_id: u16) -> Vec<Step> {
    vec![
        Step {
            kind: StepKind::Config,
            chunk: 0,
            frame: config_frame(TIME_BETWEEN_COMMANDS_MS, FILE_ACCESS_READWRITE, MAX_USB_TIMEOUT_MS).to_vec(),
            wire_len: CONFIG_REPORT_LEN,
            reply_report_id: None,
            reply_len: None,
            dest: None,
            buffer_offset: None,
            size: 0,
        },
        Step {
            kind: StepKind::Pull,
            chunk: 0,
            frame: pull_frame(fs_id, 0, 2).to_vec(),
            wire_len: PULL_REPORT_LEN,
            reply_report_id: Some(PULL_REPLY_REPORT_ID),
            reply_len: Some(64),
            dest: None,
            buffer_offset: None,
            size: 2,
        },
        Step {
            kind: StepKind::Read,
            chunk: 0,
            frame: read_frame(0, 2).to_vec(),
            wire_len: READ_REPORT_LEN,
            reply_report_id: Some(READ_REPLY_REPORT_ID),
            reply_len: Some(READ_REPORT_LEN),
            dest: Some(0),
            buffer_offset: Some(0),
            size: 2,
        },
    ]
}

/// Pacing frame: `time_between_commands, file_access, max_usb_timeout`, three little-endian words.
pub fn config_frame(time_between_commands_ms: u32, file_access: u32, max_usb_timeout_ms: u32) -> [u8; 12] {
    let mut frame = [0u8; CONFIG_REPORT_LEN];
    frame[0..4].copy_from_slice(&time_between_commands_ms.to_le_bytes());
    frame[4..8].copy_from_slice(&file_access.to_le_bytes());
    frame[8..12].copy_from_slice(&max_usb_timeout_ms.to_le_bytes());
    frame
}

/// Pull frame: `85, reserved, file_id u16, file offset u32, size u16`.
pub fn pull_frame(fs_id: u16, file_offset: u32, size: u16) -> [u8; 11] {
    let mut frame = [0u8; PULL_FRAME_LEN];
    frame[0] = CMD_PULL_PAYLOAD;
    frame[2..4].copy_from_slice(&fs_id.to_le_bytes());
    frame[4..8].copy_from_slice(&file_offset.to_le_bytes());
    frame[8..10].copy_from_slice(&size.to_le_bytes());
    frame
}

/// Payload-read frame: `83, reserved, resource-buffer offset u16, size u16`.
pub fn read_frame(buffer_offset: u16, size: u16) -> [u8; 7] {
    let mut frame = [0u8; READ_FRAME_LEN];
    frame[0] = CMD_READ_PAYLOAD;
    frame[2..4].copy_from_slice(&buffer_offset.to_le_bytes());
    frame[4..6].copy_from_slice(&size.to_le_bytes());
    frame
}

/// Parsed header of a payload-read reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadReply {
    /// Report ID the reply arrived on.
    pub report_id: u8,
    /// Reserved header byte behind the report ID.
    pub reserved: u8,
    /// Buffer offset the keyboard echoes.
    pub offset: u16,
    /// Size the keyboard echoes.
    pub size: u16,
    /// Payload bytes extracted from the reply.
    pub data: Vec<u8>,
    /// True when every payload byte is zero.
    pub all_zero: bool,
    /// True when the payload starts with four `FF` bytes: the reference implementation treats such a page as
    /// garbage and replaces it with zeroes.
    pub all_ff: bool,
}

/// Parse a payload-read reply (`reply` starts with the report ID byte) and extract its payload.
///
/// `size` is the size that was requested; the payload is taken from the reply at
/// [`READ_REPLY_HEADER_LEN`] and truncated to the requested size when the report carries more.
pub fn parse_read_reply(reply: &[u8], requested_size: u16) -> Result<ReadReply> {
    if reply.len() < 1 + READ_REPLY_HEADER_LEN {
        return Err(Error::Profile(format!(
            "payload-read reply is {} bytes, shorter than the {}-byte report header",
            reply.len(),
            1 + READ_REPLY_HEADER_LEN
        )));
    }
    let offset = u16::from_le_bytes([reply[2], reply[3]]);
    let size = u16::from_le_bytes([reply[4], reply[5]]);
    let end = (1 + READ_REPLY_HEADER_LEN + usize::from(requested_size))
        .min(reply.len())
        .min(1 + READ_REPLY_HEADER_LEN + MAX_READ_LEN);
    let data = reply[1 + READ_REPLY_HEADER_LEN..end].to_vec();
    let all_zero = data.iter().all(|b| *b == 0);
    let all_ff = data.len() >= 4 && data[..4].iter().all(|b| *b == 0xFF);
    Ok(ReadReply {
        report_id: reply[0],
        reserved: reply[1],
        offset,
        size,
        data,
        all_zero,
        all_ff,
    })
}

/// One pull and the payload reads drained from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChunkRecord {
    /// Pull index.
    pub index: usize,
    /// File id the pull addressed.
    pub fs_id: u16,
    /// File offset the pull filled the resource buffer from.
    pub file_offset: u32,
    /// Report ID the pull answer arrived on.
    pub ack_report_id: u8,
    /// First bytes of the pull answer, hex; its layout is not known.
    pub ack_hex: String,
    /// Payload reads drained from this pull.
    pub reads: Vec<ReadReply>,
}

impl ChunkRecord {
    /// Payload bytes collected by this pull.
    pub fn data_len(&self) -> usize {
        self.reads.iter().map(|r| r.data.len()).sum()
    }

    /// True when every payload read of this pull returned only zeros.
    pub fn all_zero(&self) -> bool {
        !self.reads.is_empty() && self.reads.iter().all(|r| r.all_zero)
    }
}

/// Header disagreement between a request and its reply.
pub fn reply_mismatch(reply: &ReadReply, wanted_offset: u16, wanted_size: u16) -> Option<String> {
    if reply.report_id != READ_REPLY_REPORT_ID {
        return Some(format!(
            "reply arrived on report 0x{:02X}, expected 0x{:02X}",
            reply.report_id, READ_REPLY_REPORT_ID
        ));
    }
    if reply.offset != wanted_offset {
        return Some(format!(
            "reply echoes offset 0x{:04X}, the request asked for 0x{:04X}",
            reply.offset, wanted_offset
        ));
    }
    if reply.size != wanted_size {
        return Some(format!(
            "reply echoes size {}, the request asked for {}",
            reply.size, wanted_size
        ));
    }
    None
}

/// Aggregated statistics of one profile read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadStats {
    /// Pulls collected.
    pub chunks_read: usize,
    /// Payload reads collected.
    pub reads_read: usize,
    /// Data bytes in the assembled image.
    pub data_len: usize,
    /// Reads whose reply header did not echo the request.
    pub header_mismatches: usize,
    /// Reads whose payload was entirely zero.
    pub zero_reads: usize,
    /// Pulls whose payload was entirely zero.
    pub zero_chunks: usize,
    /// Reads that returned fewer bytes than requested.
    pub short_reads: usize,
    /// Reads whose payload started with four `FF` bytes and was stored as zeroes.
    pub garbage_reads: usize,
    /// File bytes covered by the pulls.
    pub covered_len: usize,
}

/// Assembled profile data with its per-pull records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileImage {
    /// Configuration the read ran with.
    pub config: ReadConfig,
    /// Per-pull records, in collection order.
    pub chunks: Vec<ChunkRecord>,
    /// Profile file bytes, reads concatenated in pull order.
    pub data: Vec<u8>,
    /// Aggregated reply statistics.
    pub stats: ReadStats,
}

impl ProfileImage {
    /// Assemble collected records into the profile image and tally reply statistics.
    ///
    /// Reads that land past [`ReadConfig::payload_len`] are kept in the records and dropped from the
    /// assembled data: the last pull always covers more than the payload needs.
    pub fn assemble(chunks: Vec<ChunkRecord>, config: ReadConfig) -> Result<Self> {
        config.validate()?;

        let mut data = vec![0u8; config.payload_len];
        let mut reads_read = 0usize;
        let mut header_mismatches = 0usize;
        let mut zero_reads = 0usize;
        let mut zero_chunks = 0usize;
        let mut short_reads = 0usize;
        let mut garbage_reads = 0usize;

        for record in &chunks {
            if record.all_zero() {
                zero_chunks += 1;
            }
            let mut cursor = record.file_offset as usize;
            for reply in &record.reads {
                reads_read += 1;
                if reply_mismatch(
                    reply,
                    (cursor - record.file_offset as usize) as u16,
                    config.read_len as u16,
                )
                .is_some()
                {
                    header_mismatches += 1;
                }
                if reply.all_zero {
                    zero_reads += 1;
                }
                if reply.data.len() < config.read_len {
                    short_reads += 1;
                }
                if reply.data.is_empty() {
                    continue;
                }
                if cursor >= config.payload_len {
                    continue;
                }
                let end = cursor + reply.data.len();
                if reply.all_ff {
                    // A page whose first four bytes are FF is garbage from the device's point of view; the
                    // reference implementation replaces it with zeroes and so do we.
                    garbage_reads += 1;
                    let room = config.payload_len.saturating_sub(cursor);
                    data[cursor..cursor + room].fill(0);
                    cursor += room;
                } else if end > config.payload_len {
                    let room = config.payload_len.saturating_sub(cursor);
                    data[cursor..cursor + room].copy_from_slice(&reply.data[..room]);
                    cursor += room;
                } else {
                    data[cursor..end].copy_from_slice(&reply.data);
                    cursor = end;
                }
            }
        }

        let covered_len = chunks
            .iter()
            .map(|r| r.file_offset as usize + r.data_len())
            .max()
            .unwrap_or(0);

        Ok(Self {
            config,
            stats: ReadStats {
                chunks_read: chunks.len(),
                reads_read,
                data_len: data.len(),
                header_mismatches,
                zero_reads,
                zero_chunks,
                short_reads,
                garbage_reads,
                covered_len,
            },
            chunks,
            data,
        })
    }

    /// Schema word of the assembled profile.
    pub fn schema(&self) -> u16 {
        schema_of(&self.data)
    }

    /// Offset of the first byte that differs from another read of the same profile.
    pub fn diff(&self, other: &Self) -> Option<usize> {
        image_diff(self, other)
    }
}

/// Schema word of a profile payload (`0` when the payload is shorter than the word).
pub fn schema_of(data: &[u8]) -> u16 {
    if data.len() < 2 {
        return 0;
    }
    u16::from_le_bytes([data[0], data[1]])
}

/// Payload length for a schema word; unknown schemas report the length they cannot be decoded at.
pub fn payload_len_for_schema(schema: u16) -> Result<usize> {
    match schema {
        SCHEMA_A => Ok(PAYLOAD_LEN_A),
        SCHEMA_B => Ok(PAYLOAD_LEN_B),
        other => Err(Error::Profile(format!(
            "profile schema 0x{other:04X} is not one of the supported schemas 0x{SCHEMA_A:04X} / 0x{SCHEMA_B:04X}"
        ))),
    }
}

/// Offset of the trailing checksum word and the length of the region it covers.
pub fn checksum_offset(payload_len: usize) -> usize {
    payload_len - 4
}

/// Compute the profile checksum the way the 2023-family profile blob is checksummed: STM32 hardware
/// CRC-32 over the payload prefix read as little-endian 32-bit words. Assumed, not verified on Apex 7.
pub fn compute_checksum(data: &[u8]) -> u32 {
    profile_checksum(data)
}

/// One `key_mapping` entry: a function selector plus its four key codes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyMapping {
    /// Function selector (`0x00` = key deactivated, `0x71` = macro, `0x62` = META, …).
    pub function: u8,
    /// Function selector as a name, for the human-readable summary only.
    pub function_name: String,
    /// Four raw key-code bytes, hex.
    pub key_codes: String,
}

/// A lighting colour of the on-board profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Color {
    /// Red byte.
    pub r: u8,
    /// Green byte.
    pub g: u8,
    /// Blue byte.
    pub b: u8,
}

/// Lighting block of the on-board profile (57 bytes at payload offset 27).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LightingProfile {
    /// Six raw OLED illumination option bytes, hex.
    pub oled_illumination_options: String,
    /// Colour of the lock-key cluster.
    pub lock_key_color: Color,
    /// Colour of the META-mapped cluster.
    pub meta_key_color: Color,
    /// Colour of the disabled Win key.
    pub disabled_win_key_color: Color,
    /// 32-byte META mapping mask, hex.
    pub meta_mapped_elements: String,
    /// Three lock-key HID codes, hex.
    pub lock_key_hid_codes: String,
    /// Win key HID code.
    pub win_key_hid_code: u8,
    /// META-key background dimmer.
    pub meta_key_background_dimmer: u8,
    /// Idle-mode dimmer.
    pub idle_mode_dimmer: u8,
    /// Idle timeout in milliseconds.
    pub idle_timeout_ms: u32,
}

/// Rapid Tap block of a schema [`SCHEMA_B`] profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RapidTap {
    /// Rapid Tap enable byte.
    pub enable: u8,
    /// Ten Rapid Tap key pairs.
    pub pairs: Vec<RapidTapPair>,
}

/// One Rapid Tap pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RapidTapPair {
    /// First HID code.
    pub hid_1: u8,
    /// Second HID code.
    pub hid_2: u8,
    /// Mode without the report-both-keys flag.
    pub mode: u8,
    /// High bit of the mode byte, reported separately.
    pub report_both_keys: bool,
}

/// Summary of the macro data block (never dumped in full by the JSON writer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MacroSummary {
    /// Offset of the block in the payload.
    pub offset: usize,
    /// Block length in bytes.
    pub len: usize,
    /// Non-zero bytes in the block.
    pub nonzero: usize,
    /// 64-bit FNV-1a hash of the block, hex: identity check between reads.
    pub fnv1a_64: String,
    /// First 16 bytes, hex.
    pub head: String,
}

/// Decoded on-board profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardProfile {
    /// Schema word.
    pub schema: u16,
    /// Schema name (`a`, `b` or `unknown`).
    pub schema_name: String,
    /// Profile name (payload bytes 2..26, NUL-stripped).
    pub name: String,
    /// Raw profile name bytes, hex.
    pub name_hex: String,
    /// Lighting block.
    pub lighting: LightingProfile,
    /// META toggle HID byte.
    pub meta_toggle_hid: u8,
    /// 128 button mappings.
    pub button_mappings: Vec<KeyMapping>,
    /// Two wheel mappings.
    pub wheel_mappings: Vec<KeyMapping>,
    /// 128 button meta mappings.
    pub button_meta_mappings: Vec<KeyMapping>,
    /// Two wheel meta mappings.
    pub wheel_meta_mappings: Vec<KeyMapping>,
    /// OLED pixel block summary.
    pub oled: MacroSummary,
    /// Rapid Tap block, schema [`SCHEMA_B`] only.
    pub rapid_tap: Option<RapidTap>,
    /// Macro data block summary.
    pub macro_data: MacroSummary,
    /// 16-byte profile GUID, hex.
    pub guid: String,
    /// Schema [`SCHEMA_A`] padding behind the GUID, hex (empty for schema [`SCHEMA_B`]).
    pub guid_padding: String,
    /// Checksum stored in the payload.
    pub crc_stored: u32,
    /// Checksum computed over the payload prefix.
    pub crc_computed: u32,
    /// True when both checksums match.
    pub crc_ok: bool,
    /// Length of the checksummed prefix.
    pub crc_region_len: usize,
}

/// Layout of the decoded blocks.
///
/// The Windows engine's own decoder for this payload is a sequential struct stream: schema word,
/// 24-byte name, the 57-byte lighting block, the META toggle byte, then four mapping tables of
/// 4-byte entries (128 buttons, 2 wheels, 128 button META, 2 wheel META). The offsets below are
/// that stream read through. Everything from the GUID on is inferred: the engine's decoder ends at
/// the checksum word, and no read-back shows where the OLED and macro blocks sit in a payload.
///
/// `[EXPERIMENTAL]` - the mapping counts (128/2/128/2) and the entry stride (4) are GG's; the
/// offsets of the OLED, Rapid Tap and macro blocks are a sequential guess that fills the payload.
/// The engine's start-location table for a neighbouring Apex 7 model names 644 and 2088 for the
/// OLED and macro blocks, which matches neither of the two layouts above.
/// Offset of the Rapid Tap enable byte (schema [`SCHEMA_B`]).
pub const RAPID_TAP_ENABLE_OFFSET: usize = OLED_OFFSET + OLED_LEN;
/// Offset of the Rapid Tap pair table (schema [`SCHEMA_B`]).
pub const RAPID_TAP_PAIRS_OFFSET: usize = RAPID_TAP_ENABLE_OFFSET + 1;
/// Bytes of the Rapid Tap block: one enable byte plus ten 4-byte pairs.
pub const RAPID_TAP_LEN: usize = 1 + 10 * RAPID_TAP_PAIR_LEN;
/// Stride of one Rapid Tap pair.
pub const RAPID_TAP_PAIR_LEN: usize = 4;
/// Offset of the OLED pixel block.
pub const OLED_OFFSET: usize = GUID_OFFSET + GUID_LEN + GUID_PADDING_LEN;
/// Length of the OLED pixel block.
pub const OLED_LEN: usize = 640;
/// Offset of the button mapping table.
pub const BUTTON_MAPPINGS_OFFSET: usize = META_TOGGLE_HID_OFFSET + 1;
/// Offset of the wheel mapping table.
pub const WHEEL_MAPPINGS_OFFSET: usize = BUTTON_MAPPINGS_OFFSET + BUTTON_MAPPING_COUNT * KEY_MAPPING_LEN;
/// Offset of the button meta mapping table.
pub const BUTTON_META_MAPPINGS_OFFSET: usize = WHEEL_MAPPINGS_OFFSET + WHEEL_MAPPING_COUNT * KEY_MAPPING_LEN;
/// Offset of the wheel meta mapping table.
pub const WHEEL_META_MAPPINGS_OFFSET: usize = BUTTON_META_MAPPINGS_OFFSET + BUTTON_MAPPING_COUNT * KEY_MAPPING_LEN;
/// Offset of the lighting block.
pub const LIGHTING_OFFSET: usize = NAME_OFFSET + NAME_LEN + 1;
/// Length of the lighting block.
pub const LIGHTING_LEN: usize = 57;
/// Offset of the profile name.
pub const NAME_OFFSET: usize = 2;
/// Length of the profile name field.
pub const NAME_LEN: usize = 24;
/// Offset of the META toggle HID byte.
pub const META_TOGGLE_HID_OFFSET: usize = LIGHTING_OFFSET + LIGHTING_LEN;
/// Length of the GUID field.
pub const GUID_LEN: usize = 16;
/// Offset of the 16-byte profile GUID.
pub const GUID_OFFSET: usize = WHEEL_META_MAPPINGS_OFFSET + WHEEL_MAPPING_COUNT * KEY_MAPPING_LEN;
/// Bytes between the GUID and the checksum in a schema [`SCHEMA_A`] payload.
pub const GUID_PADDING_LEN: usize = 3;
/// Mapping table entry stride, GG's `apex_7_key_mapping_entry_size`.
pub const KEY_MAPPING_LEN: usize = 4;
/// Buttons in each mapping table, GG's decode loop.
pub const BUTTON_MAPPING_COUNT: usize = 128;
/// Wheels in each mapping table.
pub const WHEEL_MAPPING_COUNT: usize = 2;
/// Offset of the macro block behind the OLED block (schema [`SCHEMA_A`]).
pub const MACRO_OFFSET_A: usize = OLED_OFFSET + OLED_LEN;
/// Offset of the macro block behind Rapid Tap (schema [`SCHEMA_B`]).
pub const MACRO_OFFSET_B: usize = RAPID_TAP_PAIRS_OFFSET + 10 * RAPID_TAP_PAIR_LEN;

/// Function selector names, for summaries only; `0x00` means the key is deactivated.
pub fn function_name(function: u8) -> &'static str {
    match function {
        0x00 => "deactivated",
        0x01 => "mouse_left",
        0x02 => "mouse_right",
        0x03 => "mouse_middle",
        0x04 => "mouse_button_4",
        0x05 => "mouse_button_5",
        0x06 => "mouse_button_6",
        0x07 => "mouse_button_7",
        0x08 => "mouse_button_8",
        0x30 => "cpi",
        0x31 => "wheel_up",
        0x32 => "wheel_down",
        0x33 => "ac_pan_left",
        0x34 => "ac_pan_right",
        0x51 => "combo_key",
        0x61 => "media_key",
        0x62 => "meta",
        0x71 => "macro",
        _ => "unknown",
    }
}

/// 64-bit FNV-1a hash, hex. Used for block identity between reads, not as a protocol checksum.
pub fn fnv1a_64(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016X}")
}

fn summary_of(data: &[u8], offset: usize, len: usize) -> MacroSummary {
    let end = (offset + len).min(data.len());
    let start = offset.min(data.len());
    let block = &data[start..end];
    MacroSummary {
        offset,
        len,
        nonzero: block.iter().filter(|b| **b != 0).count(),
        fnv1a_64: fnv1a_64(block),
        head: hex_of(&block[..block.len().min(16)]),
    }
}

/// Hex of a byte slice, space separated.
pub fn hex_of(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 3);
    for (i, byte) in data.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{byte:02X}"));
    }
    out
}

/// Decode one `key_mapping` entry.
fn key_mapping(data: &[u8], at: usize) -> Result<KeyMapping> {
    if at + KEY_MAPPING_LEN > data.len() {
        return Err(Error::Profile(format!(
            "key mapping at {at} needs {} bytes, the payload has {}",
            KEY_MAPPING_LEN,
            data.len()
        )));
    }
    Ok(KeyMapping {
        function: data[at],
        function_name: function_name(data[at]).to_string(),
        key_codes: hex_of(&data[at + 1..at + KEY_MAPPING_LEN]),
    })
}

fn key_table(data: &[u8], at: usize, count: usize) -> Result<Vec<KeyMapping>> {
    (0..count)
        .map(|i| key_mapping(data, at + i * KEY_MAPPING_LEN))
        .collect()
}

fn color(data: &[u8], at: usize) -> Result<Color> {
    if at + 3 > data.len() {
        return Err(Error::Profile(format!(
            "colour at {at} needs 3 bytes, the payload has {}",
            data.len()
        )));
    }
    Ok(Color {
        r: data[at],
        g: data[at + 1],
        b: data[at + 2],
    })
}

/// Decode a profile payload.
///
/// `data` must be a whole payload: [`PAYLOAD_LEN_A`] for schema [`SCHEMA_A`] or [`PAYLOAD_LEN_B`] for
/// schema [`SCHEMA_B`]. A shorter buffer is rejected rather than decoded into partial fields.
pub fn decode(data: &[u8]) -> Result<OnboardProfile> {
    let schema = schema_of(data);
    let expected = payload_len_for_schema(schema)?;
    if data.len() < expected {
        return Err(Error::Profile(format!(
            "payload is {} bytes, schema 0x{schema:04X} needs {expected}",
            data.len()
        )));
    }

    let raw_name = &data[NAME_OFFSET..NAME_OFFSET + NAME_LEN];
    let name_end = raw_name.iter().position(|b| *b == 0).unwrap_or(raw_name.len());
    let name = String::from_utf8_lossy(&raw_name[..name_end]).into_owned();

    let l = LIGHTING_OFFSET;
    let lighting = LightingProfile {
        oled_illumination_options: hex_of(&data[l..l + 6]),
        lock_key_color: color(data, l + 6)?,
        meta_key_color: color(data, l + 9)?,
        disabled_win_key_color: color(data, l + 12)?,
        meta_mapped_elements: hex_of(&data[l + 15..l + 47]),
        lock_key_hid_codes: hex_of(&data[l + 47..l + 50]),
        win_key_hid_code: data[l + 50],
        meta_key_background_dimmer: data[l + 51],
        idle_mode_dimmer: data[l + 52],
        idle_timeout_ms: u32::from_le_bytes([data[l + 53], data[l + 54], data[l + 55], data[l + 56]]),
    };

    let schema_b = schema == SCHEMA_B;
    let rapid_tap = schema_b.then(|| RapidTap {
        enable: data[RAPID_TAP_ENABLE_OFFSET],
        pairs: (0..10)
            .map(|i| {
                let at = RAPID_TAP_PAIRS_OFFSET + i * RAPID_TAP_PAIR_LEN;
                RapidTapPair {
                    hid_1: data[at],
                    hid_2: data[at + 1],
                    mode: data[at + 2] & 0x7f,
                    report_both_keys: data[at + 2] & 0x80 != 0,
                }
            })
            .collect(),
    });

    let macro_offset = if schema_b { MACRO_OFFSET_B } else { MACRO_OFFSET_A };
    let guid_offset = GUID_OFFSET;
    let crc_offset = data.len() - 4;
    let computed = compute_checksum(&data[..crc_offset]);
    let stored = u32::from_le_bytes([
        data[crc_offset],
        data[crc_offset + 1],
        data[crc_offset + 2],
        data[crc_offset + 3],
    ]);

    Ok(OnboardProfile {
        schema,
        schema_name: match schema {
            SCHEMA_A => "a".to_string(),
            SCHEMA_B => "b".to_string(),
            _ => "unknown".to_string(),
        },
        name,
        name_hex: hex_of(&raw_name[..name_end]),
        lighting,
        meta_toggle_hid: data[META_TOGGLE_HID_OFFSET],
        button_mappings: key_table(data, BUTTON_MAPPINGS_OFFSET, BUTTON_MAPPING_COUNT)?,
        wheel_mappings: key_table(data, WHEEL_MAPPINGS_OFFSET, WHEEL_MAPPING_COUNT)?,
        button_meta_mappings: key_table(data, BUTTON_META_MAPPINGS_OFFSET, BUTTON_MAPPING_COUNT)?,
        wheel_meta_mappings: key_table(data, WHEEL_META_MAPPINGS_OFFSET, WHEEL_MAPPING_COUNT)?,
        oled: summary_of(data, OLED_OFFSET, OLED_LEN),
        rapid_tap,
        macro_data: summary_of(data, macro_offset, MACRO_DATA_LEN),
        guid: hex_of(&data[guid_offset..guid_offset + GUID_LEN]),
        guid_padding: if schema_b {
            String::new()
        } else {
            hex_of(&data[guid_offset + GUID_LEN..guid_offset + GUID_LEN + GUID_PADDING_LEN])
        },
        crc_stored: stored,
        crc_computed: computed,
        crc_ok: stored == computed,
        crc_region_len: crc_offset,
    })
}

/// Offset of the first byte that differs between two images, `None` when they are equal.
pub fn image_diff(a: &ProfileImage, b: &ProfileImage) -> Option<usize> {
    a.data.iter().zip(b.data.iter()).position(|(x, y)| x != y)
}

/// Multi-line hex dump of `data[from..to]`, 16 bytes per line with an offset column.
pub fn hex_dump(data: &[u8], from: usize, to: usize) -> String {
    let from = from.min(data.len());
    let to = to.max(from).min(data.len());
    let mut out = String::new();
    for (i, chunk) in data[from..to].chunks(16).enumerate() {
        out.push_str(&format!("{:06X}  {}\n", from + i * 16, hex_of(chunk)));
    }
    out
}

/// One-line human summary of a decoded profile.
pub fn summary_line(profile: &OnboardProfile) -> String {
    format!(
        "schema {}  name {:?}  guid {}  macro {} bytes ({} nonzero, fnv1a {})  crc {}",
        profile.schema_name,
        profile.name,
        profile.guid,
        profile.macro_data.len,
        profile.macro_data.nonzero,
        profile.macro_data.fnv1a_64,
        if profile.crc_ok {
            "ok".to_string()
        } else {
            format!(
                "MISMATCH stored {:08X} computed {:08X}",
                profile.crc_stored, profile.crc_computed
            )
        }
    )
}

/// JSON representation of a decoded profile.
pub fn profile_json(profile: &OnboardProfile) -> Result<String> {
    serde_json::to_string_pretty(profile).map_err(|e| Error::Profile(format!("serialize profile: {e}")))
}

/// JSON representation of a raw profile image with its per-pull records.
pub fn to_json(image: &ProfileImage) -> Result<String> {
    let payload = &image.data;
    let schema = image.schema();
    let decoded = if payload_len_for_schema(schema).is_ok() {
        json!(decode(payload).ok().map(|p| p.schema_name).unwrap_or_default())
    } else {
        json!(null)
    };
    let value = json!({
        "profile_slot": image.config.profile,
        "fs_id": format!("0x{:04X}", image.config.fs_id()),
        "schema": format!("0x{schema:04X}"),
        "schema_decoded": decoded,
        "data_len": payload.len(),
        "sha_like_fnv1a_64": fnv1a_64(payload),
        "stats": image.stats,
        "chunks": image.chunks,
        "head_hex": hex_of(&payload[..payload.len().min(64)]),
    });
    serde_json::to_string_pretty(&value).map_err(|e| Error::Profile(format!("serialize profile image: {e}")))
}

/// JSON representation of a read plan.
pub fn plan_json(config: &ReadConfig) -> Result<String> {
    let steps = config.steps();
    let value = json!({
        "profile_slot": config.profile,
        "fs_id": format!("0x{:04X}", config.fs_id()),
        "pulls": config.chunks,
        "resource_buffer_len": config.resource_len,
        "read_len": config.read_len,
        "payload_len": config.payload_len,
        "covered_len": config.total_resource_len(),
        "gap_ms": config.gap_ms,
        "pacing": {
            "time_between_commands_ms": config.time_between_commands_ms,
            "file_access": config.file_access,
            "max_usb_timeout_ms": config.max_usb_timeout_ms,
            "sent": config.send_config,
        },
        "transfers": steps,
    });
    serde_json::to_string_pretty(&value).map_err(|e| Error::Profile(format!("serialize read plan: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_payload(schema: u16) -> Vec<u8> {
        let len = payload_len_for_schema(schema).unwrap();
        let mut data = vec![0u8; len];
        data[0..2].copy_from_slice(&schema.to_le_bytes());
        data[NAME_OFFSET..NAME_OFFSET + 4].copy_from_slice(b"Pro7");
        data[META_TOGGLE_HID_OFFSET] = 0x62;
        data[LIGHTING_OFFSET + 6] = 0xFF; // lock key colour red
        data[LIGHTING_OFFSET + 53..LIGHTING_OFFSET + 57].copy_from_slice(&1500u32.to_le_bytes());
        data[BUTTON_MAPPINGS_OFFSET + KEY_MAPPING_LEN] = 0x04; // second mapping: left shift
        data[BUTTON_MAPPINGS_OFFSET + KEY_MAPPING_LEN + 1] = 0xE0;
        if schema == SCHEMA_B {
            data[RAPID_TAP_ENABLE_OFFSET] = 1;
            data[RAPID_TAP_PAIRS_OFFSET] = 0x04;
            data[RAPID_TAP_PAIRS_OFFSET + 1] = 0x05;
            data[RAPID_TAP_PAIRS_OFFSET + 2] = 0x83;
        }
        let macro_offset = if schema == SCHEMA_B {
            MACRO_OFFSET_B
        } else {
            MACRO_OFFSET_A
        };
        data[macro_offset] = 0x01;
        data[macro_offset + 1] = 0xA7;
        data[GUID_OFFSET] = 0xAB;
        let crc_offset = len - 4;
        let computed = compute_checksum(&data[..crc_offset]);
        data[crc_offset..].copy_from_slice(&computed.to_le_bytes());
        data
    }

    #[test]
    fn frames_match_the_documented_wire_layout() {
        assert_eq!(
            config_frame(1, FILE_ACCESS_READWRITE, 500),
            [0x01, 0, 0, 0, 0x02, 0, 0, 0, 0xF4, 0x01, 0, 0]
        );
        assert_eq!(
            pull_frame(PROFILE_FS_BASE, 1024, 1024),
            [0x85, 0x00, 0x01, 0x02, 0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00]
        );
        assert_eq!(read_frame(512, 512), [0x83, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00]);
        assert_eq!(read_frame(0, 2), [0x83, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00]);
    }

    #[test]
    fn plan_is_pacing_then_pull_read_read() {
        let config = ReadConfig::default();
        config.validate().unwrap();
        let steps = config.steps();
        assert_eq!(steps.len(), 1 + 12 * 3);
        assert_eq!(steps[0].kind, StepKind::Config);
        assert_eq!(steps[1].kind, StepKind::Pull);
        assert_eq!(steps[2].kind, StepKind::Read);
        assert_eq!(steps[3].kind, StepKind::Read);
        assert_eq!(steps[2].dest, Some(0));
        assert_eq!(steps[3].dest, Some(512));
        assert_eq!(steps[5].dest, Some(1024));
        let last = steps.last().unwrap();
        assert_eq!(last.kind, StepKind::Read);
        assert_eq!(last.dest, Some(11 * 1024 + 512));
        assert_eq!(last.dest.unwrap() + usize::from(last.size), 12_288);
        let pulls: Vec<usize> = steps
            .iter()
            .filter(|s| s.kind == StepKind::Pull)
            .map(|s| u32::from_le_bytes([s.frame[4], s.frame[5], s.frame[6], s.frame[7]]) as usize)
            .collect();
        assert_eq!(pulls, (0..12).map(|i| i * 1024).collect::<Vec<usize>>());
    }

    #[test]
    fn plan_targets_the_profile_file_id() {
        let config = ReadConfig {
            profile: 3,
            ..ReadConfig::default()
        };
        assert_eq!(config.fs_id(), 0x0204);
        let steps = config.steps();
        let pull = steps.iter().find(|s| s.kind == StepKind::Pull).unwrap();
        assert_eq!(
            u16::from_le_bytes([pull.frame[2], pull.frame[3]]),
            0x0204,
            "the pull frame carries the file id"
        );
    }

    #[test]
    fn schema_read_is_one_pull_of_two_bytes() {
        let steps = schema_read_steps(0x0203);
        assert_eq!(steps.len(), 3);
        let pull = &steps[1];
        assert_eq!(pull.size, 2);
        assert_eq!(
            u32::from_le_bytes([pull.frame[4], pull.frame[5], pull.frame[6], pull.frame[7]]),
            0,
            "the schema pull reads from the file head"
        );
        assert_eq!(steps[2].size, 2);
        assert_eq!(steps[2].dest, Some(0));
    }

    #[test]
    fn reply_header_is_echoed_and_payload_starts_at_six() {
        let mut reply = vec![0x83u8, 0x00, 0x00, 0x02, 0x00, 0x02];
        reply.extend((1u8..=64).cycle().take(636));
        let parsed = parse_read_reply(&reply, 512).unwrap();
        assert_eq!(parsed.report_id, 0x83);
        assert_eq!(parsed.offset, 512);
        assert_eq!(parsed.size, 512);
        assert_eq!(parsed.data.len(), 512);
        assert_eq!(parsed.data[0], 1);
        assert!(!parsed.all_zero);
        assert!(!parsed.all_ff);
    }

    #[test]
    fn a_page_starting_with_four_ff_bytes_is_counted_as_garbage() {
        let mut reply = vec![0x83u8, 0x00, 0x00, 0x00, 0x00, 0x04];
        reply.extend([0xFFu8, 0xFF, 0xFF, 0xFF, 0x01, 0x02]);
        let parsed = parse_read_reply(&reply, 6).unwrap();
        assert!(parsed.all_ff);
        assert!(!parsed.all_zero);
    }

    fn reply_report(prefix: [u8; 6], fill: u8, len: usize) -> Vec<u8> {
        let mut reply = prefix.to_vec();
        reply.extend(std::iter::repeat_n(fill, len));
        reply
    }

    #[test]
    fn zero_filled_reply_is_flagged() {
        let reply = reply_report([0x83, 0, 0, 0, 0, 0x02], 0, 636);
        let parsed = parse_read_reply(&reply, 512).unwrap();
        assert!(parsed.all_zero);
        assert_eq!(parsed.data.len(), 512);
    }

    #[test]
    fn reply_mismatch_is_reported() {
        let reply = reply_report([0x83, 0, 0x02, 0x00, 0, 0x02], 0, 636);
        let parsed = parse_read_reply(&reply, 512).unwrap();
        let mismatch = reply_mismatch(&parsed, 0, 512).expect("offset mismatch");
        assert!(mismatch.contains("offset"), "{mismatch}");
        let bad_report = reply_report([0x03, 0, 0, 0, 0, 0x02], 0, 636);
        let parsed = parse_read_reply(&bad_report, 512).unwrap();
        let mismatch = reply_mismatch(&parsed, 0, 512).expect("report mismatch");
        assert!(mismatch.contains("report"), "{mismatch}");
        assert!(reply_mismatch(&parsed, 0, 512).is_some());
    }

    #[test]
    fn short_reply_is_a_short_read() {
        let reply = reply_report([0x83, 0, 0, 0, 0, 0x02], 7, 10);
        let parsed = parse_read_reply(&reply, 512).unwrap();
        assert_eq!(parsed.data.len(), 10, "a truncated report yields a short read");
    }

    #[test]
    fn assemble_scatters_reads_by_file_offset() {
        let config = ReadConfig {
            chunks: 2,
            payload_len: 2048,
            ..ReadConfig::default()
        };
        let records = (0..2)
            .map(|chunk| ChunkRecord {
                index: chunk,
                fs_id: config.fs_id(),
                file_offset: (chunk * config.resource_len) as u32,
                ack_report_id: 0x85,
                ack_hex: "85".to_string(),
                reads: (0..2)
                    .map(|read| ReadReply {
                        report_id: 0x83,
                        reserved: 0,
                        offset: (read * config.read_len) as u16,
                        size: config.read_len as u16,
                        data: vec![(chunk * 8 + read + 1) as u8; config.read_len],
                        all_zero: false,
                        all_ff: false,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        let image = ProfileImage::assemble(records, config).unwrap();
        assert_eq!(image.data.len(), 2048);
        assert_eq!(image.data[0], 1);
        assert_eq!(image.data[512], 2);
        assert_eq!(image.data[1024], 9);
        assert_eq!(image.data[1536], 10);
        assert_eq!(image.stats.chunks_read, 2);
        assert_eq!(image.stats.reads_read, 4);
        assert_eq!(image.stats.header_mismatches, 0);
        assert_eq!(image.stats.zero_chunks, 0);
        assert_eq!(image.stats.covered_len, 2048);
    }

    #[test]
    fn assemble_drops_reads_past_the_payload() {
        let config = ReadConfig {
            chunks: 1,
            resource_len: 1024,
            read_len: 512,
            payload_len: 600,
            ..ReadConfig::default()
        };
        let record = ChunkRecord {
            index: 0,
            fs_id: config.fs_id(),
            file_offset: 0,
            ack_report_id: 0x85,
            ack_hex: "85".to_string(),
            reads: (0..2)
                .map(|read| ReadReply {
                    report_id: 0x83,
                    reserved: 0,
                    offset: (read * 512) as u16,
                    size: 512,
                    data: vec![0xAA_u8; 512],
                    all_zero: false,
                    all_ff: false,
                })
                .collect(),
        };
        let image = ProfileImage::assemble(vec![record], config).unwrap();
        assert_eq!(image.data.len(), 600);
        assert_eq!(image.data[599], 0xAA, "the tail read is clipped to the payload");
        assert_eq!(image.stats.short_reads, 0);
    }

    #[test]
    fn config_rejects_plans_that_cannot_cover_the_payload() {
        let config = ReadConfig {
            chunks: 11,
            ..ReadConfig::default()
        };
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("short of"), "{err}");
        let config = ReadConfig {
            resource_len: 700,
            read_len: 700,
            payload_len: 11_648,
            chunks: 12,
            ..ReadConfig::default()
        };
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("reply report"), "{err}");
        let config = ReadConfig {
            resource_len: 1000,
            read_len: 300,
            payload_len: 11_648,
            chunks: 12,
            ..ReadConfig::default()
        };
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("whole number"), "{err}");
        assert!(
            ReadConfig {
                profile: 5,
                ..ReadConfig::default()
            }
            .validate()
            .unwrap_err()
            .to_string()
            .contains("highest slot")
        );
    }

    #[test]
    fn decode_schema_a_payload() {
        let data = synthetic_payload(SCHEMA_A);
        let profile = decode(&data).unwrap();
        assert_eq!(profile.schema, SCHEMA_A);
        assert_eq!(profile.schema_name, "a");
        assert_eq!(profile.name, "Pro7");
        assert_eq!(profile.meta_toggle_hid, 0x62);
        assert_eq!(profile.lighting.lock_key_color, Color { r: 0xFF, g: 0, b: 0 });
        assert_eq!(profile.lighting.idle_timeout_ms, 1500);
        assert_eq!(profile.rapid_tap, None, "schema A has no Rapid Tap block");
        assert_eq!(profile.button_mappings.len(), 128);
        assert_eq!(profile.button_mappings[1].function, 0x04);
        assert_eq!(profile.button_mappings[1].function_name, "mouse_button_4");
        assert_eq!(profile.macro_data.offset, MACRO_OFFSET_A);
        assert_eq!(profile.macro_data.len, MACRO_DATA_LEN);
        assert_eq!(profile.macro_data.nonzero, 2);
        assert!(profile.guid.starts_with("AB"), "{}", profile.guid);
        assert_eq!(profile.guid_padding, "00 00 00");
        assert!(profile.crc_ok, "stored crc matches the computed one");
        assert_eq!(profile.crc_region_len, PAYLOAD_LEN_A - 4);
    }

    #[test]
    fn decode_schema_b_payload_carries_rapid_tap() {
        let data = synthetic_payload(SCHEMA_B);
        let profile = decode(&data).unwrap();
        assert_eq!(profile.schema_name, "b");
        let rapid = profile.rapid_tap.expect("schema B has a Rapid Tap block");
        assert_eq!(rapid.enable, 1);
        assert_eq!(rapid.pairs.len(), 10);
        assert_eq!(rapid.pairs[0].hid_1, 0x04);
        assert_eq!(rapid.pairs[0].hid_2, 0x05);
        assert_eq!(rapid.pairs[0].mode, 3);
        assert!(rapid.pairs[0].report_both_keys);
        assert_eq!(profile.macro_data.offset, MACRO_OFFSET_B);
        assert!(profile.guid_padding.is_empty(), "schema B has no GUID padding");
        assert!(profile.crc_ok);
        assert_eq!(profile.crc_region_len, PAYLOAD_LEN_B - 4);
    }

    #[test]
    fn decode_reports_a_crc_disagreement() {
        let mut data = synthetic_payload(SCHEMA_A);
        let crc_offset = data.len() - 4;
        data[crc_offset] ^= 0xFF;
        let profile = decode(&data).unwrap();
        assert!(!profile.crc_ok, "a flipped stored byte must disagree");
        assert_ne!(profile.crc_stored, profile.crc_computed);
        assert!(summary_line(&profile).contains("MISMATCH"));
    }

    #[test]
    fn decode_rejects_unknown_schema_and_short_payloads() {
        let data = synthetic_payload(SCHEMA_A);
        assert!(
            decode(&data[..data.len() - 1]).is_err(),
            "a payload shorter than the schema needs is rejected"
        );
        let mut bogus = vec![0u8; PAYLOAD_LEN_A];
        bogus[0..2].copy_from_slice(&0x0009u16.to_le_bytes());
        assert!(decode(&bogus).is_err(), "an unknown schema is rejected");
        assert!(payload_len_for_schema(0x0009).is_err());
        assert_eq!(schema_of(&[0x0B, 0x00]), SCHEMA_B);
        assert_eq!(schema_of(&[0x01]), 0, "a 1-byte payload has no schema word");
    }

    #[test]
    fn json_output_is_valid() {
        let data = synthetic_payload(SCHEMA_B);
        let profile = decode(&data).unwrap();
        let text = profile_json(&profile).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["schema_name"], "b");
        assert_eq!(value["name"], "Pro7");
        assert_eq!(value["lighting"]["lock_key_color"]["r"], 255);
        assert_eq!(value["button_mappings"].as_array().unwrap().len(), 128);
        assert!(value["rapid_tap"].is_object());

        let image = ProfileImage::assemble(
            {
                let reads_per_pull = RESOURCE_BUFFER_LEN / READ_LEN;
                (0..DEFAULT_CHUNKS)
                    .map(|chunk| ChunkRecord {
                        index: chunk,
                        fs_id: 0x0201,
                        file_offset: (chunk * RESOURCE_BUFFER_LEN) as u32,
                        ack_report_id: 0x85,
                        ack_hex: "85 00".to_string(),
                        reads: (0..reads_per_pull)
                            .map(|read| {
                                let i = chunk * reads_per_pull + read;
                                let start = i * READ_LEN;
                                let data_slice = data
                                    .get(start..start + READ_LEN)
                                    .map(|s| s.to_vec())
                                    .unwrap_or_else(|| vec![0u8; READ_LEN]);
                                ReadReply {
                                    report_id: 0x83,
                                    reserved: 0,
                                    offset: (read * READ_LEN) as u16,
                                    size: READ_LEN as u16,
                                    all_zero: data_slice.iter().all(|b| *b == 0),
                                    all_ff: false,
                                    data: data_slice,
                                }
                            })
                            .collect(),
                    })
                    .collect::<Vec<_>>()
            },
            ReadConfig {
                payload_len: PAYLOAD_LEN_B,
                ..ReadConfig::default()
            },
        )
        .unwrap();
        let raw = to_json(&image).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["schema_decoded"], "b");
        assert_eq!(value["data_len"], PAYLOAD_LEN_B);

        let plan = plan_json(&ReadConfig::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&plan).unwrap();
        assert_eq!(value["pulls"], 12);
        assert_eq!(value["transfers"].as_array().unwrap().len(), 37);
    }

    #[test]
    fn image_diff_finds_the_first_difference() {
        let config = ReadConfig {
            payload_len: 12 * 1024,
            ..ReadConfig::default()
        };
        let record = |first: u8| ChunkRecord {
            index: 0,
            fs_id: config.fs_id(),
            file_offset: 0,
            ack_report_id: 0x85,
            ack_hex: "85".to_string(),
            reads: (0..(RESOURCE_BUFFER_LEN / READ_LEN) * DEFAULT_CHUNKS)
                .map(|i| ReadReply {
                    report_id: 0x83,
                    reserved: 0,
                    offset: ((i % 2) * READ_LEN) as u16,
                    size: READ_LEN as u16,
                    data: vec![if i == 3 { first } else { 0u8 }; READ_LEN],
                    all_zero: i != 3,
                    all_ff: false,
                })
                .collect(),
        };
        let a = ProfileImage::assemble(vec![record(1)], config).unwrap();
        let b = ProfileImage::assemble(vec![record(1)], config).unwrap();
        let c = ProfileImage::assemble(vec![record(2)], config).unwrap();
        assert_eq!(image_diff(&a, &b), None);
        assert_eq!(image_diff(&a, &c), Some(3 * READ_LEN));
        assert_eq!(a.diff(&c), Some(3 * READ_LEN));
    }

    #[test]
    fn hex_dump_is_offsetted() {
        let data = synthetic_payload(SCHEMA_A);
        let dump = hex_dump(&data, 0, 32);
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("000000  0A 00 50 72 6F"), "{}", lines[0]);
        assert!(lines[1].starts_with("000010  "), "{}", lines[1]);
        assert!(hex_dump(&data, 10, 5).is_empty(), "an inverted range dumps nothing");
    }
}
