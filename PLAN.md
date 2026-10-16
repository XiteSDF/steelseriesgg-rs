# PLAN: next steps

Single backlog for steelseriesgg-rs. Replaces the old `PLAN.md`, `TODO.md` and `handoff.md`.
Last reviewed 2026-10-15 against `main` at `a7e5915`.

Read `AGENTS.md` (same file as `CLAUDE.md`) and `.claude/rules/experimental-protocol.md` before any code change.
Protocol facts for the Apex Pro TKL 2023 are in `AGENTS.md` ("Protocol research notes") and
`docs/development/apex-pro-tkl-2023-gg-capture.md`; this file only tracks work.

## Baseline (verified 2026-10-15, Windows)

- `cargo clippy-ci` (alias for `clippy --all-targets --locked -- -D warnings`): clean.
- `cargo test --locked`: 140 passed, 2 ignored (Windows); 141 passed, 2 ignored on Linux (WSL Ubuntu, 2026-10-15).
- Linux `cargo clippy --all-targets -- -D warnings`, also with `--features sonar`: clean (WSL, 2026-10-15).
- `cargo report future-incompatibilities --id 1`: "no reports are currently available".
- `cargo test --locked --features audio`: fails to link on Windows (no libpulse). Expected; not a code bug. CI covers
  `audio` on Linux for clippy and release build.

## How to work this list

- Items marked **[HW]** need the physical device. The owner has an Apex Pro TKL 2023 wired (`0x1628`) on Windows 11
  with SteelSeries GG installed. Other devices need an outside tester (named per item).
- Hardware steps: prepare the build and the exact commands, then ask the user to run them and report what the
  keyboard shows. Do not claim a result the user did not confirm.
- Stop GG before any `ssgg rgb` test on Windows (kill every `SteelSeries*` process); GG overwrites lighting at ~19 fps.
- Protocol labels: anything not confirmed on hardware stays `[EXPERIMENTAL]` / `[PLACEHOLDER]`. Promotion needs a
  commit naming the device model and test date.
- Run `cargo fmt --all -- --check`, `cargo clippy-ci` and `cargo test --locked` after every code change. `cfg(unix)`
  code does not compile on Windows; check it in WSL (`CARGO_TARGET_DIR` outside `/mnt/c`).

---

## P0 (onboard). Apex 7 on-board read: first hardware run on the device path

Code is in place (2026-10-17, `onboard-configs-dev`): `Apex7` in `src/devices/keyboards/apex7.rs` wraps
`GenericKeyboard` (its `HidReportBuilder` and the open `HidDevice`) and does the read's packet I/O
itself on that handle; `Keyboard::read_onboard_profile` is the device API,
`discovery::open_keyboard` returns `Apex7` for `0x1612`, `0x1614`, `0x1616`, `0x1618` - **with or without
the `onboard` feature** (2026-10-17: the device class is part of the device tree, only the read path is
feature-gated). `ssgg onboard read` runs on that device. Nothing has been replayed on hardware.

1. **[HW] Collection check first:** `ls -l the device sysfs input nodes`. The 2026-10-15
   run found the interface-1 collection (`the hidraw node`) had no `input*`, so the device manager opened
   interface 4 (`usage 0xFFC1`) — the collection with an `input*` node, not the one GG used. If interface
   1 has no `input*`, boot once with the kernel argument `usbhid.poll_descriptor_ignores_interfaces=0`
   (`sudo raspistop --bootcmd setusbhid` form from `ONBOARD.md` Step 1) and re-check.
2. **[HW] Reply framing on the device:** short reads, one per candidate framing -
   `ssgg --debug onboard read --yes --profile 0 --chunks 1 --resource-len 512 --read-len 512
   --payload-len 512 --reply-header N` for `N` in `0, 1, 2, 5, 6, 14, 19, 20, 21, 36`; the `90` poll answer
   in the log confirms the device path, and the framing whose answer carries schema `0x000B` with a
   readable name is the anchor.
3. **[HW] Read slot 0** with the winning framing (`--out /tmp/p0.bin --json /tmp/p0.json`), then slots
   `1..4`, then `--verify` and the `--gap-ms` ladder. Record the checksum line and the decoded layout as
   `ONBOARD.md` "Next hardware steps" describes.
