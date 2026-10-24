// ssgg on-board profile JSON editor.
//
// Edits the JSON `ssgg onboard read -o` and `ssgg onboard decode --out` write (schema `a`/`b`) and
// encodes it back for `ssgg onboard encode`. It is a file editor only: it never opens a device and
// writes nothing to a keyboard - sending a profile is the CLI's job (`ssgg onboard write`). The report
// is the whole decoded profile: keyboard identity (product_id, product, firmware - stamped by a device
// read, absent for a file decode), schema, name, lighting, the META toggle byte, the four mapping
// tables, the OLED block, the Rapid Tap block, the macro block and its packed records, the GUID and the
// checksum words. The crc_* fields come from the read and are never recomputed here.
//
// Key identity: GG turns a profile mapping offset into a key with the legacy family's HID -> offset
// table (`lisp/base_apex_legacy.lisp`, `device-offset-lookup-table`) inverted, and its own
// onboard -> config conversion goes through that inverse (`onboard-to-config-button-mapping`).
// `button_mappings` is the base layer, `button_meta_mappings` the META layer; Rapid Tap pairs and the
// lock / META / Win lighting bytes carry raw HID codes.
//
// Every `lisp/*.lisp` citation below is a **decrypted** `*.edevice` device specification: GG ships those
// specs encrypted in `apps\engine\deviceSpecifications\`, and the plaintext is lisp.
//
// The protocol behind these bytes is reverse-engineered from GG captures of one Apex 7 (`0x1612`); its
// read, write and deploy are confirmed on that board, and decode and encode match GG for clean and modified
// profiles. This page writes no keyboard.
// Every label is derived from data in this repo, cited where used.

