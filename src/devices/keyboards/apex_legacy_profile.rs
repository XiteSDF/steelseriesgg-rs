//! [EXPERIMENTAL] On-board profile read-back for the legacy/"macho" Apex platform.
//!
//! This module is the pure half of the read path: frame builders, reply parsing, payload assembly,
//! struct decode, checksum reporting and JSON. It performs no I/O. The hardware loop is
//! [`crate::devices::keyboards::apex_legacy_onboard`].
//!
//! It lives in this keyboard namespace next to its consumers, because everything
//! in it belongs to this one platform: the reply framing, resource-buffer and window sizes, file-id
//! base, payload sizes, schema words and the payload struct offsets all come from the legacy platform's
//! own protocol module (`apexui.dll`). Nothing here is generic - the Gen3 platform (`0x1628`) reads its
//! on-board profile through a completely different transport (see
//! [`crate::devices::keyboards::apex_pro_tkl_2023_profile`]) and shares none of it.
//!
//! Protocol status: reverse-engineered. The file ids, chunk geometry, frame layouts and the payload
//! struct layout below are reproduced from the Windows engine's device descriptors, and the wire part of
//! them is visible in one Windows USBPcap capture of the official GG talking to an Apex 7 (firmware
//! 4.16.8, 2026-10-15, `ssgg-capture-1612-20261015-004400`, decoded from `hub-2.pcap` frame by frame):
//! 120 pulls of 1024 bytes at file offsets 0..11264, each drained by reads at buffer offsets 0 and 512,
//! 100 of them on file `0x0201` (profile slot 0) plus one schema probe; 241 payload reads, every one of
//! them answering with the first payload byte `0x0B`, i.e. schema [`SCHEMA_B`]. On 2026-10-15 an Apex 7
//! accepted a pull and a payload-read request on its interface-4 vendor collection and answered a pull
//! on the interrupt endpoint; the payload read timed out there, which is why the reply is read on the
//! control endpoint first. Since the read-back fix of 2026-10-17 a full slot read completes on an Apex 7
//! under Linux (reported 2026-10-18, with GG's opening chain sent; the read without it answered on the
//! wire with scrambled payload bytes). Nothing here writes a profile, and the payload struct offsets stay
//! unverified until a read is compared with what GG shows for that slot.
//!
//! # Wire format
//!
//! All transfers run on the keyboard's vendor collections. The Apex 7 exposes usage page `0xffc0` on
//! USB interface 1 (this is where the reference capture shows every profile transfer running) and usage
//! page `0xffc1` on interface 4 (the collection the Linux `hidraw` node for the profile protocol sits
//! on). Report numbers are embedded in the payload, so in `hidapi` terms the first byte of every buffer
//! is the report ID.
//!
//! * firmware poll (output report ID `0x00`, 64 data bytes on the wire, `SET_REPORT wValue 0x0200 wIndex
//!   1 wLength 64`): `90 00`, answered on the interrupt endpoint with the ASCII firmware version
//!   (`34 2e 31 36 2e 38 00…` = `4.16.8`, see [`parse_firmware_version]). An Apex 7 wired answered `90 00`
//!   and `90 01` identically on 2026-10-17. This crate sends it inside GG's opening chain - `90 00`,
//!   `90 01`, `f4 00`, `6c 00 01`, then `90 00` and `90 01` again, 11 ms apart
//!   ([`crate::devices::hid_reports::ONBOARD_INIT_CHAIN`]), which is what a read needs on hardware: with
//!   only the poll the replies come back scrambled (2026-10-18). The meanings of `f4 00` (answered
//!   `00 01 00…`) and `6c 00 01` (never answered in the capture) are unknown. The capture contains no
//!   13-byte transfer anywhere, so the pacing frame earlier versions of this module sent is not part of
//!   GG's read.
//! * pull (`HIDIO`, output report ID `0x00`, 64 bytes on the wire, `SET_REPORT wValue 0x0200 wIndex 1`):
//!   `85 | 00 | file_id u16 LE | offset u32 LE | size u16 LE | 00…` (10 frame bytes, 11-byte report buffer)
//!   — copies `size` bytes of the profile file into the device's resource buffer at buffer offset 0.
//!   **The pull has no answer of its own.** The interrupt endpoint streams the keyboard's ordinary
//!   input reports (endpoint `0x82`, report `0x01`, 63 bytes, all-zero = no keys; median gap 14 ms =
//!   the poll interval), and GG's read API consumes exactly one of them per chunk with
//!   `read-chunk HIDIO 65`, keeping **no bytes** of it: the byte specifier for the pull chunk in the
//!   read API is `(0 0 0)` - zero length. The read answer is what carries the data ([`PAYLOAD_READ_LEN`] bytes
//!   from reply byte 1 in GG's specifiers, our framing constant's offset behind the report-ID byte).
//!   All 121 pulls in the capture are followed by an all-zero `0x01` report 1.7-3.7 ms later, and so
//!   are all 72 pushes of the write runs: the interrupt packet is a pacing token, not a status frame.
//!   The `85 00 fs offset size` frames the capture kit lists as 64-byte "pull replies" are the
//!   `SET_REPORT` requests themselves.
//! * payload read (`HIDFEATURE`, feature report ID `0x00`, 642 bytes on the wire, `SET_REPORT wValue
//!   0x0300 wIndex 1`): `83 | 00 | offset u16 LE | size u16 LE | 00…` (6 frame bytes, a
//!   `crate::devices::hid_reports::ONBOARD_FEATURE_REPORT_SIZE` write buffer, report number byte included)
//!   — returns `size` bytes of the resource buffer. GG reads the answer with `GET_REPORT wValue 0x0300
//!   wIndex 1 wLength 642`, and that answer carries no header: the payload starts at byte 1. The 2-byte
//!   probe of file `0x0201` answers `03 | 0b 00` and the first 512-byte read of the same file answers
//!   `03 | 0b 00 43 6f 6e 66 69 67 20 31 …`, i.e. the schema word at payload offset 0 and the profile
//!   name at payload offset 2. Some firmware echoes the request header instead, with the payload from
//!   byte 6; [`parse_read_reply`] accepts both.
//!
//! The read geometry is not tunable: a profile file is read as [`PULLS_PER_FILE`] resource-buffer fills
//! ([`DEVICE_RESOURCE_BUFFER_LEN`] each); every fill is drained with two [`PAYLOAD_READ_LEN`]-byte payload
//! reads at buffer offsets 0 and 512. The profile files are addressed by id: [`PROFILE_FS_BASE`] plus the
//! profile index (`0..=4`).
//!
//! The first two payload bytes are the profile schema: GG probes a file with one 2-byte pull of the file
//! head and one 2-byte payload read. Firmware `4.16.3` selects schema [`SCHEMA_B`]; older firmware uses
//! [`SCHEMA_A`].
//!
//! The payload starts at file offset 0 with the schema word; the engine's payload structs carry one
//! leading constant id byte that is not part of the file, so a payload read lands at its file offset and
//! the assembled image is [`PROFILE_FILE_BYTES`] bytes with no id byte.

