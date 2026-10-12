//! [EXPERIMENTAL] Mouse support (Rival, Sensei, Aerox, Prime, Kana, Kinzu series).
//!
//! Every mouse rivalcfg supports is listed in [`MODELS`]. Each model points at a profile in
//! `profiles.rs`: a data table ported from rivalcfg (`rivalcfg/devices/*.py`, WTFPL, commit
//! f16c521) that says which command each setting sends and how its value is encoded. One
//! implementation, [`SteelSeriesMouse`], drives every model from its table and exposes the
//! uniform settings model (`devices::settings`), so the CLI, daemon and UI need no
//! mouse-specific code.
//!
//! Nothing here has been tested on hardware by this project. Correctness rests on reproducing
//! rivalcfg's own test expectations byte for byte (`handler_tests.rs`, `device_spec_tests.rs`),
//! and every descriptor is marked [`Verification::Reference`](super::settings::Verification).
//!
//! `discovery.rs` asks [`model_for_product_id`] whether a PID is a mouse and which interface to
//! open, then hands the opened HID handle to [`open`].

mod device;
pub mod dpi_tables;
pub mod encode;
pub mod profile;
pub mod profiles;

// Generated from rivalcfg's tests: one case per line reads better than rustfmt's layout.
#[cfg(test)]
#[rustfmt::skip]
mod device_spec_tests;
#[cfg(test)]
mod handler_tests;
#[cfg(test)]
mod tests;

use hidapi::HidDevice;

pub use device::SteelSeriesMouse;
use device::{HidTransport, Transport};
pub use profile::Profile;

use super::settings::Configurable;
use super::{Device, DeviceInfo};
use crate::rgb::Color;
use crate::{Error, Result};

/// Static description of one mouse model.
#[derive(Clone, Copy, Debug)]
pub struct MouseModel {
    pub product_id: u16,
    pub name: &'static str,
    /// USB interface number that carries the vendor control endpoint (rivalcfg's `endpoint`).
    pub interface_number: i32,
    /// Commands and settings of this model.
    pub profile: &'static Profile,
}

/// Look up a mouse model by USB product ID.
pub fn model_for_product_id(product_id: u16) -> Option<&'static MouseModel> {
    MODELS.iter().find(|m| m.product_id == product_id)
}

/// Mouse-specific functionality on top of the uniform settings model.
pub trait Mouse: Device + Configurable {
    fn model(&self) -> &'static MouseModel;

    /// Names of the LED zones [`Mouse::set_zone_colors_direct`] drives, in order. Empty when
    /// the mouse has no addressable LEDs.
    fn color_zone_names(&self) -> Vec<String> {
        Vec::new()
    }

    /// Set every LED zone to a steady color right away, for animations driven by the daemon.
    /// `colors` holds one color per zone of [`Mouse::color_zone_names`], or one color for all.
    fn set_zone_colors_direct(&mut self, _colors: &[Color]) -> Result<()> {
        Err(Error::Unsupported(format!(
            "{} has no directly addressable LED zones",
            self.model().name
        )))
    }
}

/// Wrap an opened HID handle in the right mouse implementation.
pub fn open(info: DeviceInfo, device: HidDevice) -> Result<Box<dyn Mouse>> {
    let model = model_for(&info)?;
    let transport = HidTransport::new(&info, model, device);
    Ok(open_with_transport(info, model, Box::new(transport)))
}

/// The model of a detected device, or [`Error::UnsupportedDevice`].
fn model_for(info: &DeviceInfo) -> Result<&'static MouseModel> {
    model_for_product_id(info.product_id).ok_or(Error::UnsupportedDevice {
        vendor_id: info.vendor_id,
        product_id: info.product_id,
    })
}

fn open_with_transport(info: DeviceInfo, model: &'static MouseModel, transport: Box<dyn Transport>) -> Box<dyn Mouse> {
    Box::new(SteelSeriesMouse::new(info, model, transport))
}

