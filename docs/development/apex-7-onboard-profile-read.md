# Apex 7 on-board profile read

Source: capture notes (Windows USBPcap capture of SteelSeries GG talking to an
Apex 7, PID `0x1612`, 2026-10-15, firmware `4.16.8`). Every item below is `[EXPERIMENTAL]`. Read status
2026-10-18: `ssgg onboard read` completes a full slot read on an Apex 7 under Linux (operator-reported;
model, firmware and the decoded bytes are not recorded here), so the **read** is replayed on hardware —
what is not verified is the decode (slot layout, checksum words, the per-slot compare against GG). The read
sends GG's six-frame opening chain (`ONBOARD_INIT_CHAIN`): with only the `90 00` poll the device answers and
the payload replies come back scrambled (2026-10-18). Nothing in this protocol writes to the device. Code: `src/devices/hid_reports.rs` (the command codes
and every report buffer: `OnboardFirmwareVersionCommand`, `OnboardInitCommand` / `ONBOARD_INIT_CHAIN`,
`OnboardPullCommand`,
`OnboardReadCommand`), `src/devices/keyboards/apex_legacy_profile.rs` (reply framing, payload geometry, reply parsing, decode - the module moved there on 2026-10-17, from `src/onboard_profile.rs`, because everything in it is specific to this legacy platform),
`src/devices/keyboards/apex7.rs` (`Apex7`: the keyboard device of the legacy Apex family, which wraps
`GenericKeyboard`, and the read plan, reply assembly and polling - its own `read_profile` /
`read_image` / `send_init_chain` / `probe_schema` / `pull_window` / `read_window` methods; the
`apex_legacy_onboard` module these lived in was folded into `Apex7` on 2026-10-19) - the packet I/O is
`GenericKeyboard`'s own profile transfers on the handle it owns
(`write_profile_packet` for output frames - not `send_report`, whose write dedups: see "Write path" below -
`send_feature`, `read_profile_feature_packet`, `read_profile_input_packet`), `ssgg onboard` (`src/cli.rs`, `src/main.rs`). Discovery builds `Apex7` for the family
(the device class is part of the device tree); the read path
(transport, read plan, `Keyboard::read_onboard_profile`) is core code, labelled `[EXPERIMENTAL]`, as are the report builders.

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
device manager opened for the keyboard (the handle `Apex7` owns), selected by `vid:pid`, not by node path
and not by an interface-1 preference: the device manager picks a vendor-defined collection by usage page
(`keyboards/mod.rs`, `find_best_device_info`), which on the 2026-10-15 box landed on interface 4 - the
collection that accepted the frames there. **Open**: if a box needs interface 1 (the `wIndex 1` of every
profile transfer in the capture) and the device manager opens another collection, that collection must be
discovered as its own device; the old path-based probe that tried interface 1 first
(`candidate_paths`, `PathTransport`) is gone.

| Transfer | Control setup (from the capture) | Frame bytes (data stage) |
|----------|------------------------------|------------------------|
| firmware poll (1 frame, once) | `SET_REPORT`, `bmRequestType 0x21`, `wValue 0x0200`, `wIndex 1`, `wLength 64` | `90 00 …` zero-padded to 64, answered on the interrupt endpoint with the ASCII firmware version (`34 2e 31 …` = `4.16.8`). **This poll is the first frame of GG's opening chain, 11 ms apart: `90 00`, `90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01`, and the read sends all six** (`ONBOARD_INIT_CHAIN`). `90 00` and `90 01` answer identically — measured on an Apex 7 wired on 2026-10-17 — `f4 00` answers `00 01 00…`, `6c 00 01` answers nothing, and the meanings of both are unknown. The chain is not decoration: without it the payload replies come back scrambled (Apex 7, 2026-10-18), which is why `OnboardInitCommand`/`ONBOARD_INIT_CHAIN` were restored on 2026-10-18 after being removed on 2026-10-17. **Every frame is followed by one input read**: a chain run that drained only the first answer collected the `90 00` and `90 01` answers and nothing from frame 3 on (Apex 7, 2026-10-18) |
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
pull shape on `wIndex 0`. The 230 split by leading byte: 121 pulls (`85`), 72 lighting frames (`05`),
10 `6e`, 6 `68`, 6 `6a`, 4 `90`, 3 `89`, 3 `41`, 2 `69`, 1 `f4`, 1 `6c`, 1 `ff`. The four `90` frames,
the `f4` and the `6c` are the opening chain (see "Procedure"). The capture contains **no** 13-byte
transfer: an earlier version of this document described a 13-byte "pacing frame" with
`01 00 00 …`; it is a misreading of the capture's text export (see "Reply framing"), and
no such transfer exists on the wire. The words `f4 01 60 00` do appear on the wire, as `f4 00`-family
frames zero-padded to 64 bytes.