4. **[HW] Windows:** same read through the hidapi path; confirms the interface choice for the control
   collection.

## P1. Apex Pro TKL 2023 wired (`0x1628`): finish lighting

Lighting is the only part replayed from `ssgg` so far (one solid red, 2026-10-14).

0. **[HW] `send_feature` transport test (code 2026-10-17, never measured on `0x1628`)**: `Keyboard::send_feature`
   is back on the device's own hidapi handle - the 2026-10-15 path-based call (interface-1 lookup via sysfs) was
   reverted without ever being measured. With GG stopped: `ssgg rgb color FF0000`, then `rgb color 00FF00`,
   `rgb brightness 50`, `rgb effect wave`. If the keyboard ignores every direct-mode frame, the interface-1
   reading is real: re-open interface 1 as its own collection (not the old path lookup) and record it here.

1. **[HW] Lighting test matrix**, GG stopped, `.\target\debug\ssgg.exe`:
   - `rgb color 00FF00`, `rgb color 0000FF`, `rgb color FFFFFF` (all keys, correct colour?)
   - `rgb off`
   - `rgb brightness 50` then `rgb brightness 100` (does the brightness path scale RGB for this model, or send a
     command the keyboard ignores? GG does software scaling only.)
   - `rgb effect breathing`, `rgb effect spectrum`, `rgb effect wave` (animated effects need a running loop; note
     whether the CLI returns immediately)
   - `rgb perkey ...` (read `PerKeyAction` in `src/cli.rs` for syntax): set W, A, Esc, F1, arrow keys. Esc, F-row and
     nav cluster are in `WIRED_2023_KEYS` for lighting even though they have no profile slot.
   - Capture one run with USBPcap to confirm frames still match the GG layout (filter in `AGENTS.md`).
   Fix whatever fails. If brightness sends a no-op command for `0x1628`, implement it as RGB scaling in the 2023
   direct path (matches GG behaviour).
   Code state (2026-10-15): `rgb perkey` on `0x1628` now sends the GG-layout 84-key frame with unlisted keys black
   (`set_key_colors` in `apex_pro_tkl_2023.rs`); `rgb brightness` still sends the generic `00 22 <level>` output
   report. Expect per-key to light only the named keys and the rest to go dark (`set-key`, `set-keys`,
   `set-region` and `test-matrix` all use the frame). Per-key *effects* (`set_per_key_effect`,
   `trigger_key_reactive`, `apply_per_key_effect_with_brightness`) still reach the generic `0x23` placeholder and
   `0x09` apply on direct-protocol models; route them through the frame if the effect tests fail.
2. **Extra output report before each frame.** The existing `initialize()` path sends `00 09 00 ..` as an output report
   before frames. Code comments say `0x09` makes the controller fall back to its stored profile. Check (by USBPcap
   during step 1) whether `0x1628` still sends it; if yes, test with it skipped for `0x1628` and keep whichever
   variant gives a stable frame.
3. ~~**`experimental-apex-2023` feature path does not match GG.**~~ Done 2026-10-15 (user chose removal): feature
   flag, `Apex2023DirectCommand` and the unused `smallvec` dependency removed; per-key uses the default frame path.
4. **[HW, Linux] hidraw path.** `send_feature` on Unix uses `info.path` for `0x1628`. Untested. Needs a Linux machine
   with the keyboard; record the result in the capture note.
5. **Status decision (ask the user).** After steps 1-4, ask whether to drop `[EXPERIMENTAL]` for the `0x40` lighting
   frame on `0x1628`. If yes: update `.claude/rules/experimental-protocol.md`, the code labels and the docs in one
   commit naming "Apex Pro TKL 2023 (wired, PID 0x1628)" and the test date.

## P2. Apex Pro TKL 2023 wired: actuation and profile (blocked on checksum)

`ssgg actuation` sends the experimental `0x2D` command; GG instead rewrites the 12,500-byte on-board profile.
Writing the profile is blocked on the checksum at blob offset 12320..12323.

