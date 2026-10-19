# Apex 7 on-board profile write

`[EXPERIMENTAL]` - every byte here is reverse-engineered, and **nothing of this write has been sent to a
keyboard**. The frames are the Windows capture's, the shapes are the engine's own descriptors, and
the implementation is compared with the capture in software only. Command-code status:
`.claude/rules/experimental-protocol.md`; the read side:
[apex-7-onboard-profile-read.md](apex-7-onboard-profile-read.md).

The capture: **GG write capture in the repo root** (untracked, 102,380 bytes, sha256
<digest recorded outside the repo>, 440 pcapng EP packets over 11.34 s; USBPcap
header of 28 bytes, `packet[0]` is its length) - a Windows capture of official GG writing a profile. Its write
session is decoded by `scripts/gg-write-frames.py` into
`tests/fixtures/onboard-write-apex7.manifest.txt` (122 frames).

The decode is pinned by the capture's own content. Reassembling the buffer writes and pushes gives a
40,960-byte file region whose first 11,696 bytes are a schema `b` payload named `Config 1` that
`ssgg onboard decode` accepts with a **matching** checksum (`0EB1C3D1`, stored and computed equal), and that
differs from the hardware read of the same slot (`config_1.json`, checksum `AE57B9DC`) in five
bytes: one binding byte at 791 and the four checksum bytes. The frame ONBOARD 1.6 names
(`05 00 01 …`) is in it - frame 37 of the profile session.

`ONBOARD.md` quotes a capture kit (GG write capture, 7.7 MB, 2150
transfers) for the same session; that kit is **not in the repo** - only its summary text is. Its profile
session has the same shape this file shows: 1 open, 80 buffer writes, 40 pushes, `eb 01`, answered by `88 01`
and 40 `05 01`.

## The two write sessions

| t (s) | file | frames | shape |
|-------|------|--------|-------|
| 0.000-0.847 | `0x0201` = profile 1 (`Config 1`) | 122 | `88 00 01 02` open, 40 windows, `eb 01` commit |
| 0.935-1.523 | `0x0301` = lighting file 1 | 40 | `88 00 01 03` open, 12 windows, `89 01` deploy (profile 1), `41 00` volatile, `74 00 01` OLED, **no commit** |

A window is two feature writes of 512 bytes into the resource buffer (buffer offsets `0` and `512`) followed by
one push of the whole 1024-byte buffer into the file:

```
88 00 | file id u16 LE                                        # output report, 64 wire bytes
03 00 | buffer offset u16 LE | size u16 LE (=512) | 512 payload bytes   # feature report, 642 wire bytes
03 00 | 512            | 512            | 512 payload bytes
05 00 | file id u16 LE | file offset u32 LE | size u16 LE (=1024)       # output report, 64 wire bytes
eb 01                                                    # commit after the last window
```

The profile session pushes at file offsets `0, 1024, 2048, …, 39936` - 40 windows, **40,960 bytes of file
region**, which is the same region the read pulls (§`OnboardPullCommand` of the read note: 40 fills, offsets
`0..=0x9C00`). The capture's "250 pushes reaching `0x9C00`" is this geometry (`0x9C00` = 39,936).
The lighting session is the same shape at 12 windows (12,288 bytes, the size of the lighting files).