The engine's tail after a profile transfer is three output reports: `69` (Gamesense OLED off), `89 <profile>`
(deploy that profile) and `41 00` (profile not volatile); the engine sleeps 250 ms behind the deploy frame.
`ssgg onboard read` sends four frames at the end of a read (see "Procedure", `Apex7::send_end_sequence`): those
three plus `6c 00 00`, which closes driver mode - the read's opening chain opened it with `6c 00 01`, and
closing it is what the engine does at the end of a session. §Driver mode is the engine-side reading of all of
this, including the `ff` frame the frame inventory above lists exactly once (recorded by the kit under the
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
11696-byte schema B payload. A schema-only read is the opening chain, one 2-byte pull of the file head
and one 2-byte read (GG's own probe, kept in the transfer map). `ssgg onboard read` runs that pair as the
first step of every read (`Apex7::probe_schema`: 2-byte pull at file offset 0, then one window read at
buffer offset 0) and **stops the read when the schema word is not `0x000B`** - schema B is what the decoder
implements and what has been measured on hardware, so a device answering any other schema is refused before
anything is pulled, rather than later as a profile whose checksum never matches. Measured 2026-10-18: the
probe's schema word is read on an Apex 7. `[EXPERIMENTAL]` (the refusal threshold is a decoder limitation,
not a device fact).
`ssgg --debug onboard read --yes --profile N` runs the whole read, and the read loop logs its geometry (`info` level) before the first transfer; frames are built
per chunk from the typed commands, nothing materialises a transfer list (`Step`, `StepKind`,
`ReadConfig::steps()`, `init_steps()`, `plan_json` and the `onboard plan` subcommand were removed
2026-10-17).
The profile slot is the only run-time choice. The geometry (12 pulls of 1024, 2 reads of 512), the pacing
([`GG_GAP_MS`]), the firmware poll (one `90 00`; GG's chain is not sent) and the reply framing
([`READ_REPLY_FRAMING_LEN`], 0 = the offset the capture's replies show) are constants:
`ReadConfig` and its CLI tunables (`--resource-len`, `--read-len`, `--chunks`, `--payload-len`,
`--fs-base`, `--gap-ms`, `--no-init-chain`, `--reply-header`) were removed 2026-10-17. A keyboard that
needs other geometry is a code change, measured first on the capture.


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
| 27 | lighting block, 57 bytes (lock/meta/win colours, dimmers, idle mode and timeout, three lock-key HID codes, OLED illumination options, 6 trailing reserved bytes) | engine decoder, layout `[EXPERIMENTAL]` |
| 84 | META toggle HID code | engine decoder |
| 85 | 128 button mappings, 4-byte entries | engine decoder |
| 597 | 2 wheel mappings | engine decoder |
| 605 | 128 button META mappings | engine decoder |
| 1117 | 2 wheel META mappings | engine decoder |
| 1125 | profile GUID, 16 bytes | **inferred** |
| 1141 | 3 padding bytes (schema A) | **inferred** |
| 1144 | OLED pixel block, 640 bytes | engine decoder says 640 bytes; offset **inferred** |
| 1784 | Rapid Tap: 1 enable byte + 10 pairs of `(HID, HID, mode, reserved)` (schema B only) | engine decoder for the 10 pairs; offset **inferred** |
| 1784 / 1825 | macro data, 9600 bytes (schema A / schema B) | **inferred** |
| end − 4 | checksum (u32 LE) | confirmed |
| end − 260 | — | unaccounted bytes in schema A, 267 in schema B |

Each mapping entry is `function HID byte`, `key_1`, `key_2`, `key_3`; a zero function byte means
"no key bound", mode nibbles `0x80` report key 1 only, `0x08` key 2, `0x04` key 3 (mode 3 = report
keys 1 and 2, which is what the Rapid Tap pairs use).

The Windows engine's start-location table for a neighbouring Apex 7 model names 644 and 2088 for the
OLED and macro blocks; neither the 4-byte nor the 5-byte entry hypothesis lands on those numbers, so
they are recorded as an open question, not as a layout.

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
words, matches nothing). `decode` prints stored and computed checksum words side by side and never claims
a profile is verified.

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
   lighting pushes of the write runs behave the same way. The only command answers on the interrupt endpoint
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

## Write path: `write_profile_packet`, not `send_report` (measured in source, 2026-10-18)

The read's output frames go through `GenericKeyboard::write_profile_packet`, which calls `device.write` with no
cache. It is not the keyboard's usual output write: `GenericKeyboard::send_report` calls `write_padded_report`
(`src/devices/mod.rs:217`), whose global `HidOptimizer` (`src/devices/mod.rs:124`) hashes every report and
**skips any report byte-identical to one sent in the last 50 ms** ("Skipping duplicate HID report" in
`--debug`). Both writes put the same `SET_REPORT wValue 0x0200`, 64-byte frame on the wire for a frame the
device has not seen recently.

The difference shows up in one place, the opening chain: `90 00` and `90 01` each appear twice in
`ONBOARD_INIT_CHAIN`, 20 ms apart on our 5 ms cadence, inside the 50 ms window. On `send_report` the device
would receive a **four-frame** chain; the measurement that made the chain load-bearing (section
"Opening chain") had all six frames on the wire, as the capture shows GG sending them. `send_report` was tried
here on 2026-10-18 and reverted the same day for that reason. Pulls and payload reads never repeat (their file
and buffer offsets differ), so nothing after the chain is affected by the choice.

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

The read path is implemented for the legacy family only, and its two driver-mode frames are that family's: the opening
chain's fourth frame `6c 00 01` and the closing sequence's last frame `6c 00 00`. A port of the read to the Apex 3
family cannot reuse the closing frame: on Apex 3 the command byte is `0x03`, in a 33-byte report, and the closing
frame is byte-identical to the opening frame.

The engine's session close in the legacy family writes three frames (`base_apex_legacy.lisp:299-304`):
`reset_all_illumination` (command `0x3B`, `:692`), `disable_gamesense_oled` (command `0x69`, `:724`) and
`end_driver_mode` (`0x6C` with the last byte 0, `:570`). The read's closing sequence writes the last two, not the
lighting reset.

The deploy pair behind every profile file in the capture is `load_profile` (command `0x89`, `profile_id` typed
`(range 1 5)`, sent as `succ index` by `switch_to_onboard_config`, `:2280`, `:2300`) with the engine's own sleep
`chunk HIDSLEEP 4 (uint32-to-bytes 250)` - "the firmware to finish deploying", `:2289` - and `set_profile_volatile`
(command `0x41`, sent as 0 by `post_deploy_onboard` for every slot index `>= 0`, `:2291`, `:2531`).

**The `ff` frame.** The capture has exactly one 64-byte `ff` frame, recorded by the kit under its "idle, do nothing"
step (capture summary). Driver mode in this family is `0x6C` and on Apex 3 it is `0x03`, so `ff` is not the family's
driver-mode frame; no lisp of the family names a `0xff` command; its direction (IN versus OUT) is not settled. This
crate does not send it. A version of this code sent it as a closing frame on 2026-10-18 and it was removed on
2026-10-19, on the same pass that put `6c 00 00` in its place.

## Safety

The read's transfers are reads, and it ends with the engine's closing sequence - four frames that make the
keyboard **run** the slot that was read (`89 <slot + 1>`) and close the driver mode its own opening chain opened
(`6c 00 00`), see `Apex7::send_end_sequence`, "Procedure" and §Driver mode. Nothing else in the protocol's write
direction is sent: the push command (`0x05`) is named in
`src/devices/keyboards/apex_legacy_profile.rs` for completeness and nothing sends it, and no profile payload is
written. Do not add profile writes: the profile blob contains blocks this layout does not explain, and writing a
synthetic blob can put the keyboard in a state GG itself cannot repair. `read` requires `--yes`; `decode` never
touches a device.

## What is not settled

- Where this platform keeps the profile checksum. GG's assembled image ends in zeros at the offsets
  the schema A and schema B layouts predict, and the CRC-32/STM32 of every region candidate matches
  none of them. The 2022/2023 profile blobs use a word at blob offset 12320 (see
  [apex-pro-tkl-2023-gg-capture.md](apex-pro-tkl-2023-gg-capture.md)); nothing in this capture says the
  legacy family does the same.
- What `f4 00` and `6c 00 01` do. GG sends them in its opening chain, `f4 00` answers `00 01 00 …` and
  `6c 00 01` is not answered at all. **The chain as a whole is required on hardware**: a read that sends only
  the `90 00` poll is answered on the wire and its payload replies come back as scrambled bytes, with the
  chain sent the same read collects a profile slot (Apex 7, 2026-10-18), so `ssgg onboard read` sends all six
  frames (`ONBOARD_INIT_CHAIN`, `Apex7::send_init_chain`, restored 2026-10-18 after being
  removed 2026-10-17). **Which of the six frames the keyboard needs is open** - that is the question to answer
  by dropping them one at a time on a run that reads a known slot. The answers are produced **one frame at a
  time**: a chain run that drained only the first answer saw `90 00` and `90 01` answered and nothing from
  frame 3 on, with all six frames on the wire (Apex 7, 2026-10-18), so `send_init_chain` reads one input
  report after every frame (GG's `read-chunk HIDIO 65`). Why an undrained answer silences the frames after it
  is not measured - the command-queue flow-control reading is inference.
- GG's write direction (`03 00 <offset> <size> <payload>`, 144 transfers in the capture): not used
  here, and there is no read-back command known for this family, so `ssgg onboard` writes no profile payload.
  The only writes the read sends are the four closing frames (`onboard_end_sequence`).
- What the closing sequence does on hardware. The first three frames are named on the engine side, and none of
  the four has been sent from this crate on hardware: does `89` switch the keyboard to the slot (the engine's
  config-switching function is what sends it), what does disabling the Gamesense OLED look like on an Apex 7, and
  what changes when driver mode is closed? `ONBOARD.md` step 8 is the test. The capture's single `ff` frame
  (recorded in the kit's "idle" step, capture summary) is unaccounted for: no engine-side name for it in this family
  and its direction is not settled, so it is not sent (§Driver mode).
- The other `wValue 0x0200` frames of the capture (`0x6e` x10, `0x68` x6, `0x6a` x6 = idle in/out, Gamesense
  OLED enable, restart graphics) and the 57-byte lighting block layout, which is copied from the engine decoder
  and unverified.
- Whether the read-back can carry a window at all. Every answer in the capture is the file head plus
  zeros; no capture in this repo shows a payload-read reply that changes with the requested offset.
- The Linux framing of the read-back. GG asks report number 0 on interface 1 in both directions; the run
  of 2026-10-15 asked the command byte as the report number on interfaces 0-4 (`wValue 0x0290`, `0x0285`,
  `0x0383`) and its interface-1 read-back went out at `wLength 641`, one byte short of GG's 642 (the
  raw-ioctl code subtracted the report-ID byte; hidapi does not). The report number is fixed in code since
  2026-10-17 (every buffer now starts with report number 0, `READ_REPLY_ID = 0x00`) and the length since
  2026-10-18 (`READ_REPORT_LEN = 642`, asserted against `ONBOARD_MAX_READ_LEN`); what remains unmeasured is
  asking the read-back of the collection that declares it - interface 1, `0xFFC0` - with `wLength 642` and
  seeing a control-IN data stage come back (the `--debug` log distinguishes "the request failed" from "the
  transfer answered with the wrong length").

## Windows GG vs Linux `ssgg onboard read`, same keyboard (measured 2026-10-15)

Both sides re-derived from raw URBs: the engine's own read (engine session capture
and engine capture 2, the same session, 13:44:21-13:46:21, USBPcap linktype 152) and `ssgg onboard read`
(Linux run capture, 22:03:17.904 -> 22:03:18.930, 1.026 s, 1428 URBs, usbmon linktype 220, bus 1
device 24 = the `0x1612` control interface, 612 control submits and 612 completes, 204 interrupt URBs).

| Item | Windows GG | Linux `ssgg onboard read` |
|------|------------|---------------------------|
| Opening chain | six 64-byte output reports on interface 1 (`90 00`, `90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01`), 8-14 ms apart, median 11 ms | same six frames, 5-6 ms apart, **sprayed over interfaces 0-4** (340 `90*` writes = 68 per interface, 17 `f4 00` and 17 `6c 00 01` on each) |
| Report ID / lengths | report ID 0 everywhere: `wValue 0x0200` wLength 64 for output frames, `wValue 0x0300` wLength 642 for the 642-byte frames | report ID = command byte: `wValue 0x0290`/`0x02f4`/`0x026c` wLength 101, `0x0285` wLength 102, `0x0383` wLength 1603 (the report-ID byte is counted twice, so every frame declares one byte more than GG's and carries 100/101/1602 payload bytes) |
| Pull `85` | 121 transfers, header `85 00 01 02` with a u32 at bytes 4-7 stepping 0, 4, 8, ... 44 | 85 transfers (17 x 5 interfaces), 7 distinct payloads, that word stays 0 |
| Read `83` | 241 requests in three shapes: `83 00 00 …`, `83 00 00 …`, `83 00 00 …` (121/120: file 1 window 2 and file 2 window 2, read over and over) | 17 requests, **all byte-identical** `83 00 00 …` — the file and window words never change |
| **Read-back** | **241 GET_REPORT control-IN transfers `a1 01 wValue 0x0300 wIndex 1 wLength 642`**, one per read request, each answered by a 643-byte control-IN data stage (report-ID byte `03` + 642 payload bytes; 1..513 non-zero bytes per reply, 89% of its u32 words are zero). The name-bearing reply is `03 0b 00 …` = schema word `0x000B` + `Config 1` | GET transfers **did** appear: 24 setup blocks `a1 01 wValue 0x0300 wLength 641` - one byte short of GG's 642, because the raw-ioctl code subtracted the report-ID byte from the length it passed to the kernel. No complete carries a data stage, so no payload arrived; the 64-byte interrupt reports `ssgg` collected were `34 2e 31 …` ("4.16.8") x 68, `00 01` x 17, `85 01` x 17 |
| Chain answers | `90 00`/`90 01` -> "4.16.8", `f4 00` -> `00 01 00 …`, `6c 00 01` -> nothing | identical, byte for byte |
| Other traffic in the window | 144 `03` 642-byte profile writes, 72 `05` lighting frames | none (read-only run) |
| Write path (2026-10-18, code, not this capture) | GG's engine writes each frame once | `ssgg` writes with `write_profile_packet`, no cache, so all six chain frames reach the device; the keyboard's ordinary output write `send_report` would not, because the hid optimizer drops a report byte-identical to one sent in the last 50 ms - that is chain frames 5 and 6. See "Write path" below |

The point that decides everything: **the window is fetched with a GET_REPORT of the feature report**
(`bmRequestType 0xa1`, `bRequest 0x01`, `wValue 0x0300`, `wIndex 1`, `wLength 642`) issued right after
the `83` request, on the same interface. `ssgg onboard read` never issues it, so its capture has nothing
to reassemble: the request is accepted, the answers it collects are the 64-byte interrupt status reports,
and no control transfer in its own capture carries data.

The kernel strips the report-ID byte from a feature-report **write** (the report number in the buffer's byte 0 is
removed and `wLength` is the remaining byte count), which is why a 643-byte write buffer goes out as
`wLength 642`. On the **read** side the kernel forwards the requested length verbatim: hidapi hands the slice
length straight to `HIDIOCGFEATURE` (hidapi 2.6.7 `src/linux_native/ioctl.rs:26`). So the ask equals the wire
number, the device's report-ID byte arrives in buffer[0], and a 642-byte ask delivers `[report ID][641 bytes]`.
The length to ask for is **642**: 642 answers, 643 makes the control transfer never complete (measured
2026-10-15, `Connection timed out`), and the 2026-10-15 `ssgg` run asked 641 - its raw-ioctl code subtracted
the report-ID byte itself, one byte short of GG's number. See the collection map below.

What is a blocker: GG works on **interface 1 only** (the `0xFFC0` control collection), and that is exactly
where our writes failed - the collection map records `EPIPE` on interface 1's chain write, while GG's
identical interface-1 frames are accepted. The difference is the report number: GG uses **report number 0**
for everything (`wValue 0x0200` for the 64-byte output frames, `wValue 0x0300` for the 642-byte frames);
the 2026-10-15 frames carried the command byte as the report number (`wValue 0x0290`, `0x0285`, `0x0383`).
The kernel builds `wValue` from the first byte of the `HIDIOCSFEATURE`/`HIDIOCGFEATURE` buffer, and validates
a GET_REPORT against the report descriptor of the interface being asked, so report number 0 on interface 1 is
the frame that reproduces GG's transfer. Fixed in code 2026-10-17: the report buffers are built in
`src/devices/hid_reports.rs` with report number 0 as byte 0 of every buffer, so a transfer is
`wValue 0x0200 wLength 64` / `wValue 0x0300 wLength 642`, and the read-back asks report number 0. Not a blocker: the interface spray (interfaces 0-4 accept our
frames and answer the chain there, GG never touches them) - the answers are identical, so the spray is only
wasted traffic.

Our run's read-back did reach the wire, one byte short: this file's setup blocks include
`GET_REPORT wValue 0x0300 wLength 641` (24 of them, on interface 1) beside the 24
`SET_REPORT wValue 0x0300 wLength 642` writes and the 18 `GET_DESCRIPTOR`s; the raw-ioctl code subtracted the
report-ID byte from the length it handed the kernel, and every control complete carries a 0-byte data stage,
so no reply payload arrived. hidapi sends the buffer length to `HIDIOCGFEATURE` verbatim
(`src/linux_native/ioctl.rs:26`), so today's ask is `READ_REPORT_LEN = 642` and that decodes as
`wLength 642`, GG's number. The size history, recorded before `--debug` existed: on 2026-10-15 a
643-byte request produced a 5-second `ETIMEDOUT`, which means a transfer was submitted then. The 22:03 run
asked 641 and its completes carry no data stage: the request went out, the keyboard answered nothing.

## Linux collection map (measured 2026-10-15)

From the kernel's hidraw `uevent` files for a wired Apex 7 (firmware `2.4.3`). Each USB interface is
its own hidraw node, and the profile-capable collection is the vendor page `0xFFC1`:

| Node | Interface | Usage page | Max feature | Max input | Max output | Frames accepted |
|------|-----------|------------|-------------|-----------|------------|---------------|
| `hidraw3` | 4 | `0xFFC1` | 644 | 1024 | 64 | opening-chain frame, pull, 643-byte read request |
| `hidraw2` | 1 | `0xFFC0` | not captured | — | — | `EPIPE` on the chain write |
| `hidraw4`, `hidraw5` | 2, 3 | `0x0001`, `0x000C` | — | — | — | `EPIPE` on the chain write |

The reply is read as a feature report on the same node, with a buffer of **642** bytes: on the read side the
kernel forwards that number to the device as `wLength` verbatim, and the device's report is 642 bytes. Asking
for 643 (the report-ID byte counted twice) makes the control transfer never complete, which is the
`Connection timed out (os error 110)` recorded on 2026-10-15 after the read request was accepted. The
64-byte input reports this collection carries are poll and status reports (report IDs `0x0B`, `0x12`);
the collector skips them and keeps the interrupt endpoint as a fallback reply channel, read on the node
(see "Input reports: the node must be open when the report arrives").

## Input reports: the node must be open when the report arrives (measured 2026-10-16)

`usbhid` polls a hidraw collection's interrupt endpoint only while a descriptor on that minor is open. In
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
