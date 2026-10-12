//! Keyboard families: which lighting dialect each Apex product ID speaks and how far that
//! dialect is trusted.
//!
//! [`keyboard_profile`] is the one table from product ID to Rust type, lighting family,
//! verification and settings capabilities. `GenericKeyboard` encodes its reports according to
//! the family, so supporting another PID of a known family needs only a row in that table.
//!
//! Families other than [`KeyboardFamily::Legacy`] are `[EXPERIMENTAL]`: their byte layouts come
//! from OpenRGB (`Controllers/SteelSeriesController/`, facts only) and are not tested on hardware
//! by this project. Report layouts live in `hid_reports.rs`.

use crate::devices::hid_reports::{
    APEX_9_DIRECT_REPORT_SIZE, APEX_DIRECT_REPORT_SIZE, APEX_OUTPUT_REPORT_SIZE, ApexDirectCommand, ApexDirectPacket,
    ApexMDirectCommand,
};
use crate::devices::key_mapping::{
    APEX_LED_COUNT, APEX_LED_TABLE, APEX_M750_GRID, KeyId, apex_led_index, apex_led_index_for_hid,
};
use crate::devices::product_ids;
use crate::devices::settings::Verification;
use crate::rgb::Color;

/// Protocol generation of the Apex per-key family (OpenRGB `protocol_quirk`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApexGeneration {
    /// 2019-22 case design: direct frame `0x3A`, no initialisation.
    Gen1,
    /// 2023 USB-C models and the Apex 9 series: direct frame `0x40` / `0x61`, no initialisation.
    Gen2,
    /// Omnipoint 3.0 models and Gen 2 models with updated firmware: `0x4B` initialisation, then
    /// `0x40` / `0x61`.
    Gen3,
}

/// Where the per-key family's feature reports are sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureTarget {
    /// The hidraw node of the opened control interface (OpenRGB opens interface 1, or interface 3
    /// with usage page `0xFFC0` on the 2023 / Gen 3 wireless models).
    ControlInterface,
    /// `GenericKeyboard::send_feature`: interface 3 first, then the opened path. This is the
    /// transport of the Apex Pro TKL 2023 Wireless / Gen 3 path added in PR #290, kept unchanged.
    Interface3,
}

/// Parameters of one Apex per-key model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApexPerKeyProtocol {
    pub generation: ApexGeneration,
    /// Wireless models use direct packet `0x61` once they are Gen 2 or newer.
    pub wireless: bool,
    /// Direct frame length including the report ID.
    pub report_len: usize,
    /// Gen 3 initialisation report length including the report ID.
    pub init_report_len: usize,
    /// Pause after the initialisation report, in milliseconds.
    pub init_settle_ms: u64,
    /// Ask the keyboard for its firmware version when opened: from 1.19.7 the model switches to
    /// Gen 3 (OpenRGB `SendInitialization`, Apex Pro TKL / Apex 9 TKL / Apex 9 Mini only).
    pub firmware_probe: bool,
    pub feature_target: FeatureTarget,
}

impl ApexPerKeyProtocol {
    /// OpenRGB defaults: 643-byte frame, 65-byte init, control interface.
    const fn openrgb(generation: ApexGeneration, wireless: bool) -> Self {
        Self {
            generation,
            wireless,
            report_len: APEX_DIRECT_REPORT_SIZE,
            init_report_len: APEX_OUTPUT_REPORT_SIZE,
            init_settle_ms: 0,
            firmware_probe: false,
            feature_target: FeatureTarget::ControlInterface,
        }
    }

    /// Apex 9 TKL / Apex 9 Mini: 513-byte frame (OpenRGB `direct_packet_length_map`) and the
    /// firmware check.
    const fn apex_9() -> Self {
        let mut protocol = Self::openrgb(ApexGeneration::Gen2, false);
        protocol.report_len = APEX_9_DIRECT_REPORT_SIZE;
        protocol.firmware_probe = true;
        protocol
    }

    /// This crate's Apex Pro TKL 2023 Wireless / Gen 3 path (PR #290): 643-byte init followed by
    /// a 50 ms pause, sent through [`FeatureTarget::Interface3`].
    const fn tkl_2023(wireless: bool) -> Self {
        Self {
            generation: ApexGeneration::Gen3,
            wireless,
            report_len: APEX_DIRECT_REPORT_SIZE,
            init_report_len: APEX_DIRECT_REPORT_SIZE,
            init_settle_ms: 50,
            firmware_probe: false,
            feature_target: FeatureTarget::Interface3,
        }
    }

