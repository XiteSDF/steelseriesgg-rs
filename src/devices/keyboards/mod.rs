//! Keyboard device support (Apex series).

pub mod apex;
pub mod apex_7;
pub mod apex_pro_tkl_2023;
pub mod oled;
pub mod protocol;

use super::diagnostics::{HidOperation, with_global_diagnostics};
use super::hid_reports::{
    ACTUATION_TENTHS_RANGE, APEX_2023_ACTUATION_REPORT_SIZE, APEX_GEN1_ACTUATION_REPORT_SIZE,
    APEX_GEN1_ACTUATION_TABLE, APEX_ILLUMINATION_MAX, APEX_M_REPORT_SIZE, APEX_OUTPUT_REPORT_SIZE,
    APEX_SHORT_REPORT_SIZE, ActuationCommand, ActuationLive2023Command, ActuationLiveGen1Command, ApexInitCommand,
    ApexMEnableCommand, ApexMEnableStep, ApexQuery, ApplyCommand, BrightnessCommand, EIGHT_ZONE_BRIGHTNESS_MAX,
    HidCommand, HidDeviceType, HidReportBuilder, IlluminationBrightnessCommand, KEYBOARD_REPORT_SIZE,
    OLD_APEX_BRIGHTNESS_MAX, OLD_APEX_BRIGHTNESS_MIN, OldApexColorCommand, OldApexPollingRate,
    OldApexPollingRateCommand, PerKeyRgbBuilder, PerKeyRgbCommand, RapidTapCommand, RgbZoneCommand,
    TRI_ZONE_BRIGHTNESS_MAX, TriZoneBrightnessCommand, TriZoneColorCommand, TriZoneSaveCommand, TriZoneSaveStep,
    firmware_selects_gen3, parse_apex_firmware_version, parse_illumination_reply,
};
use super::key_mapping::{KeyAddress, KeyId, KeyMapping, KeyMappingDatabase};
use super::settings::{
    ChoiceOption, Configurable, SettingDescriptor, SettingKind, SettingValue, Verification, validate_against,
};
use super::zone_mapping::{ZoneEffect, ZoneFallback, ZoneMapping as ZoneMap};
use super::{Device, DeviceInfo, DeviceType, write_padded_report, zone_count_for_product_id};
use crate::input::BindingSet;
use crate::rgb::{Color, PerKeyEffect, PerKeyRgbController};
use crate::{Error, Result};
use async_trait::async_trait;
use hidapi::HidDevice;
use parking_lot::Mutex;
use protocol::{
    ApexFrame, ApexGeneration, FeatureTarget, KeyboardFamily, KeyboardProfile, LiveActuation, keyboard_profile,
};
use std::sync::Arc;

/// Stable ids of the keyboard settings exposed through [`Configurable`].
pub mod setting_ids {
    /// Backlight brightness, in the keyboard's native range.
    pub const BRIGHTNESS: &str = "brightness";
    /// Global actuation through the experimental `0x2D` command, in 0.1 mm.
    pub const ACTUATION: &str = "actuation";
    /// Global actuation through a model's live actuation frame, in 0.1 mm.
    pub const ACTUATION_LIVE: &str = "actuation_live";
    /// Live Rapid Tap switch.
    pub const RAPID_TAP: &str = "rapid_tap";
    /// Keyboard-side USB polling rate, in Hz.
    pub const POLLING_RATE: &str = "polling_rate";
    /// Store the current lighting on the keyboard.
    pub const SAVE_TO_DEVICE: &str = "save_to_device";
}

/// What reading one onboard slot of a keyboard returns: the name the device spells for it, and the
/// bindings its payload carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardConfig {
    pub name: Option<String>,
    pub bindings: BindingSet,
}

/// Trait for keyboard-specific functionality.
#[async_trait]
pub trait Keyboard: Device + Configurable {
    /// Set the entire keyboard to a single color.
    async fn set_color(&mut self, _color: Color) -> Result<()> {
        Err(Error::DeviceCommunication(
            "set_color is not supported for this keyboard".to_string(),
        ))
    }

    /// Set colors for individual zones.
    async fn set_zone_colors(&mut self, _colors: &[Color]) -> Result<()> {
        Ok(())
    }

    /// Get the number of RGB zones.
    fn zone_count(&self) -> usize {
        0
    }

    /// Set keyboard brightness (0-100).
    async fn set_brightness(&mut self, _brightness: u8) -> Result<()> {
        Ok(())
    }

    /// Apply the current RGB settings.
    async fn apply(&mut self) -> Result<()> {
        Ok(())
    }

    /// Write a whole profile's keybindings into the keyboard's own memory.
    ///
    /// Only one model is covered here: the Apex 7 (2023) keeps a whole profile, bindings
    /// included, in an onboard FS object the engine rewrites (`src/devices/keyboards/apex_7.rs`). Other
    /// Apex keyboards have onboard profiles too - the
    /// engine's own descriptors give the legacy family the same `profile_1`..`profile_5`,
    /// `onboard_config_count` and `profile_size` fields (`base_apex_legacy.lisp:1553`, `:1609`) -
    /// but their payload has never been decoded from a capture, so this fork cannot write one
    /// for them, and they refuse and say so: their bindings only ever live in the engine's
    /// `InputState`, which imposes them by grabbing the device and replaying it through uinput
    /// (`src/input/linux.rs`).
    fn set_key_bindings(&mut self, bindings: &BindingSet) -> Result<()> {
        let _ = bindings;
        Err(
            Error::DeviceCommunication(
                "this model has no onboard payload decoded in this fork, so a keybinding cannot become one of its own: the engine's InputState imposes it by grabbing the device and replaying it through uinput".to_string()
            )
        )
    }

    /// Read back what the keyboard's own memory holds for the slot a keybinding rewrite uses: the
    /// configuration's name as the device spells it, and the bindings it carries, so a configuration
    /// the device already has can become one of the engine's profiles.
    ///
    /// The same one model is covered as for [`Self::Keyboard::set_key_bindings`]: reading that memory
    /// means staging each window of the device's own buffer and asking for it (`macho-pull-payload`
    /// and `macho-read-payload` in the engine's device code, spelled at
    /// [`crate::devices::keyboards::apex_7::Apex7::fs_read_frames`]), and the payload of any other
    /// Apex keyboard has never been decoded here, so it refuses and says so.
    fn pull_key_bindings(&mut self) -> Result<OnboardConfig> {
        Err(
            Error::DeviceCommunication(
                "this model has no onboard payload decoded in this fork, so nothing can be read back from its own memory: the engine's InputState is the only place its bindings live".to_string(),
            )
        )
    }

    // === Per-Key RGB Control ===

    /// Check if per-key RGB key-mapping is available for this keyboard.
    ///
    /// ⚠️ **Experimental**: returning `true` means a HID key mapping exists, but the
    /// per-key protocol (`CommandCode::PerKeyRgb`, `0x23`) has not been confirmed on
    /// hardware. Calls to `set_key_color` / `set_key_colors` may be silently ignored by
    /// the device until the packet format is verified via USB capture.
    fn supports_per_key_rgb(&self) -> bool {
        false
    }

    /// Get the key mapping for this keyboard (if available).
    fn get_key_mapping(&self) -> Option<&KeyMapping> {
        None
    }

    /// Set RGB color for a specific key by logical key ID.
    async fn set_key_color(&mut self, _key_id: KeyId, _color: Color) -> Result<()> {
        Err(Error::DeviceCommunication("Per-key RGB not supported".to_string()))
    }

    /// Set RGB colors for multiple keys by logical key IDs.
    async fn set_key_colors(&mut self, _key_colors: &[(KeyId, Color)]) -> Result<()> {
        Ok(())
    }

    /// Set RGB color for a specific key by direct HID code address.
    async fn set_key_color_direct(&mut self, _address: KeyAddress, _color: Color) -> Result<()> {
        Ok(())
    }

    /// Set RGB colors for multiple keys by direct HID code addresses.
    async fn set_key_colors_direct(&mut self, _key_colors: &[(KeyAddress, Color)]) -> Result<()> {
        Ok(())
    }

    /// Set all keys to black (turn off per-key RGB).
    async fn clear_per_key_rgb(&mut self) -> Result<()> {
        Ok(())
    }

    /// Set a region of keys to the same color using HID codes.
    async fn set_key_region(&mut self, _start_hid: u8, _count: u8, _color: Color) -> Result<()> {
        Ok(())
    }

    // === Zone-based RGB Fallback ===

    /// Get zone mapping information for this keyboard.
    fn get_zone_mapping(&self) -> Option<&ZoneMap> {
        None
    }

    /// Set zone-based RGB effect as fallback.
    async fn set_zone_effect(&mut self, _effect: ZoneEffect) -> Result<()> {
        Ok(())
    }

    /// Simulate per-key effect using zone-based fallback.
    async fn simulate_per_key_with_zones(&mut self, _key_colors: &[(KeyId, Color)]) -> Result<()> {
        Ok(())
    }

    /// Enhanced zone-based RGB with retry logic.
    async fn set_zone_colors_with_retry(&mut self, _colors: &[Color], _max_retries: usize) -> Result<()> {
        Ok(())
    }

