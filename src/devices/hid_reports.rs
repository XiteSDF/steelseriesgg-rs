//! Structured HID report types for SteelSeries devices.
//!
//! This module provides type-safe HID command construction and validation
//! for SteelSeries keyboards and headsets, replacing primitive byte array building.

use super::key_mapping::{KeyAddress, KeyId, KeyMapping};
use crate::rgb::Color;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

/// Standard HID report size for SteelSeries keyboards (includes report ID).
pub const KEYBOARD_REPORT_SIZE: usize = 65;

/// Standard HID report size for SteelSeries headsets (no report ID).
pub const HEADSET_REPORT_SIZE: usize = 64;

/// Maximum number of RGB zones supported by keyboards.
pub const MAX_RGB_ZONES: usize = 12;

/// SteelSeries HID command codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum CommandCode {
    /// Apply/Save settings (0x09)
    Apply = 0x09,
    /// RGB zone control (0x21)
    RgbControl = 0x21,
    /// Brightness control (0x22)
    Brightness = 0x22,
    /// Reactive mode (0x25)
    ReactiveMode = 0x25,
    /// Color shift (0x26)
    ColorShift = 0x26,
    /// Per-key RGB control (0x23)
    /// NOTE: This command code is a placeholder. The actual per-key RGB command
    /// for SteelSeries keyboards has not been discovered and may be different.
    PerKeyRgb = 0x23,
    /// [EXPERIMENTAL] Apex Pro TKL 2023 direct-mode lighting frame (0x40), sent as a feature
    /// report by `ApexProTkl2023`. See docs/development/apex-pro-tkl-2023-gg-capture.md.
    Apex2023Direct = 0x40,
    /// Actuation point control (0x2D) - EXPERIMENTAL
    /// NOTE: This command code is experimental based on hardware research.
    ActuationControl = 0x2D,
    /// [EXPERIMENTAL] Apex Pro TKL 2023 on-board profile chunk (0x03, header `03 01 80`), see
    /// `ProfileChunkCommand`.
    ProfileChunk = 0x03,
    /// [EXPERIMENTAL] Legacy Apex (Apex 7, Apex Raw, Apex 2014) resource-buffer pull (0x85), see
    /// `OnboardPullCommand`.
    OnboardPull = 0x85,
    /// [EXPERIMENTAL] Legacy Apex on-board payload read (0x83), see `OnboardReadCommand`.
    OnboardPayloadRead = 0x83,
    /// [EXPERIMENTAL] Legacy Apex opening-chain firmware-version poll (0x90), answered on the interrupt
    /// endpoint with the ASCII firmware version. See `ONBOARD_INIT_CHAIN`.
    FirmwareVersion = 0x90,
    /// [EXPERIMENTAL] Legacy Apex opening-chain frame (0xF4), answered `00 01 00…`; meaning unknown.
    EngineStatus = 0xF4,
    /// [EXPERIMENTAL] Legacy Apex opening-chain frame (0x6C); no answer was ever captured.
    OpenFiles = 0x6C,
}

impl fmt::Display for CommandCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandCode::Apply => write!(f, "APPLY"),
            CommandCode::RgbControl => write!(f, "RGB_CTRL"),
            CommandCode::Brightness => write!(f, "BRIGHTNESS"),
            CommandCode::ReactiveMode => write!(f, "REACTIVE"),
            CommandCode::ColorShift => write!(f, "COLOR_SHIFT"),
            CommandCode::PerKeyRgb => write!(f, "PERKEY_RGB_EXPERIMENTAL"),
            CommandCode::Apex2023Direct => write!(f, "APEX2023_DIRECT_EXPERIMENTAL"),
            CommandCode::ActuationControl => write!(f, "ACTUATION_CTRL_EXPERIMENTAL"),
            CommandCode::ProfileChunk => write!(f, "PROFILE_CHUNK_EXPERIMENTAL"),
            CommandCode::OnboardPull => write!(f, "ONBOARD_PULL_EXPERIMENTAL"),
            CommandCode::OnboardPayloadRead => write!(f, "ONBOARD_PAYLOAD_READ_EXPERIMENTAL"),
            CommandCode::FirmwareVersion => write!(f, "FIRMWARE_VERSION_EXPERIMENTAL"),
            CommandCode::EngineStatus => write!(f, "ENGINE_STATUS_EXPERIMENTAL"),
            CommandCode::OpenFiles => write!(f, "OPEN_FILES_EXPERIMENTAL"),
        }
    }
}

impl CommandCode {
    /// Parse a command byte found in a HID report.
    ///
    /// This keeps report-byte handling in one place, including compatibility
    /// with legacy captures that have used `0x2A` for placeholder per-key RGB.
    pub const fn from_report_byte(value: u8) -> Option<Self> {
        match value {
            0x09 => Some(CommandCode::Apply),
            0x21 => Some(CommandCode::RgbControl),
            0x22 => Some(CommandCode::Brightness),
            0x25 => Some(CommandCode::ReactiveMode),
            0x26 => Some(CommandCode::ColorShift),
            0x23 | 0x2A => Some(CommandCode::PerKeyRgb),
            0x40 => Some(CommandCode::Apex2023Direct),
            0x2D => Some(CommandCode::ActuationControl),
            0x03 => Some(CommandCode::ProfileChunk),
            0x85 => Some(CommandCode::OnboardPull),
            0x83 => Some(CommandCode::OnboardPayloadRead),
            0x90 => Some(CommandCode::FirmwareVersion),
            0xF4 => Some(CommandCode::EngineStatus),
            0x6C => Some(CommandCode::OpenFiles),
            _ => None,
        }
    }
}

/// Device type for HID report sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidDeviceType {
    /// Keyboard (65-byte reports with report ID)
    Keyboard,
    /// Headset (64-byte reports without report ID)
    Headset,
}

impl HidDeviceType {
    /// Get the report size for this device type.
    pub fn report_size(self) -> usize {
        match self {
            HidDeviceType::Keyboard => KEYBOARD_REPORT_SIZE,
            HidDeviceType::Headset => HEADSET_REPORT_SIZE,
        }
    }

    /// Whether this device type includes a report ID byte.
    pub fn includes_report_id(self) -> bool {
        matches!(self, HidDeviceType::Keyboard)
    }
}

/// A structured HID command that can be serialized to bytes.
pub trait HidCommand {
    /// Get the command code for this command.
    fn command_code(&self) -> CommandCode;

    /// Serialize the command to a byte buffer.
    /// Returns the number of bytes written.
    fn serialize(&self, buffer: &mut [u8], device_type: HidDeviceType) -> Result<usize>;

    /// Validate the command parameters.
    fn validate(&self) -> Result<()>;

    /// Get a human-readable description of the command.
    fn description(&self) -> String;
}

/// RGB zone control command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbZoneCommand<'a> {
    /// Zone colors (up to MAX_RGB_ZONES)
    pub colors: Cow<'a, [Color]>,
    /// Zone selector (0xFF for all zones)
    pub zone_selector: u8,
}

impl<'a> RgbZoneCommand<'a> {
    /// Create a new RGB zone command for all zones.
    pub fn new_all_zones(colors: &'a [Color]) -> Self {
        Self {
            colors: Cow::Borrowed(colors),
            zone_selector: 0xFF,
        }
    }

    /// Create a new RGB zone command with a single color for all zones.
    pub fn new_single_color(color: Color, zone_count: usize) -> Self {
        Self {
            colors: Cow::Owned(vec![color; zone_count]),
            zone_selector: 0xFF,
        }
    }

    /// Create a new RGB zone command for a specific zone.
    pub fn new_specific_zone(zone_index: u8, color: Color) -> Self {
        let mut colors = vec![Color::BLACK; MAX_RGB_ZONES];
        if zone_index < MAX_RGB_ZONES as u8 {
            colors[zone_index as usize] = color;
        }

        Self {
            colors: Cow::Owned(colors),
            zone_selector: zone_index,
        }
    }
}

impl<'a> HidCommand for RgbZoneCommand<'a> {
    fn command_code(&self) -> CommandCode {
        CommandCode::RgbControl
    }

    fn serialize(&self, buffer: &mut [u8], device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;

        let report_size = device_type.report_size();
        if buffer.len() < report_size {
            return Err(Error::DeviceCommunication(format!(
                "Buffer too small: {} bytes (expected {})",
                buffer.len(),
                report_size
            )));
        }

        buffer[..report_size].fill(0);

        let mut offset = 0;

        if device_type.includes_report_id() {
            buffer[0] = 0x00;
            offset = 1;
        }

        buffer[offset] = self.command_code() as u8;
        offset += 1;

        buffer[offset] = self.zone_selector;
        offset += 1;

        for color in self.colors.iter() {
            if offset + 2 >= report_size {
                break;
            }
            buffer[offset] = color.r;
            buffer[offset + 1] = color.g;
            buffer[offset + 2] = color.b;
            offset += 3;
        }

        Ok(report_size)
    }

    fn validate(&self) -> Result<()> {
        if self.colors.is_empty() {
            return Err(Error::DeviceCommunication(
                "RGB command must have at least one color".to_string(),
            ));
        }

        if self.colors.len() > MAX_RGB_ZONES {
            return Err(Error::DeviceCommunication(format!(
                "Too many RGB zones: {} (max {})",
                self.colors.len(),
                MAX_RGB_ZONES
            )));
        }

        Ok(())
    }

    fn description(&self) -> String {
        if self.zone_selector == 0xFF {
            format!("Set {} zones to RGB colors", self.colors.len())
        } else {
            format!("Set zone {} to RGB color", self.zone_selector)
        }
    }
}

/// Brightness control command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrightnessCommand {
    /// Brightness value (0-100)
    pub brightness: u8,
}

impl BrightnessCommand {
    /// Create a new brightness command.
    pub fn new(brightness: u8) -> Self {
        Self {
            brightness: brightness.min(100), // Auto-clamp
        }
    }
}

