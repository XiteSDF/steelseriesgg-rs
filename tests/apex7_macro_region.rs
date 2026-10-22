//! The macro record region: every record a profile's region holds, and the bytes a write sends for them.
//!
//! The payload is built in this file, so no profile read from a keyboard is stored in the repository. Its
//! shape is the layout six real `0x1612` reads showed on 2026-10-22 (`ssgg onboard read`, checksum verified
//! on the read, schema b, slots `Config 1` and `Config Main`): records packed from payload 3600 with no gap
//! and no slot table, four bytes to an action, the chain ending at 4208 in the profile that carried 9
//! records of 87 actions, and the CRC field (payload 11692..11696) written by the encoder. The measured
//! sizes are reproduced here; the key codes, ids and names are invented.
//!
//! Run `python3 -I scripts/analyze-macro-region.py <dir of profile payloads>` to survey a capture the same
//! way: it prints the chain, each record's action counts, the bytes behind it and the CRC field.

use serde_json::Value;
use steelseries_gg::devices::keyboards::apex_7_profile::{
    BUTTON_META_MAPPINGS_OFFSET, KEY_MAPPING_LEN, MACRO_ACTION_LEN, MACRO_BINDING_BASE, MACRO_HEADER_LEN, MACRO_ID_MAX,
    MACRO_RECORD_FLAGS_KNOWN, MacroRecordRegion, SCHEMA_B, compute_checksum, decode, encode_json, macro_region_bounds,
    profile_json,
};

const PAYLOAD_LEN: usize = 11_696;
const CRC_OFFSET: usize = 11_692;
/// Byte one past the chain of the profile that carried the most records (9 records, 87 actions): measured
/// on the payload, 1524 bytes into the macro block and 608 bytes long.
const SURVEYED_CHAIN_END: usize = 4208;
/// Where that chain starts on the payload: 1524 bytes into the macro block that starts at 2076.
const SURVEYED_CHAIN_START: usize = 3600;

/// A key push: `0x02 | key | 0x01 0x01`. The milliseconds belong to a delay; a push carries none.
fn press(key: u8) -> Vec<u8> {
    vec![0x02, key, 0x01, 0x01]
}

/// A key release: `0x02 | key | 0x01 0x00`.
fn release(key: u8) -> Vec<u8> {
    vec![0x02, key, 0x01, 0x00]
}

/// A wait: `0x04 | 0x00 | ms u16 LE`.
fn delay(ms: u16) -> Vec<u8> {
    let mut bytes = vec![0x04, 0x00];
    bytes.extend_from_slice(&ms.to_le_bytes());
    bytes
}

/// A record header and the actions packed behind it.
fn record(mode: u8, id: u16, interval_ms: u16, actions: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![0x77, mode];
    bytes.extend_from_slice(&id.to_le_bytes());
    bytes.extend_from_slice(&MACRO_RECORD_FLAGS_KNOWN[0].to_le_bytes());
    bytes.extend_from_slice(&interval_ms.to_le_bytes());
    for action in actions {
        bytes.extend_from_slice(action);
    }
    bytes
}

/// The longest record of the reads surveyed: 20 actions, `8 + 20 * 4` bytes.
fn longest_record() -> Vec<u8> {
    let keys: Vec<u8> = (0x20..0x27).collect();
    let mut actions = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        actions.push(press(*key));
        if index + 1 < keys.len() {
            actions.push(delay(60));
        }
    }
    for key in &keys {
        actions.push(release(*key));
    }
    record(1, 27, 15, &actions)
}

/// The record list the reads carried: two 16-byte records of macro id 3 and a record of macro id 27 with
/// 20 actions, packed at the region start with zero bytes behind them.
fn stock_records() -> Vec<Vec<u8>> {
    vec![
        record(4, 3, 5000, &[press(0x1E), release(0x1E)]),
        record(4, 3, 5000, &[press(0x1F), release(0x1F)]),
        longest_record(),
    ]
}