// Loaded as a plain script (no ES modules): a page opened from the filesystem
// has origin `null`, and browsers refuse module scripts there, so the app
// publishes one namespace object instead of importing.
(function ssggProfileApp() {
'use strict';

// High bit of a Rapid Tap mode byte, reported separately as report_both_keys.
const MASK_MODE_REPORT_BOTH = 0x80;

// HID usage code -> profile mapping offset. -1 = this code has no offset.
const HID2OFF = [
  -1, -1, -1, -1, 11, 52, 36, 27, 26, 35, 43, 51, 66, 59, 67, 75,
  68, 60, 74, 82, 10, 34, 19, 42, 58, 44, 18, 28, 50, 20, 9, 17,
  25, 33, 41, 49, 57, 65, 73, 81, 107, 0, 113, 2, 45, 89, 97, 90,
  98, 106, 99, 83, 91, 1, 76, 84, 92, 3, 8, 16, 24, 32, 40, 48,
  56, 64, 72, 80, 88, 96, 104, 112, 14, 114, 22, 30, 115, 70, 78, 118,
  109, 117, 116, 38, 46, 54, 62, 23, 95, 63, 71, 79, 31, 39, 47, 86,
  94, 15, 119, 87, 12, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
  -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
  -1, -1, -1, -1, -1, 55, -1, 100, 61, 105, 53, 37, -1, -1, -1, -1,
  -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
  -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
  -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
  -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
  -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
  5, 4, 21, 13, 101, 108, 77, 85, -1, -1, -1, -1, -1, -1, -1, -1,
  93, 120, 121, 122, 123, 124, 125, 126, 127, -1, -1, -1, -1, -1, -1, -1,
];

// offset -> HID usage code, inverted from HID2OFF (GG's hid-lookup-table).
const OFF2HID = (() => {
  const out = new Array(128).fill(null);
  for (let hid = 0; hid < HID2OFF.length; hid += 1) {
    const off = HID2OFF[hid];
    if (off >= 0) out[off] = hid;
  }
  return out;
})();

// A mapping slot's own code is what the file carries (slot 93 carries GG's code 0xF0), but anywhere
// that names the key of a slot names the board cap: slot 93 is the Meta cap.
function boardHid(code) {
  return code === META_KEY_GG_CODE ? META_KEY_HID : code;
}

// Names for HID usage page 0x07 codes. Unlisted codes are shown as hex.
const KEY_NAMES = {
  0x04: 'A', 0x05: 'B', 0x06: 'C', 0x07: 'D', 0x08: 'E', 0x09: 'F', 0x0a: 'G',
  0x0b: 'H', 0x0c: 'I', 0x0d: 'J', 0x0e: 'K', 0x0f: 'L', 0x10: 'M', 0x11: 'N',
  0x12: 'O', 0x13: 'P', 0x14: 'Q', 0x15: 'R', 0x16: 'S', 0x17: 'T', 0x18: 'U',
  0x19: 'V', 0x1a: 'W', 0x1b: 'X', 0x1c: 'Y', 0x1d: 'Z',
  0x1e: '1', 0x1f: '2', 0x20: '3', 0x21: '4', 0x22: '5', 0x23: '6', 0x24: '7',
  0x25: '8', 0x26: '9', 0x27: '0',
  0x28: 'Enter', 0x29: 'Esc', 0x2a: 'Bksp', 0x2b: 'Tab', 0x2c: 'Space',
  0x2d: '-', 0x2e: '=', 0x2f: '[', 0x30: ']', 0x31: '\\', 0x32: 'NONUS',
  0x33: ';', 0x34: "'", 0x35: '`', 0x36: ',', 0x37: '.', 0x38: '/', 0x39: 'Caps',
  0x3a: 'F1', 0x3b: 'F2', 0x3c: 'F3', 0x3d: 'F4', 0x3e: 'F5', 0x3f: 'F6',
  0x40: 'F7', 0x41: 'F8', 0x42: 'F9', 0x43: 'F10', 0x44: 'F11', 0x45: 'F12',
  0x46: 'PrtSc', 0x47: 'ScrLk', 0x48: 'Pause',
  0x49: 'Ins', 0x4a: 'Home', 0x4b: 'PgUp', 0x4c: 'Del', 0x4d: 'End',
  0x4e: 'PgDn', 0x4f: 'Right', 0x50: 'Left', 0x51: 'Down', 0x52: 'Up',
  0x53: 'NumLk', 0x54: 'Ke /', 0x55: 'Ke *', 0x56: 'Ke -', 0x57: 'Ke +',
  0x58: 'Ke Ent', 0x59: 'Ke 1', 0x5a: 'Ke 2', 0x5b: 'Ke 3', 0x5c: 'Ke 4',
  0x5d: 'Ke 5', 0x5e: 'Ke 6', 0x5f: 'Ke 7', 0x60: 'Ke 8', 0x61: 'Ke 9',
  0x62: 'Ke 0', 0x63: 'Ke .', 0x64: 'Ke ,', 0x65: 'Menu',
  0x85: 'Int1', 0x86: 'Int2', 0x87: 'Int3', 0x88: 'Int4', 0x89: 'Int5',
  0x8a: 'Int6', 0x8b: 'Int7',
  0xe0: 'LCtrl', 0xe1: 'LShft', 0xe2: 'LAlt', 0xe3: 'LWin', 0xe4: 'RCtrl',
  0xe5: 'RShft', 0xe6: 'RAlt', 0xe7: 'RWin',
  0xf0: 'Fn F0', 0xf1: 'Fn F1', 0xf2: 'Fn F2', 0xf3: 'Fn F3', 0xf4: 'Fn F4',
  0xf5: 'Fn F5', 0xf6: 'Fn F6', 0xf7: 'Fn F7', 0xf8: 'Fn F8',
  // The wheel keys above the numpad: wheel_mappings entries, not 128-table offsets.
  0x100: 'Wheel ↑', 0x101: 'Wheel ↓',
};

// functionByte is optional: with 0x61 (media_key) the code is a consumer-page usage and
// reads through MEDIA_NAMES, everywhere else it is a keyboard-page usage.
function keyName(hid, functionByte) {
  const media = functionByte === 0x61 ? MEDIA_NAMES[hid] : undefined;
  const n = media ?? KEY_NAMES[hid];
  return n ? n : `0x${hex2(hid)}`;
}

// Board legends. The numpad is drawn as its own block, so its caps carry the plain keypad
// legends (no `Ke ` prefix); `Ke 1` and friends stay in KEY_NAMES for the lists and tooltips.
const LAYOUT_LABELS = {
  0x53: 'NumLk', 0x54: '/', 0x55: '*', 0x56: '-', 0x57: '+', 0x58: 'Enter',
  0x59: '1', 0x5a: '2', 0x5b: '3', 0x5c: '4', 0x5d: '5', 0x5e: '6',
  0x5f: '7', 0x60: '8', 0x61: '9', 0x62: '0', 0x63: '.', 0x64: ',',
};

// The arrow keys are drawn as glyphs, centred: the words Up/Down/Left/Right do not read
// as directions at 22px, and the glyph takes no more room than the word.
const ARROW_GLYPHS = { 0x4f: '→', 0x50: '←', 0x51: '↓', 0x52: '↑' };

function boardLabel(hid) {
  const n = LAYOUT_LABELS[hid];
  return n ?? keyName(hid);
}

// Labels are centred, and a 1-unit cap is UNIT-GAP wide: 42px at UNIT 48, which takes 5
// glyphs at 11px and needs a smaller font for 6 and 7.
function labelClass(text) {
  if (text.length <= 5) return '';
  return text.length === 6 ? 'long' : 'xlong';
}

// Shrink class for the sub line: 9.5px, and the tightest place it lands is the half-height
// wheel cap (42 px wide), where 8 characters is the fit.
function subClass(text) {
  if (text.length <= 8) return '';
  return text.length === 9 ? 'long' : 'xlong';
}

// The sub line is clipped to the cap box (the clipPath in buildBoard), so a line longer than the cap
// is cut by the clip. Cut it here too, at the width its shrink class draws it at, so the text the cap
// paints is the text the cap holds and a truncated line reads as truncated. The advance is the mono
// glyph width of each class (9.5, 8.5, 7.2 px in style.css) against the cap box width.
function fitSubText(body, suffix, cls, widthPx) {
  const perGlyph = cls === '' ? 5.7 : cls === 'long' ? 5.1 : 4.3;
  const max = Math.max(1, Math.floor(widthPx / perGlyph));
  const full = `${body}${suffix}`;
  if (full.length <= max) return full;
  // the suffix is the Rapid Tap marker: the one part of the line that must survive a tight cap. When
  // it leaves no room for a truncated body, paint the marker alone - a key marked for Rapid Tap is
  // the information, the binding text is in the tooltip and the key panel.
  if (suffix && max < suffix.length + 2) return suffix;
  const room = Math.max(1, max - suffix.length - 1);
  return `${body.slice(0, room)}\u2026${suffix}`;
}

// Mirrors function_name in src/devices/keyboards/apex_7_profile.rs.
const FUNCTION_NAMES = {
  0x00: 'deactivated', 0x01: 'mouse_left', 0x02: 'mouse_right',
  0x03: 'mouse_middle', 0x04: 'mouse_button_4', 0x05: 'mouse_button_5',
  0x06: 'mouse_button_6', 0x07: 'mouse_button_7', 0x08: 'mouse_button_8',
  0x30: 'cpi', 0x31: 'wheel_up', 0x32: 'wheel_down', 0x33: 'ac_pan_left',
  0x34: 'ac_pan_right', 0x51: 'combo_key', 0x61: 'media_key', 0x62: 'meta',
  0x71: 'macro',
};

function functionName(fn) {
  return FUNCTION_NAMES[fn] || 'unknown';
}

// The binding a user picks, in the order GG's own UI groups them. Each group writes one of a set
// of function bytes and owns the option list that goes with it; the labels are what a binding
// means to a person, the bytes are what the JSON stores.
const MOUSE_FUNCTIONS = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x30, 0x31, 0x32, 0x33, 0x34];
const MOUSE_NAMES = {
  0x01: 'Left button', 0x02: 'Right button', 0x03: 'Middle button',
  0x04: 'Button 4 (back)', 0x05: 'Button 5 (forward)', 0x06: 'Button 6', 0x07: 'Button 7',
  0x08: 'Button 8', 0x30: 'CPI', 0x31: 'Wheel up', 0x32: 'Wheel down',
  0x33: 'Pan left', 0x34: 'Pan right',
};
// The actions of the meta function. `META_ACTION_NAMES` is what the Meta action select lists,
// `META_CAP_LABELS` is the short word the cap prints. 0x05 is Lock Win (it locks the Windows key) and
// 0x0A is Rapid Tap - the same reading GG's own UI uses, and the stock META layer agrees with the first
// of them: the slot GG marks with 0x05 is slot 13, LWin. None of it is measured in writes.
// Lock Win and the META toggle are two separate things: Lock Win is a meta action, a binding byte
// (`0x62` with code 1 = `0x05`); the META toggle is the profile byte at `meta_toggle_hid`, which names
// the key GG switches META mode with and carries no binding byte at all.
const META_ACTION_NAMES = {
  0x01: 'Profile Switch (F9)', 0x02: 'Macro (F10)', 0x03: 'Brightness Down (F11)',
  0x04: 'Brightness Up (F12)', 0x05: 'Lock Win', 0x0a: 'Rapid Tap',
};
const META_CAP_LABELS = {
  0x01: 'Profile', 0x02: 'Macro', 0x03: 'Bright down', 0x04: 'Bright up',
  0x05: 'Lock Win', 0x0a: 'Rapid Tap',
};
// The meta function and its Lock Win action. In every read this page decodes, the META-layer entry GG
// marks is `0x62` with code 1 = `0x05` - the Lock Win action - on slot 13 = LWin (a stock slot read) and
// on slots 13 and 35 in a wired user profile read. It is a binding byte pair: nothing in the
// reads says it is the key that switches META mode. That key is named by the profile byte at
// `meta_toggle_hid` and by nothing else: a wired read names 0x10, the M key, whose base entry stays
// self-bound (51 10 00 00 00) and whose META entry stays all zero; the factory byte is 0xF0, GG's code
// for the Meta cap (META_KEY_GG_CODE). [EXPERIMENTAL] read-side evidence; an encode carrying it has been
// written on an Apex 7 and read back.
const META_TOGGLE_FUNCTION = 0x62;
const META_LOCK_WIN_ACTION = 0x05;
// The Meta cap is usage 0x65 - electrically the HID Application key, the keystroke that opens the
// Context Menu - and the legacy table has no slot under that usage (-1 at 0x65, the crate's
// hid_to_offset(0x65) is None: no profile key is named Meta). The key GG switches META mode with is slot
// 93, the slot whose table code is GG's 0xF0 - the code this page's usage table names Fn F0, the M key
// of the Apex 7. Every stock read this page decodes carries meta_toggle_hid 0xF0 with slot 93 bound to
// itself (51 F0 00 00 00), and a wired read binds slot 93 to 0x65 while its byte 0x10 names the M
// key. The page draws the cap at usage 0x65 and binds it through slot 93, so it edits like any other key
// (the crate's meta_key_offset carries the same alias).
const META_KEY_HID = 0x65;
const META_KEY_GG_CODE = 0xf0;
const META_KEY_SLOT = 93;
// The slot a usage binds on: the table's own slot, plus the Meta cap alias. HID2OFF and OFF2HID stay
// byte-for-byte the Rust table; this alias is where the two representations part, and it is the alias
// the crate's meta_key_offset carries.
function hidSlot(hid) {
  const off = HID2OFF[hid];
  if (off !== undefined && off >= 0) return off;
  return hid === META_KEY_HID ? META_KEY_SLOT : -1;
}
const BINDING_GROUPS = [
  { id: 'none', label: 'Deactivated', functions: [0x00] },
  { id: 'mouse', label: 'Mouse Key', functions: MOUSE_FUNCTIONS },
  { id: 'keyboard', label: 'Keyboard Key', functions: [0x51] },
  { id: 'media', label: 'Media Key', functions: [0x61] },
  { id: 'meta', label: 'Meta', functions: [0x62] },
  { id: 'macro', label: 'Macro', functions: [0x71] },
];

function bindingGroup(functionByte) {
  return BINDING_GROUPS.find((group) => group.functions.includes(functionByte)) ?? null;
}

// One option list per group. Every option carries the byte it writes, so the panel can show a
// name and the file keeps the number.
function optionSelect(options, selected, cls) {
  const select = el('select', { class: cls ?? 'code-select' });
  for (const option of options) {
    select.appendChild(el('option', { value: String(option.value), text: option.label }));
  }
  if (!options.some((option) => option.value === selected)) {
    select.appendChild(el('option', {
      value: String(selected),
      text: `0x${hex2(selected)} (a value this list does not name)`,
    }));
  }
  select.value = String(selected);
  return select;
}

// The six binding types GG's UI lists. Meta Toggle is not one of them: it is the 0x05 action of the
// Meta group, so it is offered by metaActionOptions, under Meta.
function bindingGroupOptions(functionByte) {
  const options = BINDING_GROUPS.map((group) => ({ value: group.id, label: group.label }));
  if (!bindingGroup(functionByte)) {
    options.push({ value: 'raw', label: `0x${hex2(functionByte)} ${functionName(functionByte)} - this editor has no group for it` });
  }
  return options;
}

function mouseActionOptions(selected) {
  const options = MOUSE_FUNCTIONS.map((fn) => ({ value: fn, label: `${MOUSE_NAMES[fn]} (0x${hex2(fn)})` }));
  if (!options.some((option) => option.value === selected)) {
    options.push({ value: selected, label: `0x${hex2(selected)} ${functionName(selected)}` });
  }
  return options;
}

function keyboardKeyOptions(selected) {
  const options = Object.keys(KEY_NAMES).map(Number)
    .filter((usage) => usage < 0x100 && usage !== 0)
    .sort((a, b) => a - b)
    .map((usage) => ({
      // 0x65 is the Meta cap's own keystroke: the HID Application usage, the key that types the
      // Context Menu. GG's own binding list calls it Menu, so the select uses that word: 'Meta' alone
      // reads like the META toggle key.
      value: usage,
      label: `${usage === META_KEY_HID ? 'Menu' : KEY_NAMES[usage]} (0x${hex2(usage)})`,
    }));
  options.unshift({ value: 0, label: 'none (0x00)' });
  if (!options.some((option) => option.value === selected)) {
    options.push({ value: selected, label: `0x${hex2(selected)} (no name for this usage${selected === META_KEY_GG_CODE ? ' - the code GG carries for the Meta cap slot' : ''})` });
  }
  return options;
}

function mediaKeyOptions(selected) {
  const codes = [...new Set([...CONSUMER_TARGETS, ...Object.keys(MEDIA_NAMES).map(Number)])].sort((a, b) => a - b);
  const options = codes.map((code) => ({ value: code, label: `${MEDIA_NAMES[code] ?? 'consumer code'} (0x${hex2(code)})` }));
  options.unshift({ value: 0, label: 'none (0x00)' });
  if (!options.some((option) => option.value === selected)) {
    options.push({ value: selected, label: `0x${hex2(selected)} (no name for this consumer code)` });
  }
  return options;
}

// The Meta group's own list: the meta actions of function 0x62. Lock Win (0x05) is one of them and
// writes only this entry's code 1. The META toggle is not in this list - it is the profile byte at
// meta_toggle_hid and has its own row in the binding window (the two are separate options).
function metaActionOptions(selected) {
  const options = Object.keys(META_ACTION_NAMES).map(Number).sort((a, b) => a - b)
    .map((code) => ({ value: code, label: `${META_ACTION_NAMES[code]} (0x${hex2(code)})` }));
  if (!options.some((option) => option.value === selected)) {
    options.push({ value: selected, label: `meta action 0x${hex2(selected)} (no name for this action: the META layer of a read carries 62 with an action byte, and this page names the ones the reads show - 01 Profile Switch, 02 Macro, 03 Brightness Down, 04 Brightness Up, 05 Lock Win, 0A Rapid Tap)` });
  }
  return options;
}

function macroBindingOptions(doc, selected) {
  // The value a 0x71 slot writes is the binding byte, in its third code byte: the byte counts 4-byte
  // units from the block start, so it reaches the macro packed `byte x 4` bytes into the block.
  // Short labels only: a select is as wide as its longest option, and a long macro is a long
  // keystroke list. The whole record is shown under the select (macroDetailRows).
  const options = macroRecords(doc).map((record, position) => {
    const ordinal = macroRecordOrdinal(record);
    const byte = macroBindingByte(record.offset - macroRegionStart(doc));
    const mode = record.mode_name ?? MACRO_MODE_NAMES[record.mode] ?? `mode 0x${hex2(record.mode)}`;
    const actions = (record.actions ?? []).length;
    return {
      value: byte,
      label: `Record #${ordinal} - ${mode}${record.mode === 4 ? ` ${record.interval_ms} ms` : ''} - ${actions} action(s) - binding byte ${byte}`,
    };
  });
  if (!options.some((option) => option.value === selected)) {
    const known = macroRecordFor(doc, selected);
    options.push({
      value: selected,
      label: known
        ? `Record #${macroRecordOrdinal(known)} - binding byte ${selected}, the macro at ${macroOffsetForBindingByte(selected)} bytes into the block`
        : `Binding byte ${selected}: no macro starts at ${macroOffsetForBindingByte(selected)} bytes into the block`,
    });
  }
  return options;
}

// The binding a factory profile has on a slot, measured from a stock slot read (`config_3.bin`, the
// read with an empty macro block) and reproduced byte for byte for all 256
// entries of both layers by this rule plus the exceptions below: every key the 128-table maps is
// bound to itself (0x51 with its own usage as the first code), the slots the table leaves unused
// are all zero, GG's media cluster carries its consumer codes, two slots point at 0x90/0x91, and
// the META layer holds GG's five defaults.
const FACTORY_BASE_ZEROS = [6, 7, 102, 103, 110, 111, 121, 122, 123, 127];
const FACTORY_BASE_MEDIA = { 120: 0xE2, 124: 0xCD, 125: 0xB5, 126: 0xB6 };
const FACTORY_BASE_EXTRA = { 29: '91 00 00 00', 69: '90 00 00 00' };
const FACTORY_META = {
  13: '05 00 00 00', 72: '01 00 00 00', 80: '02 01 00 00', 88: '03 01 00 00', 96: '04 01 00 00',
};
// The wheel lists' factory binding, measured in the stock slot read and byte-for-byte the same in
// every read this page decodes (the stock slot, config_1, the remapped slot): GG pairs the scroll
// wheel with the numpad + and / keys (usages 0x29 and 0x35), binds Media Vol+ on the up row and
// Media Vol- on the down row, and leaves both META-layer rows deactivated.
const FACTORY_WHEEL_BASE = ['E9 00 00 00', 'EA 00 00 00'];
const FACTORY_WHEEL_META = ['00 00 00 00', '00 00 00 00'];

// A META-layer entry bound as the toggle: the meta function carrying action 0x05.
function isLockWinEntry(entry) {
  return !!entry && entry.function === META_TOGGLE_FUNCTION && mappingCodes(entry).list[0] === META_LOCK_WIN_ACTION;
}

// The binding GG's own toggle byte is read from, in slot order: the first entry of the META layer
// marked 0x62 / 05, and the key on the board that slot maps to.
function lockWinMarkers(doc) {
  const out = [];
  (doc.button_meta_mappings ?? []).forEach((entry, offset) => {
    if (isLockWinEntry(entry)) out.push({ offset, listName: 'button_meta_mappings', hid: boardHid(OFF2HID[offset]) ?? null, entry });
  });
  return out;
}

// The marker GG's read carries first - the META-layer entry bound 0x62 with code 1 = 05.
function lockWinBinding(doc) {
  for (const [offset, entry] of (doc.button_meta_mappings ?? []).entries()) {
    if (isLockWinEntry(entry)) return { offset, listName: 'button_meta_mappings', hid: boardHid(OFF2HID[offset]) ?? null, entry };
  }
  return null;
}

// Whether this profile's byte is GG's code for the Meta cap, i.e. the cap is the toggle key.
// The key on the board the profile's byte names: GG's code 0xF0 for the Meta cap reads back as the cap
// itself (usage 0x65), any other byte is the usage of the key it names (0x10 in a wired read is
// the M key).
function metaToggleByte(byte) {
  return Number(byte) === META_KEY_GG_CODE ? META_KEY_HID : Number(byte);
}

// The byte a key is named by in meta_toggle_hid: GG's code for the Meta cap, its own usage for
// everything else.
function toggleByteFor(hid) {
  return Number(hid) === META_KEY_HID ? META_KEY_GG_CODE : Number(hid);
}

// The META mapping mask (lighting.meta_mapped_elements: 32 bytes at payload offset 42, 256 bits, the
// bit index is a key's own HID usage). The rule is measured, not invented: in 8 of the 9 profile reports
// decoded so far - this page's three fixtures and five wired reads of one `0x1612` board, taken as the
// board stored them and as GG stored them - the stored bytes are exactly what this returns: a bit is set
// for every key whose META-layer entry carries a binding (function byte != 0), the key named by
// meta_toggle_hid is not set, and the wheel's two entries are never in it (no slot, no usage). The ninth
// report is a copy of a read edited on this page: its mask is the read's stale bytes and misses the
// binding that edit added, which is why the mask is derived here and not typed.
// [EXPERIMENTAL] measured bit-for-bit in the reads this page decodes; a write carrying the derived mask has
// applied on an Apex 7 (2026-10-20).
function computeMetaMask(doc) {
  const bits = new Array(256).fill(0);
  const byte = Number(doc.meta_toggle_hid ?? 0);
  const toggleHid = byte > 0 ? metaToggleByte(byte) : null;
  (doc.button_meta_mappings ?? []).forEach((entry, offset) => {
    if (!entry || entry.function === 0) return;
    const hid = OFF2HID[offset];
    if (hid === null || hid === undefined || hid < 0 || hid > 255) return;
    if (toggleHid !== null && hid === toggleHid) return;
    bits[hid] = 1;
  });
  return hexOf(Array.from({ length: 32 }, (_, i) => bits.slice(i * 8, i * 8 + 8).reduce((value, bit, j) => value | (bit << j), 0)));
}

// How many bits a stored mask disagrees with the derived one by, for the row's note.
function metaMaskDiffers(stored, derived) {
  const a = parseHex(String(stored ?? ''));
  const b = parseHex(String(derived ?? ''));
  if (!a || !b) return -1;
  let n = 0;
  for (let i = 0; i < Math.max(a.length, b.length); i += 1) {
    const x = (a[i] ?? 0) ^ (b[i] ?? 0);
    for (let bit = 0; bit < 8; bit += 1) if ((x >> bit) & 1) n += 1;
  }
  return n;
}


function factoryEntry(listName, offset) {
  if (listName === 'wheel_mappings') {
    return { function: 0x61, key_codes: FACTORY_WHEEL_BASE[offset] ?? '00 00 00 00' };
  }
  if (listName === 'wheel_meta_mappings') {
    const codes = FACTORY_WHEEL_META[offset] ?? '00 00 00 00';
    return { function: codes === '00 00 00 00' ? 0x00 : 0x61, key_codes: codes };
  }
  if (listName === 'button_meta_mappings') {
    const codes = FACTORY_META[offset];
    return { function: codes ? 0x62 : 0x00, key_codes: codes ?? '00 00 00 00' };
  }
  const extra = FACTORY_BASE_EXTRA[offset];
  if (extra) return { function: 0x51, key_codes: extra };
  const media = FACTORY_BASE_MEDIA[offset];
  if (media !== undefined) return { function: 0x61, key_codes: `${hex2(media)} 00 00 00` };
  if (FACTORY_BASE_ZEROS.includes(offset)) return { function: 0x00, key_codes: '00 00 00 00' };
  const own = OFF2HID[offset];
  if (own === null || own === undefined) return { function: 0x00, key_codes: '00 00 00 00' };
  return { function: 0x51, key_codes: `${hex2(own)} 00 00 00` };
}

function hex2(byte) {
  return (byte & 0xff).toString(16).padStart(2, '0').toUpperCase();
}

function hexOf(bytes) {
  return bytes.map(hex2).join(' ');
}

// "29 00 00" -> [0x29, 0, 0]. Tolerates the 3-byte and 4-byte shapes the
// decoder has emitted, and empty strings.
function parseHex(text) {
  const out = [];
  for (const tok of String(text ?? '').trim().split(/\s+/)) {
    if (!tok) continue;
    const v = Number.parseInt(tok, 16);
    if (Number.isNaN(v)) return null;
    out.push(v & 0xff);
  }
  return out;
}

function colorToHex(color) {
  return `#${hex2(color.r)}${hex2(color.g)}${hex2(color.b)}`;
}

function hexToColor(value) {
  const m = /^#?([0-9a-fA-F]{6})$/.exec(String(value).trim());
  if (!m) return null;
  const n = Number.parseInt(m[1], 16);
  return { r: (n >> 16) & 0xff, g: (n >> 8) & 0xff, b: n & 0xff };
}

// ANSI full-size geometry in key units: {hid, w, h, x, y}.
// [hid, width, height, xGapBefore]. The main block is exactly 15 units wide in every row
// from Bksp down, so Backspace, the backslash, Enter, RShift and RCtrl share the right
// edge; the nav block starts at 16 and the numpad at 20 (one unit clear of the nav column and the
// arrows) in every row they appear in. PrtSc/ScrLk/Pause sit above Ins/Home/PgUp. The F-row groups:
// F1-F4 above 2-5, F5-F8 from x 6.5 (half way between 6 and 7) to 10.5 (half way between 9 and 0),
// F9-F12 from x 11, so F9 sits above `-` and F12 ends on the 15-unit edge with Backspace.
// Ke -, Ke +, Ke Enter share the numpad's fourth column (x 23); the Up arrow stays in the
// inverted-T, above Down.
const LAYOUT_ROWS = [
  // Esc, F1-F12 in four groups, PrtSc/ScrLk/Pause
  { y: 0, keys: [[0x29, 1], [0x3a, 1, 1, 1], [0x3b, 1], [0x3c, 1], [0x3d, 1], [0x3e, 1, 1, 0.5], [0x3f, 1], [0x40, 1], [0x41, 1], [0x42, 1, 1, 0.5], [0x43, 1], [0x44, 1], [0x45, 1], [0x46, 1, 1, 1], [0x47, 1], [0x48, 1]] },
  // numbers, nav top row, numpad top row
  { y: 1.3, keys: [[0x35, 1], [0x1e, 1], [0x1f, 1], [0x20, 1], [0x21, 1], [0x22, 1], [0x23, 1], [0x24, 1], [0x25, 1], [0x26, 1], [0x27, 1], [0x2d, 1], [0x2e, 1], [0x2a, 2], [0x49, 1, 1, 1], [0x4a, 1], [0x4b, 1], [0x53, 1, 1, 1], [0x54, 1], [0x55, 1], [0x56, 1]] },
  // Q row, nav second row, numpad 7-9 and the tall +
  { y: 2.3, keys: [[0x2b, 1.5], [0x14, 1], [0x1a, 1], [0x08, 1], [0x15, 1], [0x17, 1], [0x1c, 1], [0x18, 1], [0x0c, 1], [0x12, 1], [0x13, 1], [0x2f, 1], [0x30, 1], [0x31, 1.5], [0x4c, 1, 1, 1], [0x4d, 1], [0x4e, 1], [0x5f, 1, 1, 1], [0x60, 1], [0x61, 1], [0x57, 1, 2]] },
  // home row, numpad 4-6 and the tall Enter
  { y: 3.3, keys: [[0x39, 1.75], [0x04, 1], [0x16, 1], [0x07, 1], [0x09, 1], [0x0a, 1], [0x0b, 1], [0x0d, 1], [0x0e, 1], [0x0f, 1], [0x33, 1], [0x34, 1], [0x28, 2.25], [0x5c, 1, 1, 5], [0x5d, 1], [0x5e, 1]] },
  // Z row, the up arrow, numpad 1-3
  { y: 4.3, keys: [[0xe1, 2.25], [0x1d, 1], [0x1b, 1], [0x06, 1], [0x05, 1], [0x19, 1], [0x11, 1], [0x10, 1], [0x36, 1], [0x37, 1], [0x38, 1], [0xe5, 2.75], [0x52, 1, 1, 2], [0x59, 1, 1, 2], [0x5a, 1], [0x5b, 1], [0x58, 1, 2]] },
  // modifiers, arrows bottom, numpad 0 and .
  // LWin sits between LCtrl and LAlt here, which is the order of the caps on the board. The binding
  // a cap carries comes from its mapping offset, so the drawing order changes nothing about the file.
  { y: 5.3, keys: [[0xe0, 1.25], [0xe3, 1.25], [0xe2, 1.25], [0x2c, 6.25], [0xe6, 1.25], [0xe7, 1.25], [0x65, 1.25], [0xe4, 1.25], [0x50, 1, 1, 1], [0x51, 1], [0x4f, 1], [0x62, 2, 1, 1], [0x63, 1]] },
];

const LAYOUT = (() => {
  const keys = [];
  for (const row of LAYOUT_ROWS) {
    let x = 0;
    for (const spec of row.keys) {
      const [hid, w = 1, h = 1, gapBefore = 0] = spec;
      x += gapBefore;
      keys.push({ hid, w, h, x, y: row.y });
      x += w;
    }
  }
  return keys;
})();

const LAYOUT_HIDS = new Set(LAYOUT.map((k) => k.hid));

// Mapped codes the ANSI shape above has no place for (Int'l keys, the Fn
// cluster, the non-US backslash). Shown in their own strip so no offset of a
// 128-entry table is hidden.
const EXTRA_HIDS = (() => {
  const out = [];
  for (let hid = 0; hid < HID2OFF.length; hid += 1) {
    if (HID2OFF[hid] >= 0 && !LAYOUT_HIDS.has(hid)) out.push(hid);
  }
  return out;
})();

const MAPPING_COUNT = 128;

// Key codes per mapping entry: GG's `key_mapping` is a function byte plus four key codes.
const CODE_COUNT = 4;
// Payload lengths per schema, mirroring PAYLOAD_LEN_A / PAYLOAD_LEN_B in the Rust codec. The read fills
// the whole profile region (PROFILE_REGION_BYTES), so a read report carries bytes behind the payload and
// a payload-only file does not: the page names that split, and edits none of those bytes.
const PAYLOAD_LEN_A = 11_648;
const PAYLOAD_LEN_B = 11_696;
const PROFILE_REGION_BYTES = 40_960;
// Macro block length, mirroring MACRO_DATA_LEN in the Rust codec. The engine declares the field an opaque
// `bytearray` of `(macro-data-size)` = 9600 (`lisp/apex_7.lisp:62, 101`, `lisp/base_apex_legacy.lisp:109`), so a
// write sends the block's bytes back verbatim and the parsed record list under it is display only.
const MACRO_DATA_LEN = 9_600;
// Where the schema b macro block starts on the payload.
const MACRO_DATA_OFFSET_B = 2_076;

function payloadLenForSchema(schema) {
  const word = Number(schema) & 0xffff;
  if (word === 0x000a) return PAYLOAD_LEN_A;
  if (word === 0x000b) return PAYLOAD_LEN_B;
  return null;
}

function payloadSplit(doc) {
  const len = payloadLenForSchema(doc.schema);
  const payload = len === null ? `schema 0x${hex2(doc.schema & 0xff)}` : `payload ${len}`;
  return Number(doc.extra_len) ? `${payload} + extra region ${Number(doc.extra_len)}` : `${payload} (no extra region)`;
}

// Consumer-page codes GG's own factory bindings put in mapping slots (media keys, the wheel
// cluster's brightness/volume keys). They are legal binding targets with no key at that usage.
// SteelSeries product ID -> model name. This mirrors `device_name_from_product_id` in
// `src/devices/mod.rs` (which mirrors GG's firmware registry, the same table the crate's
// `product_ids` constants come from): `selftest.mjs` parses the Rust match and the constants and
// compares them with this table entry by entry, so the page cannot drift from the crate.
// The name is a label: two IDs share a name where GG ships one model as two IDs, and an ID the
// crate does not list has no entry here at all.
const PRODUCT_NAMES = {
  0x0616: 'Apex M750',
  0x1252: 'Arctis Pro',
  0x1290: 'Arctis Pro Wireless',
  0x12AA: 'Arctis 5',
  0x12AD: 'Arctis 1 / Arctis 7 (2018)',
  0x12B3: 'Arctis 1 Wireless',
  0x12C2: 'Arctis 9',
  0x12CB: 'Arctis Nova Pro',
  0x12CF: 'Arctis 7 (2019)',
  0x12E0: 'Arctis Nova Pro Wireless (TX)',
  0x12E4: 'Arctis Nova Pro Wireless',
  0x12EA: 'Arctis Nova 5',
  0x12EC: 'Arctis Nova 3',
  0x12EE: 'Arctis Nova 1',
  0x1610: 'Apex Pro',
  0x1612: 'Apex 7',
  0x1614: 'Apex Pro TKL',
  0x1616: 'Apex 150',
  0x1618: 'Apex 7 TKL',
  0x161A: 'Apex 3',
  0x161C: 'Apex 5',
  0x161E: 'Apex Pro Mini',
  0x1620: 'Apex 9 Mini',
  0x1622: 'Apex 3 TKL',
  0x1624: 'Apex Pro Mini Wireless (Dongle)',
  0x1626: 'Apex Pro Mini Wireless',
  0x1628: 'Apex Pro TKL (2023)',
  0x1630: 'Apex Pro TKL (2023) Wireless',
  0x1632: 'Apex Pro TKL (2023) Wireless',
  0x1634: 'Apex 9 TKL',
  0x1640: 'Apex Pro (2024)',
  0x1642: 'Apex Pro TKL (2024)',
  0x1644: 'Apex Pro TKL Wireless (2024) (Dongle)',
  0x1646: 'Apex Pro TKL Wireless (2024)',
  0x1648: 'Apex Pro Mini (2024)',
  0x1650: 'Apex 5 (2024)',
  0x1652: 'Apex 7 (2024)',
  0x2200: 'Arctis Nova 7 (RX)',
  0x2202: 'Arctis Nova 7 (TX)',
  0x2204: 'Arctis Nova 7X (RX)',
  0x2206: 'Arctis Nova 7X (TX)',
  0x2208: 'Arctis Nova 7P (RX)',
  0x220A: 'Arctis Nova 7P (TX)',
  0x2230: 'Arctis Nova 5 (RX)',
  0x2232: 'Arctis Nova 5 (TX)',
  0x2267: 'Arctis Nova 3 Wireless (RX)',
  0x2269: 'Arctis Nova 3 Wireless (TX)',
  0x2290: 'Arctis Nova Pro Omni',
  0x22A1: 'Arctis Nova 7',
};

// Product IDs the Apex on-board protocol was reverse-engineered on, mirroring the class the discovery
// gate builds for them: the `KeyboardClass::Apex7` arm of `keyboard_class` in
// `src/devices/discovery.rs`. `Apex7` is the on-board class - it answers the protocol through the
// codec the class hands to the CLI (`Keyboard::onboard_codec`), which is what lets `ssgg onboard
// read` reach a board - and it is built for the board the frames are known from and for nothing
// else: the `0x1614` / `0x1616` / `0x1618` platform siblings build as `Generic`, answer no codec, and
// a read run on them is refused as DeviceNotFound (checked by `selftest.mjs`). A profile JSON naming
// a board outside this list was written by a family this page does not implement: its payload layout,
// its encodable fields and its feature set are its own.
const LEGACY_ONBOARD_PIDS = [0x1612];

// Model names and the per-model feature set, imported from Rust: `tools/profile-gui/sync-products.mjs`
// parses the product registry and the on-board family function and writes `products.generated.js`, which
// `index.html` loads before this file. `PRODUCT_NAMES` and `LEGACY_ONBOARD_PIDS` above are the
// hand-mirrored fallback for a page opened where the generator was never run; `selftest.mjs` fails when
// the imported tables and the Rust sources disagree, and when the fallback and the import disagree.
const PRODUCTS = globalThis.SsggProducts ?? null;

/// Where the model names on screen came from, shown next to them so a page using the fallback says so.
const IMPORT_SOURCE = PRODUCTS === null ? 'hand-mirrored table, run tools/profile-gui/sync-products.mjs' : 'imported from src/devices/mod.rs';

/// Model name a product ID has in Rust's registry (`device_name_from_product_id` in src/devices/mod.rs).
/// Rust answers a name for every ID it lists and a placeholder for the rest, so a placeholder reads as
/// "no name" and the caller shows the bare number.
function importedName(pid) {
  const hex = productHex(pid);
  if (hex === null || PRODUCTS === null) return null;
  const name = PRODUCTS.products[hex] && PRODUCTS.products[hex].name;
  return typeof name === 'string' && name !== 'Unknown SteelSeries Device' ? name : null;
}

/// Models whose on-board profile protocol Rust reverse-engineered (the `KeyboardClass::Apex7` arms of
/// `keyboard_class` in src/devices/discovery.rs). The page encodes one payload layout, so a model
/// outside this list needs its own family's layout, encoder and feature set.
function onboardModelPids() {
  return PRODUCTS === null ? LEGACY_ONBOARD_PIDS.slice() : PRODUCTS.onboardPids.map((hex) => Number.parseInt(hex, 16));
}

/// What kind of device a product ID is in Rust's registry (`device_type_from_product_id` in
/// src/devices/mod.rs): keyboard, mouse, headset, the registry's fallback, or null when the import is
/// absent.
function productKind(pid) {
  const hex = productHex(pid);
  if (hex === null || PRODUCTS === null) return null;
  const info = PRODUCTS.products[hex];
  return info && info.kind ? info.kind : PRODUCTS.kindFallback ?? null;
}

/// What this page can do for a model, built from the imported feature set: what kind of device Rust
/// says it is, the payload layout its bytes are read with, and the extra blocks only some boards
/// carry. A model the on-board protocol was not read back on has a payload layout this page does not
/// implement, so the page says that instead of encoding it with the legacy table.
function deviceFeatures(pid) {
  const hex = productHex(pid);
  if (hex === null) return ['not a product ID'];
  const info = PRODUCTS === null ? null : PRODUCTS.products[hex] ?? null;
  const onboard = info ? info.onboard === true : LEGACY_ONBOARD_PIDS.includes(pid);
  const kind = productKind(pid);
  const features = [
    // No import means no kind facts at all: say that rather than implying the registry is silent.
    PRODUCTS === null ? 'device registry: not imported (run tools/profile-gui/sync-products.mjs)'
      : info === null ? 'device registry: no entry in Rust'
      : `device registry: ${kind}`,
    onboard ? 'on-board profile protocol: read back on this family' : 'on-board profile protocol: not in Rust',
    onboard ? 'payload layout: legacy Apex 750-byte A/B' : 'payload layout: not the one this page encodes',
  ];
  if (kind === 'mouse') features.push('mouse layout: side/M/O buttons');
  if (kind === 'headset') features.push('no keyboard layout: a headset has no key mappings');
  if (onboard) features.push('extra blocks this page decodes: 128 mapping slots, scroll wheel, OLED, macros');
  return features;
}


const CONSUMER_TARGETS = [0xb5, 0xb6, 0xcd, 0xe2, 0xe9, 0xea];

// Consumer-page usages (USB HID Usage Tables, §10 Consumer page), the names GG's own media
// cluster uses. The same numbers are keyboard-page usages under other functions - 0xe2 is
// LAlt for a keyboard combo - so the name follows the mapping's function byte: only
// media_key (0x61) reads its codes out of this table.
const MEDIA_NAMES = {
  0xb5: 'Media Next',
  0xb6: 'Media Prev',
  0xcd: 'Media Play',
  0xe2: 'Media Mute',
  0xe9: 'Media Vol+',
  0xea: 'Media Vol-',
};

// ---------------------------------------------------------------------------
// Model: everything below works on the parsed JSON object and never touches
// the DOM, so `node selftest.mjs` exercises the same code the page runs.
// ---------------------------------------------------------------------------

const LAYER_BASE = 'base';
const LAYER_META = 'meta';

const EXPECTED_ARRAYS = {
  button_mappings: 128,
  wheel_mappings: 2,
  button_meta_mappings: 128,
  wheel_meta_mappings: 2,
};

function isValidProfileDoc(doc) {
  if (!doc || typeof doc !== 'object' || Array.isArray(doc)) return 'not an object';
  for (const [name, len] of Object.entries(EXPECTED_ARRAYS)) {
    const arr = doc[name];
    if (!Array.isArray(arr)) return `missing array ${name}`;
    if (arr.length !== len) return `${name} has ${arr.length} entries, expected ${len}`;
  }
  for (const [name, len] of Object.entries(EXPECTED_ARRAYS)) {
    const arr = doc[name];
    for (let i = 0; i < len; i += 1) {
      const codes = parseHex(arr[i] && arr[i].key_codes);
      if (codes === null) return `${name}[${i}].key_codes is not hex byte pairs`;
      if (codes.length > CODE_COUNT) {
        return `${name}[${i}].key_codes has ${codes.length} bytes, a mapping entry holds ${CODE_COUNT}`;
      }
    }
  }
  if (typeof doc.name !== 'string') return 'name is not a string';
  if (typeof doc.schema !== 'number') return 'schema is not a number';
  // The identity the read stamps (product_id, product, firmware) is absent for a file decode and
  // typed for a device read; anything else is not the report this page edits.
  if (doc.product_id !== undefined && doc.product_id !== null && !Number.isFinite(doc.product_id)) {
    return 'product_id is not a number';
  }
  if (doc.product !== undefined && doc.product !== null && typeof doc.product !== 'string') return 'product is not a string';
  if (doc.firmware !== undefined && doc.firmware !== null && typeof doc.firmware !== 'string') return 'firmware is not a string';
  return null;
}

// Bytes per mapping entry in a dump: 3 means the pre-fix decoder (the fourth code was cut off);
// the editor pads it to four, which is what GG writes there.
function codesPerEntry(doc) {
  let max = 0;
  for (const name of Object.keys(EXPECTED_ARRAYS)) {
    for (const entry of doc[name]) max = Math.max(max, (parseHex(entry.key_codes) ?? []).length);
  }
  return max;
}

// GG's factory defaults for a mapping entry, verbatim from a wired Apex 7 profile slot read
// (slot 0): function 0x51 with the slot's own key as the first code and the rest
// zero - a key bound to itself, not a remap - and the all-zero slot GG writes for keys the profile
// does not bind. Only anything else is a real binding.
function bindingOf(functionByte, codes, ownHid) {
  const zero = codes.every((code) => code === 0);
  if (zero && functionByte === 0) return 'none';
  if (
    functionByte === 0x51
    && ownHid !== null
    && ownHid !== undefined
    && codes[0] === ownHid
    && codes.slice(1).every((code) => code === 0)
  ) {
    return 'self';
  }
  return 'custom';
}

// Keep the decoder's derived fields honest after an edit: `offset`, `hid_code`, `function_name`
// and `binding` all follow from the raw function byte and key codes.
function normalizeEntry(entry, offset) {
  if (offset === null || offset === undefined) return entry;
  const codes = mappingCodes(entry).list;
  const own = OFF2HID[offset];
  entry.offset = offset;
  entry.hid_code = own === null || own === undefined ? null : own;
  entry.function_name = functionName(entry.function);
  entry.binding = bindingOf(entry.function, codes, own);
  // A 0x71 slot carries the macro record index at codes[2]; mirror of KeyMapping::macro_index.
  entry.macro_index = entry.function === 0x71 ? codes[2] : null;
  return entry;
}

function normalizeDoc(doc) {
  for (const name of ['button_mappings', 'button_meta_mappings']) {
    for (let offset = 0; offset < doc[name].length; offset += 1) normalizeEntry(doc[name][offset], offset);
  }
  return doc;
}

function layerArray(doc, layer) {
  return layer === LAYER_META ? doc.button_meta_mappings : doc.button_mappings;
}

// The mapping entry of one key on the current layer, resolved through the
// offset table. Returns null when the code has no offset in the table.
function keyEntry(doc, layer, hid) {
  const offset = hidSlot(hid);
  if (offset < 0) return null;
  const entry = layerArray(doc, layer)[offset];
  return entry ? { offset, entry } : null;
}

function mappingCodes(entry) {
  const codes = parseHex(entry.key_codes) ?? [];
  // Pad to the four codes of GG's `key_mapping` so the editor always edits four, and never
  // silently drop a fifth byte if a dump carries one.
  const out = codes.slice(0, CODE_COUNT);
  while (out.length < CODE_COUNT) out.push(0);
  return { list: out, extra: codes.slice(CODE_COUNT) };
}

function writeCodes(entry, codes, extra = []) {
  entry.key_codes = hexOf([...codes, ...extra]);
}

function setFunction(doc, layer, hid, fn) {
  const found = keyEntry(doc, layer, hid);
  if (!found) return false;
  found.entry.function = fn & 0xff;
  normalizeEntry(found.entry, found.offset);
  return true;
}

function setCode(doc, layer, hid, index, code) {
  const found = keyEntry(doc, layer, hid);
  if (!found) return false;
  const { list, extra } = mappingCodes(found.entry);
  list[index] = code & 0xff;
  writeCodes(found.entry, list, extra);
  normalizeEntry(found.entry, found.offset);
  return true;
}

function setAllCodes(doc, layer, hid, codes) {
  const found = keyEntry(doc, layer, hid);
  if (!found) return false;
  const { extra } = mappingCodes(found.entry);
  writeCodes(found.entry, codes, extra);
  normalizeEntry(found.entry, found.offset);
  return true;
}

// How a key should look on the board, from its mapping entry. `self` and `none` are the two
// "this key types normally" states; only `custom` is a remap.
function keyState(doc, layer, hid) {
  const offset = hidSlot(hid);
  if (offset === undefined || offset < 0) return { kind: 'no-offset', label: 'no offset', offset: null };
  const found = keyEntry(doc, layer, hid);
  if (!found) return { kind: 'no-offset', label: 'no offset', offset };
  const entry = found.entry;
  const codes = mappingCodes(entry).list;
  const own = OFF2HID[offset];
  const binding = entry.binding === 'self' || entry.binding === 'none' || entry.binding === 'custom'
    ? entry.binding
    : bindingOf(entry.function, codes, own);
  // The key the profile byte names is the key GG switches META mode with, and the byte is that
  // binding: GG shows this key as Meta - Meta Toggle. The page paints it as a meta key, prints META in
  // its cap, and the binding selects open on Meta / Meta Toggle for it. It binds like any other key.
  if (Number(doc.meta_toggle_hid) > 0 && boardHid(Number(doc.meta_toggle_hid)) === hid) {
    return { kind: 'meta', binding: 'custom', label: 'META', offset, entry, codes };
  }
  if (binding === 'self') return { kind: 'self', binding: 'self', label: 'self', offset, entry, codes };
  if (binding === 'none') return { kind: 'none', binding: 'none', label: 'not bound', offset, entry, codes };
  const kind = FUNCTION_NAMES[entry.function] ? FUNCTION_NAMES[entry.function] : 'function';
  // A 0x71 slot carries a binding byte in its third code byte: printing the codes as key names would
  // call a key that GG shows pressing 4 every 5000 ms "no code".
  if (entry.function === 0x71) {
    const byte = entry.macro_index ?? macroIndex(entry);
    const record = macroRecordFor(doc, byte);
    return {
      kind,
      binding,
      label: `binding byte ${byte} -> the macro ${macroOffsetForBindingByte(byte)} B into the block${record ? ` - ${macroKeystrokes(record)}` : ' (no macro at that byte)'}`,
      offset,
      entry,
      codes,
    };
  }
  if (entry.function === META_TOGGLE_FUNCTION) {
    // A meta key is named by its action (code 1): the four known ones print their word, the toggle
    // prints META, a code with no reading prints its own byte.
    const label = META_CAP_LABELS[codes[0]] ?? `meta 0x${hex2(codes[0])}`;
    return { kind: 'meta', binding, label, offset, entry, codes };
  }
  const target = codes.filter((c) => c !== 0).map((c) => keyName(c, entry.function)).join('+');
  return { kind, binding, label: target || 'no code', offset, entry, codes };
}

// Custom bindings of a layer, in slot order: the list a person wants to see first.
function customBindings(doc, layer) {
  return layerArray(doc, layer)
    .map((entry, offset) => ({ entry, offset, hid: OFF2HID[offset], board: boardHid(OFF2HID[offset]) }))
    .filter(({ entry, hid }) => bindingOf(entry.function, mappingCodes(entry).list, hid) === 'custom');
}

function bindingCounts(doc) {
  return [LAYER_BASE, LAYER_META].map((layer) => {
    const counts = { self: 0, none: 0, custom: 0 };
    layerArray(doc, layer).forEach((entry, offset) => {
      counts[bindingOf(entry.function, mappingCodes(entry).list, OFF2HID[offset])] += 1;
    });
    return { layer, counts };
  });
}

function rapidTap(doc) {
  return doc.rapid_tap ?? null;
}

function setRapidPair(doc, index, field, value) {
  const rt = rapidTap(doc);
  if (!rt || !rt.pairs || !rt.pairs[index]) return false;
  rt.pairs[index][field] = value;
  return true;
}

// Rapid Tap pairs reference keys by raw HID code, not by mapping offset.
function rapidTapKeysFor(doc, hid) {
  const rt = rapidTap(doc);
  if (!rt) return [];
  return rt.pairs
    .map((pair, index) => ({ index, pair }))
    .filter(({ pair }) => pair.hid_1 === hid || pair.hid_2 === hid);
}

function lighting(doc) {
  return doc.lighting ?? null;
}

function setLightingColor(doc, field, color) {
  const l = lighting(doc);
  if (!l || !l[field]) return false;
  l[field] = { r: color.r, g: color.g, b: color.b };
  return true;
}

// The name field is 24 payload bytes; the decoder strips the NUL padding and
// reports the rest. Writing a name regenerates name_hex the same way.
const NAME_FIELD_BYTES = 24;

function setProfileName(doc, name) {
  const clean = Array.from(name).filter((ch) => ch.codePointAt(0) >= 0x20 && ch.codePointAt(0) <= 0x7e);
  if (clean.length > NAME_FIELD_BYTES) return false;
  doc.name = clean.join('');
  doc.name_hex = hexOf(Array.from(doc.name).map((ch) => ch.codePointAt(0)));
  return true;
}

function serializeDoc(doc) {
  // The META mapping mask is derived from the META layer, so the JSON a write reads always carries a
  // mask that matches its bindings. The open document keeps the bytes the read carried, which is what
  // the Lighting panel compares against.
  const out = cloneDoc(doc);
  if (out.lighting) out.lighting.meta_mapped_elements = computeMetaMask(out);
  return `${JSON.stringify(out, null, 2)}
`;
}

function cloneDoc(doc) {
  return JSON.parse(JSON.stringify(doc));
}

// Flat change list of what differs from the file as loaded, for the panel and
// the "what will be saved" view.
function diffAgainst(original, current) {
  const changes = [];
  const walk = (a, b, path) => {
    if (a === b) return;
    if (Array.isArray(a) && Array.isArray(b)) {
      const len = Math.max(a.length, b.length);
      for (let i = 0; i < len; i += 1) walk(a[i], b[i], `${path}[${i}]`);
      return;
    }
    if (a && b && typeof a === 'object' && typeof b === 'object') {
      const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
      for (const key of keys) walk(a[key], b[key], path ? `${path}.${key}` : key);
      return;
    }
    changes.push({ path, from: a, to: b });
  };
  walk(original, current, '');
  return changes;
}

// Which key a change path belongs to, for the keyboard to show pending edits.
function changeKeyHids(changes) {
  const hids = new Map();
  for (const { path } of changes) {
    const m = /^(button_mappings|button_meta_mappings)\[(\d+)\]/.exec(path);
    if (!m) continue;
    const hid = boardHid(OFF2HID[Number(m[2])]);
    if (hid !== null && hid !== undefined) hids.set(hid, m[1]);
  }
  return hids;
}

// ---------------------------------------------------------------------------
// View: DOM wiring. Skipped when there is no document (node selftest).
// ---------------------------------------------------------------------------

const SVG_NS = ['http', '://www', '.w3', '.org/2000/svg'].join('');

function el(tag, attrs = {}, children = []) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (key === 'class') node.className = value;
    else if (key === 'text') node.textContent = value;
    else if (key.startsWith('on') && typeof value === 'function') node.addEventListener(key.slice(2).toLowerCase(), value);
    else if (value !== null && value !== undefined) node.setAttribute(key, String(value));
  }
  for (const child of [].concat(children)) if (child) node.appendChild(child);
  return node;
}

