//! [EXPERIMENTAL] Apex 7 and the rest of the legacy "macho" Apex family.
//!
//! Wraps [`GenericKeyboard`] — its [`HidReportBuilder`](crate::devices::hid_reports::HidReportBuilder)
//! and the open [`HidDevice`](hidapi::HidDevice) — and adds the on-board profile read: this object
//! performs the packet reads and writes itself, on the handle of the collection the device manager
//! opened, and that collection stays open for the whole read (`usbhid` polls a
//! collection's interrupt endpoint only while a descriptor on that node is open, so the opening-chain
//! answers and the pull acknowledges have nowhere to arrive on a per-report open).
//!
//! Frame construction, reply parsing, payload geometry and decode live in [`crate::onboard_profile`].
//!
//! `[EXPERIMENTAL]`: the protocol is reverse-engineered from the Windows USBPcap capture of
//! official GG on an Apex 7 (PID `0x1612`, firmware `4.16.8`), written up in
//! `docs/development/apex-7-onboard-profile-read.md`. No read has completed on hardware from this code,
//! and this device writes no profile: there is no read-back command for the family.

use std::ops::{Deref, DerefMut};
#[cfg(feature = "onboard")]
use std::sync::Arc;

#[cfg(feature = "onboard")]
use parking_lot::Mutex;

#[cfg(feature = "onboard")]
use hidapi::HidDevice;

use async_trait::async_trait;

#[cfg(feature = "onboard")]
use super::apex_legacy_onboard;
use super::{GenericKeyboard, Keyboard};
use crate::devices::key_mapping::{KeyId, KeyMapping};
use crate::devices::product_ids;
use crate::devices::zone_mapping::{ZoneEffect, ZoneMapping};
use crate::devices::{Device, DeviceType};
#[cfg(feature = "onboard")]
use crate::error::Error;
use crate::error::Result;
#[cfg(feature = "onboard")]
use crate::onboard_profile::{ProfileImage, ReadConfig};
use crate::rgb::{Color, PerKeyEffect};

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

/// Apex 7 keyboard device: standard lighting control through [`GenericKeyboard`], plus - with the
/// `onboard` feature - the `[EXPERIMENTAL]` on-board profile read on its own handle.
///
/// Discovery builds this device for every [`onboard_family`] product ID whether or not the crate was
/// built with `onboard`: the device class is part of the device tree, only the read is feature-gated.
pub struct Apex7 {
    inner: GenericKeyboard,
    #[cfg(feature = "onboard")]
    onboard: ReadConfig,
}

impl Apex7 {
    /// Product ID of the Apex 7 the reference capture was taken on.
    pub const PRODUCT_ID: u16 = product_ids::APEX_7;

    /// Create from a generic keyboard.
    pub fn new(keyboard: GenericKeyboard) -> Self {
        Self {
            inner: keyboard,
            #[cfg(feature = "onboard")]
            onboard: ReadConfig::default(),
        }
    }

    /// Whether this device's product ID is on the platform the read was reverse-engineered from.
    #[cfg(feature = "onboard")]
    #[must_use]
    pub fn supports_onboard(&self) -> bool {
        onboard_family(self.inner.info().product_id)
    }

    /// Read plan this device uses when the caller does not pass one.
    #[cfg(feature = "onboard")]
    #[must_use]
    pub fn onboard_config(&self) -> &ReadConfig {
        &self.onboard
    }

    /// Replace the read plan (slot, geometry, pacing, reply framing).
    #[cfg(feature = "onboard")]
    pub fn set_onboard_config(&mut self, config: ReadConfig) {
        self.onboard = config;
    }

    /// The profile collection this device reads on: its own open HID handle, which the read keeps
    /// locked per transfer.
    ///
    /// # Errors
    ///
    /// Returns an error when the device was created without a hidapi handle.
    #[cfg(feature = "onboard")]
    fn profile_handle(&self) -> Result<Arc<Mutex<HidDevice>>> {
        self.inner
            .hid_handle()
            .cloned()
            .ok_or_else(|| Error::DeviceCommunication("Apex 7 has no open HID handle".to_string()))
    }

    /// Reads one on-board profile slot with an explicit plan.
    ///
    /// # Errors
    ///
    /// Returns an error when the plan is invalid, the collection rejects a frame, or it never answers
    /// a payload read.
    #[cfg(feature = "onboard")]
    pub async fn read_profile(&self, config: &ReadConfig) -> Result<ProfileImage> {
        apex_legacy_onboard::read_profile(self.profile_handle()?, config, false).await
    }

    /// Reads the slot this device's [`Apex7::onboard_config`] selects.
    ///
    /// # Errors
    ///
    /// As [`Apex7::read_profile`].
    #[cfg(feature = "onboard")]
    pub async fn read_profile_slot(&self) -> Result<ProfileImage> {
        self.read_profile(&self.onboard.clone()).await
    }
}

// Deref gives the Device trait methods of the wrapped keyboard.
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
    #[cfg(feature = "onboard")]
    fn supports_onboard_profile(&self) -> bool {
        self.supports_onboard()
    }

    #[cfg(feature = "onboard")]
    async fn read_onboard_profile(&self, config: &ReadConfig) -> Result<ProfileImage> {
        self.read_profile(config).await
    }
});

#[cfg(test)]
mod tests {
    use super::*;

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

    #[cfg(feature = "onboard")]
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
            match keyboard.profile_handle() {
                Ok(_) => panic!("a device without a handle must not report a profile collection"),
                Err(error) => assert!(
                    error.to_string().contains("no open HID handle"),
                    "the error names the missing handle: {error}"
                ),
            }
        }

        #[test]
        fn the_device_starts_on_the_captured_read_plan() {
            let keyboard = device_without_handle(product_ids::APEX_7);
            let config = keyboard.onboard_config();
            assert_eq!(config.fs_id(), crate::onboard_profile::PROFILE_FS_BASE);
            assert_eq!(config.total_resource_len(), 12_288);
        }
    }
}