/// A schema b payload holding `records` packed in its macro record region, `bindings` of them written as
/// `0x71` mapping rows of the META layer (slot, binding number), the region zero behind the last record as
/// every read surveyed had it, and the checksum its bytes hash to.
fn payload(records: &[Vec<u8>], bindings: &[(usize, u8)]) -> Vec<u8> {
    let mut data = vec![0u8; PAYLOAD_LEN];
    data[0..2].copy_from_slice(&SCHEMA_B.to_le_bytes());
    data[2..19].copy_from_slice(b"Macro Region Test");
    let (start, _) = macro_region_bounds(SCHEMA_B);
    let mut at = start;
    for bytes in records {
        data[at..at + bytes.len()].copy_from_slice(bytes);
        at += bytes.len();
    }
    for (slot, index) in bindings {
        let row = BUTTON_META_MAPPINGS_OFFSET + slot * KEY_MAPPING_LEN;
        data[row] = 0x71;
        data[row + 3] = *index;
    }
    let crc = compute_checksum(&data[..CRC_OFFSET]);
    data[CRC_OFFSET..CRC_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());
    data
}

/// The record list of a decoded profile as JSON, for editing it the way a caller would.
fn json_of(bytes: &[u8]) -> Value {
    let profile = decode(bytes).expect("decodes");
    serde_json::from_str(&profile_json(&profile).expect("json")).expect("json value")
}

/// Write a report back and decode the bytes it produced.
fn write_back(json: &Value) -> Vec<u8> {
    encode_json(&serde_json::to_string(json).expect("json text")).expect("encodes")
}

#[test]
fn the_record_region_of_a_schema_b_profile_is_measured() {
    let (start, block) = macro_region_bounds(SCHEMA_B);
    assert_eq!(
        (start, block),
        (2076, 9600),
        "the macro block is the engine's one opaque 9,600-byte field, at 2076 of a schema b payload"
    );
    assert_eq!(
        SURVEYED_CHAIN_START - start,
        1524,
        "six reads of a wired 0x1612 pack their records from payload 3600, 1524 bytes into the block"
    );
    assert_eq!(
        SURVEYED_CHAIN_END - SURVEYED_CHAIN_START,
        608,
        "the chain of the profile that carried the most records ran 608 bytes, 3600 to 4208"
    );
    assert_eq!(
        PAYLOAD_LEN - start,
        9620,
        "the block runs 20 bytes past the payload end, so the bytes a record list has are the block"
    );
    assert_eq!(
        SURVEYED_CHAIN_START + 8 + 8 + 16,
        3632,
        "the two META rows carrying binding 61 (macro id 27) played the record at 3632 in the read that \
         carried binding 61: 8 bytes of the id 3 record, 8 of the id 3 record's second copy, 8 of the id 27 \
         record's header, and its first action byte is 3632"
    );
    assert_eq!(
        MACRO_BINDING_BASE + usize::from(MACRO_ID_MAX) - 1,
        255,
        "a binding row is one byte, so ids 1..={MACRO_ID_MAX} reach bindings {MACRO_BINDING_BASE}..=255"
    );
    assert_eq!(
        MACRO_BINDING_BASE + 83 - 1,
        117,
        "the read whose META binding carried 117 held a record of macro id 83: 35 + 83 - 1"
    );
}

#[test]
fn all_records_of_the_region_decode_and_report_their_capacity() {
    let bytes = payload(&stock_records(), &[]);
    let profile = decode(&bytes).expect("decodes");
    assert!(profile.crc_ok, "the payload carries the checksum its bytes hash to");
    assert_eq!(
        profile.macro_note, None,
        "a region of real shape walks clean: {}/{}",
        profile.macro_region.used_bytes, profile.macro_region.len
    );
    assert_eq!(
        profile.macro_region,
        MacroRecordRegion {
            start: 2076,
            len: 9600,
            used_bytes: 120,
            free_bytes: 9480,
            records: 3,
            over_capacity: false,
        },
        "the records packed at the block start, the space they leave behind, and no more than the region \
         holds"
    );
    assert_eq!(
        profile
            .macros
            .iter()
            .map(|rec| (rec.id, rec.mode, rec.interval_ms, rec.record_bytes, rec.index))
            .collect::<Vec<_>>(),
        vec![(3, 4, 5000, 16, 37), (3, 4, 5000, 16, 37), (27, 1, 15, 88, 61)],
        "every record of the region, in payload order: 8 header bytes plus 4 per action, and the binding \
         number of its macro id"
    );
    assert!(
        profile.macros.iter().all(|rec| rec.flags_ok),
        "every record carries {:04X} at +4, the bytes every record of every read surveyed had",
        MACRO_RECORD_FLAGS_KNOWN[0]
    );
    assert_eq!(
        profile.macros[2]
            .actions
            .iter()
            .filter(|action| action.kind == "press")
            .count(),
        7,
        "the longest record holds seven pushes"
    );
    assert_eq!(
        profile.macros[2]
            .actions
            .iter()
            .filter(|action| action.kind == "release")
            .count(),
        7,
        "the seven releases that close them"
    );
    assert_eq!(
        profile.macros[2]
            .actions
            .iter()
            .filter(|action| action.kind == "delay")
            .count(),
        6,
        "and six waits: 8 + 20 * {MACRO_ACTION_LEN} = {MACRO_HEADER_LEN} + 80 = 88 bytes, the record's own size"
    );
    assert_eq!(
        profile.macros[2]
            .actions
            .iter()
            .filter_map(|action| action.ms)
            .collect::<Vec<_>>(),
        vec![60u16; 6],
        "only a wait carries milliseconds"
    );
}