function svg(tag, attrs = {}, children = []) {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (key === 'text') node.textContent = value;
    else if (value !== null && value !== undefined) node.setAttribute(key, String(value));
  }
  for (const child of [].concat(children)) if (child) node.appendChild(child);
  return node;
}

const state = {
  files: [],
  activeId: null,
  layer: LAYER_BASE,
  selected: null,
  // The OLED pixel editor: which brush the next click uses, and the stroke in progress, so a drag
  // paints a connected line instead of the two ends of it.
  oledPaint: { mode: 'white', stroke: false, last: null },
  // Set when the Meta group is picked on the base layer: the binding select writes 0x62 to show the
  // action list, and picking Meta Toggle gives this entry its function byte back, because binding the
  // toggle is the profile byte alone.
};

function activeFile() {
  return state.files.find((file) => file.id === state.activeId) ?? null;
}

function activeDoc() {
  const file = activeFile();
  return file ? file.doc : null;
}

function nextFileId() {
  return (state.files.at(-1)?.id ?? 0) + 1;
}

function addFile(filename, text) {
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch (err) {
    return { ok: false, error: `${filename}: not valid JSON (${err.message})` };
  }
  const problem = isValidProfileDoc(parsed);
  if (problem) return { ok: false, error: `${filename}: ${problem}` };
  const doc = normalizeDoc(parsed);
  // One profile at a time: a new load replaces the board, so nothing edits a stale slot
  // by accident and the Save prompt always has one file to name.
  const file = { id: nextFileId(), filename, doc, original: cloneDoc(doc) };
  state.files = [file];
  state.activeId = file.id;
  state.selected = null;
  return { ok: true, file };
}

function downloadText(filename, text) {
  const blob = new Blob([text], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const link = el('a', { href: url, download: filename });
  document.body.appendChild(link);
  link.click();
  link.remove();
}

// The copy name is derived, not typed: a browser download already asks where to put the file, so
// a prompt in front of it is a second dialog for one decision. The loaded name gains a `-copy`
// marker, which is what keeps a save from ever looking like the file that was loaded.
function copyName(file) {
  let base = String(file?.filename ?? '').trim().replace(/\.json$/i, '');
  if (!base) {
    base = String(file?.doc?.name ?? '').trim().toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '');
  }
  base = base.replace(/-copy(?:-\d+)?$/i, '');
  return `${base || 'profile'}-copy.json`;
}

function saveFile(file, name = copyName(file)) {
  if (!name) return null;
  downloadText(name, serializeDoc(file.doc));
  return name;
}

function fileChanges(file) {
  return diffAgainst(file.original, file.doc);
}

function changedFiles() {
  return state.files.filter((file) => fileChanges(file).length > 0);
}

// Put one file back to the JSON it came in with. Every panel edit is a page-local edit of `doc`, and
// `original` is the parsed file untouched, so the undo for a whole profile is one clone of it.
function revertFile(file) {
  const index = state.files.findIndex((item) => item.id === file.id);
  if (index < 0) return null;
  state.files[index] = { ...file, doc: cloneDoc(file.original) };
  return state.files[index];
}

// Codes offered in the pickers: every code the offset table maps, plus 0x00.
function codeOptions() {
  const codes = [0];
  for (let hid = 0; hid < HID2OFF.length; hid += 1) if (HID2OFF[hid] >= 0) codes.push(hid);
  // Consumer-page codes the factory bindings use: legal binding targets with no key at that usage.
  for (const code of CONSUMER_TARGETS) if (!codes.includes(code)) codes.push(code);
  codes.sort((a, b) => (keyName(a) > keyName(b) ? 1 : -1));
  return codes;
}

const CODE_OPTIONS = codeOptions();

function buildKeySelect(selected, { noneLabel = '0x00 - none', functionByte = undefined } = {}) {
  const select = el('select', { class: 'code-select' });
  for (const code of CODE_OPTIONS) {
    const label = code === 0 ? noneLabel : `0x${hex2(code)} ${keyName(code, functionByte)}`;
    select.appendChild(el('option', { value: String(code), text: label }));
  }
  if (!CODE_OPTIONS.includes(selected)) {
    select.appendChild(el('option', { value: String(selected), text: `0x${hex2(selected)} ${keyName(selected)}` }));
  }
  select.value = String(selected);
  return select;
}

function buildFunctionSelect(selected) {
  const select = el('select', { class: 'fn-select' });
  const values = Object.keys(FUNCTION_NAMES).map(Number).sort((a, b) => a - b);
  for (const value of values) {
    select.appendChild(el('option', { value: String(value), text: `0x${hex2(value)} ${FUNCTION_NAMES[value]}` }));
  }
  if (!values.includes(selected)) {
    select.appendChild(el('option', { value: String(selected), text: `0x${hex2(selected)} ${functionName(selected)}` }));
  }
  select.value = String(selected);
  return select;
}

const UNIT = 48;
const GAP = 6;

// The Apex 7 wheel keys sit above the numpad's `-` column, as two half-height caps.
const WHEEL_UP = 0x100;
const WHEEL_DOWN = 0x101;
const WHEEL_KEYS = [
  { hid: WHEEL_UP, w: 1, h: 0.5, x: 23, y: 0 },
  { hid: WHEEL_DOWN, w: 1, h: 0.5, x: 23, y: 0.5 },
];

// The on-board OLED, over the numpad's first three columns with its top on the function-row
// line. Drawn 16:5 - the same 128:40 the bitmap is - with the height derived from the width
// the board draws (w*UNIT-GAP), so the ratio holds at any UNIT.
const OLED_RECT = { x: 20, y: 0, w: 3 };
OLED_RECT.h = ((OLED_RECT.w * UNIT - GAP) * 5 / 16 + GAP) / UNIT;

// The oled block is a 128 x 40 monochrome bitmap: 8 pixels per byte, 16 bytes per row,
// 40 rows = 640 bytes = OLED_LEN in the decoder. Bit 7 of a byte is its leftmost pixel,
// confirmed on an Apex 7 (0x1612): the image GG paints on its panel decodes to these bytes and
// writing them back paints that picture. There is no bit-order switch.
const OLED_W = 128;
const OLED_H = 40;
const OLED_BYTES = OLED_W * OLED_H / 8;
// Board selection id for the OLED rect: no HID code is negative, so it cannot clash with a key.
const SELECT_OLED = -1;

// The macro block's bytes as a report carries them: `null` when the report has none (a read written
// before the codec shipped the block), and only a full 9,600-byte block counts.
function macroBlockBytes(doc) {
  const hex = doc && doc.macro_data && doc.macro_data.bytes_hex;
  if (typeof hex !== 'string') return null;
  const bytes = parseHex(hex);
  return bytes && bytes.length === MACRO_DATA_LEN ? bytes : null;
}

function oledBytes(doc) {
  const hex = doc && doc.oled && doc.oled.bytes_hex;
  if (typeof hex !== 'string') return null;
  const bytes = parseHex(hex);
  return bytes && bytes.length === OLED_W * OLED_H / 8 ? bytes : null;
}

// Rasterise to RGBA: a set bit is a lit pixel, bit 7 of each byte is the leftmost pixel.
function oledPixels(bytes) {
  const px = new Uint8ClampedArray(OLED_W * OLED_H * 4);
  const stride = OLED_W / 8;
  for (let row = 0; row < OLED_H; row += 1) {
    for (let col = 0; col < OLED_W; col += 1) {
      const byte = bytes[row * stride + (col >> 3)];
      const value = ((byte >> (7 - (col & 7))) & 1) === 1 ? 255 : 0;
      const at = (row * OLED_W + col) * 4;
      px[at] = value;
      px[at + 1] = value;
      px[at + 2] = value;
      px[at + 3] = 255;
    }
  }
  return px;
}