**What GG writes into the 40,960-byte region** (reassembled from the capture's buffer writes and pushes, 2026-10-20):
11,696 bytes of a schema `b` profile payload at `0..11695` (`0b 00`, name `Config 1`, checksum word `D1 C3 B1 0E`
at 11,692) and **zeros for the rest** - 547 non-zero bytes in the region, the last one at 11,695. So "the engine
writes more than it reads" is the region, not new content: the content is the payload, and GG pads the region
with zeros.

`ssgg onboard decode` over that reassembled region decodes it cleanly and reports **stored = computed =
`0EB1C3D1`, match: true** - GG's own write image satisfies the same checksum rule the read side verifies.

That payload and the encoder's payload for `config_1.json` (the read of the same slot) differ in
**exactly five bytes**: byte **791** (`00` in GG's image, `04` in the read's - the first key code of entry 11 of
the button-meta mapping table, payload offsets `735..1375`, 5-byte entries) and the four checksum bytes at
11,692, each the CRC-32/STM32 of its own payload. One binding was removed between the read and GG's write.
The lighting session's 12,288-byte region carries 752 non-zero bytes, all inside `0..2879`, with the word
`62 17 ee 25` at 1018..1021 (the end of the first 1024-byte window).

## Answers and pacing (measured 2026-10-20)

The capture carries **162 `SET_REPORT` frames** and **55 interrupt-IN reports**. Nothing else: no firmware
polls, no driver-mode frames, no `GET_REPORT`, no idle input-report stream - this file is the two write
sessions and the frames around them.

| SET frame | count | answered? |
|-----------|-------|-----------|
| `03 00` buffer write | 104 (80 profile + 24 lighting) | no |
| `05 00` push | 52 (40 profile + 12 lighting) | yes: `05 01`, 52 of 52 |
| `88 00` file open | 2 (one per session) | yes: `88 01`, 2 of 2 |
| `eb 01` commit | 1 (profile session) | no |
| `89 01` deploy (LoadProfile) | 1 | no |
| `41 00` volatile, `74 00` OLED | 1 each | no |

| Event | Latency |
|-------|---------|
| profile `88 00` open -> `88 01` | 305 ms (the longest gap of the run) |
| lighting `88 00` open -> `88 01` | 90 ms |
| `05 00` push -> `05 01` | 5-6 ms, median 5 ms, 52 of 52 |
| gap between two consecutive SET frames | median 3 ms, 90th percentile 7.5 ms, max 307 ms |
| profile open -> commit | 847 ms |
| deploy frame -> next frame | 301 ms |

Every answer carries the status byte `01` behind its report id. `ssgg` paces its write at `GG_GAP_MS` = 5 ms,
the capture's median push-answer latency, which is above its median inter-frame gap of 3 ms.

**Endpoints**: the `05 01` / `88 01` answers arrive on the profile collection's interrupt endpoint - the
collection at `2.2` of the Apex 7 composite device, `0x82`, the endpoint the read path drains too.
Keystrokes come from the keyboard collection on the other endpoint (stated by the operator 2026-10-20, and
consistent with the read capture: 293 interrupt reports against 750 payload reads, 698 of them inside 40 ms after a
payload read, which is key chatter) - so a write path that drains this endpoint is not wading through key reports,
and its ack
filter only has to skip protocol reports (the `01 00 …` ones).

`ssgg` does not consume that stream for its acks: `Apex7::await_write_ack` asks for the **feature report with the
id the ack carries** (`read_profile_feature_packet(0x88)` for the open, `(0x05)` for the push), the same primitive
the read's payload replies use. That is the engine's own way - its resource read is
`read-report(DEVICE_FEATURE, resource_report_id, max-usb-timeout)` - and it means a write never reads a keyboard
report. `[EXPERIMENTAL]`: GET_REPORT is the untested half of this path, like the rest of the write.

## Engine side

`base_apex_legacy.lisp:1993-2022` describes this run: `write-onboard-profile-ram-0x600` (`:1995`) opens the
file with `macho-file-open-payload`, loops over the resource buffer and, per window, sends
`macho-write-payload 0` then `macho-write-payload chunk` (`:2012-2016`, the two 512-byte writes) and
`macho-push-payload` (`:2017-2019`, the 1024-byte push), then `0xEB` with a 1-based profile id (`:2022`) plus
an input read. Ghidra, `SSEHIDDevice.dll`: `WriteOnboardProfile` at `0x227a20`, with the buffer-write and
push paths behind it; `WriteOnboardProfileWithCallback` at `0x2297a0`.

`load-profile-with-pause` (`base_apex_legacy.lisp:2039-2045`) is the deploy: `0x89` with a 1-based profile id,
then a 250 ms pause. The capture shows the deploy frame (`89 01`, profile 1) and the pause on the wire: 301 ms
from that frame to the next frame of the session.

The engine's own pacing is a **device setting, written with `0x6C`**: `hidconfig` writes 12 bytes read as three
u32 - `time-between-commands`, `file-access`, `max-usb-timeout` - whose boilerplate values are 10, `FA_READWRITE`
and 500, with the engine's comment "HID Configuration - 10 ms is also the default in devicelib"
(`standard-boilerplate.lisp:8-10, 24`). The profile write spec writes that frame twice in front of the file open,
with `5000` and then with `max-usb-timeout` as the third word, the read spec writes it in front of the pulls, and
every one of them resets it in `api-finally` with `{"hidconfig": []}` (`apex_7.lisp:217, 224, 251, 253, 270`;
`base_apex_legacy.lisp:546, 1514, 2008, 2064`). The `6c 00 01 …` frame in the captures is this write - see
`apex-7-onboard-profile-read.md` §"What paces the read, and what GG paces with `6c`" for the decode and for the
other, 4-byte `0x6C` shape the same family uses for driver mode.

## What `ssgg` does

`Apex7::write_profile` (via `Keyboard::write_onboard_profile`) runs the plan; the geometry, the frames and the
pacing come from `apex_legacy_profile::write_plan` (`OnboardWritePlan`, `OnboardWriteFrame`), and the frames go
out through the typed commands `OnboardFileOpenCommand` (`0x88`), `OnboardBufferWriteCommand` (`0x03`, feature
report), `OnboardPushCommand` (`0x05`), `OnboardCommitCommand` (`0xEB`) and `OnboardLoadProfileCommand`
(`0x89`), all built in `src/devices/hid_reports.rs`.

- The payload goes through the existing encoder (`encode_json`): decode report → payload, checksum recomputed
  at the payload tail (`apex_legacy_profile::encode_json`). `--bin` takes raw payload bytes as they are.
- The payload is zero-padded to the 40,960-byte file region - the padding GG's own session carries - and the
  40 windows are built over that image. The payload's checksum word is written **before** the padding, at
  11,692, as GG's does; no checksum is computed over the padded region.
- Pacing is `GG_GAP_MS` (5 ms) between frames, the capture's median.
- The write waits the answers the capture shows: `88 01` after the open, `05 01` after every push; it does not
  wait on writes or on the commit, which are not answered. A status byte other than `01`, and a frame that is
  not answered, stop the write. The wait is `Apex7::await_write_ack`: a poll for the **report with the id the ack
  carries** (`0x88` for the open, `0x05` for the push, the codec's `write_ack_report_ids`), on the profile
  collection with the read path's own 50 ms poll (`REPLY_POLL_MS`) - one ack primitive for the whole transfer, and
  never a keyboard input report, which is where key events come from (§Endpoints above, and the read capture's 293
  interrupt reports against 750 payload reads).
- The lighting files, driver mode, OLED, volatile, firmware polls and pacing frames are not sent. The write
  touches one profile file and, at the end, deploys that slot (`89 <slot + 1>` right after the commit - the
  engine sleeps 250 ms behind the deploy frame, the write path does not, ONBOARD step 1.4).
- There is no confirmation gate: `onboard write` and `onboard load` send when asked (`--yes` was dropped
  2026-10-20; asking for the command is the authorisation). `--dry-run` is the mode that sends nothing.

CLI: `ssgg onboard write <report.json> --profile N [--bin <file>] [--dry-run]` and
`ssgg onboard load --profile N`; `--profile N` is GG's 1-based config number, `--dry-run` prints the plan and
writes nothing (ONBOARD steps 1, 1.5, 2).

## Acceptance (ONBOARD 1.6), measured 2026-10-20 in software

`ssgg onboard write --dry-run --profile 1 config_1.json` produces **122 frames**, the capture's profile session
frame for frame in report kind, wire length, order and content:

- 120 of 122 frames are byte-identical to the capture.
- Frame 3 (window 0's write at buffer offset 512) differs in **one byte**: GG `00`, ours `04`, at wire index
  285, which is payload byte **791** - the first key code of entry 11 of the button-meta mapping table
  (`config_1.json`, `button_meta_mappings[11].key_codes = "04 00 00 00"`). It is a payload content difference,
  not a framing difference: the encoder writes what its input report says.
- Frame 35 (window 11's write at buffer offset 0) differs in **four bytes** at wire 434..437: the payload
  checksum word. GG's is `D1 C3 B1 0E` (`0x0EB1C3D1`), ours is `DC B9 57 AE` (`0xAE57B9DC`). Both are the
  CRC-32/STM32 of their own payload; they differ because the payloads differ at byte 791.

`tests/fixtures/onboard-write-apex7.manifest.txt` (frame index, kind, wire length, FNV-1a-64 digest,
head bytes, and `full=` data stage for frames 3 and 35) is the capture-derived fixture;
`apex_legacy_profile::tests::the_write_plan_of_the_read_report_is_the_capture_write_sequence` compares the plan
against it frame by frame - report kind, wire length, head bytes, serializability, the 120 byte-identical
frames, and the exact five payload bytes named above. The acceptance frame (window 11's push, ONBOARD 1.6's
"that frame" - that number counts the capture's control frames from the session start, its bytes are
`05 00 01 …`, file offset `0x2C00`) is asserted byte-identical.

**Availability of the capture (2026-10-20):** GG write capture is not in the repository and was not kept on disk
after the decode - the fixture is the durable record, and the checksum number above (`0EB1C3D1`) is recorded in the
fixture header and asserted by the test. The decode line (`Config 1`, schema `0x000B`) and the lighting-session
measurements were read from the reassembled image in the session that built the fixture. Anyone who re-captures the
same session should regenerate the manifest from the capture and diff it against the fixture.

## Not settled

- **No read-back.** A write cannot be verified from the device: the read path is the only way to see a written
  profile, and a written slot has never been read back from hardware.
- **Nothing of this has been sent to a keyboard.** Not one frame of the write plan - not the open, not a
  buffer write, not a push, not the commit, not the deploy - has been replayed on hardware.
- The status byte `01` is what every write answer of the capture carries; what it means, and what a refusal
  looks like, is not known. The 305 ms latency of the open answer is measured, its reason is not.
- The lighting file's write (12,288-byte region, deployed without a commit) carries a 4-byte word at region end
  (`62 17 EE 25`) that is not CRC-32/STM32 over the region as u32 words, as u16 words, or over the full
  12,288 bytes. Its algorithm is open, and `ssgg` does not write lighting files.
- The `eb` commit's 1-based slot byte and the `89` deploy's are the engine's (`base_apex_legacy.lisp:2022`,
  `:2039-2045`); what the keyboard does with the commit beyond answering nothing is unmeasured.
- GG's session overhead in this capture (driver mode `6c`, `f4 00`, `01 00 00 02 00`, `90` polls, `69`,
  `41 01`) is sent by this implementation nowhere, and the read runs without it.
