# Apex 7 on-board profile read

Source: `ssgg-capture-1612-20261015-004400` (Windows USBPcap capture of SteelSeries GG talking to an
Apex 7, PID `0x1612`, 2026-10-15, firmware `4.16.8`). Every item below is `[EXPERIMENTAL]`: nothing of
it has been replayed from `ssgg` on real hardware. Code: `src/onboard_profile.rs` (wire format, decode),
`src/devices/keyboards/apex_legacy_onboard.rs` (transfers), `ssgg onboard` (`src/cli.rs`,
`src/main.rs`), all behind the `onboard` feature.

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
`--device` node first, then every other collection of the same `vid:pid`, ordered by interface number,
and stops at the first one that answers.

| Transfer | Control setup (from the capture) | Frame bytes (data stage) |
|----------|------------------------------|------------------------|
| pacing frame | `SET_REPORT`, `bmRequestType 0x21`, `wValue 0x0200`, `wIndex 1`, `wLength 13` | `01 00 00 02 00 f4 01 60 00`: pacing `u8` 1 ms, `u16` 0, file access `u16` 2, USB timeout `u16` 500 ms, trailing `u16` 96 |
| pull | `SET_REPORT`, `wValue 0x0200`, `wIndex 1`, `wLength 64` | `85 00 <file selector u16 LE> <file offset u32 LE> <size u16 LE>`, then zeros to 64 |
| payload read | `SET_REPORT`, `wValue 0x0300`, `wIndex 1`, `wLength 642` | `83 00 <resource-buffer offset u16 LE> <size u16 LE>`, then zeros to 642 |
| payload write | `SET_REPORT`, `wValue 0x0300`, `wIndex 1`, `wLength 642` | `03 00 <buffer offset u16 LE> <size u16 LE> <payload…>` — GG's write direction, 144 transfers, not sent by `ssgg` |
| payload reply | `GET_REPORT`, `wValue 0x0300`, `wIndex 1`, `wLength 642` | 642 bytes: report-ID byte `B5`/`B6`/`B7`, then 20 framing bytes, then the file bytes (see "Reply framing") |

All three request shapes are output-report (`wValue 0x0200`) or feature-report (`wValue 0x0300`)
`SET_REPORT` transfers: report **number 0** in both cases, the command byte sits in the data stage, not
in `wValue`. On Linux that means `write()` for the pacing and pull frames (data[0] = report number 0)
and the feature-report write for the read request.


## Capture evidence

Counts in `hub-2.pcap` for the profile collection (interface 1): 385 `SET_REPORT wValue 0x0300
wLength 642` transfers (241 read requests, 144 payload writes), 241 `GET_REPORT wValue 0x0300
wLength 642` transfers (the answers), 230 `SET_REPORT wValue 0x0200 wLength 64` transfers (121 pulls,
the rest pacing and other traffic on the same collection), 12 of the pull shape on `wIndex 0`.

The read requests are exactly three shapes:

| buffer offset | size | count |
|---------------|------|-------|
| 0 | 2 | 1 (the schema probe of the profile file of slot 0) |
| 0 | 512 | 120 |
| 512 | 512 | 120 |

So a 1024-byte pull is drained by two 512-byte reads, and GG never asks for more than 512 bytes per
read. The 121 pulls are: one head probe of file `0x0201` at offset 0 with size 0, then 12 pulls of
1024 bytes per file at offsets `0, 1024, …, 11264` for each of the five profile files `0x0201`-`0x0205`
and the five lighting files `0x0301`-`0x0305`. Median gap between two consecutive replies is 5 ms; one
pull with its two reads takes about 9.5 ms; the submit-to-completion latency of a payload read is
1-3 ms.


## Procedure

```
write output report 0, 13 bytes: 00 | 01 00 00 02 00 f4 01 60 00          # pacing frame, once
for each chunk c in 0..12:
    write output report 0, 64 bytes: 85 00 | fs_id u16 | (c * 1024) u32 | 1024 u16   # fill the buffer
    read the pull answer (the capture carries it as an input report)
    for buffer_offset in {0, 512}:
        write feature report 0, 642 bytes: 83 00 | buffer_offset u16 | 512 u16
        GET_REPORT wValue 0x0300 wLength 642 -> file bytes behind the 20 framing bytes
    sleep 5 ms
```