// PNG data URL through a canvas. null when the page has no 2D canvas, and the board then
// keeps its text label instead of the bitmap.
function oledDataUrl(bytes) {
  const canvas = document.createElement('canvas');
  canvas.width = OLED_W;
  canvas.height = OLED_H;
  const ctx = canvas.getContext && canvas.getContext('2d');
  if (!ctx || !ctx.createImageData || !canvas.toDataURL) return null;
  const image = ctx.createImageData(OLED_W, OLED_H);
  image.data.set(oledPixels(bytes));
  ctx.putImageData(image, 0, 0);
  return canvas.toDataURL('image/png');
}

// A picture upload has to become the block geometry above: 128x40, 8 px per byte, bit 7 left.
// The Rust codec carries the block byte-for-byte, so this page is the only place a file turns
// into those 640 bytes. Sampling is nearest-neighbour over the whole picture - the rule for a
// picture stretched to fill a fixed slot, which is how GG fills this panel from any image - and a
// pixel lights at OLED_LIT_THRESHOLD/255 or above.
const OLED_LIT_THRESHOLD = 128;
const BMP_DIB_SIZES = [12, 40, 64, 108, 124];
const BMP_ACCEPTED_BITS = [1, 4, 8, 24, 32];
// BI_RGB, BI_RLE8, BI_RLE4, BI_BITFIELDS, BI_JPEG, BI_PNG, BI_ALPHABITFIELDS, then the CMYK set.
const BMP_COMPRESSION_NAMES = {
  0: 'BI_RGB', 1: 'BI_RLE8', 2: 'BI_RLE4', 3: 'BI_BITFIELDS', 4: 'BI_JPEG', 5: 'BI_PNG',
  6: 'BI_ALPHABITFIELDS', 11: 'BI_CMYSTRGB', 12: 'BI_CMYSTRLE8', 13: 'BI_CMYSTRLE4',
  14: 'BI_CMYSTBitFields', 15: 'BI_CMYSTALPHABITFIELDS',
};
// The masks a 32 bpp BI_BITFIELDS file carries, in the order the file stores them (little-endian
// u32 each): blue, green, red. Anything else is a layout this page does not guess.
const BMP_PLAIN_MASKS = [0x00ff0000, 0x0000ff00, 0x000000ff];

const bmpU16 = (b, at) => b[at] | (b[at + 1] << 8);
const bmpU32 = (b, at) => (b[at] | (b[at + 1] << 8) | (b[at + 2] << 16) | (b[at + 3] << 24)) >>> 0;
const bmpI32 = (b, at) => b[at] | (b[at + 1] << 8) | (b[at + 2] << 16) | (b[at + 3] << 24);

// How many palette entries the file actually carries: biClrCount says it, and 0 means the file
// ships none (the spec's default of 2^bits entries is a synthetic palette, not bytes in the file).
const declaredPaletteEntries = (bits, colorCount) => (colorCount > 0 ? Math.min(colorCount, 1 << bits) : 0);
const defaultPalette = (bits) => (bits === 1
  ? [[0, 0, 0], [255, 255, 255]]
  : Array.from({ length: 1 << bits }, (_, i) => Array(3).fill(Math.round((i * 255) / ((1 << bits) - 1)))));

// Reads the header only: width, height, bit depth, compression, orientation, row stride, pixel
// offset and the palette. Rows are padded to 4 bytes and a positive height means bottom-up, so
// both are recorded here and applied by bmpColor. Every rejection names what it found.
function decodeBmp(input) {
  const bytes = input instanceof Uint8Array ? input : Uint8Array.from(input ?? []);
  const fail = (message) => { throw new Error(`not a BMP this page can read: ${message}`); };
  if (bytes.length < 54) fail(`the file is ${bytes.length} bytes, a BMP needs a 14-byte file header and at least a 40-byte DIB header`);
  if (bytes[0] !== 0x42 || bytes[1] !== 0x4d) fail(`the magic is 0x${hex2(bytes[0])} 0x${hex2(bytes[1])}, not 0x42 0x4d ("BM")`);
  const dibSize = bmpU32(bytes, 14);
  if (!BMP_DIB_SIZES.includes(dibSize)) fail(`the DIB header is ${dibSize} bytes, expected one of ${BMP_DIB_SIZES.join(', ')}`);
  const core = dibSize === 12;
  const width = core ? bmpU16(bytes, 18) : bmpI32(bytes, 18);
  const rawHeight = core ? bmpU16(bytes, 20) : bmpI32(bytes, 22);
  const bits = core ? bmpU16(bytes, 24) : bmpU16(bytes, 28);
  const compression = core ? 0 : bmpU32(bytes, 30);
  const compressionName = BMP_COMPRESSION_NAMES[compression] ?? `compression #${compression}`;
  if (width <= 0) fail(`the width is ${width}, a BMP width must be positive`);
  if (rawHeight === 0) fail('the height is 0, an empty picture');
  const height = Math.abs(rawHeight);
  const topDown = rawHeight < 0;
  if (compression === 4 || compression === 5) fail(`${compressionName} embeds a ${compression === 4 ? 'JPEG' : 'PNG'} in the file, and this page decodes BMP pixels only`);
  if (compression === 1 || compression === 2) fail(`${compressionName} is run-length encoded, this page reads uncompressed rows only`);
  if (compression !== 0 && compression !== 3) fail(`${compressionName} is not a layout this page reads`);
  if (!BMP_ACCEPTED_BITS.includes(bits)) {
    fail(`${bits} bpp is not one of ${BMP_ACCEPTED_BITS.join(', ')} bpp${bits === 16 ? ' (5-5-5 and 5-6-5 are not decoded)' : ''}`);
  }
  const masksAt = 14 + dibSize;
  let paletteAt = masksAt;
  if (compression === 3) {
    const masks = [bmpU32(bytes, masksAt), bmpU32(bytes, masksAt + 4), bmpU32(bytes, masksAt + 8)];
    if (bits !== 32 || masks.some((mask, i) => mask !== BMP_PLAIN_MASKS[i])) {
      fail(`BI_BITFIELDS with masks 0x${masks.map((mask) => hexOf([mask & 0xff, (mask >> 8) & 0xff, (mask >> 16) & 0xff, (mask >> 24) & 0xff].reverse())).join(' ')} at ${bits} bpp, this page takes the plain blue/green/red masks at 32 bpp`);
    }
    paletteAt = masksAt + 12;
  }
  const colorCount = core ? 1 << bits : bmpU32(bytes, 46);
  const stride = (((bits * width + 31) >> 5) << 2);
  const needed = stride * height;
  let dataOffset = bmpU32(bytes, 10);
  if (dataOffset < 14 + dibSize) dataOffset = paletteAt + (bits <= 8 ? declaredPaletteEntries(bits, colorCount) : 0);
  if (bytes.length < dataOffset + needed) {
    fail(`the file is ${bytes.length} bytes, ${dataOffset} + ${needed} are needed for ${width}x${height} at ${bits} bpp with ${stride}-byte rows`);
  }
  let palette = null;
  if (bits <= 8) {
    // A 1 bpp file usually ships no palette: index 0 is black, index 1 white. A 4 bpp file
    // without one gets the grey ramp its 16 indices stand for, an 8 bpp file its own grey ramp.
    // Entries the file does carry win over the default.
    const declared = declaredPaletteEntries(bits, colorCount);
    if (bytes.length < paletteAt + declared * 4) fail(`the file has ${bytes.length} bytes, the ${bits} bpp palette needs ${declared} entries at offset ${paletteAt}`);
    palette = defaultPalette(bits);
    for (let i = 0; i < declared; i += 1) {
      // A BMP palette entry is blue, green, red, reserved.
      palette[i] = [bytes[paletteAt + i * 4 + 2], bytes[paletteAt + i * 4 + 1], bytes[paletteAt + i * 4]];
    }
  }
  return { bytes, width, height, bits, compression, compressionName, topDown, stride, dataOffset, palette };
}

// The colour of one source pixel: palette formats read the palette entry, 24 and 32 bpp read the
// triple. 1 bpp puts bit 7 of a byte on its left pixel and 4 bpp its high nibble, matching the
// row-major left-to-right order of the block itself.
function bmpColor(bmp, x, y) {
  const row = bmp.topDown ? y : bmp.height - 1 - y;
  const at = bmp.dataOffset + row * bmp.stride;
  if (bmp.bits === 24) return [bmp.bytes[at + x * 3 + 2], bmp.bytes[at + x * 3 + 1], bmp.bytes[at + x * 3]];
  if (bmp.bits === 32) return [bmp.bytes[at + x * 4 + 2], bmp.bytes[at + x * 4 + 1], bmp.bytes[at + x * 4]];
  let index;
  if (bmp.bits === 1) index = (bmp.bytes[at + (x >> 3)] >> (7 - (x & 7))) & 1;
  else if (bmp.bits === 4) index = (x & 1) ? bmp.bytes[at + (x >> 1)] & 0x0f : (bmp.bytes[at + (x >> 1)] >> 4) & 0x0f;
  else index = bmp.bytes[at + x];
  return bmp.palette[Math.min(index, bmp.palette.length - 1)];
}

// Nearest-neighbour sampling of a decoded BMP into the block, and the lit-pixel count it produced.
function bmpToOledBytes(bmp, { width = OLED_W, height = OLED_H, threshold = OLED_LIT_THRESHOLD } = {}) {
  const out = new Array((width * height) / 8).fill(0);
  let lit = 0;
  for (let y = 0; y < height; y += 1) {
    const sy = Math.min(bmp.height - 1, Math.floor((y * bmp.height) / height));
    for (let x = 0; x < width; x += 1) {
      const sx = Math.min(bmp.width - 1, Math.floor((x * bmp.width) / width));
      const [r, g, b] = bmpColor(bmp, sx, sy);
      if (Math.round((r + g + b) / 3) >= threshold) {
        out[y * (width / 8) + (x >> 3)] |= 1 << (7 - (x & 7));
        lit += 1;
      }
    }
  }
  return { bytes: out, lit, width, height };
}

// Decode, sample and write the block in one call; the summary fields come from setOledBitmap.
function applyBmpBytes(doc, input, options = {}) {
  const bmp = decodeBmp(input);
  const { bytes, lit, width, height } = bmpToOledBytes(bmp, options);
  setOledBitmap(doc, bytes);
  return {
    source_width: bmp.width, source_height: bmp.height, bits: bmp.bits,
    compression: bmp.compressionName, top_down: bmp.topDown, lit,
    pixels: width * height, threshold: options.threshold ?? OLED_LIT_THRESHOLD,
  };
}

// A File from the picker, read as bytes. The page has FileReader; the node-side suite calls
// applyBmpBytes with the bytes directly.
function applyBmpFile(doc, file) {
  return new Promise((resolve, reject) => {
    if (typeof FileReader !== 'function') { reject(new Error('this page has no FileReader')); return; }
    const reader = new FileReader();
    reader.onload = () => {
      try { resolve(applyBmpBytes(doc, new Uint8Array(reader.result))); } catch (error) { reject(error); }
    };
    reader.onerror = () => reject(new Error(`${file?.name ?? 'the file'} could not be read`));
    reader.readAsArrayBuffer(file);
  });
}

// Per-pixel painting. A pixel is one bit of the block: pixel index y * 128 + x lives in byte
// index >> 3 at bit 7 - (index & 7), the same order the raster and the BMP sampler use, so a
// painted pixel is the pixel the preview shows and the one the board lights.
const OLED_PAINT_MODES = ['white', 'black'];
const OLED_PAINT_SCALE = 2;

const oledPixelIndex = (x, y) => (x >= 0 && x < OLED_W && y >= 0 && y < OLED_H ? y * OLED_W + x : null);
const oledPixelXY = (index) => ({ x: index % OLED_W, y: Math.floor(index / OLED_W) });
const oledLitPixels = (bytes) => bytes.reduce((total, byte) => {
  let bits = byte & 0xff;
  let count = 0;
  while (bits) { bits &= bits - 1; count += 1; }
  return total + count;
}, 0);

// Bresenham between two pixels: a fast drag reports a few points along it and every one of them
// has to land, or the stroke comes out as dots.
function oledLine(from, to) {
  const a = oledPixelXY(from);
  const b = oledPixelXY(to);
  const dx = Math.abs(b.x - a.x);
  const dy = Math.abs(b.y - a.y);
  const stepX = a.x < b.x ? 1 : -1;
  const stepY = a.y < b.y ? 1 : -1;
  const out = [];
  let x = a.x;
  let y = a.y;
  let err = dx - dy;
  for (let guard = 0; guard < OLED_W * OLED_H; guard += 1) {
    out.push(y * OLED_W + x);
    if (x === b.x && y === b.y) break;
    const twice = 2 * err;
    if (twice > -dy) { err -= dy; x += stepX; }
    if (twice < dx) { err += dx; y += stepY; }
  }
  return out;
}

// Paint a run of pixels with one brush into a copy of the block. `changed` counts the bits that
// actually flipped, so painting white over white reports nothing written. A block with no
// bytes_hex starts from every pixel off, like the Invert control treats it.
function applyOledPixels(bytes, indices, mode) {
  const out = Array.from(bytes && bytes.length === OLED_BYTES ? bytes : new Array(OLED_BYTES).fill(0));
  const litBefore = oledLitPixels(out);
  let changed = 0;
  for (const index of indices) {
    if (!Number.isInteger(index) || index < 0 || index >= OLED_W * OLED_H) continue;
    const at = index >> 3;
    const bit = 7 - (index & 7);
    const want = mode === 'black' ? 0 : 1;
    if (((out[at] >> bit) & 1) !== want) {
      out[at] ^= 1 << bit;
      changed += 1;
    }
  }
  return { bytes: out, changed, lit: oledLitPixels(out), litBefore, pixels: OLED_W * OLED_H };
}

// Paint on the doc through setOledBitmap, so len, nonzero, fnv1a_64 and head stay true to the bytes.
function applyOledPaint(doc, indices, mode) {
  const info = applyOledPixels(oledBytes(doc), indices, mode);
  setOledBitmap(doc, info.bytes);
  return info;
}

// offsetX and offsetY are relative to the target, and the preview is drawn at OLED_PAINT_SCALE
// device pixels per bitmap pixel, so the click's own coordinates name the pixel under it.
function oledPixelAtEvent(event, scale = OLED_PAINT_SCALE) {
  return oledPixelIndex(Math.floor((event.offsetX ?? 0) / scale), Math.floor((event.offsetY ?? 0) / scale));
}

function boardKeyList() {
  return [
    ...LAYOUT.map((k) => ({ ...k, extra: false })),
    ...WHEEL_KEYS.map((key) => ({ ...key, extra: true })),
    ...EXTRA_HIDS.map((hid, i) => ({ hid, w: 1, h: 1, x: i * 1.12, y: 7.5, extra: true })),
  ];
}

function buildBoard(onSelect) {
  const keys = boardKeyList();
  const width = (Math.max(...LAYOUT.map((k) => k.x + k.w)) + 1) * UNIT;
  const height = 8.8 * UNIT;
  const root = svg('svg', { viewBox: `0 0 ${width} ${height}`, class: 'board', width: '100%' });
  for (const key of keys) {
    const x = key.x * UNIT + GAP / 2;
    const y = key.y * UNIT + GAP / 2;
    const w = key.w * UNIT - GAP;
    const h = key.h * UNIT - GAP;
    const group = svg('g', { class: 'key', 'data-hid': String(key.hid), tabindex: '0', role: 'button' });
    group.appendChild(svg('rect', { x, y, width: w, height: h, rx: 6, class: 'cap' }));
    group.appendChild(svg('rect', { x, y, width: w, height: h, rx: 6, class: 'fill' }));
    // The cap box is the control's bounds: everything the cap draws is clipped to it, so a long sub
    // line (a macro binding reads "macro 7 (record 1) - LCtrl > 20 ms") never paints out of the
    // button, never adds a click target outside it, and never widens what the browser rings around a
    // focused control - the clipPath is the cap rect, and every text carries clip-path.
    const clipId = `cap-clip-${key.hid}`;
    const clip = svg('clipPath', { id: clipId });
    clip.appendChild(svg('rect', { x, y, width: w, height: h, rx: 6 }));
    group.appendChild(clip);
    const insideCap = { 'clip-path': `url(#${clipId})` };
    const isWheel = key.hid === WHEEL_UP || key.hid === WHEEL_DOWN;
    const glyph = ARROW_GLYPHS[key.hid];
    if (isWheel) {
      // Half-height cap (42 x 18): a direction glyph over the mapping text, both inside.
      group.appendChild(svg('text', {
        x: x + w / 2,
        y: y + 11,
        class: 'wheel-glyph',
        text: key.hid === WHEEL_UP ? '↑' : '↓',
        ...insideCap,
      }));
      group.appendChild(svg('text', { x: x + w / 2, y: y + h - 1, class: 'sub wheel-sub', text: '', ...insideCap }));
    } else {
      if (glyph) {
        group.appendChild(svg('text', { x: x + w / 2, y: y + h / 2 + 8, class: 'arrow', text: glyph, ...insideCap }));
      } else {
        const label = boardLabel(key.hid);
        group.appendChild(svg('text', {
          x: x + w / 2,
          y: y + 20,
          class: `label ${labelClass(label)}`.trim(),
          text: label,
          ...insideCap,
        }));
      }
      group.appendChild(svg('text', { x: x + w / 2, y: y + h - 7, class: 'sub', text: '', ...insideCap }));
    }
    group.appendChild(svg('title', { text: '' }));
    group.addEventListener('click', () => onSelect(key.hid));
    // The cap is the control (role button, tabindex): Enter and Space select it too, so the target
    // is the cap for a keyboard user as well, not the label glyph.
    group.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        onSelect(key.hid);
      }
    });
    root.appendChild(group);
  }
  const oled = svg('g', { class: 'oled', 'data-oled': '1', role: 'button', tabindex: '0' });
  const ox = OLED_RECT.x * UNIT + GAP / 2;
  const oy = OLED_RECT.y * UNIT + GAP / 2;
  oled.appendChild(svg('rect', {
    x: ox,
    y: oy,
    width: OLED_RECT.w * UNIT - GAP,
    height: OLED_RECT.h * UNIT - GAP,
    rx: 4,
    class: 'oled-screen',
  }));
  const oledW = OLED_RECT.w * UNIT - GAP;
  const oledH = OLED_RECT.h * UNIT - GAP;
  oled.appendChild(svg('image', {
    x: ox,
    y: oy,
    width: oledW,
    height: oledH,
    href: '',
    'xlink:href': '',
    preserveAspectRatio: 'none',
    class: 'oled-img',
  }));
  oled.appendChild(svg('text', {
    x: ox + oledW / 2,
    y: oy + oledH / 2 + 4,
    class: 'oled-label',
    text: 'OLED',
  }));
  oled.appendChild(svg('title', { text: 'on-board OLED: click to edit the oled block (128x40 monochrome bitmap)' }));
  oled.addEventListener('click', () => onSelect(SELECT_OLED));
  oled.addEventListener('keydown', (event) => {
    if (event.key === 'EnterQ' || event.key === ' ') {
      event.preventDefault();
      onSelect(SELECT_OLED);
    }
  });
  root.appendChild(oled);
  root.appendChild(svg('text', {
    x: 4,
    y: 7.3 * UNIT,
    class: 'strip-label',
    text: 'codes the offset table maps that the ANSI shape above has no place for:',
  }));
  return root;
}

// State colours, picked apart in hue and lightness: the neutral ones sit clearly above the
// cap (a factory board must read as keys, not as background), macro (green) and media
// (magenta) are on opposite sides of the wheel, and 'no-offset' is deliberately the one that
// recedes - it is the state with nothing bound in the legacy table.
// Mirrored by .sw-* in style.css; the selftest compares the two lists.
const FILL_COLORS = {
  self: '#46537c',
  none: '#2d3449',
  deactivated: '#892424',
  combo_key: '#217283',
  meta: '#6d2a84',
  macro: '#2a793e',
  media_key: '#862757',
  function: '#8c6921',
  'no-offset': '#0c0e12',
};

// The wheel caps have no offset in the 128-entry table: their state is the wheel_mappings
// pair of the current layer, so the board can paint them like any other key.
function wheelEntry(doc, layer, index) {
  const entry = (layer === LAYER_META ? doc.wheel_meta_mappings : doc.wheel_mappings)[index];
  if (!entry) return null;
  const codes = mappingCodes(entry).list;
  const binding = bindingOf(entry.function, codes, null);
  const kind = FUNCTION_NAMES[entry.function] ?? 'function';
  if (entry.function === 0x71) {
    const byte = entry.macro_index ?? macroIndex(entry);
    const record = macroRecordFor(doc, byte);
    return {
      index, entry, codes, binding, kind,
      label: `binding byte ${byte} -> the macro ${macroOffsetForBindingByte(byte)} B into the block${record ? ` - ${macroKeystrokes(record)}` : ' (no macro at that byte)'}`,
    };
  }
  // A meta binding that carries the Lock Win action: the cap prints its action, and the META cap
  // word comes from meta_toggle_hid alone (keyState), never from this byte pair.
  if (entry.function === META_TOGGLE_FUNCTION && codes[0] === META_LOCK_WIN_ACTION) {
    return { index, entry, codes, binding, kind: 'meta', label: 'Lock Win' };
  }
  const target = codes.filter((code) => code !== 0).map((code) => keyName(code, entry.function)).join('+');
  return { index, entry, codes, binding, kind, label: binding === 'none' ? 'not bound' : target || 'no code' };
}

