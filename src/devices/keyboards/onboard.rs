//! On-board profile protocol interface. `[EXPERIMENTAL]`: every protocol fact behind it is
//! reverse-engineered from official GG captures and read from keyboards; nothing here writes a profile.
//!
//! Two interfaces, one per side of the work:
//!
//! - [`Keyboard::onboard_codec`](crate::devices::keyboards::Keyboard::onboard_codec) answers *which
//!   family protocol a keyboard speaks*, and
//!   [`Keyboard::read_onboard_profile`](crate::devices::keyboards::Keyboard::read_onboard_profile)
//!   pulls one slot on the handle the device manager opened. A keyboard that has no on-board profile
//!   says so by leaving both defaults.
//! - [`OnboardProfileCodec`] is that family's codec: the file-side work - decode, encode, report JSON,
//!   dump file names, the text a read prints - which needs no device, so `ssgg onboard decode` and
//!   `ssgg onboard encode` run it on files with no keyboard connected.
//!
//! One family implements them today: `apex_legacy_profile` (the legacy Apex family, `0x1612`), through
//! [`LegacyApexOnboard`](crate::devices::keyboards::apex_legacy_profile::LegacyApexOnboard), which
//! [`Apex7`](crate::devices::keyboards::apex7::Apex7) hands out. The report types below **are** that
//! family's payload layout as data: the next keyboard plugs in by implementing the codec for its own
//! layout, extending these types where its profile has fields this one has, and registering its codec
//! in [`codecs`]. `src/main.rs` is written against these interfaces and names no keyboard family.

use crate::error::Result;
use serde::Serialize;

pub use crate::devices::keyboards::apex_legacy_profile::{
    KeyMapping, LightingProfile, MacroAction, MacroSummary, OnboardMacro, OnboardProfile, ProfileImage, RapidTap,
    RapidTapPair, ReadReply, ReadStats,
};

/// The keyboard a read collected profile bytes from, recorded in the read's record and in the decode
/// report the read writes. The payload layout, what can be encoded back, and which features exist are
/// all per device, so profile bytes without a product ID do not say which board's codec produced them.
/// A decode of a file records nothing: the file does not name a keyboard, only the schema word inside it.
/// `[EXPERIMENTAL]`: one family records a device today (legacy Apex).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnboardDevice {
    /// USB vendor ID (every SteelSeries device answers `0x1038`).
    pub vendor_id: u16,
    /// USB product ID: the model selector, and the key of the model-name table.
    pub product_id: u16,
    /// Model name this crate lists for the product ID ([`crate::devices::device_name_from_product_id`]).
    pub product: String,
    /// False when this crate's product registry does not list the ID: the name is then the crate's
    /// fallback, and a reader should trust the numbers, not the name.
    pub known: bool,
}

/// A keyboard family's on-board profile codec: everything the protocol does with bytes, with no device.
///
/// A family implements this for its own payload layout. [`Keyboard::onboard_codec`] hands the codec to
/// a caller that has the keyboard; [`codecs`] lists the families a file-only command can use.
pub trait OnboardProfileCodec: Send + Sync {
    /// Family label for logs and read output: which boards this codec's protocol covers.
    fn family_name(&self) -> &'static str;

    /// Whether this family's protocol was reverse-engineered on this product ID. A codec answers for
    /// its own family, so `false` on a codec means "these bytes are from a board this codec does not
    /// know", which is what a device recorded in a file has to be checked against.
    fn knows_product(&self, product_id: u16) -> bool;

    /// Number of profile slots the keyboard stores, as GG numbers them: `Config 1 .. Config N`.
    fn config_count(&self) -> u8;

    /// The 0-based slot behind a GG config number, the number `ssgg onboard read --profile` takes.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::InvalidConfig`] when the number is not a config number.
    fn slot_for_config(&self, config: u8) -> Result<u8>;

    /// Whether this family's layout knows a payload schema word.
    fn knows_schema(&self, schema: u16) -> bool;

    /// Bytes one key-mapping entry occupies in this layout.
    fn mapping_len(&self) -> usize;

    /// Decode payload bytes into the report.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload is too short or its schema is not one this family knows.
    fn decode(&self, data: &[u8]) -> Result<OnboardProfile>;

    /// Write a decode report JSON back into payload bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the JSON is not a decode report of this family, or carries a field this
    /// layout cannot write faithfully.
    fn encode_json(&self, text: &str) -> Result<Vec<u8>>;

    /// A decoded profile as the JSON report `ssgg onboard decode` writes.
    ///
    /// # Errors
    ///
    /// Propagates a serde failure.
    fn profile_json(&self, profile: &OnboardProfile) -> Result<String>;

    /// A read image as the JSON read record `ssgg onboard read --report` writes.
    ///
    /// # Errors
    ///
    /// Propagates a serde failure.
    fn image_json(&self, image: &ProfileImage) -> Result<String>;

    /// File name the profile name in a payload yields (`Config 1` -> `config_1.json`). `config` is GG's
    /// config number, used when the payload carries no name to build a file name from.
    fn dump_name(&self, data: &[u8], config: u8) -> String;

    /// One-line summary of a decoded profile.
    fn summary_line(&self, profile: &OnboardProfile) -> String;

    /// Payload bytes as a hex dump, `[from, to)`.
    fn hex(&self, data: &[u8], from: usize, to: usize) -> String;

    /// The lines describing one read image: the transfer record, the firmware poll answer, the decode.
    fn image_lines(&self, image: &ProfileImage, label: &str) -> Vec<String>;

    /// The lines describing one decoded profile.
    fn profile_lines(&self, profile: &OnboardProfile) -> Vec<String>;

    /// The `[EXPERIMENTAL]` notes this family prints before a device read.
    fn banner_lines(&self) -> Vec<String>;

