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

/// HID feature report size for the Apex Pro TKL 2023 per-key RGB command (0x40).
///
/// Confirmed via IOCTL_HID_SET_FEATURE capture: 3-byte header + 84 key entries × 4 bytes + 306-byte zero padding.
#[cfg(feature = "experimental-apex-2023")]
pub const APEX_2023_PERKEY_REPORT_SIZE: usize = 645;

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
    ///
    /// This is the logical code of every brightness command. This crate's legacy dialect writes
    /// `0x22`; the OpenRGB-referenced Apex per-key and 8-zone families write `0x23` on the wire
    /// (see [`IlluminationBrightnessCommand`]), which collides with the `PerKeyRgb` placeholder.
    Brightness = 0x22,
    /// Reactive mode (0x25)
    ReactiveMode = 0x25,
    /// Color shift (0x26)
    ColorShift = 0x26,
    /// Per-key RGB control (0x23)
    /// NOTE: This command code is a placeholder. The actual per-key RGB command
    /// for SteelSeries keyboards has not been discovered and may be different.
    PerKeyRgb = 0x23,
    /// Apex Pro TKL 2023 experimental direct per-key RGB control (0x40)
    ///
    /// NOTE: This command is based on reverse-engineering notes and is kept
    /// behind the `experimental-apex-2023` feature until validated on hardware.
    Apex2023Direct = 0x40,
    /// Actuation point control (0x2D) - EXPERIMENTAL
    /// NOTE: This command code is experimental based on hardware research.
    ActuationControl = 0x2D,
    /// [EXPERIMENTAL] Gen 1 Apex per-key direct frame (0x3A). Reference: OpenRGB
    /// `APEX_GEN1_PACKET_ID_DIRECT`, apex7tkl_linux `send_colors`.
    ApexLegacyDirect = 0x3A,
    /// [EXPERIMENTAL] Apex 2023 / Gen 3 wireless per-key direct frame (0x61). Reference: OpenRGB
    /// `APEX_2023_PACKET_ID_DIRECT_WIRELESS`.
    Apex2023DirectWireless = 0x61,
    /// [EXPERIMENTAL] Apex Gen 3 initialisation (0x4B). Reference: OpenRGB `APEX_2023_PACKET_ID_INIT`.
    Apex2023Init = 0x4B,
    /// [EXPERIMENTAL] Illumination brightness read-back request (0xA3). Reference: OpenRGB
    /// `APEX_PACKET_ID_GET_BRIGHTNESS`.
    BrightnessQuery = 0xA3,
    /// [EXPERIMENTAL] Firmware version request (0x90). Reference: OpenRGB `APEX_PACKET_ID_FIRMWARE`.
    FirmwareQuery = 0x90,
    /// [EXPERIMENTAL] Apex 3 tri-zone brightness (0x0A). Reference: OpenRGB `SteelSeriesApexTZoneController`.
    TriZoneBrightness = 0x0A,
    /// [EXPERIMENTAL] Apex 3 tri-zone colours (0x0B). Reference: OpenRGB `SteelSeriesApexTZoneController`.
    TriZoneColor = 0x0B,
    /// [EXPERIMENTAL] First report of the Apex 3 tri-zone save sequence (0x06); the second is `Apply`.
    TriZoneSave = 0x06,
    /// [EXPERIMENTAL] Old Apex / Apex 350 zone colours (0x07). Reference: OpenRGB
    /// `SteelSeriesOldApexController`, apexctl `ID_COLORS`.
    OldApexColor = 0x07,
    /// [EXPERIMENTAL] Old Apex polling rate (0x04, sent as the feature report ID). Reference: apexctl `ID_POLL`.
    OldApexPollingRate = 0x04,
    /// [EXPERIMENTAL] Apex M750 direct frame. The M750 has no command byte at the usual offset; `0x8E`
    /// is byte 4 of every direct frame. Reference: OpenRGB `SteelSeriesApexMController`.
    ApexMDirect = 0x8E,
    /// [EXPERIMENTAL] Apex M750 LED-control enable sequence; `0x85` is byte 6 of its first report.
    ApexMEnable = 0x85,
    /// [EXPERIMENTAL] Apex Pro TKL 2023 live actuation frame (0x38). Reference: apex-web `PROTOCOL.md`.
    ActuationLive2023 = 0x38,
    /// [EXPERIMENTAL] Apex Pro TKL (Gen 1) live actuation frame (0x31 0x47). Reference: apex-control
    /// `Actuation.cs`.
    ActuationLiveGen1 = 0x31,
    /// [EXPERIMENTAL] Apex Pro TKL (Gen 1) live Rapid Tap on/off (0x1A). Reference: apex-control
    /// `PROTOCOL_NOTES.md`.
    RapidTap = 0x1A,
    /// One write chunk of one onboard FS object (`0x03`, `macho-write-payload` in the engine's
    /// own descriptor: `PACKET.md` §10). This is the write side of the Apex 7's onboard file
    /// system, whose payloads carry the profile, its keybindings and its lighting.
    ConfigWrite = 0x03,
    /// Wipe one onboard FS object before writing it back: body `88 00 <u16 fs_id>`.
    FsErase = 0x88,
    /// Hand one whole FS block to the device once both halves are in: body
    /// `05 00 <u16 fs_id> <u32 offset> <u16 size>`.
    FsCommit = 0x05,
    /// Ask the device to validate the profile it just took: body `eb <profile index>`.
    FsValidate = 0xEB,
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
            CommandCode::ApexLegacyDirect => write!(f, "APEX_DIRECT_GEN1_EXPERIMENTAL"),
            CommandCode::Apex2023DirectWireless => write!(f, "APEX2023_DIRECT_WIRELESS_EXPERIMENTAL"),
            CommandCode::Apex2023Init => write!(f, "APEX2023_INIT_EXPERIMENTAL"),
            CommandCode::BrightnessQuery => write!(f, "BRIGHTNESS_QUERY_EXPERIMENTAL"),
            CommandCode::FirmwareQuery => write!(f, "FIRMWARE_QUERY_EXPERIMENTAL"),
            CommandCode::TriZoneBrightness => write!(f, "TRIZONE_BRIGHTNESS_EXPERIMENTAL"),
            CommandCode::TriZoneColor => write!(f, "TRIZONE_COLOR_EXPERIMENTAL"),
            CommandCode::TriZoneSave => write!(f, "TRIZONE_SAVE_EXPERIMENTAL"),
            CommandCode::OldApexColor => write!(f, "OLD_APEX_COLOR_EXPERIMENTAL"),
            CommandCode::OldApexPollingRate => write!(f, "OLD_APEX_POLLING_RATE_EXPERIMENTAL"),
            CommandCode::ApexMDirect => write!(f, "APEX_M_DIRECT_EXPERIMENTAL"),
            CommandCode::ApexMEnable => write!(f, "APEX_M_ENABLE_EXPERIMENTAL"),
            CommandCode::ActuationLive2023 => write!(f, "ACTUATION_LIVE_2023_EXPERIMENTAL"),
            CommandCode::ActuationLiveGen1 => write!(f, "ACTUATION_LIVE_GEN1_EXPERIMENTAL"),
            CommandCode::RapidTap => write!(f, "RAPID_TAP_EXPERIMENTAL"),
            CommandCode::ConfigWrite => write!(f, "FS_WRITE"),
            CommandCode::FsErase => write!(f, "FS_ERASE"),
            CommandCode::FsCommit => write!(f, "FS_COMMIT"),
            CommandCode::FsValidate => write!(f, "FS_VALIDATE"),
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
            0x3A => Some(CommandCode::ApexLegacyDirect),
            0x61 => Some(CommandCode::Apex2023DirectWireless),
            0x4B => Some(CommandCode::Apex2023Init),
            0xA3 => Some(CommandCode::BrightnessQuery),
            0x90 => Some(CommandCode::FirmwareQuery),
            0x0A => Some(CommandCode::TriZoneBrightness),
            0x0B => Some(CommandCode::TriZoneColor),
            0x06 => Some(CommandCode::TriZoneSave),
            0x07 => Some(CommandCode::OldApexColor),
            0x04 => Some(CommandCode::OldApexPollingRate),
            0x8E => Some(CommandCode::ApexMDirect),
            0x85 => Some(CommandCode::ApexMEnable),
            0x38 => Some(CommandCode::ActuationLive2023),
            0x31 => Some(CommandCode::ActuationLiveGen1),
            0x1A => Some(CommandCode::RapidTap),
            0x03 => Some(CommandCode::ConfigWrite),
            0x88 => Some(CommandCode::FsErase),
            0x05 => Some(CommandCode::FsCommit),
            0xEB => Some(CommandCode::FsValidate),
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

    /// Whether the report starts with report ID `0x00` followed by a known command byte.
    ///
    /// `HidReportBuilder` checks that header only when this returns `true`. Reports framed
    /// differently (Apex M750 frames, apexctl feature reports whose report ID is the command)
    /// return `false`.
    fn has_standard_header(&self) -> bool {
        true
    }
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

