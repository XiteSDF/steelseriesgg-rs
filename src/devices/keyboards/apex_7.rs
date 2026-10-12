//! The Apex 7 (2023): the Apex whose onboard profile this fork has decoded, bindings included.
//!
//! Onboard profiles are a family feature, not this model's alone: the engine's own device
//! descriptors carry the same `profile_1`..`profile_5`, `onboard_config_count` and
//! `profile_size` fields for the wider legacy Apex family (`base_apex_legacy.lisp:1553`, `:1609`),
//! so other Apex keyboards are addressed the same way. What is established here is the Apex 7's
//! payload and its write recipe and nothing else, since the captures are Apex 7 captures and no
//! other model has been put on the wire from this tree. The scope of this module is therefore
//! Apex 7: the other models have no decoded payload to write, so they stay on the engine's path,
//! and covering them means capturing one of them rather than assuming this one's layout fits.
//!
//! For the models this module does not cover, keybindings are imposed on the *wire*: the input
//! engine grabs the device with `EVIOCGRAB` and replays it through uinput (`src/input/linux.rs`),
//! so a `BindingSet` never reaches the device. On the Apex 7 the engine instead serialises a whole
//! profile - bindings, macros, lighting, OLED, actuation - into one payload and rewrites an
//! onboard FS object, so its bindings become the device's own and survive with no engine running.
//! This is the only place in this fork a `BindingSet` has been turned into device-side bytes,
//! which is a statement about what has been decoded here and not about what devices can do.
//!
//! The payload and its frames are pinned by the captures (`PACKET.md` §9; `tools/onboard.py`
//! reproduces all 159 frames of the three captures byte for byte) and cross-read in the engine's
//! device descriptors (`apex_7.lisp:37-105` is the payload's field list, `:235-267` its write
//! recipe, `base_apex_legacy.lisp:186-219` the table that numbers the binding slots).
//!
//! # Shape
//!
//! - The payload is 11648 octets for the release schema `0x000A` and 11696 for `0x000B`, which
//!   adds the dual-actuation block: one `rapid_tap_enable` octet at 2025 and ten five-octet
//!   `rapid_tap_pairs` at 2026-2075, and drops the three `crc_padding` octets schema A has. This
//!   module writes schema B, because that is what the captures show.
//! - The crc covers the payload without its own four trailing octets, which is the same value as
//!   the crc of a copy whose last four octets are zero: the walk is four-octet aligned and the
//!   payload ends with the crc.
//! - Bindings are 128 five-octet records at 85, in the device's own slot order: a slot is not a
//!   HID usage, it is `HID_SLOT_TABLE[usage]`, and the eight slots the table never names (6, 7,
//!   29, 69, 102, 103, 110, 111) get no record. The captures show the numbering byte for byte:
//!   slot 0 carries `51 29` (space), slot 1 `51 35`, slot 2 `51 2b`, slot 4 `51 e1`, slot 5 `51
//!   e0`, and the table sends those usages to those slots.
//! - Every slot the table names carries a record in the captures, unbound ones included: a bare
//!   key bound to itself, `51 <its own usage>`. A zero record means the deactivating button
//!   (`function` 0x00 of the `key_mapping` struct), so an unbound slot has to be written as its
//!   identity or the key stops typing.
//! - The wheel's two usages (0x6A/0x6B) get no slot; the wheel has its own two-record layer at
//!   725, and the captures find it at rest bound to two media keys, `61 e9` and `61 ea`. Those
//!   two octets are a vendor numbering this fork has no table for, so they are copied as the
//!   payload's default and a `MediaKey` is never compiled into one.
//! - The two meta layers (735 and 1375) are the same bindings with the SteelSeries key (usage
//!   0xF0) held. The engine's binding model has no source key for it, because evdev invents no
//!   code for the SS key, so this module preserves those layers and zeroes them only in a payload
//!   it built itself from nothing.
//! - A payload's name is the 24 ASCII octets at 2, left-justified and zero-padded (`Config 1`
//!   fills 8 of them and one capture set renames a profile to `Config Main`, `PACKET.md` §6b);
//!   the engine's default spelling is the slot's number, and the device shows this name on its
//!   OLED, so a rewrite carries a name rather than inventing one.
//! - The FS object's size never changes with the payload: it is `profile_size` = `0xA000` = 40
//!   blocks of 1024 whatever the payload's size, so a short payload is zero-padded to it.
//!
//! # Wire
//!
//! Upload is 122 frames, and the frame count is geometry rather than policy: `1 + 3 * 40 + 1` =
//! erase, then per block two `ConfigWrite` writes of 512 octets (offsets 0 and 512, sent as a
//! 643-octet feature report, since a fixed-size report cannot carry 512 octets) plus one push of
//! the whole block, then ask the device to validate the profile it now holds, one-based. The
//! block count is `profile_size / 1024` and independent of the payload, which is why both
//! schemas cost the same 122 frames.
//!
//! A rewrite is not a deploy: the device keeps running whatever profile it had selected, so the
//! frames that make the new one live (`89 01`, `41 00`, `74 00 01 00` - `PACKET.md` §9, deploy
//! tail) are a
//! separate step, and [`Self::deploy_tail`] returns them without sending anything.

use super::{GenericKeyboard, Keyboard};
use crate::devices::hid_reports::{
    FS_BLOCK, FS_WRITE_MAX, FS_WRITE_REPORT_SIZE, FsCommitCommand, FsEraseCommand, FsValidateCommand, FsWriteCommand,
    HidCommand, HidDeviceType, KEYBOARD_REPORT_SIZE, stm32crc,
};
use crate::devices::key_mapping::{APEX_LED_TABLE, KeyAddress, KeyId, KeyMapping};
use crate::devices::product_ids;
use crate::devices::settings::{
    ChoiceOption, Configurable, SettingDescriptor, SettingKind, SettingValue, Verification,
};
use crate::devices::zone_mapping::{ZoneEffect, ZoneMapping};
use crate::devices::{Device, DeviceInfo, DeviceType};
use crate::input::{Action, Binding, BindingSet, InputKey};
use crate::rgb::{Color, PerKeyEffect};
use crate::{Error, Result};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

/// The payload of one onboard profile, as the captures show it: schema `0x000B`, with the
/// dual-actuation block (`apex_7.lisp:107`).
pub const PROFILE_PAYLOAD_LEN: usize = 11696;

/// The payload of the release schema `0x000A` (`apex_7.lisp:33`): forty-eight octets shorter than
/// the live one, since it has a three-octet `crc_padding` and no `rapid_tap` block. This module
/// reads the length and refuses to write it, because its layout has never been seen on the wire
/// here.
pub const PROFILE_PAYLOAD_A_LEN: usize = 11648;

/// The size of the FS object a profile is written into, whatever the payload's size: the engine's
/// `profile_size`, 40 blocks of 1024.
pub const PROFILE_PADDED_LEN: usize = 0xA000;

/// The FS block the device receives at a time (`onboard-profile-resource-buffer-size`) and the
/// most data octets one write chunk carries (its `onboard-profile-chunk-size`).
pub const PROFILE_BLOCK: usize = FS_BLOCK;
pub const PROFILE_CHUNK: usize = FS_WRITE_MAX;

/// How many blocks, and therefore how many frames, one upload costs: both schemas pay the same
/// 122, because the object's size sets the count and not the payload's.
pub const PROFILE_BLOCKS: usize = PROFILE_PADDED_LEN / PROFILE_BLOCK;
pub const PROFILE_FRAMES: usize = 1 + 3 * PROFILE_BLOCKS + 1;