    /// Direct frame packet ID (OpenRGB `SetLEDsDirect`).
    pub const fn direct_packet(&self) -> ApexDirectPacket {
        match self.generation {
            ApexGeneration::Gen1 => ApexDirectPacket::Gen1,
            _ if self.wireless => ApexDirectPacket::Wireless2023,
            _ => ApexDirectPacket::Wired2023,
        }
    }

    /// Whether the `0x4B` initialisation must precede the first frame.
    pub const fn needs_init(&self) -> bool {
        matches!(self.generation, ApexGeneration::Gen3)
    }
}

/// Lighting dialect of a keyboard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardFamily {
    /// This crate's original dialect: `0x21` zones, `0x22` brightness, `0x09` apply, `0x23`
    /// per-key placeholder, written with an extra leading `0x00`. Used by the Apex Pro TKL (2023)
    /// and by models no reference covers.
    Legacy,
    /// OpenRGB `SteelSeriesApexController`: per-key direct feature report.
    ApexPerKey(ApexPerKeyProtocol),
    /// OpenRGB `SteelSeriesApex8ZoneController` (Apex 3 TKL).
    EightZone,
    /// OpenRGB `SteelSeriesApexTZoneController` (Apex 3).
    TriZone,
    /// OpenRGB `SteelSeriesApexMController` (Apex M750).
    ApexM,
    /// OpenRGB `SteelSeriesOldApexController` (Apex OG / Fnatic / 350).
    OldApex,
}

impl KeyboardFamily {
    /// Short name for logs and docs.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::ApexPerKey(_) => "apex-per-key",
            Self::EightZone => "8-zone",
            Self::TriZone => "tri-zone",
            Self::ApexM => "apex-m",
            Self::OldApex => "old-apex",
        }
    }

    /// Whether lighting is sent as a whole-keyboard [`ApexFrame`].
    pub const fn uses_key_frame(&self) -> bool {
        matches!(self, Self::ApexPerKey(_) | Self::ApexM)
    }

    /// Whether the keyboard has zones only, with no per-key lighting.
    pub const fn is_zone_only(&self) -> bool {
        matches!(self, Self::EightZone | Self::TriZone | Self::OldApex)
    }
}

/// Rust type `DeviceManager::open_keyboard` builds for a PID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardKind {
    /// `GenericKeyboard`, encoding by family.
    Generic,
    /// `apex::Apex3Tkl` around a `GenericKeyboard`.
    Apex3Tkl,
    /// `apex_7::Apex7` around a `GenericKeyboard`: the model whose onboard payload this fork
    /// decodes, so a keybinding written to it becomes one of its own.
    Apex7,
    /// `apex_pro_tkl_2023::ApexProTkl2023` around a `GenericKeyboard`.
    ApexProTkl2023,
}

/// Live actuation frame a model accepts, from a published tool tested on that model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveActuation {
    /// `0x38` frame (apex-web, Apex Pro TKL 2023).
    ApexWeb2023,
    /// `0x31 0x47` frame (apex-control, Apex Pro TKL).
    ApexControlGen1,
}

/// Everything this crate knows about one keyboard PID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyboardProfile {
    pub kind: KeyboardKind,
    pub family: KeyboardFamily,
    /// Trust in the lighting path.
    pub lighting: Verification,
    /// OmniPoint adjustable actuation (Apex Pro models); enables the experimental `0x2D` setting.
    pub actuation: bool,
    pub live_actuation: Option<LiveActuation>,
    /// Live Rapid Tap switch (apex-control, Apex Pro TKL only).
    pub rapid_tap: bool,
}

impl KeyboardProfile {
    const fn new(kind: KeyboardKind, family: KeyboardFamily, lighting: Verification) -> Self {
        Self {
            kind,
            family,
            lighting,
            actuation: false,
            live_actuation: None,
            rapid_tap: false,
        }
    }

    const fn with_actuation(mut self) -> Self {
        self.actuation = true;
        self
    }

    const fn with_live_actuation(mut self, live: LiveActuation) -> Self {
        self.live_actuation = Some(live);
        self
    }

    const fn with_rapid_tap(mut self) -> Self {
        self.rapid_tap = true;
        self
    }
}

