# Apex 7 on-board profile read

Source: capture notes (Windows USBPcap capture of SteelSeries GG talking to an
Apex 7, PID `0x1612`, 2026-10-15, firmware `4.16.8`). Every item below is `[EXPERIMENTAL]`. Read status
2026-10-18: `ssgg onboard read` completes a full slot read on an Apex 7 under Linux (Config 1, wired,
firmware `4.16.3`), and that read image now **decodes**: the block layout matches GG's payload struct and
its CRC-32/STM32 over `payload[0..11692]` equals the stored word `AE57B9DC`. Before the layout fix
(5-byte mapping entries, OLED behind the wheel META table, GUID behind the macro block) the same image was
decoded over a 12284-byte region and never matched. The read
sends no frames that change anything on the keyboard: GG's opening chain and its closing sequence turned out
not to be needed (2026-10-18, §Sequences the read does not need). Code: `src/devices/hid_reports.rs` (the command codes
and every report buffer: `OnboardFirmwareVersionCommand`
with its `FirmwareSource` byte, `OnboardDriverModeCommand`, `OnboardProfileDeployCommand`,
`OnboardPayloadPullCommand`, `OnboardBufferReadCommand`), `src/devices/keyboards/apex_7_profile.rs` (reply framing, payload geometry, reply parsing, decode - the module moved there on 2026-10-17, from `src/onboard_profile.rs`, because everything in it is specific to this legacy platform),
`src/devices/keyboards/apex_7.rs` (`Apex7`: the keyboard device of the legacy Apex family, which wraps
`GenericKeyboard`, and the read plan, reply assembly and polling - its own `read_profile` /
`read_image` / `firmware_version` / `probe_schema` / `pull_window` / `read_window` methods; the
`apex_legacy_onboard` module these lived in was folded into `Apex7` on 2026-10-18) - the packet I/O is
`GenericKeyboard`'s own profile transfers on the handle it owns
(`write_profile_packet` for output frames - not `send_report`, whose write dedups: see "Write path" below -
`send_feature`, `read_profile_feature_packet`, `read_profile_input_packet`), `ssgg onboard` (`src/cli.rs`, `src/main.rs`), and the interfaces the CLI runs on: `src/devices/keyboards/onboard.rs` (2026-10-19). Discovery builds `Apex7` from the keyboard's HID collection and product ID - it never asks the on-board
question (the catch-all arm that wrapped on-board-capable product IDs in `Apex7` is deleted 2026-10-19), so every
keyboard the manager enumerates is a keyboard with its lighting path. The on-board scope is stated once, in the device
class: `Apex7` answers the codec, and discovery builds `Apex7` for the captured collection alone (`usage_page
0xFFC0, usage 0x9F` with product ID `0x1612`, the board the frames were captured on and measured on).
`Keyboard::supports_onboard_profile` is `onboard_codec().is_some()`, and that is the gate. The read path
(transport, read plan, `Keyboard::read_onboard_profile`) is core code, labelled `[EXPERIMENTAL]`, as are the report builders.

The interfaces (2026-10-19) are per keyboard family: `Keyboard::onboard_codec` answers *which family protocol a
keyboard speaks* (`Apex7` answers with `apex_7_profile::Apex7Onboard`, every other keyboard `None`),
`Keyboard::read_onboard_profile` pulls a slot on the keyboard's handle, and `OnboardProfileCodec` is the family's
device-free codec - decode, encode, report JSON, dump names, the `Config 1..5` numbering, and the text a read
prints. `DeviceManager::open_onboard_keyboard` selects a keyboard that answers it, and `src/main.rs` names no family
module: what moved out of it is the device selection by Apex 7 product ID, the read's print text, the dump file-name
rule and the config-number rule. Nothing on the wire changed - same frames, same geometry, same output text.

The framing below is decoded from the capture's URBs (engine capture 2, parsed packet by packet: the 8-byte
setup block of every control transfer plus its data stage), not from the decoded response listing. The CSV holds only
*responses* (615 rows), so it was the starting point and not the authority: the request shapes and the
reply framing in this document are what the URB decode shows. Counts quoted here are from that decode.

## Device and transport

Legacy Apex family (Apex 7, Apex Raw, Apex 2014): four HID collections. On Windows every profile
transfer in the capture runs on **USB interface 1** (`wIndex 1`), the collection GG opens for profile
traffic; the counts below are from that file. On the Linux box used for the hardware probe
(same keyboard family, firmware `2.4.3`) the collection that accepts the same frames is
`the hidraw node`, usage page `0xFFC1`, interface 4, `max_report_size 644`, `max_input_report 1024`,
`max_output_report 64`. `/dev/hidrawN` numbers are not enumeration order. Since 2026-10-17 the read runs on the collection the
device manager opened for the keyboard (the handle `Apex7` owns), selected by product ID (`--device <pid>`, hex; every SteelSeries device shares the vendor ID, so the product ID identifies it), not by node path
and not by an interface-1 preference: the device manager picks a vendor-defined collection by usage page
(`keyboards/mod.rs`, `find_best_device_info`), which on the 2026-10-15 box landed on interface 4 - the
collection that accepted the frames there. **Open**: if a box needs interface 1 (the `wIndex 1` of every
profile transfer in the capture) and the device manager opens another collection, that collection must be
discovered as its own device; the old path-based probe that tried interface 1 first
(`candidate_paths`, `PathTransport`) is gone.

| Transfer | Control setup (from the capture) | Frame bytes (data stage) |
|----------|------------------------------|------------------------|
| firmware poll (1 frame, once) | `SET_REPORT`, `bmRequestType 0x21`, `wValue 0x0200`, `wIndex 1`, `wLength 64` | `90 <source>` zero-padded to 64, answered on the interrupt endpoint with the ASCII firmware version (`34 2e 31 …` = `4.16.8`). The source byte selects which firmware answers: `90 00` the keyboard's, `90 01` the LED controller's — both read from an Apex 7 on 2026-10-18, both forms in the capture. `ssgg onboard read` sends one `90 00` in front of the probe for the image's firmware line; GG's own session polls both forms, and in the reference capture 8 of them are chain frames, 11 ms apart with `f4 00` and `6c 00 01` (see "Opening chain" and §Sequences the read does not need — neither sequence is sent). `f4 00` answers `00 01 00…`, `6c 00 01` answers nothing, meanings unknown |
| pull | `SET_REPORT`, `wValue 0x0200`, `wIndex 1`, `wLength 64` | `85 00 <file selector u16 LE> <file offset u32 LE> <size u16 LE>`, then zeros to 64 — answered by an ordinary input report (`0x01`, all-zero on an idle keyboard), see "What a pull is answered by" |
| payload read | `SET_REPORT`, `wValue 0x0300`, `wIndex 1`, `wLength 642` | `83 00 <resource-buffer offset u16 LE> <size u16 LE>`, then zeros to 642 |
| payload write | `SET_REPORT`, `wValue 0x0300`, `wIndex 1`, `wLength 642` | `03 00 <buffer offset u16 LE> <size u16 LE> <payload…>` — GG's write direction, 144 transfers, not sent by `ssgg` |
| payload reply | `GET_REPORT`, `wValue 0x0300`, `wIndex 1`, `wLength 642` | 643 bytes on the wire: report-ID byte `03`, then a payload that carries the file head (`0b 00` + name) and not the requested window (see "Reply framing") |

All request shapes are output-report (`wValue 0x0200`) or feature-report (`wValue 0x0300`) `SET_REPORT`
transfers: report **number 0** in both cases, the command byte sits in the data stage, not in `wValue`.
On Linux that means `write()` for the firmware-poll and pull frames (data[0] = report number 0) and the
feature-report write for the read request.


## Capture evidence

Counts in the engine capture 2 for the profile collection (interface 1): 385 `SET_REPORT wValue 0x0300
wLength 642` transfers (241 read requests, 144 payload writes), 241 `GET_REPORT wValue 0x0300
wLength 642` transfers (the answers), 230 `SET_REPORT wValue 0x0200 wLength 64` transfers, 12 of the
pull shape on `wIndex 0`. The 230 split by leading byte: 121 pulls (`85`), 72 push frames (`05`, §Push: the pull frame with the direction bit swapped),
10 `6e`, 6 `68`, 6 `6a`, 4 `90`, 3 `89`, 3 `41`, 2 `69`, 1 `f4`, 1 `6c`, 1 `ff`. The four `90` frames,
the `f4` and the `6c` are the opening chain (see "Procedure"). The capture contains **no** 13-byte
transfer: an earlier version of this document described a 13-byte "pacing frame" with
`01 00 00 …`; it is a misreading of the capture's text export (see "Reply framing"), and
no such transfer exists on the wire. The words `f4 01 60 00` do appear on the wire, as `f4 00`-family
frames zero-padded to 64 bytes.