/// The schema this module writes.
pub const PROFILE_SCHEMA: u16 = 0x000B;

/// Every field offset of the payload, in the schema-B order `apex_7.lisp:72-105` states.
pub const PROFILE_SCHEMA_OFF: usize = 0;
pub const PROFILE_NAME_OFF: usize = 2;
pub const PROFILE_NAME_LEN: usize = 24;
pub const PROFILE_LIGHTING_OFF: usize = 27;
pub const PROFILE_LIGHTING_LEN: usize = 57;
pub const PROFILE_META_TOGGLE_OFF: usize = 84;
pub const PROFILE_BUTTON_OFF: usize = 85;
pub const PROFILE_WHEEL_OFF: usize = 725;
pub const PROFILE_BUTTON_META_OFF: usize = 735;
pub const PROFILE_WHEEL_META_OFF: usize = 1375;
pub const PROFILE_RAPID_TAP_ENABLE_OFF: usize = 2025;
pub const PROFILE_RAPID_TAP_PAIRS_OFF: usize = 2026;
pub const PROFILE_RAPID_TAP_PAIRS_LEN: usize = 50;
pub const PROFILE_MACRO_OFF: usize = 2076;
pub const PROFILE_MACRO_LEN: usize = 9600;
pub const PROFILE_GUID_OFF: usize = 11676;
pub const PROFILE_CRC_OFF: usize = PROFILE_PAYLOAD_LEN - 4;

/// One mapping record and the layers built from them: 128 slots for the keys and two for the
/// wheel, four key codes per record.
pub const PROFILE_MAPPING_LEN: usize = 5;
pub const PROFILE_MAPPING_KEYS: usize = 4;
pub const PROFILE_BUTTON_SLOTS: usize = 128;
pub const PROFILE_WHEEL_SLOTS: usize = 2;
pub const PROFILE_BUTTON_LAYER_LEN: usize = PROFILE_BUTTON_SLOTS * PROFILE_MAPPING_LEN;
pub const PROFILE_WHEEL_LAYER_LEN: usize = PROFILE_WHEEL_SLOTS * PROFILE_MAPPING_LEN;

/// The `function` octet of a record: what kind of action the key bound to this slot performs,
/// from the `key_mapping` struct's `values` (`base_apex_legacy.lisp:320-356`).
pub const FUNCTION_FIRST_BUTTON: u8 = 0x01;
pub const FUNCTION_COMBO: u8 = 0x51;
pub const FUNCTION_MEDIA: u8 = 0x61;
pub const FUNCTION_META: u8 = 0x62;
pub const FUNCTION_MACRO: u8 = 0x71;
pub const FUNCTION_DEACTIVATED: u8 = 0x00;

/// The wheel layer as the captures find it at rest: the wheel's two scroll directions bound to
/// two media keys, in a vendor numbering this fork has no table for.
pub const WHEEL_DEFAULT: [u8; PROFILE_WHEEL_LAYER_LEN] = [FUNCTION_MEDIA, 0xE9, 0, 0, 0, FUNCTION_MEDIA, 0xEA, 0, 0, 0];

/// The SteelSeries key's own HID usage (`key_mapping.rs:389`), which is what byte 84 of the
/// captures names as the profile's `meta_toggle_hid`.
pub const SS_KEY_USAGE: u8 = 0xF0;

/// The slot of one HID usage: the engine's own `device-offset-lookup-table`
/// (`base_apex_legacy.lisp:186-219`), whose index is a HID usage in the page-folded numbering
/// `APEX_LED_TABLE` and `KeyAddress::hid_code` use (0xE1 is left shift, 0xF0 the SS key) and
/// whose value is the slot that key's bindings live in. `-1` marks a usage with no slot.
/// The captures confirm the numbering byte for byte: slot 0 carries `51 29` (space), slot 1
/// `51 35`, slot 2 `51 2b`, slot 4 `51 e1`, slot 5 `51 e0`.
pub const HID_SLOT_TABLE: [i8; 256] = [
    -1, -1, -1, -1, 11, 52, 36, 27, 26, 35, 43, 51, 66, 59, 67, 75, 68, 60, 74, 82, 10, 34, 19, 42, 58, 44, 18, 28, 50,
    20, 9, 17, 25, 33, 41, 49, 57, 65, 73, 81, 107, 0, 113, 2, 45, 89, 97, 90, 98, 106, 99, 83, 91, 1, 76, 84, 92, 3,
    8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 88, 96, 104, 112, 14, 114, 22, 30, 115, 70, 78, 118, 109, 117, 116, 38, 46,
    54, 62, 23, 95, 63, 71, 79, 31, 39, 47, 86, 94, 15, 119, 87, 12, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 55, -1, 100, 61, 105, 53, 37, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 5, 4, 21, 13,
    101, 108, 77, 85, -1, -1, -1, -1, -1, -1, -1, -1, 93, 120, 121, 122, 123, 124, 125, 126, 127, -1, -1, -1, -1, -1,
    -1, -1,
];

/// The slots the table never names: they get no record, so a payload that writes one is
/// describing a key the device has no slot for.
pub const HID_SLOT_GAPS: [usize; 8] = [6, 7, 29, 69, 102, 103, 110, 111];

/// The FS object of one onboard slot, and the lighting map that sits beside it (`PACKET.md` §9,
/// FS objects: the object ids and their order are confirmed by all three captures).
pub const FS_PROFILES_BASE: u16 = 0x0201;
pub const FS_LIGHTING_MAP: u16 = 0x0301;

/// The onboard slots a keybinding rewrite may target: the engine's `onboard_config_count` of
/// five (`base_apex_legacy.lisp:178`), and the captures answer that the device runs `Config 1`.
pub const PROFILE_SLOTS: [u8; 5] = [1, 2, 3, 4, 5];

/// One frame of an upload: its wire bytes, and whether they go out as a feature report. The two
/// write halves of a block are feature reports, since a fixed-size report cannot carry 512 data
/// octets; every other frame of the sequence is an ordinary write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsFrame {
    pub report: Vec<u8>,
    pub feature: bool,
}

impl FsFrame {
    /// The bytes to hand to the device: a feature report carries its own `0x00` report octet and
    /// goes out whole, an ordinary frame is the body, which `send_raw` pads behind its report
    /// octet.
    pub fn body(&self) -> Vec<u8> {
        if self.feature {
            return self.report.clone();
        }
        return self.report[1..self.report.len()].to_vec();
    }
}

/// The four mapping layers of one payload, so a caller can change one and keep the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingLayers {
    pub button: Vec<u8>,
    pub wheel: Vec<u8>,
    pub button_meta: Vec<u8>,
    pub wheel_meta: Vec<u8>,
}

/// The Apex 7 (2023): a `GenericKeyboard` plus the onboard payloads this device holds.
///
/// `payloads` is the invariant the whole file rests on: each onboard slot the device has is
/// keyed to the 2560-octet payload last seen there, so any payload read back for one of
/// [`Self::PROFILE_SLOTS`] is a payload that can be edited and written back. `names` is what a
/// slot's payload is called: `Config {slot}` is the engine's default, and a slot can carry any
/// ASCII name that fits the field.
pub struct Apex7 {
    inner: GenericKeyboard,
    payloads: BTreeMap<u8, Vec<u8>>,
    names: BTreeMap<u8, String>,
    bindings_slot: u8,
}

impl Apex7 {
    /// Product ID for Apex 7.
    pub const PRODUCT_ID: u16 = product_ids::APEX_7;

    /// Which onboard slot [`Self::set_key_bindings`] writes. Slots are one-based, and the
    /// captures show the engine rewrote `Config 1`, the first slot, every time.
    pub const DEFAULT_BINDINGS_SLOT: u8 = 1;