/// Confirmed direct per-key RGB command for Apex Pro TKL 2023 (`0x1628`).
///
/// Sends a 645-byte HID feature report (IOCTL_HID_SET_FEATURE, code 0x40) containing
/// up to 84 key entries. Each entry is `[key_id, R, G, B]` where key_id is the USB HID
/// keyboard usage ID. The count byte is always 0x54 (= 84). Remaining bytes are zero-padded.
///
/// Packet layout (confirmed via live capture from SteelSeriesEngine.exe):
/// ```text
/// [0x00][0x40][0x54][key_id R G B] × 84 + [0x00 × 306]
/// ```
#[cfg(feature = "experimental-apex-2023")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Apex2023DirectCommand {
    /// Key ID and color pairs.
    ///
    /// `key_id` is a USB HID keyboard usage ID (e.g. 0x04 = A, 0x28 = Enter).
    /// Keys are transmitted in the order added; the firmware ignores order.
    pub key_colors: smallvec::SmallVec<[(u8, Color); 84]>,
}

#[cfg(feature = "experimental-apex-2023")]
impl Apex2023DirectCommand {
    /// Maximum key entries per report (confirmed via live IOCTL capture).
    ///
    /// The 645-byte report carries exactly 84 `[key_id R G B]` entries.
    /// The count byte (offset 2) is always 0x54 = 84.
    pub const MAX_KEYS_PER_REPORT: usize = 84;

    /// Fixed count byte transmitted at offset 2 of every per-key RGB report.
    const KEY_COUNT_BYTE: u8 = 0x54;

    /// Create an empty direct command.
    pub fn new() -> Self {
        Self {
            key_colors: smallvec::SmallVec::new(),
        }
    }

    /// Add or update a logical key color.
    pub fn set_key_color(&mut self, key_id: u8, color: Color) {
        // Linear scan via a loop over idiomatic slice iterator.
        // The overhead of iter_mut().find() closure is eliminated.
        for (existing_key_id, existing_color) in &mut self.key_colors {
            if *existing_key_id == key_id {
                *existing_color = color;
                return;
            }
        }

        // Prevent exceeding the fixed boundary when pushing
        if self.key_colors.len() < Self::MAX_KEYS_PER_REPORT {
            self.key_colors.push((key_id, color));
        }
    }

    /// Number of logical keys in this command.
    pub fn key_count(&self) -> usize {
        self.key_colors.len()
    }

    /// Whether the command contains no key updates.
    pub fn is_empty(&self) -> bool {
        self.key_colors.is_empty()
    }
}

#[cfg(feature = "experimental-apex-2023")]
impl Default for Apex2023DirectCommand {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "experimental-apex-2023")]
impl HidCommand for Apex2023DirectCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::Apex2023Direct
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;

        if buffer.len() < APEX_2023_PERKEY_REPORT_SIZE {
            return Err(Error::DeviceCommunication(format!(
                "Buffer too small: {} bytes (expected {})",
                buffer.len(),
                APEX_2023_PERKEY_REPORT_SIZE
            )));
        }

        buffer[..APEX_2023_PERKEY_REPORT_SIZE].fill(0);

        buffer[0] = 0x00; // report ID
        buffer[1] = CommandCode::Apex2023Direct as u8;
        buffer[2] = Self::KEY_COUNT_BYTE;

        let mut offset = 3;
        for (key_id, color) in &self.key_colors {
            buffer[offset] = *key_id;
            buffer[offset + 1] = color.r;
            buffer[offset + 2] = color.g;
            buffer[offset + 3] = color.b;
            offset += 4;
        }

        Ok(APEX_2023_PERKEY_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if self.key_colors.is_empty() {
            return Err(Error::DeviceCommunication(
                "Apex 2023 per-key RGB command must have at least one key".to_string(),
            ));
        }

        if self.key_colors.len() > Self::MAX_KEYS_PER_REPORT {
            return Err(Error::DeviceCommunication(format!(
                "Apex 2023 per-key RGB command exceeds {} keys per report",
                Self::MAX_KEYS_PER_REPORT
            )));
        }

        Ok(())
    }

    fn description(&self) -> String {
        format!("Apex 2023 per-key RGB for {} keys", self.key_colors.len())
    }
}

/// One onboard FS block: the device receives an object `0xA000` octets at a time, and a
/// profile payload is padded to a whole number of blocks (`PACKET.md` §9: 40 blocks of
/// profiles, 12 of the 12288-octet lighting map).
pub const FS_BLOCK: usize = 0x400;

/// Most data octets one write chunk carries (the engine's `max_data_bytes`).
pub const FS_WRITE_MAX: usize = 0x200;

/// The width of the feature report a write chunk goes out in: `HIDFEATURE 643` in the engine's
/// device code, and a 643-octet report on the wire in the captures (`PACKET.md` §9, the write
/// frame). It is
/// wider than a fixed-size report, which is why a write chunk is padded to it by its caller.
pub const FS_WRITE_REPORT_SIZE: usize = 643;

/// Erase one onboard FS object before writing it back: body `88 00 <u16 fs_id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsEraseCommand {
    pub fs_id: u16,
}

impl FsEraseCommand {
    pub fn new(fs_id: u16) -> Self {
        Self { fs_id }
    }
}

impl HidCommand for FsEraseCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::FsErase
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
        let body = if device_type.includes_report_id() { 1 } else { 0 };

        buffer[body] = self.command_code() as u8;
        buffer[body + 1] = 0x00;
        buffer[body + 2] = (self.fs_id & 0xFF) as u8;
        buffer[body + 3] = (self.fs_id >> 8) as u8;

        Ok(report_size)
    }

    fn validate(&self) -> Result<()> {
        if self.fs_id == 0 {
            return Err(Error::InvalidConfig(
                "FS object id 0 is not an onboard object".to_string(),
            ));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("erase FS object {:#06x}", self.fs_id)
    }
}

/// Commit one whole FS block once both of its halves have been written: body
/// `05 00 <u16 fs_id> <u32 offset> <u16 size>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsCommitCommand {
    pub fs_id: u16,
    pub block_offset: usize,
}

impl FsCommitCommand {
    pub fn new(fs_id: u16, block_offset: usize) -> Self {
        Self { fs_id, block_offset }
    }
}

impl HidCommand for FsCommitCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::FsCommit
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
        let body = if device_type.includes_report_id() { 1 } else { 0 };

        buffer[body] = self.command_code() as u8;
        buffer[body + 1] = 0x00;
        buffer[body + 2] = (self.fs_id & 0xFF) as u8;
        buffer[body + 3] = (self.fs_id >> 8) as u8;
        buffer[body + 4] = (self.block_offset & 0xFF) as u8;
        buffer[body + 5] = ((self.block_offset >> 8) & 0xFF) as u8;
        buffer[body + 6] = ((self.block_offset >> 16) & 0xFF) as u8;
        buffer[body + 7] = (self.block_offset >> 24) as u8;
        buffer[body + 8] = (FS_BLOCK & 0xFF) as u8;
        buffer[body + 9] = (FS_BLOCK >> 8) as u8;

        Ok(report_size)
    }

    fn validate(&self) -> Result<()> {
        if self.fs_id == 0 {
            return Err(Error::InvalidConfig(
                "FS object id 0 is not an onboard object".to_string(),
            ));
        }
        if self.block_offset % FS_BLOCK != 0 {
            return Err(Error::InvalidConfig(
                "a commit names a whole FS block, so its offset must be a multiple of the block size".to_string(),
            ));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!(
            "commit the block at offset {} of FS object {:#06x}",
            self.block_offset, self.fs_id
        )
    }
}

/// Ask the device to validate one onboard profile after the rewrite: body `eb <profile index>`,
/// one-based, the profile the client just wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsValidateCommand {
    pub profile_index: u8,
}

impl FsValidateCommand {
    pub fn new(profile_index: u8) -> Self {
        Self { profile_index }
    }
}

impl HidCommand for FsValidateCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::FsValidate
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
        let body = if device_type.includes_report_id() { 1 } else { 0 };

        buffer[body] = self.command_code() as u8;
        buffer[body + 1] = self.profile_index;

        Ok(report_size)
    }

    fn validate(&self) -> Result<()> {
        if self.profile_index == 0 {
            return Err(Error::InvalidConfig(
                "profile indexes are one-based, so 0 is not a profile".to_string(),
            ));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("validate profile {}", self.profile_index)
    }
}

/// The crc the device puts at the end of an onboard payload: the CRC-32 walk the libsteel
/// device code seeds with `0x04C11DB7` (`PACKET.md` §10, checked against the captures by
/// `tools/stm32crc.py`). It runs over the payload with its own four trailing octets zeroed.
pub fn stm32crc(data: Vec<u8>) -> u32 {
    return stm32crc_with(data, 0x04C11DB7, 0xFFFF_FFFF);
}

/// The same walk with an explicit polynomial and seed: four octets at a time, MSB first, with
/// a short tail zero-padded on the high end.
pub fn stm32crc_with(data: Vec<u8>, poly: u32, init: u32) -> u32 {
    let table = crc_table(poly);
    let mut crc = init;
    let mut i = 0;
    let n = data.len();
    while i + 4 <= n {
        for k in (0..4).rev() {
            crc = (crc << 8) ^ table[(((crc >> 24) ^ (data[i + k] as u32)) & 0xFF) as usize];
        }
        i = i + 4;
    }
    let rem = n - i;
    if rem > 0 {
        let mut word = 0u32;
        for k in 0..rem {
            word |= (data[i + k] as u32) << (24 - 8 * k);
        }
        word &= 0xFFFF_FFFF << (32 - 8 * rem);
        for k in 0..4 {
            let b = ((word >> (24 - 8 * k)) & 0xFF) as usize;
            crc = (crc << 8) ^ table[((crc >> 24) as usize ^ b) & 0xFF];
        }
    }
    return crc;
}

