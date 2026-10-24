# Apex 7 (wired, PID 0x1612): SteelSeries GG wire capture

Status: the on-board profile **read, write and deploy are confirmed on hardware** on an Apex 7 (`0x1612`)
under Linux (2026-10-20): every frame below that `ssgg` sends has been put on the wire and answered by that
keyboard, and the command statuses are listed in `.claude/rules/experimental-protocol.md`. The **payload
contents are understood**: the crate's codec decodes and encodes every block of the table below - header,
name, lighting fields, the META byte, all four mapping tables, Rapid Tap, the GUID and its padding, the tail
checksum - and `encode` is verified frame for frame against captured clean and modified profiles. The OLED
block is read as what it is: a 128x40 monochrome bitmap whose bytes **were read back from an Apex 7 (`0x1612`)
and written back to one**, and a write carries those bytes verbatim. The only part of a payload nobody
interprets is the *inside* of the 9,600-byte macro block: the engine declares it an opaque byte array and a
write sends those bytes back unchanged, and the macro-record grammar in this document is `[EXPERIMENTAL]` and is
used for reporting only. The GUI converts the OLED block between its bytes and a drawn picture (draw, clear,
invert, PNG export, an *Upload a BMP* control that turns a picture into those 640 bytes, a per-pixel paint
editor with white and black brushes, and a revert that puts the block back as the file was loaded with it). The same
protocol covers the
other Apex keyboards of this legacy platform - the engine's protocol file is written per platform, not per
model - and `discovery::keyboard_class` enables it on the one board that was measured, leaving the rest
untested.

Two USBPcap captures of official SteelSeries GG on the same keyboard, taken on **the same firmware** (the
`90 00` poll answers `4.16.8`): a **read capture** (GG running a profile read session) and a **write capture**
(2026-10-20, 677,636 bytes). Nothing differs between them except direction - every payload in both is schema
`B`. Frame counts quoted below are from those captures. The write frames are extracted by
`scripts/gg-write-frames.py`; its manifest (`tests/fixtures/onboard-write-apex7.manifest.txt`, 122 lines) is
in git history, not in the current tree.

## Transport

All profile traffic runs on the keyboard's vendor collections on USB interface 1 (`wIndex = 1`): usage page
`0xffc0` on interface 1, where every capture transfer sits, and usage page `0xffc1` on interface 4 — the
collection the Linux `hidraw` node for the profile protocol sits on. Report numbers are embedded in the
payload, so in `hidapi` terms byte 0 of every buffer is the report number, and it is `0x00` for every frame
of this protocol.

| Channel | `wValue` | Wire length | Use |
|---------|----------|-------------|-----|
| Output report, report 0 | `0x0200` | 64 | firmware poll, pull, payload push, file open, commit, deploy, driver mode |
| Feature write, report 0 | `0x0300` | 642 | payload read request (`83`), resource-buffer write (`03 00`) |
| Feature read (`GET_REPORT`) | `0x0300` | 642 | the answer to a payload read |
| Interrupt IN endpoint `0x82` | — | 65 read size | input reports: pacing packet after a pull, firmware answer, write answers |

Report-buffer sizes are the constants in `src/devices/hid_reports.rs`: `ONBOARD_OUTPUT_REPORT_SIZE` (65, i.e.
64 wire bytes), `ONBOARD_FEATURE_REPORT_SIZE` (643, 642 wire bytes), `ONBOARD_READ_WIRE_BYTES` (642) and
`ONBOARD_MAX_READ_LEN` (636, the ceiling a payload read may ask for). A 643-byte `GET_REPORT` ask never
completes: the capture's `wLength` is 642 and the read forwards it verbatim.

The interrupt endpoint streams the keyboard's ordinary input reports (report `0x01`, 63 bytes, all-zero on an
idle keyboard; median gap 14 ms = the key polling interval). GG reads that endpoint with a 65-byte read
(`GG_INPUT_READ_BYTES`: a report-ID slot plus 64 data bytes) and that same read serves the pacing packet after
a pull, the firmware poll's answer and the write answers.

Transfers run on the handle the device manager opened for the collection and keep open for the whole transfer:
`usbhid` polls a collection's interrupt endpoint only while a descriptor on that node is open, so the pacing
packets and answers arrive only on a handle that stays open (`apex_7.rs`, module header).

## Firmware poll (output report, `0x90`)

`90 00` asks the keyboard's firmware, `90 01` the LED controller's firmware. The answer lands 1-3 ms later on
the interrupt endpoint as `[report ID 0x6E][ASCII version]`, NUL padded (`34 2e 31 …` = `4.16.8`). GG polls
**both**, and they are the first two frames of its opening chain (below). `ssgg` sends `90 00` in front of the
schema probe for the firmware line of a read image; an unanswered poll is not an error
(`Apex7::firmware_version`).

## The session framing GG adds (and `ssgg` does not send)

GG opens a profile session with a **six-frame chain** of 64-byte output reports, 11 ms apart, and closes it with
a four-frame sequence:

| Order | Frame | Answer |
|-------|-------|--------|
| 1 | `90 00 00 …` | `[0x6E][ASCII]` - the **keyboard** firmware (above) |
| 2 | `90 01 00 …` | `[0x6E][ASCII]` - the **LED controller** firmware (above) |
| 3 | `f4 00 00 …` | `00 01 00 …` - the answer is `1`; **meaning unknown** (see below) |
| 4 | `6c 00 01 00 …` | none — **driver mode enable**: `DriverModeEnable` in the engine's init-sequence descriptor, frame in the read capture |
| 5 | `90 00 00 …` | `[0x6E][ASCII]` |
| 6 | `90 01 00 …` | `[0x6E][ASCII]` |
| - | `69 00 …` | closing sequence: OLED off |
| - | `89 <slot + 1> 00 …` | closing sequence, after a 250 ms sleep: deploy that slot |
| - | `41 00 00 …` | closing sequence: profile not volatile |
| - | `6c 00 00 …` | driver mode released at session close - **engine-side only, not in the capture** (GG was killed in it) |

Frame 4 is the engine's **driver mode enable**: the init-sequence descriptor names `6c 00 01` as
`DriverModeEnable`, the read capture carries that frame as 64 zero-padded bytes, and nothing answers it. The same
opcode closes the mode with `6c 00 00` at session close - engine-side, not in the capture.