/// The PID table.
///
/// Sources: OpenRGB `SteelSeriesControllerDetect.cpp` (which controller each PID gets) and
/// `SteelSeriesApexController.cpp` (`protocol_map`, `direct_packet_length_map`, the firmware
/// check and the wireless packet list). Rows marked `Guess` are models OpenRGB does not detect,
/// placed in the family of their nearest sibling.
pub fn keyboard_profile(product_id: u16) -> KeyboardProfile {
    use ApexGeneration::{Gen1, Gen2, Gen3};
    use KeyboardFamily::{ApexM, ApexPerKey, EightZone, Legacy, OldApex, TriZone};
    use KeyboardKind::{Apex3Tkl, Apex7, ApexProTkl2023, Generic};
    use Verification::{Guess, Hardware, Reference};
    use product_ids::*;

    let openrgb = |generation, wireless| ApexPerKey(ApexPerKeyProtocol::openrgb(generation, wireless));
    match product_id {
        APEX_5 => KeyboardProfile::new(Generic, openrgb(Gen1, false), Reference),
        // The Apex 7 (2023) is the one model this crate can rewrite its own keybindings into:
        // its onboard profile FS object is decoded here, and its lighting
        // stays the per-key family of the OpenRGB reference.
        APEX_7 => KeyboardProfile::new(Apex7, openrgb(Gen1, false), Reference),
        APEX_7_TKL => KeyboardProfile::new(Generic, openrgb(Gen1, false), Reference),
        APEX_PRO => KeyboardProfile::new(Generic, openrgb(Gen1, false), Reference).with_actuation(),
        APEX_PRO_TKL => {
            let mut protocol = ApexPerKeyProtocol::openrgb(Gen1, false);
            protocol.firmware_probe = true;
            KeyboardProfile::new(Generic, ApexPerKey(protocol), Reference)
                .with_actuation()
                .with_live_actuation(LiveActuation::ApexControlGen1)
                .with_rapid_tap()
        }
        // OpenRGB detects 0x1640 as "Apex Pro 3" and applies no Gen 2 / Gen 3 quirk to it.
        APEX_PRO_2024 => KeyboardProfile::new(Generic, openrgb(Gen1, false), Reference).with_actuation(),
        APEX_9_TKL | APEX_9_MINI => KeyboardProfile::new(Generic, ApexPerKey(ApexPerKeyProtocol::apex_9()), Reference),
        // Not detected by OpenRGB. SteelSeriesApexRegions.h lists the Pro Mini SKUs with Gen 2.
        APEX_PRO_MINI => KeyboardProfile::new(Generic, openrgb(Gen2, false), Guess).with_actuation(),
        // Not detected by OpenRGB; treated like the 2023 TKL wireless pair (Gen 3, packet 0x61).
        APEX_PRO_MINI_WIRELESS_DONGLE | APEX_PRO_MINI_WIRELESS => {
            KeyboardProfile::new(Generic, openrgb(Gen3, true), Guess).with_actuation()
        }
        // Not detected by OpenRGB; SteelSeriesApexRegions.h lists the Pro Mini Gen 3 SKU.
        APEX_PRO_MINI_2024 => KeyboardProfile::new(Generic, openrgb(Gen3, false), Guess).with_actuation(),
        // Zone path (0x21), brightness (0x22) and the feature-gated 0x40 frame are documented as
        // tested on this model; actuation stays experimental.
        APEX_PRO_TKL_2023 => KeyboardProfile::new(ApexProTkl2023, Legacy, Hardware)
            .with_actuation()
            .with_live_actuation(LiveActuation::ApexWeb2023),
        APEX_PRO_TKL_2023_WIRELESS
        | APEX_PRO_TKL_2023_WIRELESS_2
        | APEX_PRO_TKL_WIRELESS_2024_DONGLE
        | APEX_PRO_TKL_WIRELESS_2024 => KeyboardProfile::new(
            ApexProTkl2023,
            ApexPerKey(ApexPerKeyProtocol::tkl_2023(true)),
            Reference,
        )
        .with_actuation(),
        APEX_PRO_TKL_2024 => KeyboardProfile::new(
            ApexProTkl2023,
            ApexPerKey(ApexPerKeyProtocol::tkl_2023(false)),
            Reference,
        )
        .with_actuation(),
        APEX_3_TKL => KeyboardProfile::new(Apex3Tkl, EightZone, Reference),
        APEX_3 => KeyboardProfile::new(Generic, TriZone, Reference),
        APEX_M750 => KeyboardProfile::new(Generic, ApexM, Reference),
        APEX_OG | APEX_350 => KeyboardProfile::new(Generic, OldApex, Reference),
        // No reference: Apex 150, Apex 5 (2024), Apex 7 (2024) and unknown PIDs keep the legacy
        // dialect, extrapolated from the Apex Pro TKL (2023).
        _ => KeyboardProfile::new(Generic, Legacy, Guess),
    }
}