    pub fn new(keyboard: GenericKeyboard) -> Self {
        Self {
            inner: keyboard,
            payloads: BTreeMap::new(),
            names: BTreeMap::new(),
            bindings_slot: Apex7::DEFAULT_BINDINGS_SLOT,
        }
    }

    /// Are the five slots the device offers addressed one-based?
    pub fn valid_slot(&self, slot: u8) -> bool {
        return slot >= 1 && slot <= 5;
    }

    /// Which slot a keybinding rewrite goes to.
    pub fn bindings_slot(&self) -> u8 {
        return self.bindings_slot;
    }

    /// Point a keybinding rewrite at another slot, so a binding can be pushed to a profile the
    /// device already holds instead of the running one.
    pub fn select_bindings_slot(&mut self, slot: u8) -> Result<()> {
        if !self.valid_slot(slot) {
            return Err(Error::InvalidConfig(
                "onboard profiles are numbered one-based, so a slot is 1 to 5, not any other byte".to_string(),
            ));
        }
        self.bindings_slot = slot;
        return Ok(());
    }

    /// Which slot one `onboard-slot` choice id names: this fork's ids for the choice are the
    /// slot numbers themselves.
    pub fn slot_of_choice(&self, text: &str) -> Option<u8> {
        for slot in PROFILE_SLOTS {
            if text == slot.to_string().as_str() {
                return Some(slot);
            }
        }
        return None;
    }

    /// The name one slot's payload carries, which is what the device shows on its OLED. `Config
    /// {slot}` is the engine's own default spelling, the one the captures show (`Config 1` at
    /// payload offset 2), and [`Self::set_payload_name`] replaces it for a slot.
    pub fn payload_name(&self, slot: u8) -> String {
        match self.names.get(&slot) {
            Some(name) => {
                return name.clone();
            }
            None => {
                return format!("Config {}", slot);
            }
        }
    }

    /// Does a name fit the payload's name field? The field is the 24 octets at 2, which the
    /// captures show left-justified and zero-padded, and it holds ASCII: the captures spell the
    /// default `Config 1` (8 octets) and one capture set renames the profile to `Config Main`
    /// (11), so a name is a free-form string rather than a `Config <number>` pattern, and an
    /// octet outside ASCII is not something a name field has ever been seen to hold.
    pub fn valid_name(&self, name: &str) -> bool {
        return name.is_ascii() && name.len() >= 1 && name.len() <= PROFILE_NAME_LEN;
    }

    /// The reason a name does not fit the field, if it does not.
    pub fn name_rule(&self, name: &str) -> Result<()> {
        if !self.valid_name(name) {
            return Err(Error::InvalidConfig(format!(
                "a payload name is 1 to {} ASCII octets, the width of the payload's name field: a longer name is cut off by the field, and a non-ASCII name has never been seen in one",
                PROFILE_NAME_LEN
            )));
        }
        return Ok(());
    }

    /// Name one slot's payload, so a rewrite carries the profile's own name instead of the
    /// default spelling. A name that does not fit the field is refused with the reason.
    pub fn set_payload_name(&mut self, slot: u8, name: &str) -> Result<()> {
        if !self.valid_slot(slot) {
            return Err(Error::InvalidConfig(
                "onboard profiles are numbered one-based, so a slot is 1 to 5, not any other byte".to_string(),
            ));
        }
        match self.name_rule(name) {
            Err(err) => {
                return Err(err);
            }
            Ok(_) => {}
        }
        self.names.insert(slot, name.to_string());
        return Ok(());
    }

    /// Forget one slot's name, so it goes back to the default spelling.
    pub fn forget_payload_name(&mut self, slot: u8) {
        self.names.remove(&slot);
    }

    // === The payload ===

    /// What this instance knows of one slot's payload, if anything. A payload is remembered so a
    /// bindings rewrite can keep the macro, lighting and OLED layers a `BindingSet` says nothing
    /// about.
    pub fn payload_of(&self, slot: u8) -> Option<Vec<u8>> {
        match self.payloads.get(&slot) {
            Some(payload) => {
                return Some(payload.clone());
            }
            None => {
                return None;
            }
        }
    }

    /// Forget one slot's remembered payload.
    pub fn forget_payload(&mut self, slot: u8) {
        self.payloads.remove(&slot);
    }

    /// A payload of the right length with nothing in it, except what the captures find in every
    /// payload: this fork's schema, a name, the wheel at rest, the SS key as the profile's meta
    /// toggle, the captures' guid, and identity bindings in every slot the table names. Building
    /// one is the only honest "nothing remembered yet" state. The name is written into the field
    /// left-justified and zero-padded, and is truncated to the field if it is longer than it:
    /// [`Self::named_payload`] is the checked entry point that refuses such a name.
    pub fn bare_payload(&self, name: &str) -> Vec<u8> {
        let mut payload = vec![0u8; PROFILE_PAYLOAD_LEN];
        let schema = PROFILE_SCHEMA.to_le_bytes();
        payload[PROFILE_SCHEMA_OFF] = schema[0];
        payload[PROFILE_SCHEMA_OFF + 1] = schema[1];
        let raw = name.as_bytes();
        let taken = PROFILE_NAME_LEN.min(raw.len());
        for index in 0..taken {
            payload[PROFILE_NAME_OFF + index] = raw[index];
        }
        payload[PROFILE_META_TOGGLE_OFF] = SS_KEY_USAGE;
        for index in 0..WHEEL_DEFAULT.len() {
            payload[PROFILE_WHEEL_OFF + index] = WHEEL_DEFAULT[index];
        }
        payload[PROFILE_GUID_OFF + 15] = 1;
        for slot in 0..PROFILE_BUTTON_SLOTS {
            match self.usage_of_slot(slot) {
                Some(usage) => {
                    let record = self.identity_record(usage);
                    for index in 0..PROFILE_MAPPING_LEN {
                        payload[PROFILE_BUTTON_OFF + PROFILE_MAPPING_LEN * slot + index] = record[index];
                    }
                }
                None => {}
            }
        }
        return payload;
    }

    /// A payload this module built itself from nothing, under a name the caller may use: any ASCII
    /// string that fits the payload's name field.
    pub fn named_payload(&self, name: &str) -> Result<Vec<u8>> {
        match self.name_rule(name) {
            Err(err) => {
                return Err(err);
            }
            Ok(_) => {}
        }
        return Ok(self.bare_payload(name));
    }

    /// The payload with its crc in: the crc covers the payload without its own four octets, which
    /// is the crc of a copy whose last four octets are zero (`apex_7.lisp:243`).
    pub fn seal_payload(&self, payload: Vec<u8>) -> Vec<u8> {
        if payload.len() != PROFILE_PAYLOAD_LEN {
            return payload.clone();
        }
        let mut body = payload.clone();
        for index in (PROFILE_PAYLOAD_LEN - 4)..PROFILE_PAYLOAD_LEN {
            body[index] = 0;
        }
        let crc = stm32crc(body);
        let mut sealed = payload.clone();
        let bytes = crc.to_le_bytes();
        for index in 0..4 {
            sealed[PROFILE_CRC_OFF + index] = bytes[index];
        }
        return sealed;
    }