/// The 256-entry table for the table-driven walk above.
fn crc_table(poly: u32) -> [u32; 256] {
    let mut out: [u32; 256] = [0u32; 256];
    for i in 0..256 {
        let mut c: u32 = (i as u32) << 24;
        for _ in 0..8 {
            if (c & 0x8000_0000) != 0 {
                c = (c << 1) ^ poly;
            } else {
                c = c << 1;
            }
        }
        out[i] = c;
    }
    return out;
}

/// One write of one FS object: body `03 00 <u16 offset> <u16 size>` followed by `size` data
/// octets - the engine's `ConfigWrite` on the write side (`PACKET.md` §6.1 lists its command
/// octet, §9 and §10 list this use of it). The offset counts inside one FS block, so the two
/// halves of a block send 0 and 512. A write chunk is wider than a fixed-size report, so it
/// goes through `HidCommand::serialize` rather than `HidReportBuilder::build_report`, and the
/// caller zero-pads the rest of the report and sends its full width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsWriteCommand {
    pub offset: usize,
    pub data: Vec<u8>,
}

impl FsWriteCommand {
    pub fn new(offset: usize, data: Vec<u8>) -> Self {
        Self { offset, data }
    }
}

impl HidCommand for FsWriteCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::ConfigWrite
    }

    fn serialize(&self, buffer: &mut [u8], device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;

        let body = if device_type.includes_report_id() { 1 } else { 0 };
        let needed = body + 6 + self.data.len();
        if buffer.len() < needed {
            return Err(Error::DeviceCommunication(format!(
                "Buffer too small: {} bytes (expected {})",
                buffer.len(),
                needed
            )));
        }

        buffer[..needed].fill(0);

        buffer[body] = self.command_code() as u8;
        buffer[body + 1] = 0x00;
        buffer[body + 2] = (self.offset & 0xFF) as u8;
        buffer[body + 3] = (self.offset >> 8) as u8;
        buffer[body + 4] = (self.data.len() & 0xFF) as u8;
        buffer[body + 5] = (self.data.len() >> 8) as u8;
        for (index, octet) in self.data.iter().enumerate() {
            buffer[body + 6 + index] = *octet;
        }

        Ok(needed)
    }

    fn validate(&self) -> Result<()> {
        if self.data.len() == 0 {
            return Err(Error::InvalidConfig("a write command must have a payload".to_string()));
        }
        if self.data.len() > FS_WRITE_MAX {
            return Err(Error::InvalidConfig(
                "a write command's payload is at most max_data_bytes of the write API".to_string(),
            ));
        }
        if self.offset + self.data.len() > FS_BLOCK {
            return Err(Error::InvalidConfig(
                "a write command's payload stays inside one FS block".to_string(),
            ));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("write {} octets at offset {}", self.data.len(), self.offset)
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
// ===========================================================================
// OpenRGB-referenced Apex families [EXPERIMENTAL]
//
// Every layout in this section is taken from a published open-source driver. None is tested on
// hardware by this project.
// - OpenRGB `Controllers/SteelSeriesController/` (GPL-2.0-or-later; byte facts only, no code
//   copied): `SteelSeriesApexBaseController.h`, `SteelSeriesApexController.cpp`,
//   `SteelSeriesApex8ZoneController.{h,cpp}`, `SteelSeriesApexTZoneController.{h,cpp}`,
//   `SteelSeriesApexMController.cpp`, `SteelSeriesOldApexController.cpp`.
// - apexctl `src/apexctl.c` (Apache-2.0, https://github.com/AstroSnail/apexctl): old Apex polling rate.
// - apex-web (MIT, https://github.com/trottyva/apex-web, `PROTOCOL.md`, `index.html`,
//   `capture-data.js`): Apex Pro TKL 2023 live actuation.
// - apex-control (MIT, https://github.com/zunuza/apex-control, `src/ApexControl.Core/Actuation.cs`,
//   `docs/PROTOCOL_NOTES.md`): Apex Pro TKL live actuation and Rapid Tap.
//
// Every buffer starts with the HID report ID and is written to the device unchanged, so the
// command byte is the first byte the keyboard receives. `GenericKeyboard`'s legacy output path
// inserts one more `0x00` in front of the report, so these reports must not go through it.
// ===========================================================================

/// Output report of the OpenRGB Apex controllers: report ID + 64 bytes (`STEELSERIES_PACKET_OUT_SIZE`).
pub const APEX_OUTPUT_REPORT_SIZE: usize = 65;
/// Per-key direct frame: report ID + 642 bytes (`APEX_PACKET_LENGTH`).
pub const APEX_DIRECT_REPORT_SIZE: usize = 643;
/// Apex 9 TKL / Apex 9 Mini direct frame (`APEX_9_PACKET_LENGTH`).
pub const APEX_9_DIRECT_REPORT_SIZE: usize = 513;
/// Apex 3 tri-zone (`STEELSERIES_TZ_WRITE_PACKET_SIZE`) and old Apex output reports: report ID + 32 bytes.
pub const APEX_SHORT_REPORT_SIZE: usize = 33;
/// Apex M750 feature report (`SS_APEX_M_PACKET_SIZE`).
pub const APEX_M_REPORT_SIZE: usize = 513;
/// Apex Pro TKL 2023 live actuation frame: report ID + 644 bytes (apex-web `bloc0x38`).
pub const APEX_2023_ACTUATION_REPORT_SIZE: usize = 645;
/// Apex Pro TKL (Gen 1) live actuation frame: report ID + 642 bytes (apex-control `BuildFrame`).
pub const APEX_GEN1_ACTUATION_REPORT_SIZE: usize = 643;
/// Highest illumination level of the per-key family (`APEX_BRIGHTNESS_MAX`).
pub const APEX_ILLUMINATION_MAX: u8 = 10;
/// Highest brightness of the 8-zone family (`STEELSERIES_8Z_BRIGHTNESS_MAX`).
pub const EIGHT_ZONE_BRIGHTNESS_MAX: u8 = 0x10;
/// Highest brightness of the tri-zone family (`STEELSERIES_TZ_BRIGHTNESS_MAX`).
pub const TRI_ZONE_BRIGHTNESS_MAX: u8 = 0x64;
/// Zones driven by the 8-zone family (`STEELSERIES_8Z_LED_COUNT`).
pub const EIGHT_ZONE_COUNT: usize = 8;
/// Zones driven by the tri-zone family (`STEELSERIES_TZ_LED_COUNT`).
pub const TRI_ZONE_COUNT: usize = 10;
/// Zones of the old Apex, in wire order: QWERTY, ten-key, function keys, MX keys, logo.
pub const OLD_APEX_ZONE_COUNT: usize = 5;
/// Lowest old Apex zone brightness (apexctl: 1 turns the zone off).
pub const OLD_APEX_BRIGHTNESS_MIN: u8 = 1;
/// Highest old Apex zone brightness (apexctl; OpenRGB always sends this value).
pub const OLD_APEX_BRIGHTNESS_MAX: u8 = 8;
/// Slots in an Apex M750 direct frame: 6 rows of 22 columns.
pub const APEX_M_SLOT_COUNT: usize = 132;

fn prepare_buffer(buffer: &mut [u8], size: usize) -> Result<()> {
    if buffer.len() < size {
        return Err(Error::DeviceCommunication(format!(
            "Buffer too small: {} bytes (expected {})",
            buffer.len(),
            size
        )));
    }
    buffer[..size].fill(0);
    Ok(())
}

/// Packet ID of the per-key direct frame, chosen by protocol generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApexDirectPacket {
    /// Gen 1 (2019-22 case design): `0x3A`.
    Gen1,
    /// Gen 2 / Gen 3, wired: `0x40`.
    Wired2023,
    /// Gen 2 / Gen 3 wireless models, on the dongle or on cable: `0x61`.
    Wireless2023,
}

impl ApexDirectPacket {
    /// Command code carried by this packet.
    pub const fn command_code(self) -> CommandCode {
        match self {
            Self::Gen1 => CommandCode::ApexLegacyDirect,
            Self::Wired2023 => CommandCode::Apex2023Direct,
            Self::Wireless2023 => CommandCode::Apex2023DirectWireless,
        }
    }

    /// Byte written after the report ID.
    pub const fn byte(self) -> u8 {
        self.command_code() as u8
    }
}

/// [EXPERIMENTAL] Per-key direct frame of the Apex 5/7/9/Pro family.
///
/// Feature report `[0x00][packet][count][hid R G B] x count`, zero-padded to `report_len`.
/// Reference: OpenRGB `SteelSeriesApexController::SetLEDsDirect`. The Gen 1 layout also matches
/// apex7tkl_linux `Device.send_colors` (`0x3A`, 642-byte payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexDirectCommand {
    /// Packet ID.
    pub packet: ApexDirectPacket,
    /// Total report length including the report ID (643, or 513 on the Apex 9 series).
    pub report_len: usize,
    /// `(HID usage, colour)` pairs in wire order.
    pub entries: Vec<(u8, Color)>,
}

impl ApexDirectCommand {
    /// Create an empty frame.
    pub fn new(packet: ApexDirectPacket, report_len: usize) -> Self {
        Self {
            packet,
            report_len,
            entries: Vec::new(),
        }
    }

