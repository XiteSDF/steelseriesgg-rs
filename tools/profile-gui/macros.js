// ssgg macro editor (prototype): the packed macro records of a profile, edited as key push, key
// release and wait instructions. It opens as its own window (`macros.html`) when `ssgg gui` serves the
// page, and the same editor mounts inside the profile editor when a window is not available.
//
// The grammar and every limit here mirror `src/devices/keyboards/apex_7_profile.rs`, which is the
// authority: the record header `0x77 | mode u8 | header word u16 | flags u16 | interval u16` is 8 bytes
// and every action is 4 - a key push `0x02 | key u8 | 01 01`, a key release `0x02 | key u8 | 01 00`, a
// mouse-button press `0x00 | mask u8 | 00 01`, a mouse-button release `0x00 | mask u8 | 00 00`, a wait
// `0x04 | 00 | ms u16`. A push has no duration of its own: the milliseconds belong to a wait. The mask
// is a mouse-button bitmask (bit 0 button 1 through bit 7 button 8), so one action presses several
// buttons at once - mask 0x03 is the left+right pair a macro uses for a middle click. The engine's
// own reader names a mouse action's button log2(mask) + 1 and rejects a byte that is not a single bit
// in 0x01..0x80, so a single-bit mask is the form GG can read back, and the multi-bit masks the reads
// carry (0x03, 0x07) are bytes its reader refuses. A mask of 0 is not an action: the walk stops at it,
// which is what keeps a record from swallowing the block's zero padding.
//
// Records are packed with no stride and no length word, from payload 3600 in the six reads surveyed
// (wired Apex 7 `0x1612`, schema b, 2026-10-22) - 1524 bytes into the macro block, which starts at 2076
// and is the engine's one opaque 9,600-byte field. GG's own per-macro structures share that block, so
// the records do not fill it, and the chain of the profile that carried the most records ran 608 bytes
// to 4208. The capacity a list has is the block; a list that needs more is refused with its numbers,
// exactly as `encode_macro_records` refuses it.
//
// A binding names a record by where it sits, not by its place in the list: a `0x71` binding row carries
// one byte, and the record it plays starts at `byte x 4` bytes into the block - the same 4-byte unit an
// action fills. Measured on a wired read (schema B) whose keys c, v, x carry 0, 7, 14 and whose macros
// 0, 28 and 56 bytes into the block press LCtrl with c, v, x; and on a read whose number row carries 25,
// 28, 33, 38, 43 and whose macros at 100, 112, 132, 152, 172 press mouse masks 0x03, 0x08, 0x10, 0x20,
// 0x07. A byte reaches 1020 bytes of the block - the binding byte, not a slot count and not the block's
// capacity, is the limit on what a key can play, and a macro packed past that is out of reach.
//
// `[EXPERIMENTAL]`, same label the codec carries: the split is what the surveyed reads parse, and only
// mode 4 is measured on a read.
//
// This file writes no keyboard. It edits the `macros` list of the profile JSON; the checksum words and
// the block bytes stay as the read wrote them, and `ssgg onboard write` recomputes what it sends.
//
// Loaded as a plain script for the same reason app.js is: a page opened from the filesystem has origin
// `null` and refuses module scripts. It publishes `globalThis.SsggMacroEditor`.
(function ssggMacroEditor() {
  'use strict';

  const HEADER_LEN = 8;
  const PRESS_LEN = 4;
  const RELEASE_LEN = 4;
  const DELAY_LEN = 4;
  const RECORD_MARKER = 0x77;
  const RECORD_FLAGS = 0x0101;
  const ACTION_KEY = 0x02;
  const ACTION_DELAY = 0x04;
  const ACTION_MOUSE = 0x00;
  const ACTION_LEN = 4;
  // The mouse-button mask bits, and the highest mask a read carries. A mask of 0 is not an action.
  const MOUSE_BITS = [[1, 'mouse 1'], [2, 'mouse 2'], [4, 'mouse 3'], [8, 'mouse 4'], [16, 'mouse 5'],
    [32, 'mouse 6'], [64, 'mouse 7'], [128, 'mouse 8']];
  // The masks the engine's own reader accepts are the single bits, mouse 1 (0x01) to mouse 8 (0x80):
  // it names a mouse action's button log2(mask) + 1 and rejects the byte when that is not a whole
  // number in 1..8. A mask of several bits is what the reads carry (0x03 = mouse 1+mouse 2); the
  // engine's reader rejects those, so the editor offers the single bits first.
  const MASK_MAX = 0x80;
  const MASK_BUTTONS = [1, 2, 4, 8, 16, 32, 64, 128];
  // The macro block of a schema b payload: 9,600 bytes from 2076, the engine's one `macro-data-size`
  // field. The six reads surveyed packed their records from 3600, inside it.
  const REGION_START_B = 2076;
  const REGION_LEN_B = 9600;
  const SURVEYED_CHAIN_START = 3600;
  const SURVEYED_CHAIN_END = 4208;
  const SCHEMA_B = 0x000b;
  const MODE_REPEAT = 4;
  const MODE_ONCE = 1;
  const MODE_COUNTED = 2;
  const MODE_WHILE_PRESSED = 3;
  const MODE_TOGGLE_HOLD = 5;
  const MODE_LOOP = 6;
  const MODE_MAX = 8;
  // The header's flags word, ground truth (Apex 7 `0x1612`, firmware 4.16.8, schema b, slot 2,
  // 2026-10-24: a profile GG wrote, its five macros named in GG's own words): the low byte is a flag
  // (0 on the mode-1 record, 1 on the mode 2, 3, 4 and 5 records) and the high byte is the repeat
  // count - 69 on the "play 69 times every 15 ms" record, 1 on the four others.
  const LOOP_FLAGS = [0, 1];
  const REPEAT_MAX = 255;
  const U16_MAX = 65_535;
  // A `0x71` binding row names a record with one byte: the byte counts 4-byte units from the block
  // start, so the record it plays starts at `byte x 4` bytes into the block. Measured on a wired read
  // (schema B) whose keys c, v, x carry 0, 7, 14 and whose macros 0, 28, 56 bytes into the block press
  // LCtrl with c, v, x, and on a read whose number row carries 25, 28, 33, 38, 43 over macros at 100,
  // 112, 132, 152, 172 pressing mouse masks 0x03, 0x08, 0x10, 0x20, 0x07.
  const INDEX_UNIT = 4;
  const BINDING_BYTE_MAX = 255;
  const BINDING_REACH_BYTES = BINDING_BYTE_MAX * INDEX_UNIT;
  const CHANNEL = 'ssgg-macros';

  // Modes the editor offers: the five playback modes of GG's macro editor, each measured on the
  // ground-truth profile named on `LOOP_FLAGS` - one record per mode, all five in one profile, each
  // named by the operator from GG's UI. The words are GG's own.
  const MODE_OPTIONS = [
    { value: MODE_ONCE, label: 'play once on release', measured: true },
    { value: MODE_COUNTED, label: 'play N times every interval', measured: true },
    { value: MODE_WHILE_PRESSED, label: 'repeat while pressed, every interval', measured: true },
    { value: MODE_REPEAT, label: 'toggle auto repeat, every interval', measured: true },
    { value: MODE_TOGGLE_HOLD, label: 'toggle hold', measured: true },
  ];

  const bindingByte = (offsetFromStart) => Math.floor(offsetFromStart / INDEX_UNIT);
  const offsetForBindingByte = (byte) => byte * INDEX_UNIT;
  // The name of a mouse mask, for the panels and the editor: 0x03 reads as "mouse 1+mouse 2".
  function maskName(mask) {
    if (mask === null || mask === undefined) return null;
    const bits = MOUSE_BITS.filter(([bit]) => mask & bit).map(([, name]) => name);
    return bits.length ? bits.join('+') : `mask 0x${hex2(mask)}`;
  }
  // The one button a mask names, by the engine's rule (log2(mask) + 1, mouse 1..8); null for a mask
  // of no bit or of several bits - the bytes the reads carry for a number-row macro (0x03, 0x07).
  const mouseButton = (mask) => (mask > 0 && mask <= 128 && (mask & (mask - 1)) === 0
    ? MOUSE_BITS.findIndex(([bit]) => bit === mask) + 1
    : null);
  // The header word a record carries: the measured rule is its action count (14 of the 15 records
  // surveyed carry exactly that; one carries 83 with 48 actions). It is data, not an identity - a
  // binding names the position, never this word, and a rewrite keeps the number the read carried.
  const headerWordFor = (record) => record.header_word ?? (record.actions ?? []).length;

  const hex2 = (byte) => byte.toString(16).padStart(2, '0');

  // The macro record region of a profile: the read's own numbers when the report carries them
  // (`macro_region`, written by `ssgg onboard read`), the measured schema b constants otherwise.
  // Ground truth (see `LOOP_FLAGS`): GG packs the records from the block's first byte, 8-byte header
  // plus 4-byte actions, with no length word and no terminator between them.
  function regionOf(doc) {
    const region = doc && doc.macro_region;
    return {
      start: Number.isInteger(region?.start) ? region.start : REGION_START_B,
      len: Number.isInteger(region?.len) ? region.len : REGION_LEN_B,
      fromReport: !!region,
      schema: Number.isInteger(doc?.schema) ? doc.schema : null,
    };
  }

  // Schema a (the 2023 board) is surveyed on no read: the codec decodes no macro records for it, so the
  // editor has no grammar to write with. Say so instead of offering edits.
  function editorDisabledReason(doc) {
    if (!doc || typeof doc !== 'object') return 'no profile is open';
    const region = regionOf(doc);
    if (region.schema !== null && region.schema !== SCHEMA_B) {
      return `schema 0x${hex2((region.schema >> 8) & 0xff)}${hex2(region.schema & 0xff)}: the macro records of this `
        + 'slot are not decoded (measured on schema b only), so there is nothing here to edit';
    }
    if (!Array.isArray(doc.macros) && !isHexBlock(doc.macro_data && doc.macro_data.bytes_hex)) {
      return 'this report carries neither a macro record list nor the block bytes: read the profile with `ssgg onboard read`';
    }
    return null;
  }

  function isHexBlock(text) {
    return typeof text === 'string' && /^([0-9a-fA-F]{2})( [0-9a-fA-F]{2})*$/.test(text.trim());
  }

  // Bytes one action fills, and bytes one record fills: the header plus its actions.
  function actionLen(action) {
    if (action.kind === 'press') return PRESS_LEN;
    if (action.kind === 'release') return RELEASE_LEN;
    if (action.kind === 'delay') return DELAY_LEN;
    // An action the grammar does not know is measured as one action slot: `limitProblems` names it, and
    // the encoder refuses the list for the same reason. Throwing here would stop the panel from drawing
    // a record it is showing as a problem.
    return ACTION_LEN;
  }

  function recordBytes(record) {
    return HEADER_LEN + (record.actions ?? []).reduce((sum, action) => sum + actionLen(action), 0);
  }

  // The most actions one record can hold: what is left of the block after its header.
  const maxActions = (region) => Math.floor(((region ? region.len : REGION_LEN_B) - HEADER_LEN) / ACTION_LEN);

  function totalBytes(records) {
    return records.reduce((sum, record) => sum + recordBytes(record), 0);
  }

  // How a record list sits in the region, with the same numbers `ssgg onboard write` uses.
  function plan(records, region) {
    const used = totalBytes(records);
    const len = region ? region.len : REGION_LEN_B;
    return {
      records: records.length,
      used,
      len,
      free: len - used,
      fits: used <= len,
      over: Math.max(0, used - len),
      end: (region ? region.start : REGION_START_B) + used,
    };
  }

  // The words of a record's bytes, as the encode side reads them: header, then one action per chunk.
  function recordRaw(record) {
    const out = [RECORD_MARKER, record.mode & 0xff];
    out.push(headerWordFor(record) & 0xff, (headerWordFor(record) >> 8) & 0xff);
    out.push(record.flags & 0xff, (record.flags >> 8) & 0xff);
    out.push(record.interval_ms & 0xff, (record.interval_ms >> 8) & 0xff);
    for (const action of record.actions ?? []) {
      if (action.kind === 'press') {
        out.push(ACTION_KEY, (action.key ?? 0) & 0xff, 0x01, 0x01);
      } else if (action.kind === 'release') {
        out.push(ACTION_KEY, (action.key ?? 0) & 0xff, 0x01, 0x00);
      } else if (action.kind === 'press_mouse') {
        out.push(ACTION_MOUSE, (action.mask ?? 0) & 0xff, 0x00, 0x01);
      } else if (action.kind === 'release_mouse') {
        out.push(ACTION_MOUSE, (action.mask ?? 0) & 0xff, 0x00, 0x00);
      } else {
        out.push(ACTION_DELAY, 0x00);
        out.push((action.ms ?? 0) & 0xff, ((action.ms ?? 0) >> 8) & 0xff);
      }
    }
    return out.map(hex2).join(' ');
  }

  // Records renumbered as the encoder packs them: the list is packed from the region's first byte, so
  // every record's payload offset, ordinal, binding byte, size and raw bytes follow from the order.
  function packRecords(records, region) {
    const start = region ? region.start : REGION_START_B;
    let at = start;
    return records.map((record, ordinal) => {
      const actions = (record.actions ?? []).map(normalizeAction);
      const packed = {
        ...record,
        actions,
        offset: at,
        ordinal,
        index: bindingByte(at - start),
        mode: record.mode & 0xff,
        mode_name: modeName(record.mode),
        flags: record.flags ?? RECORD_FLAGS,
        flags_ok: flagsOk({ flags: record.flags ?? RECORD_FLAGS }),
        interval_ms: record.interval_ms,
        header_word: headerWordFor(record),
        action_count: actions.length,
      };
      packed.record_bytes = recordBytes(packed);
      packed.raw = recordRaw(packed);
      at += packed.record_bytes;
      return packed;
    });
  }

  function modeName(mode) {
    const option = MODE_OPTIONS.find((entry) => entry.value === mode);
    return option ? option.label : 'unknown';
  }

  // The two halves of the flags word, see `LOOP_FLAGS`.
  const loopFlagOf = (record) => (record.flags ?? RECORD_FLAGS) & 0xff;
  const repeatOf = (record) => ((record.flags ?? RECORD_FLAGS) >> 8) & 0xff;
  const flagsOk = (record) => flagsRuleOk(record.flags ?? RECORD_FLAGS, record.mode);
  // The measured rule, in one place: low byte 0 on the mode-1 record and on the mode-4 record that
  // carries a repeat count (the second dump's `00 03`), 1 on the other measured records; the high byte
  // is the repeat count and takes any byte. Modes the reads carry above the five measured ones keep
  // their flags as they are.
  const flagsRuleOk = (flags, mode) => {
    const low = flags & 0xff;
    const high = (flags >> 8) & 0xff;
    if (mode > MODE_TOGGLE_HOLD) return true;
    if (low === 1) return mode >= MODE_ONCE;
    return low === 0 && (mode === MODE_ONCE || high > 0);
  };
  const withRepeat = (record, times) => ((record.flags ?? RECORD_FLAGS) & 0xff) | ((times & 0xff) << 8);

  function normalizeAction(action) {
    if (action.kind === 'press') return { kind: 'press', key: action.key ?? 0 };
    if (action.kind === 'release') return { kind: 'release', key: action.key ?? 0 };
    if (action.kind === 'press_mouse') return { kind: 'press_mouse', mask: action.mask ?? 0 };
    if (action.kind === 'release_mouse') return { kind: 'release_mouse', mask: action.mask ?? 0 };
    return { kind: 'delay', ms: action.ms ?? 0 };
  }

  const makePress = (key) => ({ kind: 'press', key });
  const makeRelease = (key) => ({ kind: 'release', key });
  const makePressMouse = (mask) => ({ kind: 'press_mouse', mask });
  const makeReleaseMouse = (mask) => ({ kind: 'release_mouse', mask });
  const makeDelay = (ms) => ({ kind: 'delay', ms });

  // A new record with no actions: the header alone, packed at the end of the list.
  function makeRecord(ordinal, offsetFromStart = 0) {
    return {
      offset: 0,
      ordinal,
      index: bindingByte(offsetFromStart),
      flags_ok: true,
      mode: MODE_REPEAT,
      mode_name: modeName(MODE_REPEAT),
      header_word: 1,
      flags: RECORD_FLAGS,
      interval_ms: 100,
      record_bytes: HEADER_LEN,
      action_count: 0,
      actions: [],
      raw: '',
    };
  }

  // Walk the record region of a block, the way the codec walks it: a record starts at 0x77, actions run
  // to the next marker or to the zero run that pads the region. Used when a report carries the block
  // bytes and no `macros` list (a read from before the record list shipped).
  function recordsFromBlock(bytesHex, region) {
    const bytes = (bytesHex ?? '').trim().split(/\s+/).filter(Boolean).map((word) => Number.parseInt(word, 16));
    const start = region.start;
    const end = Math.min(bytes.length, region.start + region.len);
    const records = [];
    // The same walk as the codec's `decode_macros`: a record is a marker, a header GG would write (mode
    // 1..8, header word >= 1, interval >= 1) and its actions, read until the grammar stops matching. A header
    // with no action, a header GG's own structures wear, and a record the block end cuts off are all
    // skipped - the bytes behind the records belong to GG, and this grammar invents nothing from them.
    const headerAt = (at) => {
      if (at + HEADER_LEN > end || bytes[at] !== RECORD_MARKER) return null;
      const mode = bytes[at + 1];
      const headerWord = bytes[at + 2] | (bytes[at + 3] << 8);
      const flags = bytes[at + 4] | (bytes[at + 5] << 8);
      const interval = bytes[at + 6] | (bytes[at + 7] << 8);
      if (mode < 1 || mode > MODE_MAX || headerWord < 1 || interval < 1) return null;
      return { mode, headerWord, flags, interval };
    };
    const actionAt = (at) => {
      if (at + ACTION_LEN > end) return null;
      const byte = bytes[at];
      if (byte === ACTION_KEY) {
        const key = bytes[at + 1];
        const flag = bytes[at + 2];
        const state = bytes[at + 3];
        if (flag !== 0x01) return null;
        return state === 0x01 ? makePress(key) : makeRelease(key);
      }
      if (byte === ACTION_MOUSE && bytes[at + 1] !== 0 && bytes[at + 2] === 0x00) {
        // A mouse mask of 0 is the zero padding, not an action: that is what stops the walk there.
        return bytes[at + 3] === 0x01 ? makePressMouse(bytes[at + 1]) : makeReleaseMouse(bytes[at + 1]);
      }
      if (byte === ACTION_DELAY && bytes[at + 1] === 0x00) {
        return makeDelay(bytes[at + 2] | (bytes[at + 3] << 8));
      }
      return null;
    };
    let at = start;
    while (at < end) {
      const header = headerAt(at);
      if (!header) {
        at += 1;
        continue;
      }
      const actions = [];
      let cursor = at + HEADER_LEN;
      for (;;) {
        const action = actionAt(cursor);
        if (!action) break;
        actions.push(action);
        cursor += ACTION_LEN;
      }
      if (actions.length && cursor < end) {
        records.push({
          offset: at,
          ordinal: records.length,
          index: bindingByte(totalBytes(records)),
          flags_ok: flagsOk(header),
          mode: header.mode,
          mode_name: modeName(header.mode),
          header_word: header.headerWord,
          flags: header.flags,
          interval_ms: header.interval,
          record_bytes: cursor - at,
          action_count: actions.length,
          actions,
          raw: bytes.slice(at, cursor).map(hex2).join(' '),
        });
      }
      at = cursor > at ? cursor : at + 1;
    }
    return packRecords(records, region);
  }

  // The records a profile gives the editor to work on.
  function workingRecords(doc) {
    const region = regionOf(doc);
    if (Array.isArray(doc.macros) && doc.macros.length) {
      return packRecords(doc.macros.map((record) => ({ ...record, actions: [...(record.actions ?? [])] })), region);
    }
    if (Array.isArray(doc.macros)) return [];
    const hex = doc.macro_data && doc.macro_data.bytes_hex;
    return isHexBlock(hex) ? recordsFromBlock(hex, region) : [];
  }

  // A profile JSON whose macro list is this record list, packed from the region's first byte.
  function applyRecords(doc, records, region) {
    const packed = packRecords(records, region);
    const used = totalBytes(packed);
    const out = { ...doc, macros: packed };
    out.macro_region = {
      start: region.start,
      len: region.len,
      used_bytes: used,
      free_bytes: region.len - used,
      records: packed.length,
    };
    return out;
  }

  // Everything that would stop `ssgg onboard write` from taking this list, in the codec's own terms.
  function limitProblems(records, region) {
    const problems = [];
    const size = plan(records, region);
    const cap = maxActions(region);
    if (!size.fits) {
      problems.push(`${records.length} macro record(s) need ${size.used} bytes and the macro block at `
        + `${region.start} has ${region.len}: the bytes that do not fit are lost, so the board runs the `
        + `macros the region holds`);
    }
    // The binding byte is one byte counting 4-byte units, so it names offsets 0, 4, ... 1020 of the
    // block. A record packed past that is written, and read back, and no key can play it.
    let packed = 0;
    const outOfReach = records.filter((record) => {
      const at = packed;
      packed += recordBytes(record);
      return at > BINDING_REACH_BYTES;
    }).length;
    if (outOfReach) {
      problems.push(`${outOfReach} of ${records.length} records pack past byte ${BINDING_REACH_BYTES} of `
        + `the block, where no binding byte reaches (255 x ${INDEX_UNIT}): they are written and read back, `
        + 'and no key can play them');
    }
    records.forEach((record, position) => {
      const where = `record ${position} (${recordBytes(record)} bytes)`;
      if (!Number.isInteger(headerWordFor(record)) || headerWordFor(record) < 1 || headerWordFor(record) > U16_MAX) {
        problems.push(`${where}: the action count (the header word) must be a whole number 1..${U16_MAX} - every record of every `
          + 'read surveyed carries a non-zero word there; it is the number GG keeps for the record, equal to '
          + 'the action count on one dump and short of it on the other');
      }
      if ((record.actions ?? []).length === 0) {
        problems.push(`${where}: a record with no action is not a macro - the walk skips a header with no `
          + 'action behind it, so a header-only record would not come back from a read');
      }
      if (!Number.isInteger(record.interval_ms) || record.interval_ms < 1 || record.interval_ms > U16_MAX) {
        problems.push(`${where}: interval must be a whole number 1..${U16_MAX} ms (a 16-bit word, and every `
          + 'record of every read surveyed has one)');
      }
      if (!flagsRuleOk(record.flags ?? RECORD_FLAGS, record.mode)) {
        problems.push(`${where}: flags 0x${hex2(record.flags & 0xff)}${hex2((record.flags >> 8) & 0xff)} - the low byte `
          + 'is 0 on the mode-1 record and on the mode-4 record that carries a repeat count (the dumps '
          + 'carry 00 03 there) and 1 on the other measured records; the high byte is the repeat count '
          + 'and takes any byte');
      }
      if (!Number.isInteger(record.mode) || record.mode < 1 || record.mode > MODE_MAX) {
        problems.push(`${where}: mode must be a whole number 1..${MODE_MAX} (one byte; the reads surveyed `
          + 'carry 1, 2, 4 and 5)');
      }
      const actions = record.actions ?? [];
      if (actions.length === 0) {
        problems.push(`${where}: has no action, so it plays nothing - add a key push, or delete the record`);
      }
      if (actions.length > cap) {
        problems.push(`${where}: ${actions.length} actions need ${recordBytes(record)} bytes, more than the `
          + `block's ${region ? region.len : REGION_LEN_B} - ${cap} actions is the most one record can hold`);
      }
      actions.forEach((action, index) => {
        const at = `${where} action ${index}`;
        const KINDS = ['press', 'release', 'press_mouse', 'release_mouse', 'delay'];
        if (!KINDS.includes(action.kind)) {
          problems.push(`${at}: is "${action.kind}", and the grammar knows only key push, key release, `
            + 'mouse press, mouse release and wait');
        }
        if (action.kind === 'delay' && (!Number.isInteger(action.ms) || action.ms < 0 || action.ms > U16_MAX)) {
          problems.push(`${at}: ms must be a whole number 0..${U16_MAX} (16-bit word)`);
        }
        if (action.kind === 'press' || action.kind === 'release') {
          if (!Number.isInteger(action.key) || action.key < 0 || action.key > 255) {
            problems.push(`${at}: key code must be a whole number 0..255 (one byte)`);
          }
        }
        if (action.kind === 'press_mouse' || action.kind === 'release_mouse') {
          if (!Number.isInteger(action.mask) || action.mask < 1 || action.mask > MASK_MAX) {
            problems.push(`${at}: mask must be a whole number 1..${MASK_MAX} (the mouse-button bits: bit 0 `
              + 'is button 1, bit 7 button 8) - a mask of 0 is not an action, the walk stops at it');
          }
        }
      });
    });
    return problems;
  }

  // ---------------------------------------------------------------- names

  function keyNames() {
    return (globalThis.SsggProfileApp && globalThis.SsggProfileApp.KEY_NAMES) ?? {};
  }

  function keyLabel(code) {
    const names = keyNames();
    const name = names[code];
    return name ? `${name} (0x${hex2(code)})` : `0x${hex2(code)}`;
  }

  function keyOptions(selected) {
    const codes = Object.keys(keyNames()).map(Number).filter((code) => code > 0).sort((a, b) => a - b);
    const options = codes.map((code) => ({ value: code, label: keyLabel(code) }));
    if (!codes.includes(selected)) options.push({ value: selected, label: keyLabel(selected) });
    return options;
  }

  function describeAction(action) {
    if (action.kind === 'press') return `push ${keyLabel(action.key ?? 0)}`;
    if (action.kind === 'release') return `release ${keyLabel(action.key ?? 0)}`;
    if (action.kind === 'press_mouse') return `${maskName(action.mask ?? 0)} down (mask 0x${hex2(action.mask ?? 0)})`;
    if (action.kind === 'release_mouse') return `${maskName(action.mask ?? 0)} up (mask 0x${hex2(action.mask ?? 0)})`;
    return `wait ${action.ms ?? 0} ms`;
  }

  // ---------------------------------------------------------------- DOM

  // Same shape as app.js's `el`: `class` and `text` are properties, everything else an attribute.
  function el(tag, props, children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props ?? {})) {
      if (key === 'class') node.className = value;
      else if (key === 'text') node.textContent = value;
      else if (value !== null && value !== undefined) node.setAttribute(key, String(value));
    }
    for (const child of [].concat(children ?? [])) if (child) node.appendChild(child);
    return node;
  }

  function button(label, onClick, props) {
    const extra = props && props.class ? ` ${props.class}` : '';
    const node = el('button', { type: 'button', ...props, class: `btn${extra}`, text: label });
    node.addEventListener('click', onClick);
    return node;
  }

  function numberInput(value, min, max, ms) {
    const input = el('input', {
      type: 'number',
      class: 'mono',
      value: String(value),
      min: String(min),
      max: String(max),
      step: '1',
      ...(ms ? { 'aria-label': ms } : {}),
    });
    // The `value` attribute is only the control's default; readers use the property, so set it too.
    input.value = String(value);
    return input;
  }

  function select(options, selected) {
    const node = el('select', { class: 'code-select' });
    for (const option of options) {
      node.append(el('option', { value: String(option.value), text: option.label }));
    }
    node.value = String(selected);
    return node;
  }

  function clampNumber(input, min, max) {
    const value = Number.parseInt(input.value, 10);
    if (!Number.isInteger(value)) return min;
    return Math.min(max, Math.max(min, value));
  }

  // ---------------------------------------------------------------- editor

  // Mounts the editor into `root`. `doc` is the open profile; `onSave(records)` is called with the
  // packed records, and the caller writes them into the profile (the page's own copy, or the `ssgg gui`
  // session the main editor picks up).
  function mount(root, { doc, onSave, embedded }) {
    const region = regionOf(doc);
    const state = { records: workingRecords(doc), selected: 0 };
    const list = el('div', { class: 'macro-list' });
    const detail = el('div', { class: 'macro-detail' });
    const meterText = el('p', { class: 'mono macro-meter' });
    const status = el('p', { class: 'mono note macro-status' });
    status.setAttribute('aria-live', 'polite');

    function records() {
      return state.records;
    }

    function selectedRecord() {
      return state.records[state.selected] ?? null;
    }

    function setStatus(text) {
      status.textContent = text;
    }

    function drawMeter() {
      const size = plan(records(), region);
      meterText.textContent = `${size.records} record(s) - ${size.used} of the block's ${size.len} bytes `
        + `used, ${size.free} free${size.fits ? '' : `, ${size.over} bytes too many`}`;
      meterText.classList.toggle('over', !size.fits);
    }

    function drawList() {
      const rows = records().map((record, position) => {
        const size = recordBytes(record);
        const label = `record #${position} - ${record.mode_name} (mode ${hex2(record.mode)}), binding byte `
          + `${bindingByte(record.offset - (region ? region.start : REGION_START_B))}, ${record.interval_ms} ms, `
          + `${record.action_count ?? (record.actions ?? []).length} action(s), ${size} B`;
        const row = el('button', {
          type: 'button',
          class: `macro-item${position === state.selected ? ' active' : ''}`,
          text: label,
        });
        row.addEventListener('click', () => {
          state.selected = position;
          draw();
        });
        return row;
      });
      if (!rows.length) rows.push(el('p', { class: 'hint', text: 'no macro record in this region (GG leaves it empty)' }));
      list.replaceChildren(...rows);
    }

    function drawDetail() {
      const record = selectedRecord();
      if (!record) {
        detail.replaceChildren(el('p', {
          class: 'hint',
          text: 'Pick a macro above, or add one. A record is a header (mode, action count, interval, repeat count) and its actions: key push, key release, mouse press, mouse release, wait.',
        }));
        return;
      }
      const modeSelect = select(MODE_OPTIONS.map((option) => ({
        value: option.value,
        label: `${option.label} (mode ${option.value})`,
      })), record.mode);
      modeSelect.addEventListener('change', () => {
        record.mode = Number.parseInt(modeSelect.value, 10);
        record.mode_name = modeName(record.mode);
        draw();
      });
      const headerWordInput = numberInput(headerWordFor(record), 1, U16_MAX, 'action count (the header word: the measured rule is the action count)');
      headerWordInput.addEventListener('change', () => {
        record.header_word = clampNumber(headerWordInput, 1, U16_MAX);
        headerWordInput.value = String(record.header_word);
        drawMeter();
      });
      const intervalInput = numberInput(record.interval_ms, 0, U16_MAX, 'interval in milliseconds');
      intervalInput.addEventListener('change', () => {
        record.interval_ms = clampNumber(intervalInput, 0, U16_MAX);
        intervalInput.value = String(record.interval_ms);
        drawMeter();
      });
      // The repeat count, the flags word's high byte: GG's "play N times". Measured at 69 on the
      // mode-2 record; the other modes carry 1 there.
      const timesInput = numberInput(repeatOf(record), 0, REPEAT_MAX, "repeat count (the flags high byte; GG's \"play N times\")");
      timesInput.addEventListener('change', () => {
        record.flags = withRepeat(record, clampNumber(timesInput, 0, REPEAT_MAX));
        timesInput.value = String(repeatOf(record));
        drawMeter();
      });

      const actionRows = (record.actions ?? []).map((action, index) => {
        const move = (delta) => {
          const to = index + delta;
          if (to < 0 || to >= record.actions.length) return;
          const [moved] = record.actions.splice(index, 1);
          record.actions.splice(to, 0, moved);
          draw();
        };
        return el('div', { class: 'macro-action-row' }, [
          el('span', { class: `macro-action-kind mono mono-${action.kind}`, text: action.kind }),
          el('span', { class: 'mono', text: describeAction(action) }),
          button('up', () => move(-1), { class: 'tiny', title: 'move earlier' }),
          button('down', () => move(1), { class: 'tiny', title: 'move later' }),
          button('remove', () => {
            record.actions.splice(index, 1);
            draw();
          }, { class: 'tiny warn', title: 'remove this action' }),
        ]);
      });

      // Key push: one key code. A push holds no duration - the milliseconds belong to a wait.
      const keySelect = select(keyOptions(record.lastKey ?? 0x04), record.lastKey ?? 0x04);
      const pushRow = el('div', { class: 'macro-add-row' }, [
        el('span', { class: 'mono macro-add-label', text: 'key push' }),
        keySelect,
        button('add', () => {
          const key = Number.parseInt(keySelect.value, 10);
          record.actions.push(makePress(key));
          record.lastKey = key;
          draw();
        }),
      ]);

      // Key release: its own picker, seeded from the last key this record pushed.
      const releaseSelect = select(keyOptions(record.lastKey ?? 0x04), record.lastKey ?? 0x04);
      const releaseRow = el('div', { class: 'macro-add-row' }, [
        el('span', { class: 'mono macro-add-label', text: 'key release' }),
        releaseSelect,
        button('add', () => {
          const key = Number.parseInt(releaseSelect.value, 10);
          record.actions.push(makeRelease(key));
          record.lastKey = key;
          draw();
        }),
      ]);

      // Mouse buttons: one bit per button, mouse 1 to mouse 8 - the set the engine's own reader
      // accepts. The multi-button masks the reads carry are offered after them, labelled by the
      // buttons they press at once.
      const maskOptions = [...MASK_BUTTONS, 0x03, 0x07, 0x30].map((mask) => ({
        value: mask,
        label: `${maskName(mask)} (0x${hex2(mask)})`,
      }));
      const pressMaskSelect = select(maskOptions, 0x03);
      const pressMouseRow = el('div', { class: 'macro-add-row' }, [
        el('span', { class: 'mono macro-add-label', text: 'mouse press' }),
        pressMaskSelect,
        button('add', () => {
          record.actions.push(makePressMouse(Number.parseInt(pressMaskSelect.value, 10)));
          draw();
        }),
      ]);
      const releaseMaskSelect = select(maskOptions, 0x03);
      const releaseMouseRow = el('div', { class: 'macro-add-row' }, [
        el('span', { class: 'mono macro-add-label', text: 'mouse release' }),
        releaseMaskSelect,
        button('add', () => {
          record.actions.push(makeReleaseMouse(Number.parseInt(releaseMaskSelect.value, 10)));
          draw();
        }),
      ]);

      // Wait: one duration.
      const waitInput = numberInput(50, 0, U16_MAX, 'pause in ms');
      const waitRow = el('div', { class: 'macro-add-row' }, [
        el('span', { class: 'mono macro-add-label', text: 'wait' }),
        el('label', { class: 'mono', text: 'ms ' }, waitInput),
        button('add', () => {
          record.actions.push(makeDelay(clampNumber(waitInput, 0, U16_MAX)));
          draw();
        }),
      ]);

      detail.replaceChildren(
        el('h3', {
          class: 'subhead',
          text: `record #${state.selected} - binding byte `
            + `${bindingByte(record.offset - (region ? region.start : REGION_START_B))} `
            + `(${INDEX_UNIT} x the bytes from the block start), action count ${headerWordFor(record)}, `
            + `${recordBytes(record)} bytes, ${record.action_count ?? (record.actions ?? []).length} action(s)`,
        }),
        el('div', { class: 'macro-fields' }, [
          el('label', { class: 'mono', text: 'mode ' }, modeSelect),
          el('label', { class: 'mono', text: 'actions ' }, headerWordInput),
          el('label', { class: 'mono', text: 'interval ms ' }, intervalInput),
          el('label', { class: 'mono', text: 'times ' }, timesInput),
        ]),
        el('p', {
          class: 'hint mono',
          text: `flags 0x${hex2(record.flags & 0xff)}${hex2((record.flags >> 8) & 0xff)} - the low byte is 0 on the mode-1 record and on the mode-4 record carrying a repeat count (the second dump's 00 03) and 1 on the other records of the profile measured (the flag gates what is not known); the high byte is the repeat count, 69 on the "play 69 times every 15 ms" record and 1 on the four others. Mode 1 plays once on release, 2 plays N times every interval, 3 repeats while the key is pressed, 4 toggles auto repeat, 5 toggles hold - all five measured in one profile, each named from GG's UI. A push, a release, a mouse press and a mouse release are 4 bytes each and hold no duration: a wait is what carries milliseconds. The header word is GG's own number per record: 1, 3, 3, 3, 1 on the five records of the profile measured, each matching its actions, and on the second dump GG counts 5 actions in a record whose first triple carries its 20 ms delay on the key-push byte (02 00 14 00, byte 1 = 0 where the grammar wants a page), so the walk stops after the press and the rest is named as bytes that are not records.`,
        }),
        el('div', { class: 'macro-actions-list' }, actionRows),
        pushRow,
        releaseRow,
        pressMouseRow,
        releaseMouseRow,
        waitRow,
        el('p', { class: 'mono hint', text: recordRaw(record) }),
      );
    }

    function draw() {
      drawMeter();
      drawList();
      drawDetail();
    }

    const header = el('header', { class: 'macro-head' }, [
      el('h2', {}, [
        el('span', { text: 'Macro editor ' }),
        el('span', { class: 'tag', text: 'experimental' }),
        el('span', { class: 'mono macro-profile-name', text: ` - ${doc.name ?? 'profile'}` }),
      ]),
      el('p', {
        class: 'mono note',
        text: `macro block ${region.start}..${region.start + region.len} of the ${doc.schema === SCHEMA_B ? 'schema b' : 'profile'} payload, `
          + `${region.fromReport ? 'as the read reported it' : 'the measured schema b block'}; the reads surveyed packed their records from `
          + `${SURVEYED_CHAIN_START} to ${SURVEYED_CHAIN_END}. Header 8 B, and every action 4 B - key push, key release, mouse press, mouse release, wait - packed with no stride. `
          + `A key binds the macro at byte x ${INDEX_UNIT} of the block, so a binding byte reaches ${BINDING_REACH_BYTES} bytes of it. - the bytes behind the records belong to GG and are left alone`,
      }),
      meterText,
    ]);

    const controls = el('div', { class: 'btnrow' }, [
      button('+ add macro', () => {
        state.records.push(makeRecord(state.records.length));
        state.selected = state.records.length - 1;
        draw();
      }),
      button('delete this macro', () => {
        if (!state.records.length) return;
        state.records.splice(state.selected, 1);
        state.selected = Math.max(0, Math.min(state.selected, state.records.length - 1));
        draw();
      }, { class: 'warn' }),
      button('apply to the profile editor', () => {
        const problems = limitProblems(records(), region);
        if (problems.length) {
          setStatus(`not applied: ${problems.join(' - ')}`);
          return;
        }
        onSave(packRecords(records(), region));
        setStatus(`${records().length} record(s) ${plan(records(), region).used} bytes handed to the profile editor`);
      }, { class: 'primary' }),
    ]);

    const disabled = editorDisabledReason(doc);
    if (disabled) {
      root.replaceChildren(
        header,
        el('p', { class: 'hint mono', text: `the macro editor is not available here: ${disabled}` }),
      );
      return { setStatus, refresh: draw };
    }

    root.replaceChildren(
      header,
      controls,
      el('div', { class: 'macro-columns' }, [
        el('div', { class: 'macro-column' }, [el('h3', { class: 'subhead', text: 'records in the region' }), list]),
        el('div', { class: 'macro-column' }, [detail]),
      ]),
      status,
      el('p', {
        class: 'mono note',
        text: embedded
          ? 'The editor changes the open profile only. Nothing here writes a keyboard: the bytes reach the board through `ssgg onboard write`, which recomputes the payload checksum.'
          : 'Applying hands the records to the profile editor window; the main window adopts them as its open file. Nothing here writes a keyboard, and `ssgg onboard write` recomputes the payload checksum.',
      }),
    );
    draw();
    return { setStatus, refresh: draw };
  }

  // ---------------------------------------------------------------- popup session

  // The popup page (`macros.html`) is served by `ssgg gui` with the run token in a meta tag; it reads
  // the profile from the session the main window posted and writes its edits back to the same session.
  async function bootPopup(root) {
    const token = globalThis.SsggLive ? globalThis.SsggLive.tokenFrom(document) : null;
    if (!token) {
      root.replaceChildren(el('p', {
        class: 'hint mono',
        text: 'Open this page from `ssgg gui` (the profile editor\'s Macro editor button). It reads the open profile from the editor session, and that session only exists under `ssgg gui`.',
      }));
      return null;
    }
    const call = globalThis.SsggLive.call;
    let revision = 0;
    let session;
    try {
      session = await call(token, '/api/macros/session');
    } catch (error) {
      root.replaceChildren(el('p', { class: 'hint mono', text: `no profile in the editor session yet: ${error.message}` }));
      return null;
    }
    revision = session.revision;
    const doc = session.profile;
    document.title = `ssgg macro editor - ${doc.name ?? 'profile'}`;
    let channel = null;
    try {
      channel = new BroadcastChannel(CHANNEL);
    } catch {
      channel = null;
    }
    const editor = mount(root, {
      doc,
      embedded: false,
      onSave: async (records) => {
        const edited = applyRecords(doc, records, regionOf(doc));
        try {
          const reply = await call(token, '/api/macros/session', { profile: edited });
          revision = reply.revision;
          const meter = ` (ssgg encodes it as a ${reply.bytes}-byte payload, region ${reply.region.used_bytes}/${reply.region.len} bytes)`;
          editor.setStatus(`applied - the profile editor has it${meter}`);
          if (channel) channel.postMessage({ type: 'macros-applied', revision: reply.revision });
          return { ok: true, reply };
        } catch (error) {
          editor.setStatus(`ssgg refused it: ${error.message}`);
          return { ok: false, error };
        }
      },
    });
    return editor;
  }

  const api = {
    HEADER_LEN, PRESS_LEN, RELEASE_LEN, DELAY_LEN, ACTION_LEN, RECORD_MARKER, RECORD_FLAGS, ACTION_KEY,
    ACTION_DELAY, ACTION_MOUSE, MASK_MAX, MASK_BUTTONS, mouseButton,
    REGION_START_B, REGION_LEN_B, SURVEYED_CHAIN_START, SURVEYED_CHAIN_END,
    SCHEMA_B, MODE_REPEAT,
    MODE_ONCE, MODE_LOOP, MODE_MAX, U16_MAX, INDEX_UNIT, BINDING_BYTE_MAX, BINDING_REACH_BYTES, bindingByte,
    offsetForBindingByte, maskName, makePressMouse, makeReleaseMouse, makePress, makeRelease, makeDelay,
    headerWordFor, maxActions, MODE_OPTIONS, loopFlagOf, repeatOf, flagsOk, withRepeat, MODE_COUNTED,
    regionOf, editorDisabledReason, actionLen, recordBytes, totalBytes, plan, recordRaw, packRecords, modeName,
    normalizeAction, makePress, makeRelease, makeDelay, makeRecord, recordsFromBlock, workingRecords, applyRecords,
    limitProblems, keyLabel, keyOptions, describeAction, mount, bootPopup, CHANNEL,
  };

  globalThis.SsggMacroEditor = api;

  if (typeof document !== 'undefined') {
    const root = document.getElementById('macroEditorBox');
    // Only the popup page carries the marker; the main editor mounts the same UI inline without it.
    if (root && root.hasAttribute && root.hasAttribute('data-ssgg-macro-popup')) bootPopup(root);
  }
})();