    /// Test zone connectivity and reliability.
    async fn test_zone_reliability(&mut self) -> Result<Vec<bool>> {
        Err(Error::from(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "test_zone_reliability is not supported for this keyboard",
        )))
    }

    // === Per-Key RGB Effect Support ===

    /// Check if per-key RGB effects are supported.
    fn supports_per_key_effects(&self) -> bool {
        false
    }

    /// Set per-key RGB effect.
    async fn set_per_key_effect(&mut self, _effect: PerKeyEffect) -> Result<()> {
        Ok(())
    }

    /// Get current per-key RGB effect (if available).
    fn get_per_key_effect(&self) -> Option<&PerKeyEffect> {
        None
    }

    /// Trigger reactive effect for specific keys.
    async fn trigger_key_reactive(&mut self, _keys: &[KeyId], _duration: f32) -> Result<()> {
        Ok(())
    }

    /// Apply per-key effect with brightness control.
    async fn apply_per_key_effect_with_brightness(&mut self, _brightness: f32) -> Result<()> {
        Ok(())
    }

    /// Convert per-key effect to zone-based fallback.
    async fn convert_per_key_to_zones(&mut self, _effect: &PerKeyEffect) -> Result<()> {
        Ok(())
    }

    // === Performance Optimization ===

    /// Get current performance statistics for RGB operations.
    fn get_rgb_performance_stats(&self) -> Option<&crate::performance::PerformanceStats> {
        None
    }

    /// Get optimal frame time for current adaptive refresh rate.
    fn get_optimal_frame_time(&self) -> Option<std::time::Duration> {
        None
    }

    /// Force cleanup of performance caches.
    fn cleanup_rgb_caches(&mut self) {}

    /// Enable/disable performance optimizations.
    fn set_performance_optimization(&mut self, _enabled: bool) {}

    /// Read current actuation point setting from keyboard (if supported).
    ///
    /// **NOTE**: This is currently a placeholder. The HID command to read
    /// actuation settings has not yet been discovered. This function always
    /// returns an error indicating the feature is not implemented.
    ///
    /// Returns actuation point in 0.1mm units (e.g., 4 = 0.4mm, 36 = 3.6mm).
    fn read_actuation_point(&mut self) -> Result<u8> {
        Err(Error::DeviceCommunication(
            "Reading actuation point not implemented".to_string(),
        ))
    }

    /// Set actuation point for adjustable actuation keyboards (if supported).
    ///
    /// Value is in 0.1mm increments (e.g. 4 = 0.4mm, 36 = 3.6mm).
    /// Returns an error if the keyboard doesn't support adjustable actuation.
    fn set_actuation_point(&mut self, _value: u8) -> Result<()> {
        Err(Error::DeviceCommunication(
            "Setting actuation point not supported".to_string(),
        ))
    }

    /// Set actuation point for adjustable actuation keyboards in millimeters (if supported).
    ///
    /// Value is in millimeters with precision limited to 0.1mm increments.
    /// Returns an error if the keyboard doesn't support adjustable actuation.
    fn set_actuation_point_mm(&mut self, _mm: f32) -> Result<()> {
        Err(Error::DeviceCommunication(
            "Setting actuation point not supported".to_string(),
        ))
    }

    // === OLED screen ===

    /// `(width, height)` in pixels of this keyboard's OLED screen, or `None` when it has none.
    ///
    /// How far the screen's protocol can be trusted differs per model; see
    /// [`oled::oled_verification_for_product_id`].
    fn oled_size(&self) -> Option<(u32, u32)> {
        None
    }

    /// Show `frame` on the OLED screen. `[EXPERIMENTAL]`, see [`oled`].
    ///
    /// The frame must match [`oled_size`](Self::oled_size). It stays on screen until replaced:
    /// no reference documents a command that hands the screen back to the keyboard's own UI.
    async fn draw_oled(&mut self, _frame: &crate::oled::OledFrame) -> Result<()> {
        Err(Error::Unsupported(format!(
            "OLED drawing is not supported for this keyboard (PID {:#06x})",
            self.info().product_id
        )))
    }

    /// Show one complete lighting frame: every listed key takes its colour and every other key
    /// turns off. Used by the daemon's animation loop.
    ///
    /// Per-key families send the whole frame as one report. The default falls back to
    /// [`Keyboard::set_key_colors`], which leaves unlisted keys unchanged.
    async fn set_all_key_colors(&mut self, colors: &[(KeyId, Color)]) -> Result<()> {
        self.set_key_colors(colors).await
    }
}

/// Generic SteelSeries keyboard implementation.
pub struct GenericKeyboard {
    info: DeviceInfo,
    device: Option<Arc<Mutex<HidDevice>>>,
    zone_count: usize,
    report_builder: HidReportBuilder,
    key_mapping: Option<KeyMapping>,
    zone_fallback: ZoneFallback,
    zone_mapping: Option<ZoneMap>,
    per_key_controller: Option<PerKeyRgbController>,
    zone_color_buffer: Vec<Color>,
    actuation_point_cache: Option<u8>,
    /// PID row of [`protocol::keyboard_profile`].
    profile: KeyboardProfile,
    /// Lighting dialect in use; starts as `profile.family` and moves to Gen 3 when the firmware
    /// check asks for it.
    family: KeyboardFamily,
    /// Whole-keyboard colour state of the per-key families.
    apex_frame: ApexFrame,
    /// Gen 3 initialisation or Apex M750 enable sequence already sent.
    family_ready: bool,
    /// Per-key family: brightness read back from the keyboard (`None`: no answer, so brightness
    /// is not offered, as in OpenRGB).
    illumination_level: Option<u8>,
    /// Zone families: brightness level chosen by the user, in the native range.
    brightness_target: Option<u8>,
    /// Last brightness level written to the keyboard.
    brightness_sent: Option<u8>,
}

impl GenericKeyboard {
    /// Create a new keyboard instance.
    pub fn new(info: DeviceInfo, device: HidDevice) -> Self {
        let zone_count = zone_count_for_product_id(info.product_id);

        let key_mapping = KeyMappingDatabase::new().get_mapping(info.product_id).cloned();

        if key_mapping.is_some() {
            tracing::debug!("Loaded key mapping for product ID 0x{:04x}", info.product_id);
        } else {
            tracing::warn!(
                "No key mapping available for product ID 0x{:04x} - per-key RGB disabled",
                info.product_id
            );
        }

        let zone_fallback = ZoneFallback::new();
        let zone_mapping = zone_fallback.get_mapping(info.product_id).cloned();

        if zone_mapping.is_some() {
            tracing::debug!("Loaded zone mapping for product ID 0x{:04x}", info.product_id);
        } else {
            tracing::warn!(
                "No zone mapping available for product ID 0x{:04x} - using basic zone fallback",
                info.product_id
            );
        }

        let per_key_controller = key_mapping.as_ref().map(|mapping| {
            tracing::debug!(
                "Initializing per-key RGB controller for product ID 0x{:04x}",
                info.product_id
            );
            PerKeyRgbController::new_with_performance(mapping.clone())
        });

        let profile = keyboard_profile(info.product_id);
        tracing::debug!(
            "Keyboard 0x{:04x} uses the {} lighting family",
            info.product_id,
            profile.family.name()
        );

        Self {
            info,
            device: Some(Arc::new(Mutex::new(device))),
            zone_count,
            report_builder: HidReportBuilder::new(HidDeviceType::Keyboard),
            key_mapping,
            zone_fallback,
            zone_mapping,
            per_key_controller,
            zone_color_buffer: Vec::with_capacity(zone_count),
            actuation_point_cache: None,
            profile,
            family: profile.family,
            apex_frame: ApexFrame::new(),
            family_ready: false,
            illumination_level: None,
            brightness_target: None,
            brightness_sent: None,
        }
    }

    /// Create without a HID device handle (for wireless raw-only mode).
    pub fn new_without_device(info: DeviceInfo) -> Self {
        let zone_count = zone_count_for_product_id(info.product_id);
        let key_mapping = KeyMappingDatabase::new().get_mapping(info.product_id).cloned();
        let zone_fallback = ZoneFallback::new();
        let zone_mapping = zone_fallback.get_mapping(info.product_id).cloned();
        let per_key_controller = key_mapping
            .as_ref()
            .map(|mapping| PerKeyRgbController::new_with_performance(mapping.clone()));
        let profile = keyboard_profile(info.product_id);

        Self {
            info,
            device: None,
            zone_count,
            report_builder: HidReportBuilder::new(HidDeviceType::Keyboard),
            key_mapping,
            zone_fallback,
            zone_mapping,
            per_key_controller,
            zone_color_buffer: Vec::with_capacity(zone_count),
            actuation_point_cache: None,
            profile,
            family: profile.family,
            apex_frame: ApexFrame::new(),
            family_ready: false,
            illumination_level: None,
            brightness_target: None,
            brightness_sent: None,
        }
    }

    /// PID row this keyboard was built from.
    pub fn profile(&self) -> &KeyboardProfile {
        &self.profile
    }

    /// Lighting dialect in use.
    pub fn family(&self) -> KeyboardFamily {
        self.family
    }

    /// Whole-keyboard colour state of the per-key families.
    pub fn apex_frame(&self) -> &ApexFrame {
        &self.apex_frame
    }

    /// Send a HID report to the keyboard synchronously (blocking).
    fn send_report(&mut self, data: &[u8]) -> Result<()> {
        use tracing::debug;

        with_global_diagnostics(|diag| {
            if !diag.validate_report_structure(data) {
                debug!("HID report validation failed, sending anyway");
            }
        });

        debug!("Sending HID report ({} bytes): {:02x?}", data.len(), data);

        let device = self
            .device
            .as_ref()
            .ok_or(Error::DeviceCommunication("Device not connected".to_string()))?;
        let device = device.lock();

        let result = if let Some(result) = with_global_diagnostics(|diag| {
            diag.record_timed_operation(HidOperation::Send, data, || {
                write_padded_report(&device, data, 65, true)
            })
        }) {
            result
        } else {
            write_padded_report(&device, data, 65, true)
        };

        if result.is_ok() {
            debug!("HID report sent successfully");
        } else if let Err(e) = &result {
            debug!("HID report failed: {:?}", e);
        }

        result
    }

    /// Send a HID report to the keyboard asynchronously (non-blocking).
    ///
    /// Offloads the blocking hidapi write to `spawn_blocking` so the tokio
    /// worker thread is not stalled during HID I/O.
    /// Build and send a HID command report to the keyboard asynchronously.
    async fn send_command_async<C: crate::devices::HidCommand>(&self, command: C) -> Result<()> {
        let mut buffer = [0u8; crate::devices::KEYBOARD_REPORT_SIZE];
        let size = self.report_builder.build_report(command, &mut buffer)?;
        self.send_report_async(&buffer[..size]).await
    }

    async fn send_report_async(&self, data: &[u8]) -> Result<()> {
        use tracing::debug;

        with_global_diagnostics(|diag| {
            if !diag.validate_report_structure(data) {
                debug!("HID report validation failed, sending anyway");
            }
        });

        debug!("Sending HID report ({} bytes): {:02x?}", data.len(), data);

        let device = self
            .device
            .clone()
            .ok_or(Error::DeviceCommunication("Device not connected".to_string()))?;

        let data = data.to_vec();

        let result = tokio::task::spawn_blocking(move || {
            let device = device.lock();

            if let Some(result) = with_global_diagnostics(|diag| {
                diag.record_timed_operation(HidOperation::Send, &data, || {
                    write_padded_report(&device, &data, 65, true)
                })
            }) {
                result
            } else {
                write_padded_report(&device, &data, 65, true)
            }
        })
        .await
        .map_err(|e| {
            if e.is_cancelled() {
                Error::DeviceCommunication(format!("HID write task was cancelled: {}", e))
            } else {
                Error::DeviceCommunication(format!("HID write task failed: {}", e))
            }
        })?;

        match &result {
            Ok(_) => debug!("HID report sent successfully"),
            Err(e) => debug!("HID report failed: {:?}", e),
        }

        result
    }