use crate::devices::hid_reports::{ONBOARD_GET_WIRE_BYTES, ONBOARD_MAX_READ_LEN, ONBOARD_OUTPUT_REPORT_SIZE};
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
/// Firmware the Windows engine decoder lists as switching the on-board profile schema to [`SCHEMA_B`].
/// Documentation only: the read takes its schema from the word it reads at payload offset 0 and never
/// compares a firmware version with this.
pub const SCHEMA_B_FIRMWARE_VERSION: &str = "4.16.3";
/// Size of the device's on-board resource buffer, in bytes - the fixed fill size of every pull. A fact
/// of the device, not a size this code can request differently.
pub const DEVICE_RESOURCE_BUFFER_LEN: usize = 1024;
/// Size of one payload-read window in bytes: the fixed `size` of every payload read, the window GG's
/// read specifiers use. A protocol fact, not a read length to tune.
pub const PAYLOAD_READ_LEN: usize = 512;
/// Pulls per profile file, the fixed pull count of the read: [`PULLS_PER_FILE`] fills of
/// [`DEVICE_RESOURCE_BUFFER_LEN`] bytes cover both payload lengths. The read loop runs exactly this many
/// pulls; there is no chunk-count option.
pub const PULLS_PER_FILE: usize = 12;
/// File id of profile slot 0; slot `n` is this plus `n`.
pub const PROFILE_FS_BASE: u16 = 0x0201;
/// Highest profile slot the device offers (`0..=4`), a fixed slot count of the platform.
pub const MAX_PROFILE_INDEX: u8 = 4;
/// File id of the per-profile lighting config of slot 0.
pub const LIGHTING_FS_BASE: u16 = 0x0301;
/// Macro data block inside a profile payload, in bytes.
pub const MACRO_DATA_LEN: usize = 9600;