    /// Does a payload's own crc agree with its content? Reading one back is only trustworthy if
    /// it does.
    pub fn payload_intact(&self, payload: Vec<u8>) -> bool {
        if payload.len() != PROFILE_PAYLOAD_LEN {
            return false;
        }
        let stored = u32::from_le_bytes([
            payload[PROFILE_CRC_OFF],
            payload[PROFILE_CRC_OFF + 1],
            payload[PROFILE_CRC_OFF + 2],
            payload[PROFILE_CRC_OFF + 3],
        ]);
        let sealed = self.seal_payload(payload);
        let expected = u32::from_le_bytes([
            sealed[PROFILE_CRC_OFF],
            sealed[PROFILE_CRC_OFF + 1],
            sealed[PROFILE_CRC_OFF + 2],
            sealed[PROFILE_CRC_OFF + 3],
        ]);
        return stored == expected;
    }

    /// Read a payload's four mapping layers out, so a caller can change one and put them back.
    pub fn layers_of(&self, payload: Vec<u8>) -> Option<MappingLayers> {
        if payload.len() != PROFILE_PAYLOAD_LEN {
            return None;
        }
        return Some(MappingLayers {
            button: payload[PROFILE_BUTTON_OFF..(PROFILE_BUTTON_OFF + PROFILE_BUTTON_LAYER_LEN)].to_vec(),
            wheel: payload[PROFILE_WHEEL_OFF..(PROFILE_WHEEL_OFF + PROFILE_WHEEL_LAYER_LEN)].to_vec(),
            button_meta: payload[PROFILE_BUTTON_META_OFF..(PROFILE_BUTTON_META_OFF + PROFILE_BUTTON_LAYER_LEN)]
                .to_vec(),
            wheel_meta: payload[PROFILE_WHEEL_META_OFF..(PROFILE_WHEEL_META_OFF + PROFILE_WHEEL_LAYER_LEN)].to_vec(),
        });
    }

    /// Write mapping layers back over a payload, leaving every other field alone.
    pub fn put_layers(&self, payload: Vec<u8>, layers: MappingLayers) -> Vec<u8> {
        if payload.len() != PROFILE_PAYLOAD_LEN {
            return payload.clone();
        }

        let given = vec![
            (PROFILE_BUTTON_OFF, PROFILE_BUTTON_LAYER_LEN, layers.button.clone()),
            (PROFILE_WHEEL_OFF, PROFILE_WHEEL_LAYER_LEN, layers.wheel.clone()),
            (
                PROFILE_BUTTON_META_OFF,
                PROFILE_BUTTON_LAYER_LEN,
                layers.button_meta.clone(),
            ),
            (
                PROFILE_WHEEL_META_OFF,
                PROFILE_WHEEL_LAYER_LEN,
                layers.wheel_meta.clone(),
            ),
        ];
        let mut out = payload.clone();
        for (offset, width, layer) in given {
            if layer.len() != width {
                continue;
            }
            for octet in 0..width {
                out[offset + octet] = layer[octet];
            }
        }
        return out;
    }

    // === Slots and records ===

    /// Which slot one HID usage's bindings live in, `None` for the eight usages the table declines
    /// and for every usage above the keys it names (the wheel's own 0x6A/0x6B among them).
    pub fn slot_of_usage(&self, usage: u8) -> Option<usize> {
        let named = HID_SLOT_TABLE[usize::from(usage)];
        if named < 0 {
            return None;
        }
        return Some(usize::try_from(named).unwrap_or(PROFILE_BUTTON_SLOTS));
    }

    /// Which HID usage one slot describes, the inverse of [`Self::slot_of_usage`].
    pub fn usage_of_slot(&self, slot: usize) -> Option<u8> {
        if slot >= PROFILE_BUTTON_SLOTS {
            return None;
        }
        let want = i8::try_from(slot).unwrap_or(-1);
        for usage in 0..256 {
            if HID_SLOT_TABLE[usage] == want {
                return Some(u8::try_from(usage).unwrap_or(0));
            }
        }
        return None;
    }

    /// The HID usage of a key this fork knows, through its [`KeyId`]: a key whose name has no
    /// entry in `APEX_LED_TABLE` (a laptop-only key, a media function) has no usage, and so no
    /// slot to write its bindings into.
    pub fn usage_of_key(&self, key: InputKey) -> Option<u8> {
        for pair in APEX_LED_TABLE {
            match InputKey::from_key_id(pair.0) {
                Some(named) => {
                    if named == key {
                        return Some(pair.1);
                    }
                }
                None => {}
            }
        }
        return None;
    }

    /// The key a HID usage names, for reading a payload's bindings back.
    pub fn key_of_usage(&self, usage: u8) -> Option<InputKey> {
        for pair in APEX_LED_TABLE {
            if pair.1 == usage {
                return InputKey::from_key_id(pair.0);
            }
        }
        return None;
    }

    /// The slot of an engine key, or `None` when the key has no bindings of its own to put.
    pub fn slot_of_key(&self, key: InputKey) -> Option<usize> {
        match self.usage_of_key(key) {
            Some(usage) => {
                return self.slot_of_usage(usage);
            }
            None => {
                return None;
            }
        }
    }

    /// The key one slot carries its bindings under.
    pub fn key_of_slot(&self, slot: usize) -> Option<InputKey> {
        match self.usage_of_slot(slot) {
            Some(usage) => {
                return self.key_of_usage(usage);
            }
            None => {
                return None;
            }
        }
    }

    /// The record of one slot inside a button layer, zero when the layer is short.
    pub fn record_of(&self, layer: &[u8], slot: usize) -> Vec<u8> {
        let want = PROFILE_MAPPING_LEN * (slot + 1);
        if slot >= PROFILE_BUTTON_SLOTS || layer.len() < want {
            return vec![0; PROFILE_MAPPING_LEN];
        }
        return layer[(PROFILE_MAPPING_LEN * slot)..want].to_vec();
    }

    /// A slot's own key bound to itself: pressing it emits a combo of itself, which is what an
    /// unbound key must do, because a zero record is the deactivating button instead.
    pub fn identity_record(&self, usage: u8) -> Vec<u8> {
        return vec![FUNCTION_COMBO, usage, 0, 0, 0];
    }

    /// Is a record a slot bound to its own key, i.e. an unbound key?
    pub fn identity_record_for(&self, record: &[u8], usage: u8) -> bool {
        return self.identity_record(usage) == record;
    }

    /// The action's keys a record names, as far as its `function` octet says they are.
    pub fn combo_of_record(&self, record: &[u8]) -> Vec<InputKey> {
        let mut out = Vec::new();
        if record.len() != PROFILE_MAPPING_LEN {
            return out;
        }
        if record[0] == FUNCTION_MEDIA || record[0] == FUNCTION_MACRO {
            return out;
        }
        for index in 1..PROFILE_MAPPING_LEN {
            if record[index] == 0 {
                continue;
            }
            match self.key_of_usage(record[index]) {
                Some(key) => {
                    out.push(key);
                }
                None => {}
            }
        }
        return out;
    }

    /// The bindings a payload carries, read back as the engine's model: the device's own
    /// bindings as a `BindingSet`. An identity record is an unbound key, and a `BindingSet`
    /// spells unbound keys by leaving them out, so it is skipped.
    pub fn bindings_of_payload(&self, payload: Vec<u8>) -> BindingSet {
        let mut out = BindingSet::new();
        match self.layers_of(payload) {
            Some(layers) => {
                for slot in 0..PROFILE_BUTTON_SLOTS {
                    match self.key_of_slot(slot) {
                        Some(source) => {
                            let record = self.record_of(&layers.button, slot);
                            let skip = match self.usage_of_key(source) {
                                Some(usage) => self.identity_record_for(&record, usage),
                                None => false,
                            };
                            if skip {
                                continue;
                            }
                            out.bindings
                                .push(Binding::new(source, Action::Combo(self.combo_of_record(&record))));
                        }
                        None => {}
                    }
                }
            }
            None => {}
        }
        return out;
    }