1. ~~**Solve the checksum.**~~ Solved 2026-10-15: STM32 CRC-32 over `0..12320` as LE words, 26/26 blobs match
   (capture note, "Checksum"). Original notes follow. Inputs: `the artifact kit's blobs.bin` (13 blobs) and `blobs3.bin`
   (19 blobs), 12,500 bytes each. Already ruled out: 10 common CRC32 polynomials (all init/xor/reflection),
   Adler-32, byte sum, over ranges starting 0/1/2/4/9 and ending 12320/12324/12500. Next approaches, in order:
   - Static RE of `C:\Program Files\SteelSeries\GG\apps\engine\SteelSeriesEngine.exe` (Go) and `SSEdevice.dll`:
     find the profile-serialize code (search strings near "profile", "checksum", "crc", table constants such as
     CRC polynomials or hash IVs) and read the function directly.
   - Fit the function from data: XOR pairs of blobs that differ by one byte to test linearity (linear over GF(2)
     means a CRC-like function with a non-standard polynomial or range).
   - Ask the reporter of issue #280 for access to their decrypted `.edevice` files (their linked repo returns 404).
2. **Partly done 2026-10-15:** `keyboards/apex_pro_tkl_2023_profile.rs` has the checksum, `set_actuation` (rejects
   unknown mm values and keys without a slot) and the 25-chunk split, all unit-tested. Not wired to any command:
   without a read-back (step 3) there is no current blob to modify. Original task: implement a profile read-modify-write for actuation on `0x1628` (chunk
   header `03 01 80 | len | offset | 00 00`, 25 chunks), using `WIRED_2023_PROFILE_SLOTS` and the four known raw
   actuation values. Unknown mm values must be rejected, not interpolated, until more points are captured. Get
   explicit user approval before the first write to the keyboard.
3. **Capture GG startup** (USBPcap started before launching GG) to get the init sequence and see whether GG reads
   the profile back. A read-back command would allow safe read-modify-write and replaces the old
   "discover an actuation read-back command" task (`src/bin/discover_actuation.rs`).
4. **Decode remaining commands** while capturing: `b1`, `b2`, `34`, `38 66`, `38 67`; the profile tables at 1080,
   1224, 1590; where `second_actuation_thresholds` is stored. Record findings in the capture note.

## P3. Apex 3 TKL (`0x1622`) RGB not applied (issues #173, #257)

The code already sends `0x21` with selector `0xFF` and `R G B` per zone (OpenRGB 8-zone layout). Open differences:

1. ~~**Zone count.**~~ Done 2026-10-15 (Underglow zone dropped from the invented 9-zone map). Code used 9 (`zone_count_for_product_id`, `src/devices/mod.rs`; `zone_mapping.rs` around line 313).
   SteelSeries advertises 8 zones and OpenRGB models 8. Change `APEX_3_TKL` to 8 zones in both places and update
   the tests that assert the count.
2. ~~**Brightness byte.**~~ Done 2026-10-15: `Apex8ZoneBrightnessCommand` (`00 23 <0x00-0x10>`), used by
   `Apex3Tkl::set_brightness`, `[EXPERIMENTAL]`. Code has `CommandCode::Brightness = 0x22`; OpenRGB's 8-zone controller uses `0x22` = rainbow
   wave and `0x23` = brightness (`0x00`-`0x10`). Do not change globally; if needed, add an Apex 3 TKL-specific
   brightness via `HidReportBuilder`, labelled `[EXPERIMENTAL]`.
3. **[HW, outside tester]** `yann-morin-1998` offered to test on #257. Push a branch, post exact commands
   (`ssgg -d rgb color FF0000`, `ssgg rgb off`, `ssgg rgb brightness 50`) and ask for debug logs. Confirm with the
   user before posting on GitHub.
4. #257 duplicates #173; close #257 in favour of #173 once the test result is in (ask the user first).

## P4. Apex Pro original (`0x1610`) RGB does nothing on Linux (issue #283)

Reporter (NixOS) sees successful HID writes but no change. `0x1610` uses the generic 1-zone path.