    /// The result of comparing encoded bytes with a profile file, as `ssgg onboard encode --check` prints
    /// it. A read image is longer than the payload it carries, so what is compared is the payload and the
    /// bytes behind it are named.
    fn check_lines(&self, encoded: &[u8], reference: &[u8], path: &str) -> Vec<String>;

    /// Byte where two read images start to differ, `None` when they are equal.
    fn diff(&self, a: &ProfileImage, b: &ProfileImage) -> Option<usize>;
}

/// Every family codec this build knows, in the order a file-only command tries them.
///
/// A new keyboard family registers its codec here, and its keyboards return it from
/// [`Keyboard::onboard_codec`](crate::devices::keyboards::Keyboard::onboard_codec).
#[must_use]
pub fn codecs() -> Vec<&'static dyn OnboardProfileCodec> {
    vec![&crate::devices::keyboards::apex_legacy_profile::LegacyApexOnboard]
}

/// The codec of the family that knows a payload schema word.
#[must_use]
pub fn codec_for_schema(schema: u16) -> Option<&'static dyn OnboardProfileCodec> {
    codecs().into_iter().find(|codec| codec.knows_schema(schema))
}

/// The codec a file whose schema no family knows is read with: the first registered family, which then
/// reports the schema it does not know. With one family registered this is that family.
#[must_use]
pub fn default_codec() -> Option<&'static dyn OnboardProfileCodec> {
    codecs().into_iter().next()
}

/// Payload bytes as a decode report, with the family that knows the schema word inside them.
#[must_use]
pub fn codec_for_bytes(data: &[u8]) -> Option<&'static dyn OnboardProfileCodec> {
    codec_for_schema(schema_of(data)).or_else(default_codec)
}

/// A decode report JSON as payload bytes, with the family that knows the `schema` field inside it.
#[must_use]
pub fn codec_for_json(text: &str) -> Option<&'static dyn OnboardProfileCodec> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("schema").and_then(serde_json::Value::as_u64))
        .and_then(|schema| u16::try_from(schema).ok())
        .and_then(codec_for_schema)
        .or_else(default_codec)
}

/// Payload schema word: the first two bytes, little endian (`0` when the payload is shorter than the
/// word).
#[must_use]
pub fn schema_of(data: &[u8]) -> u16 {
    if data.len() < 2 {
        return 0;
    }
    u16::from_le_bytes([data[0], data[1]])
}

/// Bytes as the space-separated hex the report fields carry.
#[must_use]
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

/// Offset of the first byte that differs between two images, `None` when they are equal.
#[must_use]
pub fn image_diff(a: &ProfileImage, b: &ProfileImage) -> Option<usize> {
    a.data.iter().zip(b.data.iter()).position(|(x, y)| x != y)
}

/// Bytes where two payloads start to differ, `None` when they are equal. Payloads of different lengths
/// answer the shorter length: `--check` of a read image reports the payload it compares.
#[must_use]
pub fn payload_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter().zip(b).position(|(x, y)| x != y)
}

/// Multi-line hex dump of `data[from..to]`, 16 bytes per line with an offset column.
#[must_use]
pub fn hex_dump(data: &[u8], from: usize, to: usize) -> String {
    let from = from.min(data.len());
    let to = to.max(from).min(data.len());
    let mut out = String::new();
    for (i, chunk) in data[from..to].chunks(16).enumerate() {
        out.push_str(&format!("{:06X}  {}\n", from + i * 16, hex_of(chunk)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_registry_answers_the_family_that_knows_a_schema() {
        use crate::devices::keyboards::apex_legacy_profile::{SCHEMA_A, SCHEMA_B};

        for schema in [SCHEMA_A, SCHEMA_B] {
            let codec = codec_for_schema(schema).expect("the legacy Apex family knows its schema words");
            assert!(codec.knows_schema(schema));
            assert_eq!(codec.config_count(), 5, "GG numbers Config 1 .. Config 5");
            assert!(codec.family_name().contains("Apex"), "{}", codec.family_name());
        }
        assert!(
            codec_for_schema(0).is_none(),
            "a schema no family knows is not claimed by one"
        );
        assert!(default_codec().is_some(), "a file of an unknown schema has a reader");
    }

    #[test]
    fn codec_for_a_file_picks_the_family_inside_it() {
        let codec = codec_for_json(r#"{"schema": 11}"#).expect("a JSON carrying a schema word has a family");
        assert!(codec.knows_schema(11));
        assert!(
            codec_for_json(r#"{"schema": 9999}"#)
                .expect("an unknown schema falls back to the default family")
                .knows_schema(11),
            "the fallback is the registered family, which then reports the schema it does not know"
        );
        assert!(
            codec_for_json("not json").is_some(),
            "a file that is not JSON gets the default family"
        );
    }

    /// The CLI addresses slots through the codec, so the config-number rule lives with the family.
    #[test]
    fn codec_maps_ggs_config_number_to_the_slot() {
        let codec = default_codec().expect("a codec is registered");
        for config in 1..=5 {
            assert_eq!(codec.slot_for_config(config).expect("a config number"), config - 1);
        }
        for config in [0, 6, 200] {
            assert!(
                codec.slot_for_config(config).is_err(),
                "{config} is not a config number"
            );
        }
    }

    #[test]
    fn hex_dump_covers_the_requested_window() {
        let data: Vec<u8> = (0..40u8).collect();
        let dump = hex_dump(&data, 16, 32);
        assert!(dump.starts_with("000010  10 11 12"), "{dump}");
        assert_eq!(dump.lines().count(), 1, "one window is one line");
        assert!(
            hex_dump(&data, 36, 100).contains("000024"),
            "a window past the end is clipped"
        );
    }
}
