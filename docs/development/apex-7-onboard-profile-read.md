# Apex 7 on-board profile read

Source: `ssgg-capture-1612-20261015-004400` (Windows USBPcap capture of SteelSeries GG talking to an
Apex 7, PID `0x1612`, 2026-10-15, firmware `4.16.8`). Every item below is `[EXPERIMENTAL]`: nothing of
it has been replayed from `ssgg` on real hardware. Code: `src/devices/hid_reports.rs` (the command codes
and every report buffer: `OnboardInitCommand` and `ONBOARD_INIT_CHAIN`, `OnboardPullCommand`,
`OnboardReadCommand`), `src/onboard_profile.rs` (read sequence plan, reply parsing, decode),
`src/devices/keyboards/apex_legacy_onboard.rs` (transfers, plus the `OnboardTransport` the device path
uses), `src/devices/keyboards/apex7.rs` (`Apex7`: the keyboard device of the legacy Apex family, which
wraps `GenericKeyboard` — its `HidReportBuilder` and open `HidDevice` — and runs the read on that
handle), `ssgg onboard` (`src/cli.rs`, `src/main.rs`); the read path is behind the `onboard` feature,
the report builders are not.

The framing below is decoded from the capture's URBs (`hub-2.pcap`, parsed packet by packet: the 8-byte
setup block of every control transfer plus its data stage), not from `reports.csv`. The CSV holds only
*responses* (615 rows), so it was the starting point and not the authority: the request shapes and the
reply framing in this document are what the URB decode shows. Counts quoted here are from that decode.

## Device and transport

Legacy Apex family (Apex 7, Apex Raw, Apex 2014): four HID collections. On Windows every profile
transfer in the capture runs on **USB interface 1** (`wIndex 1`), the collection GG opens for profile
traffic; the counts below are from that file. On the Linux box used for the 2026-10-15 hardware probe
(same keyboard family, firmware `2.4.3`) the collection that accepts the same frames is
`/dev/hidraw3`, usage page `0xFFC1`, interface 4, `max_report_size 644`, `max_input_report 1024`,
`max_output_report 64`. `/dev/hidrawN` numbers are not enumeration order, so `ssgg onboard` takes a
`--device` node first, then the **interface 1** collection of the same `vid:pid` (every profile transfer
in the capture carries `wIndex 1`), and stops at the first one that answers. That 2026-10-15 box accepted
the frames on interface 4, so a box whose frames are accepted on another collection needs an explicit
`--device /dev/hidrawN`; on Windows the interface number cannot pick a collection and
`find_hidraw_for_interface` ranks by usage page (`0xFFC0` first) there.

| Transfer | Control setup (from the capture) | Frame bytes (data stage) |
|----------|------------------------------|------------------------|
| opening chain (6 frames, once, 11 ms apart) | `SET_REPORT`, `bmRequestType 0x21`, `wValue 0x0200`, `wIndex 1`, `wLength 64` | `90 00 …`, `90 01 …`, `f4 00 …`, `6c 00 01 …`, `90 00 …`, `90 01 …`, each zero-padded to 64. The `90` polls are answered with the ASCII firmware version (`34 2e 31 36 2e 38 00…` = `4.16.8`), `f4 00` with `00 01 00…`, `6c 00 01` with nothing. Meanings unknown |
| pull | `SET_REPORT`, `wValue 0x0200`, `wIndex 1`, `wLength 64` | `85 00 <file selector u16 LE> <file offset u32 LE> <size u16 LE>`, then zeros to 64 |
| payload read | `SET_REPORT`, `wValue 0x0300`, `wIndex 1`, `wLength 642` | `83 00 <resource-buffer offset u16 LE> <size u16 LE>`, then zeros to 642 |
| payload write | `SET_REPORT`, `wValue 0x0300`, `wIndex 1`, `wLength 642` | `03 00 <buffer offset u16 LE> <size u16 LE> <payload…>` — GG's write direction, 144 transfers, not sent by `ssgg` |
| payload reply | `GET_REPORT`, `wValue 0x0300`, `wIndex 1`, `wLength 642` | 643 bytes on the wire: report-ID byte `03`, then a payload that carries the file head (`0b 00` + name) and not the requested window (see "Reply framing") |

All request shapes are output-report (`wValue 0x0200`) or feature-report (`wValue 0x0300`) `SET_REPORT`
transfers: report **number 0** in both cases, the command byte sits in the data stage, not in `wValue`.
On Linux that means `write()` for the opening-chain and pull frames (data[0] = report number 0) and the
feature-report write for the read request.


## Capture evidence