    /// Send a HID feature report of arbitrary size (for Apex 2023 new protocol).
    /// Uses direct ioctl to bypass hidapi's broken HIDIOCSFEATURE direction bits.
    /// Resolves the correct hidraw path for the control interface (interface 3 for wireless).
    /// Resolves the correct hidraw path for the control interface (interface 3 for wireless).
    #[cfg(unix)]
    pub fn send_feature(&self, data: &[u8], report_len: usize) -> Result<()> {
        let path = super::find_hidraw_for_interface(self.info.vendor_id, self.info.product_id, 3)
            .unwrap_or_else(|| self.info.path.clone());
        super::send_feature_report_raw(&path, data, report_len)
    }

    #[cfg(not(unix))]
    pub fn send_feature(&self, data: &[u8], _report_len: usize) -> Result<()> {
        let device = self
            .device
            .as_ref()
            .ok_or(Error::DeviceCommunication("Device not connected".to_string()))?;
        let device = device.lock();

        device
            .send_feature_report(data)
            .map_err(|e| Error::DeviceCommunication(e.to_string()))
    }

    async fn send_zone_buffer_async(&mut self) -> Result<()> {
        match self.family {
            KeyboardFamily::Legacy => {}
            KeyboardFamily::ApexPerKey(_) | KeyboardFamily::ApexM => {
                let color = self.zone_color_buffer.first().copied().unwrap_or(Color::BLACK);
                self.apex_frame.fill(color);
                return self.send_apex_frame();
            }
            KeyboardFamily::EightZone => return self.send_eight_zone(),
            KeyboardFamily::TriZone => return self.send_tri_zone(),
            KeyboardFamily::OldApex => return self.send_old_apex(),
        }

        // Wireless keyboards (e.g. Apex Pro TKL 2023 Wireless, PID 0x1632) don't
        // support the 0xFF "all zones" selector. Send per-zone commands instead,
        // with zone indices starting at 1.
        if self.info.product_id == super::product_ids::APEX_PRO_TKL_2023_WIRELESS
            || self.info.product_id == super::product_ids::APEX_PRO_TKL_2023_WIRELESS_2
        {
            for i in 0..self.zone_color_buffer.len() {
                let color = self.zone_color_buffer[i];
                let zone_index = (i + 1) as u8; // zones start at 1, not 0
                self.send_command_async(RgbZoneCommand::new_specific_zone(zone_index, color))
                    .await?;
            }
            return Ok(());
        }

        self.send_command_async(RgbZoneCommand::new_all_zones(&self.zone_color_buffer))
            .await
    }

    /// Update the cached actuation point value.
    ///
    /// This should be called by wrapper structs (like ApexProTkl2023) when they
    /// successfully set the actuation point, so that `read_actuation_point` can
    /// return the last known value.
    pub fn update_cached_actuation_point(&mut self, value: u8) {
        self.actuation_point_cache = Some(value);
    }

    // === Transport for the OpenRGB-referenced families [EXPERIMENTAL] ===

    /// Build `command` into a `size`-byte buffer.
    fn build_exact<C: HidCommand>(&self, command: C, size: usize) -> Result<Vec<u8>> {
        let mut buffer = vec![0u8; size];
        let written = self.report_builder.build_report(command, &mut buffer)?;
        buffer.truncate(written);
        Ok(buffer)
    }

    /// Write an output report exactly as built. `report[0]` is the report ID, so the command byte
    /// is the first byte the keyboard receives, as with OpenRGB's `hid_write`. The legacy path
    /// (`send_report`) inserts an extra `0x00` and is kept only for the legacy family.
    fn write_output_exact(&self, report: &[u8]) -> Result<()> {
        let device = self
            .device
            .as_ref()
            .ok_or(Error::DeviceCommunication("Device not connected".to_string()))?;
        let device = device.lock();
        let write = || device.write(report).map(|_| ()).map_err(Error::from);
        with_global_diagnostics(|diag| diag.record_timed_operation(HidOperation::Send, report, write))
            .unwrap_or_else(write)
    }

    /// Write a request and read the answer: one 64-byte input report, 100 ms timeout (OpenRGB
    /// `STEELSERIES_APEX_HID_TIMEOUT`).
    fn query_exact(&self, request: &[u8]) -> Result<Vec<u8>> {
        self.write_output_exact(request)?;
        let device = self
            .device
            .as_ref()
            .ok_or(Error::DeviceCommunication("Device not connected".to_string()))?;
        let mut buffer = [0u8; 64];
        let read = device.lock().read_timeout(&mut buffer, 100)?;
        Ok(buffer[..read].to_vec())
    }

    fn query_command(&self, query: ApexQuery) -> Result<Vec<u8>> {
        let request = self.build_exact(query, APEX_OUTPUT_REPORT_SIZE)?;
        self.query_exact(&request)
    }

    /// Send a feature report to the opened control interface.
    #[cfg(unix)]
    pub fn send_feature_to_control(&self, report: &[u8]) -> Result<()> {
        let path = usize::try_from(self.info.interface_number)
            .ok()
            .and_then(|interface| {
                super::find_hidraw_for_interface(self.info.vendor_id, self.info.product_id, interface)
            })
            .unwrap_or_else(|| self.info.path.clone());
        super::send_feature_report_raw(&path, report, report.len())
    }

    /// Send a feature report to the opened control interface.
    #[cfg(not(unix))]
    pub fn send_feature_to_control(&self, report: &[u8]) -> Result<()> {
        let device = self
            .device
            .as_ref()
            .ok_or(Error::DeviceCommunication("Device not connected".to_string()))?;
        device
            .lock()
            .send_feature_report(report)
            .map_err(|e| Error::DeviceCommunication(e.to_string()))
    }

    fn send_feature_to(&self, target: FeatureTarget, report: &[u8]) -> Result<()> {
        match target {
            FeatureTarget::ControlInterface => self.send_feature_to_control(report),
            FeatureTarget::Interface3 => self.send_feature(report, report.len()),
        }
    }

    /// Send the Gen 3 initialisation or the Apex M750 enable sequence once.
    fn ensure_family_ready(&mut self) -> Result<()> {
        if self.family_ready {
            return Ok(());
        }
        match self.family {
            KeyboardFamily::ApexPerKey(protocol) if protocol.needs_init() => {
                let report =
                    self.build_exact(ApexInitCommand::new(protocol.init_report_len), protocol.init_report_len)?;
                self.send_feature_to(protocol.feature_target, &report)?;
                if protocol.init_settle_ms > 0 {
                    // The controller needs a moment before it honours direct-mode packets.
                    std::thread::sleep(std::time::Duration::from_millis(protocol.init_settle_ms));
                }
                tracing::info!("Sent Apex Gen 3 init (0x4B) to 0x{:04x}", self.info.product_id);
            }
            KeyboardFamily::ApexM => {
                for step in ApexMEnableStep::ALL {
                    let report = self.build_exact(ApexMEnableCommand { step }, APEX_M_REPORT_SIZE)?;
                    self.send_feature_to_control(&report)?;
                }
            }
            _ => {}
        }
        self.family_ready = true;
        Ok(())
    }

    /// Per-key family: read the firmware version (models that switch to Gen 3 on newer
    /// firmware) and the brightness, as OpenRGB does when it opens the keyboard.
    fn probe_apex_per_key(&mut self) {
        let KeyboardFamily::ApexPerKey(mut protocol) = self.family else {
            return;
        };
        if self.device.is_none() {
            return;
        }
        if protocol.firmware_probe {
            match self.query_command(ApexQuery::FirmwareVersion) {
                Ok(reply) => match parse_apex_firmware_version(&reply) {
                    Some(version) if firmware_selects_gen3(version) => {
                        tracing::info!(
                            "Keyboard 0x{:04x} firmware {}.{}.{} uses the Gen 3 protocol",
                            self.info.product_id,
                            version.0,
                            version.1,
                            version.2
                        );
                        protocol.generation = ApexGeneration::Gen3;
                    }
                    Some(version) => tracing::debug!("Keyboard firmware {version:?} keeps its protocol"),
                    None => tracing::debug!("No firmware version in reply {reply:02x?}"),
                },
                Err(e) => tracing::debug!("Firmware version query failed: {e}"),
            }
            self.family = KeyboardFamily::ApexPerKey(protocol);
        }
        self.illumination_level = match self.query_command(ApexQuery::Brightness) {
            Ok(reply) => parse_illumination_reply(&reply),
            Err(e) => {
                tracing::debug!("Brightness read-back failed: {e}");
                None
            }
        };
        self.brightness_sent = self.illumination_level;
    }

    /// Send [`Self::apex_frame`] in the family's direct format.
    fn send_apex_frame(&mut self) -> Result<()> {
        self.ensure_family_ready()?;
        match self.family {
            KeyboardFamily::ApexPerKey(protocol) => {
                let command = self
                    .apex_frame
                    .direct_command(protocol.direct_packet(), protocol.report_len);
                let report = self.build_exact(command, protocol.report_len)?;
                self.send_feature_to(protocol.feature_target, &report)
            }
            KeyboardFamily::ApexM => {
                let report = self.build_exact(self.apex_frame.m750_command(), APEX_M_REPORT_SIZE)?;
                self.send_feature_to_control(&report)
            }
            _ => Err(self.no_per_key_error()),
        }
    }

    /// Apply `key_colors` to the frame (after clearing it when `clear_first`) and send it.
    fn update_key_frame(&mut self, key_colors: &[(KeyId, Color)], clear_first: bool) -> Result<()> {
        let mapping = self.key_mapping.as_ref().ok_or_else(|| {
            Error::DeviceCommunication("Per-key RGB not supported - no key mapping available".to_string())
        })?;
        if clear_first {
            self.apex_frame.fill(Color::BLACK);
        }
        let mut applied = 0usize;
        for (key_id, color) in key_colors {
            match mapping.get_key_address(*key_id) {
                Some(address) if self.apex_frame.set_hid(address.hid_code, *color) => applied += 1,
                _ => tracing::debug!("Key {key_id:?} has no LED on this keyboard"),
            }
        }
        if applied == 0 && !clear_first {
            return Err(Error::DeviceCommunication(
                "No valid keys found in key mapping".to_string(),
            ));
        }
        self.send_apex_frame()
    }

