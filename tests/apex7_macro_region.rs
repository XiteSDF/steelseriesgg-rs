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
    BUTTON_MAPPINGS_OFFSET, BUTTON_META_MAPPINGS_OFFSET, KEY_MAPPING_LEN, MACRO_ACTION_LEN, MACRO_BINDING_BYTE_MAX,
    MACRO_BINDING_REACH_BYTES, MACRO_HEADER_LEN, MACRO_INDEX_UNIT, MACRO_RECORD_FLAGS_KNOWN, MacroAction,
    MacroRecordRegion, SCHEMA_B, compute_checksum, decode, encode_json, macro_index, macro_mouse_button,
    macro_mouse_mask_name, macro_offset_for_binding_byte, macro_region_bounds, profile_json,
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

/// A mouse press: `0x00 | mask | 0x00 0x01`. The mask says which buttons go down together.
fn press_mouse(mask: u8) -> Vec<u8> {
    vec![0x00, mask, 0x00, 0x01]
}

/// A mouse release: `0x00 | mask | 0x00 0x00`.
fn release_mouse(mask: u8) -> Vec<u8> {
    vec![0x00, mask, 0x00, 0x00]
}

/// A record header and the actions packed behind it.
fn record(mode: u8, header_word: u16, interval_ms: u16, actions: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![0x77, mode];
    bytes.extend_from_slice(&header_word.to_le_bytes());
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
    payload_layers(records, &[], bindings)
}