Decoded 2026-10-19 from the same file's control-endpoint SETUP records plus their data fragments, the 72
`0x05` frames are one shape: `05 00 | fs_id u16 | file offset u32 | size u16` with `fs_id 0x0102`, offsets
`0x0, 0x400, …, 0x2C00` and size 1024 - 12 chunks per run, 6 runs, 12,288 bytes declared per run. See §Push.

The engine's tail after a profile transfer is three output reports: `69` (Gamesense OLED off), `89 <profile>`
(deploy that profile) and `41 00` (profile not volatile); the engine sleeps 250 ms behind the deploy frame.
**`ssgg onboard read` sends none of them** (2026-10-18, §Sequences the read does not need); earlier builds sent
those three plus `6c 00 00` (close driver mode). §Driver mode is the engine-side reading of all of this,
including the `ff` frame the frame inventory above lists exactly once (recorded by the kit under the
capture's "idle, do nothing" step, capture summary): no engine-side name for it in this family, direction not
settled, not sent by this crate.

The read requests are exactly three shapes:

| buffer offset | size | count |
|---------------|------|-------|
| 0 | 2 | 1 (the schema probe of the profile file of slot 0) |
| 0 | 512 | 120 |
| 512 | 512 | 120 |

So a 1024-byte pull is drained by two 512-byte reads, and GG never asks for more than 512 bytes per
read. The 121 pulls are: one head probe of file `0x0201` at offset 0 with size 2 (its 2-byte read is the
schema probe), then 12 pulls of
1024 bytes per file at offsets `0, 1024, …, 11264` for each of the five profile files `0x0201`-`0x0205`
and the five lighting files `0x0301`-`0x0305`. At the control-endpoint level the submit-to-completion latency of a
payload read is 1-3 ms and the six opening-chain frames are 11 ms apart; at the endpoint level of the engine session capture (see
"What answers a pull") a pull is answered in 1.7-3.7 ms, its two reads are answered by their `GET_REPORT` 3.4 ms
(median) after the request, and GG's whole collection of the profile files - 121 pulls with 241 read-backs, lighting
frames interleaved - takes 1.8 s, i.e. about 15 ms per chunk, matching the 11 ms pacing plus transfer latency.


## Procedure

```
write output report 0, 64 bytes: 90 00 00 …  90 01 00 …  f4 00 00 …  6c 00 01 00 …  90 00 00 …  90 01 00 …
                                                                    # the opening chain, once, 11 ms apart
write output report 0, 64 bytes: 85 00 | fs_id u16 | 0 u32 | 2 u16   # schema probe of the file head
write feature report 0, 642 bytes: 83 00 | 0 u16 | 2 u16
GET_REPORT wValue 0x0300 wLength 642 -> the file bytes behind the report-ID byte
for each chunk c in 0..12:
    write output report 0, 64 bytes: 85 00 | fs_id u16 | (c * 1024) u32 | 1024 u16   # fill the buffer
    read 65 bytes of interrupt input (read-chunk HIDIO 65) and keep 0 bytes of it  # pacing, see below
    for buffer_offset in {0, 512}:
        write feature report 0, 642 bytes: 83 00 | buffer_offset u16 | 512 u16
        GET_REPORT wValue 0x0300 wLength 642 -> the file bytes behind the report-ID byte
    sleep 11 ms
write output report 0, 64 bytes: 69 00 …  89 (slot+1) 00 …  41 00 00 …  6c 00 00 00 …
                                                                    # closing: OLED off, deploy that slot
                                                                    # (250 ms sleep), not volatile, close
                                                                    # driver mode (the chain opened it)
```

12 pulls of 1024 bytes cover 12288 bytes, enough for the 11648-byte schema A payload and the
11696-byte schema B payload. **`ssgg onboard read` pulls 40 fills, not 12** (2026-10-19,
`PULLS_PER_FILE = 40`): file offsets `0..=0x9C00`, so the assembled image is 40,960 bytes and the
29,264 bytes behind the payload are kept. The extra 28 pulls are GG's **write** reach - its pushes
declare offsets up to `0x9C00` - and the point of pulling them is the open question of whether the
board stores more macro data past the 12,288 bytes its read walks. Nothing decodes that region: the
decode report records it as `payload_len`, `extra_len` and `extra_fnv1a_64`, the read's record prints
it as `region:`, and `--bin` writes the payload while the region goes to `<out>.extra.bin`. The
reference reads have only device-buffer content behind the payload (zeros; 80 bytes of `FF` behind
slot 3), and the 28 extra pulls per file have **not been run on hardware**. A schema-only read is the opening chain, one 2-byte pull of the file head
and one 2-byte read (GG's own probe, kept in the transfer map). `ssgg onboard read` runs that pair as the
first step of every read (`Apex7::probe_schema`: 2-byte pull at file offset 0, then one window read at
buffer offset 0) and **stops the read when the schema word is not `0x000B`** - schema B is what the decoder
implements and what has been measured on hardware, so a device answering any other schema is refused before
anything is pulled, rather than later as a profile whose checksum never matches. Measured 2026-10-18: the
probe's schema word is read on an Apex 7. `[EXPERIMENTAL]` (the refusal threshold is a decoder limitation,
not a device fact).
`ssgg --debug onboard read --profile N` (GG's config number, 1-5; the read addresses slot `N - 1`, the
file it pulls as `0x020N`) runs the whole read, and the read loop logs its geometry (`info` level) before the first transfer; frames are built
per chunk from the typed commands, nothing materialises a transfer list (`Step`, `StepKind`,
`ReadConfig::steps()`, `init_steps()`, `plan_json` and the `onboard plan` subcommand were removed
2026-10-17). What it writes: `-o/--out` is the decoded profile JSON, byte-for-byte the report `decode`
writes for the same image (the format `tools/profile-gui` opens); `--report FILE` adds the per-pull read
record (chunk headers, framing stats, image hash) and `--bin FILE` the raw image bytes.

**The keyboard identity is four plain fields, written first**: `product_id`, `product` (the model name this
crate lists for that number), `firmware` (what the `0x90` poll answered) and `slot` (the 0-based profile slot
the read ran on) sit above the profile fields in `ProfileImage` and in the decode report - the record says
*which keyboard, which slot* before it says *what it holds*. A read stamps all four; a decode of a file names
no keyboard and no slot, so it writes none of them (`ProfileImage` writes
them as `null` in the read record, which states the absence openly). There is no `device` object and no nested
block: a nested struct of the same three values was one layer of indirection over three fields, and the two
fields it used to carry are gone with it - every SteelSeries device answers the same vendor ID `0x1038`, so a
product ID identifies the model on its own, and a model name is derivable from the number it names, so a
record that carries both carries no redundancy. A model outside this crate's registry is labelled by that name
and a reader trusts the number. The encoder ignores all four. The GUI
shows a model **name** for that number, and takes it from Rust rather than from the file:
`tools/profile-gui/sync-products.mjs` parses `pub mod product_ids`, `device_name_from_product_id` and
`device_type_from_product_id` (`src/devices/mod.rs`) and the `codecs()`
registry (`onboard.rs`) into `tools/profile-gui/products.generated.js`, which `index.html` loads before
`app.js` (2026-10-19). The registry's name for the recorded ID wins over the `product` string the file
carries - the crate's own rule, trust the number and not the name - and the same import carries the
per-model feature set the page prints: the device kind, and membership of the family this payload layout
was read back on, which is the set its encodings apply to. A model outside that family is labelled as
needing its own layout, encoder and feature set, which is the point of recording the product ID at all.
`node tools/profile-gui/sync-products.mjs --check` fails when Rust moved and the generated file did not,
and `selftest.mjs` runs it as a check.
The profile slot is the only run-time choice. The geometry (12 pulls of 1024, 2 payload reads of 512 per
pull), the pacing ([`GG_GAP_MS`]), the firmware poll (one `90 00`) and the reply framing
([`READ_REPLY_FRAMING_LEN`], 0 = the offset the capture's replies show) are constants:
`ReadConfig` and its CLI tunables (`--resource-len`, `--read-len`, `--chunks`, `--payload-len`,
`--fs-base`, `--gap-ms`, `--no-init-chain`, `--reply-header`) were removed 2026-10-17. A keyboard that
needs other geometry is a code change, measured first on the capture.

### What paces the read, and what GG paces with `6c`

GG's pacing is **a device setting it writes to the keyboard**, not only a host-side sleep: the engine's
`hidconfig` API writes command `6c` with 12 bytes read as three u32 - `time-between-commands`,
`file-access`, `max-usb-timeout` (`standard-boilerplate.lisp:24`). Its default is 10 ms with the engine's own
comment "HID Configuration - 10 ms is also the default in devicelib" (`standard-boilerplate.lisp:8-10`),
`file-access` is `FA_READWRITE` and `max-usb-timeout` is 500 (`:9-10`; `apex_7.lisp:312-313` restates the last
two for this model). The profile APIs set it themselves: the read's spec writes
`HIDCONFIG 12 [(uint32-to-bytes 1) (uint32-to-bytes (file-access)) (uint32-to-bytes (max-usb-timeout))]` with the
comment "Set time between commands" before the first pull (`apex_7.lisp:217`), the write's spec writes the same
frame with `5000` as the third word in front of the file open (`apex_7.lisp:251`, and the normal value again at
`:253`), and both reset it in `api-finally` with `{"hidconfig": []}` (`apex_7.lisp:224`, `:270`). The `6c` frames
in the captures **are** those writes. The read capture's chain frame is
`6c 00 01 00 00 00 00 64 00 00 00 10 27 00 00` (§Opening chain lists it); its 12 bytes after the 2-byte header
read as three LE u32: `1`, `0x6400` = 25600, `0x271000` = 2560000 - the same three words the engine writes, in the
same order, with `FA_READWRITE` as the middle one (`(define (file-access) FA_READWRITE)`, `standard-boilerplate
.lisp:9`, and 25600 = `0x6400` is that flag). `ssgg`'s own `6c 00 01` builder is the **driver-mode** shape of the
same opcode (`base_apex_legacy.lisp:562`), which is 4 bytes, not 14: the two uses of `6c` in this family have the
same command byte and different payloads, so a builder for one is not a builder for the other. **The units of the
capture's words are not settled** (1 ms pacing against the engine's 10 ms default, and a third word of 2560000
against its 500).

Host side, the measured spacing on the wire is what `ssgg` paces by. The read capture's 125 pull frames are
**0 ms** apart and 708 of its 750 payload reads are **20 ms** apart; the write capture's 103 frame gaps have a
**7.1 ms** mean, 45 of them are 0 and 47 are 10-11 ms, with a 100 ms pause every 32 frames (the engine's own
bulk-transfer step, cited in §"Answered frames and pacing" of
[`apex-7-onboard-profile-write.md`](apex-7-onboard-profile-write.md)). `ssgg` paces both paths at
`GG_GAP_MS` = **5 ms** (the write capture's median, [`GG_GAP_MS`] in
[`apex_7_profile`](crate::devices::keyboards::apex_7_profile)), and waits for each answer with the
50 ms poll of [`REPLY_POLL_MS`] in [`apex7`](crate::devices::keyboards::apex_7) - the read's answer arrives 1-3 ms
after its request, so the 5 ms step covers the 1 ms the board needs to prepare a buffer (the wait in `read_window`
that keeps a fast read from pulling an unprepared buffer, recorded in `ONBOARD.md`). Nothing in the read or the
write runs on a thread of its own: `Apex7::read_profile`, `Apex7::write_profile` and `Apex7::deploy_profile` are
plain blocking calls (2026-10-19 - they used `std::thread::scope`, which joined before returning, so the work
blocked a thread either way and nothing ran concurrently).


## Payload layout

The payload starts at file offset 0 with a little-endian 16-bit schema word: `0x000A` (schema A,
payload 11648 bytes) and `0x000B` (schema B, payload 11696 bytes, from firmware `4.16.3`, which adds
the Rapid Tap block). The last 4 bytes of the payload are a little-endian checksum: CRC-32/STM32
(poly `0x04C11DB7`, init `0xFFFFFFFF`, no reflection, no final xor) over the payload read as
little-endian 32-bit words — the same algorithm solved for the Apex Pro TKL 2022/2023 profile blobs
(see [apex-pro-tkl-2023-gg-capture.md](apex-pro-tkl-2023-gg-capture.md)); `ssgg onboard decode`
reports stored and computed values.

Block layout, in the order the Windows engine's decoder walks the payload:

| Offset | Field | Status |
|-------:|-------|--------|
| 0 | schema word (u16) | confirmed |
| 2 | profile name, 24 bytes | engine decoder |
| 27 | lighting block, 57 bytes (lock/meta/win colours, dimmers, idle mode and timeout, three lock-key HID codes, the 32-byte META mapping mask at `27 + 15 = 42`, OLED illumination options, 6 trailing reserved bytes) | engine decoder, layout `[EXPERIMENTAL]` |
| 84 | META toggle HID code | engine decoder, measured value in every read decoded so far: `0xF0` - see §The META toggle is a byte and a binding |
| 85 | 128 button mappings, **5-byte** entries (`key_mapping`: function byte + 4 key codes) | engine struct, verified on hardware |
| 725 | 2 wheel mappings, 5-byte entries | engine struct |
| 735 | 128 button META mappings, 5-byte entries | engine struct, verified on hardware |
| 1375 | 2 wheel META mappings, 5-byte entries | engine struct |
| 1385 | OLED pixel block, 640 bytes (`oled_settings: bytearray (size 640)`) | engine struct, verified on hardware |
| 2025 / 2026 | Rapid Tap (schema B only): 1 enable byte + 10 **5-byte** `onboard_rapid_tap_pair` (`hid1`, `hid2`, `mode`, 2 reserved bytes, printed as the hex pair `reserved` in the decode JSON) | engine struct, verified on hardware |
| 2025 / 2076 | macro data, 9600 bytes (`uint16` count + bytes) (schema A / schema B). `2025` is the **length word** (2210) of the packed **macro record list** the block holds - see §The macro block is a packed record list | engine struct, verified on hardware |
| 11625 / 11676 | profile GUID, 16 bytes (schema A / schema B) | engine struct, verified on hardware |
| 11641 | 3 padding bytes (schema A only - schema B has none) | engine struct |
| 11644 / 11692 | checksum (u32 LE) = payload length − 4 | **verified on hardware 2026-10-18** (`AE57B9DC`) |

### The META toggle is a byte and a binding (measured 2026-10-19) `[EXPERIMENTAL]`

Payload offset 84 (`meta_toggle_hid`) is one byte naming the key GG switches META mode with. The
binding GG marks a slot with lives in the META-layer table: `function 0x62` with **code 1 = 0x05**. Code 1
is the meta **action** byte: `0x62` runs a meta action and code 1 says which one - the `MetaAction` enum
in `apex_7.lisp`, ordered `1,2,3,4,5,10` = Lock Win, Lock Game, Lock Switch, Lock All, Perf, Rapid Tap
(five enum slots, and the sixth written as its number `0x0A` by `MetaActionToKeystroke`).
The reads decoded so far carry the pair the same way:

| file | byte 84 | the META entry marked `62 / 05` | that slot's base entry |
|------|---------|--------------------------------|----------------------|
| `config_3.bin` (stock, firmware 4.16.3) | `0xF0` | slot 13 = `62 05 00 00 00`, usage `0xE3` (LWin) | slot 13 = `51 E3 00 00 00` (LWin bound to itself) |
| the remapped slot, the macro slot (same family) | `0xF0` | slot 13 = `62 05 00 00 00` | slot 13 = `51 E3 00 00 00` |
| `apex_pro_tkl_2023` `some_rebounds.json` | `0x10` (the M key) | slot 13 = LWin and slot 35 = PrtSc, both `62 05 00 00 00` | slot 93 = `51 65 00 00 00` (GG's Meta slot rebound to `0x65`) |

Four facts come off that:

- **`0xF0` is GG's code for the Meta cap, and the cap binds through slot 93.** The cap is the HID usage
  `0x65` (electrically the Application/Select keystroke), and `hid_to_offset(0x65)` is `None` - the
  embedded legacy table has `-1` at 0x65, so no profile key is named Meta. The slot GG carries the cap
  on is the slot whose own table code is `0xF0`: slot 93 (`apex_7_profile::meta_key_offset`, the
  code the page's usage table calls `Fn F0`), which a stock read binds to itself (`51 F0 00 00 00`).
  The user profile above binds slot 93 to `0x65` - the cap types the Context Menu - while its byte
  names the M key, so the cap's binding, the slot and the byte are three separable things. Every other
  key the byte names is named by its own usage.
- **What the action bytes mean (2026-10-20, Apex 7 `0x1612` wired, GG 3.20.2527.0, firmware 4.16.8;
  7 stock slot reads, 2 user profiles, GG's per-key JSON, GG's 3.20 UI strings).** The marker `0x05` is
  GG's **Lock Win** action: 7/7 stock reads carry `62 05 00 00 00` on slot 13 = LWin, and 6 of the 19
  slots GG's own JSON marks carry byte `05`. `0x0A` is **Rapid Tap**: slot 14 = PrtSc carries
  `62 0A 00 00 00` in the slot-0 read, in the user profile, and in GG's JSON, which marks 19 slots that
  byte-match that layer byte for byte. The key this page marks as META - slot 93, GG's code `0xF0` -
  binds `62 01 00 00 00`: the plain **Meta action (LCtrl)**, with the marker byte (`0xF0`) pointing at
  it, and GG's own name for that keybind is **Menu**, not Meta, in the 3.20 UI. So the action byte is a
  meta action, not a "is bound" flag: `05` is the mark GG leaves when the key it marks is bound to Lock
  Win, and the marked key of these reads is LWin, the key a meta action acts on.
- **`0x05` at code 1 is the toggle's marker, not a general "is bound" byte.** The five `0x62` entries
  of a stock META layer write `01`, `02`, `03`, `04` at code 1 on GG's four defaults and `05` **only**
  on the slot the byte names, and no read decoded so far carries a `0x62` entry on the base layer at
  all - so a binding write marks the toggle by writing `62` with code 1 = `05`, and nothing in these
  reads supports writing `05` anywhere else.
- **Lock Win and the META toggle are separate, and the GUI keeps them apart (operator report,
  2026-10-19, implemented 2026-10-20).** The binding window used to offer one list where the byte
  `0x05` was both the Lock Win label and the thing that moved the toggle: picking Lock Win wrote
  `62 05` **and** named the key in `meta_toggle_hid`. They are now two controls. The Meta action
  select lists the six actions of `0x62` (`01` Profile Switch, `02` Macro, `03` Brightness Down,
  `04` Brightness Up, `05` **Lock Win**, `0A` Rapid Tap) and picking one writes only this entry's
  code 1. The META toggle is a row of its own in the binding window (a checkbox) and writes only the
  profile byte: checking writes this key's code (`0xF0` for the Meta cap), unchecking writes `0x00`,
  and no binding byte of any layer is touched - the shape `some_rebounds.json` carries. The key the
  byte names keeps its own binding bytes in its binding window, and is painted as a meta key with
  `META` in its cap on both layers; a `62 05` entry prints `Lock Win` in its cap.
  `[EXPERIMENTAL]` neither byte has been written to a keyboard; `apex_7.lisp:145` names code `0x04`
  `toggle-key` and `:146` code `0x05` `lock-win`, which is the same split the page now draws.
- **The META mapping mask is derived from the META layer (measured 2026-10-20).** The 32 bytes at
  payload `42` (`lighting.meta_mapped_elements`) are a 256-bit mask whose **bit index is a key's own HID
  usage**. The rule the data shows: a bit is set for every key whose **META-layer** entry carries a
  binding (function byte != 0), the key named by `meta_toggle_hid` is **not** set, and the wheel's two
  entries are never in it (they have no slot and no usage). Bit-for-bit, this page's own computation
  reproduces the stored bytes in **8 of the 9 reports it decodes** - `sample-identity`, `sample-remap`,
  `sample-macros`, `config_1`, `config_3` (both stored versions), `some_rebounds.json` and
  `some_rebounds_prev.json`. The ninth, `config_1-copy.json`, is a file edited on this page: its mask is
  the read's stale bytes and misses the binding that edit added - direct evidence that GG treats the
  field as derived from the META layer, not as an independent setting. The GUI therefore shows it read-only
  and writes the derived bytes; a stock read's mask is the five default META bindings - the four
  F9-F12 usages `0x42-0x45` and LWin `0xE3`.
  `[EXPERIMENTAL]` measured in reads only; nothing of it has been sent to a keyboard. The operator
  accepted this rule and the Lock Win / META toggle split on 2026-10-20 as the page's behaviour
  ("confirmed working"); it is not hardware evidence, and the status line above is unchanged.
- **The rest of the META layer's `0x62` entries are GG's own defaults**, not the toggle: slots 72, 80,
  88 and 96 (usages `0x42`-`0x45` = F9-F12) carry `01 00 00 00`, `02 01 00 00`, `03 01 00 00`,
  `04 01 00 00`. That is why a stock slot read classifies as `meta: 6 custom` - the toggle slot plus
  the four defaults, none of them self-bound.
- **The byte and the marker can name different keys, so the byte is the binding.** The stock reads mark
  slot 13 and name the cap (`0xF0`); the user profile marks slots 13 and 35 and names the M key
  (`0x10`), whose own base and META entries are plain self-bindings (`51 10 00 00 00`) with no
  `0x62 / 05` marker. A stock META layer has no `0x62` entry bound on the *base* layer at all, so the
  base-layer `0x62` with code 1 = `05` is the binding this page writes and the META-layer marker is
  GG's own note, which the page shows and leaves alone.

No write path is measured. `tools/profile-gui` writes the byte and the binding: the Meta action select
of a key bound `0x62` (and any base-layer key, which the page fills with GG's factory `0x62` binding)
offers `Meta Toggle (0x05) - make this key the META toggle`, which writes `62 05 00 00 00` on the base
layer and moves `meta_toggle_hid` to that key's code. The option is not offered on the META layer - a
stock META layer has no `0x62` entry bound on the base layer, and GG's marker slots are shown as the
file has them - nor on a wheel entry, because GG switches a layer with a key. The Meta cap edits
through slot 93, so it rebinds like any other key and keeps the byte. See
`tools/profile-gui/README.md`, §The META toggle.

### The OLED block is the picture GG shows (measured 2026-10-19)

GG's own decoder (`profile.py` in `app_code/core.zip`, extracted from
`app.asar/contents/services/steelseriesengine/core/devices/protocols/sssapex7legacy/`) reads the
640-byte block as `oled_height = 8`, `oled_width = 128`, `bpp = 1`, and iterates
`row in 0..40`, `col in 0..128` taking `pixel[row*8 + col]` bit `7 - (col % 8)`:
**128 x 40 monochrome pixels, 1 bit per pixel, MSB first, 16 bytes per row, top row first** —
byte 0 bit 7 is the top-left pixel, byte 16 is row 2 pixel 1. `ssgg` writes that sentence into
the JSON next to the block.

The engine also serves the profile picture on `http://localhost:27304/screenshots/<id>.bmp`, and
that screenshot contains the same 128x40 bitmap GG drew, which is why the block is the picture.

Two fields carry the name `fnv1a_64`, and they use the same hash with **two different inputs**.
The variant is the point: `lisp/base_apex_legacy.lisp:343` (`:356` for the wireless struct) uses
`(fnv1a-64 (sha-256 (…))))`, and its FNV **prime literal is one byte short** - `#x10000000_01b3`
= `0x1000000001b3`, where FNV-1a-64 needs `0x100000001b3`; the offset basis is the correct
`0xcbf29ce484222325`.

- **GG's block tag** (`0x59e9c7338286e654` for the 12,500-byte payload in the device-offset table,
  written in `docs/development/apex-7-ggpkg-format.md`) runs the variant over the **hex string of
  the SHA-256 digest** - the 64 ASCII characters, not the 32 digest bytes. `profile.py` agrees on
  the SHA-256 step. Rebuilding that combination reproduces the recorded tag exactly; hashing the
  raw digest bytes instead gives a different value.
- **This decoder's summary fields** (`oled.fnv1a_64`, `macro_data.fnv1a_64`) run the same variant
  over the **block's own bytes**, no digest in between: the 640 OLED bytes of a stock slot read
  hash to `ED411A7750D88E1B`, which is what the JSON records, and 9600 zero bytes hash to
  `083B452CD5075125`, the value a macro-less read records. Measured 2026-10-19; pinned by
  `tools/profile-gui/selftest.mjs`, which recomputes both and asserts they match.

Because the two primes agree in the low 32 bits and differ only above them, the published-prime
variant agrees with the stored field in its last 8 hex digits - an earlier revision of the profile
GUI hashed with the published prime and reported the decoder's field as inconsistent with the
bytes next to it. The GUI now uses the engine variant and calls out a stored field only when it
disagrees. Nothing here is a parity test: the read image is compared against GG's own image, not
against these fields.

### The macro block is a packed record list (measured 2026-10-19) `[EXPERIMENTAL]`

The block (9600 bytes at 2076, schema B) holds a **packed list of macro records with no stride and
no length word**: in the reference read 97 bytes are non-zero, they form a 136-byte record stream at
**2076..2211**, and the block is zero-padded behind it. The engine comment "uint16 length of the
macro data" (`apex_7.lisp`) describes its in-memory struct, not a word in the payload: bytes
2020..2075 of this profile are all zero (Rapid Tap off, its ten pairs empty), so nothing in the
region declares a length. `ssgg` walks the actions and stops at the zero pad.

A record is its header plus its actions, and the next record begins at the next `0x77` byte:

| field | bytes | the two records of the reference read |
|-------|-------|-----------------------------------|
| marker | 1 | `0x77` on both. The engine's `macro_function` is `0x71` (the mapping function byte, see below), so why the record header carries `0x77` is **open** |
| `mode` | 1 | `4` on record 0 = GG's "Repeat every (time interval)"; `1` on record 1 = "Play once" |
| `id` | u16 LE | 3 and 27 - GG's macro id (the engine's own counter is `+id 1`) |
| `flags` | u16 LE | `0x0101` on both - named "repeats" in `apex_7.lisp`, meaning open |
| `interval_ms` | u16 LE | 5000 on the repeat record (matches GG's "every 5000 ms"), 15 on the once record |
| actions | — | record 0 fills 20 bytes total, record 1 fills 116 and starts at 2096 |

Actions are typed by their first byte with the length the engine states
(`lisp/base_apex_legacy.lisp:602-625`: a press is 8 bytes, a release 4, a delay 4):

| action | bytes | reading |
|--------|-------|---------|
| press | `02 <key u8> 01 01 <word u16 LE> <ms u16 LE>` | engine `(macro-action-press (ord 2) (length 8))`; `word` is 4 in every press seen, unexplained |
| release | `02 <key u8> 01 00` | engine `(macro-action-release (ord 2) (length 4))` |
| delay | `04 00 <ms u16 LE>` | engine `(macro-action-delay (ord 4) (length 4))` |
| random delay | not seen | the engine draws in `[ms, ms+19]` (`:619`); the wire byte is unknown |

`<key>` is the **HID code** of the key: record 1 presses `0x55` = `Ke *`, then `0x5A` = `Ke 2` five
times, then `0x62` = `Ke 0`, 15 ms apart. Record 0's whole key list is the `4` key - GG's macro on
this keyboard presses `4` for 1 ms, releases it, and repeats every 5000 ms.

**The index a `0x71` binding names is its third code byte.** `apex_7.lisp:415` reads
`(key-code 2) == macro id`; META+Insert - the key GG shows bound to the macro - is `71 00 00 00 00`,
index **0**, and that is record 0, the every-5000 ms press of `4`. (The engine's comment
`base_apex_legacy.lisp:378`, "0x71 macro: (,ord ,id)", is stale: the byte carries an **index**, not
GG's id, which is 3 here.) **The index is 7 times the record's position** - measured 2026-10-19 on a
user's own Config 1 read (`some_rebounds.json`: 5 records at 2104, 14 bindings, no device involved):
the base keys C, V, X carry indices **0, 7, 14** and bind records 0, 1, 2, whose decoded content is
`press LCtrl, 20 ms, tap C/V/X, release LCtrl` - the user's description of that profile is "12 keys
(C, V, X) bound to the macro list indices, aka 0, 7, 14". So a key binds record *n* with any index in
`7n .. 7n+6`, the record an index names is `floor(index / 7)` (`MACRO_INDEX_STRIDE`), and the same
read's meta keys at 21 and 25 both land on record 3 and at 28 and 33 on record 4. That is also what
the reference read's META+Home (`0x4A`, `71 00 05 00 00`, index **5**) means: 5 is in the group of
indices 0..6, so it binds the **same** record as META+Insert - GG shows one macro on that keyboard,
and the two keys share it. An index is a **deleted macro** only when its ordinal `floor(index / 7)` is
past the list: in the user's read 38, 43, 46, 63, 92, 96, 117 have ordinals 5, 6, 9, 13, 16 against 5
records - the bindings of macros GG deleted, left on the keys. The factor 7 is measured from that
profile's indices against its records' content; *why* 7 (and not, say, the number of code bytes) is
open. So `ssgg` writes `macro_index` (the raw byte) and `ordinal` on every record, and the profile GUI
resolves a binding by floor division and writes `7 x position` when a binding is chosen from the list.

What the JSON carries: `macros[]` with `index` (the binding index of the group, `7 x ordinal`), `ordinal`
(the position in the packed list), `offset`, `mode`, `mode_name`, `id`, `flags`,
`interval_ms`, `record_bytes`, `action_count`, `actions[]` (`kind`, `key`, `word`, `ms`) and `raw`
(the record's own bytes, for comparing with a capture); `macro_index` on the `0x71` entries;
`macro_note` when a block does not pack cleanly. `macro_data` carries the summary (offset, len,
nonzero, FNV, head) **and the block's 9,600 bytes as `bytes_hex`** (since 2026-10-19, like the OLED block).

### The engine keeps the macro block as an opaque bytearray, so a write sends it back verbatim `[EXPERIMENTAL]`

The engine's own struct declares this field `(field macro_data bytearray (size (macro-data-size)))`
with `(macro-data-size) 9600` (`lisp/apex_7.lisp:62` schema `a`, `:101` schema `b`,
`lisp/base_apex_legacy.lisp:109`): GG parses nothing inside it, and what the engine writes is the bytes it
was given. Our record grammar is therefore a **display view**, not the block's format, and it is partial: a
user's own profile (`Some rebounds`, read 2026-10-19, 581 non-zero block bytes) walks as five records -
28, 28, 28, 16, 8 bytes, 108 in all - and then hits the byte at `+109`, a `03` after one zero byte, which is
no marker we know, so the walk stops there with the five records listed. Real GG blocks carry action bytes
beyond the `0x02` press/release and `0x04` delay we name, and bytes behind a record header our header shape
does not cover.

So the block's bytes ride in the report as `macro_data.bytes_hex` (9,600 of them, space separated) and a
write, an encode and the GUI send them back **unchanged**; the parsed list is never the source of bytes when
the report carries them. A report written before 2026-10-19 has no `bytes_hex`, and a noted block in such a
report is refused - the bytes behind its note are not in the file, and `ssgg` does not invent them (re-read
the slot to get a report the write path accepts). Pinned by the crate test
`a_real_macro_block_the_grammar_rejects_writes_back_verbatim`, whose fixture bytes are the five record runs
of that real block, the zero pad byte, the `03`, and eight bytes standing in for what the decoder does not
name.

GG's `device-offset-lookup-table` (`lisp/base_apex_legacy.lisp`) maps a key's HID usage to its slot in
these tables: 120 of the 128 slots, `0xFF` for the 8 slots this device has no key for. Slots 120-127
belong to the `0xF1..0xF8` function codes - the media cluster - and carry the shipped media bindings.

**A factory-default entry is not a binding.** In the wired Apex 7 read, 112 of the 128 base slots are
`0x51` (combo_key) with the slot's **own** key as the first code and zeros behind it - a key bound to
itself - and the 12 remaining slots are all zero, which is what GG writes for a key the profile does not
bind. Only the 4 media-cluster slots hold anything else, and the META layer holds the shipped shortcuts
(`0x62` meta, `0x71` macro, and combos such as `RAlt+RCtrl+RShift+Esc` on META+M). `decode` reports each
entry as `binding: "self" | "none" | "custom"`, and the profile summary counts only `custom`. For a
`0x71` entry the four code bytes are **not** key codes: they carry the macro record index, and
`decode` prints `macro: <index>` in its binding line (see §The macro block is a packed record
list).

Rapid Tap mode nibbles belong to the pairs, not to the mappings: `0x80` report key 1 only, `0x08` key 2,
`0x04` key 3 (mode 3 = report keys 1 and 2).

## Encode: the decoder run backwards (2026-10-19, software only) `[EXPERIMENTAL]`

`encode_json` (`src/devices/keyboards/apex_7_profile.rs`) takes a decode report and writes payload
bytes: the schema word, the name bytes, the lighting block, the META toggle key, the four mapping tables,
the OLED pixels, the Rapid Tap pairs, the **macro block's bytes as the report carries them**, the GUID and its
padding, and last the checksum it computes over the prefix - the report's stored word is never copied.
Geometry comes from the same layout table the decoder reads, so a report that describes a block where this
schema does not put it is refused, as is a macro block whose report carries neither its bytes
(`macro_data.bytes_hex`) nor a clean record list - a noted block with no bytes has bytes no list can hold, so
the write says that instead of sending invented ones - a mapping table whose `offset` fields are not in payload
order, Rapid Tap on schema `a` or missing on schema `b`, and any hex field of the wrong length. The zero
pad the reference reads carry behind the name, behind the mapping tables and in the rest of the macro
block is written as zeros, because that is what the reads have there.

Verified on the 2026-10-18 images in this repo (schema `b`, 11,696-byte payload): `decode` -> JSON ->
`encode_json` gives back the 11,696 payload bytes **byte for byte** for `config_1.bin`, `config_1_2.bin`
and `config_3.bin`, `config_1.json` included with its macro block sent back as the 9,600 bytes the report
carries (2 records' runs inside 97 non-zero bytes, all of it unchanged). Those files are 12,288-byte read windows from the 12-pull read:
the 592 bytes behind the payload are device buffer content, not profile bytes - 80 bytes of `FF` then zeros
behind slot 3, all zeros behind slot 1 - which is why the check compares the payload and says what the tail
is. Since 2026-10-19 a read's `--bin` is exactly the 11,696-byte payload and the region behind it (the read
pulls 40,960 bytes) is written to a side file `<out>.extra.bin`, so a `--check` against a current read
image is payload against payload. Unit tests
(`encode_writes_back_a_decoded_payload_byte_for_byte` and friends) do the same round trip on the synthetic
payloads of both schemas, and cover the refusals.

CLI:

```
ssgg onboard encode config_3.json -o config_3.enc.bin --check config_3.bin
```

prints the summary of the bytes it wrote (the encoded bytes are read back through `decode` first, so the
line reports what the file says, not what the JSON claimed), the output path, and the check result.

**No byte from this path has ever been sent to a keyboard**, and the write path that will use it is not
implemented (§Write path, §Safety): encode inherits the `[EXPERIMENTAL]` status of every offset, stride
and field width it writes.

## Reply framing (measured; the anchor is a constant, not a flag)

Every payload answer in the capture is a 643-byte `GET_REPORT wValue 0x0300` data stage whose first byte
is the report-ID byte `03` and whose payload starts directly behind it: the framing length is 0. The two
shapes for file `0x0201`:

```
83 00 00 00 02 00  ->  03 0b 00                                   # 2-byte head probe: schema 0x000B
83 00 00 00 00 02  ->  03 0b 00 43 6f 6e 66 69 67 20 31 00 …      # 512 bytes: schema + "Config 1"
```

The schema word sits at payload offset 0 (`0b 00` = `0x000B`, schema B, the Rapid Tap layout) and the
profile name at payload offset 2, exactly where the Windows engine decoder puts it.

The answers are **not** a window into the file. 241 reads return only **33 distinct payloads**: 179
have an all-zero payload and 180 carry at most two non-zero bytes, the schema word `0b 00` appears in 6
replies, and a profile name appears in five — one each of `Config 1`, `Config 2`, `Config 3`, `Config 4`
and `Config 5`, never twice in one session. The requested buffer offset and the requested file offset do
not change the answer: 100 of the 120 buffer-512 replies are byte-identical to the buffer-0 reply of the
same pull. So the read-back carries the file head (schema word, and a name) and never the requested
window, which is why `ssgg onboard read` assembles payloads only as a labelled experiment: reassembling GG's 24 reads of file `0x0201` with framing 0 gives an image whose schema word
reads `0x000B` and whose name reads `Config 1`, with no checksum word at the offsets the schema A and
schema B layouts predict (CRC-32/STM32 over the 11648-byte and 11696-byte regions, as u32 and as u16
words, matches nothing). `decode` prints stored and computed checksum
words side by side; on the hardware read of Config 1 (firmware 4.16.3) they agree, `AE57B9DC`
over 11692 bytes.

An earlier version of this section claimed all 241 answers start with 20 fixed bytes
(`ff ff 00 …`), that the report-ID byte of the answers
is `B5`, `B6` or `B7`, that the name of slot 0 sits at framing offset 18, and that slots 2-5 read as
`Config 2`, `Config 3`, `Config 4`. None of that is on the wire: capture summary and decoded response listing are text
exports that prefix every report row with the capture's own USBPcap/usbmon header
(`80 02 00 …`), and its "control response" rows are host-to-device control-endpoint data - the
requests - not device answers (see "What answers a pull"). Re-derive anything here from the raw pcaps, which are the
capture's source of truth.

The framing is a constant now ([`READ_REPLY_FRAMING_LEN`] = 0, the offset the capture shows).
It was a CLI knob while the landing keyboard was unmeasured - the Apex 7 accepted 6 on 2026-10-15, and 5
suits a keyboard that echoes the request header - and the short-read sweep over
`0, 1, 2, 5, 6, 14, 19, 20, 21, 36` is no longer runnable from the CLI. Measure it from a capture
(`ssgg --debug onboard read`), record the result, and change the constant; the framing a keyboard uses is
visible in the read output: the schema word appears at byte 0 of the payload only when the anchor is right.

## What answers a pull (decoded 2026-10-17 from the raw pcaps and the engine's read spec)

A pull has **no reply with content**. Three independent lines of evidence:

1. **The engine's own read spec** (`apex_7.lisp`, the read/write APIs of the `0x1612` device): the API is
   `write-chunk HIDIO 64` + `write-chunk HIDFEATURE 643` and then per chunk
   `repeat 12 (read-chunk HIDIO 65, read-chunk HIDFEATURE 643, read-chunk HIDFEATURE 643)`. Its byte
   specifier is `(0 0 1) (repeat 12 ((0 0 0) (1 512 0) (1 512 0)))`: the pull's `HIDIO` read has **zero
   length**, it contributes no bytes to the assembled image, and the two `HIDFEATURE` reads contribute 512
   payload bytes each from offset 1. GG discards whatever packet answers a pull.
2. **Endpoint level of the engine session capture** (the engine's own session, 121 pulls): **121 of 121 pulls are followed,
   1.7-3.7 ms later, by report `0x01`, 63 bytes, every byte zero** - the ordinary input report of an idle
   keyboard. No pull is followed by a report `0x85`, `0x83` or any pull-specific frame; all 72 `0x05`
   push frames of the write runs behave the same way (§Push). The only command answers on the interrupt endpoint
   in the whole capture are report `0x2e` (ASCII firmware version `.16.8`, the answers of the `90` polls)
   and report `0x34` (ASCII serial `464233229222200029`); the all-zero `0x01` stream fills the rest (1649
   reports, median gap 14 ms = the keyboard's own poll interval).
3. **The engine's transport** (`SSEdevice.dll`, `HandleHIDCmd`): chunk types 5 and 6 are thin wrappers over
   the device transport's write/read virtual methods, and chunk types above `0x10` return `0xbadaf6` - there
   is no richer reply path.

So the packet GG reads after a pull is a **pacing token**: one HID read of the device's input stream
(65 bytes = the report-ID slot plus the 64-byte report), issued so the kernel serialises the transfer, and
its content is thrown away by design. All profile bytes arrive through the `0x83` window replies (the
`GET_REPORT 0x0300` read-back), never through a pull's answer - which is what the read loop here does too
(`Apex7::read_pull_ack` consumes one packet and looks at none of it).

**The read is required on Linux, not just a GG habit.** On 2026-10-17 an Apex 7 `0xFFC1` collection was run with
that per-pull input read removed: every payload read-back then failed with `io::read_exact: unexpected end of
file`, and with the read restored the same collection read the profile. The step is
`Apex7::read_pull_ack` (`PULL_ACK_LEN` = 65 = GG's `read-chunk HIDIO 65`, the report plus its
report-ID slot; it was 64 until 2026-10-18, which works for the pacing packet and truncates the last byte of a
65-byte answer).

The capture kit's decoded response listing lists 121 "control responses" of 64 bytes starting `85 00 <fs_id> <offset>
<size>`: `capture.ps1` labels host-to-device control-endpoint data "control response", so those rows are the
`SET_REPORT` requests GG submitted, not device answers (the same rows exist for the `05` and `83` requests,
72 and 241 of them). Reading them as replies is what produced the earlier claim that pull replies are
constant, byte-identical frames.

Timing at that level: a pull is answered in **1.7-3.7 ms**; a read request is answered by its `GET_REPORT`
**3.4 ms** later (median); GG's collection of the profile files (121 pulls, 241 read-backs, lighting frames
interleaved) takes **1.8 s**, i.e. about 15 ms per chunk - its 11 ms pacing plus transfer latency. The kit
summary's "about 2 s per chunk" is a pairing artifact: it pairs the 42 `0x85` pulls of the write runs with
the 144 `0x03` payload writes spread over the whole session.

## Push: `0x05` is the pull frame with the direction bit swapped (measured 2026-10-19) `[EXPERIMENTAL]`

The operator's Windows write capture (GG writing a profile through this toolchain) showed the
write side of the same pair: GG's writes run as `0x05` frames until the paired command of an
`OnboardPull`-shaped transfer has reached file offset `0x9C`, i.e. `0x9C00`, where the read
stops at `0x2C00`, and the frame is **byte-identical to the pull with bit 7 of the command byte cleared**:

```
85 00 | fs_id u16 | file offset u32 | size u16     # pull: fill the resource buffer, read it back with 0x83
05 00 | fs_id u16 | file offset u16->u32 | size u16  # push: declare the bytes GG then writes into the buffer
```

Re-decoded the same day from the capture, the 72 `0x05` frames are exactly that shape
(`fs_id 0x0102`, offsets `0x0..0x2C00` in 1024-byte steps, size 1024, 12 chunks per run, 6 runs; the 2026-10-19
capture carries 250 pushes reaching `0x9C00`). A push is paced like a pull: the packet after it is the
keyboard's ordinary all-zero input report, no push-specific reply. The declared bytes themselves travel in
the report `03 00 <buffer offset u16> <size u16> <payload>` 642-byte feature writes.

What this settles: the `0x05` frame layout is **measured**, not guessed (`CMD_PUSH_PAYLOAD` is named in
`apex_7_profile.rs`, and the `0x85` command builder in `hid_reports` takes the same three fields).
What it does not settle: **`fs_id 0x0102` is a file no read pulls** (the read walks profile files
`0x0201..0x0205` and lighting files `0x0301..0x0305`), so which file GG writes with `0x05`, and what a
push is answered by on the device, were open until the write capture: a push is answered with
`05 01 …` (202/202, `docs/development/apex-7-onboard-profile-write.md` §Answers and pacing). `ssgg` sends `0x05` only
in the write plan (`OnboardProfileCodec::write_plan`), no push byte has been replayed on hardware, and there is still
no read-back for an on-board write (§Safety).

## Write path: `write_profile_packet`, not `send_report` (measured in source, 2026-10-18)

The read's output frames go through `GenericKeyboard::write_profile_packet`, which calls `device.write` with no
cache. It is not the keyboard's usual output write: `GenericKeyboard::send_report` calls `write_padded_report`
(`src/devices/mod.rs:217`), whose global `HidOptimizer` (`src/devices/mod.rs:124`) hashes every report and
**skips any report byte-identical to one sent in the last 50 ms** ("Skipping duplicate HID report" in
`--debug`). Both writes put the same `SET_REPORT wValue 0x0200`, 64-byte frame on the wire for a frame the
device has not seen recently.

The difference mattered while the read sent GG's opening chain: `90 00` and `90 01` each appear twice in it,
20 ms apart on the 5 ms cadence, inside the 50 ms window, so on `send_report` the device received a
**four-frame** chain. `send_report` was tried here on 2026-10-18 and reverted the same day for that reason;
the chain itself went out of the code on 2026-10-18 (§Sequences the read does not need), and the choice of
`write_profile_packet` stands for the same reason - the firmware polls repeat.

## Sequences the read does not need (2026-10-18)

**Measured on hardware: the read works with neither sequence, so ssgg sends neither.** Operator runs of
`ssgg onboard read` on an Apex 7 collected a profile slot with GG's opening chain not sent and with the
closing sequence not sent. Code: `Apex7::send_init_chain` and `Apex7::send_end_sequence` were removed, with
`ONBOARD_INIT_CHAIN` / `OnboardInitCommand`, `onboard_end_sequence` / `ONBOARD_END_FRAME_COUNT`, and the
command codes nothing sends any more (`EngineStatus 0xF4`, `DisableGamesenseOled 0x69`,
`SetProfileVolatile 0x41`). What the read sends is one firmware poll, the 2-byte schema probe, then 25 pulls
of 1024 bytes each drained by five 636-byte payload reads (125 reads per slot).

**What that settles:** `OnboardFirmwareVersionCommand` is now a typed command with a source byte
(`FirmwareSource::Keyboard` = `90 00`, `FirmwareSource::Led` = `90 01`), and both forms were read on
hardware on 2026-10-18: `90 00` answers the firmware running the keyboard, `90 01` the LED controller's. The
capture already had both forms on the wire (capture summary rows 97 and 115 show `90 01`).

`DriverMode` (`0x6C`) and `LoadProfile` (`0x89`) stay in `hid_reports` as named commands
(`OnboardDriverModeCommand`, `OnboardProfileDeployCommand`); **nothing in ssgg sends them** - the frames are
kept as the typed way to do the driver-mode and deploy work later, per hard constraint 1 (no hand-built byte
arrays). `LoadProfile` is a device write: it makes the keyboard run the slot named.

One transfer detail measured on the same pass: a payload read needs a short wait (~1 ms) after its request
is written, before reading the reply back - without it the read races the device filling its buffer and
answers with garbage. See `Apex7::read_window`.

**Not measured:** whether the keyboard needs *any* of GG's chain frames for a read (the capture
shows GG sending all six, the hardware runs collect a slot with none of them - the "scrambled
replies" result measured the same day is what an undrained answer, not the missing frames, produced:
inference, not settled), which of the closing frames
have any keyboard effect, and whether other legacy models behave the same.

## Driver mode (engine side)

Engine-side reading of the frames that open and close a session. `ONBOARD.md` records that the `lisp/` descriptors
(dumped from the Windows engine, untracked in the working copy) are not to be referenced in code comments, so the
engine-side citations live here and the code states wire facts and points to this section.

Driver mode is one command per family, and its **shape is not the same across families**:

| Family | Command | Report chunk | Engine-side facts |
|---|---|---|---|
| legacy Apex (the `0x1612` family this read targets) | `0x6C` | 65 | `base_apex_legacy.lisp:562`, `:570` - report 0, command `0x6C`, `profile_driver_mode 0`, `macro_driver_mode 1` to open, both 0 to close. The engine writes the opening frame at device init (`:181`, wrapped in `devour-but-log-panic`, commented that on old MP firmware the write "will return zeros on the IN endpoint, which throws any consequent 'read' command off") and the closing frame in its `shutdown` handler (`:303`) |
| Apex 3 | **`0x03`** | **33** | `apex_3.lisp:23`, `:32` - report 0, command `0x03`, `reserved 0`, `profile_button_driver_mode 0`, `macro_button_driver_mode 1`, and the opening and closing structs are **byte-identical**, so on this family opening and closing are the same frame on the wire |
| Apex 3 TKL | `0x6C` | 65 | `apex_3_tkl.lisp:100`, `:109` - opens with `profile_driver_mode 1`, `macro_driver_mode 1` (the legacy family opens with 0), closes with both 0. `apex_3_tkl.lisp:99` states what the mode is for: "software driver mode must be enabled for SSKey combos to be included in the keyboard report" |

The read path is implemented for the legacy family only, and its driver-mode frames are that family's: the opening
frame `6c 00 01` (GG's device init) and the closing frame `6c 00 00` (GG's shutdown). **Neither is sent by
`ssgg`** - the read needs neither (2026-10-18, §Sequences the read does not need);
`OnboardDriverModeCommand` is the named builder for a tool that wants them. A port of the read to the Apex 3
family cannot reuse them: on Apex 3 the command byte is `0x03`, in a 33-byte report, and the closing frame is
byte-identical to the opening frame.

The engine's session close in the legacy family writes three frames (`base_apex_legacy.lisp:299-304`):
`reset_all_illumination` (command `0x3B`, `:692`), `disable_gamesense_oled` (command `0x69`, `:724`) and
`end_driver_mode` (`0x6C` with the last byte 0, `:570`). The read sends none of
them (`OnboardProfileDeployCommand` is the named builder for `89 <profile + 1>`, and nothing in ssgg sends it).

The deploy pair behind every profile file in the capture is `load_profile` (command `0x89`, `profile_id` typed
`(range 1 5)`, sent as `succ index` by `switch_to_onboard_config`, `:2280`, `:2300`) with the engine's own sleep
`chunk HIDSLEEP 4 (uint32-to-bytes 250)` - "the firmware to finish deploying", `:2289` - and `set_profile_volatile`
(command `0x41`, sent as 0 by `post_deploy_onboard` for every slot index `>= 0`, `:2291`, `:2531`).

**The `ff` frame.** The capture has exactly one 64-byte `ff` frame, recorded by the kit under its "idle, do nothing"
step (capture summary). Driver mode in this family is `0x6C` and on Apex 3 it is `0x03`, so `ff` is not the family's
driver-mode frame; no lisp of the family names a `0xff` command; its direction (IN versus OUT) is not settled. This
crate does not send it. A version of this code sent it as a closing frame on 2026-10-18 and it was removed on
2026-10-18, on the same pass that put `6c 00 00` in its place.

## Safety

The read's transfers are reads: it sends the firmware poll, the pulls and the payload-read requests, and
**nothing that changes anything on the keyboard** - GG's opening chain and closing sequence are not sent
(2026-10-18, §Sequences the read does not need). The closing sequence is where the read's only device
*writes* used to live, and `89 <slot + 1>` in it is the frame that makes the keyboard run the slot read;
`OnboardProfileDeployCommand` names it and nothing in ssgg sends it. No profile payload is written either:
do not add profile writes - the profile blob contains blocks this layout does not explain, and writing a
synthetic blob can put the keyboard in a state GG itself cannot repair. `read` needs no confirmation flag (2026-10-18: `--yes`
was dropped; 2026-10-20 the write-side `--yes` went the same way) - it is reverse-engineered protocol code, but it writes request frames only and changes
nothing on the keyboard; `decode` never touches a device. Both write the JSON decode report by default,
named from the profile name the image carries (`Config 1` -> `config_1.json`, and GG's config number when
a slot carries no name); `--bin FILE` writes the raw image and `--hex` hex-dumps it. `--profile` takes
GG's config number `1..5` and addresses slot `N - 1`, the file `0x020N`. `encode` (2026-10-19, §Encode) writes bytes to a **file** and touches no device. The device side of it is `onboard write`: a write from a JSON report runs `encode` internally, and the write path has run on hardware (2026-10-20 with `--bin` payload bytes, 2026-10-19 from a JSON report - a modified one and an unmodified one, both applied) - see `apex-7-onboard-profile-write.md`.

## What is not settled

- Whether a payload `encode` writes is accepted by a keyboard. The round trip is verified in software on
  the 2026-10-18 images (§Encode); no encoded byte has been sent. Since 2026-10-20 a code path **can** send one
  (`ssgg onboard write` - it sends when asked, `--dry-run` prints the frames first), and it never has been:
  `docs/development/apex-7-onboard-profile-write.md`.
- ~~Where this platform keeps the profile checksum.~~ Settled 2026-10-18: it is the last word of the
  *payload* (schema A at `11648-4`, schema B at `11696-4`), not of the 12288-byte read image - the image is
  padded with whole resource buffers behind the payload, and reading the image tail as the checksum is what
  made every CRC of the capture disagree.
- What lives in the file region `0x2C00..0x9C00`. The read pulls it since 2026-10-19 and records it
  (`extra_len`, `extra_fnv1a_64`, `region:` in the record, `<out>.extra.bin`), and nothing decodes it. The
  reason it is pulled at all: GG's **writes** declare offsets to `0x9C00` (§Push) while its read walks 12
  chunks, so the board may keep more macro storage than the 11,696-byte payload the decode walks. The
  reference reads show device-buffer content there (zeros, 80 bytes of `FF` behind slot 3), which is
  evidence for "nothing decodable", not a settled answer - and the 40-pull read has not been run on hardware.
- Which file `fs_id 0x0102` is (the file GG's `0x05` pushes write, §Push), and what answers a push.
- What `f4 00` and `6c 00 01` do. GG sends them in its opening chain, `f4 00` answers `00 01 00 …` and
  `6c 00 01` is not answered at all; meanings still unknown. **The chain is not needed: measured 2026-10-18,
  `ssgg onboard read` collects a profile slot without any of the six frames** (§Sequences the read does not
  need), so the code no longer sends them and `ONBOARD_INIT_CHAIN` / `Apex7::send_init_chain` are out of the
  code. The measurements stay on record in §Opening chain (including the
  "only the `90 00` poll → scrambled replies" result, which the runs later the same day contradict).
  The one-frame-at-a-time answer draining (`read-chunk HIDIO 65` per frame) is likewise moot.
- GG's write direction (`03 00 <offset> <size> <payload>`, 144 transfers in the capture): not used
  here, and there is no read-back command known for this family, so `ssgg onboard` writes no profile payload.
  The read sends no writes at all (2026-10-18): the closing sequence is out of the code.
- What the closing sequence does on hardware. The first three frames are named on the engine side, and **the
  read no longer sends any of them** (2026-10-18, §Sequences the read does not need): does `89` switch the
  keyboard to the slot (the engine's config-switching function is what sends it), what does disabling the Gamesense OLED look like on an Apex 7, and
  does `6c 00 00` put a device back to polling? The frames are named, never replayed, and the keyboard-effect question is open. In
Linux run capture (the 1428-packet Linux run capture; measurements below are from that file - only a 66-packet
excerpt of a related run, sha256 <digest recorded outside the repo>, was still in the
tree on 2026-10-17, so re-verification needs the capture taken again) - every frame was answered 1.0-3.4 ms after it was
submitted, and the endpoint sat idle in 1133 gaps totalling 52 s: 737 gaps of 10-20 ms, 196 of 30-40 ms, 200 of 40-50 ms.
A reader that opens the node per report leaves the endpoint unpolled exactly in those gaps, so the kernel discards the
report:
`poll(POLLIN)` on a freshly opened descriptor waits out its whole timeout and returns 0, and `read(2)` issued without
a poll returns `EAGAIN` (frames 364, 431, 557, 765, 811, 1020, 1092 of that capture). Every `poll` + `read` of that run
timed out on reports the device had answered.

Hidapi's `read_timeout` is the same `poll` + `read(2)` pair (`hidapi-2.6.7 src/linux_native.rs:511` in the
`linux-native` backend this crate builds; `open` is `O_RDWR | O_CLOEXEC`, `src/linux_native.rs:451`; the C backend is
`etc/hidapi/linux/hid.c:1108`) and returns `0` on timeout, which keeps "nothing arrived" apart from "the read failed" -
the run's `EAGAIN` came from a `read(2)` issued after its `poll` had timed out. The read path was switched
briefly to one descriptor held open for the whole session, reverted the same day back to per-call opens, and ended up on
the keyboard's own handle (commit `05c7bc7`, 2026-10-17), which is one open descriptor for the whole read: what the
endpoint-polling rule above argues for. The 2026-10-17/18 runs that read a profile ran on that handle.

The kernel's own read-back owns the endpoint for its duration: `usb_get_report()` unlinks the in-flight URB, issues
`GET_REPORT`, copies the answer into the URB buffer, queues it to `hidraw`, then resubmits the URB
(`drivers/hid/usbhid/hid-core.c:964-983`, kernel 6.12.90). A report submitted into that window goes into the read-back
buffer and not onto the descriptor, and the read-back bytes are queued to every open descriptor, so they appear on the
handle as a duplicate input report. Measured in the same capture: URB submitted 3.866646, `GET_REPORT` 51 ms later at
3.866697, URB unlinked with 0 bytes at 3.920627. Not verified on hardware.

**Transport (2026-10-17).** The hand-rolled raw paths are gone: every transfer is a hidapi call on a path opened
for that call. The `linux-native` backend this crate builds implements `send_feature_report` and `get_feature_report`
as the kernel's own `HIDIOCSFEATURE`/`HIDIOCGFEATURE` (`hidapi-2.6.7/src/linux_native/ioctl.rs:20,26`), `write` as
`HIDIOCSOUTPUT` (nr `0x0b`, `:38`) and `read_timeout` as `poll` + `read(2)`; the direction-bit bug the earlier native
backend sent was fixed in 2.6.6 and 2.6.7 is pinned, so one code path serves Linux, Windows and macOS - the
`#[cfg(unix)]`/`#[cfg(not(unix))]` pairs are deleted. `devices::send_feature_report_raw`,
`read_feature_report_raw`, `write_output_report_raw` and `read_input_report_raw` - one path opened per call - were then
**removed the same day**: the Apex 7 profile read runs on the `Apex7` device handle, because a reader
that opens the node per report cannot collect the opening-chain answers or the pull acknowledges, and Apex 7 wired works
through hidapi. `Keyboard::send_feature` (the Apex 2023 lighting path) is back on the device handle: the interface-1 reading
behind its 2026-10-15 path special case was never measured, and **needs a hardware test on `0x1628`**. Two consequences worth knowing: `HIDIOCSOUTPUT` puts output frames on the control port as
`SET_REPORT wValue 0x0200` - the shape the reference capture shows - where the raw `write(2)` this replaces went to an
OUT endpoint the profile collection does not have; and `HidApi::new` enumerates the device tree per call. The read is
confirmed on hardware (Apex 7, 2026-10-18); the geometry of the decoded payload is not.

Measured 2026-10-16 on the usbmon capture of a full `ssgg onboard read` run (Linux run capture, 1428 URBs,
bus 1 device 24, 1.026 s): the chain answered `4.16.8` (68 reports) and `00 01`, `6c 00 01` answered
nothing, and the `0x85` pulls and `0x83` read requests were accepted — and the capture contains **no
control-IN transfer and no control data stage at all**, so not one byte of payload came back. All five
interfaces appear as `wIndex 0..4`; interface 1 never carries a 642-byte request. Side-by-side with GG's
own read of the same keyboard in the section above: GG reads the window with `GET_REPORT wValue 0x0300
wIndex 1 wLength 642` after every request.

The knob that reproduces GG's shape on Linux is the **report number**, not the length: the kernel builds
`wValue` from the first byte of the `HIDIOCSFEATURE`/`HIDIOCGFEATURE` buffer, so the run above asked
`wValue 0x0303` for its read-back (`READ_REPLY_ID = 0x03` at the time) and framed every command as
`wValue 0x0290`, `0x0285`, `0x0383`. GG asks for **report number 0** in both directions, and the platform's
own descriptors agree: the legacy base platform declares its 643-byte feature chunks and 65-byte output chunks with
`report-id 0x00` (643 = report-ID byte + 642 payload, 65 = report-ID byte + 64). The kernel validates a
GET_REPORT against the report descriptor of the interface being asked, so the request must go to the
collection that declares it (interface 1, `0xFFC0`) — the `0xFFC1` collections are where the chain, pull
and read *requests* are accepted, and interface 1 answered `EPIPE` to our report-ID-framed writes.
