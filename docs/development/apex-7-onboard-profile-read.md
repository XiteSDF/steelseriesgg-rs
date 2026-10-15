# Apex 7 on-board profile read

Source: capture notes (Windows USBPcap capture of SteelSeries GG talking to an
Apex 7, 2026-10-15, firmware `4.16.3`). Every item below is `[EXPERIMENTAL]`: nothing of it has been
replayed from `ssgg` on real hardware. Code: `src/onboard_profile.rs` (wire format, decode),
`src/devices/keyboards/apex_legacy_onboard.rs` (transfers), `ssgg onboard` (`src/cli.rs`,
`src/main.rs`), all behind the `onboard` feature.

The capture records control transfers only as *responses* (decoded response listing: 615 rows, report IDs
`0x83`, `0x85`, `0x03`, `0x6e`, `0x68`, `0x90`, …). GG's requests are not in the CSV, so each request
frame below is the reply-side layout turned around: same command byte, same field order, padded to the
report length GG uses. Where a request detail is not visible from a reply it is marked **inferred**.

## Device and transport

Legacy Apex family (Apex 7, Apex Raw, Apex 2014): four HID collections, and the profile
protocol lives on the control collection — the third in enumeration order, the one whose
`phys` path ends in `0x10000003`. `/dev/hidrawN` nodes are not in enumeration order, so
`ssgg onboard` ranks candidate paths (the requested node first, then a `--device vid:pid` match
through `hidapi`, then the sysfs control collection, then the remaining nodes) and stops at the first
path that answers the protocol.

| Transfer | Report | Length on the wire |
|----------|--------|--------------------|
| pacing frame | feature report `0x00` | 12 |
| pull | output report `0x85` | 65 (ID byte + 64-byte frame) |
| payload read | feature report `0x83` | 643 (ID byte + 642-byte frame) |
| pull reply | input report `0x85` | 64 |
| payload-read reply | feature report `0x83` | 642 (measured in the capture) |

## Capture evidence

In step 1 (GG starting up) the capture shows, in order:

```
85 00 01 02 00 00 00 00 00      64 bytes   pull: file 0x0201, offset 0, size 0
83 00 00 00 00 02 <636 bytes>   642 bytes  read reply: offset 0, size 2   -> schema word
85 00 01 02 00 04 00 00 00 04   64 bytes   pull: file 0x0201, offset 1024, size 1024
83 00 00 00 00 02 <636 bytes>   642 bytes  read reply: buffer offset 0,   size 512
83 00 00 02 00 02 <636 bytes>   642 bytes  read reply: buffer offset 512, size 512
... 11 more pulls at offset 2048..11264, each followed by the same two reads
85 00 02 02 ...                               same sequence for file 0x0202, then 0x0203..0x0205
85 00 01 03 ... 85 00 05 03 ...               same sequence for the lighting files 0x0301..0x0305
```

Counts in the capture: 121 pulls (1 with size 0, 120 with size 1024), 241 read replies
(1 with size 2, 120 with buffer offset 0 / size 512, 120 with buffer offset 512 / size 512),
file ids `0x0201`–`0x0205` (five profile slots) and `0x0301`–`0x0305` (five per-profile lighting
files). Median gap between two consecutive replies is 5 ms; one pull-then-two-reads cycle takes
about 9.5 ms. The capture also shows `0x03`-prefixed 642-byte replies (144 of them) — the
payload-read replies of the lighting files answer on report ID `0x03`, not `0x83`.

**Inferred request frames** (`0x83` request: `83 00 <buffer offset u16 LE> <size u16 LE>` padded to
642; `0x85` request: `85 00 <file id u16 LE> <file offset u32 LE> <size u16 LE>` padded to 64;
pacing frame: `<time_between_commands u32 LE> <file_access u32 LE> <max_usb_timeout u32 LE>` =
`01 00 00 …`). The pacing frame is sent once before the profile read.

The read reply header is `[0x83][reserved u8][buffer offset u16 LE][size u16 LE]` and the payload
follows at byte 6: 642 − 6 = 636 usable payload bytes per read, which is why GG splits a 1024-byte
pull into two 512-byte reads.

## Procedure

```
send pacing frame 01 00 00 00 | 02 00 00 00 | f4 01 00 00    (1 ms, read/write, 500 ms timeout)
for each chunk c in 0..12:
    write output report 0x85: 85 00 | fs_id u16 | (c * 1024) u32 | 1024 u16      # fill the buffer
    read input report 0x85 (64)                                                  # pull ack
    for buffer_offset in {0, 512}:
        write feature report 0x83: 83 00 | buffer_offset u16 | 512 u16
        read feature report 0x83 (642) -> payload bytes at 6..6+512 land at file_offset + buffer_offset
    sleep 5 ms
```

12 pulls × 1024 bytes cover 12288 bytes, enough for the 11648-byte profile payload. A schema-only
read is one pull of the file head with size 0 followed by one read of 2 bytes. `ssgg onboard plan`
prints this sequence; `ssgg onboard schema` runs the schema-only read; `ssgg onboard read --yes`
runs the whole read.

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

## Safety

Read-only code. `ssgg onboard` writes nothing to the keyboard: the push command (`0x05`) is named in
`src/onboard_profile.rs` for completeness and nothing sends it. Do not add profile writes: the
profile blob contains blocks this layout does not explain, and writing a synthetic blob can put the
keyboard in a state GG itself cannot repair. `read` and `schema` require `--yes`; `plan` and
`decode` never touch a device.

## Open questions

- The `0x4B` init frame of the Apex Pro TKL 2022/2023 direct path: unknown whether the legacy Apex
  family needs it, and the capture's opening frames (`0x90`, `0xf4`, `0x6c`, `0x6e`, `0x68`) are not
  decoded.
- Which of the 121 pulls GG issued per file is the size-0 pull at the file head: it precedes the
  schema read of file `0x0201`; the capture shows no size-0 pulls for the other files.
- The 260 (schema A) / 267 (schema B) payload bytes between the macro block and the checksum.
- The lighting files (`0x0301`–`0x0305`, and the shared `0x0102`): same split/read procedure,
  different reply report ID (`0x03`); payload length and layout not examined.
- Interface number for the control collection on Linux (`/dev/hidraw` does not need one; a Windows
  `hid_open` path built from a device path must use the control collection's own interface).
- Timing limits: the capture shows ~5 ms between replies; whether the keyboard tolerates more is
  untested. `--gap-ms` is exposed for exactly that reason.
- The `0x85` pull ack content: the capture shows GG reading it back as a 64-byte report; its fields
  are not decoded, so `ssgg onboard read` only records the ack's report ID and first bytes.
- Schema words of the five slots: `onboard schema` is the first thing to run on hardware; if a slot
  answers with a schema word the decode supports, the layout table above can be confirmed byte by byte.