/// Every known mouse model, one entry per USB product ID, generated from rivalcfg's
/// `models` lists. Names drop the "SteelSeries" prefix; "(wired)" and "(2.4 GHz)" mark the two
/// product IDs a dual-mode mouse reports.
pub static MODELS: &[MouseModel] = &[
    MouseModel {
        product_id: 0x1836,
        name: "Aerox 3",
        interface_number: 3,
        profile: &profiles::AEROX_3,
    },
    MouseModel {
        product_id: 0x183A,
        name: "Aerox 3 Wireless (wired)",
        interface_number: 3,
        profile: &profiles::AEROX_3_WIRELESS_WIRED,
    },
    MouseModel {
        product_id: 0x187A,
        name: "Aerox 3 Wireless CS2 Dragon Lore Edition (wired)",
        interface_number: 3,
        profile: &profiles::AEROX_3_WIRELESS_WIRED,
    },
    MouseModel {
        product_id: 0x1838,
        name: "Aerox 3 Wireless (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::AEROX_3_WIRELESS_WIRELESS,
    },
    MouseModel {
        product_id: 0x1878,
        name: "Aerox 3 Wireless CS2 Dragon Lore Edition (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::AEROX_3_WIRELESS_WIRELESS,
    },
    MouseModel {
        product_id: 0x1850,
        name: "Aerox 5",
        interface_number: 3,
        profile: &profiles::AEROX_5,
    },
    MouseModel {
        product_id: 0x1854,
        name: "Aerox 5 Wireless (wired)",
        interface_number: 3,
        profile: &profiles::AEROX_5_WIRELESS_WIRED,
    },
    MouseModel {
        product_id: 0x185E,
        name: "Aerox 5 Wireless Destiny 2 Edition (wired)",
        interface_number: 3,
        profile: &profiles::AEROX_5_WIRELESS_WIRED,
    },
    MouseModel {
        product_id: 0x1862,
        name: "Aerox 5 Wireless Diablo IV Edition (wired)",
        interface_number: 3,
        profile: &profiles::AEROX_5_WIRELESS_WIRED,
    },
    MouseModel {
        product_id: 0x1852,
        name: "Aerox 5 Wireless (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::AEROX_5_WIRELESS_WIRELESS,
    },
    MouseModel {
        product_id: 0x185C,
        name: "Aerox 5 Wireless Destiny 2 Edition (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::AEROX_5_WIRELESS_WIRELESS,
    },
    MouseModel {
        product_id: 0x1860,
        name: "Aerox 5 Wireless Diablo IV Edition (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::AEROX_5_WIRELESS_WIRELESS,
    },
    MouseModel {
        product_id: 0x185A,
        name: "Aerox 9 Wireless (wired)",
        interface_number: 3,
        profile: &profiles::AEROX_9_WIRELESS_WIRED,
    },
    MouseModel {
        product_id: 0x1876,
        name: "Aerox 9 Wireless WOW Edition (wired)",
        interface_number: 3,
        profile: &profiles::AEROX_9_WIRELESS_WIRED,
    },
    MouseModel {
        product_id: 0x1858,
        name: "Aerox 9 Wireless (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::AEROX_9_WIRELESS_WIRELESS,
    },
    MouseModel {
        product_id: 0x1874,
        name: "Aerox 9 Wireless WOW Edition (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::AEROX_9_WIRELESS_WIRELESS,
    },
    MouseModel {
        product_id: 0x137A,
        name: "Kana v2",
        interface_number: 0,
        profile: &profiles::KANA_V2,
    },
    MouseModel {
        product_id: 0x1366,
        name: "Kinzu v2",
        interface_number: 0,
        profile: &profiles::KINZU_V2,
    },
    MouseModel {
        product_id: 0x1378,
        name: "Kinzu v2",
        interface_number: 0,
        profile: &profiles::KINZU_V2,
    },
    MouseModel {
        product_id: 0x182E,
        name: "Prime",
        interface_number: 0,
        profile: &profiles::PRIME,
    },
    MouseModel {
        product_id: 0x182A,
        name: "Prime Rainbow 6 Siege Black Ice Edition",
        interface_number: 0,
        profile: &profiles::PRIME,
    },
    MouseModel {
        product_id: 0x1856,
        name: "Prime CS:GO Neo Noir Edition",
        interface_number: 0,
        profile: &profiles::PRIME,
    },
    MouseModel {
        product_id: 0x184D,
        name: "Prime Mini",
        interface_number: 3,
        profile: &profiles::PRIME_MINI,
    },
    MouseModel {
        product_id: 0x182C,
        name: "Prime+",
        interface_number: 0,
        profile: &profiles::PRIME_PLUS,
    },
    MouseModel {
        product_id: 0x1842,
        name: "Prime Wireless (wired)",
        interface_number: 3,
        profile: &profiles::PRIME_WIRELESS_WIRED,
    },
    MouseModel {
        product_id: 0x184A,
        name: "Prime Mini Wireless (wired)",
        interface_number: 3,
        profile: &profiles::PRIME_WIRELESS_WIRED,
    },
    MouseModel {
        product_id: 0x1840,
        name: "Prime Wireless (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::PRIME_WIRELESS_WIRELESS,
    },
    MouseModel {
        product_id: 0x1848,
        name: "Prime Mini Wireless (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::PRIME_WIRELESS_WIRELESS,
    },
    MouseModel {
        product_id: 0x1824,
        name: "Rival 3",
        interface_number: 3,
        profile: &profiles::RIVAL_3,
    },
    MouseModel {
        product_id: 0x184C,
        name: "Rival 3 (firmware v0.37.0.0)",
        interface_number: 3,
        profile: &profiles::RIVAL_3,
    },
    MouseModel {
        product_id: 0x1870,
        name: "Rival 3 Gen 2",
        interface_number: 3,
        profile: &profiles::RIVAL_3_GEN_2,
    },
    MouseModel {
        product_id: 0x1830,
        name: "Rival 3 Wireless (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::RIVAL_3_WIRELESS,
    },
    MouseModel {
        product_id: 0x1872,
        name: "Rival 3 Wireless Gen 2 (2.4 GHz)",
        interface_number: 3,
        profile: &profiles::RIVAL_3_WIRELESS_GEN_2,
    },
    MouseModel {
        product_id: 0x183C,
        name: "Rival 5",
        interface_number: 0,
        profile: &profiles::RIVAL_5,
    },
    MouseModel {
        product_id: 0x183E,
        name: "Rival 5 Destiny Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_5,
    },
    MouseModel {
        product_id: 0x1706,
        name: "Rival 95",
        interface_number: 0,
        profile: &profiles::RIVAL_95,
    },
    MouseModel {
        product_id: 0x1707,
        name: "Rival 95 MSI Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_95,
    },
    MouseModel {
        product_id: 0x1704,
        name: "Rival 95 PC Bang",
        interface_number: 0,
        profile: &profiles::RIVAL_95,
    },
    MouseModel {
        product_id: 0x1708,
        name: "Rival 100 PC Bang",
        interface_number: 0,
        profile: &profiles::RIVAL_95,
    },
    MouseModel {
        product_id: 0x1702,
        name: "Rival 100",
        interface_number: 0,
        profile: &profiles::RIVAL_100,
    },
    MouseModel {
        product_id: 0x170A,
        name: "Rival 100 (Dell China)",
        interface_number: 0,
        profile: &profiles::RIVAL_100,
    },
    MouseModel {
        product_id: 0x170B,
        name: "Rival 100 Dota 2 Edition (retail)",
        interface_number: 0,
        profile: &profiles::RIVAL_100,
    },
    MouseModel {
        product_id: 0x170C,
        name: "Rival 100 Dota 2 Edition (Lenovo)",
        interface_number: 0,
        profile: &profiles::RIVAL_100,
    },
    MouseModel {
        product_id: 0x1814,
        name: "Rival 105",
        interface_number: 0,
        profile: &profiles::RIVAL_100,
    },
    MouseModel {
        product_id: 0x1729,
        name: "Rival 110",
        interface_number: 0,
        profile: &profiles::RIVAL_110,
    },
    MouseModel {
        product_id: 0x1816,
        name: "Rival 106",
        interface_number: 0,
        profile: &profiles::RIVAL_110,
    },
    MouseModel {
        product_id: 0x1384,
        name: "Rival",
        interface_number: 0,
        profile: &profiles::RIVAL_300,
    },
    MouseModel {
        product_id: 0x1392,
        name: "Rival Dota 2 Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_300,
    },
    MouseModel {
        product_id: 0x1710,
        name: "Rival 300",
        interface_number: 0,
        profile: &profiles::RIVAL_300,
    },
    MouseModel {
        product_id: 0x1712,
        name: "Rival 300 Fallout 4 Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_300,
    },
    MouseModel {
        product_id: 0x171C,
        name: "Rival 300 Evil Geniuses Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_300,
    },
    MouseModel {
        product_id: 0x1394,
        name: "Rival 300 CS:GO Fade Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_300,
    },
    MouseModel {
        product_id: 0x171A,
        name: "Rival 300 CS:GO Hyper Beast Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_300,
    },
    MouseModel {
        product_id: 0x1716,
        name: "Rival 300 CS:GO Fade Edition (stm32)",
        interface_number: 0,
        profile: &profiles::RIVAL_300,
    },
    MouseModel {
        product_id: 0x1714,
        name: "Rival 300 Acer Predator Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_300,
    },
    MouseModel {
        product_id: 0x1718,
        name: "Rival 300 HP OMEN Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_300,
    },
    MouseModel {
        product_id: 0x1810,
        name: "Rival 300S",
        interface_number: 0,
        profile: &profiles::RIVAL_300S,
    },
    MouseModel {
        product_id: 0x1720,
        name: "Rival 310",
        interface_number: 0,
        profile: &profiles::RIVAL_310,
    },
    MouseModel {
        product_id: 0x171E,
        name: "Rival 310 CS:GO Howl Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_310,
    },
    MouseModel {
        product_id: 0x1736,
        name: "Rival 310 PUBG Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_310,
    },
    MouseModel {
        product_id: 0x170E,
        name: "Rival 500",
        interface_number: 0,
        profile: &profiles::RIVAL_500,
    },
    MouseModel {
        product_id: 0x1724,
        name: "Rival 600",
        interface_number: 0,
        profile: &profiles::RIVAL_600,
    },
    MouseModel {
        product_id: 0x172E,
        name: "Rival 600 Dota 2 Edition",
        interface_number: 0,
        profile: &profiles::RIVAL_600,
    },
    MouseModel {
        product_id: 0x172B,
        name: "Rival 650 Wireless (wired)",
        interface_number: 0,
        profile: &profiles::RIVAL_650,
    },
    MouseModel {
        product_id: 0x1726,
        name: "Rival 650 Wireless (2.4 GHz)",
        interface_number: 0,
        profile: &profiles::RIVAL_650,
    },
    MouseModel {
        product_id: 0x1700,
        name: "Rival 700",
        interface_number: 0,
        profile: &profiles::RIVAL_700,
    },
    MouseModel {
        product_id: 0x1730,
        name: "Rival 710",
        interface_number: 0,
        profile: &profiles::RIVAL_700,
    },
    MouseModel {
        product_id: 0x1722,
        name: "Sensei 310",
        interface_number: 0,
        profile: &profiles::SENSEI_310,
    },
    MouseModel {
        product_id: 0x1369,
        name: "Sensei [RAW]",
        interface_number: 0,
        profile: &profiles::SENSEI_RAW,
    },
    MouseModel {
        product_id: 0x1362,
        name: "Sensei [RAW] Diablo III Edition",
        interface_number: 0,
        profile: &profiles::SENSEI_RAW,
    },
    MouseModel {
        product_id: 0x136D,
        name: "Sensei [RAW] Guild Wars 2 Edition",
        interface_number: 0,
        profile: &profiles::SENSEI_RAW,
    },
    MouseModel {
        product_id: 0x136F,
        name: "Sensei [RAW] CoD Black Ops II Edition",
        interface_number: 0,
        profile: &profiles::SENSEI_RAW,
    },
    MouseModel {
        product_id: 0x1380,
        name: "Sensei [RAW] World of Tanks Edition",
        interface_number: 0,
        profile: &profiles::SENSEI_RAW,
    },
    MouseModel {
        product_id: 0x1390,
        name: "Sensei [RAW] Heroes of the Storm Edition",
        interface_number: 0,
        profile: &profiles::SENSEI_RAW,
    },
    MouseModel {
        product_id: 0x1832,
        name: "Sensei TEN",
        interface_number: 0,
        profile: &profiles::SENSEI_TEN,
    },
    MouseModel {
        product_id: 0x1834,
        name: "Sensei TEN CS:GO Neon Rider Edition",
        interface_number: 0,
        profile: &profiles::SENSEI_TEN,
    },
];