function paintBoard(board, doc, layer, selected, changedHids) {
  for (const group of board.querySelectorAll('g.key')) {
    const hid = Number(group.getAttribute('data-hid'));
    const isWheel = hid === WHEEL_UP || hid === WHEEL_DOWN;
    const wheel = isWheel ? wheelEntry(doc, layer, hid === WHEEL_UP ? 0 : 1) : null;
    const key = isWheel ? wheel : keyState(doc, layer, hid);
    if (!key) continue;
    const entry = key.entry;
    const kind = key.kind;
    const cls = [`k-${String(kind).replace(/[^a-z]/gi, '')}`];
    const offset = isWheel ? null : hidSlot(hid);
    if (offset === null || offset === undefined || offset < 0) cls.push('unmapped');
    if (selected === hid) cls.push('selected');
    if (changedHids.has(hid)) cls.push('changed');
    group.setAttribute('class', ['key', ...cls].join(' '));
    group.querySelector('.fill').setAttribute('fill', FILL_COLORS[kind] ?? '#3c3320');
    const pairs = isWheel ? [] : rapidTapKeysFor(doc, hid);
    const mark = pairs.length ? ` [RT ${pairs.map(({ index }) => index + 1).join(',')}]` : '';
    // Only a remap prints a sub line: 'self' on every factory key is noise, and the binding
    // is in the tooltip and the key panel. The wheel caps always show their mapping.
    const sub = group.querySelector('.sub');
    // The designated META key prints its mark even when the slot itself is empty (in the user profile
    // the byte names a key whose META entry is deactivated).
    const text = isWheel ? key.label : (key.kind === 'meta' ? key.label : (key.binding === 'custom' ? key.label : ''));
    const full = `${text}${mark}`;
    const subCls = subClass(full);
    // the cap box width, so a wide cap gets its longer line and a 1-unit cap truncates: the binding
    // text is cut, the Rapid Tap marker never is
    sub.textContent = fitSubText(text, mark, subCls, Number(group.querySelector('.cap').getAttribute('width')));
    sub.setAttribute('class', `${isWheel ? 'sub wheel-sub' : 'sub'} ${subCls}`.trim());
    group.querySelector('title').textContent = isWheel
      ? `${keyName(hid)} - wheel mapping ${wheel.index + 1} (${layer}) ${wheel.binding} `
        + `function 0x${hex2(entry.function)} ${functionName(entry.function)} codes ${hexOf(key.codes)}`
      : offset < 0
        ? `${keyName(hid)} (0x${hex2(hid)}) - no mapping offset in the legacy HID table`
        : `${keyName(hid)} (0x${hex2(hid)}) offset ${offset} ${layer} ${key.binding ?? key.kind} `
          + `function 0x${hex2(entry.function)} ${functionName(entry.function)} codes ${hexOf(key.codes)}`;
  }
  paintOled(board, doc);
}

// Mirrors fnv1a_64 in src/devices/keyboards/apex_7_profile.rs: 16 uppercase hex digits,
// block identity between reads. Not a protocol checksum.
function fnv1a64(bytes) {
  let hash = 0xcbf29ce484222325n;
  // The engine's **short prime** (43981), not the published FNV-1a prime 0x100000001b3. The two agree
  // in the low 32 bits and differ above them, so the difference hides in the tail - and the decoder
  // hashes the raw block bytes with this one (fnv1a_64 in apex_7_profile.rs).
  const prime = 0x1000000001b3n;
  const mask = (1n << 64n) - 1n;
  for (const byte of bytes) hash = ((hash ^ BigInt(byte)) * prime) & mask;
  return hash.toString(16).toUpperCase().padStart(16, '0');
}

// Write a new 128x40 bitmap into the oled block, keeping every summary field the decoder
// writes consistent with the bytes the block now holds.
function setOledBitmap(doc, bytes) {
  const oled = doc.oled;
  const clean = Array.from(bytes);
  oled.len = clean.length;
  oled.nonzero = clean.filter((byte) => byte !== 0).length;
  oled.fnv1a_64 = fnv1a64(clean);
  oled.head = hexOf(clean.slice(0, 16));
  oled.bytes_hex = hexOf(clean);
  return oled;
}

// The block as the file was loaded with it. Every OLED control on the page is a page-local edit, so
// this is the undo for all of them: the block object the file came in with goes back over the
// current one, summary fields included, because those describe the bytes being restored.
function revertOledBlock(file) {
  const original = file?.original?.oled;
  if (!original) return null;
  file.doc.oled = JSON.parse(JSON.stringify(original));
  const bytes = oledBytes(file.doc);
  return {
    bytes,
    lit: oledLitPixels(bytes ?? []),
    len: file.doc.oled.len,
    nonzero: file.doc.oled.nonzero,
    fnv1a_64: file.doc.oled.fnv1a_64,
  };
}

// Whether the block differs from the file as loaded, which is what the revert button is disabled on.
const oledBlockDiffers = (file) => JSON.stringify(file?.original?.oled ?? null) !== JSON.stringify(file?.doc?.oled ?? null);

// ---------------------------------------------------------------------------
// Panels
// ---------------------------------------------------------------------------

let ui = null;

// Fill the board's OLED rect from the block bytes: a PNG data URL when the page can raster,
// the text label when it cannot (or when the dump has no bytes, i.e. a read that predates the block dump).
function paintOled(board, doc) {
  const group = board.querySelector('g.oled');
  if (!group || !doc.oled) return;
  const bytes = oledBytes(doc);
  const url = bytes ? oledDataUrl(bytes) : null;
  group.setAttribute('class', `oled${state.selected === SELECT_OLED ? ' selected' : ''}`);
  const image = group.querySelector('image');
  const label = group.querySelector('.oled-label');
  if (image) {
    if (url) {
      image.setAttribute('href', url);
      image.setAttribute('xlink:href', url);
      image.removeAttribute('visibility');
    } else {
      image.removeAttribute('href');
      image.removeAttribute('xlink:href');
      image.setAttribute('visibility', 'hidden');
    }
  }
  if (label) label.setAttribute('visibility', url ? 'hidden' : 'visible');
  group.querySelector('title').textContent = `on-board OLED: ${OLED_W}x${OLED_H} monochrome bitmap, bit 7 of each byte is the `
    + `left pixel, payload offset ${doc.oled.offset}, ${doc.oled.nonzero}/${doc.oled.len} bytes non-zero, fnv1a ${doc.oled.fnv1a_64}. `
    + (url ? 'Click the rect to edit the block. ' : 'Not drawn: ')
    + (bytes ? (url ? '' : 'the page has no 2D canvas.')
      : 'this dump has no oled.bytes_hex, re-decode the saved image with `ssgg onboard decode`.');
}

function panel(title, rows) {
  return el('section', { class: 'panel' }, [el('h2', { text: title }), ...rows]);
}

function line(label, control, note) {
  return el('div', { class: 'line' }, [
    el('span', { class: 'line-label', text: label }),
    el('span', { class: 'line-control' }, [control]),
    note ? el('span', { class: 'line-note', text: note }) : null,
  ]);
}

function numberInput(value, min, max, onChange) {
  return el('input', {
    class: 'num',
    type: 'number',
    value: String(value),
    min: String(min),
    max: String(max),
    oninput: (event) => onChange(Number(event.target.value)),
  });
}

function textInput(value, onChange, { maxlength } = {}) {
  return el('input', {
    class: 'text',
    type: 'text',
    value,
    maxlength,
    oninput: (event) => onChange(event.target.value),
  });
}

function button(label, onClick, cls) {
  return el('button', { class: cls ?? 'btn', onclick: onClick, text: label });
}

// The profile name editor, above the board. It writes the 24-byte name field (setProfileName keeps the
// bytes printable ASCII and in length, and rewrites name_hex with it). While you type it updates its own
// indicator and leaves the rest of the page alone - repainting on every keystroke would pull the caret out
// of the field - and the full repaint comes on the field's change event.
function renderNameBar(file) {
  const box = ui.nameBar;
  if (!box) return;
  const doc = file.doc;
  const original = file.original;
  const isChanged = () => !!original
    && (original.name !== doc.name || String(original.name_hex).toUpperCase() !== String(doc.name_hex).toUpperCase());
  const state = el('span', { class: 'name-state' });
  const revert = el('button', {
    class: 'btn name-revert',
    text: 'Revert the name',
    onclick: () => {
      if (!original) return;
      if (!setProfileName(doc, original.name)) {
        ui.status.textContent = 'the name the file was loaded with does not fit the 24-byte field';
        render();
        return;
      }
      ui.status.textContent = `reverted the profile name to "${original.name}", as the file was loaded (${original.name_hex})`;
      render();
    },
  });
  const paint = () => {
    const changed = isChanged();
    state.className = `name-state${changed ? ' changed' : ''}`;
    state.textContent = changed
      ? `changed - the file as loaded had "${original.name}" (${original.name_hex})`
      : `as the file - ${doc.name_hex}`;
    if (changed) revert.removeAttribute('disabled');
    else revert.setAttribute('disabled', 'true');
  };
  const input = textInput(doc.name, (value) => {
    if (!setProfileName(doc, value)) {
      ui.status.textContent = 'name must stay within 24 printable ASCII bytes (the field is 24 bytes)';
    }
    paint();
  }, { maxlength: '24' });
  input.setAttribute('name', 'profile-name');
  input.addEventListener('change', () => render());
  paint();
  box.replaceChildren(
    el('label', { class: 'name-label', text: 'Profile name' }),
    input,
    state,
    revert,
    el('span', {
      class: 'hint name-note',
      text: `the name field of the profile is ${NAME_FIELD_BYTES} bytes (name_hex: ${doc.name_hex}); the tab row marks the file as edited and its revert button takes the name back with the rest of the file, and Save writes it with the rest of the JSON`,
    }),
  );
}

function renderTabs() {
  const box = ui.tabs;
  box.replaceChildren();
  for (const file of state.files) {
    const changes = fileChanges(file).length;
    const tab = el('button', {
      class: `tab${file.id === state.activeId ? ' active' : ''}`,
      onclick: () => {
        state.activeId = file.id;
        state.selected = null;
        render();
      },
      text: `${file.filename}${changes ? ` *${changes}` : ''}`,
    });
    box.appendChild(tab);
  }
  if (!state.files.length) {
    box.appendChild(el('span', { class: 'hint', text: 'no file loaded' }));
    return;
  }
  // The undo for the whole profile sits at the end of the tab row, where the tab it names is.
  const file = activeFile();
  const changes = file ? fileChanges(file).length : 0;
  box.appendChild(el('span', { class: 'tabs-spacer' }));
  box.appendChild(el('button', {
    class: 'btn warn tab-revert',
    text: changes ? `Revert ${file.filename} (${changes} change${changes === 1 ? '' : 's'})` : 'Revert this file',
    ...(changes ? {} : { disabled: 'true' }),
    onclick: () => {
      if (!file) return;
      revertFile(file);
      state.selected = null;
      if (ui.status) {
        ui.status.textContent = `${file.filename} reverted to the JSON it was loaded with - save a copy to write it out`;
      }
      render();
    },
  }));
}

/// The keyboard a decode report records, stamped by the read as the report's first three fields:
/// `product_id` (a number), `product` (the model name Rust's registry lists for it) and `firmware`
/// (the version that answered the read's `0x90 00` poll). `null` when no product ID is stamped - a
/// `decode` of a file names no keyboard, only the schema word inside it. The report carries no vendor
/// (every SteelSeries device is 0x1038), no "known" flag and no platform: the crate names no vendor,
/// and which class a product ID opens as is a discovery-time fact, not a field of the report.
function deviceLabel(doc) {
  if (!doc || doc.product_id === null || doc.product_id === undefined) return null;
  const pid = Number(doc.product_id);
  const listed = importedName(pid) ?? PRODUCT_NAMES[pid] ?? null;
  const claim = doc.product ? String(doc.product) : null;
  // Rust names every ID it lists and answers a placeholder for the rest; a placeholder in a file is not
  // a model name, so an unlisted ID is shown by number.
  const placeholder = claim !== null && PRODUCTS !== null && claim === PRODUCTS.nameFallback;
  const name = listed ?? (placeholder ? null : claim) ?? `unknown product ${productHex(pid)}`;
  return {
    pid,
    hex: productHex(pid),
    name,
    firmware: doc.firmware === null || doc.firmware === undefined ? null : String(doc.firmware),
    // False when the crate's registry does not list the ID: its name is a placeholder, so the reader
    // trusts the number.
    known: listed !== null || Boolean(claim),
    legacy: onboardModelPids().includes(pid),
  };
}

function productHex(pid) {
  return `0x${Number(pid).toString(16).toUpperCase().padStart(4, '0')}`;
}

function renderHeader(file, changes) {
  const doc = file.doc;
  const edited = changes.length > 0;
  const toggle = lockWinBinding(doc);
  const rows = [
    line('Schema', el('span', { class: 'mono', text: `0x${hex2(doc.schema & 0xff)} ("${doc.schema_name}")` })),
    line('Device', el('span', {
      class: 'mono',
      text: (() => {
        const device = deviceLabel(doc);
        if (!device) return 'not recorded (decoded from a file, which names no keyboard)';
        return `${device.name} - ${device.hex} - ${IMPORT_SOURCE}`;
      })(),
    }), (() => {
      const device = deviceLabel(doc);
      if (!device) return 'a read stamps the keyboard it ran on into the JSON it writes; a file decode records nothing, so the page cannot name a model it was not told';
      const notes = [`model name ${IMPORT_SOURCE}`, `product ID ${device.hex}${device.known ? '' : ' - the crate registry does not list it, so trust the number and not the name'}`, `firmware ${device.firmware === null ? 'not recorded (the read\'s 0x90 00 poll got no answer)' : device.firmware}`];
      if (!device.legacy) {
        notes.push(`this board is not on the on-board family (${onboardModelPids().map(productHex).join(', ')}) the page decodes: a profile from it needs its own family's layout, encoder and feature set`);
      }
      return notes.join(' - ');
    })()),
    line('Firmware', el('span', {
      class: 'mono',
      text: (() => {
        const device = deviceLabel(doc);
        if (!device) return 'not recorded (a file decode records no firmware)';
        return device.firmware === null ? 'no answer to the read\'s 0x90 00 firmware poll' : device.firmware;
      })(),
    }), 'the version the read\'s 0x90 00 poll answered with, stamped into the report in front of the profile; 90 01 answers the LED controller\'s version, which the read does not ask, and a file decode records no firmware'),
    line('Features', el('span', { text: deviceFeatures(deviceLabel(doc) ? deviceLabel(doc).pid : -1).join(' \u00b7 ') }), 'the encodings and blocks this page has for this model; a model the on-board protocol was not read back on has a payload layout this page does not implement'),
    line('META toggle', el('span', {
      class: 'mono',
      text: (() => {
        const byte = Number(doc.meta_toggle_hid);
        const byteText = Number.isFinite(byte) && byte > 0 ? `${keyName(metaToggleByte(byte))} (meta_toggle_hid 0x${hex2(byte)})` : 'no key (meta_toggle_hid 0x00)';
        return `${byteText} | Lock Win bindings (62 05): ${toggle ? `${toggle.listName}[${toggle.offset}] = ${keyName(toggle.hid)}` : 'none in this file'}`;
      })(),
    }), `meta_toggle_hid (the byte at META_TOGGLE_HID_OFFSET) is the binding: it names the key GG switches META mode with, and binding a key as Meta Toggle writes that byte and nothing else - a wired read carries 0x10, the M key, whose own entries are untouched. GG's code for the Meta cap is 0x${hex2(META_KEY_GG_CODE)}, which every stock read here carries; the cap is usage 0x${hex2(META_KEY_HID)}, which has no slot of its own and binds through slot ${META_KEY_SLOT} (GG's code 0xF0, ${keyName(META_KEY_GG_CODE)} in this page's usage table - the M key). The 0x${hex2(META_TOGGLE_FUNCTION)} with code 1 = 0x${hex2(META_LOCK_WIN_ACTION)} pair is the Lock Win action of the META layer (slot 13 = LWin in a stock read, slots 13 and 35 in the user profile): a binding byte, not this byte; this row edits nothing and the two are edited apart from each other in the binding window. [EXPERIMENTAL] the binding byte's meaning is read-side evidence; writes of encoded profiles are hardware-tested on an Apex 7 (2026-10-20). The byte is edited in the key panel of the key you bind; the buttons below write it on the current file`),
    line('GUID', el('span', { class: 'mono', text: doc.guid }), doc.guid_padding ? `padding ${doc.guid_padding}` : 'schema b: no padding field in use'),
    line('Checksum', el('span', {
      class: 'mono',
      text: `stored ${doc.crc_stored} computed ${doc.crc_computed} ${doc.crc_ok ? 'match' : 'MISMATCH'}`,
    }), `over the first ${doc.crc_region_len} payload bytes; ${edited ? 'this file is edited, the checksums are the read\'s and are not recomputed here' : 'never recomputed by this editor'}`),
  ];
  const counts = bindingCounts(doc);
  rows.push(line('Bindings', el('span', {
    class: 'mono',
    text: counts.map(({ layer, counts: c }) => `${layer}: ${c.custom} custom, ${c.self} bound to themselves, ${c.none} empty`).join(' | '),
  }), 'GG writes a factory-default key as function 0x51 plus its own key code and an empty slot as all zero: neither is a binding, so only the custom count is a remap'));
  const perCodes = codesPerEntry(doc);
  if (perCodes < CODE_COUNT) {
    rows.push(line('Dump format', el('span', { class: 'mono', text: `${perCodes} key code(s) per mapping entry` }), 'read by the pre-fix decoder, which cut the fourth code of the 5-byte entry; the editor pads it with 0, which is what GG writes there - re-read the slot to get the real bytes'));
  }
  if (Number(doc.extra_len)) {
    rows.push(line('Extra region', el('span', {
      class: 'mono',
      text: `offset ${payloadLenForSchema(doc.schema) ?? doc.extra_offset ?? '?'} len ${Number(doc.extra_len)} fnv1a ${String(doc.extra_fnv1a_64 ?? '')} head ${String(doc.extra_head ?? '')}`,
    }), 'the region the read pulled behind the payload (the read fills the whole profile region, only its head is the payload): recorded by the read, decoded by nothing, and this page edits none of it - a payload-only report carries no such fields and shows no row'));
  }
  const oled = doc.oled;
  const macro = doc.macro_data;
  rows.push(line('OLED block', el('span', { class: 'mono', text: `offset ${oled.offset} len ${oled.len} nonzero ${oled.nonzero}` }), `fnv1a ${oled.fnv1a_64} head ${oled.head}`));
  const blockBytes = macroBlockBytes(doc);
  rows.push(line('Macro block', el('span', {
    class: 'mono',
    text: `offset ${macro.offset} len ${macro.len} nonzero ${macro.nonzero}${blockBytes ? ` bytes ${blockBytes.length}` : ''}`,
  }), blockBytes
    ? `fnv1a ${macro.fnv1a_64} - the block's ${blockBytes.length} bytes ride in the report and a write sends them back verbatim, the page edits none of them`
    : `fnv1a ${macro.fnv1a_64} - this report carries no block bytes (a read from before the codec shipped them): re-read the profile before writing it`));
  return panel(`File: ${file.filename}${edited ? ' (edited)' : ''}`, rows);
}

function mappingRows(doc, listName, index, mapping) {
  const rows = [];
  rows.push(line('Function', buildFunctionSelect(mapping.function), `function_name "${mapping.function_name}"`));
  const codes = mappingCodes(mapping);
  const media = mapping.function === 0x61;
  codes.list.forEach((code, position) => {
    rows.push(line(
      `Code ${position + 1}`,
      buildKeySelect(code, { functionByte: mapping.function }),
      `0x${hex2(code)} ${keyName(code, mapping.function)}`,
    ));
  });
  if (media) {
    rows.push(el('p', {
      class: 'hint',
      text: 'function 0x61 media_key: the codes are consumer-page usages - 0xE2 Mute, 0xB5 Next, '
        + '0xB6 Previous, 0xCD Play/Pause, 0xE9 Volume up, 0xEA Volume down (USB HID Usage Tables, consumer page). '
        + 'The same numbers under any other function are keyboard-page usages (0xE2 = LAlt).',
    }));
  }
  if (mapping.function === 0x71) {
    rows.push(el('p', {
      class: 'hint',
      text: `function 0x71 macro: the third code byte is the binding byte = ${mapping.macroIndex ?? macroIndex(mapping)}, `
        + `the record's offset in the block in 4-byte units. `
        + macroLine(doc, mapping.macroIndex ?? macroIndex(mapping))
        + ` Measured on a wired read (schema B) whose number row 3 to 7 is bound to macros: the bytes 25, 28, 33, `
        + '38 and 43 land on records at offsets 100, 112, 132, 152 and 172, and those records press mouse 1+2, 4, '
        + '5, 6 and 1+2+3 - which is what those keys do on that board. A byte whose record the block does not hold '
        + 'is a macro GG deleted with the binding left on the key.',
    }));
  }
  return rows.map((row, position) => {
    if (position === 0) {
      const select = row.querySelector('select');
      select.addEventListener('change', (event) => {
        const target = doc[listName][index];
        target.function = Number(event.target.value);
        normalizeEntry(target, index);
        render();
      });
      return row;
    }
    const select = row.querySelector('select');
    if (!select) return row; // the media-key hint row below the code selects
    select.addEventListener('change', (event) => {
      const target = doc[listName][index];
      const current = mappingCodes(target);
      current.list[position - 1] = Number(event.target.value);
      writeCodes(target, current.list, current.extra);
      normalizeEntry(target, index);
      render();
    });
    return row;
  });
}