    fn no_per_key_error(&self) -> Error {
        Error::Unsupported(format!(
            "per-key lighting is not available on 0x{:04x}: it has {} lighting zones",
            self.info.product_id, self.zone_count
        ))
    }

    /// 8-zone colours, then brightness whenever it differs from the last level written
    /// (OpenRGB `SteelSeriesApex8ZoneController::SetColor`; its cache starts unset, so the first
    /// colour update also writes brightness, full by default).
    fn send_eight_zone(&mut self) -> Result<()> {
        let report = self.build_exact(
            RgbZoneCommand::new_all_zones(&self.zone_color_buffer),
            APEX_OUTPUT_REPORT_SIZE,
        )?;
        self.write_output_exact(&report)?;
        let level = self.brightness_target.unwrap_or(EIGHT_ZONE_BRIGHTNESS_MAX);
        if self.brightness_sent != Some(level) {
            self.write_illumination(level, EIGHT_ZONE_BRIGHTNESS_MAX)?;
        }
        Ok(())
    }

    /// Tri-zone brightness, then colours (OpenRGB `SteelSeriesApexTZoneController::SetColor`).
    fn send_tri_zone(&mut self) -> Result<()> {
        self.write_tri_zone_brightness(self.brightness_target.unwrap_or(TRI_ZONE_BRIGHTNESS_MAX))?;
        let colors = TriZoneColorCommand {
            colors: self.zone_color_buffer.clone(),
        };
        let report = self.build_exact(colors, APEX_SHORT_REPORT_SIZE)?;
        self.write_output_exact(&report)
    }

    fn write_tri_zone_brightness(&mut self, level: u8) -> Result<()> {
        let report = self.build_exact(TriZoneBrightnessCommand { level }, APEX_SHORT_REPORT_SIZE)?;
        self.write_output_exact(&report)?;
        self.brightness_sent = Some(level);
        Ok(())
    }

    /// Old Apex colours with the zone brightness in every zone.
    fn send_old_apex(&mut self) -> Result<()> {
        let brightness = self.brightness_target.unwrap_or(OLD_APEX_BRIGHTNESS_MAX);
        let command = OldApexColorCommand {
            zones: self.zone_color_buffer.clone(),
            brightness,
        };
        let report = self.build_exact(command, APEX_SHORT_REPORT_SIZE)?;
        self.write_output_exact(&report)?;
        self.brightness_sent = Some(brightness);
        Ok(())
    }

    fn write_illumination(&mut self, level: u8, max: u8) -> Result<()> {
        let report = self.build_exact(IlluminationBrightnessCommand::new(level, max), APEX_OUTPUT_REPORT_SIZE)?;
        self.write_output_exact(&report)?;
        self.brightness_sent = Some(level);
        Ok(())
    }

    /// Native brightness range of the family, `None` when brightness is not offered.
    fn brightness_range(&self) -> Option<(u8, u8)> {
        match self.family {
            KeyboardFamily::Legacy => Some((0, 100)),
            KeyboardFamily::ApexPerKey(_) => self.illumination_level.map(|_| (0, APEX_ILLUMINATION_MAX)),
            KeyboardFamily::EightZone => Some((0, EIGHT_ZONE_BRIGHTNESS_MAX)),
            KeyboardFamily::TriZone => Some((0, TRI_ZONE_BRIGHTNESS_MAX)),
            KeyboardFamily::OldApex => Some((OLD_APEX_BRIGHTNESS_MIN, OLD_APEX_BRIGHTNESS_MAX)),
            KeyboardFamily::ApexM => None,
        }
    }

    fn brightness_unsupported(&self) -> Error {
        Error::Unsupported(format!(
            "brightness is not available on 0x{:04x} ({} family)",
            self.info.product_id,
            self.family.name()
        ))
    }

    /// Write a brightness level in the family's native range (see [`Self::brightness_range`]).
    fn apply_brightness_level(&mut self, level: u8) -> Result<()> {
        let (min, max) = self.brightness_range().ok_or_else(|| self.brightness_unsupported())?;
        if !(min..=max).contains(&level) {
            return Err(Error::InvalidConfig(format!(
                "brightness must be {min}..={max}, got {level}"
            )));
        }
        match self.family {
            KeyboardFamily::Legacy => {
                let mut buffer = [0u8; KEYBOARD_REPORT_SIZE];
                let size = self
                    .report_builder
                    .build_report(BrightnessCommand::new(level), &mut buffer)?;
                self.send_report(&buffer[..size])
            }
            KeyboardFamily::ApexPerKey(_) => {
                self.write_illumination(level, APEX_ILLUMINATION_MAX)?;
                self.illumination_level = Some(level);
                Ok(())
            }
            KeyboardFamily::EightZone => {
                self.brightness_target = Some(level);
                self.write_illumination(level, EIGHT_ZONE_BRIGHTNESS_MAX)
            }
            KeyboardFamily::TriZone => {
                self.brightness_target = Some(level);
                self.write_tri_zone_brightness(level)
            }
            KeyboardFamily::OldApex => {
                // The level travels inside the colour report; without colours yet it is kept
                // for the next colour update.
                self.brightness_target = Some(level);
                if self.zone_color_buffer.is_empty() {
                    Ok(())
                } else {
                    self.send_old_apex()
                }
            }
            KeyboardFamily::ApexM => Err(self.brightness_unsupported()),
        }
    }

    /// Send the model's live actuation frame with one depth (0.1 mm) for every addressed key.
    fn apply_live_actuation(&mut self, tenths: u8) -> Result<()> {
        let report = match self.profile.live_actuation {
            Some(LiveActuation::ApexWeb2023) => self.build_exact(
                ActuationLive2023Command::global(tenths)?,
                APEX_2023_ACTUATION_REPORT_SIZE,
            )?,
            Some(LiveActuation::ApexControlGen1) => self.build_exact(
                ActuationLiveGen1Command::global(tenths)?,
                APEX_GEN1_ACTUATION_REPORT_SIZE,
            )?,
            None => {
                return Err(Error::Unsupported(
                    "live actuation is not available on this keyboard".to_string(),
                ));
            }
        };
        self.send_feature_to_control(&report)
    }

    fn apply_rapid_tap(&mut self, enabled: bool) -> Result<()> {
        if !self.profile.rapid_tap {
            return Err(Error::Unsupported(
                "Rapid Tap is not available on this keyboard".to_string(),
            ));
        }
        let report = self.build_exact(RapidTapCommand { enabled }, APEX_OUTPUT_REPORT_SIZE)?;
        self.write_output_exact(&report)
    }

    fn apply_polling_rate(&mut self, rate: OldApexPollingRate) -> Result<()> {
        if self.family != KeyboardFamily::OldApex {
            return Err(Error::Unsupported(
                "keyboard-side polling rate is not available on this keyboard".to_string(),
            ));
        }
        let report = self.build_exact(
            OldApexPollingRateCommand { rate },
            OldApexPollingRateCommand::REPORT_SIZE,
        )?;
        self.send_feature_to_control(&report)
    }

    fn save_tri_zone(&mut self) -> Result<()> {
        if self.family != KeyboardFamily::TriZone {
            return Err(Error::Unsupported(
                "saving lighting is not available on this keyboard".to_string(),
            ));
        }
        for step in [TriZoneSaveStep::Prepare, TriZoneSaveStep::Commit] {
            let report = self.build_exact(TriZoneSaveCommand { step }, APEX_SHORT_REPORT_SIZE)?;
            self.write_output_exact(&report)?;
        }
        Ok(())
    }

    /// Send the experimental `0x2D` actuation command, the same bytes and path as the Apex Pro
    /// TKL (2023) uses.
    fn send_actuation(&mut self, value: u8) -> Result<()> {
        if !self.profile.actuation {
            return Err(Error::DeviceCommunication(
                "Setting actuation point not supported".to_string(),
            ));
        }
        let command = ActuationCommand::new(value);
        command.validate()?;
        let mut buffer = [0u8; KEYBOARD_REPORT_SIZE];
        let size = self.report_builder.build_report(command, &mut buffer)?;
        self.send_report(&buffer[..size])?;
        self.actuation_point_cache = Some(value);
        Ok(())
    }

    // === Settings ===

    fn brightness_descriptor(&self) -> Option<SettingDescriptor> {
        let (min, max) = self.brightness_range()?;
        let (verification, unit, description) = match self.family {
            KeyboardFamily::Legacy => (
                Verification::Guess,
                Some("%"),
                "Backlight brightness, written with this crate's original 0x22 command. No reference \
                 documents it for this model; it is extrapolated from the Apex Pro TKL (2023).",
            ),
            KeyboardFamily::ApexPerKey(_) => (
                self.profile.lighting,
                None,
                "Illumination level, the setting the keyboard's brightness keys step through (0x23, \
                 OpenRGB SteelSeriesApexController). Offered because the keyboard answered the \
                 brightness read-back request. It applies to on-board lighting.",
            ),
            KeyboardFamily::EightZone => (
                Verification::Reference,
                None,
                "Brightness multiplier, 0 (dark) to 16 (full) (0x23, OpenRGB SteelSeriesApex8ZoneController).",
            ),
            KeyboardFamily::TriZone => (
                Verification::Reference,
                Some("%"),
                "Brightness, written before every colour update (0x0A, OpenRGB SteelSeriesApexTZoneController).",
            ),
            KeyboardFamily::OldApex => (
                Verification::Reference,
                None,
                "Zone brightness: 1 turns the backlight off, 2 is dimmest, 8 brightest (apexctl). It is \
                 sent inside the colour report, so it takes effect with the next colour update.",
            ),
            KeyboardFamily::ApexM => return None,
        };
        Some(
            SettingDescriptor::new(
                setting_ids::BRIGHTNESS,
                "Brightness",
                SettingKind::Range {
                    min: i64::from(min),
                    max: i64::from(max),
                    step: 1,
                    unit: unit.map(str::to_string),
                },
            )
            .description(description)
            .verification(verification),
        )
    }

