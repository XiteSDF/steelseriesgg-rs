// ssgg macro editor (prototype): the packed macro records of a profile, edited as key push, key
// release and wait instructions. It opens as its own window (`macros.html`) when `ssgg gui` serves the
// page, and the same editor mounts inside the profile editor when a window is not available.
//
// The grammar and every limit here mirror `src/devices/keyboards/apex_7_profile.rs`, which is the
// authority: the record header `0x77 | mode u8 | id u16 | flags u16 | interval u16` is 8 bytes and every
// action is 4 - a key push `0x02 | key u8 | 01 01`, a key release `0x02 | key u8 | 01 00`, a wait
// `0x04 | 00 | ms u16`. A push has no duration of its own: the milliseconds belong to a wait.
//
// Records are packed with no stride and no length word, from payload 3600 in the six reads surveyed
// (wired Apex 7 `0x1612`, schema b, 2026-10-22) - 1524 bytes into the macro block, which starts at 2076
// and is the engine's one opaque 9,600-byte field. GG's own per-macro structures share that block, so
// the records do not fill it, and the chain of the profile that carried the most records ran 608 bytes
// to 4208. The capacity a list has is the block; a list that needs more is refused with its numbers,
// exactly as `encode_macro_records` refuses it.
//
// A macro is named by its id: a `0x71` binding row carries `35 + id - 1`, so ids 1..221 reach bindings
// 35..255 - the list's limit is the binding byte, not a slot count.
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
  const ACTION_LEN = 4;
  // The macro block of a schema b payload: 9,600 bytes from 2076, the engine's one `macro-data-size`
  // field. The six reads surveyed packed their records from 3600, inside it.
  const REGION_START_B = 2076;
  const REGION_LEN_B = 9600;
  const SURVEYED_CHAIN_START = 3600;
  const SURVEYED_CHAIN_END = 4208;
  const SCHEMA_B = 0x000b;
  const MODE_REPEAT = 4;
  const MODE_ONCE = 1;
  const MODE_LOOP = 6;
  const MODE_MAX = 8;
  const U16_MAX = 65_535;
  // A `0x71` binding row names a macro with one byte: `BINDING_BASE + id - 1`.
  const BINDING_BASE = 35;
  const ID_MAX = 221;
  const CHANNEL = 'ssgg-macros';

  // Modes the editor offers. Names come from the codec's `macro_mode_name` and GG's own words; only
  // mode 4 is measured on a read, so the panel says what is measured and what is inferred.
  const MODE_OPTIONS = [
    { value: MODE_REPEAT, label: 'repeat every interval (mode 4)', measured: true },
    { value: MODE_ONCE, label: 'once, interval ms between the actions (mode 1)', measured: true },
    { value: MODE_LOOP, label: 'loop (mode 6, name from GG, unverified)', measured: false },
  ];

  const bindingIndex = (id) => BINDING_BASE + id - 1;

  const hex2 = (byte) => byte.toString(16).padStart(2, '0');

  // The macro record region of a profile: the read's own numbers when the report carries them
  // (`macro_region`, written by `ssgg onboard read`), the measured schema b constants otherwise.
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
    out.push(record.id & 0xff, (record.id >> 8) & 0xff);
    out.push(record.flags & 0xff, (record.flags >> 8) & 0xff);
    out.push(record.interval_ms & 0xff, (record.interval_ms >> 8) & 0xff);
    for (const action of record.actions ?? []) {
      if (action.kind === 'press') {
        out.push(ACTION_KEY, (action.key ?? 0) & 0xff, 0x01, 0x01);
      } else if (action.kind === 'release') {
        out.push(ACTION_KEY, (action.key ?? 0) & 0xff, 0x01, 0x00);
      } else {
        out.push(ACTION_DELAY, 0x00);
        out.push((action.ms ?? 0) & 0xff, ((action.ms ?? 0) >> 8) & 0xff);
      }
    }
    return out.map(hex2).join(' ');
  }

  // Records renumbered as the encoder packs them: the list is packed from the region's first byte, so
  // every record's payload offset, ordinal, binding index, size and raw bytes follow from the order.
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
        index: bindingIndex(record.id),
        mode: record.mode & 0xff,
        mode_name: modeName(record.mode),
        flags: record.flags ?? RECORD_FLAGS,
        flags_ok: (record.flags ?? RECORD_FLAGS) === RECORD_FLAGS,
        interval_ms: record.interval_ms,
        id: record.id,
        action_count: actions.length,
      };
      packed.record_bytes = recordBytes(packed);
      packed.raw = recordRaw(packed);
      at += packed.record_bytes;
      return packed;
    });
  }

  function modeName(mode) {
    if (mode === MODE_REPEAT) return 'repeat';
    if (mode === MODE_ONCE) return 'once';
    return 'unknown';
  }

  function normalizeAction(action) {
    if (action.kind === 'press') return { kind: 'press', key: action.key ?? 0 };
    if (action.kind === 'release') return { kind: 'release', key: action.key ?? 0 };
    return { kind: 'delay', ms: action.ms ?? 0 };
  }

  const makePress = (key) => ({ kind: 'press', key });
  const makeRelease = (key) => ({ kind: 'release', key });
  const makeDelay = (ms) => ({ kind: 'delay', ms });

  // A new record with no actions: the header alone, packed at the end of the list.
  function makeRecord(ordinal) {
    return {
      offset: 0,
      ordinal,
      index: bindingIndex(ordinal + 1),
      flags_ok: true,
      mode: MODE_REPEAT,
      mode_name: modeName(MODE_REPEAT),
      id: Math.min(ordinal + 1, ID_MAX),
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
    // 1..8, id 1..221, interval >= 1) and its actions, read until the grammar stops matching. A header
    // with no action, a header GG's own structures wear, and a record the block end cuts off are all
    // skipped - the bytes behind the records belong to GG, and this grammar invents nothing from them.
    const headerAt = (at) => {
      if (at + HEADER_LEN > end || bytes[at] !== RECORD_MARKER) return null;
      const mode = bytes[at + 1];
      const id = bytes[at + 2] | (bytes[at + 3] << 8);
      const flags = bytes[at + 4] | (bytes[at + 5] << 8);
      const interval = bytes[at + 6] | (bytes[at + 7] << 8);
      if (mode < 1 || mode > MODE_MAX || id < 1 || id > ID_MAX || interval < 1) return null;
      return { mode, id, flags, interval };
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
          index: bindingIndex(header.id),
          flags_ok: header.flags === RECORD_FLAGS,
          mode: header.mode,
          mode_name: modeName(header.mode),
          id: header.id,
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
    records.forEach((record, position) => {
      const where = `record ${position} (${recordBytes(record)} bytes)`;
      if (!Number.isInteger(record.id) || record.id < 1 || record.id > ID_MAX) {
        problems.push(`${where}: id must be a whole number 1..${ID_MAX} - a binding row carries one byte, `
          + '35 + id - 1, so that is how far a key can reach a macro');
      }
      if (!Number.isInteger(record.interval_ms) || record.interval_ms < 1 || record.interval_ms > U16_MAX) {
        problems.push(`${where}: interval must be a whole number 1..${U16_MAX} ms (a 16-bit word, and every `
          + 'record of every read surveyed has one)');
      }
      if (record.flags !== RECORD_FLAGS) {
        problems.push(`${where}: flags 0x${hex2(record.flags & 0xff)}${hex2((record.flags >> 8) & 0xff)} are `
          + 'not 0x0101, the two bytes every record of every read surveyed carries');
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
        if (action.kind !== 'press' && action.kind !== 'release' && action.kind !== 'delay') {
          problems.push(`${at}: is "${action.kind}", and the grammar knows only push, release and wait`);
        }
        if (action.kind === 'delay' && (!Number.isInteger(action.ms) || action.ms < 0 || action.ms > U16_MAX)) {
          problems.push(`${at}: ms must be a whole number 0..${U16_MAX} (16-bit word)`);
        }
        if (action.kind !== 'delay' && (!Number.isInteger(action.key) || action.key < 0 || action.key > 255)) {
          problems.push(`${at}: key code must be a whole number 0..255 (one byte)`);
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
        const label = `macro ${position} - ${record.mode_name} (mode ${hex2(record.mode)}), id ${record.id}, `
          + `${record.interval_ms} ms, ${record.action_count ?? (record.actions ?? []).length} action(s), ${size} B`;
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
          text: 'Pick a macro above, or add one. A record is a header (mode, id, interval) and its actions: key push, key release, wait.',
        }));
        return;
      }
      const modeSelect = select(MODE_OPTIONS.map((option) => ({
        value: option.value,
        label: option.label,
      })), record.mode);
      modeSelect.addEventListener('change', () => {
        record.mode = Number.parseInt(modeSelect.value, 10);
        record.mode_name = modeName(record.mode);
        draw();
      });
      const idInput = numberInput(record.id, 1, ID_MAX, 'macro id');
      idInput.addEventListener('change', () => {
        record.id = clampNumber(idInput, 1, ID_MAX);
        idInput.value = String(record.id);
        drawMeter();
      });
      const intervalInput = numberInput(record.interval_ms, 0, U16_MAX, 'interval in milliseconds');
      intervalInput.addEventListener('change', () => {
        record.interval_ms = clampNumber(intervalInput, 0, U16_MAX);
        intervalInput.value = String(record.interval_ms);
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
          text: `macro ${state.selected} - id ${record.id}, binding 35 + ${record.id} - 1, `
            + `${recordBytes(record)} bytes, ${record.action_count ?? (record.actions ?? []).length} action(s)`,
        }),
        el('div', { class: 'macro-fields' }, [
          el('label', { class: 'mono', text: 'mode ' }, modeSelect),
          el('label', { class: 'mono', text: 'id ' }, idInput),
          el('label', { class: 'mono', text: 'interval ms ' }, intervalInput),
        ]),
        el('p', {
          class: 'hint mono',
          text: `flags 0x${hex2(record.flags & 0xff)}${hex2((record.flags >> 8) & 0xff)} - every record of every read surveyed carries 0x0101, so the editor writes those; `
            + 'mode 4 is measured on a read, mode 1 and 6 are named from GG. A push and a release are 4 bytes each and hold no duration: a wait is what carries milliseconds.',
        }),
        el('div', { class: 'macro-actions-list' }, actionRows),
        pushRow,
        releaseRow,
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
          + `${SURVEYED_CHAIN_START} to ${SURVEYED_CHAIN_END}. Header 8 B, push 4 B, release 4 B, wait 4 B, packed with no stride. `
          + `A macro id 1..${ID_MAX} is bound by row 35 + id - 1. - the bytes behind the records belong to GG and are left alone`,
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
    ACTION_DELAY, REGION_START_B, REGION_LEN_B, SURVEYED_CHAIN_START, SURVEYED_CHAIN_END, SCHEMA_B, MODE_REPEAT,
    MODE_ONCE, MODE_LOOP, MODE_MAX, U16_MAX, BINDING_BASE, ID_MAX, bindingIndex, maxActions, MODE_OPTIONS,
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