/// USB interface OpenRGB opens for PIDs it matches by interface number alone
/// (`REGISTER_HID_DETECTOR_I`) on models outside the `0xFFC0` control-page convention.
pub fn openrgb_control_interface(product_id: u16) -> Option<i32> {
    use product_ids::*;
    match product_id {
        APEX_3 => Some(3),
        APEX_M750 => Some(2),
        APEX_OG | APEX_350 => Some(0),
        _ => None,
    }
}

/// One colour per LED of [`APEX_LED_TABLE`]: the whole-keyboard state of the per-key family.
///
/// The direct frame always carries every LED, as OpenRGB does; a partial update changes some
/// LEDs and resends the frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApexFrame {
    colors: [Color; APEX_LED_COUNT],
}

impl Default for ApexFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl ApexFrame {
    /// All LEDs black.
    pub const fn new() -> Self {
        Self {
            colors: [Color::BLACK; APEX_LED_COUNT],
        }
    }

    /// Set every LED.
    pub fn fill(&mut self, color: Color) {
        self.colors.fill(color);
    }

    /// Set the LED of a HID usage; `false` if no LED has that usage.
    pub fn set_hid(&mut self, hid_code: u8, color: Color) -> bool {
        match apex_led_index_for_hid(hid_code) {
            Some(index) => {
                self.colors[index] = color;
                true
            }
            None => false,
        }
    }

    /// Set the LED of a key; `false` if the key has no LED.
    pub fn set_key(&mut self, key: KeyId, color: Color) -> bool {
        match apex_led_index(key) {
            Some(index) => {
                self.colors[index] = color;
                true
            }
            None => false,
        }
    }

    /// Colour of a key's LED.
    pub fn color_of(&self, key: KeyId) -> Option<Color> {
        apex_led_index(key).map(|index| self.colors[index])
    }

    /// Colours in LED-index order.
    pub fn colors(&self) -> &[Color] {
        &self.colors
    }

    /// Direct frame carrying every LED in LED-index order, clipped to what `report_len` holds.
    pub fn direct_command(&self, packet: ApexDirectPacket, report_len: usize) -> ApexDirectCommand {
        let mut command = ApexDirectCommand::new(packet, report_len);
        let capacity = command.capacity();
        for ((_, hid_code), color) in APEX_LED_TABLE.iter().zip(self.colors.iter()).take(capacity) {
            command.push(*hid_code, *color);
        }
        command
    }