// The binding panel. It edits one mapping entry - a key's slot of the 128-entry table, or one of the
// two wheel entries, which are the same function byte plus four key codes in a list of their own -
// and it edits it the way GG's UI reads: a binding type, then that type's own option, never a row of
// raw bytes. `target` names the list, the slot, the layer, the key the panel is about, and the factory
// binding of the slot (null where this editor has no factory binding measured). A
// function byte this editor has no group for keeps the raw rows, so nothing becomes uneditable.
function renderBindingPanel(file, target) {
  const doc = file.doc;
  const { listName, index, layer, hid, factory } = target;
  const entry = doc[listName][index];
  const codes = mappingCodes(entry).list;
  const name = hid === null || hid === undefined ? `${listName}[${index}]` : keyName(hid);
  const rows = [el('p', { class: 'hint', text: target.intro })];

  const write = (mutate) => {
    mutate();
    normalizeEntry(entry, index);
    render();
  };
  const writeFunction = (fn) => write(() => { entry.function = fn & 0xff; });
  const writeCodeByte = (position, value) => write(() => {
    const current = mappingCodes(entry);
    current.list[position] = value & 0xff;
    writeCodes(entry, current.list, current.extra);
  });
  const writeNone = () => write(() => {
    const { extra } = mappingCodes(entry);
    entry.function = 0x00;
    writeCodes(entry, [0, 0, 0, 0], extra);
  });
  // The META toggle, written the way the reads show it: the profile byte and nothing else. In a
  // wired read the byte names the M key (0x10) while that key's base entry stays self-bound
  // (51 10 00 00 00) and its META entry stays all zero - so being the toggle key touches no binding
  // byte on any layer. `0` means the file names no toggle key. The binding select's function byte is
  // left exactly as it is: this writes one byte of the profile, not this entry.
  const writeToggleByte = (on) => {
    const previous = Number(doc.meta_toggle_hid ?? 0);
    const next = on ? toggleByteFor(hid) : 0x00;
    write(() => {
      doc.meta_toggle_hid = next;
    });
    ui.status.textContent = `meta_toggle_hid 0x${hex2(previous)} -> 0x${hex2(next)}: ${on ? `${keyName(hid)} is now the key GG switches META mode with` : 'this file names no toggle key'} - the byte is the whole of the binding, this entry's bytes are untouched, and the board paints the named key as a meta key with META in its cap. [EXPERIMENTAL] the byte's meaning is read-side evidence; writes of encoded profiles are hardware-tested on an Apex 7 (2026-10-20)`;
    render();
  };

  const selectRow = (label, options, selected, note, onPick, cls) => {
    const select = optionSelect(options, selected, cls);
    // The binding-type select carries group ids, the option selects carry byte values.
    const labelled = options.length > 0 && typeof options[0].value === 'string';
    select.addEventListener('change', (event) => onPick(labelled ? event.target.value : Number(event.target.value)));
    return line(label, select, note);
  };

  // The key the byte names: GG shows it as Meta - Meta Toggle, so on the base layer the selects open
  // on Meta with the Meta Toggle action, whatever the entry's own bytes say.
  // The key the profile byte names is painted as a meta key on the board and its cap prints META;
  // that is the byte's doing alone. Its binding window shows the entry's own bytes - the toggle is not
  // a binding (Lock Win and the META toggle are separate options).
  const group = bindingGroup(entry.function);
  rows.push(selectRow(
    'Binding',
    bindingGroupOptions(entry.function),
    group ? group.id : 'raw',
    `function 0x${hex2(entry.function)} ${entry.function_name}`,
    (value) => {
      const picked = BINDING_GROUPS.find((item) => item.id === value);
      if (!picked) return;
      if (picked.id === 'none') { writeNone(); return; }
      if (picked.id === 'meta') {
        // GG's META-layer meta binding is function 0x62 with the action in code 1 and 01 in code 2
        // (01 00 00 00 on the F9 slot, then 02, 03, 04). Binding a key as Meta on any layer starts
        // from that shape; the old key codes would leave code 1 holding a key code, which is not a
        // meta action at all.
        write(() => {
          const { extra } = mappingCodes(entry);
          entry.function = META_TOGGLE_FUNCTION;
          writeCodes(entry, [0x01, 0x01, 0, 0], extra);
        });
        return;
      }
      writeFunction(picked.functions[0]);
    },
    'fn-select',
  ));

  if (!group) {
    rows.push(...mappingRows(doc, listName, index, entry));
    rows.push(el('p', {
      class: 'hint',
      text: `0x${hex2(entry.function)} is not one of the binding types this editor knows, so the function byte and the four code bytes are shown as they are.`,
    }));
  } else if (group.id === 'none') {
    rows.push(el('p', {
      class: 'hint',
      text: 'Deactivated: function 0x00 with the four code bytes 00, which is what GG writes for a key the profile does not bind. Nothing else to choose.',
    }));
  } else if (group.id === 'mouse') {
    rows.push(selectRow(
      'Mouse action',
      mouseActionOptions(entry.function),
      entry.function,
      `function 0x${hex2(entry.function)} ${entry.function_name}, codes ${hexOf(codes)}`,
      writeFunction,
      'fn-select',
    ));
    rows.push(el('p', {
      class: 'hint',
      text: '[EXPERIMENTAL] the mouse action is the function byte; the four code bytes are left exactly as the file has them. No factory read contains a mouse binding - a stock slot read holds 0x51, 0x61 and 0x00 entries only - so what GG writes into the code bytes of a mouse key is not measured.',
    }));
  } else if (group.id === 'keyboard') {
    codes.forEach((code, position) => {
      rows.push(selectRow(
        `Key ${position + 1}`,
        keyboardKeyOptions(code),
        code,
        `code ${position + 1}: 0x${hex2(code)} ${keyName(code)}`,
        (value) => writeCodeByte(position, value),
      ));
    });
    rows.push(el('p', {
      class: 'hint',
      text: 'function 0x51 (combo_key) is GG keyboard binding: up to four key codes, 0x00 for a slot left empty. A key bound to itself is its own usage in code 1 and 00 00 00 after it, which is every bound key of a factory profile.',
    }));
  } else if (group.id === 'media') {
    rows.push(selectRow(
      'Media key',
      mediaKeyOptions(codes[0]),
      codes[0],
      `code 1: 0x${hex2(codes[0])} ${keyName(codes[0], 0x61)}`,
      (value) => writeCodeByte(0, value),
    ));
    rows.push(el('p', {
      class: 'hint',
      text: 'function 0x61 (media_key): the code is a consumer-page usage (USB HID Usage Tables). The same number under any other function is a keyboard-page key - 0xE2 is Mute here and LAlt there.',
    }));
  } else if (group.id === 'meta') {
    rows.push(selectRow(
      'Meta action',
      metaActionOptions(codes[0]),
      codes[0],
      `code 1: 0x${hex2(codes[0])}, codes ${hexOf(codes)}`,
      (value) => writeCodeByte(0, value),
    ));
    rows.push(el('p', {
      class: 'hint',
      text: `function 0x62 (meta): the action is code 1, and it is named like the other actions - picking one writes code 1 and keeps the rest of the code bytes. A factory META layer binds four meta keys to META+F9..F12 (measured in a stock slot read: 01 00 00 00 on the F9 slot, then 02, 03 and 04, each with 01 in code 2). ${META_ACTION_NAMES[META_LOCK_WIN_ACTION]} (0x${hex2(META_LOCK_WIN_ACTION)}) is a meta action like those: it writes this entry's code 1 and touches nothing else - the META toggle is the profile byte below, and picking Lock Win no longer moves it (operator, 2026-10-19). GG's own reads carry 62 05 on the META layer of slot 13 = LWin${lockWinMarkers(doc).length ? ` (slots ${lockWinMarkers(doc).map((m) => String(m.offset)).join(', ')} in this file)` : ''}. [EXPERIMENTAL] measured in reads; writes of encoded profiles are hardware-tested on an Apex 7 (2026-10-20)`,
    }));
  } else {
    rows.push(selectRow(
      'Macro',
      macroBindingOptions(doc, codes[2]),
      codes[2],
      `code 3 holds the record index: ${codes[2]}`,
      (value) => writeCodeByte(2, value),
    ));
    rows.push(...macroDetailRows(doc, codes[2]));
  }

  // The META toggle is a profile byte of its own, and this row is the only place that writes it:
  // separate from the Meta action select above, which is a binding byte. A wheel entry has no slot in
  // the byte's table, so the row is not offered there.
  const byte = Number(doc.meta_toggle_hid ?? 0);
  if (!target.wheelSlot) {
    const namesThisKey = byte === toggleByteFor(hid);
    const toggleBox = el('input', { class: 'toggle-check', type: 'checkbox' });
    toggleBox.checked = namesThisKey;
    toggleBox.addEventListener('change', (event) => writeToggleByte(event.target.checked));
    rows.push(line('Meta Toggle', toggleBox, `meta_toggle_hid = 0x${hex2(byte)} (${byte > 0 ? `names ${keyName(metaToggleByte(byte))}` : 'no key'}); checking it writes 0x${hex2(toggleByteFor(hid))} - this key's code${namesThisKey ? '' : `, which moves it off ${byte > 0 ? keyName(metaToggleByte(byte)) : 'no key'}`} - and unchecking writes 0x00. The byte is the whole of the binding: this entry's bytes are untouched, and a wired read shows exactly that shape (byte 0x10 names the M key, base entry self-bound 51 10 00 00 00, META entry all zero). [EXPERIMENTAL] read-side evidence; writes of encoded profiles are hardware-tested on an Apex 7 (2026-10-20)`));
  }

  const original = file.original?.[listName]?.[index];
  const dirty = !!original
    && (original.function !== entry.function
      || String(original.key_codes).toUpperCase() !== String(entry.key_codes).toUpperCase());
  const atFactory = factory
    ? factory.function === entry.function && factory.key_codes === String(entry.key_codes).toUpperCase()
    : true;
  const buttons = [
    el('button', {
      class: 'btn',
      text: 'Undo the changes on this key',
      ...(dirty ? {} : { disabled: 'true' }),
      onclick: () => {
        if (!original) return;
        entry.function = original.function;
        entry.key_codes = original.key_codes;
        normalizeEntry(entry, index);
        render();
      },
    }),
  ];
  if (factory) {
    buttons.push(el('button', {
      class: 'btn',
      text: 'Reset this key to its factory binding',
      ...(atFactory ? { disabled: 'true' } : {}),
      onclick: () => {
        entry.function = factory.function;
        entry.key_codes = factory.key_codes;
        normalizeEntry(entry, index);
        render();
      },
    }));
  }
  if (buttons.length) rows.push(el('div', { class: 'btnrow' }, buttons));
  rows.push(el('p', {
    class: 'hint',
    text: (factory
      ? `${target.wheelSlot
        ? `Factory binding of ${listName}[${index}] - measured in a stock slot read and byte for byte the same in every read this page decodes: GG pairs the scroll wheel with the numpad + and / keys, binds Media Vol+ on the up row and Media Vol- on the down row, and leaves both META-layer rows deactivated`
        : `Factory binding of this slot (${keyName(boardHid(OFF2HID[index]))} on the base layer)`}: function 0x${hex2(factory.function)} ${functionName(factory.function)} codes ${factory.key_codes}.`
        + (dirty ? ` This key is changed: 0x${hex2(entry.function)} ${hexOf(codes)}, and the file as loaded had 0x${hex2(original.function)} ${original.key_codes}.` : '')
      : `This entry has no factory binding measured in this editor, so there is nothing to reset to.`
        + (dirty ? ` This entry is changed: 0x${hex2(entry.function)} ${hexOf(codes)}, and the file as loaded had 0x${hex2(original.function)} ${original.key_codes}.` : ` The file as loaded had function 0x${hex2(original.function)} ${original.key_codes}.`))
      + ' Undo and factory reset change this entry only: the profile byte meta_toggle_hid is a separate field and shows in the tab row as a change count.',
  }));
  return panel(`${name} - ${layer} layer`, rows);
}

function renderKeyPanel(file) {
  const doc = file.doc;
  const hid = state.selected;
  if (hid === null || hid === undefined) {
    return panel('Key', [el('p', {
      class: 'hint',
      text: 'click a key on the board, one of the two wheel caps, or the OLED rectangle. The panel then binds what you clicked.',
    })]);
  }
  const layer = state.layer;
  if (hid === WHEEL_UP || hid === WHEEL_DOWN) {
    const index = hid === WHEEL_UP ? 0 : 1;
    const listName = layer === LAYER_META ? 'wheel_meta_mappings' : 'wheel_mappings';
    if (!doc[listName]?.[index]) {
      return panel(`Key ${keyName(hid)}`, [el('p', { class: 'hint', text: `this file has no ${listName}[${index}]: the wheel lists hold 2 entries each.` })]);
    }
    return renderBindingPanel(file, {
      listName,
      index,
      layer,
      hid,
      factory: factoryEntry(listName, index),
      wheelSlot: true,
      intro: `The wheel caps are not 128-table offsets: this cap is ${listName}[${index}], the entry GG pairs with the wheel (${layer} layer). It binds like any key - function byte plus four key codes - and the Meta action list offers no Meta Toggle here, because a wheel entry cannot be the key that switches the layer.`,
    });
  }
  // The Meta cap has no slot under its own usage; hidSlot resolves it to slot 93, the slot GG binds it
  // through, so it edits like any other key (see META_KEY_SLOT).
  const offset = hidSlot(hid);
  if (offset < 0) {
    return panel(`Key ${keyName(hid)}`, [
      el('p', { class: 'hint', text: `0x${hex2(hid)} has no mapping offset in the legacy HID table, so this key has no entry in the mapping tables.` }),
    ]);
  }
  const listName = layer === LAYER_META ? 'button_meta_mappings' : 'button_mappings';
  const intro = hid === META_KEY_HID
    ? `The Meta cap (0x${hex2(META_KEY_HID)}, electrically the HID Application key - the keystroke that opens the Context Menu) has no slot under its own usage, so it binds through ${listName}[${offset}]: the slot whose table code is GG's 0x${hex2(META_KEY_GG_CODE)}, which this page's usage table names ${keyName(META_KEY_GG_CODE)} - the M key of the Apex 7. A stock read binds that slot to itself (51 ${hex2(META_KEY_GG_CODE)} 00 00 00), and a wired read binds it to 0x65, the Context Menu keystroke. The profile byte is the META toggle binding: meta_toggle_hid 0x${hex2(Number(doc.meta_toggle_hid))}${Number(doc.meta_toggle_hid) === META_KEY_GG_CODE ? ' = this cap' : ''}.`
    : `${keyName(hid)} (0x${hex2(hid)}) -> ${listName}[${offset}] of the ${layer} layer`;
  return renderBindingPanel(file, {
    listName,
    index: offset,
    layer,
    hid,
    factory: factoryEntry(listName, offset),
    intro,
  });
}

// The custom bindings of both layers, in slot order. This is the answer to "what does this profile
// actually change", and it is empty for a stock profile.
function renderBindingsPanel(file) {
  const doc = file.doc;
  const rows = [];
  for (const layer of [LAYER_BASE, LAYER_META]) {
    const list = customBindings(doc, layer);
    if (!list.length) {
      rows.push(el('p', {
        class: 'hint',
        text: `${layer} layer: no custom bindings - every key GG can bind is bound to itself, and the slots GG does not use are empty`,
      }));
      continue;
    }
    rows.push(el('p', { class: 'hint', text: `${layer} layer: ${list.length} custom binding(s)` }));
    rows.push(el('table', { class: 'grid' }, [
      el('tr', {}, [
        el('th', { text: 'key' }),
        el('th', { text: 'slot' }),
        el('th', { text: 'function' }),
        el('th', { text: 'codes' }),
        el('th', { text: 'reads as' }),
      ]),
      ...list.map(({ entry, offset, hid, board }) => {
        const codes = mappingCodes(entry).list;
        const state = keyState(doc, layer, hid);
        const tr = el('tr', { class: 'clickable' }, [
          el('td', { class: 'mono', text: hid === null || hid === undefined ? '-' : keyName(board) }),
          el('td', { class: 'mono', text: String(offset) }),
          el('td', { class: 'mono', text: `0x${hex2(entry.function)} ${functionName(entry.function)}` }),
          el('td', { class: 'mono', text: hexOf(codes) }),
          el('td', { text: state.label }),
        ]);
        tr.addEventListener('click', () => {
          if (hid === null || hid === undefined) return;
          selectKey(layer, hid);
        });
        return tr;
      }),
    ]));
  }
  return panel('Custom bindings', rows);
}

function renderLightingPanel(file) {
  const doc = file.doc;
  const light = lighting(doc);
  if (!light) return panel('Lighting', [el('p', { class: 'hint', text: 'this dump has no lighting block' })]);
  const colorRow = (label, field, note) => {
    const input = el('input', { type: 'color', class: 'color', value: colorToHex(light[field]) });
    input.addEventListener('input', (event) => {
      const color = hexToColor(event.target.value);
      if (color) setLightingColor(doc, field, color);
      render();
    });
    return line(label, input, note ?? `${field}: ${hexOf([light[field].r, light[field].g, light[field].b])}`);
  };
  const hexFieldRow = (label, field, bytes, note) => line(label, textInput(light[field], (value) => {
    const parsed = parseHex(value);
    if (!parsed || parsed.length !== bytes) {
      ui.status.textContent = `${field} needs ${bytes} hex bytes`;
      return;
    }
    light[field] = hexOf(parsed);
    render();
  }), `${field}: ${bytes} bytes, hex${note ? `. ${note}` : ''}`);
  const winRow = line('Win key code', buildKeySelect(light.win_key_hid_code, { noneLabel: '0x00 - none' }), `0x${hex2(light.win_key_hid_code)} ${keyName(light.win_key_hid_code)}`);
  winRow.querySelector('select').addEventListener('change', (event) => {
    light.win_key_hid_code = Number(event.target.value);
    render();
  });
  const oledNote = [
    "GG's name for these six bytes - nothing in the engine binary and nothing in the 4.16.8 firmware names the",
    "fields, and the name comes from the encrypted lisp spec. GG's Apex 7 front end offers no OLED option beyond",
    "the image upload (operator-reported 2026-10-23), so these are the keyboard's own DISPLAY-menu settings the",
    "profile carries: BRIGHTNESS, DISPLAY ICONS, TIMEOUT. Which byte is which is unverified - the read-out is set",
    "one on the board, read the slot, diff it. The captures this page ships read 02 00 00 00 00 00 on the live",
    "board's slot and 00 02 00 00 00 00 on the stock slot, and this page never edits these bytes on a write: a",
    "decoded payload round-trips them byte for byte.",
  ].join(' ');
  const dimmerNote = [
    "0-255, and every capture this page ships carries 255. The board's own menu offers DIM and BRIGHTNESS levels.",
    "Unverified as a brightness scale.",
  ].join(' ');
  const timeoutNote = [
    "u32 ms, and 0 in every capture this page ships. The board's own ladder, read out of the 4.16.8 firmware",
    "2026-10-23, is 5 SECS, 10 SECS, 30 SECS, 1 MIN, 5, 10, 15, 30, 45 MINS, 1, 2, 3, 5 HOURS, NONE, NEVER - so 0,",
    "5000, 10000, 30000, 60000, 300000, 600000, 900000, 1800000, 2700000, 3600000, 7200000, 10800000, 18000000 are",
    "the values it can show. Unverified as this field's units.",
  ].join(' ');
  const rows = [
    colorRow('Lock-key cluster', 'lock_key_color'),
    colorRow('META cluster', 'meta_key_color'),
    colorRow('Disabled Win key', 'disabled_win_key_color'),
    hexFieldRow('Lock-key HID codes', 'lock_key_hid_codes', 3),
    (() => {
      const derived = computeMetaMask(doc);
      const stored = light.meta_mapped_elements;
      const bits = metaMaskDiffers(stored, derived);
      return line('META mapped mask', el('span', { class: 'mono', text: derived }), `derived from the META layer, so it is not editable: bit n is set for every key whose META-layer entry carries a binding (function byte != 0), the bit index is that key's own HID usage, the key meta_toggle_hid names (0x${hex2(Number(doc.meta_toggle_hid ?? 0))}) is left out, and the wheel's two entries are never in it. ${bits === 0 ? 'the bytes this read carries (meta_mapped_elements) are the same' : `this read carries ${stored}, which differs from the derived mask in ${bits} bit(s) - a write sends the derived mask`} [EXPERIMENTAL] measured bit-for-bit in the reports this page decodes, and verified on hardware 2026-10-20 (a write carrying it applied: the META lighting followed the META-layer bindings with the meta_toggle_hid key left out)`);
    })(),
    hexFieldRow('OLED illumination options', 'oled_illumination_options', 6, oledNote),
    winRow,
    line('META background dimmer', numberInput(light.meta_key_background_dimmer, 0, 255, (value) => {
      light.meta_key_background_dimmer = value;
      render();
    }), dimmerNote),
    line('Idle-mode dimmer', numberInput(light.idle_mode_dimmer, 0, 255, (value) => {
      light.idle_mode_dimmer = value;
      render();
    }), dimmerNote),
    line('Idle timeout (ms)', numberInput(light.idle_timeout_ms, 0, 4294967295, (value) => {
      light.idle_timeout_ms = value;
      render();
    }), timeoutNote),
  ];
  return panel('Lighting block', rows);
}