    fn actuation_descriptors(&self) -> Vec<SettingDescriptor> {
        if !self.profile.actuation {
            return Vec::new();
        }
        let tenths = SettingKind::Range {
            min: i64::from(*ACTUATION_TENTHS_RANGE.start()),
            max: i64::from(*ACTUATION_TENTHS_RANGE.end()),
            step: 1,
            unit: Some("0.1 mm".to_string()),
        };
        let mut descriptors = vec![
            SettingDescriptor::new(setting_ids::ACTUATION, "Actuation point", tenths.clone())
                .description(
                    "Global actuation depth sent with the experimental 0x2D command. No published \
                     reference documents 0x2D; on models other than the Apex Pro TKL (2023) it is \
                     extrapolated from that model.",
                )
                .verification(Verification::Guess),
        ];
        match self.profile.live_actuation {
            Some(LiveActuation::ApexWeb2023) => descriptors.push(
                SettingDescriptor::new(setting_ids::ACTUATION_LIVE, "Actuation point (live)", tenths)
                    .description(
                        "Actuation depth for every adjustable key, sent as the 0x38 live frame \
                         (apex-web, https://github.com/trottyva/apex-web, tested by its author on \
                         this model with firmware 1.19.7). Lost on unplug.",
                    )
                    .verification(Verification::Reference),
            ),
            Some(LiveActuation::ApexControlGen1) => descriptors.push(
                SettingDescriptor::new(
                    setting_ids::ACTUATION_LIVE,
                    "Actuation point (live)",
                    SettingKind::Choice {
                        options: APEX_GEN1_ACTUATION_TABLE
                            .iter()
                            .map(|(tenths, _)| {
                                ChoiceOption::new(tenths.to_string(), format!("{}.{} mm", tenths / 10, tenths % 10))
                            })
                            .collect(),
                    },
                )
                .description(
                    "Actuation depth for every adjustable key, sent as the 0x31 0x47 live frame \
                     (apex-control, https://github.com/zunuza/apex-control, tested by its author on \
                     this model with firmware 4.16.8). Only the depths SteelSeries GG was seen \
                     sending are offered; the id is the depth in 0.1 mm.",
                )
                .verification(Verification::Reference),
            ),
            None => {}
        }
        descriptors
    }

    fn keyboard_setting_descriptors(&self) -> Vec<SettingDescriptor> {
        let mut descriptors: Vec<SettingDescriptor> = self.brightness_descriptor().into_iter().collect();
        descriptors.extend(self.actuation_descriptors());
        if self.profile.rapid_tap {
            descriptors.push(
                SettingDescriptor::new(setting_ids::RAPID_TAP, "Rapid Tap", SettingKind::Toggle)
                    .description(
                        "Rapid Tap (SOCD) live on/off, 0x1A (apex-control, \
                         https://github.com/zunuza/apex-control, tested on firmware 4.16.8). The key \
                         pairs come from the profile stored on the keyboard.",
                    )
                    .verification(Verification::Reference),
            );
        }
        match self.family {
            KeyboardFamily::OldApex => descriptors.push(
                SettingDescriptor::new(
                    setting_ids::POLLING_RATE,
                    "Polling rate",
                    SettingKind::Choice {
                        options: OldApexPollingRate::ALL
                            .iter()
                            .map(|rate| ChoiceOption::new(rate.hz().to_string(), format!("{} Hz", rate.hz())))
                            .collect(),
                    },
                )
                .description("USB polling rate set by the keyboard (apexctl `poll`, feature report 0x04).")
                .verification(Verification::Reference),
            ),
            KeyboardFamily::TriZone => descriptors.push(
                SettingDescriptor::new(
                    setting_ids::SAVE_TO_DEVICE,
                    "Save lighting to keyboard",
                    SettingKind::Action,
                )
                .description(
                    "Store the current colours on the keyboard (OpenRGB SteelSeriesApexTZoneController::Save).",
                )
                .verification(Verification::Reference)
                .persists_on_device(true),
            ),
            _ => {}
        }
        descriptors
    }

    fn apply_keyboard_setting(&mut self, id: &str, value: &SettingValue) -> Result<()> {
        let descriptors = self.keyboard_setting_descriptors();
        validate_against(&descriptors, id, value)?;
        let tenths = |v: i64| u8::try_from(v).map_err(|_| Error::InvalidConfig(format!("{id}: {v} is out of range")));
        match (id, value) {
            (setting_ids::BRIGHTNESS, SettingValue::Int(level)) => self.apply_brightness_level(tenths(*level)?),
            (setting_ids::ACTUATION, SettingValue::Int(v)) => self.send_actuation(tenths(*v)?),
            (setting_ids::ACTUATION_LIVE, SettingValue::Int(v)) => self.apply_live_actuation(tenths(*v)?),
            (setting_ids::ACTUATION_LIVE, SettingValue::Choice(choice)) => {
                let v = choice
                    .parse::<u8>()
                    .map_err(|_| Error::InvalidConfig(format!("{id}: invalid depth '{choice}'")))?;
                self.apply_live_actuation(v)
            }
            (setting_ids::RAPID_TAP, SettingValue::Bool(enabled)) => self.apply_rapid_tap(*enabled),
            (setting_ids::POLLING_RATE, SettingValue::Choice(choice)) => {
                let rate = choice
                    .parse::<u16>()
                    .ok()
                    .and_then(OldApexPollingRate::from_hz)
                    .ok_or_else(|| Error::InvalidConfig(format!("{id}: invalid rate '{choice}'")))?;
                self.apply_polling_rate(rate)
            }
            (setting_ids::SAVE_TO_DEVICE, SettingValue::Trigger) => self.save_tri_zone(),
            _ => Err(Error::Unsupported(format!(
                "setting '{id}' is not supported by this device"
            ))),
        }
    }
}

impl Configurable for GenericKeyboard {
    fn setting_descriptors(&self) -> Vec<SettingDescriptor> {
        self.keyboard_setting_descriptors()
    }

    fn apply_setting(&mut self, id: &str, value: &SettingValue) -> Result<()> {
        self.apply_keyboard_setting(id, value)
    }
}

/// Convert a 0-100 % brightness to the family's native range.
fn percent_to_level(percent: u8, (min, max): (u8, u8)) -> u8 {
    let span = u32::from(max - min);
    let level = (u32::from(percent.min(100)) * span + 50) / 100;
    min + level as u8
}

impl Device for GenericKeyboard {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Keyboard
    }

    fn initialize(&mut self) -> Result<()> {
        match self.family {
            KeyboardFamily::Legacy => {
                let mut buffer = [0u8; 65];
                let size = self.report_builder.build_report(ApplyCommand, &mut buffer)?;
                self.send_report(&buffer[..size])?;
                Ok(())
            }
            KeyboardFamily::ApexPerKey(_) => {
                self.probe_apex_per_key();
                self.ensure_family_ready()
            }
            KeyboardFamily::ApexM => self.ensure_family_ready(),
            // OpenRGB sends nothing when it opens the zone families.
            KeyboardFamily::EightZone | KeyboardFamily::TriZone | KeyboardFamily::OldApex => Ok(()),
        }
    }

    fn close(&mut self) -> Result<()> {
        self.device = None;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.device.is_some()
    }

    fn send_raw(&mut self, data: &[u8]) -> Result<()> {
        self.send_report(data)
    }

    fn receive_raw(&mut self, buf: &mut [u8]) -> Result<usize> {
        let device = self
            .device
            .as_ref()
            .ok_or(Error::DeviceCommunication("Device not connected".to_string()))?;
        let device = device.lock();

        if let Some(result) = with_global_diagnostics(|diag| {
            diag.record_timed_operation(HidOperation::Receive, &[], || {
                let len = device.read(buf)?;
                Ok((len, buf[..len].to_vec()))
            })
        }) {
            result.map(|(len, _data)| len)
        } else {
            let len = device.read(buf)?;
            Ok(len)
        }
    }
}

impl GenericKeyboard {
    fn compute_average_color(&self, key_colors: &[(KeyId, Color)]) -> Color {
        if key_colors.is_empty() {
            return Color::BLACK;
        }

        let (total_r, total_g, total_b) = key_colors.iter().fold((0u32, 0u32, 0u32), |(r, g, b), (_, color)| {
            (r + color.r as u32, g + color.g as u32, b + color.b as u32)
        });

        let count = key_colors.len() as u32;
        Color::new(
            (total_r / count) as u8,
            (total_g / count) as u8,
            (total_b / count) as u8,
        )
    }
}

#[async_trait]
impl Keyboard for GenericKeyboard {
    async fn set_color(&mut self, color: Color) -> Result<()> {
        self.zone_color_buffer.clear();
        self.zone_color_buffer.resize(self.zone_count, color);
        self.send_zone_buffer_async().await
    }

    async fn set_zone_colors(&mut self, colors: &[Color]) -> Result<()> {
        self.zone_color_buffer.clear();
        let len = colors.len().min(self.zone_count);
        self.zone_color_buffer.extend_from_slice(&colors[..len]);

        while self.zone_color_buffer.len() < self.zone_count {
            self.zone_color_buffer.push(Color::BLACK);
        }

        self.send_zone_buffer_async().await
    }

    fn zone_count(&self) -> usize {
        self.zone_count
    }

    async fn set_brightness(&mut self, brightness: u8) -> Result<()> {
        if self.family == KeyboardFamily::Legacy {
            return self.send_command_async(BrightnessCommand::new(brightness)).await;
        }
        let range = self.brightness_range().ok_or_else(|| self.brightness_unsupported())?;
        self.apply_brightness_level(percent_to_level(brightness, range))
    }

    async fn apply(&mut self) -> Result<()> {
        // Only the legacy dialect has an apply command. The OpenRGB families stream colours
        // live and OpenRGB never sends 0x09 to them; on Gen 3 boards it reverts to the stored
        // profile.
        if self.family != KeyboardFamily::Legacy {
            return Ok(());
        }
        let apply_command = ApplyCommand;
        let mut buffer = [0u8; 65];
        let size = self.report_builder.build_report(apply_command, &mut buffer)?;
        let _ = self.send_report_async(&buffer[..size]).await;
        Ok(())
    }

    // === Per-Key RGB Control Implementation ===

    fn supports_per_key_rgb(&self) -> bool {
        self.key_mapping.is_some()
    }

    fn get_key_mapping(&self) -> Option<&KeyMapping> {
        self.key_mapping.as_ref()
    }

    async fn set_key_color(&mut self, key_id: KeyId, color: Color) -> Result<()> {
        let mapping = self.key_mapping.as_ref().ok_or_else(|| {
            Error::DeviceCommunication("Per-key RGB not supported - no key mapping available".to_string())
        })?;

        let address = mapping
            .get_key_address(key_id)
            .ok_or_else(|| Error::DeviceCommunication(format!("Key {:?} not found in key mapping", key_id)))?;

        self.set_key_color_direct(address, color).await
    }