/// The same payload with `0x71` rows in both mapping layers: `base` rows go in the base table, `meta` rows
/// in the META table, each as (slot, binding byte).
fn payload_layers(records: &[Vec<u8>], base: &[(usize, u8)], meta: &[(usize, u8)]) -> Vec<u8> {
    let mut data = vec![0u8; PAYLOAD_LEN];
    data[0..2].copy_from_slice(&SCHEMA_B.to_le_bytes());
    data[2..19].copy_from_slice(b"Macro Region Test");
    let (start, _) = macro_region_bounds(SCHEMA_B);
    let mut at = start;
    for bytes in records {
        data[at..at + bytes.len()].copy_from_slice(bytes);
        at += bytes.len();
    }
    let rows = base
        .iter()
        .map(|(slot, index)| (BUTTON_MAPPINGS_OFFSET, slot, index))
        .chain(
            meta.iter()
                .map(|(slot, index)| (BUTTON_META_MAPPINGS_OFFSET, slot, index)),
        );
    for (table, slot, index) in rows {
        let row = table + slot * KEY_MAPPING_LEN;
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
        "the two META rows carrying binding 61 played the record at 3632 in the read that carried \
         binding 61: 8 bytes of the first record, 8 of the second, 8 of the third record's header, and its \
         first action byte is 3632"
    );
    assert_eq!(
        MACRO_BINDING_REACH_BYTES, 1020,
        "a binding row is one byte counting 4-byte units, so it names block offsets 0, 4, ... 1020 and no \
         further; the block is 9600 bytes, so the last part of it is out of reach of any binding"
    );
    assert_eq!(
        (
            macro_offset_for_binding_byte(0, 2076),
            macro_offset_for_binding_byte(7, 2076),
            macro_offset_for_binding_byte(14, 2076)
        ),
        (2076, 2104, 2132),
        "the read whose keys c, v, x carry the bytes 0, 7, 14 presses LCtrl with c, v, x in the macros at \
         block offsets 0, 28, 56 - the keystrokes are the measurement"
    );
    assert_eq!(
        (MACRO_INDEX_UNIT, MACRO_ACTION_LEN),
        (4, 4),
        "the unit a binding counts is one action: a macro 28 bytes into the block is reached by byte 7"
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
            .map(|rec| (rec.header_word, rec.mode, rec.interval_ms, rec.record_bytes, rec.index))
            .collect::<Vec<_>>(),
        vec![(3, 4, 5000, 16, 0), (3, 4, 5000, 16, 4), (27, 1, 15, 88, 8)],
        "every record of the region, in payload order: 8 header bytes plus 4 per action, and the binding \
         byte that reaches it - its offset from the block start over {MACRO_INDEX_UNIT}"
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

/// A macro that drives the mouse instead of the keyboard: the shape a wired read carries when a number-row
/// key is bound to a macro that clicks mouse buttons. The binding byte is the macro's offset from the block
/// start in 4-byte units, and a mask of 0 is not an action - that is what keeps the walk off the block's
/// zero padding.
#[test]
fn a_mouse_macro_decodes_and_a_binding_reaches_it() {
    let key_macro = record(
        1,
        5,
        15,
        &[press(0xE0), press(0x06), delay(20), release(0x06), release(0xE0)],
    );
    let click = record(1, 1, 15, &[press_mouse(0x03)]);
    let toggle = record(2, 3, 15, &[press_mouse(0x08), delay(1), release_mouse(0x08)]);
    let bytes = payload(&[key_macro, click, toggle], &[(8, 7), (9, 10)]);
    let profile = decode(&bytes).expect("decodes");
    assert_eq!(
        profile.macros.iter().map(|rec| rec.record_bytes).sum::<usize>(),
        60,
        "28 + 12 + 20 bytes of macros, and the zero padding behind them is not actions"
    );
    assert_eq!(
        profile.macros.iter().map(|rec| rec.index).collect::<Vec<_>>(),
        vec![0, 7, 10],
        "the binding byte is each macro's offset from the block start over {MACRO_INDEX_UNIT}"
    );
    let click = &profile.macros[1];
    assert_eq!(
        click.actions,
        vec![MacroAction {
            kind: "press_mouse",
            key: None,
            mask: Some(0x03),
            ms: None,
        }],
        "one mouse press, no key code"
    );
    assert_eq!(
        macro_mouse_mask_name(click.actions[0].mask),
        "mouse 1+mouse 2",
        "mask 0x03 presses the two buttons together, which is how a macro makes a middle click"
    );
    let toggle = &profile.macros[2];
    assert_eq!(
        toggle.actions.iter().map(|a| a.kind).collect::<Vec<_>>(),
        vec!["press_mouse", "delay", "release_mouse"],
        "press, one millisecond, release: a click"
    );
    assert_eq!(
        macro_mouse_mask_name(toggle.actions[0].mask),
        "mouse 4",
        "mask 0x08 is the fourth button"
    );
    for (slot, byte) in [(8usize, 7u8), (9, 10)] {
        let row = profile
            .button_meta_mappings
            .iter()
            .find(|mapping| mapping.offset == slot)
            .expect("the row is bound");
        assert_eq!((row.function, row.macro_index), (0x71, Some(byte)));
        let at = macro_offset_for_binding_byte(byte, profile.macro_region.start);
        assert!(
            profile.macros.iter().any(|rec| rec.offset == at),
            "byte {byte} names the macro at block offset {}",
            at - profile.macro_region.start
        );
    }
}

#[test]
fn a_write_of_the_same_report_is_the_payload_byte_for_byte() {
    let bytes = payload(&stock_records(), &[(8, 7)]);
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
fn a_binding_byte_names_the_record_it_points_at() {
    let bytes = payload(&stock_records(), &[(8, 4), (9, 4), (10, 255)]);
    let profile = decode(&bytes).expect("decodes");
    let bound: Vec<(usize, u8)> = profile
        .button_meta_mappings
        .iter()
        .filter(|mapping| mapping.function == 0x71)
        .map(|mapping| {
            (
                mapping.offset,
                mapping.macro_index.expect("a 0x71 row carries its byte"),
            )
        })
        .collect();
    assert_eq!(
        bound,
        vec![(8, 4), (9, 4), (10, 255)],
        "three META rows bound to macros, in the shape the read carried"
    );
    let at = macro_offset_for_binding_byte(4, profile.macro_region.start);
    assert_eq!(at, 2092, "the byte 4 names the macro 16 bytes into the block");
    let hits: Vec<usize> = profile
        .macros
        .iter()
        .filter(|rec| rec.offset == at)
        .map(|rec| rec.offset)
        .collect();
    assert_eq!(
        hits,
        vec![2092],
        "both rows play the one macro at that offset, packed behind the macro at the block start"
    );
    let far = macro_offset_for_binding_byte(MACRO_BINDING_BYTE_MAX, profile.macro_region.start);
    assert_eq!(
        far - profile.macro_region.start,
        MACRO_BINDING_REACH_BYTES,
        "255 names the farthest byte of the block a binding can reach"
    );
    assert!(
        profile.macros.iter().all(|rec| rec.offset != far),
        "no macro starts at the farthest byte a binding can name: the deleted-macro state, with the \
         binding left standing"
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
        MACRO_BINDING_REACH_BYTES / long.len(),
        11,
        "a binding byte reaches the first 1020 bytes of the block - 11 of the longest records - while the \
         block's capacity is 109 of them: the byte, not the block, is what limits binding, and the UI grid \
         is neither"
    );
}

#[test]
fn a_record_list_over_capacity_is_refused_with_the_numbers() {
    let mut doc = json_of(&payload(&stock_records(), &[]));
    let record_json = |position: usize| {
        let key = 0x04 + (position % 0x70) as u8;
        let header_word = (position as u16 % 200) + 1;
        let actions = (0..20)
            .map(|_| serde_json::json!({ "kind": "press", "key": key }))
            .collect::<Vec<_>>();
        let raw: Vec<String> = std::iter::once(format!("77 04 {header_word:02x} 00 01 01 64 00"))
            .chain(actions.iter().map(|_| format!("02 {key:02x} 01 01")))
            .collect();
        serde_json::json!({
            "offset": 2076 + position * 88,
            "ordinal": position,
            "index": macro_index(2076 + position * 88, 2076),
            "flags_ok": true,
            "mode": 4,
            "mode_name": "repeat",
            "header_word": header_word,
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

/// `n` actions of key typing: press, wait, release triplets on invented key codes, so a record of a measured
/// size can be built without carrying the keystrokes a person typed into their own macro.
fn typed_actions(n: usize) -> Vec<Vec<u8>> {
    (0x30u8..0x3A)
        .cycle()
        .take(n.div_ceil(3))
        .flat_map(|key| [press(key), delay(15), release(key)])
        .take(n)
        .collect()
}

/// The board the binding rule was measured on: a wired `0x1612` read (firmware 4.16.8, schema b, 2026-10-22)
/// whose number row `3` to `7` is built from mouse macros. Its 14 records are packed from the block start with
/// no gap, its 14 binding bytes are the third code byte of their rows, and every one of them names a record the
/// block holds. The record sizes, modes, header words and intervals are the read's own; the mouse masks are what
/// its number-row macros press; the key codes of the typing records are invented.
fn mouse_board_records() -> Vec<Vec<u8>> {
    vec![
        record(
            1,
            5,
            15,
            &[press(0xE0), press(0x06), delay(20), release(0x06), release(0xE0)],
        ),
        record(
            1,
            5,
            15,
            &[press(0xE0), press(0x19), delay(20), release(0x19), release(0xE0)],
        ),
        record(
            1,
            5,
            15,
            &[press(0xE0), press(0x1B), delay(20), release(0x1B), release(0xE0)],
        ),
        record(4, 2, 15, &[press(0x25), release(0x25)]),
        record(1, 1, 15, &[press_mouse(0x03)]),
        record(2, 3, 15, &[press_mouse(0x08), delay(1), release_mouse(0x08)]),
        record(3, 3, 25, &[press_mouse(0x10), delay(1), release_mouse(0x10)]),
        record(4, 3, 30, &[press_mouse(0x20), delay(1), release_mouse(0x20)]),
        record(5, 1, 15, &[press_mouse(0x07)]),
        record(1, 15, 15, &typed_actions(15)),
        record(1, 27, 15, &typed_actions(27)),
        record(1, 2, 15, &[press(0x25), release(0x25)]),
        record(1, 19, 15, &typed_actions(19)),
        record(1, 83, 15, &typed_actions(83)),
    ]
}

/// A layer's macro bindings: (slot, binding byte).
type Bindings = Vec<(usize, u8)>;

/// The read's 14 macro bindings: (slot, binding byte) in the base layer and the META layer.
fn mouse_board_bindings() -> (Bindings, Bindings) {
    (
        vec![(28, 14), (36, 0), (44, 7)],
        vec![
            (15, 117),
            (20, 21),
            (25, 25),
            (33, 28),
            (39, 92),
            (41, 33),
            (49, 38),
            (57, 43),
            (63, 46),
            (79, 63),
            (86, 96),
        ],
    )
}

#[test]
fn every_binding_of_a_mouse_macro_board_resolves_to_a_record() {
    let (base, meta) = mouse_board_bindings();
    let bytes = payload_layers(&mouse_board_records(), &base, &meta);
    let profile = decode(&bytes).expect("decodes");
    assert_eq!(profile.macros.len(), 14, "every record the block holds is decoded");
    assert_eq!(
        profile
            .macros
            .iter()
            .map(|rec| rec.offset - profile.macro_region.start)
            .collect::<Vec<_>>(),
        vec![0, 28, 56, 84, 100, 112, 132, 152, 172, 184, 252, 368, 384, 468],
        "records packed with no gap, the mouse records 100 to 172 bytes into the block"
    );
    assert!(
        profile
            .macros
            .iter()
            .all(|rec| rec.header_word as usize == rec.action_count),
        "the header word is the action count in every record of this board, not a macro id"
    );
    let mut bindings: Vec<(usize, u8)> = profile
        .button_mappings
        .iter()
        .chain(profile.button_meta_mappings.iter())
        .filter(|mapping| mapping.function == 0x71)
        .map(|mapping| {
            (
                mapping.offset,
                mapping.macro_index.expect("a 0x71 row carries its byte"),
            )
        })
        .collect();
    bindings.sort();
    assert_eq!(bindings.len(), 14, "the board's 14 macro bindings");
    for (slot, byte) in &bindings {
        let at = macro_offset_for_binding_byte(*byte, profile.macro_region.start);
        assert!(
            profile.macros.iter().any(|rec| rec.offset == at),
            "slot {slot} carries byte {byte}, which names offset {at} - the block holds a record there"
        );
    }
    // The five number-row keys the mouse macros are built from, in the order the read carried them.
    let masks: Vec<String> = [25usize, 33, 41, 49, 57]
        .iter()
        .map(|slot| {
            let byte = profile
                .button_meta_mappings
                .iter()
                .find(|mapping| mapping.offset == *slot)
                .and_then(|mapping| mapping.macro_index)
                .expect("the key is bound to a macro");
            let at = macro_offset_for_binding_byte(byte, profile.macro_region.start);
            let rec = profile
                .macros
                .iter()
                .find(|rec| rec.offset == at)
                .expect("the binding names a record");
            macro_mouse_mask_name(
                rec.actions
                    .iter()
                    .find(|action| action.kind == "press_mouse")
                    .and_then(|action| action.mask),
            )
        })
        .collect();
    assert_eq!(
        masks,
        vec![
            "mouse 1+mouse 2",
            "mouse 4",
            "mouse 5",
            "mouse 6",
            "mouse 1+mouse 2+mouse 3"
        ],
        "keys 3, 4, 5, 6, 7 press the buttons their records carry - the measurement that fixed the rule"
    );
    // The operator's board is a macro that presses a mouse button, and the bytes for keys 3 and 7 are
    // not the single-bit mask the engine's own reader names that button with: the engine's rule is
    // log2(mask) + 1, so mouse 3 is 0x04 and mouse 7 is 0x40, and it rejects 0x03 and 0x07 outright.
    // The board carries the button number in the mask byte for those two keys, and the single-bit mask
    // for the three between them.
    let masks: Vec<u8> = [25usize, 33, 41, 49, 57]
        .iter()
        .map(|slot| {
            let byte = bindings
                .iter()
                .find(|(offset, _)| *offset == *slot)
                .map(|(_, byte)| *byte)
                .expect("the key is bound to a macro");
            let at = macro_offset_for_binding_byte(byte, profile.macro_region.start);
            let rec = profile
                .macros
                .iter()
                .find(|rec| rec.offset == at)
                .expect("the binding names a record");
            rec.actions
                .iter()
                .find(|action| action.kind == "press_mouse")
                .and_then(|action| action.mask)
                .expect("the record presses a mouse button")
        })
        .collect();
    assert_eq!(
        masks,
        vec![0x03, 0x08, 0x10, 0x20, 0x07],
        "the mask bytes the board carries for keys 3, 4, 5, 6, 7"
    );
    assert_eq!(
        masks.iter().map(|mask| macro_mouse_button(*mask)).collect::<Vec<_>>(),
        vec![None, Some(4), Some(5), Some(6), None],
        concat!(
            "only the three middle bytes are the button the engine's reader names; keys 3 and 7 carry ",
            "the button number (3 and 7), which GG's reader rejects and which the operator's GG shows ",
            "as mouse 3 and mouse 7 from its own database"
        )
    );
    // The superseded rule (byte = 7 x the record's position in the chain) agrees only on the four bindings
    // whose records are 28 bytes apart, and that is the whole board the rule was measured on.
    let mut agree: Vec<u8> = bindings
        .iter()
        .filter(|(_, byte)| {
            let legacy = profile
                .macros
                .iter()
                .find(|rec| rec.ordinal == usize::from(*byte) / 7)
                .map(|rec| rec.offset);
            legacy == Some(macro_offset_for_binding_byte(*byte, profile.macro_region.start))
        })
        .map(|(_, byte)| *byte)
        .collect();
    agree.sort();
    assert_eq!(
        agree,
        vec![0, 7, 14, 21],
        "the four bindings whose record 7 x position names are the ones the old measurement saw; the other          ten name a record the board plays differently - keys 5, 6, 7 land on the mouse buttons of 3 and 4"
    );
}