    /// Append one key.
    pub fn push(&mut self, hid_code: u8, color: Color) {
        self.entries.push((hid_code, color));
    }

    /// Most entries that fit: `count * 4 + 3 <= report_len`, and `count` is one byte.
    pub fn capacity(&self) -> usize {
        (self.report_len.saturating_sub(3) / 4).min(u8::MAX as usize)
    }
}

impl HidCommand for ApexDirectCommand {
    fn command_code(&self) -> CommandCode {
        self.packet.command_code()
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        prepare_buffer(buffer, self.report_len)?;
        buffer[1] = self.packet.byte();
        buffer[2] = self.entries.len() as u8;
        for (i, (hid_code, color)) in self.entries.iter().enumerate() {
            let offset = 3 + i * 4;
            buffer[offset] = *hid_code;
            buffer[offset + 1] = color.r;
            buffer[offset + 2] = color.g;
            buffer[offset + 3] = color.b;
        }
        Ok(self.report_len)
    }

    fn validate(&self) -> Result<()> {
        if self.report_len < 7 {
            return Err(Error::DeviceCommunication(format!(
                "Apex direct frame length {} is too short",
                self.report_len
            )));
        }
        if self.entries.is_empty() {
            return Err(Error::DeviceCommunication(
                "Apex direct frame must contain at least one key".to_string(),
            ));
        }
        if self.entries.len() > self.capacity() {
            return Err(Error::DeviceCommunication(format!(
                "Apex direct frame holds at most {} keys in {} bytes, got {}",
                self.capacity(),
                self.report_len,
                self.entries.len()
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!(
            "Apex direct frame {:#04x} for {} keys",
            self.packet.byte(),
            self.entries.len()
        )
    }
}

/// [EXPERIMENTAL] Gen 3 initialisation feature report: `[0x00][0x4B]`, zero-padded.
///
/// OpenRGB sends it as a 65-byte report (`SteelSeriesApexController::SendInitialization`). This
/// crate's Apex Pro TKL 2023 Wireless / Gen 3 path sends the same two bytes in a 643-byte report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexInitCommand {
    /// Total report length including the report ID.
    pub report_len: usize,
}

impl ApexInitCommand {
    /// Create an init report of `report_len` bytes.
    pub fn new(report_len: usize) -> Self {
        Self { report_len }
    }
}

impl HidCommand for ApexInitCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::Apex2023Init
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        prepare_buffer(buffer, self.report_len)?;
        buffer[1] = CommandCode::Apex2023Init as u8;
        Ok(self.report_len)
    }

    fn validate(&self) -> Result<()> {
        if self.report_len < 2 {
            return Err(Error::DeviceCommunication(
                "Apex init report needs at least 2 bytes".to_string(),
            ));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Apex Gen 3 init ({} bytes)", self.report_len)
    }
}

/// [EXPERIMENTAL] Illumination brightness: output report `[0x00][0x23][level]`.
///
/// Per-key family: `0..=10` (OpenRGB `SteelSeriesApexController::SetBrightness`). 8-zone family:
/// `0..=0x10` (OpenRGB `SteelSeriesApex8ZoneController::SetBrightness`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IlluminationBrightnessCommand {
    /// Level to write.
    pub level: u8,
    /// Highest level the family accepts.
    pub max: u8,
}

impl IlluminationBrightnessCommand {
    /// Byte written after the report ID.
    pub const WIRE_BYTE: u8 = 0x23;