12 pulls of 1024 bytes cover 12288 bytes, enough for the 11648-byte schema A payload and the
11696-byte schema B payload. A schema-only read is one 2-byte read of the file head. `ssgg onboard
plan` prints the sequence, `ssgg onboard schema` runs the 2-byte probe (with `--sweep` it probes
several reply framings), `ssgg onboard read --yes` runs the whole read. The reply framing length is
`--reply-header` (default 20, the offset the capture's probe reply shows).


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

All 241 payload answers in the capture start with the same 20 bytes:

```
ff ff 00 00 00 00 08 00 01 02 00 07 00 80 02 82 02 00 00 03
```

That block carries the file selector (`01 02` = `0x0201`), GG's pacing words (`f4 01 60 00` = 500 ms,
96) and two status bytes; it is framing, not profile data. Behind it:

- the 2-byte probe reply of file `0x0201` carries `0b 00` at framing offset 0, i.e. schema word
  `0x000B`, 11696-byte payload (schema B, the Rapid Tap layout);
- the 512-byte replies of the same file carry the profile name 16 bytes later, at reply byte 36;
- the report-ID byte of these answers is `B5`, `B6` or `B7`, never `83` (the request's report number)
  and never `03` (the write direction's report number).

Assembling GG's own reads of file `0x0201` (12 pulls, 24 replies) with the 20-byte framing gives a
12288-byte image whose schema word at offset 0 reads `0x0000`, whose profile name (`Config 1`) sits at
offset 18, whose last 4 bytes are zeros, and whose CRC-32/STM32 computed over the 11648, 11696 and
12288-byte regions (minus their last 4 bytes, as u32 and as u16 words) matches none of them. The same
holds for slots 2-5 (`Config 2`, `Config 3`, `Config 4`) and for the lighting files, which carry no
name at all. Slot 5 and lighting file 3 (`0x0205`, `0x0303`) were captured mid-transfer: their
assembled bytes contain a timestamp-shaped word where the name should be, so those two files are not
clean evidence.

That is why the reply anchor is a CLI knob. `ssgg onboard schema --sweep` probes framing lengths
`0, 5, 6, 14, 19, 20, 21, 36` on one file and prints which of them yields a schema word the decoder
knows; `ssgg onboard read --reply-header N` assembles with that anchor. `decode` prints stored and
computed checksum words side by side and never claims a profile is verified, because on the reference
keyboard GG's own read-back does not verify either.


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
- Where the name starts. The Windows engine decoder puts it at payload offset 2; GG's own read-back of
  slots 1-4 reads it at offset 18.
- The trailing `u16 96` of the pacing frame (500 ms timeout, 96 of something) and the `u16 0` before
  the file-access word.
- GG's write direction (`03 00 <offset> <size> <payload>`, 144 transfers in the capture): not used
  here, and there is no read-back command known for this family, so `ssgg onboard` writes nothing.
- The opening frames of the capture (`0x90`, `0x6c`, `0x6e`, `0x68`, `0x65`, `0x67`, `0x34`) and the
  57-byte lighting block layout, which is copied from the engine decoder and unverified.

## Linux collection map (measured 2026-10-15)

From the kernel's hidraw `uevent` files for a wired Apex 7 (firmware `2.4.3`). Each USB interface is
its own hidraw node, and the profile-capable collection is the vendor page `0xFFC1`:

| Node | Interface | Usage page | Max feature | Max input | Max output | Frames accepted |
|------|-----------|------------|-------------|-----------|------------|---------------|
| `hidraw3` | 4 | `0xFFC1` | 644 | 1024 | 64 | pacing frame, pull, 643-byte read request |
| `hidraw2` | 1 | `0xFFC0` | not captured | — | — | `EPIPE` on the pacing write |
| `hidraw4`, `hidraw5` | 2, 3 | `0x0001`, `0x000C` | — | — | — | `EPIPE` on the pacing write |

The reply is read as a feature report on the same node, with a buffer of **642** bytes: the kernel
passes that number to the device as `wLength`, and the device's report is 642 bytes. Asking for 643
(the report-ID byte counted twice) makes the control transfer never complete, which is the
`Connection timed out (os error 110)` recorded on 2026-10-15 after the read request was accepted. The
64-byte input reports this collection carries are poll and status reports (report IDs `0x0B`, `0x12`);
the collector skips them and keeps the interrupt endpoint as a fallback reply channel.