/// Command byte of the push (profile write); defined for completeness, nothing here sends it. The
/// read path has no command bytes of its own: they are
/// [`crate::devices::hid_reports::CommandCode`] variants and every frame is built by a typed command in
/// [`crate::devices::hid_reports`] (see [`crate::devices::hid_reports::OnboardPullCommand`]).
pub const CMD_PUSH_PAYLOAD: u8 = 0x05;
/// Report ID of every request frame.
pub const REQUEST_REPORT_ID: u8 = 0x00;
/// Report ID of a payload-read reply.
pub const READ_REPLY_REPORT_ID: u8 = 0x83;
/// Command byte of a pull, which is what the capture kit recorded in the frames it labels pull
/// replies; the device answers a pull with its ordinary `0x01` input report, not with `0x85`.
pub const PULL_REPLY_REPORT_ID: u8 = 0x85;
/// Report ID of a lighting-config payload-read reply.
pub const LIGHTING_READ_REPLY_REPORT_ID: u8 = 0x03;
/// Buffer size to ask the transport for when reading a payload read back: [`ONBOARD_GET_WIRE_BYTES`] (642).
/// The read side sends this number to the device as `wLength` verbatim, which is the capture's
/// `GET_REPORT wValue 0x0300 wIndex 1 wLength 642`; asking 643 makes the transfer never complete
/// (`Connection timed out`, measured 2026-10-15). The kernel lands the device's report-ID byte in buffer[0],
/// so the reply arrives as `[report ID][641 bytes]`.
pub const READ_REPORT_LEN: usize = ONBOARD_GET_WIRE_BYTES;
/// Data-stage length of the 64-byte output reports of this protocol (`SET_REPORT wValue 0x0200
/// wIndex 1 wLength 64`): the firmware poll and every pull. The report number the transport prefixes
/// is not part of it.
pub const OUTPUT_REPORT_DATA_LEN: usize = ONBOARD_OUTPUT_REPORT_SIZE - 1;
/// Input-report size GG reads on the interrupt endpoint: its `read-chunk HIDIO 65` is a report-ID slot
/// plus 64 data bytes. The same read serves the pacing packet after a pull and the answers of the opening
/// chain (`0x6E` with the ASCII firmware version, `0x12` with `00 01 …`), so this is the length the read
/// loop asks hidapi for. A 64-byte ask works but truncates the last byte of a full 65-byte answer.
pub const GG_INPUT_READ_BYTES: usize = OUTPUT_REPORT_DATA_LEN + 1;
/// Header of a header-echoing payload-read reply, report ID byte excluded: `reserved, offset u16, size u16`.
pub const READ_REPLY_HEADER_LEN: usize = 5;
/// Framing bytes between the report-ID byte and the payload of a payload-read reply (`GET_REPORT
/// wValue 0x0300 wIndex 1 wLength 642`): the report-ID byte, and nothing else — the payload starts
/// directly behind it, so the framing length is 0. In the 2026-10-15 capture every one of the 241
/// replies answers `03` as its first byte: the 2-byte probe of file `0x0201` answers `03 0b 00`
/// (schema [`SCHEMA_B`]) and the 512-byte read at buffer offset 0 answers
/// `03 0b 00 43 6f 6e 66 69 67 20 31 …`, the profile name 2 bytes behind the schema word, the offset the
/// Windows engine decoder documents. An earlier version of this constant said 20: that many bytes sit
/// between the payload and the report bytes in the capture's **text** export, which prefixes every
/// report with the USBPcap header, and 20 of 241 replies matched by accident. The 2026-10-15 hardware
/// probe accepted 6. There is no CLI knob for it (the `--reply-header` option was removed 2026-10-17):
/// a keyboard that anchors its payload elsewhere is a change of this constant, not an option.
pub const READ_REPLY_FRAMING_LEN: usize = 0;
/// Median gap between two device replies during a GG profile read in the reference capture, in
/// milliseconds: the fixed pacing between our own transfers. (GG's opening-chain frames are 11 ms apart;
/// this read replays none of them. `--gap-ms` is gone, so this constant is the pacing.)
pub const GG_GAP_MS: u64 = 5;