    /// Create a brightness command for a family whose highest level is `max`.
    pub fn new(level: u8, max: u8) -> Self {
        Self { level, max }
    }
}

impl HidCommand for IlluminationBrightnessCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::Brightness
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        prepare_buffer(buffer, APEX_OUTPUT_REPORT_SIZE)?;
        buffer[1] = Self::WIRE_BYTE;
        buffer[2] = self.level;
        Ok(APEX_OUTPUT_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if self.level > self.max {
            return Err(Error::DeviceCommunication(format!(
                "Brightness level {} is above the maximum {}",
                self.level, self.max
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set illumination brightness to {}/{}", self.level, self.max)
    }
}

/// [EXPERIMENTAL] Read-back requests of the per-key family: output report `[0x00][code]`.
///
/// Reference: OpenRGB `SteelSeriesApexController::ReadBrightness` (`0xA3`) and
/// `SteelSeriesApexBaseController::GetVersion` (`0x90`). The answer arrives as a 64-byte input report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApexQuery {
    /// Illumination brightness (`0xA3`); see [`parse_illumination_reply`].
    Brightness,
    /// Keyboard firmware version (`0x90`); see [`parse_apex_firmware_version`].
    FirmwareVersion,
}

impl HidCommand for ApexQuery {
    fn command_code(&self) -> CommandCode {
        match self {
            Self::Brightness => CommandCode::BrightnessQuery,
            Self::FirmwareVersion => CommandCode::FirmwareQuery,
        }
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        prepare_buffer(buffer, APEX_OUTPUT_REPORT_SIZE)?;
        buffer[1] = self.command_code() as u8;
        Ok(APEX_OUTPUT_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        Ok(())
    }

    fn description(&self) -> String {
        format!("Apex query {}", self.command_code())
    }
}

/// Parse the answer to [`ApexQuery::Brightness`] from a per-key keyboard.
///
/// Reference: OpenRGB `SteelSeriesApexController::ReadBrightness`. The answer is
/// `[0xA3][status][level]`. A status other than `0x00` (unsupported models answer `0xFF`), a
/// level above 10 or a short answer means the keyboard does not offer brightness.
pub fn parse_illumination_reply(reply: &[u8]) -> Option<u8> {
    match reply {
        [0xA3, 0x00, level, ..] if *level <= APEX_ILLUMINATION_MAX => Some(*level),
        _ => None,
    }
}

/// Parse `major.minor.patch` from the answer to [`ApexQuery::FirmwareVersion`].
///
/// Reference: OpenRGB `GetVersion`, `ExtractVersion` and `SendInitialization`: NUL bytes are
/// dropped, a leading `0x90` echo is skipped, and the text must read `major.minor.patch`.
pub fn parse_apex_firmware_version(reply: &[u8]) -> Option<(u32, u32, u32)> {
    let mut text: Vec<u8> = reply.iter().copied().filter(|&b| b != 0).collect();
    if reply.first() == Some(&(CommandCode::FirmwareQuery as u8)) && !text.is_empty() {
        text.remove(0);
    }
    let text = String::from_utf8_lossy(&text);
    let mut parts = text.splitn(3, '.');
    let whole = |part: &str| -> Option<u32> {
        let part = part.trim_start();
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let major = whole(parts.next()?)?;
    let minor = whole(parts.next()?)?;
    let patch_text = parts.next()?.trim_start();
    let digits: String = patch_text.chars().take_while(char::is_ascii_digit).collect();
    let patch = digits.parse().ok()?;
    Some((major, minor, patch))
}

/// Whether a firmware version selects the Gen 3 protocol (OpenRGB: 1.19.7 or newer).
pub fn firmware_selects_gen3(version: (u32, u32, u32)) -> bool {
    version >= (1, 19, 7)
}

/// [EXPERIMENTAL] Apex 3 tri-zone colours: output report `[0x00][0x0B][0x00][R G B] x 10`, 33 bytes.
///
/// Reference: OpenRGB `SteelSeriesApexTZoneController::SetColor`. Missing zones are sent black.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriZoneColorCommand {
    /// Zone colours, at most [`TRI_ZONE_COUNT`].
    pub colors: Vec<Color>,
}

impl HidCommand for TriZoneColorCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::TriZoneColor
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        prepare_buffer(buffer, APEX_SHORT_REPORT_SIZE)?;
        buffer[1] = CommandCode::TriZoneColor as u8;
        for (i, color) in self.colors.iter().enumerate() {
            let offset = 3 + i * 3;
            buffer[offset] = color.r;
            buffer[offset + 1] = color.g;
            buffer[offset + 2] = color.b;
        }
        Ok(APEX_SHORT_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if self.colors.len() > TRI_ZONE_COUNT {
            return Err(Error::DeviceCommunication(format!(
                "Tri-zone report holds {} zones, got {}",
                TRI_ZONE_COUNT,
                self.colors.len()
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set {} tri-zone colours", self.colors.len())
    }
}

/// [EXPERIMENTAL] Apex 3 tri-zone brightness: output report `[0x00][0x0A][0x00][level]`, 33 bytes.
///
/// Reference: OpenRGB `SteelSeriesApexTZoneController::SetColor`, which sends it before every
/// colour report. Level `0..=100`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriZoneBrightnessCommand {
    /// Level `0..=100`.
    pub level: u8,
}

impl HidCommand for TriZoneBrightnessCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::TriZoneBrightness
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        prepare_buffer(buffer, APEX_SHORT_REPORT_SIZE)?;
        buffer[1] = CommandCode::TriZoneBrightness as u8;
        buffer[3] = self.level;
        Ok(APEX_SHORT_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if self.level > TRI_ZONE_BRIGHTNESS_MAX {
            return Err(Error::DeviceCommunication(format!(
                "Tri-zone brightness {} is above {}",
                self.level, TRI_ZONE_BRIGHTNESS_MAX
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set tri-zone brightness to {}", self.level)
    }
}

/// The two reports of the Apex 3 tri-zone save sequence, sent in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriZoneSaveStep {
    /// `[0x00][0x06][0x00][0x08]`.
    Prepare,
    /// `[0x00][0x09][0x00][0x00]`.
    Commit,
}

/// [EXPERIMENTAL] One report of the Apex 3 tri-zone save sequence (33 bytes).
///
/// Reference: OpenRGB `SteelSeriesApexTZoneController::Save`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriZoneSaveCommand {
    /// Which report of the sequence.
    pub step: TriZoneSaveStep,
}

impl HidCommand for TriZoneSaveCommand {
    fn command_code(&self) -> CommandCode {
        match self.step {
            TriZoneSaveStep::Prepare => CommandCode::TriZoneSave,
            TriZoneSaveStep::Commit => CommandCode::Apply,
        }
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        prepare_buffer(buffer, APEX_SHORT_REPORT_SIZE)?;
        buffer[1] = self.command_code() as u8;
        if self.step == TriZoneSaveStep::Prepare {
            buffer[3] = 0x08;
        }
        Ok(APEX_SHORT_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        Ok(())
    }

    fn description(&self) -> String {
        format!("Tri-zone save ({:?})", self.step)
    }
}

/// [EXPERIMENTAL] Old Apex / Apex 350 zone colours: output report
/// `[0x00][0x07][0x00][R G B brightness] x 5`, 33 bytes.
///
/// Zone order: QWERTY, ten-key, function keys, MX keys, logo (OpenRGB
/// `SteelSeriesOldApexController::SetColorDetailed`). OpenRGB always sends brightness `0x08`;
/// apexctl `cmd_colors` documents 1 (off) to 8 (brightest). Missing zones are sent black.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OldApexColorCommand {
    /// Zone colours, at most [`OLD_APEX_ZONE_COUNT`].
    pub zones: Vec<Color>,
    /// Brightness written in every zone, `1..=8`.
    pub brightness: u8,
}

impl HidCommand for OldApexColorCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::OldApexColor
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        prepare_buffer(buffer, APEX_SHORT_REPORT_SIZE)?;
        buffer[1] = CommandCode::OldApexColor as u8;
        for zone in 0..OLD_APEX_ZONE_COUNT {
            let color = self.zones.get(zone).copied().unwrap_or(Color::BLACK);
            let offset = 3 + zone * 4;
            buffer[offset] = color.r;
            buffer[offset + 1] = color.g;
            buffer[offset + 2] = color.b;
            buffer[offset + 3] = self.brightness;
        }
        Ok(APEX_SHORT_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if self.zones.len() > OLD_APEX_ZONE_COUNT {
            return Err(Error::DeviceCommunication(format!(
                "Old Apex report holds {} zones, got {}",
                OLD_APEX_ZONE_COUNT,
                self.zones.len()
            )));
        }
        if !(OLD_APEX_BRIGHTNESS_MIN..=OLD_APEX_BRIGHTNESS_MAX).contains(&self.brightness) {
            return Err(Error::DeviceCommunication(format!(
                "Old Apex brightness must be {OLD_APEX_BRIGHTNESS_MIN}..={OLD_APEX_BRIGHTNESS_MAX}, got {}",
                self.brightness
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set old Apex zones (brightness {})", self.brightness)
    }
}

/// Polling rates of the old Apex (apexctl `POLL_125` .. `POLL_1000`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OldApexPollingRate {
    Hz125,
    Hz250,
    Hz500,
    Hz1000,
}

impl OldApexPollingRate {
    /// All rates, slowest first.
    pub const ALL: [Self; 4] = [Self::Hz125, Self::Hz250, Self::Hz500, Self::Hz1000];

    /// Rate in Hz.
    pub const fn hz(self) -> u16 {
        match self {
            Self::Hz125 => 125,
            Self::Hz250 => 250,
            Self::Hz500 => 500,
            Self::Hz1000 => 1000,
        }
    }

    /// Value sent on the wire.
    pub const fn index(self) -> u8 {
        match self {
            Self::Hz125 => 0,
            Self::Hz250 => 1,
            Self::Hz500 => 2,
            Self::Hz1000 => 3,
        }
    }

    /// Look a rate up by Hz.
    pub fn from_hz(hz: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|rate| rate.hz() == hz)
    }
}

/// [EXPERIMENTAL] Old Apex polling rate: 3-byte feature report `[0x04][0x00][rate]`.
///
/// Reference: apexctl `cmd_poll`, sent with `hid_send_feature_report`, so `0x04` is the report ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OldApexPollingRateCommand {
    /// Rate to select.
    pub rate: OldApexPollingRate,
}

impl OldApexPollingRateCommand {
    /// Report length.
    pub const REPORT_SIZE: usize = 3;
}

impl HidCommand for OldApexPollingRateCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::OldApexPollingRate
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        prepare_buffer(buffer, Self::REPORT_SIZE)?;
        buffer[0] = CommandCode::OldApexPollingRate as u8;
        buffer[2] = self.rate.index();
        Ok(Self::REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        Ok(())
    }

    fn description(&self) -> String {
        format!("Set old Apex polling rate to {} Hz", self.rate.hz())
    }

    fn has_standard_header(&self) -> bool {
        false
    }
}

/// The three reports of the Apex M750 LED-control enable sequence, sent in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApexMEnableStep {
    First,
    Second,
    Third,
}

impl ApexMEnableStep {
    /// The sequence in order.
    pub const ALL: [Self; 3] = [Self::First, Self::Second, Self::Third];
}

/// [EXPERIMENTAL] One report of the Apex M750 LED-control enable sequence (513-byte feature report).
///
/// Reference: OpenRGB `SteelSeriesApexMController::EnableLEDControl`. OpenRGB reuses one buffer
/// without clearing it, so the third report still carries the `0xFF` written at byte 7 by the second.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexMEnableCommand {
    /// Which report of the sequence.
    pub step: ApexMEnableStep,
}

impl HidCommand for ApexMEnableCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::ApexMEnable
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        prepare_buffer(buffer, APEX_M_REPORT_SIZE)?;
        match self.step {
            ApexMEnableStep::First => {
                buffer[4] = 0x01;
                buffer[6] = 0x85;
            }
            ApexMEnableStep::Second => {
                buffer[4] = 0x03;
                buffer[5] = 0x01;
                buffer[7] = 0xFF;
            }
            ApexMEnableStep::Third => {
                buffer[4] = 0x01;
                buffer[6] = 0x85;
                buffer[7] = 0xFF;
            }
        }
        Ok(APEX_M_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        Ok(())
    }

    fn description(&self) -> String {
        format!("Apex M750 enable ({:?})", self.step)
    }

    fn has_standard_header(&self) -> bool {
        false
    }
}

/// [EXPERIMENTAL] Apex M750 direct frame (513-byte feature report).
///
/// Header `00 00 00 01 8E 01 03 06 16`, then 132 slots of `R G B` from byte 9. Slots without a
/// key are sent as `FF 32 00`. Reference: OpenRGB `SteelSeriesApexMController::SetLEDsDirect`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApexMDirectCommand {
    /// Exactly [`APEX_M_SLOT_COUNT`] slots; `None` marks a slot without a key.
    pub slots: Vec<Option<Color>>,
}

impl ApexMDirectCommand {
    const HEADER: [u8; 9] = [0x00, 0x00, 0x00, 0x01, 0x8E, 0x01, 0x03, 0x06, 0x16];
    const EMPTY_SLOT: [u8; 3] = [0xFF, 0x32, 0x00];
}

impl HidCommand for ApexMDirectCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::ApexMDirect
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        prepare_buffer(buffer, APEX_M_REPORT_SIZE)?;
        buffer[..Self::HEADER.len()].copy_from_slice(&Self::HEADER);
        for (i, slot) in self.slots.iter().enumerate() {
            let offset = Self::HEADER.len() + i * 3;
            let bytes = match slot {
                Some(color) => [color.r, color.g, color.b],
                None => Self::EMPTY_SLOT,
            };
            buffer[offset..offset + 3].copy_from_slice(&bytes);
        }
        Ok(APEX_M_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if self.slots.len() != APEX_M_SLOT_COUNT {
            return Err(Error::DeviceCommunication(format!(
                "Apex M750 frame needs {} slots, got {}",
                APEX_M_SLOT_COUNT,
                self.slots.len()
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Apex M750 direct frame".to_string()
    }

    fn has_standard_header(&self) -> bool {
        false
    }
}

/// HID usages addressed by both live actuation frames, in wire order: `0x04..=0x28`,
/// `0x2A..=0x39`, `0x64`, `0x87..=0x8B`, `0xE0..=0xE7`, `0xF0`.
///
/// Identical in apex-web (`KEY_ORDER`, `capture-data.js`) and apex-control (`BuildKeyCodes`,
/// `Actuation.cs`). Escape, the F-row, the arrows and the navigation block are not addressed.
pub const APEX_PRO_ACTUATION_KEYS: [u8; 68] = [
    0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16,
    0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2A,
    0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x64, 0x87, 0x88, 0x89,
    0x8A, 0x8B, 0xE0, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xF0,
];

/// Actuation range shared by the live actuation settings, in 0.1 mm.
pub const ACTUATION_TENTHS_RANGE: std::ops::RangeInclusive<u8> = 1..=40;

/// apex-web raw byte for a depth in mm: `round(3.5696 + 11.2840x + 4.6982x^2 + 1.5643x^3)`,
/// clamped to `5..=224` (`mmVersOctet`, `index.html`).
fn apex_2023_raw_for_mm(mm: f64) -> u8 {
    let raw = 3.5696 + 11.2840 * mm + 4.6982 * mm * mm + 1.5643 * mm * mm * mm;
    raw.round().clamp(5.0, 224.0) as u8
}

/// apex-web actuation byte for a depth in 0.1 mm.
pub fn apex_2023_actuation_raw(tenths: u8) -> u8 {
    apex_2023_raw_for_mm(f64::from(tenths) / 10.0)
}

/// apex-web release byte: the actuation curve 0.1 mm shallower, never above 0.1 mm
/// (`relachement`, `index.html`).
pub fn apex_2023_release_raw(tenths: u8) -> u8 {
    apex_2023_raw_for_mm(f64::from(tenths.saturating_sub(1).max(1)) / 10.0)
}

/// [EXPERIMENTAL] Apex Pro TKL 2023 (`0x1628`) live actuation frame, 645-byte feature report:
/// `[0x00][0x38][0x61][count LE16][hid actuation release] x count`.
///
/// Reference: apex-web `bloc0x38` / `PROTOCOL.md`, tested by its author on `0x1628` firmware
/// 1.19.7. The setting is lost on unplug. apex-web notes that wrapping the frame in other
/// commands cancels it, so it is sent alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActuationLive2023Command {
    /// `(HID usage, actuation byte, release byte)` triplets in wire order.
    pub entries: Vec<(u8, u8, u8)>,
}

impl ActuationLive2023Command {
    /// One depth for every addressed key, in 0.1 mm (`1..=40`).
    pub fn global(tenths: u8) -> Result<Self> {
        if !ACTUATION_TENTHS_RANGE.contains(&tenths) {
            return Err(Error::InvalidConfig(format!(
                "actuation must be 1..=40 (0.1 mm steps), got {tenths}"
            )));
        }
        let actuation = apex_2023_actuation_raw(tenths);
        let release = apex_2023_release_raw(tenths);
        Ok(Self {
            entries: APEX_PRO_ACTUATION_KEYS
                .iter()
                .map(|&hid| (hid, actuation, release))
                .collect(),
        })
    }
}

impl HidCommand for ActuationLive2023Command {
    fn command_code(&self) -> CommandCode {
        CommandCode::ActuationLive2023
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        prepare_buffer(buffer, APEX_2023_ACTUATION_REPORT_SIZE)?;
        let count = self.entries.len() as u16;
        buffer[1] = CommandCode::ActuationLive2023 as u8;
        buffer[2] = 0x61;
        buffer[3..5].copy_from_slice(&count.to_le_bytes());
        for (i, (hid, actuation, release)) in self.entries.iter().enumerate() {
            let offset = 5 + i * 3;
            buffer[offset] = *hid;
            buffer[offset + 1] = *actuation;
            buffer[offset + 2] = *release;
        }
        Ok(APEX_2023_ACTUATION_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if self.entries.is_empty() || 5 + self.entries.len() * 3 > APEX_2023_ACTUATION_REPORT_SIZE {
            return Err(Error::DeviceCommunication(format!(
                "Live actuation frame cannot hold {} keys",
                self.entries.len()
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Apex 2023 live actuation for {} keys", self.entries.len())
    }
}

/// apex-control raw actuation values `(0.1 mm, raw u16)` captured from SteelSeries GG on the
/// Apex Pro TKL (`0x1614`, firmware 4.16.8). 3.2 mm is missing on purpose (it caused key-repeat
/// spam on that board); 3.7-3.9 mm were never captured. Source: `KnownGoodValues`, `Actuation.cs`.
pub const APEX_GEN1_ACTUATION_TABLE: [(u8, u16); 36] = [
    (1, 1542),
    (2, 1286),
    (3, 1544),
    (4, 2058),
    (5, 2571),
    (6, 2829),
    (7, 3343),
    (8, 3858),
    (9, 4628),
    (10, 5143),
    (11, 5913),
    (12, 6428),
    (13, 7199),
    (14, 7971),
    (15, 8998),
    (16, 9770),
    (17, 10798),
    (18, 11826),
    (19, 12855),
    (20, 14139),
    (21, 15168),
    (22, 16454),
    (23, 18124),
    (24, 19538),
    (25, 21081),
    (26, 22880),
    (27, 24680),
    (28, 26737),
    (29, 29051),
    (30, 31621),
    (31, 34192),
    (33, 40106),
    (34, 43704),
    (35, 47296),
    (36, 49352),
    (40, 54489),
];

/// Keys apex-control always sends with [`APEX_GEN1_ACTUATION_SENTINEL`]: the ISO / international
/// keys its ANSI board does not have (`SentinelKeys`, `Actuation.cs`).
pub const APEX_GEN1_ACTUATION_SENTINEL_KEYS: [u8; 7] = [0x32, 0x64, 0x87, 0x88, 0x89, 0x8A, 0x8B];

/// Fixed value for [`APEX_GEN1_ACTUATION_SENTINEL_KEYS`] (`SentinelRaw`, wire bytes `23 1F`).
pub const APEX_GEN1_ACTUATION_SENTINEL: u16 = 0x1F23;

/// Raw value for a depth in 0.1 mm, if apex-control captured one.
pub fn apex_gen1_actuation_raw(tenths: u8) -> Option<u16> {
    APEX_GEN1_ACTUATION_TABLE
        .iter()
        .find(|(t, _)| *t == tenths)
        .map(|(_, raw)| *raw)
}

/// [EXPERIMENTAL] Apex Pro TKL (`0x1614`) live actuation frame, 643-byte feature report:
/// `[0x00][0x31][0x47][hid raw_lo raw_hi] x 68`.
///
/// Reference: apex-control `BuildFrame` (`Actuation.cs`), tested by its author on the wired
/// Apex Pro TKL (model 64734), firmware 4.16.8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActuationLiveGen1Command {
    /// `(HID usage, raw value)` pairs in wire order.
    pub entries: Vec<(u8, u16)>,
}

impl ActuationLiveGen1Command {
    /// One captured depth for every addressed key, in 0.1 mm.
    pub fn global(tenths: u8) -> Result<Self> {
        let raw = apex_gen1_actuation_raw(tenths).ok_or_else(|| {
            Error::InvalidConfig(format!(
                "actuation {tenths} (0.1 mm) has no captured value; use 1-31, 33-36 or 40"
            ))
        })?;
        Ok(Self {
            entries: APEX_PRO_ACTUATION_KEYS
                .iter()
                .map(|&hid| {
                    let value = if APEX_GEN1_ACTUATION_SENTINEL_KEYS.contains(&hid) {
                        APEX_GEN1_ACTUATION_SENTINEL
                    } else {
                        raw
                    };
                    (hid, value)
                })
                .collect(),
        })
    }
}

impl HidCommand for ActuationLiveGen1Command {
    fn command_code(&self) -> CommandCode {
        CommandCode::ActuationLiveGen1
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        self.validate()?;
        prepare_buffer(buffer, APEX_GEN1_ACTUATION_REPORT_SIZE)?;
        buffer[1] = CommandCode::ActuationLiveGen1 as u8;
        buffer[2] = 0x47;
        for (i, (hid, raw)) in self.entries.iter().enumerate() {
            let offset = 3 + i * 3;
            buffer[offset] = *hid;
            buffer[offset + 1..offset + 3].copy_from_slice(&raw.to_le_bytes());
        }
        Ok(APEX_GEN1_ACTUATION_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        if self.entries.is_empty() || 3 + self.entries.len() * 3 > APEX_GEN1_ACTUATION_REPORT_SIZE {
            return Err(Error::DeviceCommunication(format!(
                "Live actuation frame cannot hold {} keys",
                self.entries.len()
            )));
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Apex Gen 1 live actuation for {} keys", self.entries.len())
    }
}

/// [EXPERIMENTAL] Apex Pro TKL (`0x1614`) live Rapid Tap (SOCD) switch: output report
/// `[0x00][0x1A][0x00 | 0x01]`.
///
/// Reference: apex-control `PROTOCOL_NOTES.md`, tested on firmware 4.16.8. Which key pairs Rapid
/// Tap applies to lives in the stored profile, which this crate does not write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RapidTapCommand {
    /// On or off.
    pub enabled: bool,
}

impl HidCommand for RapidTapCommand {
    fn command_code(&self) -> CommandCode {
        CommandCode::RapidTap
    }

    fn serialize(&self, buffer: &mut [u8], _device_type: HidDeviceType) -> Result<usize> {
        prepare_buffer(buffer, APEX_OUTPUT_REPORT_SIZE)?;
        buffer[1] = CommandCode::RapidTap as u8;
        buffer[2] = u8::from(self.enabled);
        Ok(APEX_OUTPUT_REPORT_SIZE)
    }

    fn validate(&self) -> Result<()> {
        Ok(())
    }

    fn description(&self) -> String {
        format!("Rapid Tap {}", if self.enabled { "on" } else { "off" })
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

        let has_standard_header = command.has_standard_header();
        let size = command.serialize(buffer, self.device_type)?;
        let data = &buffer[..size];
        if has_standard_header {
            self.validate_report_size(data, size)?;
        }

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

    #[cfg(feature = "experimental-apex-2023")]
    #[test]
    fn test_apex_2023_direct_command_serialization() {
        let mut cmd = Apex2023DirectCommand::new();
        cmd.set_key_color(0x04, Color::RED);
        cmd.set_key_color(0x52, Color::BLUE);

        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);
        let mut buffer = [0u8; APEX_2023_PERKEY_REPORT_SIZE];
        let size = builder.build_report(cmd, &mut buffer).unwrap();

        // Confirmed 645-byte format from live IOCTL_HID_SET_FEATURE capture.
        assert_eq!(size, APEX_2023_PERKEY_REPORT_SIZE);
        assert_eq!(buffer[0], 0x00); // report ID
        assert_eq!(buffer[1], 0x40); // command byte
        assert_eq!(buffer[2], 0x54); // fixed count = 84
        // first key entry at offset 3
        assert_eq!(buffer[3], 0x04); // key_id A
        assert_eq!(buffer[4], 255); // R
        assert_eq!(buffer[5], 0); // G
        assert_eq!(buffer[6], 0); // B
        // second key entry at offset 7
        assert_eq!(buffer[7], 0x52); // key_id Up
        assert_eq!(buffer[8], 0); // R
        assert_eq!(buffer[9], 0); // G
        assert_eq!(buffer[10], 255); // B
        // remainder is zero-padded
        assert!(buffer[11..].iter().all(|&b| b == 0));
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

    // === OpenRGB-referenced Apex families ===
    // Expected bytes follow the layouts in OpenRGB Controllers/SteelSeriesController/ (file named
    // per test); offsets include the leading report ID byte.

    fn build<C: HidCommand>(command: C, size: usize) -> Vec<u8> {
        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);
        let mut buffer = vec![0u8; size];
        let written = builder.build_report(command, &mut buffer).unwrap();
        buffer.truncate(written);
        buffer
    }

    /// SteelSeriesApexController.cpp `SetLEDsDirect`: `buf[1]` packet ID, `buf[2]` key count,
    /// then `[hid R G B]` from `buf[3]`, `APEX_PACKET_LENGTH` = 643 bytes.
    #[test]
    fn apex_direct_frame_layout_per_generation() {
        for (packet, byte) in [
            (ApexDirectPacket::Gen1, 0x3A),
            (ApexDirectPacket::Wired2023, 0x40),
            (ApexDirectPacket::Wireless2023, 0x61),
        ] {
            let mut command = ApexDirectCommand::new(packet, APEX_DIRECT_REPORT_SIZE);
            command.push(0x04, Color::new(1, 2, 3));
            command.push(0xFB, Color::new(4, 5, 6));
            let report = build(command, APEX_DIRECT_REPORT_SIZE);
            assert_eq!(report.len(), 643);
            assert_eq!(report[..11], [0x00, byte, 2, 0x04, 1, 2, 3, 0xFB, 4, 5, 6]);
            assert!(report[11..].iter().all(|&b| b == 0));
        }
    }

    /// SteelSeriesApexController.cpp `direct_packet_length_map`: Apex 9 frames are 513 bytes and
    /// the key count is clamped so `count * 4 + 3` fits.
    #[test]
    fn apex_9_frame_is_513_bytes_and_bounded() {
        let mut command = ApexDirectCommand::new(ApexDirectPacket::Wired2023, APEX_9_DIRECT_REPORT_SIZE);
        assert_eq!(command.capacity(), 127);
        for hid in 0..112u8 {
            command.push(hid, Color::WHITE);
        }
        assert_eq!(build(command.clone(), APEX_9_DIRECT_REPORT_SIZE).len(), 513);
        for hid in 0..16u8 {
            command.push(hid, Color::WHITE);
        }
        assert!(command.validate().is_err(), "128 keys do not fit in 513 bytes");
        assert!(ApexDirectCommand::new(ApexDirectPacket::Gen1, 643).validate().is_err());
    }

    /// SteelSeriesApexController.cpp `SendInitialization`: `[0x00][0x4B]` in a 65-byte feature report.
    #[test]
    fn apex_gen3_init_layout() {
        let report = build(ApexInitCommand::new(APEX_OUTPUT_REPORT_SIZE), APEX_OUTPUT_REPORT_SIZE);
        assert_eq!(report.len(), 65);
        assert_eq!(report[..2], [0x00, 0x4B]);
        assert!(report[2..].iter().all(|&b| b == 0));
    }

    /// SteelSeriesApexController.cpp `SetBrightness` (0..=10) and SteelSeriesApex8ZoneController.cpp
    /// `SetBrightness` (0..=0x10): `[0x00][0x23][level]`, 65 bytes.
    #[test]
    fn illumination_brightness_layout_and_bounds() {
        let report = build(IlluminationBrightnessCommand::new(7, APEX_ILLUMINATION_MAX), 65);
        assert_eq!(report.len(), 65);
        assert_eq!(report[..3], [0x00, 0x23, 7]);
        assert!(report[3..].iter().all(|&b| b == 0));
        let report = build(IlluminationBrightnessCommand::new(0x10, EIGHT_ZONE_BRIGHTNESS_MAX), 65);
        assert_eq!(report[..3], [0x00, 0x23, 0x10]);
        assert!(
            IlluminationBrightnessCommand::new(11, APEX_ILLUMINATION_MAX)
                .validate()
                .is_err()
        );
        assert!(
            IlluminationBrightnessCommand::new(0x11, EIGHT_ZONE_BRIGHTNESS_MAX)
                .validate()
                .is_err()
        );
    }

    /// SteelSeriesApexController.cpp `ReadBrightness`: request `[0x00][0xA3]`, answer
    /// `[0xA3][0x00][level]`; status `0xFF` or level > 10 means unsupported.
    #[test]
    fn brightness_query_and_reply() {
        assert_eq!(build(ApexQuery::Brightness, 65)[..2], [0x00, 0xA3]);
        assert_eq!(build(ApexQuery::FirmwareVersion, 65)[..2], [0x00, 0x90]);
        assert_eq!(parse_illumination_reply(&[0xA3, 0x00, 7, 0, 0]), Some(7));
        assert_eq!(parse_illumination_reply(&[0xA3, 0x00, 10]), Some(10));
        assert_eq!(parse_illumination_reply(&[0xA3, 0xFF, 7]), None);
        assert_eq!(parse_illumination_reply(&[0xA3, 0x00, 11]), None);
        assert_eq!(parse_illumination_reply(&[0xA3, 0x00]), None);
        assert_eq!(parse_illumination_reply(&[0x90, 0x00, 1]), None);
    }

    /// SteelSeriesApexBaseController.cpp `GetVersion` / `ExtractVersion` and
    /// SteelSeriesApexController.cpp `SendInitialization` (Gen 3 from 1.19.7).
    #[test]
    fn firmware_version_selects_gen3_from_1_19_7() {
        let mut reply = vec![0x90];
        reply.extend_from_slice(b"1.19.7");
        reply.resize(64, 0);
        assert_eq!(parse_apex_firmware_version(&reply), Some((1, 19, 7)));
        assert_eq!(parse_apex_firmware_version(b"2.0.11 build"), Some((2, 0, 11)));
        assert_eq!(parse_apex_firmware_version(b"v1.19.7"), None);
        assert_eq!(parse_apex_firmware_version(b"1.19"), None);
        assert_eq!(parse_apex_firmware_version(&[]), None);

        assert!(firmware_selects_gen3((1, 19, 7)));
        assert!(firmware_selects_gen3((1, 20, 0)));
        assert!(firmware_selects_gen3((2, 0, 0)));
        assert!(!firmware_selects_gen3((1, 19, 6)));
        assert!(!firmware_selects_gen3((1, 18, 99)));
        assert!(!firmware_selects_gen3((0, 99, 99)));
    }

    /// SteelSeriesApex8ZoneController.cpp `SetColor`: `[0x00][0x21][0xFF][R G B] x 8`, 65 bytes,
    /// which is what `RgbZoneCommand` builds for 8 colours.
    #[test]
    fn eight_zone_colour_layout() {
        let colors: Vec<Color> = (0..EIGHT_ZONE_COUNT as u8)
            .map(|i| Color::new(i, i + 10, i + 20))
            .collect();
        let report = build(RgbZoneCommand::new_all_zones(&colors), 65);
        assert_eq!(report.len(), 65);
        assert_eq!(report[..3], [0x00, 0x21, 0xFF]);
        for (i, color) in colors.iter().enumerate() {
            assert_eq!(report[3 + i * 3..6 + i * 3], [color.r, color.g, color.b]);
        }
        assert!(report[27..].iter().all(|&b| b == 0));
    }

    /// SteelSeriesApexTZoneController.cpp: `STEELSERIES_TZ_WRITE_PACKET_SIZE` = 33; brightness
    /// `[0x00][0x0A][0x00][level]`, colours `[0x00][0x0B][0x00][R G B] x 10`, save
    /// `[0x00][0x06][0x00][0x08]` then `[0x00][0x09][0x00][0x00]`.
    #[test]
    fn tri_zone_layouts() {
        let report = build(TriZoneBrightnessCommand { level: 0x64 }, 33);
        assert_eq!(report.len(), 33);
        assert_eq!(report[..4], [0x00, 0x0A, 0x00, 0x64]);
        assert!(report[4..].iter().all(|&b| b == 0));
        assert!(TriZoneBrightnessCommand { level: 101 }.validate().is_err());

        let colors: Vec<Color> = (0..TRI_ZONE_COUNT as u8).map(|i| Color::new(i, 2 * i, 3 * i)).collect();
        let report = build(TriZoneColorCommand { colors: colors.clone() }, 33);
        assert_eq!(report[..3], [0x00, 0x0B, 0x00]);
        for (i, color) in colors.iter().enumerate() {
            assert_eq!(report[3 + i * 3..6 + i * 3], [color.r, color.g, color.b]);
        }
        assert!(
            TriZoneColorCommand {
                colors: vec![Color::RED; 11]
            }
            .validate()
            .is_err()
        );

        let prepare = build(
            TriZoneSaveCommand {
                step: TriZoneSaveStep::Prepare,
            },
            33,
        );
        let commit = build(
            TriZoneSaveCommand {
                step: TriZoneSaveStep::Commit,
            },
            33,
        );
        assert_eq!(prepare[..4], [0x00, 0x06, 0x00, 0x08]);
        assert_eq!(commit[..4], [0x00, 0x09, 0x00, 0x00]);
        assert!(prepare[4..].iter().chain(commit[4..].iter()).all(|&b| b == 0));
    }

    /// SteelSeriesOldApexController.cpp `SetColorDetailed`: `[0x00][0x07][0x00][R G B A] x 5`, 33 bytes.
    #[test]
    fn old_apex_colour_layout() {
        let zones = vec![Color::RED, Color::GREEN, Color::BLUE];
        let report = build(OldApexColorCommand { zones, brightness: 8 }, 33);
        assert_eq!(report.len(), 33);
        assert_eq!(
            report[..23],
            [
                0x00, 0x07, 0x00, 255, 0, 0, 8, 0, 255, 0, 8, 0, 0, 255, 8, 0, 0, 0, 8, 0, 0, 0, 8
            ]
        );
        assert!(report[23..].iter().all(|&b| b == 0));
        assert!(
            OldApexColorCommand {
                zones: vec![],
                brightness: 0
            }
            .validate()
            .is_err()
        );
        assert!(
            OldApexColorCommand {
                zones: vec![Color::RED; 6],
                brightness: 8
            }
            .validate()
            .is_err()
        );
    }

    /// apexctl `cmd_poll`: feature report `[0x04][0x00][0..=3]`.
    #[test]
    fn old_apex_polling_rate_layout() {
        for (rate, index) in OldApexPollingRate::ALL.into_iter().zip(0u8..) {
            let report = build(
                OldApexPollingRateCommand { rate },
                OldApexPollingRateCommand::REPORT_SIZE,
            );
            assert_eq!(report, [0x04, 0x00, index]);
            assert_eq!(OldApexPollingRate::from_hz(rate.hz()), Some(rate));
        }
        assert_eq!(OldApexPollingRate::from_hz(300), None);
    }

    /// SteelSeriesApexMController.cpp `EnableLEDControl` and `SetLEDsDirect` (513 bytes).
    #[test]
    fn apex_m750_layouts() {
        let enable: Vec<Vec<u8>> = ApexMEnableStep::ALL
            .iter()
            .map(|&step| build(ApexMEnableCommand { step }, APEX_M_REPORT_SIZE))
            .collect();
        assert_eq!(enable[0][..8], [0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x85, 0x00]);
        assert_eq!(enable[1][..8], [0x00, 0x00, 0x00, 0x00, 0x03, 0x01, 0x00, 0xFF]);
        // The buffer is not cleared between the reports, so 0xFF survives at byte 7.
        assert_eq!(enable[2][..8], [0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x85, 0xFF]);
        assert!(enable.iter().all(|r| r.len() == 513 && r[8..].iter().all(|&b| b == 0)));

        let mut slots = vec![None; APEX_M_SLOT_COUNT];
        slots[0] = Some(Color::new(9, 8, 7));
        let report = build(ApexMDirectCommand { slots: slots.clone() }, APEX_M_REPORT_SIZE);
        assert_eq!(report.len(), 513);
        assert_eq!(report[..9], [0x00, 0x00, 0x00, 0x01, 0x8E, 0x01, 0x03, 0x06, 0x16]);
        assert_eq!(report[9..15], [9, 8, 7, 0xFF, 0x32, 0x00]);
        assert_eq!(report[9 + 131 * 3..9 + 132 * 3], [0xFF, 0x32, 0x00]);
        assert!(report[9 + 132 * 3..].iter().all(|&b| b == 0));
        slots.pop();
        assert!(ApexMDirectCommand { slots }.validate().is_err());
    }

    /// apex-web `bloc0x38` (index.html) with `KEY_ORDER` (capture-data.js) and the
    /// `mmVersOctet` / `relachement` curve.
    #[test]
    fn apex_2023_live_actuation_layout() {
        assert_eq!(apex_2023_actuation_raw(1), 5);
        assert_eq!(apex_2023_actuation_raw(10), 21);
        assert_eq!(apex_2023_actuation_raw(20), 57);
        assert_eq!(apex_2023_actuation_raw(22), 68);
        assert_eq!(apex_2023_actuation_raw(40), 224);
        assert_eq!(apex_2023_release_raw(1), 5);
        assert_eq!(apex_2023_release_raw(20), 53);

        let report = build(
            ActuationLive2023Command::global(20).unwrap(),
            APEX_2023_ACTUATION_REPORT_SIZE,
        );
        assert_eq!(report.len(), 645);
        assert_eq!(report[..5], [0x00, 0x38, 0x61, 68, 0x00]);
        for (i, &hid) in APEX_PRO_ACTUATION_KEYS.iter().enumerate() {
            assert_eq!(report[5 + i * 3..8 + i * 3], [hid, 57, 53]);
        }
        assert!(report[5 + 68 * 3..].iter().all(|&b| b == 0));
        assert!(ActuationLive2023Command::global(0).is_err());
        assert!(ActuationLive2023Command::global(41).is_err());
    }

    /// apex-control `BuildFrame` / `BuildKeyCodes` / `KnownGoodValues` (Actuation.cs).
    #[test]
    fn apex_gen1_live_actuation_layout() {
        assert_eq!(apex_gen1_actuation_raw(1), Some(0x0606));
        assert_eq!(apex_gen1_actuation_raw(20), Some(0x373B));
        assert_eq!(apex_gen1_actuation_raw(36), Some(0xC0C8));
        assert_eq!(apex_gen1_actuation_raw(40), Some(0xD4D9));
        assert_eq!(apex_gen1_actuation_raw(32), None, "3.2 mm is excluded");
        assert_eq!(apex_gen1_actuation_raw(38), None);

        let report = build(
            ActuationLiveGen1Command::global(20).unwrap(),
            APEX_GEN1_ACTUATION_REPORT_SIZE,
        );
        assert_eq!(report.len(), 643);
        assert_eq!(report[..3], [0x00, 0x31, 0x47]);
        for (i, &hid) in APEX_PRO_ACTUATION_KEYS.iter().enumerate() {
            let expected = if APEX_GEN1_ACTUATION_SENTINEL_KEYS.contains(&hid) {
                [hid, 0x23, 0x1F]
            } else {
                [hid, 0x3B, 0x37]
            };
            assert_eq!(report[3 + i * 3..6 + i * 3], expected);
        }
        assert!(report[3 + 68 * 3..].iter().all(|&b| b == 0));
        assert!(ActuationLiveGen1Command::global(32).is_err());
    }

    #[test]
    fn actuation_key_list_matches_both_tools() {
        let expected: Vec<u8> = (0x04..=0x28)
            .chain(0x2A..=0x39)
            .chain([0x64])
            .chain(0x87..=0x8B)
            .chain(0xE0..=0xE7)
            .chain([0xF0])
            .collect();
        assert_eq!(APEX_PRO_ACTUATION_KEYS.to_vec(), expected);
        let table: Vec<u8> = APEX_GEN1_ACTUATION_TABLE.iter().map(|(t, _)| *t).collect();
        assert!(table.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(table.len(), 36);
    }

    /// apex-control PROTOCOL_NOTES.md: Rapid Tap `1a 00` / `1a 01` as an output report.
    #[test]
    fn rapid_tap_layout() {
        assert_eq!(build(RapidTapCommand { enabled: true }, 65)[..3], [0x00, 0x1A, 0x01]);
        assert_eq!(build(RapidTapCommand { enabled: false }, 65)[..3], [0x00, 0x1A, 0x00]);
    }

    #[test]
    fn new_command_bytes_parse_back() {
        let builder = HidReportBuilder::new(HidDeviceType::Keyboard);
        for (byte, code) in [
            (0x3A, CommandCode::ApexLegacyDirect),
            (0x61, CommandCode::Apex2023DirectWireless),
            (0x4B, CommandCode::Apex2023Init),
            (0xA3, CommandCode::BrightnessQuery),
            (0x0B, CommandCode::TriZoneColor),
            (0x07, CommandCode::OldApexColor),
            (0x38, CommandCode::ActuationLive2023),
            (0x31, CommandCode::ActuationLiveGen1),
            (0x1A, CommandCode::RapidTap),
        ] {
            assert_eq!(builder.parse_command_code(&[0x00, byte]), Some(code));
            assert_eq!(code as u8, byte);
        }
    }
}