    /// Apex M750 frame: each slot of [`APEX_M750_GRID`] takes its key's colour.
    pub fn m750_command(&self) -> ApexMDirectCommand {
        ApexMDirectCommand {
            slots: APEX_M750_GRID
                .iter()
                .map(|slot| slot.and_then(|key| self.color_of(key)))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceType, device_type_from_product_id};

    /// Every keyboard PID the crate knows, with its expected (kind, packet, verification).
    /// Families and packets follow OpenRGB SteelSeriesControllerDetect.cpp and
    /// SteelSeriesApexController.cpp (`protocol_map`, wireless packet list).
    #[test]
    fn pid_routing_table() {
        use ApexDirectPacket::{Gen1, Wired2023, Wireless2023};
        use KeyboardKind::{Apex3Tkl, Apex7, ApexProTkl2023, Generic};
        use Verification::{Guess, Hardware, Reference};
        use product_ids::*;

        let per_key = |pid| match keyboard_profile(pid).family {
            KeyboardFamily::ApexPerKey(protocol) => Some(protocol),
            _ => None,
        };
        let rows: [(u16, KeyboardKind, Option<ApexDirectPacket>, Verification); 23] = [
            (APEX_PRO, Generic, Some(Gen1), Reference),
            (APEX_7, Apex7, Some(Gen1), Reference),
            (APEX_PRO_TKL, Generic, Some(Gen1), Reference),
            (APEX_7_TKL, Generic, Some(Gen1), Reference),
            (APEX_5, Generic, Some(Gen1), Reference),
            (APEX_PRO_2024, Generic, Some(Gen1), Reference),
            (APEX_9_TKL, Generic, Some(Wired2023), Reference),
            (APEX_9_MINI, Generic, Some(Wired2023), Reference),
            (APEX_PRO_MINI, Generic, Some(Wired2023), Guess),
            (APEX_PRO_MINI_WIRELESS_DONGLE, Generic, Some(Wireless2023), Guess),
            (APEX_PRO_MINI_WIRELESS, Generic, Some(Wireless2023), Guess),
            (APEX_PRO_MINI_2024, Generic, Some(Wired2023), Guess),
            (APEX_PRO_TKL_2023, ApexProTkl2023, None, Hardware),
            (
                APEX_PRO_TKL_2023_WIRELESS,
                ApexProTkl2023,
                Some(Wireless2023),
                Reference,
            ),
            (
                APEX_PRO_TKL_2023_WIRELESS_2,
                ApexProTkl2023,
                Some(Wireless2023),
                Reference,
            ),
            (APEX_PRO_TKL_2024, ApexProTkl2023, Some(Wired2023), Reference),
            (
                APEX_PRO_TKL_WIRELESS_2024_DONGLE,
                ApexProTkl2023,
                Some(Wireless2023),
                Reference,
            ),
            (
                APEX_PRO_TKL_WIRELESS_2024,
                ApexProTkl2023,
                Some(Wireless2023),
                Reference,
            ),
            (APEX_3_TKL, Apex3Tkl, None, Reference),
            (APEX_3, Generic, None, Reference),
            (APEX_M750, Generic, None, Reference),
            (APEX_OG, Generic, None, Reference),
            (APEX_350, Generic, None, Reference),
        ];
        for (pid, kind, packet, verification) in rows {
            let profile = keyboard_profile(pid);
            assert_eq!(profile.kind, kind, "kind of {pid:#06x}");
            assert_eq!(per_key(pid).map(|p| p.direct_packet()), packet, "packet of {pid:#06x}");
            assert_eq!(profile.lighting, verification, "verification of {pid:#06x}");
            assert_eq!(device_type_from_product_id(pid), DeviceType::Keyboard, "{pid:#06x}");
        }

        assert_eq!(keyboard_profile(APEX_3_TKL).family, KeyboardFamily::EightZone);
        assert_eq!(keyboard_profile(APEX_3).family, KeyboardFamily::TriZone);
        assert_eq!(keyboard_profile(APEX_M750).family, KeyboardFamily::ApexM);
        assert_eq!(keyboard_profile(APEX_OG).family, KeyboardFamily::OldApex);
        assert_eq!(keyboard_profile(APEX_350).family, KeyboardFamily::OldApex);
        assert_eq!(keyboard_profile(APEX_PRO_TKL_2023).family, KeyboardFamily::Legacy);
        for pid in [APEX_150, APEX_5_2024, APEX_7_2024, 0xFFFF] {
            let profile = keyboard_profile(pid);
            assert_eq!(profile.family, KeyboardFamily::Legacy, "{pid:#06x}");
            assert_eq!(profile.lighting, Guess, "{pid:#06x}");
        }
    }

    /// OpenRGB `direct_packet_length_map`, the firmware-checked PIDs and `SendInitialization`.
    #[test]
    fn per_key_protocol_parameters() {
        use product_ids::*;
        let protocol = |pid| match keyboard_profile(pid).family {
            KeyboardFamily::ApexPerKey(protocol) => protocol,
            family => panic!("{pid:#06x} is {family:?}"),
        };
        for pid in [APEX_9_TKL, APEX_9_MINI] {
            assert_eq!(protocol(pid).report_len, 513);
            assert!(protocol(pid).firmware_probe);
            assert!(!protocol(pid).needs_init(), "Gen 2 until the firmware says otherwise");
        }
        assert!(protocol(APEX_PRO_TKL).firmware_probe);
        assert!(!protocol(APEX_PRO).firmware_probe);
        assert_eq!(protocol(APEX_PRO).report_len, 643);
        assert_eq!(protocol(APEX_PRO).init_report_len, 65);
        assert_eq!(protocol(APEX_PRO).feature_target, FeatureTarget::ControlInterface);
        assert!(protocol(APEX_PRO_MINI_2024).needs_init());
        assert!(!protocol(APEX_7).needs_init());

        let mut upgraded = protocol(APEX_PRO_TKL);
        assert_eq!(upgraded.direct_packet(), ApexDirectPacket::Gen1);
        upgraded.generation = ApexGeneration::Gen3;
        assert_eq!(upgraded.direct_packet(), ApexDirectPacket::Wired2023);
        assert!(upgraded.needs_init());
    }

    #[test]
    fn capabilities_per_model() {
        use product_ids::*;
        let actuation = [
            APEX_PRO,
            APEX_PRO_TKL,
            APEX_PRO_2024,
            APEX_PRO_MINI,
            APEX_PRO_MINI_WIRELESS_DONGLE,
            APEX_PRO_MINI_WIRELESS,
            APEX_PRO_MINI_2024,
            APEX_PRO_TKL_2023,
            APEX_PRO_TKL_2023_WIRELESS,
            APEX_PRO_TKL_2023_WIRELESS_2,
            APEX_PRO_TKL_2024,
            APEX_PRO_TKL_WIRELESS_2024_DONGLE,
            APEX_PRO_TKL_WIRELESS_2024,
        ];
        for pid in actuation {
            assert!(keyboard_profile(pid).actuation, "{pid:#06x} has OmniPoint switches");
        }
        for pid in [
            APEX_5,
            APEX_7,
            APEX_7_TKL,
            APEX_9_TKL,
            APEX_9_MINI,
            APEX_3,
            APEX_3_TKL,
            APEX_M750,
            APEX_OG,
        ] {
            assert!(!keyboard_profile(pid).actuation, "{pid:#06x}");
        }
        assert_eq!(
            keyboard_profile(APEX_PRO_TKL_2023).live_actuation,
            Some(LiveActuation::ApexWeb2023)
        );
        assert_eq!(
            keyboard_profile(APEX_PRO_TKL).live_actuation,
            Some(LiveActuation::ApexControlGen1)
        );
        assert!(keyboard_profile(APEX_PRO_TKL).rapid_tap);
        let with_live = actuation
            .iter()
            .filter(|&&pid| keyboard_profile(pid).live_actuation.is_some())
            .count();
        assert_eq!(with_live, 2);
    }

    #[test]
    fn openrgb_interfaces() {
        use product_ids::*;
        assert_eq!(openrgb_control_interface(APEX_3), Some(3));
        assert_eq!(openrgb_control_interface(APEX_M750), Some(2));
        assert_eq!(openrgb_control_interface(APEX_OG), Some(0));
        assert_eq!(openrgb_control_interface(APEX_350), Some(0));
        assert_eq!(openrgb_control_interface(APEX_PRO_TKL_2023), None);
    }

    #[test]
    fn frame_holds_every_led_in_order() {
        let mut frame = ApexFrame::new();
        assert!(frame.set_key(KeyId::W, Color::RED));
        assert!(frame.set_hid(0xFB, Color::BLUE));
        assert!(!frame.set_hid(0x65, Color::GREEN), "Menu has no LED");
        assert!(!frame.set_key(KeyId::VolumeWheel, Color::GREEN));
        assert_eq!(frame.color_of(KeyId::W), Some(Color::RED));
        assert_eq!(frame.color_of(KeyId::MediaPlayPause), Some(Color::BLUE));
        assert_eq!(frame.color_of(KeyId::A), Some(Color::BLACK));

        let command = frame.direct_command(ApexDirectPacket::Gen1, APEX_DIRECT_REPORT_SIZE);
        assert_eq!(command.entries.len(), APEX_LED_COUNT);
        for ((entry_hid, color), ((_, hid), frame_color)) in
            command.entries.iter().zip(APEX_LED_TABLE.iter().zip(frame.colors()))
        {
            assert_eq!(entry_hid, hid);
            assert_eq!(color, frame_color);
        }

        let m750 = frame.m750_command();
        assert_eq!(m750.slots.len(), 132);
        assert_eq!(APEX_M750_GRID[68], Some(KeyId::W));
        assert_eq!(m750.slots[68], Some(Color::RED));
        assert_eq!(m750.slots[3], None);
    }
}