/// Firmware version a [`crate::devices::hid_reports::CommandCode::FirmwareVersion`] answer carries: the printable
/// ASCII run
/// from the first byte, which the capture shows NUL padded (`34 2e 31 36 2e 38 00…` = `4.16.8`).
/// Returns `None` when the answer starts with non-printable bytes, which is how a keyboard that frames
/// its answer with a header, or does not answer the poll at all, shows up.
#[must_use]
pub fn parse_firmware_version(reply: &[u8]) -> Option<String> {
    let text = reply
        .iter()
        .take_while(|b| **b >= 0x20 && **b < 0x7F)
        .map(|&b| char::from(b))
        .collect::<String>();
    (!text.is_empty()).then_some(text)
}

/// Largest payload a payload-read reply can carry: the 642 bytes the reply delivers, minus the report-ID
/// byte the kernel puts in buffer[0] and minus the 5-byte header a header-echoing reply prepends
/// (`03 01 01 00 01`). Payload-only replies (the capture's shape) start directly behind the report-ID byte,
/// so 636 is the ceiling both shapes agree on — and it equals [`ONBOARD_MAX_READ_LEN`], the cap the read
/// frame builder enforces. A ceiling derived from the wire, not a tunable.
pub const MAX_READ_LEN: usize = READ_REPORT_LEN - 1 - READ_REPLY_HEADER_LEN;

/// The read frame may never ask for more than a reply can carry.
const _: () = assert!(MAX_READ_LEN == ONBOARD_MAX_READ_LEN);

/// File bytes one read covers: [`PULLS_PER_FILE`] pulls of [`DEVICE_RESOURCE_BUFFER_LEN`] bytes, which
/// covers both payload lengths ([`PAYLOAD_LEN_A`] and [`PAYLOAD_LEN_B`]). The read always covers exactly
/// this many bytes: the shorter image would be a partial profile, the longer one would need slots that
/// do not exist.
pub const PROFILE_FILE_BYTES: usize = PULLS_PER_FILE * DEVICE_RESOURCE_BUFFER_LEN;

/// Payload reads per pull: one pull fills [`DEVICE_RESOURCE_BUFFER_LEN`], each read drains
/// [`PAYLOAD_READ_LEN`] of it.
pub const READS_PER_PULL: usize = DEVICE_RESOURCE_BUFFER_LEN / PAYLOAD_READ_LEN;

/// The fixed read must cover both payload lengths, or the assembled image would be a partial profile.
const _: () = assert!(
    PROFILE_FILE_BYTES >= PAYLOAD_LEN_B,
    "the fixed pull loop must cover the longer payload"
);

/// Profile file id of slot `profile`: [`PROFILE_FS_BASE`] plus the slot index.
#[must_use]
pub fn profile_file_id(profile: u8) -> u16 {
    PROFILE_FS_BASE.wrapping_add(u16::from(profile))
}

/// Reject a slot index this read does not support.
///
/// # Errors
///
/// [`Error::InvalidConfig`] when `profile` is above [`MAX_PROFILE_INDEX`].
pub fn validate_profile(profile: u8) -> Result<()> {
    if profile > MAX_PROFILE_INDEX {
        return Err(Error::InvalidConfig(format!(
            "profile slot {profile} is above the highest slot {MAX_PROFILE_INDEX}"
        )));
    }
    Ok(())
}

/// Parsed header of a payload-read reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadReply {
    /// Report ID the reply arrived on.
    pub report_id: u8,
    /// Reserved header byte behind the report ID.
    pub reserved: u8,
    /// True when the reply echoed the request header (see [`parse_read_reply`]).
    pub header: bool,
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
/// Parse a payload-read reply (`reply` starts with the report ID byte) and extract its payload.
///
/// The reference capture shows the keyboard answering a payload read in two shapes, both accepted
/// here. On the interrupt endpoint the reply echoes the request header and the payload starts at
/// byte 6; on the control endpoint (`GET_REPORT 0x0300`, 642 bytes on interface 1) the answer carries
/// no header at all, so the payload starts directly behind the report-ID byte. The reply header of
/// the first shape is `[0x83][reserved][buffer offset u16 LE][size u16 LE]`.
pub fn parse_read_reply(reply: &[u8], requested_size: u16) -> Result<ReadReply> {
    parse_read_reply_at(reply, requested_size, READ_REPLY_FRAMING_LEN)
}