impl HidCommand for BrightnessCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::Brightness
    }

    fn serialize(&self, buffer: &mut [u8], device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;

        let report_size = device_type.report_size();
        if buffer.len() < report_size {
            return Err(Error::DeviceCommunication(format!(
                "Buffer too small: {} bytes (expected {})",
                buffer.len(),
                report_size
            )));
        }

        buffer[..report_size].fill(0);
        let mut offset = 0;

        // Add report ID for keyboards
        if device_type.includes_report_id() {
            buffer[0] = 0x00; // Report ID
            offset = 1;
        }

        // Command code and brightness
        buffer[offset] = self.command_code() as u8;
        buffer[offset + 1] = self.brightness;

        Ok(report_size)
    }

    fn validate(&self) -> Result<()> {
        if self.brightness > 100 {
            return Err(Error::DeviceCommunication(format!(
                "Invalid brightness value: {} (max 100)",
                self.brightness
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set brightness to {}%", self.brightness)
    }
}

/// [EXPERIMENTAL] Brightness for 8-zone Apex keyboards (Apex 3 TKL): `00 23 <0x00-0x10>`.
///
/// Source: OpenRGB `SteelSeriesApex8ZoneController.cpp` (`SetBrightness`), untested on hardware.
/// On this controller `0x22` (`CommandCode::Brightness`) selects the rainbow-wave mode instead.
/// `CommandCode` cannot hold a second `0x23` variant, so `command_code()` (used only for logging) reports
/// `PerKeyRgb`; the opcode is written from `OPCODE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Apex8ZoneBrightnessCommand {
    /// Device brightness level (0x00-0x10).
    pub level: u8,
}

impl Apex8ZoneBrightnessCommand {
    /// Maximum device brightness level.
    pub const MAX_LEVEL: u8 = 0x10;
    /// Brightness opcode on 8-zone controllers. Same byte as the `PerKeyRgb` placeholder, unrelated meaning.
    const OPCODE: u8 = 0x23;

    /// Create from a 0-100 percentage, rounded to the nearest device level.
    pub fn from_percent(percent: u8) -> Self {
        let percent = u16::from(percent.min(100));
        Self {
            level: ((percent * u16::from(Self::MAX_LEVEL) + 50) / 100) as u8,
        }
    }
}

impl HidCommand for Apex8ZoneBrightnessCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::PerKeyRgb
    }

    fn serialize(&self, buffer: &mut [u8], device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;

        let report_size = device_type.report_size();
        if buffer.len() < report_size {
            return Err(Error::DeviceCommunication(format!(
                "Buffer too small: {} bytes (expected {})",
                buffer.len(),
                report_size
            )));
        }

        buffer[..report_size].fill(0);
        let offset = usize::from(device_type.includes_report_id());
        buffer[offset] = Self::OPCODE;
        buffer[offset + 1] = self.level;

        Ok(report_size)
    }

    fn validate(&self) -> Result<()> {
        if self.level > Self::MAX_LEVEL {
            return Err(Error::DeviceCommunication(format!(
                "Invalid 8-zone brightness level: {} (max {})",
                self.level,
                Self::MAX_LEVEL
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set 8-zone brightness to level {}", self.level)
    }
}

/// [EXPERIMENTAL] Report-ID byte plus the 644 wire bytes of one profile chunk feature report.
pub const PROFILE_CHUNK_REPORT_SIZE: usize = 645;

/// [EXPERIMENTAL] One chunk of the wired Apex Pro TKL 2023 (PID 0x1628) on-board profile upload, a feature
/// report: `00 | 03 01 80 | len u16 LE | offset u16 LE | 00 00 | data | zero padding`.
///
/// Layout from GG USBPcap captures (2026-10-14). Nothing sends it: no profile read-back is known, so a write
/// would replace the board's other settings (docs/development/apex-pro-tkl-2023-gg-capture.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChunkCommand<'a> {
    /// Byte offset of `data` in the 12,500-byte profile blob.
    pub offset: u16,
    /// Chunk payload (GG always sent 500 bytes).
    pub data: &'a [u8],
}

impl ProfileChunkCommand<'_> {
    /// Largest payload seen in a chunk.
    pub const MAX_DATA_LEN: usize = 500;
    const HEADER: [u8; 3] = [CommandCode::ProfileChunk as u8, 0x01, 0x80];
    const DATA_OFFSET: usize = 10;
}

impl HidCommand for ProfileChunkCommand<'_> {
    fn command_code(&self) -> CommandCode {
        CommandCode::ProfileChunk
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        if buffer.len() < PROFILE_CHUNK_REPORT_SIZE {
            return Err(Error::DeviceCommunication(format!(
                "Buffer too small: {} bytes (expected {})",
                buffer.len(),
                PROFILE_CHUNK_REPORT_SIZE
            )));
        }

        let report = &mut buffer[..PROFILE_CHUNK_REPORT_SIZE];
        report.fill(0);
        report[1..4].copy_from_slice(&Self::HEADER);
        report[4..6].copy_from_slice(&(self.data.len() as u16).to_le_bytes());
        report[6..8].copy_from_slice(&self.offset.to_le_bytes());
        report[Self::DATA_OFFSET..Self::DATA_OFFSET + self.data.len()].copy_from_slice(self.data);
        Ok(PROFILE_CHUNK_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if self.data.is_empty() || self.data.len() > Self::MAX_DATA_LEN {
            return Err(Error::DeviceCommunication(format!(
                "Profile chunk must carry 1-{} bytes, got {}",
                Self::MAX_DATA_LEN,
                self.data.len()
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Profile chunk: {} bytes at offset {}", self.data.len(), self.offset)
    }
}

/// Report buffer of a legacy Apex (Apex 7, Apex Raw, Apex 2014) on-board profile transfer, handed to
/// the transport as `[report number][64 data bytes]` with report number 0, which is the
/// `SET_REPORT wValue 0x0200 wIndex 1 wLength 64` transfer the 2026-10-15 reference capture shows.
///
/// `[EXPERIMENTAL]` Layout and sizes from
/// `docs/development/apex-7-onboard-profile-read.md`. Only the opening-chain frames have been answered by
/// a keyboard when sent by `ssgg` (firmware `4.16.8` on 2026-10-15).
pub const ONBOARD_OUTPUT_REPORT_SIZE: usize = 65;

/// Report buffer of a legacy Apex on-board payload read: `[report number][642 data bytes]` with report
/// number 0, which is the `SET_REPORT wValue 0x0300 wLength 642` transfer the reference capture shows
/// (the kernel takes the report number out of the data stage, so the buffer is one byte longer than the
/// wire, and the 642 data bytes carry the frame zero padded).
///
/// `[EXPERIMENTAL]` Never reproduced on hardware.
pub const ONBOARD_FEATURE_REPORT_SIZE: usize = 643;

/// Report length to ask for when reading a payload read back with `GET_REPORT wValue 0x0300 wIndex 1`
/// (the capture's reply is a 643-byte data stage, report-ID byte plus 642 payload bytes). Measured on
/// Linux 2026-10-15: 642 answers, **643 makes the control transfer never complete**.
pub const ONBOARD_REPLY_REPORT_SIZE: usize = 642;

/// Bytes a payload read may ask for: a reply report carries the payload behind its report-ID byte.
pub const ONBOARD_MAX_READ_LEN: usize = ONBOARD_REPLY_REPORT_SIZE - 1;

/// One frame of the opening chain a legacy Apex keyboard expects in front of a profile read,
/// `[EXPERIMENTAL]`: report number 0, the command byte, two argument bytes, zero padded to
/// [`ONBOARD_OUTPUT_REPORT_SIZE`].
///
/// The frame bytes are captured; the meanings of [`CommandCode::EngineStatus`] and
/// [`CommandCode::OpenFiles`] are unknown, and no answer to [`CommandCode::OpenFiles`] was ever captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardInitCommand {
    /// Command byte, one of [`CommandCode::FirmwareVersion`], [`CommandCode::EngineStatus`],
    /// [`CommandCode::OpenFiles`].
    pub command: CommandCode,
    /// The two bytes behind the command byte.
    pub args: [u8; 2],
    /// True when the capture shows an interrupt-endpoint answer to this frame.
    pub expects_reply: bool,
}

impl OnboardInitCommand {
    /// Significant frame bytes behind the report number: command byte and its two arguments.
    pub const FRAME_LEN: usize = 3;

    /// Frame bytes behind the report number, for plan reporting. Built by [`HidCommand::serialize`],
    /// the same source as the wire bytes.
    #[must_use]
    pub fn data_stage(&self) -> [u8; Self::FRAME_LEN] {
        let mut buffer = [0u8; ONBOARD_OUTPUT_REPORT_SIZE];
        serialize_stage(*self, &mut buffer);
        let mut frame = [0u8; Self::FRAME_LEN];
        frame.copy_from_slice(&buffer[1..Self::FRAME_LEN + 1]);
        frame
    }
}

impl HidCommand for OnboardInitCommand {
    fn command_code(&self) -> CommandCode {
        self.command
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        check_report_buffer(buffer, ONBOARD_OUTPUT_REPORT_SIZE)?;
        let report = &mut buffer[..ONBOARD_OUTPUT_REPORT_SIZE];
        report.fill(0);
        report[1] = self.command as u8;
        report[2..Self::FRAME_LEN + 1].copy_from_slice(&self.args);
        Ok(ONBOARD_OUTPUT_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        match self.command {
            CommandCode::FirmwareVersion | CommandCode::EngineStatus | CommandCode::OpenFiles => Ok(()),
            other => Err(Error::InvalidConfig(format!(
                "{other} is not a legacy Apex opening-chain command"
            ))),
        }
    }

    fn description(&self) -> String {
        format!("On-board opening chain: {} {:02X?}", self.command, self.args)
    }
}

/// The opening chain of a legacy Apex profile read, in wire order, `[EXPERIMENTAL]`: six 64-byte
/// output reports, 11 ms apart in the capture. Bytes from `dev.pcap` (device 7, `1792014261.316` to
/// `1792014261.390`); the `FirmwareVersion` polls are answered on the interrupt endpoint with the ASCII
/// firmware version, `EngineStatus` answers `00 01 00…`, `OpenFiles` is not answered at all.
///
/// The capture contains no 13-byte transfer anywhere, so the pacing frame earlier versions of the read
/// path sent is not part of the keyboard's procedure.
pub const ONBOARD_INIT_CHAIN: [OnboardInitCommand; 6] = [
    OnboardInitCommand {
        command: CommandCode::FirmwareVersion,
        args: [0x00, 0],
        expects_reply: true,
    },
    OnboardInitCommand {
        command: CommandCode::FirmwareVersion,
        args: [0x01, 0],
        expects_reply: true,
    },
    OnboardInitCommand {
        command: CommandCode::EngineStatus,
        args: [0, 0],
        expects_reply: true,
    },
    OnboardInitCommand {
        command: CommandCode::OpenFiles,
        args: [0, 1],
        expects_reply: false,
    },
    OnboardInitCommand {
        command: CommandCode::FirmwareVersion,
        args: [0x00, 0],
        expects_reply: true,
    },
    OnboardInitCommand {
        command: CommandCode::FirmwareVersion,
        args: [0x01, 0],
        expects_reply: true,
    },
];

/// Copy part of a profile file into the keyboard's resource buffer, `[EXPERIMENTAL]`:
/// `85 | 00 | file id u16 LE | file offset u32 LE | size u16 LE | 00…` inside
/// [`ONBOARD_OUTPUT_REPORT_SIZE`]. The keyboard answers with a 64-byte input report whose contents are
/// not interpreted here.
///
/// Sizes and frame layout from the 2026-10-15 reference capture (121 pulls of 1024 bytes at file
/// offsets 0, 4, 8, … 44 for the schema probe and 0..11264 for full files).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardPullCommand {
    /// Profile file id: the file base plus the profile index, and the same ids at +0x0100 for the
    /// lighting files.
    pub fs_id: u16,
    /// Offset in the profile file.
    pub file_offset: u32,
    /// Bytes to copy into the resource buffer.
    pub size: u16,
}

impl OnboardPullCommand {
    /// Significant frame bytes behind the report number.
    pub const FRAME_LEN: usize = 10;
    /// Largest pull the capture shows, the size of the keyboard's resource buffer.
    pub const MAX_SIZE: u16 = 1024;

    /// Frame bytes behind the report number, for plan reporting. Built by [`HidCommand::serialize`].
    #[must_use]
    pub fn data_stage(&self) -> [u8; Self::FRAME_LEN] {
        let mut buffer = [0u8; ONBOARD_OUTPUT_REPORT_SIZE];
        serialize_stage(*self, &mut buffer);
        let mut frame = [0u8; Self::FRAME_LEN];
        frame.copy_from_slice(&buffer[1..Self::FRAME_LEN + 1]);
        frame
    }
}

impl HidCommand for OnboardPullCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::OnboardPull
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        check_report_buffer(buffer, ONBOARD_OUTPUT_REPORT_SIZE)?;
        let report = &mut buffer[..ONBOARD_OUTPUT_REPORT_SIZE];
        report.fill(0);
        report[1] = CommandCode::OnboardPull as u8;
        report[3..5].copy_from_slice(&self.fs_id.to_le_bytes());
        report[5..9].copy_from_slice(&self.file_offset.to_le_bytes());
        report[9..Self::FRAME_LEN + 1].copy_from_slice(&self.size.to_le_bytes());
        Ok(ONBOARD_OUTPUT_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if self.size > Self::MAX_SIZE {
            return Err(Error::InvalidConfig(format!(
                "pull of {} bytes exceeds the {}-byte resource buffer",
                self.size,
                Self::MAX_SIZE
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!(
            "On-board pull: file {:04X} offset {} size {}",
            self.fs_id, self.file_offset, self.size
        )
    }
}

/// Read part of the keyboard's resource buffer, `[EXPERIMENTAL]`: `83 | 00 | buffer offset u16 LE |
/// size u16 LE | 00…` inside [`ONBOARD_FEATURE_REPORT_SIZE`], sent as a feature report on report number
/// 0, and answered by a `GET_REPORT wValue 0x0300 wIndex 1 wLength 642` read-back (see
/// [`ONBOARD_FEATURE_REPORT_SIZE`]).
///
/// `[EXPERIMENTAL]` The reference capture shows every read answered with the file head (schema word and
/// profile name) and zeros, never the requested window; the full read has never completed on hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardReadCommand {
    /// Offset in the resource buffer.
    pub buffer_offset: u16,
    /// Bytes to return.
    pub size: u16,
}

impl OnboardReadCommand {
    /// Significant frame bytes behind the report number.
    pub const FRAME_LEN: usize = 6;

    /// Frame bytes behind the report number, for plan reporting. Built by [`HidCommand::serialize`].
    #[must_use]
    pub fn data_stage(&self) -> [u8; Self::FRAME_LEN] {
        let mut buffer = [0u8; ONBOARD_FEATURE_REPORT_SIZE];
        serialize_stage(*self, &mut buffer);
        let mut frame = [0u8; Self::FRAME_LEN];
        frame.copy_from_slice(&buffer[1..Self::FRAME_LEN + 1]);
        frame
    }
}

impl HidCommand for OnboardReadCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::OnboardPayloadRead
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        check_report_buffer(buffer, ONBOARD_FEATURE_REPORT_SIZE)?;
        let report = &mut buffer[..ONBOARD_FEATURE_REPORT_SIZE];
        report.fill(0);
        report[1] = CommandCode::OnboardPayloadRead as u8;
        report[3..5].copy_from_slice(&self.buffer_offset.to_le_bytes());
        report[5..Self::FRAME_LEN + 1].copy_from_slice(&self.size.to_le_bytes());
        Ok(ONBOARD_FEATURE_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if usize::from(self.size) > ONBOARD_MAX_READ_LEN {
            return Err(Error::InvalidConfig(format!(
                "payload read of {} bytes exceeds the {} bytes one reply report can carry",
                self.size, ONBOARD_MAX_READ_LEN
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!(
            "On-board payload read: buffer offset {} size {}",
            self.buffer_offset, self.size
        )
    }
}

/// Serialize a command into `buffer` for plan reporting. A command that does not validate serializes
/// to zeros: plan builders validate the read configuration first, so the zeros are a report artifact,
/// never bytes that reach the keyboard.
fn serialize_stage<C: HidCommand>(command: C, buffer: &mut [u8]) {
    let size = command.serialize(buffer, HidDeviceType::Keyboard);
    debug_assert!(size.is_ok(), "{} plan frame is not serializable", command.description());
}

/// Reject a buffer that cannot hold a report of `report_size` bytes.
fn check_report_buffer(buffer: &[u8], report_size: usize) -> Result<()> {
    if buffer.len() < report_size {
        return Err(Error::DeviceCommunication(format!(
            "Buffer too small: {} bytes (expected {})",
            buffer.len(),
            report_size
        )));
    }
    Ok(())
}

/// Apply/Save command to commit changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyCommand;

impl HidCommand for ApplyCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::Apply
    }

    fn serialize(&self, buffer: &mut [u8], device_type: HidDeviceType) -> Result<usize> {
        let report_size = device_type.report_size();
        if buffer.len() < report_size {
            return Err(Error::DeviceCommunication(format!(
                "Buffer too small: {} bytes (expected {})",
                buffer.len(),
                report_size
            )));
        }

        buffer[..report_size].fill(0);
        let mut offset = 0;

        // Add report ID for keyboards
        if device_type.includes_report_id() {
            buffer[0] = 0x00; // Report ID
            offset = 1;
        }

        // Command code only
        buffer[offset] = self.command_code() as u8;

        Ok(report_size)
    }

    fn validate(&self) -> Result<()> {
        Ok(()) // Always valid
    }

    fn description(&self) -> String {
        "Apply/save current settings".to_string()
    }
}

/// Actuation point control command for adjustable actuation keyboards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActuationCommand {
    /// Actuation point in 0.1mm increments (e.g. 4 = 0.4mm, 36 = 3.6mm)
    pub actuation_point: u8,
}

impl ActuationCommand {
    /// Create a new actuation command with the specified actuation point.
    /// Value is in 0.1mm increments (e.g. 4 = 0.4mm, 36 = 3.6mm).
    pub fn new(actuation_point: u8) -> Self {
        Self { actuation_point }
    }

    /// Create a new actuation command from millimeters.
    /// Precision is limited to 0.1mm increments.
    pub fn from_mm(mm: f32) -> Self {
        let tenths_of_mm = (mm * 10.0).round() as u8;
        Self {
            actuation_point: tenths_of_mm.min(40), // Clamp to max 4.0mm
        }
    }

    /// Convert to millimeters
    pub fn to_mm(&self) -> f32 {
        self.actuation_point as f32 / 10.0
    }
}

impl HidCommand for ActuationCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::ActuationControl
    }

    fn serialize(&self, buffer: &mut [u8], device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;

        let report_size = device_type.report_size();
        if buffer.len() < report_size {
            return Err(Error::DeviceCommunication(format!(
                "Buffer too small: {} bytes (expected {})",
                buffer.len(),
                report_size
            )));
        }

        buffer[..report_size].fill(0);
        let mut offset = 0;

        // Add report ID for keyboards
        if device_type.includes_report_id() {
            buffer[0] = 0x00; // Report ID
            offset = 1;
        }

        // Command code
        buffer[offset] = self.command_code() as u8;
        offset += 1;

        // Actuation point value
        buffer[offset] = self.actuation_point;

        Ok(report_size)
    }

    fn validate(&self) -> Result<()> {
        // Validate range (0.1mm - 4.0mm in 0.1mm increments)
        if self.actuation_point == 0 || self.actuation_point > 40 {
            return Err(Error::DeviceCommunication(format!(
                "Actuation point must be between 1 (0.1mm) and 40 (4.0mm), got {}",
                self.actuation_point
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set actuation point to {:.1}mm", self.to_mm())
    }
}

/// Per-key RGB command for individual key targeting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerKeyRgbCommand {
    /// Key-to-color mappings
    pub key_colors: HashMap<KeyAddress, Color>,
    /// Addressing mode
    pub addressing_mode: PerKeyAddressingMode,
}

/// Addressing mode for per-key RGB commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PerKeyAddressingMode {
    /// HID code addressing: [hid_code] [R] [G] [B] (4 bytes per key)
    /// Uses standard USB HID Usage IDs for key identification.
    /// This is the primary addressing mode for SteelSeries keyboards.
    HidCode,
    /// Legacy matrix row/column addressing (deprecated).
    /// Use HidCode instead - SteelSeries uses HID codes, not matrix coordinates.
    #[deprecated(note = "Use PerKeyAddressingMode::HidCode instead")]
    Matrix,
    /// Legacy logical key ID addressing (deprecated).
    /// Use HidCode instead - SteelSeries uses HID codes.
    #[deprecated(note = "Use PerKeyAddressingMode::HidCode instead")]
    Logical,
}

impl PerKeyRgbCommand {
    /// Maximum keys per report to fit within 64-byte payload limit
    /// Header: Report ID (1) + Cmd (1) + Count (1) = 3 bytes
    /// Key Data: HID code (1) + R (1) + G (1) + B (1) + zone (1) = 5 bytes
    /// Max Keys: (65 - 3) / 5 = 12
    pub const MAX_KEYS_PER_REPORT: usize = 12;

    /// Maximum matrix dimensions
    pub const MAX_ROWS: u8 = 32;
    pub const MAX_COLS: u8 = 32;

    /// Create a new per-key RGB command with HID code addressing.
    pub fn new(_addressing_mode: PerKeyAddressingMode) -> Self {
        Self {
            key_colors: HashMap::new(),
            addressing_mode: PerKeyAddressingMode::HidCode,
        }
    }

    /// Create a new per-key RGB command with batch ID for complex operations.
    /// Note: Batch ID is no longer used in serialization but kept for API compatibility.
    pub fn new_with_batch(_addressing_mode: PerKeyAddressingMode, _batch_id: u32) -> Self {
        Self::new(PerKeyAddressingMode::HidCode)
    }

    /// Check if this command needs fragmentation due to size limits.
    pub fn needs_fragmentation(&self) -> bool {
        self.key_colors.len() > Self::MAX_KEYS_PER_REPORT
    }

    /// Split this command into fragments that fit within report size limits.
    pub fn fragment_into_reports(&self) -> Vec<PerKeyRgbCommand> {
        let mut fragments = Vec::new();

        // Handle empty case
        if self.key_colors.is_empty() {
            return Vec::new();
        }

        let mut current_fragment = PerKeyRgbCommand {
            key_colors: HashMap::with_capacity(Self::MAX_KEYS_PER_REPORT),
            addressing_mode: self.addressing_mode,
        };

        for (address, color) in &self.key_colors {
            current_fragment.key_colors.insert(*address, *color);

            if current_fragment.key_colors.len() == Self::MAX_KEYS_PER_REPORT {
                fragments.push(current_fragment);
                current_fragment = PerKeyRgbCommand {
                    key_colors: HashMap::with_capacity(Self::MAX_KEYS_PER_REPORT),
                    addressing_mode: self.addressing_mode,
                };
            }
        }

        if !current_fragment.key_colors.is_empty() {
            fragments.push(current_fragment);
        }

        fragments
    }

    /// Create a command for a single key.
    pub fn single_key(address: KeyAddress, color: Color) -> Self {
        let mut command = Self::new(PerKeyAddressingMode::HidCode);
        command.set_key_color(address, color);
        command
    }

    /// Create a command from logical key IDs using a key mapping.
    pub fn from_logical_keys(key_colors: HashMap<KeyId, Color>, key_mapping: &KeyMapping) -> Result<Self> {
        let mut command = Self::new(PerKeyAddressingMode::HidCode);

        for (key_id, color) in key_colors {
            if let Some(address) = key_mapping.get_key_address(key_id) {
                command.set_key_color(address, color);
            } else {
                return Err(Error::DeviceCommunication(format!(
                    "Key {:?} not found in mapping",
                    key_id
                )));
            }
        }

        Ok(command)
    }

    /// Set color for a specific key address.
    pub fn set_key_color(&mut self, address: KeyAddress, color: Color) {
        self.key_colors.insert(address, color);
    }

    /// Remove a key from the command.
    pub fn remove_key(&mut self, address: KeyAddress) {
        self.key_colors.remove(&address);
    }

    /// Get all key addresses in this command.
    pub fn get_addresses(&self) -> Vec<KeyAddress> {
        self.key_colors.keys().copied().collect()
    }

    /// Get color for a specific address.
    pub fn get_key_color(&self, address: KeyAddress) -> Option<Color> {
        self.key_colors.get(&address).copied()
    }

    /// Check if this command is empty.
    pub fn is_empty(&self) -> bool {
        self.key_colors.is_empty()
    }

    /// Get the number of keys in this command.
    pub fn key_count(&self) -> usize {
        self.key_colors.len()
    }
}

impl HidCommand for PerKeyRgbCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::PerKeyRgb
    }

    fn serialize(&self, buffer: &mut [u8], device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;

        let report_size = device_type.report_size();
        if buffer.len() < report_size {
            return Err(Error::DeviceCommunication(format!(
                "Buffer too small: {} bytes (expected {})",
                buffer.len(),
                report_size
            )));
        }

        buffer[..report_size].fill(0);
        let mut offset = 0;

        // Add report ID for keyboards
        if device_type.includes_report_id() {
            buffer[0] = 0x00; // Report ID
            offset = 1;
        }

        // Command code
        buffer[offset] = self.command_code() as u8;
        offset += 1;

        // Key count
        buffer[offset] = self.key_colors.len().min(255) as u8;
        offset += 1;

        match self.addressing_mode {
            PerKeyAddressingMode::HidCode => {
                // HID code addressing: [hid_code] [R] [G] [B]
                for (address, color) in &self.key_colors {
                    if offset + 4 > report_size {
                        break;
                    }

                    buffer[offset] = address.hid_code;
                    buffer[offset + 1] = color.r;
                    buffer[offset + 2] = color.g;
                    buffer[offset + 3] = color.b;
                    offset += 4;
                }
            }
            #[allow(deprecated)]
            PerKeyAddressingMode::Matrix | PerKeyAddressingMode::Logical => {
                // Deprecated modes - treat as HidCode
                for (address, color) in &self.key_colors {
                    if offset + 4 > report_size {
                        break;
                    }

                    buffer[offset] = address.hid_code;
                    buffer[offset + 1] = color.r;
                    buffer[offset + 2] = color.g;
                    buffer[offset + 3] = color.b;
                    offset += 4;
                }
            }
        }

        Ok(report_size)
    }

    fn validate(&self) -> Result<()> {
        if self.key_colors.is_empty() && !self.needs_fragmentation() {
            return Err(Error::DeviceCommunication(
                "Per-key RGB command must have at least one key".to_string(),
            ));
        }

        // Validate HID code bounds (valid range: 0x04-0xF0)
        for address in self.key_colors.keys() {
            if address.hid_code < 0x04 || address.hid_code > 0xF0 {
                return Err(Error::DeviceCommunication(format!(
                    "HID code 0x{:02X} is outside valid range (0x04-0xF0)",
                    address.hid_code
                )));
            }
        }

        // 1 (ID) + 1 (Cmd) + 1 (Count) + N*5
        let required_bytes = 1 + 1 + 1 + (self.key_colors.len() * 5);

        if required_bytes > 65 {
            return Err(Error::DeviceCommunication(format!(
                "Too many keys in command: {} keys require {} bytes (max 65)",
                self.key_colors.len(),
                required_bytes
            )));
        }

        Ok(())
    }

    fn description(&self) -> String {
        format!("Set {} keys via HID code addressing", self.key_colors.len())
    }
}

/// Builder for creating batch per-key RGB operations.
#[derive(Debug)]
pub struct PerKeyRgbBuilder {
    addressing_mode: PerKeyAddressingMode,
    key_colors: HashMap<KeyAddress, Color>,
    key_mapping: Option<KeyMapping>,
}

impl PerKeyRgbBuilder {
    /// Create a new per-key RGB builder.
    pub fn new(addressing_mode: PerKeyAddressingMode) -> Self {
        Self {
            addressing_mode,
            key_colors: HashMap::new(),
            key_mapping: None,
        }
    }

    /// Create a new per-key RGB builder with batch ID.
    pub fn new_with_batch(addressing_mode: PerKeyAddressingMode, _batch_id: u32) -> Self {
        Self::new(addressing_mode)
    }

    /// Create a builder with logical addressing support.
    pub fn with_key_mapping(key_mapping: KeyMapping) -> Self {
        Self {
            addressing_mode: PerKeyAddressingMode::HidCode,
            key_colors: HashMap::new(),
            key_mapping: Some(key_mapping),
        }
    }

    /// Create a builder with logical addressing and batch ID.
    pub fn with_key_mapping_and_batch(key_mapping: KeyMapping, _batch_id: u32) -> Self {
        Self::with_key_mapping(key_mapping)
    }

    /// Add a key by matrix address.
    pub fn add_key_matrix(&mut self, address: KeyAddress, color: Color) -> &mut Self {
        self.key_colors.insert(address, color);
        self
    }

    /// Add a key by logical ID (requires key mapping).
    pub fn add_key_logical(&mut self, key_id: KeyId, color: Color) -> Result<&mut Self> {
        if let Some(ref mapping) = self.key_mapping {
            if let Some(address) = mapping.get_key_address(key_id) {
                self.key_colors.insert(address, color);
                Ok(self)
            } else {
                Err(Error::DeviceCommunication(format!(
                    "Key {:?} not found in mapping",
                    key_id
                )))
            }
        } else {
            Err(Error::DeviceCommunication(
                "No key mapping available for logical addressing".to_string(),
            ))
        }
    }

    /// Add multiple keys with the same color.
    pub fn add_keys_batch(&mut self, addresses: &[KeyAddress], color: Color) -> &mut Self {
        for &address in addresses {
            self.key_colors.insert(address, color);
        }
        self
    }

    /// Add keys from a logical key list (requires key mapping).
    pub fn add_logical_keys(&mut self, key_ids: &[KeyId], color: Color) -> Result<&mut Self> {
        if let Some(ref mapping) = self.key_mapping {
            for &key_id in key_ids {
                if let Some(address) = mapping.get_key_address(key_id) {
                    self.key_colors.insert(address, color);
                } else {
                    return Err(Error::DeviceCommunication(format!(
                        "Key {:?} not found in mapping",
                        key_id
                    )));
                }
            }
            Ok(self)
        } else {
            Err(Error::DeviceCommunication(
                "No key mapping available for logical addressing".to_string(),
            ))
        }
    }

    /// Clear all keys.
    pub fn clear(&mut self) -> &mut Self {
        self.key_colors.clear();
        self
    }

    /// Set a region of keys to the same color.
    pub fn set_region(&mut self, start_hid: u8, count: u8, color: Color) -> &mut Self {
        for hid_code in start_hid..(start_hid + count) {
            self.key_colors.insert(KeyAddress::new(hid_code), color);
        }
        self
    }

    /// Build the final per-key RGB command.
    pub fn build(self) -> PerKeyRgbCommand {
        let mut command = PerKeyRgbCommand::new(self.addressing_mode);
        command.key_colors = self.key_colors;
        command
    }

    /// Get the current number of keys in the builder.
    pub fn key_count(&self) -> usize {
        self.key_colors.len()
    }

    /// Check if the builder is empty.
    pub fn is_empty(&self) -> bool {
        self.key_colors.is_empty()
    }
}
/// Structured HID report builder and validator.
#[derive(Debug)]
pub struct HidReportBuilder {
    device_type: HidDeviceType,
}

impl HidReportBuilder {
    /// Create a new report builder for the specified device type.
    pub fn new(device_type: HidDeviceType) -> Self {
        Self { device_type }
    }

    /// Build a report from a command.
    pub fn build_report<T: HidCommand>(&self, command: T, buffer: &mut [u8]) -> Result<usize> {
        tracing::debug!(
            "Building HID report: {} ({})",
            command.description(),
            command.command_code()
        );

        let size = command.serialize(buffer, self.device_type)?;
        let data = &buffer[..size];
        self.validate_report_size(data, size)?;

        tracing::debug!(
            "Built {} byte HID report: {:02x?}",
            data.len(),
            &data[..data.len().min(32)]
        );
        Ok(size)
    }

    /// Validate a raw HID report against the device's standard report size.
    pub fn validate_report(&self, data: &[u8]) -> Result<()> {
        self.validate_report_size(data, self.device_type.report_size())
    }

    fn validate_report_size(&self, data: &[u8], expected_size: usize) -> Result<()> {
        if data.len() != expected_size {
            return Err(Error::DeviceCommunication(format!(
                "Invalid report size: {} bytes (expected {})",
                data.len(),
                expected_size
            )));
        }

        // Check report ID for keyboards
        if self.device_type.includes_report_id() && !data.is_empty() && data[0] != 0x00 {
            tracing::warn!("Unexpected report ID: 0x{:02x} (expected 0x00)", data[0]);
        }

        // Validate command code
        let cmd_offset = if self.device_type.includes_report_id() { 1 } else { 0 };
        if data.len() > cmd_offset {
            let cmd_byte = data[cmd_offset];
            if CommandCode::from_report_byte(cmd_byte).is_none() {
                tracing::warn!("Unknown command byte: 0x{:02x}", cmd_byte);
            }
        }

        Ok(())
    }

    /// Parse a raw report to extract the command code.
    pub fn parse_command_code(&self, data: &[u8]) -> Option<CommandCode> {
        let cmd_offset = if self.device_type.includes_report_id() { 1 } else { 0 };

        if data.len() <= cmd_offset {
            return None;
        }

        CommandCode::from_report_byte(data[cmd_offset])
    }
}

/// HID error recovery system for handling hot-plug scenarios.
pub mod recovery {
    use super::*;
    use hidapi::{HidDevice, HidError};
    use std::time::{Duration, Instant};
    use thiserror::Error;
    use tracing::{debug, error, warn};

    /// Maximum number of retry attempts before marking error as permanent
    const MAX_RETRY_ATTEMPTS: u8 = 5;

    /// Base delay for exponential backoff (milliseconds)
    const BASE_BACKOFF_MS: u64 = 50;

    /// Maximum backoff delay to prevent excessive waiting
    const MAX_BACKOFF_MS: u64 = 2000;

    /// Circuit breaker threshold - errors per minute to trigger breaker
    const CIRCUIT_BREAKER_THRESHOLD: u32 = 10;

    /// Time window for circuit breaker error counting
    const CIRCUIT_BREAKER_WINDOW: Duration = Duration::from_secs(60);

    /// HID report validation and error recovery errors
    #[derive(Error, Debug, Clone)]
    pub enum HidReportError {
        #[error("Device disconnected during operation")]
        DeviceDisconnected,

        #[error("Invalid report format: {0}")]
        InvalidReport(String),

        #[error("Communication timeout after {0}ms")]
        Timeout(u64),

        #[error("Recovery failed after {0} attempts")]
        RecoveryFailed(u8),

        #[error("Circuit breaker open - too many errors ({0}/min)")]
        CircuitBreakerOpen(u32),

        #[error("Transient error: {0}")]
        Transient(String),

        #[error("Permanent error: {0}")]
        Permanent(String),
    }

    /// Categories of HID errors for appropriate recovery strategies
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum ErrorCategory {
        /// Temporary errors that may resolve with retry
        Transient,
        /// Permanent errors requiring device reset or reconnection
        Permanent,
        /// Device physically disconnected
        Disconnected,
    }

    /// Connection health status for proactive failure detection
    #[derive(Debug, Clone)]
    pub struct ConnectionHealth {
        /// Success rate over recent operations (0.0 - 1.0)
        pub success_rate: f64,
        /// Average response time for recent operations
        pub avg_response_time_ms: f64,
        /// Number of consecutive failures
        pub consecutive_failures: u32,
        /// Last successful operation timestamp
        pub last_success: Option<Instant>,
        /// Total operations attempted
        pub total_operations: u64,
        /// Total successful operations
        pub successful_operations: u64,
    }

    impl Default for ConnectionHealth {
        fn default() -> Self {
            Self {
                success_rate: 1.0,
                avg_response_time_ms: 0.0,
                consecutive_failures: 0,
                last_success: None,
                total_operations: 0,
                successful_operations: 0,
            }
        }
    }

    impl ConnectionHealth {
        /// Check if connection is healthy enough for operations
        pub fn is_healthy(&self) -> bool {
            self.success_rate >= 0.8 && self.consecutive_failures < 3
        }

        /// Check if connection requires immediate attention
        pub fn needs_recovery(&self) -> bool {
            self.consecutive_failures >= 5 || self.success_rate < 0.5
        }

        /// Update health metrics after an operation
        pub fn record_operation(&mut self, success: bool, response_time: Duration) {
            self.total_operations += 1;

            if success {
                self.successful_operations += 1;
                self.consecutive_failures = 0;
                self.last_success = Some(Instant::now());
            } else {
                self.consecutive_failures += 1;
            }

            // Update success rate (sliding window approach)
            self.success_rate = self.successful_operations as f64 / self.total_operations as f64;

            // Update average response time (exponential moving average)
            let response_ms = response_time.as_millis() as f64;
            if self.avg_response_time_ms == 0.0 {
                self.avg_response_time_ms = response_ms;
            } else {
                // Use alpha = 0.1 for smoother averaging
                self.avg_response_time_ms = 0.9 * self.avg_response_time_ms + 0.1 * response_ms;
            }
        }
    }

    /// Circuit breaker state for preventing cascade failures
    #[derive(Debug)]
    struct CircuitBreaker {
        error_count: u32,
        window_start: Instant,
        is_open: bool,
    }

    impl Default for CircuitBreaker {
        fn default() -> Self {
            Self {
                error_count: 0,
                window_start: Instant::now(),
                is_open: false,
            }
        }
    }

    impl CircuitBreaker {
        /// Record an error and check if circuit should open
        fn record_error(&mut self) -> bool {
            let now = Instant::now();

            // Reset window if expired
            if now.duration_since(self.window_start) > CIRCUIT_BREAKER_WINDOW {
                self.error_count = 0;
                self.window_start = now;
                self.is_open = false;
            }

            self.error_count += 1;

            if self.error_count >= CIRCUIT_BREAKER_THRESHOLD {
                self.is_open = true;
                warn!(
                    "Circuit breaker opened: {} errors in {}s window",
                    self.error_count,
                    CIRCUIT_BREAKER_WINDOW.as_secs()
                );
            }

            self.is_open
        }

        /// Check if circuit breaker allows operations
        fn is_open(&self) -> bool {
            // Auto-reset after window expires
            if Instant::now().duration_since(self.window_start) > CIRCUIT_BREAKER_WINDOW {
                return false;
            }
            self.is_open
        }
    }

    /// HID error recovery system with exponential backoff and circuit breaker
    pub struct HidErrorRecovery {
        /// Connection health tracking
        health: ConnectionHealth,
        /// Circuit breaker for cascade failure prevention
        circuit_breaker: CircuitBreaker,
        /// Last recovery attempt timestamp
        last_recovery: Option<Instant>,
        /// Number of recovery attempts for current issue
        recovery_attempts: u8,
    }

    impl Default for HidErrorRecovery {
        fn default() -> Self {
            Self::new()
        }
    }

    impl HidErrorRecovery {
        /// Create a new error recovery system
        pub fn new() -> Self {
            Self {
                health: ConnectionHealth::default(),
                circuit_breaker: CircuitBreaker::default(),
                last_recovery: None,
                recovery_attempts: 0,
            }
        }

        /// Get current connection health status
        pub fn health(&self) -> &ConnectionHealth {
            &self.health
        }

        /// Record a successful operation
        pub fn record_success(&mut self, response_time: Duration) {
            self.health.record_operation(true, response_time);
            self.recovery_attempts = 0; // Reset on success
        }

        /// Record a failed operation and determine error category
        pub fn record_failure(&mut self, error: &HidError, response_time: Duration) -> ErrorCategory {
            self.health.record_operation(false, response_time);

            let category = self.categorize_error(error);

            // Record in circuit breaker only for transient errors
            // (permanent errors and disconnections are handled differently)
            if category == ErrorCategory::Transient {
                self.circuit_breaker.record_error();
            }

            category
        }

        /// Attempt to recover from an error with exponential backoff
        pub async fn recover_from_error(&mut self, device: &HidDevice, error: &HidError) -> Result<()> {
            if self.circuit_breaker.is_open() {
                return Err(Error::DeviceCommunication(
                    HidReportError::CircuitBreakerOpen(CIRCUIT_BREAKER_THRESHOLD).to_string(),
                ));
            }

            if self.recovery_attempts >= MAX_RETRY_ATTEMPTS {
                return Err(Error::DeviceCommunication(
                    HidReportError::RecoveryFailed(self.recovery_attempts).to_string(),
                ));
            }

            let error_category = self.categorize_error(error);

            match error_category {
                ErrorCategory::Disconnected => {
                    debug!("Device disconnected, recovery not possible");
                    return Err(Error::DeviceCommunication(
                        HidReportError::DeviceDisconnected.to_string(),
                    ));
                }

                ErrorCategory::Permanent => {
                    debug!("Permanent error detected, attempting device reset");
                    self.attempt_device_reset(device).await?;
                }

                ErrorCategory::Transient => {
                    debug!("Transient error, using exponential backoff");
                    self.exponential_backoff().await;
                }
            }

            self.recovery_attempts += 1;
            self.last_recovery = Some(Instant::now());

            Ok(())
        }

        /// Categorize HID error for appropriate recovery strategy
        fn categorize_error(&self, error: &HidError) -> ErrorCategory {
            match error {
                HidError::HidApiError { message } => {
                    let msg = message.to_lowercase();
                    if msg.contains("no device") || msg.contains("disconnected") || msg.contains("not found") {
                        ErrorCategory::Disconnected
                    } else if msg.contains("timeout") || msg.contains("busy") || msg.contains("again") {
                        ErrorCategory::Transient
                    } else {
                        ErrorCategory::Permanent
                    }
                }
                HidError::HidApiErrorEmpty => ErrorCategory::Transient,
                HidError::InitializationError => ErrorCategory::Permanent,
                HidError::InvalidZeroSizeData => ErrorCategory::Permanent,
                HidError::IncompleteSendError { .. } => ErrorCategory::Transient,
                HidError::SetBlockingModeError { .. } => ErrorCategory::Permanent,
                HidError::FromWideCharError { .. } => ErrorCategory::Permanent,
                HidError::OpenHidDeviceWithDeviceInfoError { .. } => ErrorCategory::Disconnected,
                HidError::IoError { .. } => ErrorCategory::Transient,
            }
        }

        /// Apply exponential backoff delay
        async fn exponential_backoff(&self) {
            let delay_ms = std::cmp::min(
                BASE_BACKOFF_MS * 2_u64.pow(self.recovery_attempts as u32),
                MAX_BACKOFF_MS,
            );

            debug!(
                "Applying exponential backoff: {}ms (attempt {})",
                delay_ms,
                self.recovery_attempts + 1
            );
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }

        /// Attempt device reset by sending a harmless command
        async fn attempt_device_reset(&self, device: &HidDevice) -> Result<()> {
            debug!("Attempting device reset");

            // Send a minimal harmless report to test device responsiveness
            let reset_report = vec![0u8; 65]; // Empty report should be safe

            match device.write(&reset_report) {
                Ok(_) => {
                    debug!("Device reset successful");
                    Ok(())
                }
                Err(e) => {
                    warn!("Device reset failed: {}", e);
                    Err(Error::DeviceCommunication(format!("Reset failed: {}", e)))
                }
            }
        }

        /// Check if recovery should be attempted based on current state
        pub fn should_attempt_recovery(&self) -> bool {
            if self.circuit_breaker.is_open() {
                return false;
            }

            if self.recovery_attempts >= MAX_RETRY_ATTEMPTS {
                return false;
            }

            // Rate limit recovery attempts
            if let Some(last) = self.last_recovery
                && last.elapsed() < Duration::from_millis(BASE_BACKOFF_MS)
            {
                return false;
            }

            true
        }
    }

    /// HID report validator for ensuring report integrity
    pub struct ReportValidator;

    impl ReportValidator {
        /// Validate HID report before transmission
        pub fn validate_report(report: &[u8]) -> Result<()> {
            // Check minimum size (should be 65 bytes for SteelSeries devices)
            if report.is_empty() {
                return Err(Error::DeviceCommunication(
                    HidReportError::InvalidReport("Report cannot be empty".to_string()).to_string(),
                ));
            }

            // Check maximum size (prevent buffer overflows)
            if report.len() > 65 {
                return Err(Error::DeviceCommunication(
                    HidReportError::InvalidReport(format!("Report too large: {} bytes (max 65)", report.len()))
                        .to_string(),
                ));
            }

            // For SteelSeries devices, we expect exactly 65 bytes
            if report.len() != 65 {
                warn!("Unexpected report size: {} bytes (expected 65)", report.len());
            }

            // Basic validation for RGB commands (if applicable)
            if report.len() >= 2 {
                let command = report[1];
                match command {
                    0x21 => {
                        // RGB color command
                        if report.len() < 29 {
                            return Err(Error::DeviceCommunication(
                                HidReportError::InvalidReport("RGB command too short".to_string()).to_string(),
                            ));
                        }
                    }
                    _ => {
                        // Other commands are generally acceptable
                        debug!("Validating command: 0x{:02x}", command);
                    }
                }
            }

            Ok(())
        }

        /// Validate and potentially correct report format
        pub fn validate_and_correct(report: &mut Vec<u8>) -> Result<()> {
            // Ensure report is exactly 65 bytes for SteelSeries devices
            match report.len() {
                len if len < 65 => {
                    // Pad with zeros
                    report.resize(65, 0);
                    debug!("Padded report from {} to 65 bytes", len);
                }
                len if len > 65 => {
                    // Truncate to 65 bytes
                    report.truncate(65);
                    warn!("Truncated report from {} to 65 bytes", len);
                }
                65 => {
                    // Perfect size, no correction needed
                }
                _ => unreachable!(),
            }

            // Validate the corrected report
            Self::validate_report(report)
        }
    }

    /// High-level function to write HID report with automatic error recovery
    pub async fn write_report_with_recovery(
        device: &HidDevice,
        report: &[u8],
        recovery: &mut HidErrorRecovery,
    ) -> Result<()> {
        // Validate report first
        ReportValidator::validate_report(report)?;

        let start_time = Instant::now();

        loop {
            match device.write(report) {
                Ok(bytes_written) => {
                    let response_time = start_time.elapsed();
                    recovery.record_success(response_time);

                    debug!("HID write successful: {} bytes in {:?}", bytes_written, response_time);
                    return Ok(());
                }

                Err(hid_error) => {
                    let response_time = start_time.elapsed();
                    let error_category = recovery.record_failure(&hid_error, response_time);

                    warn!("HID write failed: {} (category: {:?})", hid_error, error_category);

                    if !recovery.should_attempt_recovery() {
                        error!("Recovery not possible, giving up");
                        return Err(Error::DeviceCommunication(format!("HID write failed: {}", hid_error)));
                    }

                    // Attempt recovery
                    match recovery.recover_from_error(device, &hid_error).await {
                        Ok(()) => {
                            debug!("Recovery successful, retrying write");
                            continue; // Retry the write
                        }
                        Err(recovery_error) => {
                            error!("Recovery failed: {}", recovery_error);
                            return Err(recovery_error);
                        }
                    }
                }
            }
        }
    }
}

// Re-export recovery types for convenience
pub use recovery::{
    ConnectionHealth, ErrorCategory, HidErrorRecovery, HidReportError, ReportValidator, write_report_with_recovery,
};

#[cfg(test)]
mod tests {
    use super::*;
    use recovery::*;

    #[test]
    fn opening_chain_frames_match_the_capture() {
        let expected = [
            [0x90, 0x00, 0x00],
            [0x90, 0x01, 0x00],
            [0xF4, 0x00, 0x00],
            [0x6C, 0x00, 0x01],
            [0x90, 0x00, 0x00],
            [0x90, 0x01, 0x00],
        ];
        for (frame, expected) in ONBOARD_INIT_CHAIN.iter().zip(expected) {
            assert_eq!(frame.data_stage(), expected, "frame bytes of {}", frame.command);
            let mut buffer = [0u8; ONBOARD_OUTPUT_REPORT_SIZE];
            let size = frame.serialize(&mut buffer, HidDeviceType::Keyboard).unwrap();
            assert_eq!(size, ONBOARD_OUTPUT_REPORT_SIZE);
            assert_eq!(buffer[0], 0x00, "report number 0, as the capture's wValue 0x0200");
            assert_eq!(&buffer[1..4], &expected);
            assert!(
                buffer[4..].iter().all(|b| *b == 0),
                "the 64-byte data stage is zero padded like the capture's frames"
            );
        }
        assert!(!ONBOARD_INIT_CHAIN[3].expects_reply, "no answer to 6c was captured");
        assert!(
            matches!(ONBOARD_INIT_CHAIN[0].command, CommandCode::FirmwareVersion),
            "the first frame is the firmware poll"
        );
        assert!(
            OnboardInitCommand {
                command: CommandCode::RgbControl,
                args: [0, 0],
                expects_reply: false,
            }
            .validate()
            .is_err(),
            "only the captured chain codes are accepted"
        );
    }

    #[test]
    fn pull_frame_matches_the_capture_layout() {
        let cmd = OnboardPullCommand {
            fs_id: 0x0201,
            file_offset: 4096,
            size: 1024,
        };
        assert_eq!(
            cmd.data_stage(),
            [0x85, 0x00, 0x01, 0x02, 0x00, 0x10, 0x00, 0x00, 0x00, 0x04]
        );
        let mut buffer = [0u8; ONBOARD_OUTPUT_REPORT_SIZE];
        assert_eq!(
            cmd.serialize(&mut buffer, HidDeviceType::Keyboard).unwrap(),
            ONBOARD_OUTPUT_REPORT_SIZE
        );
        assert_eq!(buffer[0], 0x00);
        assert!(buffer[11..].iter().all(|b| *b == 0));
        assert!(
            OnboardPullCommand {
                fs_id: 0x0201,
                file_offset: 0,
                size: 2048,
            }
            .validate()
            .is_err(),
            "a pull cannot exceed the resource buffer"
        );
    }

    #[test]
    fn onboard_reports_build_through_the_builder() {
        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);
        let mut chain = [0u8; ONBOARD_OUTPUT_REPORT_SIZE];
        assert_eq!(
            builder.build_report(ONBOARD_INIT_CHAIN[0], &mut chain).unwrap(),
            ONBOARD_OUTPUT_REPORT_SIZE
        );
        let mut pull = [0u8; ONBOARD_OUTPUT_REPORT_SIZE];
        assert_eq!(
            builder
                .build_report(
                    OnboardPullCommand {
                        fs_id: 0x0201,
                        file_offset: 0,
                        size: 1024,
                    },
                    &mut pull
                )
                .unwrap(),
            ONBOARD_OUTPUT_REPORT_SIZE
        );
        let mut read = [0u8; ONBOARD_FEATURE_REPORT_SIZE];
        assert_eq!(
            builder
                .build_report(
                    OnboardReadCommand {
                        buffer_offset: 512,
                        size: 512,
                    },
                    &mut read
                )
                .unwrap(),
            ONBOARD_FEATURE_REPORT_SIZE
        );
        for buffer in [&chain[..], &pull[..], &read[..]] {
            assert_eq!(buffer[0], 0x00, "report number 0 is byte 0 of every buffer");
        }
        assert!(
            builder
                .build_report(
                    OnboardPullCommand {
                        fs_id: 0x0201,
                        file_offset: 0,
                        size: 4096,
                    },
                    &mut pull
                )
                .is_err(),
            "the builder rejects an invalid pull before anything is sent"
        );
    }

    #[test]
    fn read_frame_matches_the_capture_layout() {
        let cmd = OnboardReadCommand {
            buffer_offset: 512,
            size: 512,
        };
        assert_eq!(cmd.data_stage(), [0x83, 0x00, 0x00, 0x02, 0x00, 0x02]);
        assert_eq!(
            OnboardReadCommand {
                buffer_offset: 0,
                size: 2,
            }
            .data_stage(),
            [0x83, 0x00, 0x00, 0x00, 0x02, 0x00]
        );
        let mut buffer = [0u8; ONBOARD_FEATURE_REPORT_SIZE];
        assert_eq!(
            cmd.serialize(&mut buffer, HidDeviceType::Keyboard).unwrap(),
            ONBOARD_FEATURE_REPORT_SIZE
        );
        assert_eq!(buffer[0], 0x00, "report number 0, as the capture's wValue 0x0300");
        assert!(buffer[7..].iter().all(|b| *b == 0));
        assert!(
            OnboardReadCommand {
                buffer_offset: 0,
                size: 1024,
            }
            .validate()
            .is_err(),
            "a read cannot ask for more than one reply report carries"
        );
        let mut small = [0u8; 64];
        assert!(
            cmd.serialize(&mut small, HidDeviceType::Keyboard).is_err(),
            "the feature report needs its full buffer"
        );
    }

    #[test]
    fn test_rgb_zone_command_validation() {
        // Valid command
        let cmd = RgbZoneCommand::new_single_color(Color::RED, 5);
        assert!(cmd.validate().is_ok());

        // Empty colors should fail
        let cmd = RgbZoneCommand {
            colors: vec![].into(),
            zone_selector: 0xFF,
        };
        assert!(cmd.validate().is_err());

        // Too many colors should fail
        let cmd = RgbZoneCommand {
            colors: vec![Color::RED; MAX_RGB_ZONES + 1].into(),
            zone_selector: 0xFF,
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_brightness_command_validation() {
        // Valid brightness
        let cmd = BrightnessCommand::new(50);
        assert_eq!(cmd.brightness, 50);
        assert!(cmd.validate().is_ok());

        // Auto-clamping
        let cmd = BrightnessCommand::new(150);
        assert_eq!(cmd.brightness, 100);

        // Manual validation should fail for over 100
        let cmd = BrightnessCommand { brightness: 150 };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_keyboard_report_serialization() {
        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);
        let mut buffer = [0u8; KEYBOARD_REPORT_SIZE];

        // RGB command
        let rgb_cmd = RgbZoneCommand::new_single_color(Color::RED, 1);
        let size = builder.build_report(rgb_cmd, &mut buffer).unwrap();
        assert_eq!(size, KEYBOARD_REPORT_SIZE);
        assert_eq!(buffer[0], 0x00); // Report ID
        assert_eq!(buffer[1], 0x21); // RGB command
        assert_eq!(buffer[2], 0xFF); // Zone selector
        assert_eq!(buffer[3], 255); // Red
        assert_eq!(buffer[4], 0); // Green
        assert_eq!(buffer[5], 0); // Blue

        // Brightness command
        let brightness_cmd = BrightnessCommand::new(75);
        let size = builder.build_report(brightness_cmd, &mut buffer).unwrap();
        assert_eq!(size, KEYBOARD_REPORT_SIZE);
        assert_eq!(buffer[0], 0x00); // Report ID
        assert_eq!(buffer[1], 0x22); // Brightness command
        assert_eq!(buffer[2], 75); // Brightness value

        // Apply command
        let apply_cmd = ApplyCommand;
        let size = builder.build_report(apply_cmd, &mut buffer).unwrap();
        assert_eq!(size, KEYBOARD_REPORT_SIZE);
        assert_eq!(buffer[0], 0x00); // Report ID
        assert_eq!(buffer[1], 0x09); // Apply command
    }

    #[test]
    fn test_headset_report_serialization() {
        let builder = HidReportBuilder::new(HidDeviceType::Headset);
        let mut buffer = [0u8; HEADSET_REPORT_SIZE];

        // RGB command
        let rgb_cmd = RgbZoneCommand::new_single_color(Color::BLUE, 1);
        let size = builder.build_report(rgb_cmd, &mut buffer).unwrap();
        assert_eq!(size, HEADSET_REPORT_SIZE);
        assert_eq!(buffer[0], 0x21); // RGB command (no report ID)
        assert_eq!(buffer[1], 0xFF); // Zone selector
        assert_eq!(buffer[2], 0); // Red
        assert_eq!(buffer[3], 0); // Green
        assert_eq!(buffer[4], 255); // Blue
    }

    #[test]
    fn test_command_code_parsing() {
        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);

        let rgb_data = vec![0x00, 0x21, 0xFF, 255, 0, 0]; // RGB command
        assert_eq!(builder.parse_command_code(&rgb_data), Some(CommandCode::RgbControl));

        let brightness_data = vec![0x00, 0x22, 50]; // Brightness command
        assert_eq!(
            builder.parse_command_code(&brightness_data),
            Some(CommandCode::Brightness)
        );

        let apply_data = vec![0x00, 0x09]; // Apply command
        assert_eq!(builder.parse_command_code(&apply_data), Some(CommandCode::Apply));

        let unknown_data = vec![0x00, 0xFF]; // Unknown command
        assert_eq!(builder.parse_command_code(&unknown_data), None);

        let perkey_data = vec![0x00, 0x2A, 0x00, 0x01]; // Per-key command
        assert_eq!(builder.parse_command_code(&perkey_data), Some(CommandCode::PerKeyRgb));
    }

    #[test]
    fn test_report_validation() {
        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);

        // Valid report
        let valid_data = vec![0u8; KEYBOARD_REPORT_SIZE];
        assert!(builder.validate_report(&valid_data).is_ok());

        // Invalid size
        let invalid_data = vec![0u8; 32];
        assert!(builder.validate_report(&invalid_data).is_err());
    }

    #[test]
    fn test_per_key_rgb_command() {
        // Test single key command
        let addr = KeyAddress::new(0x04); // HID code for 'a' key
        let color = Color::RED;
        let cmd = PerKeyRgbCommand::single_key(addr, color);

        assert_eq!(cmd.key_count(), 1);
        assert_eq!(cmd.get_key_color(addr), Some(color));
        assert!(!cmd.is_empty());
        assert_eq!(cmd.addressing_mode, PerKeyAddressingMode::HidCode);

        // Test validation
        assert!(cmd.validate().is_ok());

        // Test serialization
        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);
        let mut buffer = [0u8; KEYBOARD_REPORT_SIZE];
        let size = builder.build_report(cmd, &mut buffer).unwrap();
        assert_eq!(size, KEYBOARD_REPORT_SIZE);
        assert_eq!(buffer[0], 0x00); // Report ID
        assert_eq!(buffer[1], 0x23); // Per-key RGB command
        assert_eq!(buffer[2], 0x01); // Key count
        assert_eq!(buffer[3], 0x04); // HID code
        assert_eq!(buffer[4], 255); // Red
        assert_eq!(buffer[5], 0); // Green
        assert_eq!(buffer[6], 0); // Blue
    }

    #[test]
    fn test_per_key_rgb_builder() {
        let mut builder = PerKeyRgbBuilder::new(PerKeyAddressingMode::HidCode);

        // Initially empty
        assert!(builder.is_empty());
        assert_eq!(builder.key_count(), 0);

        // Add keys
        builder.add_key_matrix(KeyAddress::new(0x04), Color::RED);
        builder.add_key_matrix(KeyAddress::new(0x05), Color::GREEN);
        builder.add_key_matrix(KeyAddress::new(0x06), Color::BLUE);

        assert_eq!(builder.key_count(), 3);
        assert!(!builder.is_empty());

        // Add batch
        let addresses = vec![KeyAddress::new(0x07), KeyAddress::new(0x08), KeyAddress::new(0x09)];
        builder.add_keys_batch(&addresses, Color::WHITE);
        assert_eq!(builder.key_count(), 6);

        // Set region (start_hid, count, color)
        builder.set_region(0x10, 4, Color::CYAN);
        assert!(builder.key_count() >= 6); // At least original + region keys

        // Build final command
        let cmd = builder.build();
        assert!(cmd.key_count() >= 6);
        assert_eq!(cmd.addressing_mode, PerKeyAddressingMode::HidCode);
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_apex_8zone_brightness_report() {
        assert_eq!(Apex8ZoneBrightnessCommand::from_percent(0).level, 0x00);
        assert_eq!(Apex8ZoneBrightnessCommand::from_percent(50).level, 0x08);
        assert_eq!(Apex8ZoneBrightnessCommand::from_percent(100).level, 0x10);
        assert_eq!(Apex8ZoneBrightnessCommand::from_percent(255).level, 0x10);

        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);
        let mut buffer = [0xAAu8; KEYBOARD_REPORT_SIZE];
        let size = builder
            .build_report(Apex8ZoneBrightnessCommand::from_percent(50), &mut buffer)
            .unwrap();
        assert_eq!(size, KEYBOARD_REPORT_SIZE);
        assert_eq!(&buffer[..3], &[0x00, 0x23, 0x08]);
        assert!(buffer[3..].iter().all(|&b| b == 0));

        assert!(Apex8ZoneBrightnessCommand { level: 0x11 }.validate().is_err());
    }

    #[test]
    fn test_per_key_rgb_validation() {
        // Empty command should fail
        let empty_cmd = PerKeyRgbCommand::new(PerKeyAddressingMode::HidCode);
        assert!(empty_cmd.validate().is_err());

        // Valid command should pass (HID code 0x04 = 'a' key)
        let mut valid_cmd = PerKeyRgbCommand::new(PerKeyAddressingMode::HidCode);
        valid_cmd.set_key_color(KeyAddress::new(0x04), Color::RED);
        assert!(valid_cmd.validate().is_ok());

        // Too many keys should fail
        let mut oversize_cmd = PerKeyRgbCommand::new(PerKeyAddressingMode::HidCode);
        for i in 0..20 {
            // 20 keys * 5 bytes = 100 bytes > 64 byte limit
            oversize_cmd.set_key_color(KeyAddress::new(i + 0x04), Color::RED);
        }
        assert!(oversize_cmd.validate().is_err());
    }

    #[test]
    #[allow(deprecated)]
    fn test_per_key_addressing_modes() {
        // Matrix and Logical are deprecated variants that still exist
        // but are separate from HidCode
        let matrix_mode = PerKeyAddressingMode::Matrix;
        let logical_mode = PerKeyAddressingMode::Logical;
        let hid_mode = PerKeyAddressingMode::HidCode;

        // All modes should be different (deprecated variants are separate)
        assert_ne!(matrix_mode, hid_mode);
        assert_ne!(logical_mode, hid_mode);
        assert_ne!(matrix_mode, logical_mode);

        // Commands created with deprecated modes are converted to HidCode
        // (the deprecated modes are no longer supported)
        let cmd_matrix = PerKeyRgbCommand::new(matrix_mode);
        let cmd_logical = PerKeyRgbCommand::new(logical_mode);

        // The addressing mode is always HidCode regardless of input
        assert_eq!(cmd_matrix.addressing_mode, hid_mode);
        assert_eq!(cmd_logical.addressing_mode, hid_mode);
    }

    #[test]
    fn test_per_key_rgb_manipulation() {
        let mut cmd = PerKeyRgbCommand::new(PerKeyAddressingMode::HidCode);
        let addr1 = KeyAddress::new(0x04); // HID code for 'a'
        let addr2 = KeyAddress::new(0x05); // HID code for 'b'

        // Add keys
        cmd.set_key_color(addr1, Color::RED);
        cmd.set_key_color(addr2, Color::GREEN);
        assert_eq!(cmd.key_count(), 2);

        // Check colors
        assert_eq!(cmd.get_key_color(addr1), Some(Color::RED));
        assert_eq!(cmd.get_key_color(addr2), Some(Color::GREEN));

        // Remove key
        cmd.remove_key(addr1);
        assert_eq!(cmd.key_count(), 1);
        assert_eq!(cmd.get_key_color(addr1), None);
        assert_eq!(cmd.get_key_color(addr2), Some(Color::GREEN));

        // Get addresses
        let addresses = cmd.get_addresses();
        assert_eq!(addresses.len(), 1);
        assert!(addresses.contains(&addr2));
    }

    // Tests for HID error recovery system
    #[test]
    fn test_hid_connection_health_tracking() {
        use std::time::Duration;

        let mut health = ConnectionHealth::default();
        assert!(health.is_healthy());
        assert_eq!(health.consecutive_failures, 0);

        // Record successful operations
        health.record_operation(true, Duration::from_millis(10));
        assert!(health.is_healthy());
        assert_eq!(health.success_rate, 1.0);

        // Record some failures
        health.record_operation(false, Duration::from_millis(50));
        health.record_operation(false, Duration::from_millis(50));

        assert_eq!(health.consecutive_failures, 2);
        assert_eq!(health.success_rate, 1.0 / 3.0);
        assert!(!health.is_healthy()); // Not healthy due to low success rate (33%)

        // More failures should trigger unhealthy state
        health.record_operation(false, Duration::from_millis(50));
        health.record_operation(false, Duration::from_millis(50));

        assert_eq!(health.consecutive_failures, 4);
        assert!(!health.is_healthy()); // Now unhealthy
        assert!(health.needs_recovery()); // Needs recovery
    }

    #[test]
    fn test_hid_report_validation() {
        // Valid report
        let valid_report = vec![0u8; 65];
        assert!(ReportValidator::validate_report(&valid_report).is_ok());

        // Empty report
        let empty_report = vec![];
        assert!(ReportValidator::validate_report(&empty_report).is_err());

        // Too large report
        let large_report = vec![0u8; 100];
        assert!(ReportValidator::validate_report(&large_report).is_err());

        // RGB command validation
        let mut rgb_report = vec![0u8; 65];
        rgb_report[1] = 0x21; // RGB command
        assert!(ReportValidator::validate_report(&rgb_report).is_ok());
    }

    #[test]
    fn test_hid_report_correction() {
        // Test padding
        let mut short_report = vec![0u8; 30];
        assert!(ReportValidator::validate_and_correct(&mut short_report).is_ok());
        assert_eq!(short_report.len(), 65);

        // Test truncation
        let mut long_report = vec![0u8; 100];
        assert!(ReportValidator::validate_and_correct(&mut long_report).is_ok());
        assert_eq!(long_report.len(), 65);
    }

    #[test]
    fn test_hid_error_recovery_creation() {
        let recovery = HidErrorRecovery::new();
        assert!(recovery.health().is_healthy());
        assert_eq!(recovery.health().consecutive_failures, 0);
        assert!(recovery.should_attempt_recovery());
    }

    #[test]
    fn test_actuation_command_creation() {
        // Test creation from value
        let cmd = ActuationCommand::new(4);
        assert_eq!(cmd.actuation_point, 4);
        assert_eq!(cmd.to_mm(), 0.4);

        // Test creation from mm
        let cmd = ActuationCommand::from_mm(0.4);
        assert_eq!(cmd.actuation_point, 4);
        assert_eq!(cmd.to_mm(), 0.4);

        // Test creation with rounding
        let cmd = ActuationCommand::from_mm(0.37);
        assert_eq!(cmd.actuation_point, 4); // Rounded to nearest 0.1mm

        // Test clamping
        let cmd = ActuationCommand::from_mm(5.0);
        assert_eq!(cmd.actuation_point, 40); // Clamped to max 4.0mm (40 in 0.1mm units)
    }

    #[test]
    fn test_actuation_command_validation() {
        // Valid values
        let cmd = ActuationCommand::new(1);
        assert!(cmd.validate().is_ok());

        let cmd = ActuationCommand::new(40);
        assert!(cmd.validate().is_ok());

        // Invalid: zero
        let cmd = ActuationCommand::new(0);
        assert!(cmd.validate().is_err());

        // Invalid: too high
        let cmd = ActuationCommand::new(41);
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_actuation_command_serialization() {
        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);
        let mut buffer = [0u8; KEYBOARD_REPORT_SIZE];

        // Test valid command serialization
        let cmd = ActuationCommand::new(20);
        let size = builder.build_report(cmd, &mut buffer).unwrap();

        assert_eq!(size, KEYBOARD_REPORT_SIZE);
        assert_eq!(buffer[0], 0x00); // Report ID
        assert_eq!(buffer[1], 0x2D); // Actuation command
        assert_eq!(buffer[2], 20); // Actuation value

        // Test headset serialization
        let builder = HidReportBuilder::new(HidDeviceType::Headset);
        let mut buffer = [0u8; HEADSET_REPORT_SIZE];
        let cmd = ActuationCommand::new(15);
        let size = builder.build_report(cmd, &mut buffer).unwrap();
        assert_eq!(size, HEADSET_REPORT_SIZE);
        assert_eq!(buffer[0], 0x2D); // Actuation command (no report ID)
        assert_eq!(buffer[1], 15); // Actuation value
    }

    #[test]
    fn test_actuation_command_parsing() {
        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);

        // Test parsing actuation command from data
        let actuation_data = vec![0x00, 0x2D, 25, 0, 0]; // Actuation command
        assert_eq!(
            builder.parse_command_code(&actuation_data),
            Some(CommandCode::ActuationControl)
        );
    }
}