    // === The engine's binding model, compiled to records ===

    /// Turn the engine's `BindingSet` into the payload's mapping layers: this is the whole reason
    /// the model exists on a keyboard, since everywhere else a `BindingSet` is imposed on the
    /// wire by the input engine and here it becomes the device's own bindings.
    ///
    /// Slots the set says nothing about keep the identity record the payload already had, the
    /// meta layers keep whatever the payload had (a `BindingSet` has no source key for the SS
    /// key, so the meta layers are not something it can express), and the wheel layer keeps its
    /// payload default, since evdev gives the volume wheel no code to bind from. Everywhere this
    /// fork does not write a model's bindings onboard, a `BindingSet` is imposed on the wire by
    /// the input engine instead; here it becomes the device's own.
    pub fn compile_bindings(&self, payload: Vec<u8>, bindings: &BindingSet) -> Result<Vec<u8>> {
        if payload.len() != PROFILE_PAYLOAD_LEN {
            return Err(
                Error::InvalidConfig(
                    "only a dual-actuation payload can carry compiled bindings, so a schema A payload keeps the layers the device wrote".to_string()
                )
            );
        }
        let mut out = payload.clone();
        let mut taken = BTreeMap::new();
        for binding in &bindings.bindings {
            match self.slot_of_key(binding.source) {
                None => {
                    return Err(Error::InvalidConfig(format!(
                        "key {} has no onboard slot to write, so this binding cannot become one of the device's own: it has no HID usage the device gives bindings to",
                        binding.source
                    )));
                }
                Some(slot) => {
                    let previous = taken.insert(slot, binding.source);
                    match previous {
                        Some(other) => {
                            return Err(Error::InvalidConfig(format!(
                                "two bindings want one slot: the device has one record per key, so {} and {} cannot both be written",
                                other, binding.source
                            )));
                        }
                        None => {}
                    }
                    match self.action_record(binding.source, binding.action.clone()) {
                        None => {
                            return Err(Error::InvalidConfig(format!(
                                "key {} cannot become action {:?}: an onboard record spells a function octet and up to four HID usages, and this action spells neither",
                                binding.source, binding.action
                            )));
                        }
                        Some(record) => {
                            for index in 0..PROFILE_MAPPING_LEN {
                                out[PROFILE_BUTTON_OFF + PROFILE_MAPPING_LEN * slot + index] = record[index];
                            }
                        }
                    }
                }
            }
        }
        return Ok(out);
    }

    /// The record one binding compiles to, or `None` when no record can say it - and the two
    /// models disagree on the macro, which is the point of this module's caveat: the original
    /// engine wrote a macro's *steps* into the payload's `macro_data` and pointed a record at
    /// them, a layer this fork has no model for, so `Action::Macro` stays where it already
    /// works, in software. `Action::Media` and `Action::MouseButton` are refused one level
    /// down for the same kind of reason: their target usages (the wheel's `e9`/`ea`, the mouse
    /// buttons) have no numbering in this fork to compile against.
    pub fn action_record(&self, source: InputKey, action: Action) -> Option<Vec<u8>> {
        match action {
            Action::Passthrough => match self.usage_of_key(source) {
                Some(usage) => {
                    return Some(self.identity_record(usage));
                }
                None => {
                    return None;
                }
            },
            Action::Disabled => {
                return Some(vec![FUNCTION_DEACTIVATED, 0, 0, 0, 0]);
            }
            Action::Key(key) => match self.usage_of_key(key) {
                Some(usage) => {
                    return Some(vec![FUNCTION_COMBO, usage, 0, 0, 0]);
                }
                None => {
                    return None;
                }
            },
            Action::Combo(keys) => {
                if keys.len() == 0 || keys.len() > PROFILE_MAPPING_KEYS {
                    return None;
                }
                let mut record = vec![FUNCTION_COMBO, 0, 0, 0, 0];
                for index in 0..keys.len() {
                    match self.usage_of_key(keys[index]) {
                        Some(usage) => {
                            record[index + 1] = usage;
                        }
                        None => {
                            return None;
                        }
                    }
                }
                return Some(record);
            }
            Action::Media(_) => {
                return None;
            }
            Action::MouseButton(_) => {
                return None;
            }
            Action::Macro { .. } => {
                return None;
            }
            Action::Text(_) => {
                return None;
            }
            Action::Launch { .. } => {
                return None;
            }
            Action::SwitchProfile(_) => {
                return None;
            }
        }
    }

    // === The upload ===

    /// The FS object of one onboard slot.
    pub fn fs_id_of_slot(&self, slot: u8) -> u16 {
        return FS_PROFILES_BASE + (u16::from(slot) - 1);
    }

    /// Which slot one FS object belongs to, `None` for the lighting map and anything else.
    pub fn slot_of_fs_id(&self, fs_id: u16) -> Option<u8> {
        if fs_id >= FS_PROFILES_BASE && fs_id < (FS_PROFILES_BASE + 5) {
            return Some(1 + (fs_id - FS_PROFILES_BASE) as u8);
        }
        return None;
    }

    /// The payload out to the object's width: the FS object is `profile_size` whatever the
    /// payload, so a short payload's tail is zero and the device sees the same 40 blocks every
    /// time.
    pub fn padded_payload(&self, payload: Vec<u8>) -> Vec<u8> {
        if payload.len() >= PROFILE_PADDED_LEN {
            return payload.clone();
        }
        let mut out = payload.clone();
        out.resize(PROFILE_PADDED_LEN, 0);
        return out;
    }

    /// One 65-octet frame of the write API, ready for `send_raw`: report octet and body.
    pub fn io_frame<T: HidCommand>(&self, command: T) -> Result<Vec<u8>> {
        let mut buffer = [0u8; KEYBOARD_REPORT_SIZE];
        let used = command.serialize(&mut buffer, HidDeviceType::Keyboard)?;
        return Ok(buffer[0..used].to_vec());
    }

    /// One 643-octet write frame: a `ConfigWrite` carrying up to 512 data octets, which is wider
    /// than a fixed-size report and therefore goes out as a feature report (`PACKET.md` §9, write
    /// frame).
    pub fn write_frame(&self, chunk_offset: usize, data: Vec<u8>) -> Result<FsFrame> {
        if data.len() == 0 || data.len() > PROFILE_CHUNK {
            return Err(Error::InvalidConfig(format!(
                "a write frame carries between 1 and {} octets, not {}",
                PROFILE_CHUNK,
                data.len()
            )));
        }
        let command = FsWriteCommand::new(chunk_offset, data);
        let mut buffer = [0u8; FS_WRITE_REPORT_SIZE];
        command.serialize(&mut buffer, HidDeviceType::Keyboard)?;
        return Ok(FsFrame {
            report: buffer.to_vec(),
            feature: true,
        });
    }