Counts in `hub-2.pcap` for the profile collection (interface 1): 385 `SET_REPORT wValue 0x0300
wLength 642` transfers (241 read requests, 144 payload writes), 241 `GET_REPORT wValue 0x0300
wLength 642` transfers (the answers), 230 `SET_REPORT wValue 0x0200 wLength 64` transfers, 12 of the
pull shape on `wIndex 0`. The 230 split by leading byte: 121 pulls (`85`), 72 lighting frames (`05`),
10 `6e`, 6 `68`, 6 `6a`, 4 `90`, 3 `89`, 3 `41`, 2 `69`, 1 `f4`, 1 `6c`, 1 `ff`. The four `90` frames,
the `f4` and the `6c` are the opening chain (see "Procedure"). The capture contains **no** 13-byte
transfer: an earlier version of this document described a 13-byte "pacing frame" with
`01 00 00 02 00 f4 01 60 00`; it is a misreading of the capture's text export (see "Reply framing"), and
no such transfer exists on the wire. The words `f4 01 60 00` do appear on the wire, as `f4 00`-family
frames zero-padded to 64 bytes.

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
and the five lighting files `0x0301`-`0x0305`. Median gap between two consecutive replies is 5 ms; one
pull with its two reads takes about 9.5 ms; the submit-to-completion latency of a payload read is
1-3 ms; the six opening-chain frames are 11 ms apart.


## Procedure

```
write output report 0, 64 bytes: 90 00 00 …  90 01 00 …  f4 00 00 …  6c 00 01 00 …  90 00 00 …  90 01 00 …
                                                                    # the opening chain, once, 11 ms apart
write output report 0, 64 bytes: 85 00 | fs_id u16 | 0 u32 | 2 u16   # schema probe of the file head
write feature report 0, 642 bytes: 83 00 | 0 u16 | 2 u16
GET_REPORT wValue 0x0300 wLength 642 -> the file bytes behind the report-ID byte
for each chunk c in 0..12:
    write output report 0, 64 bytes: 85 00 | fs_id u16 | (c * 1024) u32 | 1024 u16   # fill the buffer
    read the pull answer (the capture carries it as an input report)
    for buffer_offset in {0, 512}:
        write feature report 0, 642 bytes: 83 00 | buffer_offset u16 | 512 u16
        GET_REPORT wValue 0x0300 wLength 642 -> the file bytes behind the report-ID byte
    sleep 11 ms
```