/// [`parse_read_reply`] with an explicit payload anchor: the payload starts at
/// `1 + framing_len` (byte 0 is the report ID). The capture's replies put the payload behind the
/// report-ID byte alone; a keyboard that answers with a header needs `5`, one that answers with 20
/// framing bytes needs `20`.
///
/// # Errors
///
/// Returns [`Error::Profile`] when the reply is empty.
pub fn parse_read_reply_at(reply: &[u8], requested_size: u16, framing_len: usize) -> Result<ReadReply> {
    if reply.is_empty() {
        return Err(Error::Profile("payload-read reply is empty".to_string()));
    }
    let header = reply.len() > READ_REPLY_HEADER_LEN && reply[0] == READ_REPLY_REPORT_ID && reply[1] == 0;
    let (offset, size, payload) = if header {
        (
            u16::from_le_bytes([reply[2], reply[3]]),
            u16::from_le_bytes([reply[4], reply[5]]),
            &reply[1 + READ_REPLY_HEADER_LEN..],
        )
    } else if reply.len() > 1 + framing_len {
        (0, requested_size, &reply[1 + framing_len..])
    } else {
        (0, requested_size, &reply[reply.len().saturating_sub(1)..])
    };
    let limit = usize::from(requested_size).min(MAX_READ_LEN);
    let data = payload.iter().copied().take(limit).collect::<Vec<u8>>();
    let all_zero = data.iter().all(|b| *b == 0);
    let all_ff = data.len() >= 4 && data[..4].iter().all(|b| *b == 0xFF);
    Ok(ReadReply {
        report_id: reply[0],
        reserved: reply[1],
        header,
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
    /// Report ID the packet after the pull arrived on (`0x01` on an idle keyboard).
    pub ack_report_id: u8,
    /// First bytes of that packet, hex; content is discarded, GG's read spec takes zero bytes from it.
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
    /// Profile slot the read ran on.
    pub profile_slot: u8,
    /// Per-pull records, in collection order.
    pub chunks: Vec<ChunkRecord>,
    /// Profile file bytes, reads concatenated in pull order.
    pub data: Vec<u8>,
    /// Aggregated reply statistics.
    pub stats: ReadStats,
    /// Firmware version the opening chain answered with (`None` when the chain was skipped or the
    /// keyboard did not answer the `0x90` poll).
    pub firmware: Option<String>,
}

impl ProfileImage {
    /// Assemble collected records into the profile image and tally reply statistics.
    ///
    /// Reads are scattered by their file offset; reads past [`PROFILE_FILE_BYTES`] are kept in the
    /// records and dropped from the assembled data.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConfig`] when `profile_slot` is out of range.
    pub fn assemble(profile_slot: u8, chunks: Vec<ChunkRecord>) -> Result<Self> {
        validate_profile(profile_slot)?;

        let mut data = vec![0u8; PROFILE_FILE_BYTES];
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
                    PAYLOAD_READ_LEN as u16,
                )
                .is_some()
                {
                    header_mismatches += 1;
                }
                if reply.all_zero {
                    zero_reads += 1;
                }
                if reply.data.len() < PAYLOAD_READ_LEN {
                    short_reads += 1;
                }
                if reply.data.is_empty() {
                    continue;
                }
                if cursor >= PROFILE_FILE_BYTES {
                    continue;
                }
                let end = cursor + reply.data.len();
                if reply.all_ff {
                    // A page whose first four bytes are FF is garbage from the device's point of view; the
                    // reference implementation replaces it with zeroes and so do we.
                    garbage_reads += 1;
                    let room = PROFILE_FILE_BYTES.saturating_sub(cursor);
                    data[cursor..cursor + room].fill(0);
                    cursor += room;
                } else if end > PROFILE_FILE_BYTES {
                    let room = PROFILE_FILE_BYTES.saturating_sub(cursor);
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
            profile_slot,
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
            firmware: None,
        })
    }

    /// Record the firmware version the opening chain answered with.
    #[must_use]
    pub fn with_firmware(mut self, firmware: Option<String>) -> Self {
        self.firmware = firmware;
        self
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
/// Profile name of a payload: the [`NAME_LEN`] bytes at [`NAME_OFFSET`], cut at the first NUL. An
/// empty string when the payload is too short or the name region is blank.
#[must_use]
pub fn profile_name(data: &[u8]) -> String {
    if data.len() <= NAME_OFFSET {
        return String::new();
    }
    let raw = &data[NAME_OFFSET..(NAME_OFFSET + NAME_LEN).min(data.len())];
    let name_end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..name_end]).into_owned()
}
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
        "profile_slot": image.profile_slot,
        "fs_id": format!("0x{:04X}", profile_file_id(image.profile_slot)),
        "schema": format!("0x{schema:04X}"),
        "schema_decoded": decoded,
        "firmware": image.firmware,
        "data_len": payload.len(),
        "sha_like_fnv1a_64": fnv1a_64(payload),
        "stats": image.stats,
        "chunks": image.chunks,
        "head_hex": hex_of(&payload[..payload.len().min(64)]),
    });
    serde_json::to_string_pretty(&value).map_err(|e| Error::Profile(format!("serialize profile image: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::hid_reports::{
        CommandCode, HidCommand, HidDeviceType, ONBOARD_FEATURE_REPORT_SIZE, OnboardPullCommand, OnboardReadCommand,
    };

    /// The significant frame bytes a typed command serializes, behind its report-ID byte.
    fn frame_of<C: HidCommand>(command: C, frame_len: usize) -> Vec<u8> {
        let mut buffer = vec![0u8; ONBOARD_FEATURE_REPORT_SIZE];
        let size = command
            .serialize(&mut buffer, HidDeviceType::Keyboard)
            .expect("the read plan sends this frame, so it is serializable");
        assert!(frame_len < size, "{} fits in {size} bytes", command.description());
        buffer[1..frame_len + 1].to_vec()
    }

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
    fn pull_and_read_frames_come_from_the_typed_commands() {
        let pull = frame_of(
            OnboardPullCommand {
                fs_id: profile_file_id(0),
                file_offset: 0,
                size: DEVICE_RESOURCE_BUFFER_LEN as u16,
            },
            OnboardPullCommand::FRAME_LEN,
        );
        assert_eq!(pull[0], CommandCode::OnboardPull as u8);
        assert_eq!(
            u16::from_le_bytes([pull[2], pull[3]]),
            PROFILE_FS_BASE,
            "the file id sits behind the header"
        );
        assert_eq!(pull.len(), OnboardPullCommand::FRAME_LEN);
        let read = frame_of(
            OnboardReadCommand {
                buffer_offset: 0,
                size: PAYLOAD_READ_LEN as u16,
            },
            OnboardReadCommand::FRAME_LEN,
        );
        assert_eq!(read[0], CommandCode::OnboardPayloadRead as u8);
        assert_eq!(read.len(), OnboardReadCommand::FRAME_LEN);
    }

    #[test]
    fn pulls_walk_the_resource_buffer_and_carry_the_slot_file_id() {
        let slot = 3;
        assert_eq!(profile_file_id(slot), 0x0204);
        for n in 0..PULLS_PER_FILE {
            let pull = frame_of(
                OnboardPullCommand {
                    fs_id: profile_file_id(slot),
                    file_offset: (n * DEVICE_RESOURCE_BUFFER_LEN) as u32,
                    size: DEVICE_RESOURCE_BUFFER_LEN as u16,
                },
                OnboardPullCommand::FRAME_LEN,
            );
            assert_eq!(
                u16::from_le_bytes([pull[2], pull[3]]),
                0x0204,
                "every pull carries the slot file id"
            );
            assert_eq!(
                u32::from_le_bytes([pull[4], pull[5], pull[6], pull[7]]),
                (n * DEVICE_RESOURCE_BUFFER_LEN) as u32,
                "pull n fills the buffer at file offset n * DEVICE_RESOURCE_BUFFER_LEN"
            );
            assert_eq!(
                u16::from_le_bytes([pull[8], pull[9]]),
                DEVICE_RESOURCE_BUFFER_LEN as u16,
                "pull size"
            );
        }
        for n in 0..READS_PER_PULL {
            let read = frame_of(
                OnboardReadCommand {
                    buffer_offset: (n * PAYLOAD_READ_LEN) as u16,
                    size: PAYLOAD_READ_LEN as u16,
                },
                OnboardReadCommand::FRAME_LEN,
            );
            assert_eq!(
                u16::from_le_bytes([read[2], read[3]]),
                (n * PAYLOAD_READ_LEN) as u16,
                "read n asks for the n-th window of the resource buffer"
            );
            assert_eq!(u16::from_le_bytes([read[4], read[5]]), PAYLOAD_READ_LEN as u16);
        }
        assert_eq!(
            PROFILE_FILE_BYTES,
            PULLS_PER_FILE * DEVICE_RESOURCE_BUFFER_LEN,
            "the fixed read covers 12 x 1024 file bytes"
        );
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
    fn capture_reply_payload_starts_behind_the_report_id_byte() {
        // The GET_REPORT 0x0300 answer to `83 00 00 00 00 02` on file 0x0201 in the 2026-10-15 capture:
        // report ID 03, then the schema word 0x000B, then the ASCII profile name 2 bytes behind it.
        let mut reply = vec![0x03u8, 0x0b, 0x00];
        reply.extend(b"Config 1");
        reply.resize(READ_REPORT_LEN, 0);
        let parsed = parse_read_reply(&reply, 512).unwrap();
        assert_eq!(parsed.report_id, 0x03, "no header is echoed");
        assert!(!parsed.header);
        assert_eq!(schema_of(&parsed.data), SCHEMA_B);
        assert_eq!(&parsed.data[2..10], b"Config 1");
        assert_eq!(parsed.data.len(), 512);
        assert!(!parsed.all_zero);
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

    fn record(chunk: usize, data: impl Fn(usize) -> Vec<u8>) -> ChunkRecord {
        ChunkRecord {
            index: chunk,
            fs_id: profile_file_id(0),
            file_offset: (chunk * DEVICE_RESOURCE_BUFFER_LEN) as u32,
            ack_report_id: 0x85,
            ack_hex: "85 00".to_string(),
            reads: (0..READS_PER_PULL)
                .map(|read| {
                    let data = data(read);
                    let all_zero = data.iter().all(|b| *b == 0);
                    ReadReply {
                        report_id: 0x83,
                        reserved: 0,
                        header: true,
                        offset: (read * PAYLOAD_READ_LEN) as u16,
                        size: PAYLOAD_READ_LEN as u16,
                        all_ff: data.starts_with(&[0xFF, 0xFF, 0xFF, 0xFF]),
                        all_zero,
                        data,
                    }
                })
                .collect(),
        }
    }

    #[test]
    fn assemble_scatters_reads_by_file_offset() {
        let records = (0..PULLS_PER_FILE)
            .map(|chunk| {
                record(chunk, |read| {
                    vec![(chunk * READS_PER_PULL + read + 1) as u8; PAYLOAD_READ_LEN]
                })
            })
            .collect::<Vec<_>>();
        let image = ProfileImage::assemble(0, records).unwrap();
        assert_eq!(image.data.len(), PROFILE_FILE_BYTES);
        assert_eq!(image.data[0], 1);
        assert_eq!(image.data[PAYLOAD_READ_LEN], 2);
        assert_eq!(image.data[DEVICE_RESOURCE_BUFFER_LEN], 3);
        assert_eq!(
            image.data[PROFILE_FILE_BYTES - 1],
            (PULLS_PER_FILE * READS_PER_PULL) as u8
        );
        assert_eq!(image.stats.chunks_read, PULLS_PER_FILE);
        assert_eq!(image.stats.reads_read, PULLS_PER_FILE * READS_PER_PULL);
        assert_eq!(image.stats.header_mismatches, 0);
        assert_eq!(image.stats.zero_chunks, 0);
        assert_eq!(image.stats.covered_len, PROFILE_FILE_BYTES);
        assert_eq!(image.profile_slot, 0);
    }

    #[test]
    fn assemble_drops_reads_past_the_file_and_clips_the_last_one() {
        let mut records = (0..PULLS_PER_FILE)
            .map(|chunk| record(chunk, |_| vec![0u8; PAYLOAD_READ_LEN]))
            .collect::<Vec<_>>();
        // One pull beyond the file: its reads are recorded and stay out of the assembled bytes.
        records.push(ChunkRecord {
            index: PULLS_PER_FILE,
            fs_id: profile_file_id(0),
            file_offset: PROFILE_FILE_BYTES as u32,
            ack_report_id: 0x85,
            ack_hex: "85 00".to_string(),
            reads: (0..READS_PER_PULL)
                .map(|read| ReadReply {
                    report_id: 0x83,
                    reserved: 0,
                    header: true,
                    offset: (read * PAYLOAD_READ_LEN) as u16,
                    size: PAYLOAD_READ_LEN as u16,
                    data: vec![0xAA_u8; PAYLOAD_READ_LEN],
                    all_zero: false,
                    all_ff: false,
                })
                .collect(),
        });
        let image = ProfileImage::assemble(0, records).unwrap();
        assert_eq!(image.data.len(), PROFILE_FILE_BYTES);
        assert!(
            image.data.iter().all(|b| *b == 0),
            "the extra pull contributes no bytes"
        );
        assert_eq!(image.stats.chunks_read, PULLS_PER_FILE + 1);
        assert_eq!(
            image.stats.reads_read,
            (PULLS_PER_FILE + 1) * READS_PER_PULL,
            "the extra reads are still tallied in the statistics"
        );
    }

    #[test]
    fn only_the_profile_slot_is_selectable() {
        for slot in 0..=MAX_PROFILE_INDEX {
            assert!(validate_profile(slot).is_ok(), "slot {slot} is readable");
        }
        let err = validate_profile(MAX_PROFILE_INDEX + 1).unwrap_err().to_string();
        assert!(err.contains("highest slot"), "{err}");
        assert!(validate_profile(255).is_err());
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

        let image = ProfileImage::assemble(0, {
            let reads_per_pull = READS_PER_PULL;
            (0..PULLS_PER_FILE)
                .map(|chunk| ChunkRecord {
                    index: chunk,
                    fs_id: 0x0201,
                    file_offset: (chunk * DEVICE_RESOURCE_BUFFER_LEN) as u32,
                    ack_report_id: 0x85,
                    ack_hex: "85 00".to_string(),
                    reads: (0..reads_per_pull)
                        .map(|read| {
                            let i = chunk * reads_per_pull + read;
                            let start = i * PAYLOAD_READ_LEN;
                            let data_slice = data
                                .get(start..start + PAYLOAD_READ_LEN)
                                .map(|s| s.to_vec())
                                .unwrap_or_else(|| vec![0u8; PAYLOAD_READ_LEN]);
                            ReadReply {
                                report_id: 0x83,
                                reserved: 0,
                                header: true,
                                offset: (read * PAYLOAD_READ_LEN) as u16,
                                size: PAYLOAD_READ_LEN as u16,
                                all_zero: data_slice.iter().all(|b| *b == 0),
                                all_ff: false,
                                data: data_slice,
                            }
                        })
                        .collect(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap();
        let raw = to_json(&image).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["schema_decoded"], "b");
        assert_eq!(value["profile_slot"], 0);
        assert_eq!(value["fs_id"], "0x0201");
        assert_eq!(value["data_len"], PROFILE_FILE_BYTES);
    }

    #[test]
    fn image_diff_finds_the_first_difference() {
        let record = |first: u8| ChunkRecord {
            index: 0,
            fs_id: profile_file_id(0),
            file_offset: 0,
            ack_report_id: 0x85,
            ack_hex: "85".to_string(),
            reads: (0..(DEVICE_RESOURCE_BUFFER_LEN / PAYLOAD_READ_LEN) * PULLS_PER_FILE)
                .map(|i| ReadReply {
                    report_id: 0x83,
                    reserved: 0,
                    header: true,
                    offset: ((i % 2) * PAYLOAD_READ_LEN) as u16,
                    size: PAYLOAD_READ_LEN as u16,
                    data: vec![if i == 3 { first } else { 0u8 }; PAYLOAD_READ_LEN],
                    all_zero: i != 3,
                    all_ff: false,
                })
                .collect(),
        };
        let a = ProfileImage::assemble(0, vec![record(1)]).unwrap();
        let b = ProfileImage::assemble(0, vec![record(1)]).unwrap();
        let c = ProfileImage::assemble(0, vec![record(2)]).unwrap();
        assert_eq!(image_diff(&a, &b), None);
        assert_eq!(image_diff(&a, &c), Some(3 * PAYLOAD_READ_LEN));
        assert_eq!(a.diff(&c), Some(3 * PAYLOAD_READ_LEN));
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