    /// The frames that rewrite one slot's FS object: erase it first, so a payload shorter than
    /// the object cannot leave the tail of whatever it held before; then per block two write
    /// halves and a push of the whole block; then ask the device to validate the profile it now
    /// holds, one-based. 122 frames either way, and the count is the object's geometry.
    pub fn fs_frames(&self, slot: u8, payload: Vec<u8>) -> Result<Vec<FsFrame>> {
        if !self.valid_slot(slot) {
            return Err(Error::InvalidConfig(
                "not one of the five one-based onboard slots".to_string(),
            ));
        }
        if payload.len() != PROFILE_PAYLOAD_LEN {
            return Err(Error::InvalidConfig(format!(
                "a profile payload is {} octets, not {}",
                PROFILE_PAYLOAD_LEN,
                payload.len()
            )));
        }
        let fs_id = self.fs_id_of_slot(slot);
        let mut frames = Vec::new();
        match self.io_frame(FsEraseCommand::new(fs_id)) {
            Err(err) => {
                return Err(err);
            }
            Ok(report) => {
                frames.push(FsFrame { report, feature: false });
            }
        }
        let sealed = self.seal_payload(payload);
        let padded = self.padded_payload(sealed);
        for block in 0..PROFILE_BLOCKS {
            for half in 0..2 {
                let from = PROFILE_BLOCK * block + PROFILE_CHUNK * half;
                let to = from + PROFILE_CHUNK;
                match self.write_frame(PROFILE_CHUNK * half, padded[from..to].to_vec()) {
                    Err(err) => {
                        return Err(err);
                    }
                    Ok(frame) => {
                        frames.push(frame);
                    }
                }
            }
            let push = FsCommitCommand::new(fs_id, PROFILE_BLOCK * block);
            match self.io_frame(push) {
                Err(err) => {
                    return Err(err);
                }
                Ok(report) => {
                    frames.push(FsFrame { report, feature: false });
                }
            }
        }
        match self.io_frame(FsValidateCommand::new(slot)) {
            Err(err) => {
                return Err(err);
            }
            Ok(report) => {
                frames.push(FsFrame { report, feature: false });
            }
        }
        return Ok(frames);
    }

    /// Upload a payload the caller already has, and remember it, so the next rewrite of this
    /// slot can keep the layers this module does not own.
    pub fn upload_payload(&mut self, slot: u8, payload: Vec<u8>) -> Result<()> {
        match self.fs_frames(slot, payload.clone()) {
            Err(err) => {
                return Err(err);
            }
            Ok(frames) => {
                for frame in frames {
                    if frame.feature {
                        match self.inner.send_feature_to_control(&frame.report) {
                            Err(err) => {
                                return Err(err);
                            }
                            Ok(_) => {}
                        }
                        continue;
                    }
                    let body = frame.body();
                    match self.inner.send_raw(&body) {
                        Err(err) => {
                            return Err(err);
                        }
                        Ok(_) => {}
                    }
                }
                self.payloads.insert(slot, payload.clone());
                return Ok(());
            }
        }
    }

    /// The frames that make a freshly written profile the running one: select it, apply, push it
    /// live (`PACKET.md` §9, deploy tail). These are not part of an upload - the device keeps running
    /// whatever profile it had selected - and this returns them rather than sending anything, so
    /// a caller decides when a profile goes live.
    pub fn deploy_tail(&self, slot: u8) -> Vec<Vec<u8>> {
        if !self.valid_slot(slot) {
            return Vec::new();
        }
        return vec![
            vec![0x89, 0x01, 0x00, slot],
            vec![0x41, 0x00],
            vec![0x74, 0x00, 0x01, 0x00],
        ];
    }

    /// The frames that rewrite the running profile with the engine's `BindingSet`, without
    /// sending them: useful for a tool that wants to see the sequence, and for the tests.
    pub fn bindings_frames(&mut self, bindings: &BindingSet) -> Result<Vec<FsFrame>> {
        match self.current_payload() {
            Err(err) => {
                return Err(err);
            }
            Ok(payload) => match self.compile_bindings(payload, bindings) {
                Err(err) => {
                    return Err(err);
                }
                Ok(compiled) => {
                    return self.fs_frames(self.bindings_slot, compiled);
                }
            },
        }
    }

    /// The payload of the slot a keybinding rewrite targets, remembered or built from nothing
    /// under the slot's own name.
    pub fn current_payload(&self) -> Result<Vec<u8>> {
        match self.payload_of(self.bindings_slot) {
            Some(payload) => {
                return Ok(payload);
            }
            None => {
                let name = self.payload_name(self.bindings_slot);
                return self.named_payload(&name);
            }
        }
    }
}

impl Device for Apex7 {
    fn info(&self) -> &DeviceInfo {
        return self.inner.info();
    }

    fn device_type(&self) -> DeviceType {
        return self.inner.device_type();
    }

    fn initialize(&mut self) -> Result<()> {
        return self.inner.initialize();
    }

    fn close(&mut self) -> Result<()> {
        return self.inner.close();
    }

    fn is_connected(&self) -> bool {
        return self.inner.is_connected();
    }

    fn send_raw(&mut self, data: &[u8]) -> Result<()> {
        return self.inner.send_raw(data);
    }

    fn receive_raw(&mut self, buf: &mut [u8]) -> Result<usize> {
        return self.inner.receive_raw(buf);
    }
}

impl Configurable for Apex7 {
    /// Which onboard profile a keybinding rewrite rewrites, so a binding can be pushed to a
    /// profile other than the running one.
    fn setting_descriptors(&self) -> Vec<SettingDescriptor> {
        let options: Vec<ChoiceOption> = PROFILE_SLOTS
            .iter()
            .map(|slot| ChoiceOption::new(format!("{}", slot), format!("Config {}", slot)))
            .collect();
        let slot_choice = SettingDescriptor::new(
            "onboard-slot",
            "Onboard profile a keybinding rewrites",
            SettingKind::Choice { options },
        )
        .description(
            "Which of the keyboard's own onboard profiles a keybinding rewrite rewrites: the rewrite writes that profile's FS object, whatever profile the device has selected to run. Only the payload of the profile this fork has decoded from a capture can be written."
        )
        .default_value(SettingValue::Choice(format!("{}", self.bindings_slot)))
        .verification(Verification::Hardware);
        let mut descriptors = self.inner.setting_descriptors();
        descriptors.push(slot_choice);
        return descriptors;
    }

    fn apply_setting(&mut self, id: &str, value: &SettingValue) -> Result<()> {
        if id != "onboard-slot" {
            return self.inner.apply_setting(id, value);
        }
        match value {
            SettingValue::Choice(text) => match self.slot_of_choice(text.as_str()) {
                Some(slot) => {
                    return self.select_bindings_slot(slot);
                }
                None => {}
            },
            SettingValue::Int(number) => {
                return self.select_bindings_slot(u8::try_from(*number).unwrap_or(0));
            }
            _ => {}
        }
        return Err(Error::InvalidConfig(
            "onboard-slot wants a choice of 1 to 5".to_string(),
        ));
    }
}

crate::impl_keyboard_with_delegation!(Apex7, {
    async fn apply(&mut self) -> Result<()> {
        self.inner.apply().await
    }

    async fn set_color(&mut self, color: Color) -> Result<()> {
        self.inner.set_color(color).await
    }

    async fn set_zone_colors(&mut self, colors: &[Color]) -> Result<()> {
        self.inner.set_zone_colors(colors).await
    }

    async fn set_key_color(&mut self, key_id: KeyId, color: Color) -> Result<()> {
        self.inner.set_key_color(key_id, color).await
    }

    async fn set_key_colors(&mut self, key_colors: &[(KeyId, Color)]) -> Result<()> {
        self.inner.set_key_colors(key_colors).await
    }

    // The engine's `BindingSet` becomes the device's own bindings: compile them into the
    // payload's mapping layers, then rewrite that slot's FS object. Everything about the
    // payload and its 122 frames lives in this module, which is why this arm is short.
    fn set_key_bindings(&mut self, bindings: &BindingSet) -> Result<()> {
        match self.current_payload() {
            Err(err) => {
                return Err(err);
            }
            Ok(payload) => match self.compile_bindings(payload, bindings) {
                Err(err) => {
                    return Err(err);
                }
                Ok(compiled) => {
                    return self.upload_payload(self.bindings_slot, compiled);
                }
            },
        }
    }
});