function renderRapidTapPanel(file) {
  const doc = file.doc;
  const rt = rapidTap(doc);
  if (!rt) {
    return panel('Rapid Tap', [el('p', { class: 'hint', text: 'this dump has no Rapid Tap block: it is only in schema b (0x0B)' })]);
  }
  const rows = [
    line('Enable byte', numberInput(rt.enable, 0, 255, (value) => {
      rt.enable = value;
      render();
    }), 'the decoder shows the raw byte; ssgg never writes it'),
  ];
  rt.pairs.forEach((pair, index) => {
    const row = el('div', { class: 'line pair' }, [
      el('span', { class: 'line-label', text: `Pair ${index + 1}` }),
      el('span', { class: 'line-control' }, [
        buildKeySelect(pair.hid_1, { noneLabel: '0x00' }),
        buildKeySelect(pair.hid_2, { noneLabel: '0x00' }),
        numberInput(pair.mode, 0, 255, (value) => {
          pair.mode = value;
          render();
        }),
        el('label', { class: 'check' }, [
          el('input', {
            type: 'checkbox',
            ...(pair.report_both_keys ? { checked: 'checked' } : {}),
            onchange: (event) => {
              pair.report_both_keys = event.target.checked;
              render();
            },
          }),
          el('span', { text: 'report both' }),
        ]),
      ]),
      el('span', { class: 'line-note', text: `mode 0x${hex2(pair.mode)} report bit 0x${hex2(MASK_MODE_REPORT_BOTH)} reserved ${pair.reserved || '00 00'}` }),
    ]);
    const selects = row.querySelectorAll('select');
    selects[0].addEventListener('change', (event) => {
      pair.hid_1 = Number(event.target.value);
      render();
    });
    selects[1].addEventListener('change', (event) => {
      pair.hid_2 = Number(event.target.value);
      render();
    });
    rows.push(row);
  });
  rows.push(el('p', {
    class: 'hint',
    text: 'pairs reference keys by raw HID code, not by mapping offset. Each pair is 5 bytes: hid_1, hid_2, the mode byte, the report bit and the reserved bytes the decoder carries as hex - this page edits the first four and writes the reserved bytes back exactly as the file has them. A Rapid Tap key is marked on the board with a bracket.',
  }));
  return panel('Rapid Tap', rows);
}

// Macro record marker of the packed block (apex_7_profile::MACRO_RECORD_MARKER). The decoder
// decodes the block; this page reads the records it wrote.
const MACRO_MARKER = 0x77;
const MACRO_MODE_NAMES = { 4: 'repeat', 1: 'once' };
// The unit a binding byte counts (apex_7_profile::MACRO_INDEX_UNIT): the macro a `0x71` row plays
// starts at `byte x 4` bytes into the block. Measured on a wired read (schema B) whose keys c, v, x
// carry the bytes 0, 7, 14 and whose macros 0, 28, 56 bytes into the block press LCtrl with c, v, x,
// and on a read whose number row carries 25, 28, 33, 38, 43 over macros at 100, 112, 132, 152, 172
// pressing mouse masks 0x03, 0x08, 0x10, 0x20, 0x07. The byte is one code byte, so it reaches
// 255 x 4 = 1020 bytes of the block, not the block's 9600.
const MACRO_INDEX_UNIT = 4;
const MACRO_BINDING_BYTE_MAX = 255;
const MACRO_BINDING_REACH_BYTES = MACRO_BINDING_BYTE_MAX * MACRO_INDEX_UNIT;
// The mouse-button mask bits, same order as the codec names them.
const MACRO_MOUSE_BITS = [[0x01, 'mouse 1'], [0x02, 'mouse 2'], [0x04, 'mouse 3'], [0x08, 'mouse 4'], [0x10, 'mouse 5'],
  [0x20, 'mouse 6'], [0x40, 'mouse 7'], [0x80, 'mouse 8']];

// Records of a dump, from the `macros` array the writer emits. An older dump has no such field.
function macroRecords(doc) {
  return Array.isArray(doc.macros) ? doc.macros : [];
}

// Third code byte of a 0x71 slot = the binding byte (KeyMapping::macro_index).
function macroIndex(entry) {
  return mappingCodes(entry).list[2];
}

// Where this report's macro block starts. The read reports it; a report without one is schema b.
function macroRegionStart(doc) {
  const offset = (doc.macro_data ?? {}).offset;
  return Number.isInteger(offset) ? offset : MACRO_DATA_OFFSET_B;
}

// The binding byte that reaches the macro packed `bytesFromStart` into the block.
function macroBindingByte(bytesFromStart) {
  return Math.floor(bytesFromStart / MACRO_INDEX_UNIT);
}

// The block offset a binding byte names. The macro that starts there is the one it plays; a byte
// pointing inside a record - not at its first byte - names no macro.
function macroOffsetForBindingByte(byte) {
  return byte * MACRO_INDEX_UNIT;
}

// Position of a record in the list the decoder walked. Labels use it; a binding never does.
function macroRecordOrdinal(record) {
  return record.ordinal ?? 0;
}

// The macro a binding byte names. A binding names a place in the block, never the header word.
function macroRecordFor(doc, byte) {
  const at = macroRegionStart(doc) + macroOffsetForBindingByte(byte);
  return macroRecords(doc).find((record) => record.offset === at) ?? null;
}

// A mouse mask in words: 0x03 presses the left and right buttons together.
function macroMouseMaskName(mask) {
  if (!Number.isInteger(mask)) return 'no mask';
  const bits = MACRO_MOUSE_BITS.filter(([bit]) => mask & bit).map(([, name]) => name);
  return bits.length ? bits.join('+') : `mask 0x${hex2(mask)}`;
}

// A record as the key sequence GG shows in its UI: presses with the delays between them. A macro that
// drives the mouse is named by the mask it presses.
function macroKeystrokes(record) {
  if (!record || !Array.isArray(record.actions)) return 'no actions';
  const out = [];
  for (const action of record.actions) {
    if (action.kind === 'press') out.push(keyName(action.key));
    else if (action.kind === 'press_mouse') out.push(`${macroMouseMaskName(action.mask)} down (mask 0x${hex2(action.mask ?? 0)})`);
    else if (action.kind === 'delay') out.push(`${action.ms} ms`);
  }
  return out.length ? out.join(' > ') : 'no keys';
}

// Keys of the profile bound to a record, so each record row says who uses it.
function macroBoundKeys(doc, record) {
  const names = [];
  for (const [listName, prefix] of [['button_mappings', ''], ['button_meta_mappings', 'Meta+']]) {
    (doc[listName] ?? []).forEach((mapping, offset) => {
      if (mapping.function !== 0x71) return;
      const byte = mapping.macro_index ?? macroIndex(mapping);
      if (macroRegionStart(doc) + macroOffsetForBindingByte(byte) !== record.offset) return;
      const hid = boardHid(OFF2HID[offset]);
      if (hid === null || hid === undefined) return;
      names.push(`${prefix}${keyName(hid)}`);
    });
  }
  return names;
}

// The target of one macro binding, in words.
// The record a 0x71 binding points at, in full, as rows of its own: the mode, id and interval, every
// keystroke on its own line, the raw bytes, and the keys of the profile bound to the same record.
function macroDetailRows(doc, byte) {
  const record = macroRecordFor(doc, byte);
  const at = macroRegionStart(doc) + macroOffsetForBindingByte(byte);
  if (!record) {
    return [el('p', {
      class: 'hint',
      text: `Binding byte ${byte} names the macro starting ${macroOffsetForBindingByte(byte)} bytes into the block (payload ${at}). This block carries no macro at that byte - GG deletes the macro and leaves the binding on the key. Nothing to show.`,
    })];
  }
  const actions = (record.actions ?? []).map((action) => (action.kind === 'press'
    ? `press ${keyName(action.key)}`
    : action.kind === 'release' ? `release ${keyName(action.key)}`
      : action.kind === 'press_mouse' ? `press ${macroMouseMaskName(action.mask)}`
        : action.kind === 'release_mouse' ? `release ${macroMouseMaskName(action.mask)}`
          : `wait ${action.ms} ms`));
  const bound = macroBoundKeys(doc, record);
  return [
    el('h3', {
      class: 'subhead',
      text: `the whole macro: record #${macroRecordOrdinal(record)} (binding byte ${byte}) - ${record.mode_name ?? MACRO_MODE_NAMES[record.mode] ?? `mode 0x${hex2(record.mode)}`} (0x${hex2(record.mode)}), interval ${record.interval_ms} ms, ${record.action_count ?? actions.length} action(s)`,
    }),
    el('ul', { class: 'macro-actions' }, actions.map((text) => el('li', { class: 'mono', text }))),
    el('p', { class: 'hint mono', text: record.raw }),
    line('Bound by', el('span', {
      class: 'mono',
      text: bound.length ? bound.join(', ') : 'no key',
    }), `payload offset ${record.offset}, ${record.record_bytes ?? '?'} bytes, header word ${record.header_word ?? 'n/a'} - a binding names where the macro sits, so every key bound to this byte plays this one macro`),
  ];
}

function macroLine(doc, byte) {
  const record = macroRecordFor(doc, byte);
  const named = `binding byte ${byte} -> the macro ${macroOffsetForBindingByte(byte)} bytes into the block; `;
  if (!record) {
    return `${named}no macro starts at that byte in this block: GG deletes the macro and leaves the binding on the key. `;
  }
  const mode = record.mode_name ?? MACRO_MODE_NAMES[record.mode] ?? `mode 0x${hex2(record.mode)}`;
  const interval = record.mode === 4 ? ` every ${record.interval_ms} ms` : '';
  return `${named}${mode} (0x${hex2(record.mode)})${interval}, ${record.action_count ?? (record.actions ?? []).length} action(s) - the whole record, keystroke by keystroke, is in the block under the macro select. `;
}

function blockHasBytes(doc) {
  return typeof (doc.macro_data ?? {}).bytes_hex === 'string';
}

// ---------------------------------------------------------------- macro editor window
//
// The macro editor (macros.js) edits the packed record list as key push / key release / wait
// instructions. Under `ssgg gui` the page carries the run token, so the profile travels through the
// editor session on the server (`/api/macros/session`): the open profile goes out when the window opens,
// the edited list comes back on apply, and the server validates every apply with the same codec
// `ssgg onboard write` uses - the numbers the window refuses with are the numbers the write refuses
// with. Opened from the filesystem there is no token and no session, and the same editor mounts in this
// page (the section under the toolbar) and writes into the open file. Neither path writes a keyboard.

const MACRO_CHANNEL = 'ssgg-macros';
const macroEditor = { popup: null, revision: 0, channel: null, mounted: null };

// The `ssgg gui` token this page was served with, null for a page opened from the filesystem.
function sessionToken() {
  return globalThis.SsggLive ? globalThis.SsggLive.tokenFrom(document) : null;
}

async function macroSessionPost(doc) {
  const token = sessionToken();
  if (!token || !globalThis.SsggLive) return { ok: false, error: 'no editor session: this page was not served by `ssgg gui`' };
  try {
    // What is posted is exactly what "Save a copy" would write, so the window edits the same bytes.
    const reply = await globalThis.SsggLive.call(token, '/api/macros/session', { profile: JSON.parse(serializeDoc(doc)) });
    macroEditor.revision = reply.revision;
    return { ok: true, reply };
  } catch (error) {
    return { ok: false, error: error.message };
  }
}

async function macroSessionGet() {
  const token = sessionToken();
  if (!token || !globalThis.SsggLive) return { ok: false, error: 'no editor session' };
  try {
    return { ok: true, session: await globalThis.SsggLive.call(token, '/api/macros/session') };
  } catch (error) {
    return { ok: false, error: error.message };
  }
}

// The edited records land on the open file as a change against it: the file keeps its name and its
// `original`, so the tab row's change count marks them and Save a copy writes them out.
function adoptMacroRecords(records) {
  const file = activeFile();
  const editor = globalThis.SsggMacroEditor;
  if (!file) return { ok: false, error: 'no profile is open' };
  if (!editor) return { ok: false, error: 'macros.js is not loaded' };
  const region = editor.regionOf(file.doc);
  const problems = editor.limitProblems(records, region);
  if (problems.length) return { ok: false, error: problems.join(' - ') };
  file.doc = editor.applyRecords(file.doc, records, region);
  state.selected = null;
  render();
  const size = editor.plan(records, region);
  return { ok: true, records: size.records, used: size.used, free: size.free };
}

// A whole profile from the editor session: the window is the one that holds the edited list, so its
// reply replaces the open document (last write wins, which is why the status line says where it came from).
function adoptProfileJson(profile) {
  const file = activeFile();
  if (!file) return false;
  const problem = isValidProfileDoc(profile);
  if (problem) return false;
  file.doc = normalizeDoc(cloneDoc(profile));
  state.selected = null;
  render();
  return true;
}

async function syncMacroSession(reason) {
  const got = await macroSessionGet();
  if (!got.ok || got.session.revision === macroEditor.revision) return false;
  macroEditor.revision = got.session.revision;
  const adopted = adoptProfileJson(got.session.profile);
  if (adopted && ui.status) {
    ui.status.textContent = `the macro editor window applied its records ${reason} (session revision ${got.session.revision})`;
  }
  return adopted;
}

function mountInlineMacroEditor(file) {
  const box = ui.macroBox;
  const editor = globalThis.SsggMacroEditor;
  if (!box || !editor) return false;
  box.hidden = false;
  const close = el('button', { type: 'button', class: 'btn', text: 'close the macro editor' });
  close.addEventListener('click', () => {
    box.hidden = true;
    box.replaceChildren();
    macroEditor.mounted = null;
  });
  const host = el('section', { class: 'macro-editor' });
  box.replaceChildren(el('div', { class: 'btnrow' }, [close]), host);
  macroEditor.mounted = editor.mount(host, {
    doc: file.doc,
    embedded: true,
    onSave: (records) => {
      const result = adoptMacroRecords(records);
      if (macroEditor.mounted) {
        macroEditor.mounted.setStatus(result.ok
          ? `applied: ${result.records} record(s), ${result.used} bytes used, ${result.free} free - the tab row marks the file with the change count, and Save a copy writes the JSON`
          : `not applied: ${result.error}`);
      }
    },
  });
  return true;
}

function openMacroEditor() {
  const file = activeFile();
  const editor = globalThis.SsggMacroEditor;
  if (!file) {
    ui.status.textContent = 'open a profile JSON first: the macro editor edits the profile that is open';
    return;
  }
  if (!editor) {
    ui.status.textContent = 'macros.js is not loaded, so there is no macro editor to open';
    return;
  }
  const blocked = editor.editorDisabledReason(file.doc);
  if (blocked) {
    ui.status.textContent = `the macro editor is not available for this profile: ${blocked}`;
    return;
  }
  if (!sessionToken()) {
    mountInlineMacroEditor(file);
    ui.status.textContent = 'macro editor mounted under the toolbar (no `ssgg gui` session on a page opened from the filesystem)';
    return;
  }
  ui.status.textContent = 'opening the macro editor...';
  macroSessionPost(file.doc).then((posted) => {
    if (!posted.ok) {
      mountInlineMacroEditor(file);
      ui.status.textContent = `no editor session (${posted.error}) - the macro editor is mounted under the toolbar`;
      return;
    }
    const popup = typeof window !== 'undefined' && typeof window.open === 'function'
      ? window.open('/macros.html', 'ssgg-macro-editor', 'width=1080,height=860')
      : null;
    if (!popup) {
      mountInlineMacroEditor(file);
      ui.status.textContent = 'this window cannot open a second window - the macro editor is mounted under the toolbar';
      return;
    }
    macroEditor.popup = popup;
    if (typeof BroadcastChannel !== 'undefined' && !macroEditor.channel) {
      macroEditor.channel = new BroadcastChannel(MACRO_CHANNEL);
      macroEditor.channel.addEventListener('message', (event) => {
        if (event.data && event.data.type === 'macros-applied') void syncMacroSession('from the macro editor window');
      });
    }
    ui.status.textContent = `macro editor opened in its own window for ${file.doc.name} - apply hands the records back to this page`;
  });
}


function macroSummary(doc) {
  const records = macroRecords(doc);
  if (records.length) {
    return `${records.length} macro record(s): ${records.map((record) => `#${record.index} ${record.mode_name ?? `0x${hex2(record.mode)}`}${record.mode === 4 ? ` every ${record.interval_ms} ms` : ''}`).join(', ')}`;
  }
  if (doc.macro_note) {
    return blockHasBytes(doc)
      ? `the record list stops early (${doc.macro_note}); the block's bytes ride in the report and the write sends them back verbatim`
      : `the macro block does not parse (${doc.macro_note}) and the report carries no block bytes`;
  }
  const block = doc.macro_data ?? {};
  return `no macro records (${block.nonzero ?? 0} non-zero bytes in the ${block.len ?? 0}-byte block at offset ${block.offset ?? 'n/a'})`;
}

function renderMacrosPanel(file) {
  const doc = file.doc;
  const summary = doc.macro_data ?? {};
  const records = macroRecords(doc);
  const recordsBytes = records.reduce((sum, record) => sum + (record.record_bytes ?? 0), 0);
  const rows = [el('p', {
    class: 'hint',
    text: `payload offset ${summary.offset}, ${summary.len} bytes, ${summary.nonzero} non-zero. ${records.length} macro record(s) decoded from it, filling ${recordsBytes} bytes${recordsBytes ? `, ${summary.offset} to ${summary.offset + recordsBytes - 1}` : ''}. A record is an 8-byte header (0x77, mode, action count, flags, interval) plus its actions, every action 4 bytes: a key push 02 key 01 01, a key release 02 key 01 00, a mouse press 00 mask 00 01, a mouse release 00 mask 00 00, a wait 04 00 ms. The mask is a mouse-button bitmask (bit 0 button 1 through bit 7 button 8): the engine's own reader names a mouse action's button log2(mask) + 1 and rejects a byte that is not a single bit in 0x01..0x80, so a single-bit mask is the button GG can read back, and mask 0x03 (mouse 1+mouse 2) is a pair the reads carry that GG's reader refuses. The header word is the action count: every record of the reads surveyed carries its action count there. The chain is packed with no stride and no length word, and the bytes behind it belong to GG's own structures, so the walk stops where the grammar stops matching. A 0x71 slot's third code byte counts 4-byte units from the block start: the macro it plays starts at byte x 4 of the block, so a binding byte reaches ${MACRO_BINDING_REACH_BYTES} of its ${MACRO_DATA_LEN} bytes. This panel lists the records the block walks to; a write sends the block's bytes back, never this list.`,
  })];
  if (!Array.isArray(doc.macros)) {
    rows.push(el('p', { class: 'hint', text: 'this dump has no "macros" field - it was written before the record decode. Re-read the profile to get them.' }));
  } else if (records.length === 0) {
    rows.push(el('p', {
      class: 'hint',
      text: 'the block is empty: GG fills it only when the profile uses macros (the read with macros here carried 136 bytes of records).',
    }));
  }
  const blockBytes = macroBlockBytes(doc);
  rows.push(el('p', {
    class: 'hint mono',
    text: blockBytes
      ? `block bytes: ${blockBytes.length}, fnv1a ${summary.fnv1a_64}. The write sends these bytes back unchanged - the engine keeps this field as an opaque bytearray, so nothing here has to rebuild it.`
      : 'block bytes: this report carries none, so a write has nothing to send back verbatim. Re-read the profile with the current ssgg.',
  }));
  if (doc.macro_note) {
    rows.push(el('p', {
      class: 'hint',
      text: blockBytes
        ? `the record list stops early: ${doc.macro_note}. That is a limit of the list on this page, not of the profile: the bytes below go to the keyboard as the read recorded them.`
        : `the block did not parse cleanly: ${doc.macro_note}, and this report carries no block bytes, so the write path refuses it`,
    }));
  }
  for (const [position, record] of records.entries()) {
    const ordinal = macroRecordOrdinal(record);
    const bound = macroBoundKeys(doc, record);
    rows.push(el('h3', {
      class: 'subhead',
      text: `record #${ordinal} (binding byte ${macroBindingByte(record.offset - macroRegionStart(doc))}, payload ${record.offset}): `
        + `${record.mode_name ?? MACRO_MODE_NAMES[record.mode] ?? `mode 0x${hex2(record.mode)}`} (0x${hex2(record.mode)}), interval ${record.interval_ms} ms - ${macroKeystrokes(record)}`,
    }));
    rows.push(line('Bound by', el('span', {
      class: 'mono',
      text: bound.length ? bound.join(', ') : 'no key',
    }), `payload offset ${record.offset}, ${record.record_bytes ?? '?'} bytes, ${record.action_count ?? (record.actions ?? []).length} actions, header word ${record.header_word ?? 'n/a'}, flags 0x${hex2(record.flags >> 8)} 0x${hex2(record.flags & 0xff)}`));
    rows.push(el('p', {
      class: 'hint mono',
      text: (record.actions ?? [])
        .map((action) => (action.kind === 'press'
          ? `press ${keyName(action.key)}`
          : action.kind === 'release' ? `release ${keyName(action.key)}` : `delay ${action.ms} ms`))
        .join('  |  '),
    }));
    rows.push(el('p', { class: 'hint mono', text: record.raw }));
  }
  const editor = globalThis.SsggMacroEditor;
  if (editor) {
    const region = editor.regionOf(doc);
    const size = editor.plan(macroRecords(doc), region);
    rows.push(line('Region', el('span', {
      class: 'mono',
      text: `${region.start}..${region.start + region.len} (${region.len} bytes): ${size.used} used, ${size.free} free`,
    }), `the region is the board's whole space for macros, so its capacity is the limit - a list that needs more than ${region.len} bytes is refused with its numbers, and the macros past it live in GG's own database. "Macro editor" in the toolbar edits this list as key push, key release and wait instructions (macros.js): it writes the bytes the encoder writes and never a keyboard.`));
  }
  return panel('Macro records (packed list at the macro data offset)', rows);
}