    async fn set_key_colors(&mut self, key_colors: &[(KeyId, Color)]) -> Result<()> {
        if self.family.uses_key_frame() {
            return self.update_key_frame(key_colors, false);
        }
        if self.family.is_zone_only() {
            return Err(self.no_per_key_error());
        }

        let mapping = self.key_mapping.as_ref().ok_or_else(|| {
            Error::DeviceCommunication("Per-key RGB not supported - no key mapping available".to_string())
        })?;

        let mut builder = PerKeyRgbBuilder::new(super::hid_reports::PerKeyAddressingMode::HidCode);
        for (key_id, color) in key_colors {
            if let Some(address) = mapping.get_key_address(*key_id) {
                builder.add_key_matrix(address, *color);
            } else {
                tracing::warn!("Key {:?} not found in key mapping", key_id);
            }
        }

        if builder.is_empty() {
            return Err(Error::DeviceCommunication(
                "No valid keys found in key mapping".to_string(),
            ));
        }

        let command = builder.build();
        for fragment in command.fragment_into_reports() {
            self.send_command_async(fragment).await?;
        }
        Ok(())
    }

    async fn set_key_color_direct(&mut self, address: KeyAddress, color: Color) -> Result<()> {
        self.set_key_colors_direct(&[(address, color)]).await
    }

    async fn set_key_colors_direct(&mut self, key_colors: &[(KeyAddress, Color)]) -> Result<()> {
        if key_colors.is_empty() {
            return Err(Error::DeviceCommunication("No key colors provided".to_string()));
        }
        if self.family.uses_key_frame() {
            let mut applied = 0usize;
            for (address, color) in key_colors {
                if self.apex_frame.set_hid(address.hid_code, *color) {
                    applied += 1;
                } else {
                    tracing::debug!("No LED has HID usage 0x{:02x}", address.hid_code);
                }
            }
            if applied == 0 {
                return Err(Error::DeviceCommunication(
                    "None of the HID usages has an LED".to_string(),
                ));
            }
            return self.send_apex_frame();
        }
        if self.family.is_zone_only() {
            return Err(self.no_per_key_error());
        }
        if let [(address, color)] = key_colors {
            return self
                .send_command_async(PerKeyRgbCommand::single_key(*address, *color))
                .await;
        }

        let mut builder = PerKeyRgbBuilder::new(super::hid_reports::PerKeyAddressingMode::HidCode);

        for (address, color) in key_colors {
            builder.add_key_matrix(*address, *color);
        }

        self.send_command_async(builder.build()).await
    }

    async fn clear_per_key_rgb(&mut self) -> Result<()> {
        if self.family.uses_key_frame() {
            self.apex_frame.fill(Color::BLACK);
            return self.send_apex_frame();
        }
        if self.family.is_zone_only() {
            // The legacy fallback below would send 0x23 packets, which these families read as a
            // brightness write.
            return self.set_color(Color::BLACK).await;
        }
        if let Some(ref mapping) = self.key_mapping {
            let black_keys: Vec<(KeyId, Color)> = mapping
                .get_all_keys()
                .iter()
                .map(|&key_id| (key_id, Color::BLACK))
                .collect();

            if !black_keys.is_empty() {
                self.set_key_colors(&black_keys).await
            } else {
                Ok(())
            }
        } else {
            let mut builder = PerKeyRgbBuilder::new(super::hid_reports::PerKeyAddressingMode::HidCode);

            for hid_code in 0..120 {
                builder.add_key_matrix(KeyAddress::new(hid_code), Color::BLACK);
            }

            let command = builder.build();
            let mut buffer = [0u8; 65];
            let size = self.report_builder.build_report(command, &mut buffer)?;
            self.send_report_async(&buffer[..size]).await
        }
    }

    async fn set_key_region(&mut self, start_hid: u8, count: u8, color: Color) -> Result<()> {
        if self.family.uses_key_frame() || self.family.is_zone_only() {
            let addresses: Vec<(KeyAddress, Color)> = (0..count)
                .filter_map(|offset| start_hid.checked_add(offset))
                .map(|hid_code| (KeyAddress::new(hid_code), color))
                .collect();
            if addresses.is_empty() {
                return Err(Error::DeviceCommunication(
                    "Invalid region - no keys to set".to_string(),
                ));
            }
            return self.set_key_colors_direct(&addresses).await;
        }
        let mut builder = PerKeyRgbBuilder::new(super::hid_reports::PerKeyAddressingMode::HidCode);
        builder.set_region(start_hid, count, color);

        if builder.is_empty() {
            return Err(Error::DeviceCommunication(
                "Invalid region - no keys to set".to_string(),
            ));
        }

        self.send_command_async(builder.build()).await
    }

    // === Zone-based RGB Fallback Implementation ===

    fn get_zone_mapping(&self) -> Option<&ZoneMap> {
        self.zone_mapping.as_ref()
    }

    async fn set_zone_effect(&mut self, effect: ZoneEffect) -> Result<()> {
        self.zone_fallback.set_current_effect(effect.clone());
        let colors = effect.compute_colors(self.zone_count, 0.0);
        self.set_zone_colors_with_retry(&colors, 3).await
    }

    async fn simulate_per_key_with_zones(&mut self, key_colors: &[(KeyId, Color)]) -> Result<()> {
        if let Some(zone_colors) = self
            .zone_fallback
            .simulate_per_key_effect(self.info.product_id, key_colors)
        {
            self.set_zone_colors_with_retry(&zone_colors, 3).await
        } else if !key_colors.is_empty() {
            let avg_color = self.compute_average_color(key_colors);
            self.set_color(avg_color).await?;
            self.apply().await
        } else {
            self.set_color(Color::BLACK).await?;
            self.apply().await
        }
    }