impl Deref for Apex7 {
    type Target = GenericKeyboard;

    fn deref(&self) -> &Self::Target {
        return &self.inner;
    }
}

impl DerefMut for Apex7 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        return &mut self.inner;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::MacroMode::Once;
    use crate::input::{Binding, BindingSet, InputKey};

    fn apex7() -> Apex7 {
        let info = DeviceInfo {
            name: std::borrow::Cow::Borrowed("Test Apex 7"),
            device_type: crate::devices::DeviceType::Keyboard,
            vendor_id: crate::STEELSERIES_VENDOR_ID,
            product_id: Apex7::PRODUCT_ID,
            interface_number: 1,
            usage_page: 0xFFC0,
            usage: 0x01,
            serial_number: None,
            manufacturer: None,
            path: "/test/apex7".to_string(),
        };
        return Apex7::new(GenericKeyboard::new_without_device(info));
    }

    #[test]
    fn slot_table_numbers_the_captures() {
        let device = apex7();
        // The captures' button layer, read back through the table.
        assert_eq!(device.slot_of_usage(0x29), Some(0));
        assert_eq!(device.slot_of_usage(0x35), Some(1));
        assert_eq!(device.slot_of_usage(0x2b), Some(2));
        assert_eq!(device.slot_of_usage(0xe1), Some(4));
        assert_eq!(device.slot_of_usage(0xe0), Some(5));
        assert_eq!(device.usage_of_slot(0), Some(0x29));
        assert_eq!(device.usage_of_slot(1), Some(0x35));
        // The slots the table never names get no usage, and the wheel's usages get no slot.
        for gap in HID_SLOT_GAPS {
            assert_eq!(device.usage_of_slot(gap), None);
        }
        assert_eq!(device.slot_of_usage(0x6a), None);
        assert_eq!(device.slot_of_usage(0x6b), None);
        // The table numbers 120 usages into 120 distinct slots.
        let mut named = Vec::new();
        for usage in 0..256 {
            if let Some(slot) = device.slot_of_usage(u8::try_from(usage).unwrap_or(0)) {
                named.push(slot);
            }
        }
        named.sort();
        named.dedup();
        assert_eq!(named.len(), 120);
    }

    #[test]
    fn upload_costs_122_frames() {
        assert_eq!(PROFILE_BLOCKS, 40);
        assert_eq!(PROFILE_FRAMES, 122);
        assert_eq!(PROFILE_PAYLOAD_LEN, 11696);
        assert_eq!(PROFILE_PAYLOAD_A_LEN + 48, PROFILE_PAYLOAD_LEN);
        assert_eq!(PROFILE_BUTTON_LAYER_LEN, 640);
        // The object is wider than the payload, and not a whole multiple of it: the pad is a
        // partial block's worth of zeros the device also sees.
        assert!(PROFILE_PADDED_LEN > PROFILE_PAYLOAD_LEN);
        assert_ne!(PROFILE_PADDED_LEN % PROFILE_PAYLOAD_LEN, 0);
        assert_eq!((PROFILE_PAYLOAD_LEN + PROFILE_BLOCK - 1) / PROFILE_BLOCK, 12);
    }

    #[test]
    fn payload_fields_end_where_the_captures_say() {
        assert_eq!(PROFILE_CRC_OFF + 4, PROFILE_PAYLOAD_LEN);
        assert_eq!(PROFILE_GUID_OFF + 16, PROFILE_CRC_OFF);
        assert_eq!(PROFILE_MACRO_OFF + PROFILE_MACRO_LEN, PROFILE_GUID_OFF);
        assert_eq!(
            PROFILE_RAPID_TAP_PAIRS_OFF + PROFILE_RAPID_TAP_PAIRS_LEN,
            PROFILE_MACRO_OFF
        );
        assert_eq!(PROFILE_RAPID_TAP_ENABLE_OFF + 1, PROFILE_RAPID_TAP_PAIRS_OFF);
        assert_eq!(PROFILE_WHEEL_META_OFF + PROFILE_WHEEL_LAYER_LEN, 1385);
        assert_eq!(PROFILE_BUTTON_META_OFF - PROFILE_BUTTON_OFF, 650);
        assert_eq!(PROFILE_WHEEL_OFF - PROFILE_BUTTON_OFF, 640);
    }

    #[test]
    fn bare_payload_carries_identity_bindings() {
        let device = apex7();
        let payload = device.bare_payload("Config 1");
        assert_eq!(payload.len(), PROFILE_PAYLOAD_LEN);
        assert_eq!(payload[0], 0x0B);
        assert_eq!(payload[1], 0);
        assert_eq!(payload[PROFILE_META_TOGGLE_OFF], SS_KEY_USAGE);
        assert_eq!(payload[PROFILE_NAME_OFF], 0x43);
        // Slot 0 (space) is bound to itself, and slot 6, whose number the table never names, is
        // written at all.
        let identity = device.identity_record(0x29);
        for index in 0..PROFILE_MAPPING_LEN {
            assert_eq!(payload[PROFILE_BUTTON_OFF + index], identity[index]);
        }
        for index in 0..PROFILE_MAPPING_LEN {
            assert_eq!(payload[PROFILE_BUTTON_OFF + PROFILE_MAPPING_LEN * 6 + index], 0);
        }
        // The crc this module computes is one the device accepts.
        let sealed = device.seal_payload(payload);
        assert!(device.payload_intact(sealed));
    }

    #[test]
    fn slots_are_one_based_and_the_object_size_is_the_pad() {
        let device = apex7();
        assert_eq!(device.fs_id_of_slot(1), FS_PROFILES_BASE);
        assert_eq!(device.fs_id_of_slot(5), 0x0205);
        assert_eq!(device.slot_of_fs_id(FS_LIGHTING_MAP), None);
        assert_eq!(device.valid_slot(0), false);
        assert_eq!(device.valid_slot(6), false);
        assert_eq!(
            device.padded_payload(device.bare_payload("Config 1")).len(),
            PROFILE_PADDED_LEN
        );
        // A rewrite is not a deploy: the live switch is a separate step.
        assert_eq!(device.deploy_tail(1).len(), 3);
        assert_eq!(device.deploy_tail(0).len(), 0);
    }

    #[test]
    fn an_unsupported_action_says_why() {
        let device = apex7();
        let bindings = BindingSet {
            bindings: vec![
                Binding::new(InputKey::KEY_SPACE, Action::Combo(vec![InputKey::KEY_ENTER])),
                Binding::new(InputKey::KEY_TAB, Action::Disabled),
            ],
        };
        assert!(
            device
                .compile_bindings(device.bare_payload("Config 1"), &bindings)
                .is_ok()
        );
        let macro_set = BindingSet {
            bindings: vec![Binding::new(
                InputKey::KEY_SPACE,
                Action::Macro {
                    steps: vec![],
                    repeat: 1,
                    mode: Once,
                },
            )],
        };
        assert!(
            device
                .compile_bindings(device.bare_payload("Config 1"), &macro_set)
                .is_err()
        );
    }

    #[test]
    fn a_key_with_no_slot_refuses_instead_of_guessing() {
        let device = apex7();
        // KeyId::Menu has no LED in the captures' table, so the table gives it no usage, and
        // there is no record to write for it.
        let Some(source) = InputKey::from_key_id(KeyId::Menu) else {
            assert!(false);
            return;
        };
        let unmapped = BindingSet {
            bindings: vec![Binding::new(source, Action::Passthrough)],
        };
        match device.compile_bindings(device.bare_payload("Config 1"), &unmapped) {
            Err(e) => {
                assert!(matches!(e, Error::InvalidConfig(_)))
            }
            Ok(_) => {
                assert!(false)
            }
        }
    }

    #[test]
    fn a_remembered_payload_keeps_the_layers_it_was_not_asked_about() {
        let device = apex7();
        let before = device.bare_payload("Config 1");
        assert_eq!(before.len(), PROFILE_PAYLOAD_LEN);
        let Some(layers) = device.layers_of(before.clone()) else {
            assert!(false);
            return;
        };
        assert_eq!(layers.wheel.len(), PROFILE_WHEEL_LAYER_LEN);
        for index in 0..WHEEL_DEFAULT.len() {
            assert_eq!(layers.wheel[index], WHEEL_DEFAULT[index]);
        }
        // Writing the layers back unchanged is the identity, which is what keeps the macro,
        // lighting and OLED layers a BindingSet says nothing about.
        assert_eq!(device.put_layers(before.clone(), layers), before);
    }

    #[test]
    fn the_upload_frames_are_the_captures_geometry() {
        let device = apex7();
        let frames = match device.fs_frames(1, device.bare_payload("Config 1")) {
            Ok(frames) => frames,
            Err(_) => {
                assert!(false);
                return;
            }
        };
        assert_eq!(frames.len(), PROFILE_FRAMES);
        // Every frame carries its report-id octet first, so the command octet is the second one;
        // a write half goes out whole as a feature report, and every other frame is handed to
        // `send_raw`, which pads it behind its own report octet.
        let at = 1;
        assert_eq!(frames[0].feature, false);
        assert_eq!(frames[0].report[at], 0x88);
        assert_eq!(frames[0].report[at + 1], 0);
        assert_eq!(frames[0].report[at + 2], FS_PROFILES_BASE as u8);
        assert_eq!(frames[0].report[at + 3], (FS_PROFILES_BASE >> 8) as u8);
        for block in 0..PROFILE_BLOCKS {
            for half in 0..2 {
                let frame = &frames[1 + 3 * block + half];
                assert_eq!(frame.feature, true);
                assert_eq!(frame.report.len(), FS_WRITE_REPORT_SIZE);
                assert_eq!(frame.report[at], 0x03);
                assert_eq!(frame.report[at + 1], 0);
                assert_eq!(frame.report[at + 2], (PROFILE_CHUNK * half) as u8);
                assert_eq!(frame.report[at + 3], ((PROFILE_CHUNK * half) >> 8) as u8);
                assert_eq!(frame.report[at + 4], PROFILE_CHUNK as u8);
                assert_eq!(frame.report[at + 5], (PROFILE_CHUNK >> 8) as u8);
            }
            // Both halves are in, so the device may take the whole block: `05 00 <u16 fs_id>
            // <u32 off> <u16 size>`.
            let push = &frames[1 + 3 * block + 2];
            assert_eq!(push.feature, false);
            assert_eq!(push.report[at], 0x05);
            assert_eq!(push.report[at + 1], 0);
            assert_eq!(push.report[at + 2], FS_PROFILES_BASE as u8);
            assert_eq!(push.report[at + 3], (FS_PROFILES_BASE >> 8) as u8);
            assert_eq!(push.report[at + 4], (PROFILE_BLOCK * block) as u8);
            assert_eq!(push.report[at + 5], ((PROFILE_BLOCK * block) >> 8) as u8);
            assert_eq!(push.report[at + 8], PROFILE_BLOCK as u8);
            assert_eq!(push.report[at + 9], (PROFILE_BLOCK >> 8) as u8);
        }
        // The last frame asks the device to validate the profile it now holds, one-based: `eb
        // <profile index>`, which is the capture's `eb` plus the index the engine numbered from
        // zero.
        let last = &frames[PROFILE_FRAMES - 1];
        assert_eq!(last.feature, false);
        assert_eq!(last.report[at], 0xeb);
        assert_eq!(last.report[at + 1], 1);
    }

    #[test]
    fn the_crc_is_the_python_tools_compute() {
        // The same walk `tools/stm32crc.py` does, pinned against the vectors it prints for short
        // inputs, so the crc this module writes is the crc the captures' own payloads carry (the
        // Apex7-3 profile's is `76c199fb` at 11692, which `check` confirms by hashing the buffer
        // with its own four trailing octets excluded).
        assert_eq!(stm32crc(Vec::new()), 0xffff_ffff);
        assert_eq!(stm32crc(vec![0x41]), 0xca7a_d799);
        assert_eq!(stm32crc(vec![0x41, 0x42, 0x43, 0x44]), 0xcf53_4ae1);
    }

    #[test]
    fn a_profile_name_is_any_ascii_that_fits_the_field() {
        let mut device = apex7();
        // The captures' default spelling, and the other spelling one capture set uses.
        assert_eq!(device.payload_name(1), "Config 1");
        assert_eq!(device.valid_name("Config Main"), true);
        assert_eq!(device.valid_name("Main"), true);
        assert_eq!(device.valid_name("24 ASCII octets, exactly"), true);
        // The field is 24 octets at 2, so a 25th octet has nowhere to go, and an empty name has
        // nothing to show on the OLED.
        assert_eq!(device.valid_name("25 ASCII octets, exactly!"), false);
        assert_eq!(device.valid_name(""), false);
        assert_eq!(device.valid_name("Config Ω"), false);
        // A rewrite names the payload it writes, so naming the slot a rewrite targets renames the
        // payload that reaches the wire.
        match device.set_payload_name(1, "Config Main") {
            Err(_) => {
                assert_eq!(false, true);
            }
            Ok(_) => {
                assert_eq!(device.payload_name(1), "Config Main");
                match device.current_payload() {
                    Err(_) => {
                        assert_eq!(false, true);
                    }
                    Ok(payload) => {
                        assert_eq!(payload[1], 0);
                        assert_eq!(payload[PROFILE_NAME_OFF], 0x43);
                        assert_eq!(payload[PROFILE_NAME_OFF + 10], 0x6e);
                        assert_eq!(payload[PROFILE_NAME_OFF + 11], 0);
                    }
                }
            }
        }
        // A slot a rewrite does not target can be named as well, under any ASCII name: nothing
        // about the payload written for slot 1 changes.
        match device.set_payload_name(2, "Main") {
            Err(_) => {
                assert_eq!(false, true);
            }
            Ok(_) => {
                assert_eq!(device.payload_name(2), "Main");
                assert_eq!(device.payload_name(1), "Config Main");
            }
        }
        // A name that does not fit is refused, and the slot keeps whatever name it had.
        match device.set_payload_name(3, "Config Main, plus more than fits") {
            Err(_) => {
                assert_eq!(device.payload_name(3), "Config 3");
            }
            Ok(_) => {
                assert_eq!(false, true);
            }
        }
        match device.set_payload_name(3, "Config Ω") {
            Err(_) => {
                assert_eq!(device.payload_name(3), "Config 3");
            }
            Ok(_) => {
                assert_eq!(false, true);
            }
        }
        // Forgetting a name goes back to the default rather than to nothing.
        device.forget_payload_name(1);
        assert_eq!(device.payload_name(1), "Config 1");
        device.forget_payload_name(2);
        assert_eq!(device.payload_name(2), "Config 2");
        // A slot outside the five the device has cannot be named, so the name table stays keyed
        // to slots.
        match device.set_payload_name(6, "Config Six") {
            Err(_) => {}
            Ok(_) => {
                assert_eq!(false, true);
            }
        }
    }
}