GG drains one input report after every chain frame, and the answers come one at a time: a chain run that
drained only the first answer collected the `90 00` and `90 01` answers and nothing from frame 3 on, with all
six frames on the wire (Apex 7, 2026-10-18). Why an undrained answer silences the later frames is not measured;
it is the reason the pull loop drains one input report per pull.

**`ssgg` sends none of the chain.** A full slot read completes on hardware without it (measured 2026-10-19), so
the read changes nothing on the keyboard. `0xF4` is the one frame of the chain that answers and has no meaning:
the answer is `1`, nothing on the engine side names the command, and `ssgg` implements nothing for it - a
previous version of this code called it `EngineStatus`, which is a name this project invented, not a fact. An
earlier run that sent only the `90 00` poll got scrambled payload replies; the reading that the missing piece
was the **undrained answer** and not the frames is inference, not measurement.

The closing set matters as a group because `0x89` **writes** - the keyboard runs the slot it names - so a tool
that replays it changes the keyboard's active profile. `69 00` (Gamesense OLED off) and `41 00 00` (profile not
volatile) are named from the engine's own words; the capture has three `89` and three `41` - one pair behind
each profile file - and two `69`, while `6c 00 00` is engine-side only. `ssgg` sends none of them; its only
closing frame is `0x89`, on an explicit write (`apex_7::deploy_frame`, with the engine's 250 ms pause in front
of it).

Driver mode (`0x6C`, `OnboardDriverModeCommand`) is `[EXPERIMENTAL]`: the opening frame `6c 00 01` is
`DriverModeEnable` in the engine's init-sequence descriptor and is in the read capture, answered by nothing; the
closing frame `6c 00 00` is the engine's session-close write (engine-side, not in the capture). Nothing in `ssgg`
sends either, and the profile read works without them. The Apex 3 family splits: Apex 3 uses command `0x03` in a
33-byte report, Apex 3 TKL uses `0x6C` and opens with driver mode engaged.

## Profile read: pull (`0x85`) and payload read (`0x83`)

The read is two frames deep: a pull copies a slice of a profile **file** into the keyboard's 1024-byte
resource buffer, and payload reads drain that buffer.

```
pull (output, 64)          85 00 | file id u16 LE | file offset u32 LE | size u16 LE | 00 …
payload read (feature, 642) 83 00 | buffer offset u16 LE | size u16 LE | 00 …
```

- **Files are addressed by id.** Profile slot `n` is `0x0201 + n` (`PROFILE_FS_BASE`); GG's `Config 1` ..
  `Config 5` are slots `0..4`, which is what `ssgg onboard --profile N` takes: `N` is GG's config number, the
  slot behind it is `N - 1`, the file is `0x020N`. Behind that, GG addresses a second family of file ids,
  `0x0301 + n` (`LIGHTING_FS_BASE`), whose payload-read replies carry report ID `0x03`
  (`LIGHTING_READ_REPLY_REPORT_ID`): **presumed** to be the per-profile lighting config from the engine's
  naming, and **never confirmed on hardware** - no `0x030N` file has been read from or written to a keyboard,
  and nothing in `ssgg` addresses those ids. Every frame this project has run on hardware is on a `0x020N`
  profile file.
- **A pull has no answer of its own.** All 121 pulls of the capture are followed 1.7-3.7 ms later by an
  all-zero report `0x01` on the interrupt endpoint, and GG keeps no bytes of it: the interrupt packet is a
  pacing token, not a status frame.