12 pulls of 1024 bytes cover 12288 bytes, enough for the 11648-byte schema A payload and the
11696-byte schema B payload. A schema-only read is the opening chain, one 2-byte pull of the file head
and one 2-byte read. `ssgg onboard plan` prints the sequence, `ssgg onboard schema` runs the 2-byte
probe (with `--sweep` it probes several reply framings), `ssgg onboard read --yes` runs the whole read.
The reply framing length is `--reply-header` (default 0, the offset the capture's replies show); skip
the chain itself with `--no-init-chain`.


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

## Reply framing (measured, and the reason for `--reply-header`)

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
window, which is why `ssgg onboard` reads the schema probe and assembles payloads only as a labelled
experiment: reassembling GG's 24 reads of file `0x0201` with framing 0 gives an image whose schema word
reads `0x000B` and whose name reads `Config 1`, with no checksum word at the offsets the schema A and
schema B layouts predict (CRC-32/STM32 over the 11648-byte and 11696-byte regions, as u32 and as u16
words, matches nothing). `decode` prints stored and computed checksum words side by side and never claims
a profile is verified.

An earlier version of this section claimed all 241 answers start with 20 fixed bytes
(`ff ff 00 00 00 00 08 00 01 02 00 07 00 80 02 82 02 00 00 03`), that the report-ID byte of the answers
is `B5`, `B6` or `B7`, that the name of slot 0 sits at framing offset 18, and that slots 2-5 read as
`Config 2`, `Config 3`, `Config 4`. None of that is on the wire: `summary.md` and `reports.csv` are text
exports that prefix every report row with the capture's own USBPcap/usbmon header
(`80 02 00 00 00 00 00 00 ff ff …`), and `reports.csv` lists responses only, so it cannot show request
shapes at all. Re-derive anything here from the raw pcaps, which are the capture's source of truth.

The framing stays a CLI knob because the keyboard this lands on has not been measured: the Apex 7
accepted 6 on 2026-10-15, and 5 suits a keyboard that echoes the request header. `ssgg onboard schema
--sweep` probes framings `0, 1, 2, 5, 6, 14, 19, 20, 21, 36` on one file and prints which yield a schema
word the decoder knows; `ssgg onboard read --reply-header N` assembles with that anchor.

## Safety

Read-only code. `ssgg onboard` writes nothing to the keyboard: the push command (`0x05`) is named in
`src/onboard_profile.rs` for completeness and nothing sends it. Do not add profile writes: the
profile blob contains blocks this layout does not explain, and writing a synthetic blob can put the
keyboard in a state GG itself cannot repair. `read` and `schema` require `--yes`; `plan` and
`decode` never touch a device.

## What is not settled

- Where this platform keeps the profile checksum. GG's assembled image ends in zeros at the offsets
  the schema A and schema B layouts predict, and the CRC-32/STM32 of every region candidate matches
  none of them. The 2022/2023 profile blobs use a word at blob offset 12320 (see
  [apex-pro-tkl-2023-gg-capture.md](apex-pro-tkl-2023-gg-capture.md)); nothing in this capture says the
  legacy family does the same.
- What `f4 00` and `6c 00 01` do. They are part of the opening chain, `f4 00` answers `00 01 00 …` and
  `6c 00 01` is not answered at all; nothing here says which of the six frames a keyboard requires, and
  `ssgg onboard read --no-init-chain` exists to measure that.
- GG's write direction (`03 00 <offset> <size> <payload>`, 144 transfers in the capture): not used
  here, and there is no read-back command known for this family, so `ssgg onboard` writes nothing.
- The remaining `wValue 0x0200` frames of the capture (`0x6e`, `0x68`, `0x6a`, `0x89`, `0x41`, `0x69`,
  `0xff`) and the 57-byte lighting block layout, which is copied from the engine decoder and unverified.
- Whether the read-back can carry a window at all. Every answer in the capture is the file head plus
  zeros; no capture in this repo shows a payload-read reply that changes with the requested offset.
- The Linux framing of the read-back. GG asks report number 0 on interface 1 in both directions; the run
  of 2026-10-15 asked the command byte as the report number on interfaces 0-4 (`wValue 0x0290`, `0x0285`,
  `0x0383`) and issued **no read-back at all**. The report number is fixed in code since 2026-10-17 (every
  buffer now starts with report number 0, `READ_REPLY_ID = 0x00`); what remains unmeasured is asking the
  read-back of the collection that declares it - interface 1, `0xFFC0` - and whether a 642-byte control-IN
  transfer with a data stage appears there (the `--debug` log distinguishes "the request failed" from "the
  transfer answered with the wrong length").

## Windows GG vs Linux `ssgg onboard read`, same keyboard (measured 2026-10-15)

Both sides re-derived from raw URBs: the engine's own read (`ssgg-capture-1612-20261015-004400/dev.pcap`
and `hub-2.pcap`, the same session, 13:44:21-13:46:21, USBPcap linktype 152) and `ssgg onboard read`
(`linux-read.pcapng`, 22:03:17.904 -> 22:03:18.930, 1.026 s, 1428 URBs, usbmon linktype 220, bus 1
device 24 = the `0x1612` control interface, 612 control submits and 612 completes, 204 interrupt URBs).

| Item | Windows GG | Linux `ssgg onboard read` |
|------|------------|---------------------------|
| Opening chain | six 64-byte output reports on interface 1 (`90 00`, `90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01`), 8-14 ms apart, median 11 ms | same six frames, 5-6 ms apart, **sprayed over interfaces 0-4** (340 `90*` writes = 68 per interface, 17 `f4 00` and 17 `6c 00 01` on each) |
| Report ID / lengths | report ID 0 everywhere: `wValue 0x0200` wLength 64 for output frames, `wValue 0x0300` wLength 642 for the 642-byte frames | report ID = command byte: `wValue 0x0290`/`0x02f4`/`0x026c` wLength 101, `0x0285` wLength 102, `0x0383` wLength 1603 (the report-ID byte is counted twice, so every frame declares one byte more than GG's and carries 100/101/1602 payload bytes) |
| Pull `85` | 121 transfers, header `85 00 01 02` with a u32 at bytes 4-7 stepping 0, 4, 8, ... 44 | 85 transfers (17 x 5 interfaces), 7 distinct payloads, that word stays 0 |
| Read `83` | 241 requests in three shapes: `83 00 00 00 02 00`, `83 00 00 02 00 02 00`, `83 00 00 00 00 02 00` (121/120: file 1 window 2 and file 2 window 2, read over and over) | 17 requests, **all byte-identical** `83 00 00 00 02 00 00 00 00 00 00 00` — the file and window words never change |
| **Read-back** | **241 GET_REPORT control-IN transfers `a1 01 wValue 0x0300 wIndex 1 wLength 642`**, one per read request, each answered by a 643-byte control-IN data stage (report-ID byte `03` + 642 payload bytes; 1..513 non-zero bytes per reply, 89% of its u32 words are zero). The name-bearing reply is `03 0b 00 43 6f 6e 66 69 67 20 31` = schema word `0x000B` + `Config 1` | **no GET_REPORT transfer at all**; every one of the 612 control completes carries a 0-byte data stage. `ssgg` polls the interrupt endpoint `0x82` (102 polls) and receives only 64-byte status reports: `34 2e 31 36 2e 38` ("4.16.8") x 68, `00 01` x 17, `85 01` x 17 |
| Chain answers | `90 00`/`90 01` -> "4.16.8", `f4 00` -> `00 01 00 …`, `6c 00 01` -> nothing | identical, byte for byte |
| Other traffic in the window | 144 `03` 642-byte profile writes, 72 `05` lighting frames | none (read-only run) |

The point that decides everything: **the window is fetched with a GET_REPORT of the feature report**
(`bmRequestType 0xa1`, `bRequest 0x01`, `wValue 0x0300`, `wIndex 1`, `wLength 642`) issued right after
the `83` request, on the same interface. `ssgg onboard read` never issues it, so its capture has nothing
to reassemble: the request is accepted, the answers it collects are the 64-byte interrupt status reports,
and no control transfer in its own capture carries data.

The kernel strips the report-ID byte from a feature-report reply, so the payload starts at buffer offset 0,
which is framing 0 and matches the shape above; the length to ask for is **642** (643 is the request that
never completes, see the collection map below).

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

Our run's read-back never reached the wire: the only control-IN transfers in its capture are 18
`GET_DESCRIPTOR`s, and no 642-byte request of any kind appears (its feature-report requests are 1603-byte
`SET_REPORT` writes with `wValue 0x0383`). Record the answer of the read-back with `--debug` before
changing the size: on 2026-10-15 a 643-byte request produced a 5-second `ETIMEDOUT`, which means a
transfer was submitted then, and the 2026-10-15 run at 22:03 (642) produced no transfer at all.

## Linux collection map (measured 2026-10-15)

From the kernel's hidraw `uevent` files for a wired Apex 7 (firmware `2.4.3`). Each USB interface is
its own hidraw node, and the profile-capable collection is the vendor page `0xFFC1`:

| Node | Interface | Usage page | Max feature | Max input | Max output | Frames accepted |
|------|-----------|------------|-------------|-----------|------------|---------------|
| `hidraw3` | 4 | `0xFFC1` | 644 | 1024 | 64 | opening-chain frame, pull, 643-byte read request |
| `hidraw2` | 1 | `0xFFC0` | not captured | — | — | `EPIPE` on the chain write |
| `hidraw4`, `hidraw5` | 2, 3 | `0x0001`, `0x000C` | — | — | — | `EPIPE` on the chain write |

The reply is read as a feature report on the same node, with a buffer of **642** bytes: the kernel
passes that number to the device as `wLength`, and the device's report is 642 bytes. Asking for 643
(the report-ID byte counted twice) makes the control transfer never complete, which is the
`Connection timed out (os error 110)` recorded on 2026-10-15 after the read request was accepted. The
64-byte input reports this collection carries are poll and status reports (report IDs `0x0B`, `0x12`);
the collector skips them and keeps the interrupt endpoint as a fallback reply channel, read on the node
(see "Input reports: the node must be open when the report arrives").

## Input reports: the node must be open when the report arrives (measured 2026-10-16)

`usbhid` polls a hidraw collection's interrupt endpoint only while a descriptor on that minor is open. In
`linux-read.pcapng` (the 1428-packet Linux run capture; measurements below are from that file - only a 66-packet
excerpt of a related run, sha256 `7f2fa616742aaa6c5a746303e6b1e1234aee44188dec58b7f34fdd2c078c2f9d`, was still in the
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
the 2026-10-15 run's `EAGAIN` came from a `read(2)` issued after its `poll` had timed out. The read path was switched
briefly to one descriptor held open for the whole session, which is what the endpoint-polling rule above argues for,
and reverted 2026-10-17 back to per-call opens: the gap behaviour this section measures is therefore **live in the
current code** and unverified on hardware.

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
`read_feature_report_raw`, `write_output_report_raw` and `read_input_report_raw` are those four calls;
`apex_legacy_onboard` uses them for frames, payload reads and endpoint polls, `Keyboard::send_feature` for the Apex
2023 lighting path. Two consequences worth knowing: `HIDIOCSOUTPUT` puts output frames on the control port as
`SET_REPORT wValue 0x0200` - the shape the reference capture shows - where the raw `write(2)` this replaces went to an
OUT endpoint the profile collection does not have; and `HidApi::new` enumerates the device tree per call. Nothing
here is confirmed on hardware.

Measured 2026-10-16 on the usbmon capture of a full `ssgg onboard read` run (`linux-read.pcapng`, 1428 URBs,
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