function renderOledPanel(file) {
  const doc = file.doc;
  const oled = doc.oled;
  const bytes = oledBytes(doc);
  const url = bytes ? oledDataUrl(bytes) : null;
  const rows = [
    line('Block', el('span', {
      class: 'mono',
      text: `offset ${oled.offset}, ${oled.len} bytes, ${oled.nonzero} non-zero, fnv1a ${oled.fnv1a_64}`
        + (bytes && bytes.length && fnv1a64(bytes) !== oled.fnv1a_64 ? ` - the stored hash does not match the ${bytes.length} bytes in bytes_hex (those hash to ${fnv1a64(bytes)}); the picture is drawn from bytes_hex` : ''),
    }), `${OLED_W} x ${OLED_H} monochrome pixels = 8 px per byte, 16 bytes per row, ${OLED_BYTES} bytes; bit 7 of a byte is its left pixel (confirmed on an Apex 7, 0x1612: the image GG paints on its panel decodes to these bytes, and writing them back paints that picture), so there is no bit-order switch. The decoder writes the whole block as bytes_hex.`),
  ];
  const paint = state.oledPaint;
  // A stroke ends on the pointer leaving the stage or the button coming up: the full repaint then
  // carries the new bytes to the board's picture and to the summary row.
  const endStroke = () => {
    if (!paint.stroke) return;
    paint.stroke = false;
    paint.last = null;
    render();
  };
  const paintAt = (event) => {
    if (event.button !== undefined && event.button !== 0) return;
    const index = oledPixelAtEvent(event);
    if (index === null) return;
    const stroke = paint.last === null ? [index] : oledLine(paint.last, index);
    paint.last = index;
    const info = applyOledPaint(doc, stroke, paint.mode);
    if (editor) editor.setAttribute('src', oledDataUrl(info.bytes));
    if (ui.status) {
      ui.status.textContent = `OLED painted with the ${paint.mode} brush: ${stroke.length} pixel(s) in this stroke, ${info.changed} changed, ${info.lit}/${info.pixels} lit - save the JSON, then \`ssgg onboard write\``;
    }
  };
  let editor = null;
  if (url) {
    editor = el('img', {
      class: 'oled-preview oled-editor',
      src: url,
      width: String(OLED_W * OLED_PAINT_SCALE),
      height: String(OLED_H * OLED_PAINT_SCALE),
      alt: `on-board OLED bitmap, painted with the ${paint.mode} brush`,
    });
  }
  // The pointer target is the stage, not the picture: the paint works from the click coordinates
  // alone, so it keeps working on a page that cannot raster the preview (no 2D canvas) and the
  // brush buttons above never shift the coordinates.
  const stage = el('div', {
    class: 'oled-stage',
    onmousedown: (event) => { paint.stroke = true; paintAt(event); },
    onmousemove: (event) => { if (paint.stroke) paintAt(event); },
    onmouseup: endStroke,
    onmouseleave: endStroke,
  }, [editor ?? el('p', {
    class: 'hint',
    text: bytes
      ? 'this page cannot raster the bitmap (no 2D canvas), so the paint works on the blank stage: click where the pixel is, the picture below the board shows the block after a repaint.'
      : 'this dump has no oled.bytes_hex: it was written before the decoder started dumping the block. Run `ssgg onboard decode <image.bin>` on the saved image to get it.',
  })]);
  // Every OLED control lives in this one row, above the picture it edits: the two brushes, the
  // revert to the bytes the file was loaded with, the blank and invert actions, and the BMP picker.
  const clean = !oledBlockDiffers(file);
  const upload = el('label', { class: 'btn file oled-upload' }, [
    el('span', { text: 'Upload a BMP' }),
    el('input', {
      type: 'file',
      accept: '.bmp,image/bmp',
      onchange: (event) => {
        const picked = event.target.files?.[0];
        if (!picked) return;
        applyBmpFile(doc, picked)
          .then((info) => {
            if (ui.status) {
              ui.status.textContent = `OLED block replaced from ${picked.name}: ${info.source_width}x${info.source_height} ${info.bits} bpp ${info.compression}${info.top_down ? ' (top-down rows)' : ''} sampled to ${OLED_W}x${OLED_H}, ${info.lit}/${info.pixels} pixels lit at ${info.threshold}/255 - save the JSON, then \`ssgg onboard write\``;
            }
            render();
          })
          .catch((error) => { if (ui.status) ui.status.textContent = `BMP refused: ${error.message}`; });
      },
    }),
  ]);
  rows.push(line('Paint', el('div', { class: 'oled-box oled-paint' }, [
    el('div', { class: 'oled-tools' }, [
      button('White pixel', () => { paint.mode = 'white'; paint.stroke = false; paint.last = null; render(); }, `btn seg oled-tool${paint.mode === 'white' ? ' active' : ''}`),
      button('Black pixel', () => { paint.mode = 'black'; paint.stroke = false; paint.last = null; render(); }, `btn seg oled-tool${paint.mode === 'black' ? ' active' : ''}`),
      el('button', {
        class: 'btn oled-tool oled-revert',
        text: 'Revert the OLED block',
        ...(clean ? { disabled: 'true' } : {}),
        onclick: () => {
          const info = revertOledBlock(file);
          paint.stroke = false;
          paint.last = null;
          if (ui.status) {
            ui.status.textContent = info
              ? `OLED block reverted to the ${info.len} bytes the file was loaded with (${info.lit}/${OLED_W * OLED_H} pixels lit) - save the JSON, then \`ssgg onboard write\``
              : 'this file has no OLED block as loaded, so there is nothing to revert to';
          }
          render();
        },
      }),
      button('Set all black', () => {
        paint.stroke = false;
        paint.last = null;
        setOledBitmap(doc, new Array(OLED_BYTES).fill(0));
        render();
      }, 'btn oled-tool oled-clear'),
      button('Invert every pixel', () => {
        const src = bytes ? Array.from(bytes) : new Array(OLED_BYTES).fill(0);
        setOledBitmap(doc, src.map((byte) => byte ^ 0xff));
        render();
      }, 'btn oled-tool oled-invert'),
      upload,
      el('span', { class: 'hint', text: `brush: ${paint.mode}; click paints one pixel, drag paints the line` }),
    ]),
    stage,
  ]),
    `white lights a pixel and black turns it off: a click paints the pixel under the pointer (${OLED_W}x${OLED_H} pixels at ${OLED_PAINT_SCALE}x scale, so the click's own coordinates name it) and a drag paints every pixel between the points it reports, so a fast stroke comes out connected. Revert the OLED block puts back the bytes_hex the file was loaded with, and is disabled while the block already matches it - every control in this row is a page-local edit, so that button is the undo for all of them. Set all black writes ${OLED_BYTES} zero bytes. Invert every pixel flips every bit of bytes_hex: lit becomes off and off becomes lit, bit 7 stays the left pixel of its byte, so the picture is mirrored in colour only and never moved, inverting twice is the identity, and a block with no bytes_hex inverts to every pixel lit. Upload a BMP reads the file into ${OLED_BYTES} bytes: BI_RGB at 1, 4, 8, 24 and 32 bpp, BI_BITFIELDS at 32 bpp with the plain blue/green/red masks, bottom-up and top-down rows; BI_JPEG, BI_PNG, RLE, 16 bpp and any other mask layout are refused with what was found; the picture is sampled to ${OLED_W}x${OLED_H} by nearest neighbour (the rule for a picture stretched to fill a slot, which is how GG fills this panel from any image) and a pixel lights at ${OLED_LIT_THRESHOLD}/255 or above. Every control here rewrites bytes_hex and recomputes len, nonzero, fnv1a_64 and head from it. Only the JSON changes: this page opens configs read from an Apex 7 and writes files, never a keyboard - the write to the board is \`ssgg onboard write\`.`));
  rows.push(line('Bitmap data', el('span', { class: 'mono', text: `${OLED_W} x ${OLED_H} pixels, ${OLED_BYTES} bytes, 1 bit per pixel` }),
    `oled.bytes_hex (${OLED_BYTES} bytes) is the whole picture; the top-left pixel is byte 0 bit 7 (confirmed on hardware). The summary row shows the hash the dump records and the same FNV-1a-64 recomputed from these bytes.`));
  return panel(`On-board OLED (the board rect, ${OLED_W}x${OLED_H} 1bpp)`, rows);
}

// ---------------------------------------------------------------------------
// Render / init
// ---------------------------------------------------------------------------

function legendRows() {
  const items = [
    ['self', 'bound to itself - GG\'s factory default, not a remap (function 0x51 + its own code)'],
    ['none', 'empty slot - GG writes all zero, the key types normally'],
    ['deactivated', 'function 0x00 with other codes'],
    ['combo_key', 'function 0x51 combo_key'],
    ['meta', 'function 0x62 meta'],
    ['macro', 'function 0x71 macro'],
    ['media_key', 'function 0x61 media_key'],
    ['function', 'mouse / cpi / wheel function (0x01-0x34)'],
    ['no-offset', 'code has no mapping offset in the legacy HID table'],
    ['meta', 'a meta key (function 0x62): the cap prints its action - Profile, Macro, Bright up, Bright down, Lock Win, Rapid Tap - and the meta action select names code 1. META in a cap comes from meta_toggle_hid alone: the key that byte names is the key GG switches META mode with, painted this way on both layers, and it binds like any other key'],
  ];
  const box = el('div', { class: 'legend' });
  for (const [kind, text] of items) {
    box.appendChild(el('span', { class: 'legend-item' }, [
      el('span', { class: `swatch sw-${kind.replace(/[^a-z]/gi, '')}` }),
      el('span', { class: 'mono', text }),
    ]));
  }
  box.appendChild(el('span', { class: 'legend-item' }, [
    el('span', { class: 'swatch sw-changed' }),
    el('span', { class: 'mono', text: 'edited in this session (corner marked) - a key whose mapping entry differs from the file as loaded' }),
  ]));
  box.appendChild(el('span', { class: 'legend-item' }, [
    el('span', { class: 'swatch sw-selected' }),
    el('span', { class: 'mono', text: 'selected key' }),
  ]));
  box.appendChild(el('span', { class: 'legend-item' }, [
    el('span', { class: 'mono', text: 'text low in a cap = the binding target, and only of a remapped key (a factory key prints nothing); "[RT n]" = Rapid Tap pair n uses this key\'s HID code; the two caps with arrows in the Ke - column are the wheel mappings of the current layer, and click one to bind it in the Key panel like any key; offset, function and codes are in the cap tooltip' }),
  ]));
  return box;
}

// Where each panel goes: the column right of the board edits the current selection (a key, a wheel
// cap, or the OLED), the two blocks under the board edit what the caps show (Rapid Tap, lighting), and
// the Details section under the page carries the rest of the profile.
function panelPlan(file) {
  const changes = fileChanges(file);
  return {
    changes,
    inspector: [
      state.selected === SELECT_OLED ? renderOledPanel(file) : renderKeyPanel(file),
    ],
    // Directly under the keyboard, the two blocks that edit what the caps on the board show: the Rapid Tap
    // pairs (marked [RT n] on the caps) and the lighting colour the board paints with.
    primary: [
      renderRapidTapPanel(file),
      renderLightingPanel(file),
    ],
    // Everything that describes the file rather than the keys on it, under the Details heading: the read's
    // own header, the binding tables and the macro block.
    details: [
      renderHeader(file, changes),
      renderBindingsPanel(file),
      renderMacrosPanel(file),
    ],
  };
}

function render() {
  if (!ui) return;
  const file = activeFile();
  if (ui.macroBtn) ui.macroBtn.disabled = !file;
  renderTabs();
  ui.inspector.replaceChildren();
  ui.below.replaceChildren();
  ui.details.replaceChildren();
  if (ui.detailsBox) ui.detailsBox.hidden = true;
  if (!file) {
    ui.status.textContent = 'load one or more profile JSON dumps (schema a or b). The board binds keys through the legacy HID -> offset table.';
    ui.legend.replaceChildren();
    return;
  }
  const doc = file.doc;
  const plan = panelPlan(file);
  const changedHids = changeKeyHids(plan.changes);
  renderNameBar(file);
  paintBoard(ui.board, doc, state.layer, state.selected, changedHids);
  ui.inspector.replaceChildren(...plan.inspector);
  ui.below.replaceChildren(...plan.primary);
  ui.details.replaceChildren(...plan.details);
  if (ui.detailsBox) ui.detailsBox.hidden = false;
  ui.legend.replaceChildren(legendRows());
  if (ui.onRender) ui.onRender();
}

function loadFiles(list) {
  const run = async () => {
    const [file] = list;
    if (!file) return;
    const text = await file.text();
    if (ui.fileInput) ui.fileInput.value = '';
    const result = addFile(file.name, text);
    if (!result.ok) {
      ui.status.textContent = `${file.name}: ${result.error}`;
    } else {
      const doc = result.file.doc;
      const extra = list.length > 1 ? `, ${list.length - 1} more file(s) ignored - the editor loads one profile` : '';
      const device = deviceLabel(doc);
      ui.status.textContent = `loaded ${file.name} (${text.length} chars${device ? `, ${device.name} ${device.hex}` : ''}, slot ${doc.slot ?? 'n/a'}, ${payloadSplit(doc)}, schema `
        + `${doc.schema_name}, ${doc.guid_padding ? 'b: no padding field' : `a: checksum ${doc.crc_ok ? 'matches' : 'MISMATCH'}`}`
        + `, ${macroSummary(doc)}${extra})`;
    }
    render();
  };
  run().catch((err) => {
    if (ui.fileInput) ui.fileInput.value = '';
    ui.status.textContent = `loading failed: ${err.message}`;
  });
}

function init(controls) {
  ui = controls;
  ui.fileInput.addEventListener('change', (event) => {
    loadFiles([...event.target.files]);
  });
  ui.saveBtn.addEventListener('click', () => {
    const file = activeFile();
    if (!file) return;
    const name = saveFile(file);
    const changes = fileChanges(file).length;
    ui.status.textContent = `saved a copy as ${name} (${changes} change(s) against the file as loaded; the loaded file is untouched)`;
  });
  ui.layerBaseBtn.addEventListener('click', () => {
    state.layer = LAYER_BASE;
    render();
  });
  ui.layerMetaBtn.addEventListener('click', () => {
    state.layer = LAYER_META;
    render();
  });
  if (ui.macroBtn) ui.macroBtn.addEventListener('click', () => openMacroEditor());
  // A popup writes its edits through the server session; when this page regains focus, take a newer
  // revision than the one it posted.
  if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
    window.addEventListener('focus', () => {
      void syncMacroSession('on focus');
    });
  }
  const dropTarget = ui.boardBox;
  const onDrop = (event) => {
    event.preventDefault();
    document.body.classList.remove('dropping');
    if (event.dataTransfer?.files?.length) loadFiles([...event.dataTransfer.files]);
  };
  dropTarget.addEventListener('dragover', (event) => {
    event.preventDefault();
    document.body.classList.add('dropping');
  });
  dropTarget.addEventListener('dragleave', () => document.body.classList.remove('dropping'));
  dropTarget.addEventListener('drop', onDrop);
  document.addEventListener('dragover', (event) => event.preventDefault());
  // Dropping anywhere on the page loads; only the board shows the drop highlight.
  document.addEventListener('drop', (event) => {
    event.preventDefault();
    if (event.dataTransfer?.files?.length) loadFiles([...event.dataTransfer.files]);
  });
  render();
}

// Jump from the custom-binding list to the key panel: switch layer, select, repaint.
function selectKey(layer, hid) {
  state.layer = layer;
  state.selected = hid;
  render();
}

function handleKeyClick(hid) {
  const file = activeFile();
  if (!file) return;
  if (hid === SELECT_OLED) {
    state.selected = SELECT_OLED;
    const oled = file.doc.oled;
    ui.status.textContent = oled && oled.bytes_hex
      ? `on-board OLED selected - ${OLED_W}x${OLED_H} bitmap at payload offset ${oled.offset}, ${oled.nonzero}/${oled.len} bytes non-zero; the panel next to the board paints it pixel by pixel and can blank the block`
      : 'on-board OLED selected - this dump has no oled.bytes_hex, re-decode the saved image with ssgg onboard decode';
    render();
    return;
  }
  if (hid === WHEEL_UP || hid === WHEEL_DOWN) {
    const index = hid === WHEEL_UP ? 0 : 1;
    state.selected = hid;
    const entry = wheelEntry(file.doc, state.layer, index);
    ui.status.textContent = entry
      ? `wheel mapping ${index + 1} (${state.layer}) selected - function 0x${hex2(entry.entry.function)} `
        + `${functionName(entry.entry.function)} codes ${hexOf(entry.codes)} (${entry.label}); the Key panel binds it like any key`
      : `wheel mapping ${index + 1} (${state.layer}) is missing from this file: the wheel lists hold 2 entries each`;
    render();
    return;
  }
  state.selected = hid;
  render();
}

const SsggProfileApp = {
  MASK_MODE_REPORT_BOTH, HID2OFF, OFF2HID, KEY_NAMES, keyName, LAYOUT_LABELS, boardLabel, labelClass, subClass, fitSubText, ARROW_GLYPHS,
  WHEEL_UP, WHEEL_DOWN, WHEEL_KEYS, OLED_RECT, OLED_W, OLED_H, oledBytes, oledPixels, oledDataUrl, paintOled, renderOledPanel,
  MACRO_DATA_LEN, macroBlockBytes,
  wheelEntry, copyName, CODE_COUNT, CONSUMER_TARGETS, PRODUCT_NAMES, LEGACY_ONBOARD_PIDS, PRODUCTS, IMPORT_SOURCE, onboardModelPids, importedName, productKind, deviceFeatures, deviceLabel, productHex, MEDIA_NAMES, MOUSE_FUNCTIONS, MOUSE_NAMES, META_ACTION_NAMES, META_CAP_LABELS, BINDING_GROUPS, FACTORY_BASE_ZEROS, FACTORY_BASE_MEDIA, FACTORY_BASE_EXTRA, FACTORY_META, bindingGroup, factoryEntry, optionSelect, bindingGroupOptions, mouseActionOptions, keyboardKeyOptions, mediaKeyOptions, metaActionOptions, macroBindingOptions, macroSummary, bindingOf, bindingCounts, codesPerEntry, customBindings, normalizeDoc, normalizeEntry, selectKey, renderBindingsPanel, FUNCTION_NAMES, functionName, hex2, hexOf, parseHex, colorToHex, hexToColor, LAYOUT_ROWS, LAYOUT, LAYOUT_HIDS, EXTRA_HIDS, MAPPING_COUNT, LAYER_BASE, LAYER_META, EXPECTED_ARRAYS, isValidProfileDoc, layerArray, keyEntry, mappingCodes, writeCodes, setFunction, setCode, setAllCodes, keyState, rapidTap, setRapidPair, rapidTapKeysFor, lighting, setLightingColor, NAME_FIELD_BYTES, setProfileName, serializeDoc, cloneDoc, diffAgainst, changeKeyHids, SVG_NS, el, svg, state, activeFile, activeDoc, nextFileId, addFile, downloadText, saveFile, fileChanges, changedFiles, revertFile, codeOptions, CODE_OPTIONS, buildKeySelect, buildFunctionSelect, UNIT, GAP, boardKeyList, buildBoard, FILL_COLORS, paintBoard, ui, panel, line, numberInput, textInput, button, renderTabs, renderHeader, mappingRows, renderKeyPanel, renderBindingPanel, renderNameBar, panelPlan,
  MACRO_MARKER, MACRO_MODE_NAMES, MACRO_INDEX_UNIT, MACRO_BINDING_BYTE_MAX, MACRO_BINDING_REACH_BYTES, MACRO_MOUSE_BITS, macroRecords, macroIndex, macroRegionStart, macroBindingByte, macroOffsetForBindingByte, macroRecordOrdinal, macroRecordFor, macroMouseMaskName, macroKeystrokes, macroBoundKeys, macroLine, macroDetailRows, renderMacrosPanel,
  MACRO_CHANNEL, macroEditor, sessionToken, macroSessionPost, macroSessionGet, adoptMacroRecords, adoptProfileJson, syncMacroSession, mountInlineMacroEditor, openMacroEditor,
  META_TOGGLE_FUNCTION, META_LOCK_WIN_ACTION, computeMetaMask, metaMaskDiffers, META_KEY_HID, META_KEY_GG_CODE, META_KEY_SLOT, hidSlot, boardHid, metaToggleByte, toggleByteFor, isLockWinEntry, lockWinBinding, lockWinMarkers,
  PAYLOAD_LEN_A, PAYLOAD_LEN_B, PROFILE_REGION_BYTES, payloadLenForSchema, payloadSplit,
  FACTORY_WHEEL_BASE, FACTORY_WHEEL_META,
  fnv1a64, setOledBitmap, OLED_BYTES, SELECT_OLED, oledBytes, oledPixels, oledDataUrl, OLED_LIT_THRESHOLD, BMP_COMPRESSION_NAMES, decodeBmp, bmpColor, bmpToOledBytes, applyBmpBytes, applyBmpFile, OLED_PAINT_MODES, OLED_PAINT_SCALE, oledPixelIndex, oledPixelXY, oledLitPixels, oledLine, applyOledPixels, applyOledPaint, oledPixelAtEvent, revertOledBlock, oledBlockDiffers, renderLightingPanel, renderRapidTapPanel, legendRows, render, loadFiles, init, handleKeyClick,
};

globalThis.SsggProfileApp = SsggProfileApp;
})();