#[test]
fn a_write_of_the_same_report_is_the_payload_byte_for_byte() {
    let bytes = payload(&stock_records(), &[(8, 61)]);
    let profile = decode(&bytes).expect("decodes");
    let written = write_back(&json_of(&bytes));
    assert_eq!(
        written, bytes,
        "a report nobody edited writes back the bytes the read produced, the checksum field included"
    );
    let again = decode(&written).expect("re-decodes");
    assert_eq!(again.macros, profile.macros, "the record list survives the round trip");
    assert_eq!(again.macro_region, profile.macro_region);
    assert!(
        again.crc_ok,
        "the written payload carries a checksum, so the read that follows verifies it"
    );
}

#[test]
fn a_binding_number_names_the_macro_its_record_sits_in() {
    let bytes = payload(&stock_records(), &[(8, 61), (9, 61), (10, 255)]);
    let profile = decode(&bytes).expect("decodes");
    let bound: Vec<(usize, u8)> = profile
        .button_meta_mappings
        .iter()
        .filter(|mapping| mapping.function == 0x71)
        .map(|mapping| {
            (
                mapping.offset,
                mapping.macro_index.expect("a 0x71 row carries its number"),
            )
        })
        .collect();
    assert_eq!(
        bound,
        vec![(8, 61), (9, 61), (10, 255)],
        "three META rows bound to macros, in the shape the read carried"
    );
    let id = u16::try_from(61 - MACRO_BINDING_BASE + 1).expect("an in-range binding number");
    assert_eq!(id, 27, "61 is the binding number of macro id 27");
    let hits: Vec<usize> = profile
        .macros
        .iter()
        .filter(|rec| rec.id == id)
        .map(|rec| rec.offset)
        .collect();
    assert_eq!(
        hits,
        vec![2108],
        "the two rows play the one record of macro id 27, packed behind the two records of id 3"
    );
    assert!(
        profile.macros.iter().all(|rec| rec.id != MACRO_ID_MAX),
        "the row carrying 255 names macro id 221, which this profile has no record for: the deleted-macro \
         state, with the binding left standing"
    );
}

#[test]
fn the_capacity_is_the_region_and_not_the_ui_grid() {
    let long = longest_record();
    assert_eq!(long.len(), 88, "the longest record the reads carried");
    let (_, capacity) = macro_region_bounds(SCHEMA_B);
    assert_eq!(
        capacity / long.len(),
        109,
        "that many of them fit in the block's {capacity} bytes"
    );
    assert!(
        109 * long.len() <= capacity && 110 * long.len() > capacity,
        "one more does not fit"
    );
    assert_eq!(
        usize::from(MACRO_ID_MAX),
        221,
        "a binding row reaches 221 macros, so the UI grid is not the board's limit either way"
    );
}