- **The payload read answer carries no header**: `GET_REPORT wValue 0x0300 wIndex 1 wLength 642`, with the
  payload starting at byte 1 (the kernel lands the device's report-ID byte in buffer[0]). Some firmware echoes
  the request header (`03 01 01 00 01`) and puts the payload at byte 6; `parse_read_reply` accepts both, which
  is why a read frame never asks for more than 636 bytes.
- **GG probes a file before reading it**: one 2-byte pull of the file head plus one 2-byte payload read
  answers `03 | 0b 00`. The first two payload bytes are the schema word. The first 512-byte read of the same
  file answered `03 | 0b 00 43 6f 6e 66 69 67 20 31 …` — schema word at offset 0, profile name at offset 2.

Read geometry (a fact of the device, not a tunable — `apex_7_profile.rs`):

| Number | Value |
|--------|-------|
| Resource buffer per pull | 1024 bytes (`DEVICE_RESOURCE_BUFFER_LEN`) |
| Pulls per profile file | 12 (`PULLS_PER_FILE`), at file offsets 0, 1024, … 11264 |
| Payload reads per pull | 2 of 512 bytes, at buffer offsets 0 and 512 |
| File bytes one read covers | 12,288 (`PROFILE_FILE_BYTES`) |
| Pacing between our frames | 5 ms (`GG_GAP_MS`); GG's own opening-chain frames are 11 ms apart |

Capture counts: 120 pulls of 1024 bytes (100 of them on file `0x0201`) plus the 2-byte probe, 241 payload
reads, and **every** payload answer started with the first payload byte `0x0B`, i.e. schema `SCHEMA_B`.

In `ssgg`: `ssgg onboard read --profile N` (add `--bin` for the image bytes, `--report` for the per-pull read
record, `--verify` to read twice and compare, `--hex` to dump). About 2 s per slot. The read sends no closing
sequence and no driver-mode frame, so nothing on the keyboard changes.

## Profile payload layout

The payload is GG's own struct: `profile_all_settings_struct` (schema `0x000A`, 11,648 bytes) and
`profile_all_settings_schema_b_struct` (schema `0x000B`, 11,696 bytes) in the engine's protocol files, whose
byte comments give every offset below. Both captures are schema `B` (the Rapid Tap block added): the engine's
decoder lists firmware `4.16.3` as the switch to schema `B`
(`apex_7_profile::SCHEMA_B_FIRMWARE_VERSION`, documentation only), and every payload seen on the wire - 241
read answers and the write capture's 11,696-byte payloads - starts `0B 00`. The read never compares a firmware
version with anything, it takes the schema from the word it probes. The engine's structs carry one leading
constant id byte that is not part of the file, so the assembled image has no id byte. `decode` (in
`apex_7_profile.rs`) reads every block of this table into named JSON fields, and `encode` writes all of them
back and recomputes the tail checksum, so a report the read path produced round-trips byte for byte; the two
blocks declared as byte arrays are carried through as hex and parsed on the side for display.

Schema A is 11,648 bytes and schema B is 11,696; they share every offset below except the rows marked.

| Offset | Length | Content |
|----------------|--------|---------|
| 0 | 2 | schema word (`0x000A` / `0x000B`) |
| 2 | 24 | profile name (ASCII String, NUL-terminated), then one pad byte |
| 27 | 57 | lighting block (fields below) |
| 84 | 1 | META toggle key code (see below) |
| 85 | 640 | 128 button mappings (5 bytes each) |
| 725 | 10 | 2 wheel mappings |
| 735 | 640 | 128 button META mappings |
| 1375 | 10 | 2 wheel META mappings |
| 1385 | 640 | OLED block (opaque `bytearray`, carried byte-for-byte - see below) |
| 2025 (B only) | 1 | Rapid Tap enable byte |
| 2026 (B only) | 50 | ten 5-byte Rapid Tap pairs (`onboard_rapid_tap_pair`) |
| 2025 (A) / 2076 (B) | 9600 | macro block (the engine's `macro-data-size`, shipped as a `bytearray`) |
| 11625 (A) / 11676 (B) | 16 | profile GUID |
| 11641 (A only) | 3 | pad (schema B has none) |
| 11644 (A) / 11692 (B) | 4 | checksum, u32 LE (see below) |

**The OLED block (1385, 640 bytes) is encoded as-is.** `decode` copies it whole into `oled.bytes_hex` and
`encode` writes those bytes back untouched, so a write sends exactly the bytes a read produced; the engine ships
the field as a `bytearray` and parses nothing inside it. The conversion between those bytes and a picture exists
only in `tools/profile-gui`: it rasterises 128×40 monochrome pixels (8 px per byte, 16 bytes per row, bit 7 of
each byte the left pixel) for the drawn screen, the Clear / Invert controls and the PNG export, and writes a
bitmap back into the same JSON field: `decodeBmp` reads a BMP (`BI_RGB` at 1, 4, 8, 24 and 32 bpp, top-down rows,
32 bpp `BI_BITFIELDS` with the plain masks, RLE / embedded JPEG / embedded PNG / 16 bpp refused), samples it to
128×40 by nearest neighbour and lights a pixel at 128/255 or above. The panel also paints single pixels into
the same field with two brushes (white lights the bit, black turns it off), writing through the same
`setOledBitmap` path. **That geometry and bit order are confirmed on
an Apex 7 (`0x1612`)**: the operator took the image GG paints on the panel, decoded it into bytes, and read
those bytes back from the
keyboard as the block; writing them back paints that picture. Nothing in the Rust codec rasterises, and no
arbitrary uploaded picture has been put on a board - the bytes an upload produces travel the same verified
`encode` path as the bytes a read produced, and that is the claim, not the picture.

Lighting block (offsets inside payload):

| Offset | Length | Field |
|--------|--------|-------|
| 27 | 6 | OLED illumination options |
| 33 / 36 / 39 | 3 each | lock-key cluster colour, META-mapped cluster colour, disabled Win key colour |
| 42 | 32 | META mapping mask |
| 74 | 3 | lock-key HID codes |
| 77, 78, 79 | 1 each | Win key HID code, META-key background dimmer, idle-mode dimmer |
| 80 | 4 | idle timeout, ms (u32 LE) |

The META byte at 84 is GG's **code** for the key that switches META mode, not always that key's own usage:
`0xF0` is GG's code for the Meta cap (usage `0x65`, which has no slot under its own usage and binds through
slot 93), any other value names a key by its own usage, and `0x00` is no key. A Rapid Tap pair is
`hid_1`, `hid_2`, a mode byte whose high bit means "report both keys", and 2 reserved bytes.

Mapping tables use GG's `key_mapping`: a function byte plus **four** key codes (5 bytes per entry, not four).
Function selectors seen in GG's config: `0x00` deactivated, `0x01`-`0x08` mouse buttons, `0x30` CPI, `0x31`/`0x32`
mouse wheel, `0x33`/`0x34` pan, `0x51` combo key (GG's factory layer binds every key to itself: `0x51` + its own HID
code), `0x61` media key, `0x62` META, `0x71` macro; anything else is `unknown`. Mapping **slot** to key is GG's
`device-offset-lookup-table`, carried in `apex_7_profile.rs` as `OFFSET_TO_HID` / `HID_TO_OFFSET`; slots
`120..127` are not keys but the `0xF1..0xF8` function codes carrying the wheel and media bindings.

The macro block is carried through by decode and encode as one 9,600-byte byte array; the record split below is
the `[EXPERIMENTAL]` grammar this project derived from it, used when reporting a profile - a write never needs
it, it sends the block's own bytes back. Records start with marker `0x77`:

```
header  0x77 | mode u8 | action count u16 LE | carried u8 | repeat count u8 | interval u16 LE   (8 bytes)
press        0x02 | key u8 | page u8 | 0x01                                (4 bytes)
release      0x02 | key u8 | page u8 | 0x00                                (4 bytes)
press mouse  0x00 | mask u8 | 0x00 0x01                                   (4 bytes)
release mouse 0x00 | mask u8 | 0x00 0x00                                  (4 bytes)
delay        0x04 | 0x00 | ms u16 LE                                      (4 bytes)
```

(`header word` is the codec's name for the action count and `flags` its name for the carried byte and the repeat
count together; the names are the codec's, the fields are GG's - see
[Macro playback modes](#macro-playback-modes-engine-binary-2026-10-28). The keyboard page byte is 1 for keyboard
usages and `0xc` for consumer/media usages; the codec's grammar takes 1 only.)

Modes seen: `1`, `2`, `3`, `4`, `5` - GG's playback types `0..4` plus one. Modes `2`, `3` and `4` are the ones GG
gives the repeat pair (interval + count): the number-row board below carries interval `25 ms` on mode `3`, `30 ms`
on mode `4` and a count of `69` on mode `2`, and a mode-4 record with interval `5000` presses `4`, which is the
META + Insert binding of profile 1. Modes `1` and `5` carry the defaults (`1`, `15 ms`) and no repeat pair. The
codec names `1` "once" and `4` "repeat" and reports every other mode as `unknown` (`macro_mode_name`); the UI label
of each mode is open.

The mouse actions are what a number-row macro is built from: on a wired `0x1612` read (firmware 4.16.8, schema b,
2026-10-22) whose number row `3` to `7` is bound to macros, those records press and release mouse masks `0x03`
(buttons 1+2), `0x08` (button 4), `0x10` (button 5), `0x20` (button 6) and `0x07` (buttons 1+2+3), each pair
wrapped in a delay (`[EXPERIMENTAL]`: measured 2026-10-28). The mask is a bit set, not a button number: bit `0x01`
is mouse 1 through bit `0x80` mouse 8, and one action can set several bits at once.

The engine binary states the rule. Its macro converter writes a mouse action as `1 << (button - 1)` for a button
in `1..=8` (`libsteelTranslateMacroToOnboard`, `SteelSeriesEngine.exe` `0x1404e05e0`), and its reader names the
button `log2(mask) + 1`, rejecting the byte when that is not a whole number in `1..=8` - `invalid mouse code %d`
(`convert.libsteelGetMacroDataFromOnboard`, `0x1404e25a0`). So a single-bit mask is the form GG can round-trip,
`0x04` is mouse 3 and `0x40` is mouse 7.

The board above shows what GG writes for the same keys in GG's own UI: the operator built these five macros as
mouse 3, 4, 5, 6, 7, and the bytes for keys 4, 5, 6 are the single-bit masks (`0x08`, `0x10`, `0x20`), while keys
3 and 7 carry `0x03` and `0x07` - the button *number* written raw, which GG's own reader rejects. `[EXPERIMENTAL]`
(the two readings disagree on those two keys; the byte is what the board holds, and only the single-bit form is
what GG's converter emits). `ssgg` names the bits, and prints the byte beside them
(`push mouse 1+mouse 2 (mask 0x03)`), so a mask GG's reader rejects is visible as one. A write carries a mask of
`0x01..=0x80` (`MACRO_MOUSE_MASK_MAX`), the engine's range.

A `0x71` mapping row carries the **binding byte** in its third code byte: **the macro record's offset in the
block, in 4-byte units** - `index = offset / 4`, `offset = 4 x index` (`MACRO_INDEX_UNIT`).
`[EXPERIMENTAL]`: measured 2026-10-28 on that same read: its 14 macro bindings carry the bytes
`0, 7, 14, 21, 25, 28, 33, 38, 43, 46, 63, 92, 96, 117`, and **all 14 land on a record the block holds** -
`25, 28, 33, 38, 43` land on the five mouse records at offsets `100, 112, 132, 152, 172`, whose masks are mouse
1+2, 4, 5, 6 and 1+2+3, which is what number-row `3` to `7` do on that board. The older reading - `7 x record
position` - names the right records only for the four bindings whose records happen to be 28 bytes apart
(`0, 7, 14, 21`) and the wrong ones for the other ten: byte 25 becomes the record at position 3, byte 28 the record
at position 4, and `5`, `6`, `7` resolve to records that press the mouse buttons of `3` and `4`. It is superseded.
One code byte reaches `255`, so a binding names records up to `4 x 255 = 1020` bytes into the block
(`MACRO_BINDING_REACH_BYTES`); the block's capacity is larger than that reach, and a record packed past it is
written and read back with no key able to play it. The u16 at record `+2` is **the record's action count**, not a
macro id: GG's own writer stores the number of actions it packed there, and it equals the number of actions in all
14 records of that read (the longest, at 2544, carries 83 and lists 83). The codec carries it as `header_word`
data and validates only that it is non-zero. The two bytes at `+4` and `+5` are `01 01` in 13 of the 14 records and
`00 01` in the one whose only action is a press with no release; `+5` is the repeat count and `+4` is a carried GG
field - see [Macro playback modes](#macro-playback-modes-engine-binary-2026-10-28) (`MACRO_RECORD_FLAGS_KNOWN`
lists the pairs the codec has seen).

**Checksum**: the little-endian u32 at the payload tail is the STM32 hardware CRC-32 (poly `0x04C11DB7`, init
`0xFFFFFFFF`, no input/output reflection, no final XOR) over `payload[0..len-4]` read as little-endian 32-bit
words — the same rule as the 2023-family profile blob, and the same function
(`apex_pro_tkl_2023_profile::profile_checksum`, re-exported as `apex_7_profile::compute_checksum`). It verifies
on a factory read: slot 0, schema B → `0xAE57B9DC` over `payload[0..11692]`. `decode`
reports stored and computed words; `encode` recomputes and ignores the stored word.
The 64-bit FNV-1a digests in the read report are identity hashes for comparing reads
(`apex_7_profile::fnv1a_64`), not protocol checksums.

## Macro decode: partial, display-only

`decode` walks the macro block into a record list (`apex_7_profile::decode_macros`), and `tools/profile-gui`
displays it: the Macro panel lists every record the walk produced (mode, header word, interval, flags, its
keystrokes and mouse buttons, the keys bound to it, its raw bytes), a key bound with `0x71` shows the same record
in its own panel, and the binding select offers the decoded records as bind targets. The walk reads the whole
block: every record whose bytes the region holds, not a fixed count. A read of the board above lists all 14
records, and the five mouse records show as `push mouse 1+mouse 2 (mask 0x03)`, `push mouse 4 (mask 0x08)`,
`push mouse 5 (mask 0x10)`, `push mouse 6 (mask 0x20)`, `push mouse 1+mouse 2+mouse 3 (mask 0x07)`.

What is **not** supported, and the limits that go with it:

- **Editing is a GUI feature, and the write path refuses its result.** The macro editor (the popup window of
  `ssgg gui`, `tools/profile-gui/macros.js`) composes, inserts, reorders and trims records and hands the list to
  the open profile. Nothing in `ssgg` writes a macro: `encode` refuses a record list that does not match the
  block's own bytes - the block between the records is GG's own per-macro structures this grammar cannot rebuild
  - and it refuses a list past the block's capacity with the byte counts. The editor shows both refusals as the
  codec states them.
- **The record list is never encoded.** A write takes `macro_data.bytes_hex`, the block as the read recorded it,
  and sends it verbatim; the decoded list in `macros` is not serialised, so a list that disagrees with the bytes
  has no effect on what the keyboard gets.
- **The walk is best-effort.** The list has no length word and no stride, so the walker reads the block to its
  end, takes the records whose bytes parse, and reports what it skipped in `macro_note`: the count of non-zero
  bytes the list does not cover and the headers that carry no action. `ssgg` has seen real GG blocks whose
  padding bytes its grammar rejects. A report carrying such a note and no `bytes_hex` is refused by the write
  path (`OnboardWriteError::MacroRecordNote`).
- **A binding cannot name every record the block holds.** A byte reaches 1020 bytes into the block; records
  packed deeper than that are decoded, shown in the editor, written back, and unreachable by any key. The editor
  says so on each record past the reach.
- **Two record fields are now GG-verified, two are not.** The mode byte, the u16 at `+2` (the action count, which
  is what the engine's writer stores there - `header_word` is that count, not an id, in all 14 records of the read
  above) and the repeat count and interval are read out of the engine binary; see
  [Macro playback modes](#macro-playback-modes-engine-binary-2026-10-28). Unconfirmed: the carried byte at `+4`
  (`flags` is `01 01` in 13 of the 14 records, `00 01` in the one whose only action is a press with no release) and
  the UI name of each mode. The binding rule is `[EXPERIMENTAL]`: `offset / 4`, measured on the read named above,
  and a binding byte no record starts at - `0x00` for an unbound key, and the deleted macro GG leaves behind -
  decodes as no macro.
- **Macro names are not in the payload.** The name and the macro as GG displays it live in GG's own database
  (`db\database.db`); the payload carries packed records only, which is why the panel labels a record by ordinal,
  mode and action count and not by name.

## Macro playback modes (engine binary, 2026-10-28)

Read out of `SteelSeriesEngine.exe` (current GG, Windows) in Ghidra on 2026-10-28. The functions that move a
macro between GG's own representation and the board's packed block, both directions:

| Function | Address | What it does |
|----------|---------|--------------|
| `pkg/models/macro/convert.libsteelTranslateMacroToOnboard` (`convert_libsteel.go:108`) | `0x1404e05e0` | GG macro + playback options -> board record header and actions |
| `pkg/models/macro/convert.libsteelMacroMetadataModel` | `0x1404e1620` | tail-calls the writer above |
| `pkg/models/macro/convert.LibsteelConvertToOnboardMacros` | `0x1404dffc0` | GG bindings -> board: calls the writer per bound key |
| `pkg/models/macro/convert.libsteelTranslateOnboardToMacro` | `0x1404e21c0` | board record -> GG binding |
| `pkg/models/macro/convert.libsteelGetMacroDataFromOnboard` | `0x1404e25a0` | board actions -> GG device events |
| `pkg/device.adjustDeviceEventsForPlaybackOptionsData` (`macro.go:224`) | `0x1404c8120` | playback options -> events for OS-side (non-onboard) macros |
| `pkg/device.unmarshalPlaybackOptions` | `0x1404c8040` | JSON -> the playback options struct |

**The mode byte is GG's playback type plus one, and it is part of the magic word.** The writer builds the header's
first word as `0x77 | (type + 1) << 8` - so the block's `77 01`, `77 02`, `77 03`, `77 04`, `77 05` are GG types
`0, 1, 2, 3, 4`. A type outside `0..=4` is refused with `unknown playback type: %d`; the reader recovers the type
as `mode - 1` and refuses a mode outside `1..=5` (`unknown onboard playback type: %d`). So the wire mode space is
exactly `1..=5`: GG writes nothing else and reads nothing else. The reader's magic test is the 24-bit word
`0x007700`, i.e. the bytes `77 01 00`.

**Header fields** (the writer's own store sequence, 8 bytes, and the reader's uses of them):

| Byte | Field | Engine evidence |
|------|-------|-----------------|
| `+0` | `0x77` | low byte of the header word above |
| `+1` | playback mode `1..=5` | `type + 1` on write, `mode - 1` on read, both range-checked |
| `+2..+3` | action count, u16 LE | writer stores the number of actions it packed; the reader loops that many 4-byte actions |
| `+4` | GG playback-options byte, carried | the writer copies the struct byte that sits just before the repeat count and never interprets it; the reader hands it back unchanged. `1` in 13 of the 14 records of the read above, `0` in the one whose only action is a press with no release. `[EXPERIMENTAL]`: carried, not explained |
| `+5` | repeat count | the writer's playback-options count byte, the same value `adjustDeviceEventsForPlaybackOptionsData` appends as an event for types `1, 2, 3` |
| `+6..+7` | repeat interval, ms, u16 LE | the writer's playback-options `u16`; the same value the same function appends for types `1, 2, 3` |

The writer checks `offset + 8 + 4*count` against the block capacity before it stores the header and logs an error
when the record would not fit: a record is `8 + 4*count` bytes, and the block is `macro-data-size` = 9600.

**Which modes use the repeat pair.** `adjustDeviceEventsForPlaybackOptionsData` switches on the playback type:
for types `1`, `2`, `3` (wire modes `2`, `3`, `4`) it appends two events - device-event function `4` carrying the
interval in ms and function `5` carrying the count - and for types `0` and `4` (wire modes `1`, `5`) it returns the
events unchanged. A type outside `0..=4` logs `Playback options data is of an unknown type. Returning default
events but you should probably add a case for this.` The read above matches: mode `2` carries count `69`, modes
`3` and `4` carry intervals `25 ms` and `30 ms` with count `1`, and modes `1` and `5` carry the defaults `1` and
`15 ms`.

**Actions are GG's device events packed to 4 bytes.** The writer takes GG's event tuple - function, usage page,
code, data - and the function selects the action kind; the reader's switch undoes the same mapping. A function the
writer has no branch for is refused: `onboard mapping does not support event type %d`.

| Kind | GG function | Bytes `+1..+3` |
|------|-----------|----------------|
| `0` | mouse button | `+1` = one-bit mask, `0x01`..`0x80` = mouse 1..8: the writer emits `1 << (code - 1)` and refuses a code outside `1..=8` (`unsupported mouse button press %d direction %d`), the reader recovers `log2(mask) + 1` and rejects any other byte (`invalid mouse code %d`); `+2` = 0; `+3` = 1 press, 0 release |
| `1` | wheel | `+1` = direction byte (1 up, `0xFF` down, GG's data value); `+2` = 0; `+3` = 1 press, 0 release |
| `2` | keyboard | `+1` = HID usage (`0xE0`..`0xE7` are the modifier keys themselves); `+2` = usage page, 1 = keyboard and `0xc` = consumer/media, the writer's own rewrite of page 7 to 1 and its check `page in {1, 0xc}`; `+3` = 1 press, 0 release |
| `4` | delay | u16 LE at `+1` = milliseconds; a delay over 65535 ms is split into several 65535 ms actions by the writer |
| `7` | mouse press+release pair | `+1` = button number, `+2` = 0, `+3` = 1. The writer's branch leaves byte `+0` **uninitialised** in the frame Ghidra shows, so the kind byte of such an action is whatever the allocation held: `[EXPERIMENTAL]`, no such action appears in the read above |
| `8` | keyboard press+release pair | written as kind `2` with `+1` = usage, `+2` = page, `+3` = 1 (press only, the release is implicit) |
| `0xc` | horizontal wheel | `+1` = direction, `+2` = 0; in the writer's switch, not present in the read above |

`libsteelGetMacroDataFromOnboard` walks the block it is handed as a **flat 4-byte action stream** - it never skips
a record header - so the header grammar above is what the translate path uses, and a block handed to that converter
raw is parsed as actions from byte 0.

**What is not settled: the UI names of the five modes.** The engine binary carries no name for the enum - the
labels live in the GG front end - so the table below is filled only as far as the binary and the read go. The read
above is the operator's own board, whose five mouse macros were each made in GG with a different play mode, so the
five labels come from that session, not from a new capture:

| Wire mode | GG type | Repeat fields in the read | UI label |
|-----------|---------|---------------------------|----------|
| 1 | 0 | defaults only | *to fill* |
| 2 | 1 | count 69, interval 15 ms | *to fill* |
| 3 | 2 | count 1, interval 25 ms | *to fill* |
| 4 | 3 | count 1, interval 30 ms | *to fill* |
| 5 | 4 | defaults only | *to fill* |

The one structural hint: mode `5` is the mode GG's own event builder does not handle, i.e. a mode only the board
implements - `play while the key is held` is the standing hypothesis, unconfirmed. The check that settles it is
behavioural, not byte-level: bind a key to a mode-5 macro and see whether it stops on release.

**Where GG gets the labels: not from anything readable offline.** Searched the string tables of
`SteelSeriesEngine.exe`, `SSEdevice.dll` and the Apex 7 `4.16.8` firmware image on 2026-10-28: none carries a
play-mode label (`Play Once`, `Loop`, `Toggle`, `while pressed`, `Repeat` match only log text such as
`unknown playback type: %d`, Go runtime names, and the firmware's OLED strings `MACROS`, `REMOVE MACRO`,
`NO MACRO FOUND`). The engine moves a number only: `device.unmarshalPlaybackOptions` (`pkg/device/macro.go:216`)
unmarshals `{"playbackOptions": <the binding's stored blob>}` and returns `.PlaybackOptions.Type`, the column
`bindings.playback_options_data`.

The engine's localization service is a mirror of the front end's strings, not their owner
(`pkg/utils/localization/localization.go:180-274`): `Initialize` registers for `EVENT_APP_SETTING_CHANGED`, and
`handleEventAppSettingChanged` re-runs `CacheLocalization` when the changed setting key is `locale`;
`getLocalizationData` calls `GetLocalizedText("locale")`, marshals the returned text and unmarshals it into the
fixed Go struct `LocaleJsonStruct`; `GetLocalizedText("a.b.c")` splits on `.`, Title-cases each part and walks that
struct by field name; `GetLocale` decodes the `settings` app setting and falls back to `en_US`. The setting is
written by the front end through the engine's `setApplicationSetting` handler
(`pkg/frontend/application_settings_handlers.go:8` -> `config.GetAppSettingStringSetterByKey`, table
`application_settings`). The label table is data the GG app pushes into the engine.

Cheapest check that can fill the label column - no hardware, no new capture: read the `locale` row of
`application_settings` in `C:\ProgramData\SteelSeries\GG\apps\engine\db\database.db` with GG stopped. If the front
end sends the macro UI's strings, the five labels are in that JSON.

**Where the codec is looser than these facts** (recorded 2026-10-28, not changed - the codec is deliberately
carrying the block through, not validating it as GG does): `MACRO_MODE_MAX` is `8` where GG accepts `1..=5`;
`macro_mode_name` names mode `1` "once" and mode `4` "repeat" and calls `2`, `3`, `5` unknown, where GG's rule is
that modes `2, 3, 4` carry the repeat pair and modes `1` and `5` carry none; and the decode walks a record's
actions by its grammar and ignores the header's action count, which agrees with GG in all 14 records of the read
above.

One more binding-side fact from the same read: GG writes a **second** binding for the mouse 4 / mouse 8 pair - the
binding writer swaps the `0xE3` and `0xE7` codes when a flag is set, and the read's mapping table shows the pair
(two rows naming one record). `[EXPERIMENTAL]`: from the writer's source, matched against the read's rows.

## Profile write: open (`0x88`), buffer write (`03 00`), push (`0x05`), commit (`0xEB`)

The engine's write API fills the whole **profile region**, which is larger than any schema's payload:
`profile_size` = `0xA000` = 40,960 bytes (`PROFILE_REGION_BYTES`). Frames (`OnboardWriteFrame`, plan built by
`write_plan`, 122 frames for one slot):

| Frame | Report | Layout | Answer |
|-------|--------|--------|----------|
| open | output 64 | `88 00 \| file id u16 LE` | `88 01 …`, 305 ms after the frame |
| buffer write ×2 | feature 642 | `03 00 \| buffer offset u16 LE \| size u16 LE \| data` (offsets 0 and 512, 512 bytes each, data behind the 6-byte header) | none |
| push | output 64 | `05 00 \| file id u16 LE \| file offset u32 LE \| size u16 LE` (1024 = the whole buffer) | `05 01 …`, 5 ms after the frame |
| commit | output 64 | `eb \| 1-based slot` | none |

Pattern: one open, then **40 windows** of the region — two buffer writes (offsets 0 and 512) and one push of
the full 1024-byte buffer per window at file offsets 0, 1024, … 39936 — then the commit. Frames go out 5 ms
apart. A write pads the payload out to the region with zeros, so the frames past the payload end (the payload
is 11,696 bytes at most) push zeros: the file's own size is never read from the board.

- Every answer a write gets carries status byte `0x01` in the capture, in 202 answers out of 202. `ssgg`
  treats any other value, and any frame the capture shows answered going unanswered, as a refusal and stops —
  the frames after a refused one build on it. What `0x01` means beyond "accepted" is observed, not proven.
- After the commit `ssgg` pauses 250 ms (`DEPLOY_SETTLE_MS`) and sends the deploy frame `89 <slot + 1>`.
  Measured on hardware: the deploy sent straight after the commit **can do nothing**; waiting **before** it is
  what makes the keyboard switch slots, and how much the device needs is not measured. 250 ms is the engine's
  own number — its `load-profile-with-pause` sleeps 250 ms *behind* the deploy frame, and its commit path ends
  with an input read that blocks until the keyboard answers.
- The write capture (2026-10-20) reproduces this geometry frame for frame; its write runs contain 72 pushes,
  every one answered, and every push is followed by an all-zero pacing packet, exactly as the pulls are.

In `ssgg`: `ssgg onboard write --profile N <profile.json>` (or `--bin <image.bin>`) prints the family's
geometry banner and the 122-frame plan, then runs it on the keyboard's own handle — `--dry-run` prints the same
plan with no keyboard opened and nothing sent. `ssgg onboard load --profile N` sends the deploy frame alone.
The payload can come from a JSON (`onboard decode` report → `encode_json`) or from bytes an earlier
`onboard read --bin` wrote.

**Verification**: an on-board write has no read-back frame. A write is checked from the device by reading the
slot back with the read path and diffing the payload (`apex_7_profile::image_diff` / `payload_diff`), and
`ssgg` prints "read the slot back before trusting it" whenever the frames it sent do not match its plan.

## Where GG's own words are

- **GG's device specifications ship encrypted** (`apps\engine\deviceSpecifications\*.edevice`). The `.lisp`
  files cited below - `apex_7.lisp`, `base_apex_legacy.lisp` - are the **decrypted** contents of those
  `*.edevice` files from the SteelSeries Engine distributable: plain lisp, one file per keyboard family,
  giving its protocol, tables, sizes and init sequence. Everything this document attributes to "the engine's
  own words" is that plaintext.
- The engine's protocol files `apex_7.lisp` and `base_apex_legacy.lisp` (GG's device protocol definitions for
  the legacy platform): the payload structs and their byte comments, `key_mapping`, `onboard_rapid_tap_pair`,
  `device-offset-lookup-table`, `profile_size` = `0xA000`, `macro-data-size` = 9600, the profile slot range
  `(range 1 5)`, and `load-profile-with-pause`. The **init-sequence descriptor** in those files is what names
  the chain's frames, `DriverModeEnable` among them.
  The code cites these files by line next to the constants they give (`apex_7_profile.rs`).
- GG's stored per-device config as JSON: `C:\ProgramData\SteelSeries\GG\apps\engine\db\database.db`, table
  `configurations`, column `settings`. Open it read-only with GG stopped.
- Reference profiles live in `tools/profile-gui/examples/`: `sample-identity.{bin,json}` (decode of a real
  stock slot — name `Config 3`, macro block all zero, 11,696 bytes, trailing checksum word equal to the CRC the
  crate computes) and `sample-remap.{bin,json}` (that decode with base remaps written in, the meta colour
  edited, name `Remapped`).

## Apex 7 firmware package 4.16.8 (read from the Ghidra project copy, 2026-10-23)

`firmware-apex-7-v4.16.8.bin` (the update payload GG ships; loaded in Ghidra as `ARM:LE:32:Cortex`,
base 0, 1 MiB) is a **container**, not a flat image. A header of `(id, offset, size)` triples sits at
offset 0, then the blocks:

| id | offset | size | what it is |
|----|--------|------|------------|
| `0x2` | `0x1000` | `0x1d000` | application image - vector table at `0x1000`, initial SP `0x2000c000` |
| `0x3` | `0x1e000` | `0x1d000` | second copy of the application image |
| `0x201`-`0x205` | `0x3b000 + 0x8000*n` | `0xa000` each | the five `0x020N` profile slots, **all zero in the shipped package** |
| `0x301`-`0x305` | `0x6d000 + 0x5000*n` | `0x5000` each | the five `0x030N` files GG names, **all zero in the shipped package** |
| `0x9` | `0x86000` | `0x800` | zero |
| `0xa` | `0x87000` | `0x400` | a table of `id 00 b c` triples (key-matrix data, not illumination) |

Two open questions get shorter: the slot region is `0xa000` = 40,960 bytes, which is the length a write
pushes (question 1), and the `0x030N` files GG names are a real `0x5000`-byte block per slot in the same
flash family as the profiles (question 5). Neither carries content in the shipped package, so nothing can
be decoded from them.

**The board's own menu vocabulary.** The application image carries a 92-entry string pool at
`0x1599b..0x15d61` - the text the keyboard draws on its 128x40 OLED - with the font's glyph table at
`0x157c8` (`(u16 offset, u16 width)` pairs) and glyph bitmaps at `0x15700`. No 32-bit word anywhere in
the package points into the pool, so the menu code reaches it PC-relative. The pool is the menu tree,
read from the end backwards:

- top level: `ILLUMINATION`, `MACROS`, `ACTUATION`, `DISPLAY`, `ABOUT`, `VERSIONS`, `FACTORY RESET`;
- `ILLUMINATION` → `PRESETS`, `CUSTOM`, `BRIGHTNESS`, and the effect and state names `FAZE`, `PRISM`,
  `FULL COLOR`, `BREATHE COLOR`, `BREATHE MULTI`, `RIPPLE`, `LINE`, `FADE`, `LIGHTS OFF`, `OVERWRITE`,
  `LIST`, `TOGGLE`, `REACTIVE`, `BASE`, `HAZE`, `COAST`, `VAPORS`, `DIMMED`, `DIM`, `ON`, `OFF`, `AFK`,
  plus the colour names `RED`, `GREEN`, `BLUE`, `WHITE`, `YELLOW`, `PURPLE`, `ORANGE`;
- `DISPLAY` → `DISPLAY ICONS`, `TIMEOUT`, `BRIGHTNESS`;
- the timeout ladder as the pool lists it: `5 SECS`, `10 SECS`, `30 SECS`, `1 MIN`, `5 MINS`, `10 MINS`,
  `15 MINS`, `30 MINS`, `45 MINS`, `1 HOUR`, `2 HOURS`, `3 HOURS`, `5 HOURS`, `NONE`, `NEVER`;
- config menu: `DE/ACTIVATE CONFIGS`, `CONFIG KEY ACTION`, `SAVE CONFIG`, `LOAD CONFIG`, `RAPID TAP`,
  `RECORD`, `DELETE`;
- macro and calibration messages: `RECORDING`, `FINISH:`, `CANCEL:`, `REMOVE MACRO`, `PRESS KEY TO`,
  `ASSIGNED`, `TO ASSIGN`, `PRESS KEY`, `NO MACRO FOUND`, `OUT OF MEMORY`, `TOO MANY EVENTS`, `ERROR`,
  `UPDATED`, `DELETED`, `SUCCESS`, `CALIBRATION DONE`, `RECALIBRATING`, `PRESS ALL KEYS`, `PRESS: + I + O`,
  `UPDATING` (the pool spells it `Updating`), `SteelSeries Apex 7 Recovery`, `NO`, `YES`.

Against the lighting block table above, the board's options line up with the fields at 78, 79 and 80:
`idle_timeout_ms` takes the ladder (`0` = never, then 5 000, 10 000, 30 000, 60 000, 300 000, 600 000,
900 000, 1 800 000, 2 700 000, 3 600 000, 7 200 000, 10 800 000, 18 000 000 ms), and the two dimmer
bytes carry the `DIM`/`BRIGHTNESS` levels. All three captures in this repo read `255, 255, 0` there.
**The six bytes at 27 stay unnamed, and they are not GG's UI's.** The live board's slot has
`02 00 00 00 00 00`; the stock slot and the synthetic remap both have `00 02 00 00 00 00`. Nothing in
`SteelSeriesEngine.exe` names them: the engine binary's only `oled` strings are the GameSense OLED path
(`shihan_monitoring`, the Apex Pro TKL frame builder) and its only `illumination` strings are the live-colour
commands (`write_all_illumination`, `reset_all_illumination`); `brightness`, `dimmer` and `icons` do not occur
at all. The names are GG's lisp spec's, which ships encrypted. GG's Apex 7 front end offers **one** OLED control
- upload an image to the panel, the bytes this document's OLED section carries - and no OLED brightness, sleep
or icon choice (operator-reported 2026-10-23). Those are the keyboard's own menu's settings, and the profile
carries them: the board's `DISPLAY` menu offers `BRIGHTNESS`, `DISPLAY ICONS` and `TIMEOUT`, and setting a
brightness on the board is enough to move a byte in the slot. That makes the read-out cheap and read-only -
change one setting on the board, read the slot, diff it. Tracked in `PLAN.md`.

## Open questions

1. A read collects 12,288 file bytes, however a write pushes 40,960. According to device specs, these should not
   be read, so the purpose of writes is not clear. The firmware package shows the slot region is `0xa000`
   (40,960) per slot, so the write length is the whole region; the read path's 12,288 is 12 pulls of 512.
2. The *meaning* of the bytes inside the macro block: the record header (mode, action count, repeat count and
   interval) and the action kinds are now read out of the engine binary - see
   [Macro playback modes](#macro-playback-modes-engine-binary-2026-10-28). Open: the carried byte at `+4`, the UI name
   of each of the five modes, and whether mode `5` is play-while-held. `decode` and `encode` still carry the block
   through intact and the record split is `[EXPERIMENTAL]`, used for reporting only.
   Note too that the engine's own action converter walks the block as a flat 4-byte action stream and does not skip
   record headers; the record grammar here is what the translate path uses.
3. Where macros beyond the last five are stored. The keys function for all macros, so the internal memory for
   them must exist, but at the current path it is not read.
4. A schema `A` payload has never been read on hardware. Both captures are the same firmware and all 241
   answers are schema B, so nothing in the captures discriminates firmware at all; the schema `A` layout comes
   from the engine's struct comments only, and the `4.16.3` threshold is the engine decoder's own number.
5. What the `0x030N` files are. Their ids and the reply report ID `03` behind them are in the capture; that
   they are the per-profile lighting config is GG's naming, not a hardware fact - `ssgg` only ever reads and
   writes `0x020N`, no `0x030N` file has been read on hardware, and nothing decodes one. `ssgg`'s Apex 7
   lighting goes out over the zone commands (`0x21`) from the OpenRGB-era code, not from this capture.
   The firmware package adds: one `0x5000`-byte block per slot, ids `0x301`-`0x305`, zero-filled in the
   shipped image, sitting next to the five profile slots.
6. What `0xF4` does. GG sends it third in its opening chain and the keyboard answers `1`; no engine-side word
   names it, `ssgg` implements nothing for it, and a read completes without the chain. Open too: what driver mode
   does on the device, and the full bytes of the closing frames - `69 00` (OLED off) and `41 00 00` (profile not
   volatile) are inferred from the engine's names.
7. What the six `oled_illumination_options` bytes at payload offset 27 are. The name is GG's (the lisp spec's);
   the values are known for three captures and nothing names the fields. GG's Apex 7 UI has no OLED option beyond
   the image upload (operator-reported 2026-10-23), so these are the board's own `DISPLAY` menu settings -
   `BRIGHTNESS`, `DISPLAY ICONS`, `TIMEOUT` - carried through the profile, not GG's settings written into it.
   The read-out is read-only: set one option on the board (brightness is the easiest to see), read the slot, and
   diff it against a read taken before. `ssgg` never edits these bytes - a decoded payload writes back
   byte-for-byte - so a profile edit cannot lose them.