Done 2026-10-15 (steps 1-2, code for 3): ssgg sent a 65-byte `0x21` output report; OpenRGB uses a 643-byte feature
report `00 3A N [usage R G B] xN` on interface 1 with no init. `0x1610` now goes through `ApexProTkl2023` with packet
ID `0x3A`, the 112-key table, and Unix hidraw interface 1. Remaining: step 3's tester request (ask first). Which
collection Windows discovery picks for `0x1610` is unknown.

1. Check which interface `ssgg` writes to for `0x1610` (log shows interfaces 0-4 enumerated) and which report the
   generic path sends.
2. Compare with OpenRGB's SteelSeries Apex controller and `FrankGrimm/apex7tkl_linux` / `AstroSnail/apexctl` for the
   Apex Pro report format and interface. Record the source for any byte layout you adopt.
3. Implement behind an `[EXPERIMENTAL]` label, then ask the reporter to test (confirm with the user before posting).

## P5. Device registration (issue #276)

Done 2026-10-15 for `0x22A1` and `0x0616` (`is_detect_only` makes `open_keyboard` / `open_headset` refuse them;
`first_device_of_type` skips them). Mouse skipped by user decision. Original task: register name and device type only, no protocol claims: `0x1858` Aerox 9 Wireless (mouse), `0x22A1` Arctis Nova 7
(headset), `0x0616` Apex M750 (keyboard). Use the `new-device` skill. Check first whether a mouse `DeviceType`
exists; if not, ask the user before adding one. Unknown devices must stay detect-only (no RGB writes).

## P6. GitHub close-out (ask before any GitHub write)

- #211 (Arctis Nova Pro Omni `0x2290`) and #165 (Apex Pro TKL Wireless 2023 `0x1630`): registered in code with
  tests. Ask the reporters to confirm on hardware, then close.
- #280: repo link is 404. No longer needed for the checksum; decrypted `.edevice` files would still help with P2.4.
- #277 (Sonar deeplink config import): format researched 2026-10-15, see `database-schemas.md` ("Shared config
  links"). Link = base64 CDN URL to the config JSON. Still unknown: how to create a config in Sonar (body of
  `POST /configs` or the `retrieveFromUrlHash` hash form). Scope after that is known.

## P7. Maintenance (low priority)

- CodeQL default setup fails on every push: its `c-cpp` job finds no sources (the only C file,
  `mock_udev/include/libudev.h`, is deleted in the pending commit). The REST API cannot set a language list that
  includes Rust, so untick C/C++ in Settings > Code security > CodeQL (manual, 2026-10-15).
- Move the hand-built direct frame (`build_direct_frame`) and `0x4B` init buffer in `apex_pro_tkl_2023.rs` to typed
  `HidCommand`s in `hid_reports.rs` (hard constraint 1; pre-existing, found in review 2026-10-15).

- `src/main.rs` is still ~2,000 lines after `cli.rs` and `daemon.rs` were split out. Extract the remaining
  low-churn `cmd_*` families into modules without changing clap behaviour, output or feature gating. Do this only
  when no P1-P5 work is in flight.
- ~~Code comments that contradict the docs~~ Done 2026-10-15 (the `0x40` doc moved to `CommandCode`, `apex.rs` zone
  comment, duplicated `send_feature` doc, `sonar.rs` doc link). Original list:
  - `src/devices/hid_reports.rs` (~lines 21-22, 682-690) calls `Apex2023DirectCommand` "Confirmed"; the rule file
    lists `0x40` as experimental. Relabel `[EXPERIMENTAL]`.
  - `src/devices/keyboards/mod.rs` (~lines 397-398): duplicated, stale doc line about "interface 3 for wireless".
  - `src/devices/keyboards/apex.rs:17` says "10-zone"; `ZONE_COUNT = 9` (and see P3.1 for 8).
  - `src/audio/sonar.rs:8` links `docs/development/SONAR_PROTOCOL.md`, which does not exist.
- ~~`apex.rs` raw byte arrays~~ Done 2026-10-15: all four uncalled helpers (reactive, colour shift, `0x23` effect,
  `0x24` OSD) deleted. Original note: `src/devices/keyboards/apex.rs` (~lines 36-55) builds raw byte arrays for reactive / colour-shift, which breaks
  hard constraint 1. Move them to typed helpers in `hid_reports.rs` (both commands have no callers outside
  `apex.rs`; check whether they can be deleted instead).