    async fn set_zone_colors_with_retry(&mut self, colors: &[Color], max_retries: usize) -> Result<()> {
        let mut last_error = None;

        for attempt in 0..max_retries {
            match self.set_zone_colors(colors).await {
                Ok(()) => {
                    if attempt > 0 {
                        tracing::info!("Zone RGB succeeded on attempt {}", attempt + 1);
                    }
                    return Ok(());
                }
                Err(e) => {
                    last_error = Some(e);
                    if attempt < max_retries - 1 {
                        tracing::warn!("Zone RGB attempt {} failed, retrying: {:?}", attempt + 1, last_error);
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        }

        if let Some(e) = last_error {
            tracing::error!("Zone RGB failed after {} attempts", max_retries);
            Err(e)
        } else {
            Err(Error::DeviceCommunication(
                "Zone RGB failed with unknown error".to_string(),
            ))
        }
    }

    async fn test_zone_reliability(&mut self) -> Result<Vec<bool>> {
        let mut results = Vec::new();

        for zone_index in 0..self.zone_count {
            let mut zone_colors = vec![Color::BLACK; self.zone_count];
            zone_colors[zone_index] = Color::WHITE;

            let success = self.set_zone_colors(&zone_colors).await.is_ok();
            results.push(success);

            if !success {
                tracing::warn!("Zone {} failed reliability test", zone_index);
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let _ = self.set_color(Color::BLACK).await;

        tracing::info!(
            "Zone reliability test completed: {}/{} zones working",
            results.iter().filter(|&&x| x).count(),
            results.len()
        );

        Ok(results)
    }

    // === Per-Key RGB Effect Implementation ===

    fn supports_per_key_effects(&self) -> bool {
        self.per_key_controller.is_some()
    }

    async fn set_per_key_effect(&mut self, effect: PerKeyEffect) -> Result<()> {
        let key_colors = if let Some(ref mut controller) = self.per_key_controller {
            controller.set_effect(effect.clone());
            Some(controller.compute_key_colors().to_vec())
        } else {
            None
        };

        if let Some(colors) = key_colors {
            self.set_key_colors(&colors).await?;
            self.apply().await
        } else {
            self.convert_per_key_to_zones(&effect).await
        }
    }

    fn get_per_key_effect(&self) -> Option<&PerKeyEffect> {
        self.per_key_controller.as_ref().map(|c| c.effect())
    }

    async fn trigger_key_reactive(&mut self, keys: &[KeyId], duration: f32) -> Result<()> {
        let key_colors = if let Some(ref mut controller) = self.per_key_controller {
            controller.trigger_reactive(keys, duration);
            Some(controller.compute_key_colors().to_vec())
        } else {
            None
        };

        if let Some(colors) = key_colors {
            self.set_key_colors(&colors).await?;
            self.apply().await
        } else if !keys.is_empty() {
            self.simulate_per_key_with_zones(&keys.iter().map(|&k| (k, Color::WHITE)).collect::<Vec<_>>())
                .await
        } else {
            Ok(())
        }
    }

    async fn apply_per_key_effect_with_brightness(&mut self, brightness: f32) -> Result<()> {
        let key_colors = if let Some(ref mut controller) = self.per_key_controller {
            controller.set_brightness(brightness.clamp(0.0, 1.0));
            Some(controller.compute_key_colors().to_vec())
        } else {
            None
        };

        if let Some(colors) = key_colors {
            self.set_key_colors(&colors).await?;
            self.apply().await
        } else {
            self.set_brightness((brightness * 100.0) as u8).await?;
            self.apply().await
        }
    }

    async fn convert_per_key_to_zones(&mut self, effect: &PerKeyEffect) -> Result<()> {
        let zone_effect = match effect {
            PerKeyEffect::Static { color } => ZoneEffect::Solid(*color),

            PerKeyEffect::Breathing { color, speed: _ } => ZoneEffect::Breathing {
                colors: vec![*color],
                phase_offset: 0.0,
            },

            PerKeyEffect::Spectrum { speed: _ } => ZoneEffect::Wave {
                colors: Color::RAINBOW_COLORS.to_vec(),
                offset: 0.0,
            },

            PerKeyEffect::Wave {
                colors,
                speed: _,
                direction: _,
            } => {
                if colors.is_empty() {
                    ZoneEffect::Solid(Color::BLACK)
                } else {
                    ZoneEffect::Wave {
                        colors: colors.clone(),
                        offset: 0.0,
                    }
                }
            }

            PerKeyEffect::Gradient {
                start,
                end,
                direction: _,
            } => ZoneEffect::Gradient {
                start: *start,
                end: *end,
            },

            PerKeyEffect::GameZone {
                wasd_color,
                default_color,
                ..
            } => ZoneEffect::Alternating(vec![*wasd_color, *default_color]),

            PerKeyEffect::Custom { key_colors } => {
                if key_colors.is_empty() {
                    ZoneEffect::Solid(Color::BLACK)
                } else {
                    let (total_r, total_g, total_b) =
                        key_colors.values().fold((0u32, 0u32, 0u32), |(r, g, b), color| {
                            (r + color.r as u32, g + color.g as u32, b + color.b as u32)
                        });
                    let count = key_colors.len() as u32;

                    let avg_color = Color::new(
                        (total_r / count) as u8,
                        (total_g / count) as u8,
                        (total_b / count) as u8,
                    );
                    ZoneEffect::Solid(avg_color)
                }
            }

            PerKeyEffect::Off => ZoneEffect::Solid(Color::BLACK),

            _ => ZoneEffect::Solid(Color::WHITE),
        };

        self.set_zone_effect(zone_effect).await
    }

    // === Performance Optimization Implementation ===

    fn get_rgb_performance_stats(&self) -> Option<&crate::performance::PerformanceStats> {
        self.per_key_controller.as_ref().and_then(|c| c.get_performance_stats())
    }

    fn get_optimal_frame_time(&self) -> Option<std::time::Duration> {
        self.per_key_controller.as_ref().and_then(|c| c.get_frame_time())
    }

    fn cleanup_rgb_caches(&mut self) {
        if let Some(ref mut controller) = self.per_key_controller {
            controller.cleanup_performance_caches();
        }
    }

    fn set_performance_optimization(&mut self, enabled: bool) {
        if let Some(ref mut controller) = self.per_key_controller {
            if enabled {
                controller.enable_performance_optimization();
            } else {
                controller.disable_performance_optimization();
            }
        }
    }

    fn read_actuation_point(&mut self) -> Result<u8> {
        if let Some(value) = self.actuation_point_cache {
            Ok(value)
        } else {
            Err(Error::DeviceCommunication(
                "Reading actuation point not yet implemented - HID read command not discovered and no cached value available. Hint: the actuation point can currently only be retrieved if it was set earlier in this session; cache the value you set instead of relying on reading it back.".to_string(),
            ))
        }
    }

    // === OLED screen ===

    fn oled_size(&self) -> Option<(u32, u32)> {
        oled::oled_size_for_product_id(self.info.product_id)
    }

    async fn draw_oled(&mut self, frame: &crate::oled::OledFrame) -> Result<()> {
        oled::draw_frame(&self.info, frame).await
    }

    fn set_actuation_point(&mut self, value: u8) -> Result<()> {
        self.send_actuation(value)
    }

    fn set_actuation_point_mm(&mut self, mm: f32) -> Result<()> {
        self.send_actuation(ActuationCommand::from_mm(mm).actuation_point)
    }

    async fn set_all_key_colors(&mut self, colors: &[(KeyId, Color)]) -> Result<()> {
        if self.family.uses_key_frame() {
            return self.update_key_frame(colors, true);
        }
        self.set_key_colors(colors).await
    }
}

/// Macro to delegate common `Keyboard` trait methods to `self.inner`.
/// This avoids duplicating dozens of identical trait method implementations.
///
/// `apply` is deliberately not generated here: devices that stream colours in
/// direct mode must suppress it, so each implementation states its own policy.
#[macro_export]
macro_rules! impl_keyboard_with_delegation {
    ($type:ty, { $($custom:item)* }) => {
        #[async_trait]
        impl Keyboard for $type {
            $($custom)*
            fn zone_count(&self) -> usize { self.inner.zone_count() }
            async fn set_brightness(&mut self, brightness: u8) -> Result<()> { self.inner.set_brightness(brightness).await }
            fn supports_per_key_rgb(&self) -> bool { self.inner.supports_per_key_rgb() }
            fn get_key_mapping(&self) -> Option<&KeyMapping> { self.inner.get_key_mapping() }
            async fn set_key_color_direct(&mut self, address: KeyAddress, color: Color) -> Result<()> { self.inner.set_key_color_direct(address, color).await }
            async fn set_key_colors_direct(&mut self, key_colors: &[(KeyAddress, Color)]) -> Result<()> { self.inner.set_key_colors_direct(key_colors).await }
            async fn clear_per_key_rgb(&mut self) -> Result<()> { self.set_color(Color::BLACK).await }
            async fn set_key_region(&mut self, start_hid: u8, count: u8, color: Color) -> Result<()> { self.inner.set_key_region(start_hid, count, color).await }
            fn get_zone_mapping(&self) -> Option<&ZoneMapping> { self.inner.get_zone_mapping() }
            async fn set_zone_effect(&mut self, effect: ZoneEffect) -> Result<()> { self.inner.set_zone_effect(effect).await }
            async fn simulate_per_key_with_zones(&mut self, key_colors: &[(KeyId, Color)]) -> Result<()> { self.inner.simulate_per_key_with_zones(key_colors).await }
            async fn set_zone_colors_with_retry(&mut self, colors: &[Color], max_retries: usize) -> Result<()> { self.inner.set_zone_colors_with_retry(colors, max_retries).await }
            async fn test_zone_reliability(&mut self) -> Result<Vec<bool>> { self.inner.test_zone_reliability().await }
            fn supports_per_key_effects(&self) -> bool { self.inner.supports_per_key_effects() }
            async fn set_per_key_effect(&mut self, effect: PerKeyEffect) -> Result<()> { self.inner.set_per_key_effect(effect).await }
            fn get_per_key_effect(&self) -> Option<&PerKeyEffect> { self.inner.get_per_key_effect() }
            async fn trigger_key_reactive(&mut self, keys: &[KeyId], duration: f32) -> Result<()> { self.inner.trigger_key_reactive(keys, duration).await }
            async fn apply_per_key_effect_with_brightness(&mut self, brightness: f32) -> Result<()> { self.inner.apply_per_key_effect_with_brightness(brightness).await }
            async fn convert_per_key_to_zones(&mut self, effect: &PerKeyEffect) -> Result<()> { self.inner.convert_per_key_to_zones(effect).await }
            fn get_rgb_performance_stats(&self) -> Option<&$crate::performance::PerformanceStats> { self.inner.get_rgb_performance_stats() }
            fn get_optimal_frame_time(&self) -> Option<std::time::Duration> { self.inner.get_optimal_frame_time() }
            fn cleanup_rgb_caches(&mut self) { self.inner.cleanup_rgb_caches() }
            fn set_performance_optimization(&mut self, enabled: bool) { self.inner.set_performance_optimization(enabled) }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::product_ids;
    use crate::devices::{HidCommand, PerKeyAddressingMode};

    fn create_test_device_info() -> DeviceInfo {
        DeviceInfo {
            name: std::borrow::Cow::Borrowed("Test Apex Pro TKL 2023"),
            device_type: crate::devices::DeviceType::Keyboard,
            vendor_id: crate::STEELSERIES_VENDOR_ID,
            product_id: product_ids::APEX_PRO_TKL_2023,
            interface_number: 1,
            usage_page: 0xFFC0,
            usage: 0x01,
            serial_number: Some("TEST123".to_string()),
            manufacturer: Some("SteelSeries".to_string()),
            path: "/test/device/path".to_string(),
        }
    }

    #[test]
    fn test_per_key_rgb_support_detection() {
        let _info = create_test_device_info();

        let db = KeyMappingDatabase::new();

        assert!(db.supports_product(product_ids::APEX_PRO_TKL_2023));
        assert!(db.get_mapping(product_ids::APEX_PRO_TKL_2023).is_some());

        assert!(!db.supports_product(0xFFFF));
        assert!(db.get_mapping(0xFFFF).is_none());
    }

    #[test]
    fn test_key_mapping_integration() {
        let db = KeyMappingDatabase::new();

        if let Some(mapping) = db.get_mapping(product_ids::APEX_PRO_TKL_2023) {
            assert!(mapping.supports_key(KeyId::A));
            assert!(mapping.supports_key(KeyId::Enter));
            assert!(mapping.supports_key(KeyId::Space));
            assert!(mapping.supports_key(KeyId::Escape));

            assert!(mapping.get_key_address(KeyId::A).is_some());
            assert!(mapping.get_key_address(KeyId::Enter).is_some());

            let stats = mapping.get_stats();
            assert!(stats.total_keys > 0);
            assert!(stats.utilization > 0.0);
        }
    }

    #[test]
    fn test_per_key_rgb_builder_integration() {
        let db = KeyMappingDatabase::new();

        if let Some(mapping) = db.get_mapping(product_ids::APEX_PRO_TKL_2023) {
            let mut builder = PerKeyRgbBuilder::with_key_mapping(mapping.clone());

            let result = builder.add_key_logical(KeyId::A, Color::RED);
            assert!(result.is_ok());

            let result = builder.add_key_logical(KeyId::S, Color::GREEN);
            assert!(result.is_ok());

            let result = builder.add_key_logical(KeyId::D, Color::BLUE);
            assert!(result.is_ok());

            assert_eq!(builder.key_count(), 3);

            let command = builder.build();
            assert_eq!(command.key_count(), 3);
            assert_eq!(command.addressing_mode, PerKeyAddressingMode::HidCode);

            assert!(command.validate().is_ok());
        }
    }

    #[test]
    fn test_zone_count_mapping() {
        assert_eq!(zone_count_for_product_id(product_ids::APEX_PRO_TKL_2023), 9);
        // OpenRGB: STEELSERIES_TZ_LED_COUNT = 10, STEELSERIES_8Z_LED_COUNT = 8, old Apex 5 zones.
        assert_eq!(zone_count_for_product_id(product_ids::APEX_3), 10);
        assert_eq!(zone_count_for_product_id(product_ids::APEX_3_TKL), 8);
        assert_eq!(zone_count_for_product_id(product_ids::APEX_OG), 5);
        assert_eq!(zone_count_for_product_id(product_ids::APEX_350), 5);
        assert_eq!(zone_count_for_product_id(product_ids::APEX_7), 1);

        assert_eq!(zone_count_for_product_id(0xFFFF), 1);
    }

    fn keyboard_without_device(product_id: u16) -> GenericKeyboard {
        let mut info = create_test_device_info();
        info.product_id = product_id;
        GenericKeyboard::new_without_device(info)
    }

    fn descriptor(keyboard: &GenericKeyboard, id: &str) -> Option<SettingDescriptor> {
        keyboard.setting_descriptors().into_iter().find(|d| d.id == id)
    }

    fn range_of(descriptor: &SettingDescriptor) -> (i64, i64) {
        match descriptor.kind {
            SettingKind::Range { min, max, .. } => (min, max),
            ref other => panic!("{} is not a range: {other:?}", descriptor.id),
        }
    }

    #[test]
    fn settings_per_family() {
        // 8-zone: brightness 0..=0x10 (OpenRGB STEELSERIES_8Z_BRIGHTNESS_MAX).
        let apex_3_tkl = keyboard_without_device(product_ids::APEX_3_TKL);
        let brightness = descriptor(&apex_3_tkl, setting_ids::BRIGHTNESS).unwrap();
        assert_eq!(range_of(&brightness), (0, 16));
        assert_eq!(brightness.verification, Verification::Reference);
        assert!(descriptor(&apex_3_tkl, setting_ids::ACTUATION).is_none());

        // Tri-zone: brightness 0..=100 and the save action.
        let apex_3 = keyboard_without_device(product_ids::APEX_3);
        assert_eq!(
            range_of(&descriptor(&apex_3, setting_ids::BRIGHTNESS).unwrap()),
            (0, 100)
        );
        let save = descriptor(&apex_3, setting_ids::SAVE_TO_DEVICE).unwrap();
        assert_eq!(save.kind, SettingKind::Action);
        assert!(save.persists_on_device);

        // Old Apex: brightness 1..=8 and the apexctl polling rate.
        let old_apex = keyboard_without_device(product_ids::APEX_OG);
        assert_eq!(
            range_of(&descriptor(&old_apex, setting_ids::BRIGHTNESS).unwrap()),
            (1, 8)
        );
        let polling = descriptor(&old_apex, setting_ids::POLLING_RATE).unwrap();
        assert_eq!(polling.verification, Verification::Reference);
        assert!(polling.parse_value("1000").is_ok());
        assert!(polling.parse_value("300").is_err());

        // M750: nothing documented.
        assert!(
            keyboard_without_device(product_ids::APEX_M750)
                .setting_descriptors()
                .is_empty()
        );

        // Per-key family without a brightness answer: no brightness, as in OpenRGB.
        let apex_7 = keyboard_without_device(product_ids::APEX_7);
        assert!(apex_7.setting_descriptors().is_empty());

        // Apex Pro TKL: 0x2D actuation (Guess), apex-control live actuation and Rapid Tap.
        let apex_pro_tkl = keyboard_without_device(product_ids::APEX_PRO_TKL);
        let actuation = descriptor(&apex_pro_tkl, setting_ids::ACTUATION).unwrap();
        assert_eq!(range_of(&actuation), (1, 40));
        assert_eq!(actuation.verification, Verification::Guess);
        let live = descriptor(&apex_pro_tkl, setting_ids::ACTUATION_LIVE).unwrap();
        assert_eq!(live.verification, Verification::Reference);
        match &live.kind {
            SettingKind::Choice { options } => assert_eq!(options.len(), 36),
            other => panic!("unexpected kind {other:?}"),
        }
        assert!(live.parse_value("20").is_ok());
        assert!(live.parse_value("2.0 mm").is_ok());
        assert!(live.parse_value("32").is_err(), "3.2 mm is excluded");
        assert!(descriptor(&apex_pro_tkl, setting_ids::RAPID_TAP).is_some());

        // Legacy models with no reference: 0x22 brightness, labelled Guess.
        let apex_150 = keyboard_without_device(product_ids::APEX_150);
        let brightness = descriptor(&apex_150, setting_ids::BRIGHTNESS).unwrap();
        assert_eq!(range_of(&brightness), (0, 100));
        assert_eq!(brightness.verification, Verification::Guess);
    }

    #[test]
    fn settings_validation_rejects_bad_values_before_sending() {
        let mut apex_pro_tkl = keyboard_without_device(product_ids::APEX_PRO_TKL);
        for bad in [0, 41] {
            assert!(matches!(
                apex_pro_tkl.apply_setting(setting_ids::ACTUATION, &SettingValue::Int(bad)),
                Err(Error::InvalidConfig(_))
            ));
        }
        assert!(matches!(
            apex_pro_tkl.apply_setting(setting_ids::ACTUATION_LIVE, &SettingValue::Choice("32".into())),
            Err(Error::InvalidConfig(_))
        ));
        assert!(matches!(
            apex_pro_tkl.apply_setting(setting_ids::BRIGHTNESS, &SettingValue::Int(5)),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            apex_pro_tkl.apply_setting("does_not_exist", &SettingValue::Int(1)),
            Err(Error::Unsupported(_))
        ));
        // A valid value reaches the transport, which has no device here.
        assert!(
            apex_pro_tkl
                .apply_setting(setting_ids::ACTUATION, &SettingValue::Int(20))
                .is_err()
        );

        let mut apex_3_tkl = keyboard_without_device(product_ids::APEX_3_TKL);
        assert!(matches!(
            apex_3_tkl.apply_setting(setting_ids::BRIGHTNESS, &SettingValue::Int(17)),
            Err(Error::InvalidConfig(_))
        ));
        assert!(matches!(
            apex_3_tkl.apply_setting(setting_ids::BRIGHTNESS, &SettingValue::Choice("x".into())),
            Err(Error::InvalidConfig(_))
        ));
    }

    #[test]
    fn percent_maps_to_native_brightness() {
        assert_eq!(percent_to_level(0, (0, 10)), 0);
        assert_eq!(percent_to_level(55, (0, 10)), 6);
        assert_eq!(percent_to_level(100, (0, 10)), 10);
        assert_eq!(percent_to_level(200, (0, 16)), 16);
        assert_eq!(percent_to_level(50, (0, 16)), 8);
        assert_eq!(percent_to_level(0, (1, 8)), 1);
        assert_eq!(percent_to_level(100, (1, 8)), 8);
    }

    #[test]
    fn key_frame_partial_and_full_updates() {
        // No device: every send fails, but the frame shows what would have been sent.
        let mut keyboard = keyboard_without_device(product_ids::APEX_7);
        assert!(keyboard.family().uses_key_frame());

        assert!(keyboard.update_key_frame(&[(KeyId::A, Color::RED)], false).is_err());
        assert!(keyboard.update_key_frame(&[(KeyId::S, Color::GREEN)], false).is_err());
        assert_eq!(keyboard.apex_frame().color_of(KeyId::A), Some(Color::RED));
        assert_eq!(keyboard.apex_frame().color_of(KeyId::S), Some(Color::GREEN));

        // A full frame turns every unlisted key off.
        assert!(keyboard.update_key_frame(&[(KeyId::D, Color::BLUE)], true).is_err());
        assert_eq!(keyboard.apex_frame().color_of(KeyId::A), Some(Color::BLACK));
        assert_eq!(keyboard.apex_frame().color_of(KeyId::D), Some(Color::BLUE));

        // Keys outside the model's mapping change nothing: the Apex 7 TKL has no numpad.
        let mut tkl = keyboard_without_device(product_ids::APEX_7_TKL);
        assert!(matches!(
            tkl.update_key_frame(&[(KeyId::Num5, Color::RED)], false),
            Err(Error::DeviceCommunication(_))
        ));
        assert_eq!(tkl.apex_frame().color_of(KeyId::Num5), Some(Color::BLACK));
    }

    #[tokio::test]
    async fn zone_families_refuse_per_key_writes() {
        let mut apex_3_tkl = keyboard_without_device(product_ids::APEX_3_TKL);
        assert!(matches!(
            apex_3_tkl.set_key_colors(&[(KeyId::A, Color::RED)]).await,
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            apex_3_tkl.set_key_color_direct(KeyAddress::new(0x04), Color::RED).await,
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            apex_3_tkl.set_key_region(0x04, 4, Color::RED).await,
            Err(Error::Unsupported(_))
        ));
        // apply() is a no-op outside the legacy family, so it succeeds without a device.
        assert!(apex_3_tkl.apply().await.is_ok());
        assert_eq!(apex_3_tkl.zone_count(), 8);
    }

    #[tokio::test]
    async fn per_key_family_brightness_needs_read_back() {
        let mut apex_7 = keyboard_without_device(product_ids::APEX_7);
        assert!(matches!(apex_7.set_brightness(50).await, Err(Error::Unsupported(_))));
        let mut m750 = keyboard_without_device(product_ids::APEX_M750);
        assert!(matches!(m750.set_brightness(50).await, Err(Error::Unsupported(_))));
    }

    #[test]
    fn actuation_only_on_apex_pro() {
        let mut apex_7 = keyboard_without_device(product_ids::APEX_7);
        assert!(matches!(
            apex_7.set_actuation_point(20),
            Err(Error::DeviceCommunication(_))
        ));
        let mut apex_pro = keyboard_without_device(product_ids::APEX_PRO);
        assert!(apex_pro.set_actuation_point(0).is_err(), "0x2D range is 1..=40");
        assert!(descriptor(&apex_pro, setting_ids::ACTUATION).is_some());
        assert!(descriptor(&apex_pro, setting_ids::ACTUATION_LIVE).is_none());
    }

    #[test]
    #[allow(deprecated)]
    fn test_per_key_addressing_modes() {
        assert_ne!(PerKeyAddressingMode::Matrix, PerKeyAddressingMode::Logical);

        let matrix_builder = PerKeyRgbBuilder::new(PerKeyAddressingMode::Matrix);
        assert!(matrix_builder.is_empty());

        let db = KeyMappingDatabase::new();
        if let Some(mapping) = db.get_mapping(product_ids::APEX_PRO_TKL_2023) {
            let logical_builder = PerKeyRgbBuilder::with_key_mapping(mapping.clone());
            assert!(logical_builder.is_empty());
        }
    }

    #[test]
    fn test_key_address_bounds() {
        // KeyAddress now uses HID codes instead of row/col
        let addr = KeyAddress::new(30); // HID code 30 (1 key)
        assert_eq!(addr.hid_code, 30);

        let mut command = PerKeyRgbCommand::new(PerKeyAddressingMode::HidCode);

        command.set_key_color(KeyAddress::new(30), Color::RED);
        assert!(command.validate().is_ok());

        // HID code 0 is invalid (not a standard USB HID keycode)
        command.set_key_color(KeyAddress::new(0), Color::BLUE);
        assert!(command.validate().is_err());
    }

    #[test]
    fn test_supported_product_ids() {
        let db = KeyMappingDatabase::new();
        let supported_products = db.get_supported_products();

        assert!(supported_products.contains(&product_ids::APEX_PRO_TKL_2023));
        assert!(supported_products.contains(&product_ids::APEX_PRO));
        assert!(supported_products.contains(&product_ids::APEX_PRO_TKL));

        assert!(!supported_products.is_empty());
    }
}