#[test]
fn a_record_list_over_capacity_is_refused_with_the_numbers() {
    let mut doc = json_of(&payload(&stock_records(), &[]));
    let record_json = |position: usize| {
        let key = 0x04 + (position % 0x70) as u8;
        let id = (position as u16 % MACRO_ID_MAX) + 1;
        let actions = (0..20)
            .map(|_| serde_json::json!({ "kind": "press", "key": key }))
            .collect::<Vec<_>>();
        let raw: Vec<String> = std::iter::once(format!("77 04 {id:02x} 00 01 01 64 00"))
            .chain(actions.iter().map(|_| format!("02 {key:02x} 01 01")))
            .collect();
        serde_json::json!({
            "offset": 3600 + position * 88,
            "ordinal": position,
            "index": MACRO_BINDING_BASE + usize::from(id) - 1,
            "flags_ok": true,
            "mode": 4,
            "mode_name": "repeat",
            "id": id,
            "flags": MACRO_RECORD_FLAGS_KNOWN[0],
            "interval_ms": 100,
            "record_bytes": 88,
            "action_count": 20,
            "actions": actions,
            "raw": raw.join(" "),
        })
    };
    doc["macros"] = Value::from((0..200).map(record_json).collect::<Vec<_>>());
    let reason = encode_json(&serde_json::to_string(&doc).expect("json text"))
        .expect_err("200 records of 88 bytes do not fit in the region");
    let reason = reason.to_string();
    assert!(
        reason.contains("17600 bytes") && reason.contains("has 9600"),
        "the refusal names the bytes needed and the region it needs them in: {reason}"
    );
}

#[test]
fn bytes_that_are_not_records_are_named_and_sent_back_verbatim() {
    let mut data = payload(&stock_records(), &[]);
    let (start, _) = macro_region_bounds(SCHEMA_B);
    // GG's own per-macro structures behind the chain, in the shape a wired read carried: a
    // header with no actions, then bytes that are not actions at all.
    let at = start + 120;
    data[at] = 0x77;
    data[at + 8] = 0x03;
    data[at + 9] = 0x01;
    data[at + 10] = 0x0A;
    let crc = compute_checksum(&data[..CRC_OFFSET]);
    data[CRC_OFFSET..CRC_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());

    let profile = decode(&data).expect("decodes");
    assert_eq!(profile.macros.len(), 3, "the records the region holds are decoded");
    let note = profile.macro_note.expect("the bytes behind them are named");
    assert!(
        note.contains("non-zero byte(s) of the block are not records") && note.contains("does not decode them"),
        "{note}"
    );
    // The write is not refused and nothing is invented: the engine declares this block an opaque
    // bytearray (`lisp/apex_7.lisp:62, 101`), so the report ships its bytes and the write sends them back.
    let written = write_back(&json_of(&data));
    assert_eq!(
        written, data,
        "the block's bytes go back exactly as the read produced them"
    );
}

#[test]
fn a_record_cut_by_the_region_end_is_not_a_record() {
    let (start, _) = macro_region_bounds(SCHEMA_B);
    // The last record of the region is cut by the payload end: a header and two actions, 16 bytes, with
    // the payload ending on its last action byte.
    // The region's own end: the block runs 20 bytes past the payload end, so its last bytes are here.
    let at = start + 9600 - 16;
    assert!(
        at + 16 <= PAYLOAD_LEN,
        "the last record of the region ends inside the payload"
    );
    let mut data = payload(&stock_records(), &[]);
    let tail = record(4, 40, 100, &[press(0x04), release(0x04)]);
    data[at..at + tail.len()].copy_from_slice(&tail);
    let crc = compute_checksum(&data[..CRC_OFFSET]);
    data[CRC_OFFSET..CRC_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());

    let profile = decode(&data).expect("decodes");
    assert_eq!(profile.macros.len(), 3, "the cut record is not added to the list");
    let note = profile.macro_note.expect("a record cut by the block end is named");
    assert!(note.contains("record(s) cut by the block end"), "{note}");
    assert_eq!(
        profile.macro_region.used_bytes, 120,
        "the cut record counts as none of the list's bytes"
    );
    assert_eq!(
        write_back(&json_of(&data)),
        data,
        "the bytes are sent back verbatim, so a read and a write of them changes nothing"
    );
}

#[test]
fn an_empty_region_is_empty() {
    let bytes = payload(&[], &[]);
    let profile = decode(&bytes).expect("decodes");
    assert!(
        profile.macros.is_empty() && profile.macro_note.is_none(),
        "a block of zeros is what a profile with no macros carries: {:?}",
        profile.macro_note
    );
    assert_eq!((profile.macro_region.records, profile.macro_region.used_bytes), (0, 0));
    assert_eq!(profile.macro_region.free_bytes, profile.macro_region.len);
}