---

## Done or dropped (do not redo)

- Issue #120 audio hang (5 s timeout in `src/audio/pulse.rs`).
- Protocol-docs reconciliation, capability-label cleanup, Windows pollrate "unsupported on this driver" notice,
  SignalRGB research (it drives devices directly via its own plugins; no SteelSeries bytes in public docs).
- Apex Pro TKL 2023 wired: GG capture, 84-key lighting table, 645-byte frame, interface-1 path, profile slot map
  (`WIRED_2023_PROFILE_SLOTS`), GG-engine warning. Committed in `f430380`.
- Apex 7 on-board profile read: `onboard` feature, `src/onboard_profile.rs`,
  `src/devices/keyboards/apex_legacy_onboard.rs`, `ssgg onboard` (2026-10-15). Read-only, `[EXPERIMENTAL]`.
  Hardware validation on 2026-10-15 got a pacing frame, a pull and the payload-read request accepted on
  the `0xFFC1` collection, then stalled: the code asked the kernel for a 643-byte feature report, the
  device's report is 642, so the control read never completed (the 5-second `ETIMEDOUT` in the log).
  Fixed: replies are read as a 642-byte feature report (matching the capture's `GET_REPORT wLength 642`)
  with the interrupt endpoint as fallback, `--device` / every collection of the same `vid:pid` is tried in
  interface order, the probe is one attempt per step and stops after `--deadline-ms`, a page starting
  `FF FF FF FF` counts as `garbage_reads`, and the chain gets a settle gap.
  Re-decoding the capture's URBs (not decoded response listing, which only holds responses, and not the text exports,
  which prefix every row with the capture's own header) settled the request shapes: the pull is a 64-byte
  output report (`wValue 0x0200`, report number 0), the payload read is a 642-byte feature report
  (`wValue 0x0300`, size 512, 2 for the schema probe), and Step 1 of the capture summary is six 64-byte output
  reports 11 ms apart - `90 00`, `90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01` - answered on the
  interrupt endpoint with the ASCII firmware version for the `90` polls. Implemented 2026-10-16 as
  `hid_reports::ONBOARD_INIT_CHAIN` (it moved to `src/devices/hid_reports.rs` on 2026-10-17, see below):
  the read path sends the chain once per run and prints the version,
  `--no-init-chain` compares the same read without it, `--gap-ms` paces it (11 ms = the capture, 1 ms =
  the old pacing word). The 13-byte "pacing frame" (`01 00 00 …`) is not on the wire: it
  came from the text export and `config_frame` was deleted.
  The answers are 643-byte `GET_REPORT` replies, report-ID byte `03`, payload directly behind it: framing
  0 is `READ_REPLY_FRAMING_LEN` and the `--reply-header` default (the old default 20 was the export
  prefix). Measured from the replies: the schema word is payload offset 0 (`0b 00` = schema B) and the
  name offset 2, and the payload-read reply is a file head, not a window - 241 reads return 33 distinct
  payloads (179 all-zero, `0b 00` in 6, one each of `Config 1`..`Config 5`), the requested buffer and file
  offsets do not change the answer, so no image can be assembled (CRC-32/STM32 matches nothing at any
  candidate offset) and only the schema probe is trustworthy.
  Windows GG vs the ssgg run of 2026-10-15 (Linux run capture, usbmon, 1428 URBs, bus 1 device 24,
  compared 2026-10-16): the chain, the chain answers (`4.16.8`, `00 01`, none for `6c 00 01`), the pull
  header and the `83` header all match, and ssgg's framing (report ID = command byte, so `wValue 0x0290`/
  `0x0285`/`0x0383` and one byte more in `wLength`) is answered exactly like GG's. The decisive difference:
  GG issues `GET_REPORT 0xa1 0x01 wValue 0x0300 wIndex 1 wLength 642` after every one of its 241 read
  requests and reads the window from that 643-byte control-IN data stage; the ssgg run sent **no**
  GET_REPORT, its 612 control completes all carry zero bytes, and all it collected is 64-byte interrupt
  status. Secondary differences, neither a blocker: ssgg sprays the chain and the pulls over interfaces
  0-4 (GG uses interface 1 only) and its 17 read requests are byte-identical, so it never varies file or
  window. Table: `docs/development/apex-7-onboard-profile-read.md`.
  Done in code 2026-10-17 (hard-constraint 1 work: buffer creation moved to `src/devices/hid_reports.rs`):
  `OnboardInitCommand` + `ONBOARD_INIT_CHAIN`, `OnboardPullCommand`, `OnboardReadCommand` and
  `ONBOARD_OUTPUT_REPORT_SIZE` / `ONBOARD_FEATURE_REPORT_SIZE` / `ONBOARD_REPLY_REPORT_SIZE` are the only
  place these frames are built, `apex_legacy_onboard` sends them through `HidReportBuilder::build_report`,
  every buffer starts with report number **0** (`wValue 0x0200 wLength 64`, `wValue 0x0300 wLength 642` -
  GG's shape, replacing `0x0290`/`0x0285`/`0x0383`), the read-back asks report number 0
  (`READ_REPLY_ID = 0x00`, `wValue 0x0300`) and `onboard_profile` keeps plan/parse/decode only. Same day:
  `devices::list_hidraw_paths` is gone (it existed in three copies) and `candidate_paths` asks
  `find_hidraw_for_interface(vid, pid, PROFILE_INTERFACE = 1)`, so the probe no longer sprays frames over
  interfaces 0-4. `find_hidraw_for_interface` on Unix now compares the interface number `interface_from_phys`
  reads from `HID_PHYS` (its old `ends_with("/input{N}")` test never matched a multi-collection phys string,
  so interface 1 resolved to nothing).
  *(superseded 2026-10-17: `candidate_paths`, `PathTransport` and the `*_report_raw` helpers are removed; the
  read runs on the `Apex7` device handle - see P0 "Linux collection" and item 0 of this section.)*
  Next step (keyboard): ask the read-back of the collection that declares it - interface 1 (`0xFFC0`): the
  kernel validates GET_REPORT against the interface's report descriptor, and interface 1 answered `EPIPE` to
  the 2026-10-15 report-ID-framed writes. Then
  with the keyboard: `ssgg --debug onboard read --yes` and, per candidate framing, `ssgg --debug onboard
  read --yes --reply-header <offset>`, and check in the capture that a control-IN transfer with 642 bytes of data
  appears; `--no-init-chain` and `--gap-ms 1` measure what the 2026-10-15 acceptance of `83`/`85` proves
  (GG runs the chain before every read session).
- Old TODO items: clippy-ci / future-incompat errors (resolved, see Baseline); `ssgg_bug_report.json` triage
  (dropped: the file was never provided); issue #6 Arch compile (closed `not_planned`).
- `docs/development/` consolidation (2026-10-15): capture-note "Transport" section matches the 645-byte /
  `info.path` code for `0x1628`; `0x40` and `0x2D` relabelled `[EXPERIMENTAL]` with a link to the capture note;
  `protocol-keyboard.md` merged into `devices.md`; index in `docs/development/README.md`.
- Old Phase 2 task "replace placeholder `KeyAddress` values for the 2023 model": superseded for `0x1628` by the
  HID-usage table from the GG capture.

## Reference projects

- Apex RGB: OpenRGB `SteelSeriesApex8ZoneController`
  (https://gitlab.com/CalcProgrammer1/OpenRGB/-/blob/master/Controllers/SteelSeriesController/SteelSeriesApex8ZoneController/SteelSeriesApex8ZoneController.h),
  https://github.com/AstroSnail/apexctl, https://github.com/FrankGrimm/apex7tkl_linux,
  https://github.com/not-jan/apex-tux
- Sonar / audio: https://github.com/PrzemekkkYT/GGSonarRev, https://github.com/wex/sonar-rev,
  https://github.com/Mark7888/steelseries-sonar-py, https://codeberg.org/Birbwell/linuxmix,
  https://github.com/Dymstro/nova-chatmix-linux
- Research only: https://github.com/flozz/rivalcfg, https://github.com/llMBQll/OmniLED,
  https://github.com/Sharper-Flow/Open-G-Hub
