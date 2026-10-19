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

---

## P0 (onboard). Apex 7 on-board write: software done, hardware open

Implemented 2026-10-20 from the operator's Windows capture (GG write capture, Apex 7 `0x1612`,
firmware 4.16.3, 122 frames): `ssgg onboard write` (JSON by default, `--bin` for payload bytes, `--profile N`
mandatory, `--dry-run`) and `ssgg onboard load` (`--profile N` mandatory). The sequence is the
capture's: `88 00 <file id>` open, 40 windows of two 512-byte `03 00 <buffer offset> <size>` feature writes
(642 wire bytes) plus one `05 00 <file id> <file offset> 1024` push, `eb <slot + 1>` commit, then `89 <slot + 1>`
deploy - the engine sends that frame 250 ms after the commit, `ssgg` sends it with the commit and with no pause, and
`--dry-run` shows it as the frame after the 122. Geometry comes from the keyboard family
(`OnboardProfileCodec::write_plan`); the capture's
answer to a push is `05 01 …` (202/202) and to the open `88 01 …`. Frame builders in `hid_reports`
(`OnboardFileOpenCommand`, `OnboardBufferWriteCommand`, `OnboardPushCommand`, `OnboardCommitCommand`), the device
path `Apex7::write_profile` / `Apex7::deploy_profile` behind `Keyboard::write_onboard_profile` /
`Keyboard::load_onboard_profile`, fixture
`tests/fixtures/onboard-write-apex7.manifest.txt` (generator `scripts/gg-write-frames.py`) asserted
frame for frame by `apex_legacy_profile::tests::the_write_plan_of_the_read_report_is_the_capture_write_sequence`.
`onboard decode` of the capture's image answers `Config 1`, schema `0x000B`, stored checksum `0EB1C3D1` equals
the one `encode` computes. **No test touches a keyboard and no write frame has been replayed on hardware**; the
read-back question stays open (the engine's own `83`/`85` read path is what `onboard read` uses). Write-up:
`docs/development/apex-7-onboard-profile-write.md`; task and open steps: `ONBOARD.md` §Apex 7 on-board profile write.

- **[HW] First write (user-approved, model and date recorded):** read the target slot back
  (`onboard read --profile N --bin …`), write the slot with `onboard write --profile N`, report what the
  keyboard does.
- **[HW] write → read → decode → diff:** prove the payload the board holds matches the JSON the write took, twice
  (the ONBOARD.md acceptance).
- **[HW] Deploy behaviour:** `onboard load` makes the board run a slot. Keyboard effects of `89` (and of the lighting
  reset `0x3B` the engine sends at session end) have never been tested from this crate. The write path sends `89`
  right after the commit with no sleep and reads nothing back (2026-10-20); whether the board needs the engine's
  250 ms pause behind the frame is a hardware question.
- **[HW] Pacing on hardware:** the capture's 7.1 ms average frame spacing, the engine's 100 ms every 32 frames, the
  250 ms deploy pause and the 20 s deploy timeout (engine `SetProfileToBoard`: 250 ms sleep, 40 × 500 ms poll)
  are engine/capture facts, never measured from `ssgg`.

## P0 (onboard). Apex 7 on-board read: first hardware run on the device path

Code is in place (2026-10-17, `onboard-configs-dev`): `Apex7` in `src/devices/keyboards/apex7.rs` wraps
`GenericKeyboard` (its `HidReportBuilder` and the open `HidDevice`) and does the read's packet I/O
itself on that handle; `Keyboard::read_onboard_profile` is the device API,
`discovery::open_keyboard` returns `Apex7` for `0x1612`, `0x1614`, `0x1616`, `0x1618` - the whole read path is
core code (2026-10-17: the `onboard` feature flag was removed, the device class and the read path are both in
the default build). `ssgg onboard read` runs on that device. **Status 2026-10-18, updated 2026-10-19:** a full profile slot read
completes on an Apex 7 under Linux, **with no opening chain and no closing sequence sent** - the 2026-10-18
finding that a read with only the `90 00` poll answers with scrambled payload bytes is superseded by the
runs, which collect a slot with none of GG's chain frames and none of its closing frames on the
wire. `ONBOARD_INIT_CHAIN`/`send_init_chain` and the four closing frames
(`send_end_sequence`, 2026-10-18 and 2026-10-19) are out of the code: **the read sends request frames only and
changes nothing on the keyboard**. **Updated 2026-10-19: the slot-0 read image decodes with a checksum match** (`p0.bin`, wired Apex 7,
firmware 4.16.3, `AE57B9DC` over `payload[0..11692]`), which settles the payload layout: 5-byte
`key_mapping` entries (the decoder used 4-byte entries, so the fourth code of every mapping was cut off
and the GUID, OLED, Rapid Tap and macro offsets all sat 128+ bytes early), OLED at 1385, Rapid Tap at
2025/2026 with 5-byte pairs, macro at 2076, GUID at 11676, and the checksum word at the **payload** tail,
not at the tail of the 12288-byte read image (the image is padded with whole resource buffers).
`decode` reports each mapping as `self` / `none` / `custom`, so the factory slot reads as 112 self-bound
keys, 12 empty slots and 4 shipped media-cluster bindings - no remaps on the base layer. So this section
is the validation backlog: record the model, firmware, slot bytes and the checksum line from the run log -
that is what promotes any of it to confirmed.

0. **[HW] The per-pull pacing read stays** (2026-10-17, Apex 7 `0xFFC1`): the pull's answer is the keyboard's
   idle input report, and draining it per chunk is what lets the following feature read-back complete - a run
   that removed the drain failed every payload read-back with `io::read_exact: unexpected end of file`. The step
   is `Apex7::read_pull_ack` (`PULL_ACK_LEN` = 64 bytes, GG's `read-chunk HIDIO 65`).
1. **[HW] Collection check - answered 2026-10-17/18:** the read reads the profile on the collection the device
   manager opened (`usage 0xFFC1`, interface 4 on that box), so the interface-1 question is not blocking. Keep the
   sysfs check (`ls -l the device sysfs input nodes`) when a box behaves differently; if a keyboard
   needs interface 1 (`wIndex 1` in the capture) and the manager opens another collection, that collection has to be
   discovered as its own device.
2. **[HW] Reply framing on the device - answered 2026-10-17/18:** framing 0 ([`READ_REPLY_FRAMING_LEN`]) is what the
   reads collect profile bytes with, matching the capture. The geometry, pacing, single `90 00` poll and reply framing are
   constants (`ReadConfig` and its CLI tunables are gone), so a keyboard that anchors its payload elsewhere is a code
   change measured from a `--debug` capture first; the `90` poll answer in the log confirms the device path.
3. **[HW] Configs `2..5`, then `--verify`** (Config 1 reads and decodes with `crc_ok: true` since 2026-10-19; 2026-10-19 also made `--profile` take GG's 1-based config number): `ssgg onboard read --profile N
   --bin /tmp/pN.bin`; every read
   prints that slot's schema word (`0x0000` means the slot was never saved). Pacing is the `GG_GAP_MS` constant, so a gap ladder is a code change. Record the checksum line and the decoded layout as
   `ONBOARD.md` "Next hardware steps" describes.
4. **[HW] Windows:** same read through the hidapi path; confirms the interface choice for the control
   collection.
5. **[HW] Closing sequence - closed 2026-10-19, the read sends none of it:** measured on hardware that the
   read completes without these frames, so `Apex7::send_end_sequence` and `onboard_end_sequence` are out of the
   code. What stays open (untested, unmeasured) is what the engine's frames do to a keyboard: `Apex7::send_end_sequence` closes
   every read with `69` (Gamesense OLED off), `89 <slot + 1>` (deploy that slot), `41 00` (profile not volatile)
   and `6c 00 00` (close driver mode). The first three are in the capture's frame inventory; the fourth is the
   counterpart of the opening chain's `6c 00 01` - the read closes the driver mode its own chain opened - and is
   in **no capture**, because GG was killed in the capture and the frame is a session-close frame. None of the
   four has been sent from this crate on hardware. Watch the keyboard: does the active profile change to the slot
   that was read, does the OLED go dark, does the keyboard still type normally after the read (driver mode is what puts
   some key combos into the keyboard report), and does the read complete with the frames sent. The capture's single 64-byte `ff` frame (kit's capture summary, "idle, do
   nothing" step) is **not** part of the sequence and is not sent: driver mode is `0x6C` in this family and
   `0x03` on Apex 3, so `ff` is not driver mode, and its direction is not settled. `ONBOARD.md` step 8.

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

- 2026-10-19: on-board **payload layout** fixed against the Windows engine's own struct (`apex_7.lisp`
  `onboard_profile_payload`) and verified on hardware by the checksum: 5-byte `key_mapping` entries
  (function byte + 4 key codes, not 4 bytes total), 5-byte Rapid Tap pairs, OLED block at 1385, wheel
  META at 1375, macro at 2076 (schema B), GUID at 11676, schema padding only in schema A, 3-byte
  `rapid_tap_enable`, and the CRC-32/STM32 word at the payload tail (`11644` / `11692`) - the read image is
  padded with whole resource buffers behind the payload, which is why every earlier CRC candidate over the
  image tail disagreed. Added to the JSON: per-entry `offset`/`hid_code` (from GG's
  `device-offset-lookup-table`, embedded as `OFFSET_TO_HID`/`HID_TO_OFFSET`, 120 of 128 slots), `binding`
  = `self` / `none` / `custom`, and a `profile.summary` line that counts only `custom` as remaps. GUI
  (`tools/profile-gui`) matches: neutral caps for self-bound and empty keys, a Custom-bindings list, four
  code bytes per entry. Slot-0 factory read of a wired Apex 7 (firmware 4.16.3) decodes as 112 self-bound,
  12 empty, 4 media-cluster entries, `crc_ok: true`; its JSON is
  `tools/profile-gui/examples/sample-identity.json` (name and block heads blanked).
- Issue #120 audio hang (5 s timeout in `src/audio/pulse.rs`).
- Protocol-docs reconciliation, capability-label cleanup, Windows pollrate "unsupported on this driver" notice,
  SignalRGB research (it drives devices directly via its own plugins; no SteelSeries bytes in public docs).
- Apex Pro TKL 2023 wired: GG capture, 84-key lighting table, 645-byte frame, interface-1 path, profile slot map
  (`WIRED_2023_PROFILE_SLOTS`), GG-engine warning. Committed in `f430380`.
- Apex 7 on-board profile read: core code (the `onboard` feature flag was removed on 2026-10-17),
  `src/devices/keyboards/apex_legacy_profile.rs`,
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
  the opening chain is always sent and pacing is `GG_GAP_MS` (11 ms = the capture, 1 ms =
  the old pacing word). The 13-byte "pacing frame" (`01 00 00 …`) is not on the wire: it
  came from the text export and `config_frame` was deleted.
  The answers are 643-byte `GET_REPORT` replies, report-ID byte `03`, payload directly behind it: framing
  0 is `READ_REPLY_FRAMING_LEN`, the constant `onboard read` uses (the old default 20 was the export
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
- 2026-10-17: **what answers a pull** (decoded at the endpoint level of the engine session capture plus the engine's read spec).
  The pull is pacing, not a data transfer: all 121 pulls are followed 1.7-3.7 ms later by the keyboard's ordinary
  input report `0x01` (63 bytes, all-zero = no keys pressed), and the read API keeps **zero** payload bytes of it -
  the byte specifier of the pull chunk is `(0 0 0)`, the payload bytes come from the two `0x83` window replies
  (`(1 512 0)` each). The 121 "64-byte pull replies" the capture kit lists as control responses starting `85 00 …`
  are the `SET_REPORT` requests GG submitted (`capture.ps1` labels host-to-device control data "control response"),
  which is where the earlier constant-reply claim came from; the same applies to all 72 `0x05` lighting pushes.
  Cadence from the same re-read: pull answered in 1.7-3.7 ms, read request answered by its `GET_REPORT` 3.4 ms
  later (median), GG's collection of the profile files takes 1.8 s (~15 ms per chunk) - the kit's "about 2 s per
  chunk" paired pulls with the 144 payload writes of the write runs. Ghidra side-check (`SSEdevice.dll
  HandleHIDCmd`): chunk types 5/6 are thin write/read wrappers over the device transport, types above `0x10`
  return `0xbadaf6`, so no richer reply path exists. Written into
  `docs/development/apex-7-onboard-profile-read.md` ("What answers a pull"), `ONBOARD.md` item 0a, the `0x85` row of
  `.claude/rules/experimental-protocol.md`, and the comments of `apex_legacy_profile.rs` /
  `apex_legacy_onboard.rs` (the packet after a pull is consumed, never parsed);
- 2026-10-17: same-day follow-ups after review - the materialised transfer plan (`OnboardTransferPlan`, `plan_frames`, `plan` and its CLI wiring), the schema probe (`OnboardSchemaProbe`, `read_config.schema_probe`), the transport layer (`OnboardTransport`, `HandleTransport`, `PathTransport`, the per-call path-open helpers) and the per-device stored slot (`Apex7::onboard_slot`, `set_onboard_slot`, `read_profile_slot`) were all removed, the read API takes the slot as an argument, every send on `GenericKeyboard` now shares one `connected_device` accessor, and `src/onboard_profile.rs` moved to `src/devices/keyboards/apex_legacy_profile.rs` (feature-gated) because its geometry, framing, payload sizes, schema words and struct offsets are all specific to this legacy platform. Same day: the CLI stopped printing that
  geometry and the schema-length line (they quoted legacy constants); the opening chain was removed from the read -
  `OnboardInitCommand`/`ONBOARD_INIT_CHAIN` are gone, `OnboardFirmwareVersionCommand` (`90 00`) is the one poll the
  read sends, because on hardware `90 00` and `90 01` answered identically on 2026-10-17 and `f4 00`/`6c 00 01` have
  no known meaning;
  geometry and the schema-length line (they quoted legacy constants), and the geometry is now logged by the read loop at
  `info` level - `src/main.rs` references no protocol constant.
  `devices::list_hidraw_paths` is gone (it existed in three copies) and `candidate_paths` asks
  `find_hidraw_for_interface(vid, pid, PROFILE_INTERFACE = 1)`, so the probe no longer sprays frames over
  interfaces 0-4. `find_hidraw_for_interface` on Unix now compares the interface number `interface_from_phys`
  reads from `HID_PHYS` (its old `ends_with("/input{N}")` test never matched a multi-collection phys string,
  so interface 1 resolved to nothing).
  *(superseded 2026-10-17: `candidate_paths`, `PathTransport` and the `*_report_raw` helpers are removed; the
  read runs on the `Apex7` device handle - see P0 "Linux collection" and item 0 of this section.)*
  *(superseded 2026-10-18: that read-back arrived - the reads of 2026-10-17/18 collect a 642-byte control-IN
  reply, framing 0, on the collection the device manager opened for the keyboard.)*
- 2026-10-18: cleanup after the first successful profile reads. Protocol constants are named as facts, not as
  tunables: `DEFAULT_CHUNKS` → `PULLS_PER_FILE`, `READ_LEN` → `PAYLOAD_READ_LEN`, `RESOURCE_BUFFER_LEN` →
  `DEVICE_RESOURCE_BUFFER_LEN`, `PROFILE_FILE_LEN` → `PROFILE_FILE_BYTES`, `SCHEMA_B_FIRMWARE` →
  `SCHEMA_B_FIRMWARE_VERSION` (documentation only - the read takes its schema from the word it reads). Deleted as
  unused: `PROFILE_FS_LEN`, `PULL_REPORT_LEN`, `INIT_REPORT_LEN`, `LIGHTING_FS_SHARED`,
  `MAX_PROFILE_INDEX_FOR_READ`, the read plan's `strict` flag (no CLI flag fed it) and its always-`None`
  `deadline`/reply-budget plumbing (one payload read now has one fixed `REPLY_BUDGET_MS`). The read loop runs on
  `std::thread::scope` instead of `spawn_blocking` fed by an `unsafe` transmute to `&'static` - the scoped borrow is
  checked, and the read blocks the task's thread for its ~2 s. Read status recorded in `ONBOARD.md` (steps 1, 1a, 1b,
  2 closed), `docs/development/apex-7-onboard-profile-read.md` (intro, transport section) and
  `.claude/rules/experimental-protocol.md`; `ONBOARD.md` steps 3-5 are what is left, starting with the model,
  firmware and decoded bytes of a run.
- 2026-10-18 (regression fix): GG's opening chain is back. `OnboardInitCommand` + `ONBOARD_INIT_CHAIN`
  (`90 00`, `90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01`, `GG_GAP_MS` apart) are rebuilt in `hid_reports.rs`,
  `apex_legacy_onboard::send_init_chain` sends it in front of the first pull and takes the `firmware:` line from
  its first `90 00`; `read_firmware_version` is gone (the chain's first poll is that poll), and a test asserts the
  six frames against the capture and chain frame 1 against `OnboardFirmwareVersionCommand` byte for byte. Hardware
  evidence: with only the `90 00` poll the read is answered on the wire and the payload replies come back
  scrambled; with the chain the same read collects a profile slot (Apex 7, 2026-10-18). Which of the six frames the
  keyboard needs is open - `ONBOARD.md` step 0b. Same day, the same run log showed the chain is answered **one frame
  at a time**: draining only the first answer left the answers of `90 00` and `90 01` and nothing from frame 3 on, so
  `send_init_chain` now sleeps `GG_GAP_MS` after each frame and reads one input report (`GG_INPUT_READ_BYTES` = 65,
  GG's `read-chunk HIDIO 65`, which `PULL_ACK_LEN` now shares); why an unread answer silences the frames after it is
  inference, not measured.
- 2026-10-18 (off-by-one): the reply-side lengths now derive from one measured number. GG's read-back is
  `GET wLength 642`, and the two directions behave differently: the write strips the buffer's report-number byte
  (`ONBOARD_FEATURE_REPORT_SIZE` 643 goes out as 642) while the read sends the requested length verbatim
  (hidapi hands the slice length straight to `HIDIOCGFEATURE`, `src/linux_native/ioctl.rs:26`), so
  `READ_REPORT_LEN` = 642 is both the ask and the wire number and a reply arrives as `[report ID][641 bytes]`.
  The size cap follows from that: `ONBOARD_MAX_READ_LEN` and `MAX_READ_LEN` are both 642 − 1 − 5 = 636 (the
  report-ID byte and the 5-byte header come off the reply), asserted equal in code; they said 641 and 637, which
  subtracted the report-ID byte but not the header. The capture shows the deleted raw-ioctl code
  putting `wLength 641` on the wire, one byte short of GG - that is why "643 produces `ETIMEDOUT`" and "no reply
  payload arrived" both read as facts. Recorded in `apex-7-onboard-profile-read.md` (§3 transfer map, §3.1
  comparison, §5 collection map) and `linux-read-interval.md` §5.
- 2026-10-18 (schema gate + read plan on the device): two changes in the same pass.
  **The probe is now the gate.** The read runs GG's 2-byte file-head probe as its first step (`Apex7::probe_schema`:
  2-byte pull at file offset 0, then one window read at buffer offset 0 - the pair the transfer map lists) and
  **refuses any schema word that is not `0x000B`**, because schema B is what `apex_legacy_profile` decodes and what
  has been measured: a schema A device now stops before anything is pulled instead of surfacing later as a profile
  whose checksum never matches. Measured 2026-10-18: the probe's schema word arrives on an Apex 7. Open question,
  `ONBOARD.md` step 0c: whether the probe is load-bearing for the read at all.
  **The read plan lives on `Apex7` now.** `send_init_chain`, `probe_schema`, `pull_window`, `read_window`,
  `read_reply_report`, `read_pull_ack`, `read_interrupt_report`, `write_output_command`, `write_feature_command`
  and `read_image` are methods of `Apex7`; the `apex_legacy_onboard` module is deleted, the duplicated pull/read
  blocks the probe introduced are factored into `pull_window`/`read_window` shared by the probe and the chunk loop,
  and the plan takes `&mut self` (the write path needs it), so `Keyboard::read_onboard_profile` became `&mut self`
  and `ssgg onboard read` binds the keyboard mutably.
  **Output frames: `send_report` tried, `write_profile_packet` kept.** The write path was measured in source for
  this: `send_report` writes via `write_padded_report`, whose global `HidOptimizer` **drops any report
  byte-identical to one sent in the last 50 ms** (`src/devices/mod.rs:124-185, 217`), while
  `GenericKeyboard::write_profile_packet` calls `device.write` with no cache. The opening chain repeats `90 00`
  and `90 01` 20 ms after their first copy, so through `send_report` frames 5 and 6 never reach the device and
  the keyboard sees a four-frame chain - against the measurement that made the chain load-bearing,
  which had all six on the wire. `send_report` was switched in on the owner's request and reverted the same day
  for exactly that reason; `write_output_command` is back on `write_profile_packet`, which is also why the plan
  stayed `&self` and `Keyboard::read_onboard_profile` did not become `&mut self`. Written up in
  `docs/development/apex-7-onboard-profile-read.md` ("Write path", plus a row in the Windows-vs-Linux table) and
  `ONBOARD.md` step 0, with the `--debug` signature "Skipping duplicate HID report" for spotting a drop.
  My earlier note for this round claimed the opposite (that `send_report` was the dedup-free write and the chain
  needed `write_profile_packet`): the mechanism was right, the direction was not - the dedup lives in
  `write_padded_report`, the path `send_report` uses.
  `docs/development/apex-7-onboard-profile-read.md` §2 probe row + code paths, §3.2 write-path paragraph,
  `docs/development/README.md` index row, `ONBOARD.md` steps 0 / 0c + Packet I/O row, `apex_legacy_profile` module doc.
  Checks: fmt, clippy `--all-targets -D warnings`, tests, release build - all clean.
- 2026-10-18 (post-deploy tail): the read now ends with GG's tail frames. **Measured in the capture** (the
  `wValue 0x0200 wIndex 1 wLength 64` writes in the profile-read steps of the kit's capture): the frame inventory
  has three `89` and three `41` frames - one pair behind each profile-file transfer - and two `69` frames. The
  engine's lisp names them: `disable_gamesense_oled` (`base_apex_legacy.lisp:724`), `load_profile` - `profile_id`
  typed `(range 1 5)`, sent as `succ index` by `switch_to_onboard_config` (`:2280`, `:2300`), with a 250 ms
  `chunk HIDSLEEP` behind it (`:2289`) - and `set_profile_volatile`, which `post_deploy_onboard` sends as 0 for
  every slot index `>= 0` (`:2291`, `:2531`). `src/devices/hid_reports.rs` adds `onboard_end_sequence(profile)`
  plus `CommandCode::DisableGamesenseOled` (0x69), `LoadProfile` (0x89) and `SetProfileVolatile` (0x41), built on
  the existing `OnboardInitCommand` so the frame stays the capture's 64-byte zero-padded output report;
  `Apex7::send_end_sequence` sends the three at the end of `read_image` - the chain's 11 ms pacing, with the
  engine's 250 ms sleep after the deploy frame. **These frames write**: `89 <slot + 1>` makes the keyboard run the
  slot it names, so the read is no longer write-free. No profile payload is written. `ONBOARD.md` step 8, the
  Safety sections of `ONBOARD.md` and `docs/development/apex-7-onboard-profile-read.md`, the `onboard` banner and
  the CLI docstrings say so. **Never sent on hardware** - the test is `ONBOARD.md` step 8.
  **Corrected 2026-10-19 (`ff`)**: this entry listed a fourth tail frame, `ff`, with capture epochs, and the code
  sent it. The capture does contain exactly one 64-byte `ff` frame, and the kit records it under its "idle, do
  nothing" step (capture summary), not behind a profile-file transfer; no lisp of the legacy Apex family names a
  `0xff` command (the engine's `end_driver_mode`, which had used 0xff, was removed 2026-10-17), and the frame's
  direction is not settled. So it is not part of the tail and is not sent: `onboard_end_sequence` is the three
  frames, `CommandCode::from_report_byte(0xff)` is back to `None`, and `test_command_code_parsing` keeps its
  original assertion. The epochs this entry quoted for `69` and `ff` were not reproducible from the pcaps on
  re-check and are dropped; the counts above are the capture's frame inventory as documented.
- Old TODO items: clippy-ci / future-incompat errors (resolved, see Baseline); `ssgg_bug_report.json` triage
  (dropped: the file was never provided); issue #6 Arch compile (closed `not_planned`).
- `docs/development/` consolidation (2026-10-15): capture-note "Transport" section matches the 645-byte /
  `info.path` code for `0x1628`; `0x40` and `0x2D` relabelled `[EXPERIMENTAL]` with a link to the capture note;
  `protocol-keyboard.md` merged into `devices.md`; index in `docs/development/README.md`.
- Old Phase 2 task "replace placeholder `KeyAddress` values for the 2023 model": superseded for `0x1628` by the
  HID-usage table from the GG capture.
- 2026-10-19 (closing sequence, driver mode): the read's closing sequence is four frames - `69`,
  `89 <slot + 1>`, `41 00` and **`6c 00 00`**. The last closes driver mode: the opening chain's fourth frame
  `6c 00 01` is the engine's driver-mode-on frame, so the read closes what it opened, the way the engine ends a
  session. `6c 00 00` is **not in the capture** - the engine writes it in its session close, and GG was killed in
  the capture, so its closing frames never hit the wire. The frames are the engine's, the sequence is ours, and
  **none of the four has been tested on hardware** (item 5). `CommandCode::OpenFiles` (`6c 00 01`, a name taken
  from a decoded engine command table) is renamed `CommandCode::DriverMode` (`0x6C`), with the family difference
  recorded: **Apex 3 uses command `0x03` in a 33-byte report** and its opening and closing frames are
  byte-identical; **Apex 3 TKL uses `0x6C` in a 65-byte report and opens with `profile_driver_mode 1`** where the
  legacy Apex family opens with 0. The engine's session close also writes `reset_all_illumination` (`0x3B`); the
  read does not send it - clearing the keyboard's lighting is not part of a profile read. `ff` stays out (see the
  corrected entry above): driver mode in this family is `0x6C` and on Apex 3 it is `0x03`, so `ff` is not driver
  mode; no lisp of the family names it and its direction is not settled. `ONBOARD_END_FRAME_COUNT` 3 -> 4;
  `onboard_end_sequence` builds the fourth frame on the same `OnboardInitCommand` path, and the tail test asserts
  all four frames plus the chain/closing command-byte relationship; the engine's 250 ms sleep stays behind the
  deploy frame and the closing frame needs no gap. Docs: dev doc §Driver mode (new section), `ONBOARD.md` step 8
  and two new Known-facts rows, rules rows for the chain and the closing sequence plus a family row, index row.
  Code comments cite wire facts and point at the doc, because `ONBOARD.md` line 10 keeps the `lisp/` descriptors
  out of code comments.
- 2026-10-19 (sequences the read does not need, firmware source byte): **measured on hardware, `ssgg onboard
  read` collects a profile slot with GG's opening chain not sent and with the closing sequence not sent**, so
  both are out of the code - `ONBOARD_INIT_CHAIN` / `OnboardInitCommand` / `Apex7::send_init_chain`,
  `onboard_end_sequence` / `ONBOARD_END_FRAME_COUNT` / `Apex7::send_end_sequence`, the 100 ms settle gap behind
  the chain, and the command codes nothing sends any more (`EngineStatus 0xF4`, `DisableGamesenseOled 0x69`,
  `SetProfileVolatile 0x41`). This supersedes the entry above (the four-frame closing sequence, never sent on
  hardware) and the 2026-10-18 chain result: the read now sends request frames only and **changes nothing on the
  keyboard**, and `read` needs no confirmation flag: 2026-10-19 dropped `--yes` (with `require_yes`, its last
  user); 2026-10-20 dropped it on the write side too - asking for `onboard write` / `onboard load` is the
  authorisation, and `--dry-run` is the mode that sends nothing. `CommandCode::DriverMode` (`0x6C`) and `CommandCode::LoadProfile` (`0x89`, a device write - it makes the
  keyboard run the slot) stay in `hid_reports` as named builders, sent by nothing. `OnboardFirmwareVersionCommand`
  is typed with a source byte - `FirmwareSource::Keyboard` (`90 00`) answers the firmware running the keyboard,
  `FirmwareSource::Led` (`90 01`) the LED controller's; both read on an Apex 7, and the capture has both
  forms (capture summary rows 97, 115). The read sends one `90 00` in front of the probe. Measured detail: a payload
  read needs ~1 ms after its request is written, before reading the reply back - without the wait the read races the
  device filling its resource buffer and answers with garbage (`Apex7::read_window`). Docs: dev doc §Sequences the
  read does not need (new), header / transfers table / §Write path / §Driver mode / §Safety / open questions
  updated, `ONBOARD.md` steps 0, 0b, 8 superseded with a new facts section, rule-file chain and closing rows
  rewritten as "GG sends it, ssgg does not", index row and item 5 updated.
- 2026-10-19 (profile JSON GUI): `tools/profile-gui/` - a dependency-free browser page that opens the
  `ssgg onboard read` / `onboard decode` JSON dumps (written by default, named from the profile name in the
  image), shows the slot on a drawn ANSI keyboard,
  edits per-key mappings (base and META, function byte + `key_codes`), the lighting block, Rapid Tap
  pairs, the four wheel mappings and the profile name, and saves the JSON with fields it does not know
  preserved. Keys bind through the legacy family's HID -> offset table (`base_apex_legacy.lisp`,
  `device-offset-lookup-table` inverted - the same inverse GG's own `onboard-to-config-button-mapping`
  uses), so the editor addresses `button_mappings` indices, shows each key's offset on the cap, and
  draws the 17 mapped codes with no place on an ANSI shape in a strip, so no offset is hidden. Function
  names mirror `function_name` in `apex_legacy_profile.rs`. **JSON-only**: no device access, `crc_*`
  never recomputed, the `oled`/`macro_data` payload summaries cannot be rebuilt (the panel says so).
  Checks: `node tools/profile-gui/selftest.mjs` (84 checks, incl. a fake-DOM pass that builds the real
  page and clicks it, and the real capture dump when `p0.json` is in the repo), `node tools/profile-gui/preview.mjs`
  renders the board to SVG. Next steps belong to items 5 and 7 (hardware read, hardware write); the 2023
  actuation JSON is a different shape and is not handled.
- 2026-10-19 (JSON is per device, and the device is what answers): the read stamps its JSON with a `device`
  block (`vendor_id`, `product_id`, `product`, `known` - the same rule as every other product-id read: trust the
  number, not the name), `onboard decode` writes its JSON as `<name>.json` by default like the read does, and
  `onboard encode` takes `--json` as the default input kind with `--report` kept as an alias. A `decode` of a
  file is a file-only command, so it cannot know the device it describes: `--device <product-id>` carries the
  device the JSON is for (`0x1612`, `6162`, `Apex 7` forms accepted) and is recorded in the file.
- 2026-10-19 (JSON schema = the engine's own version field): `apex_7.lisp:1341` `json-schema` of
  `profile-image` is 43 fields, `json-parser.lisp:109` writes `schema-version` 1, and 2026-10-19 a `ssgg` read
  of the same slot under `json-format 3` (engine `onboard-json-version`) answers byte-identically to the same
  read under `json-format 1`, with the 43 fields, one per field. So the engine's `schema-version` **is** this
  JSON's `schema_version`, and our 3 is a number we invented: a GG-created JSON and ours differ in that one
  field. The JSON's own field, the engine's `json-schema` and the 34-field reader list
  (`apex_7.lisp:1364`) are now each asserted in
  `apex_legacy_profile::tests::the_json_field_list_is_the_engine_schema` - the reader-list half is the proof
  the 43-field JSON has a 34-field reader: 9 fields (all 6 `actuation_*`, the 2 `rapid_tap_*` and `key_guard`) are
  dropped in the payload path (`:220` `deploy-profile` -> `profile-write` -> `write-profile`), 22 are dropped
  in the JSON path (`profile-json` drops what `profile-write` does not). A payload decode therefore cannot
  rebuild that JSON - it has no source for those 24 fields - so the read's JSON is the only complete shape. A
  `schema_version` other than 1 is refused by the reader, and a schema-43 GG JSON read by `onboard encode`
  fails closed with a named field, not a silent one.
- 2026-10-19 (the region GG reads from, and the `6c` frame): the read pulls a **5120-byte** region (`05 00`
  asks for 5120 = 4096 + 1024) into a 6144-byte buffer (the engine's `buffer-size` argument to `write-buffer`,
  `apex_7.lisp:217`), and keeps only the first 4096 payload bytes (`file-write`'s `min` clamp at `:230`);
  `PROFILE_FILE_BYTES` stays 4096, which is 40960/10 on the engine's own comment. The read's extra 1024-byte
  pull is the engine's `file-read` default (`buffer-length` 1024) running over the whole region, so the region
  is read in 5 windows; the extra 1024 bytes answer on the wire (the read captures them) and are not in
  `file-write`'s destination - they are **not** the bytes GG writes to this slot. Recorded behind
  `--report` as `pull_region_bytes` plus a `region` block of the extra 1024 (sha256, hex, ASCII, printable,
  decoded `region`/`unmapped`/`zone N` labels), and compared in the decode report. `ONBOARD.md` step 11 is the
  open question: write the GG-built payload to an empty slot, read it back and diff the slot to learn what GG
  writes to an untouched slot; the 1024 bytes behind the payload are **not** evidence, and
  `apex_legacy_profile::tests::the_bytes_behind_the_payload_come_back_unchanged` is an in-memory
  assemble-and-split test, not evidence from a board. Same date, the read's `6c 00 01`: `6c 00 01` is the
  **driver mode** write (`set-driver-mode`, `base_apex_legacy.lisp:671`), the engine keeps it as `hid-mode`
  (`:798`), opens driver mode with `6c 00 01` and closes the session with `6c 00 00` (it does the same on
  Apex 3, with `0x75` - which is why the frame is a family constant, not an Apex 7 one). The engine's `6c`
  values are `00`/`01` only; `6c 00 0a` is the `set-controller-id` command (controller id 10), a different
  command with the same opcode.
- 2026-10-19 (the family codec is device-scoped): every device command resolves the keyboard first
  (`select_onboard_keyboard`), then asks that keyboard for its codec (`Keyboard::onboard_codec`), and the
  `onboard` device subcommands take a `--device` selector (`read`/`write`/`load`/`verify`). The codec is a
  family-wide value, not a device: `LegacyApexOnboard` decodes and encodes, names slots, says which product ids
  its layout applies to (`applies_to`), which models its feature set describes (`feature_set_for`) and what
  geometry a write of that family has (`write_plan`); the device method answers it, file-only commands reach
  it through `onboard::codec_for_product_id`. Two product ids with the same layout share one codec, a new
  family is a new codec; the `onboard::codecs` registry is the file-only lookup (and `onboard codecs` lists
  what is registered). `onboard features` is the command that prints the feature set a device answers with.
- 2026-10-19 (scoping the read to the read path): the plan's opening chain was sent from `read_image`, so every
  read sent frames that were not asked for. The chain is now a command of its own -
  `Keyboard::onboard_init_chain` (`ssgg onboard chain`) - and the read sends only what the read needs: one
  firmware poll, the schema probe, then the pulls (the two `90` polls inside the old chain are the same
  firmware poll, and the poll is informational, so a refused or unanswered poll is recorded and the read
  continues). The chain stays in `hid_reports` as `ONBOARD_INIT_CHAIN` and in the docs, with every frame cited
  to its descriptor: `90 00`/`90 01` = firmware poll, `f4 00` = `update-status` (engine
  `base_apex_legacy.lisp:1514`, 12 bytes, firmware status), `6c 00 01` = driver mode.
- 2026-10-19 (pacing measured on both captures, and GG's `6c` is its own pacing knob): the read's 125 pull frames
  are 0 ms apart and 708 of its 750 payload reads are 20 ms apart; the write's 103 frame gaps have a 7.1 ms mean,
  45 zeros, 47 gaps of 10-11 ms and a 100 ms pause every 32 frames. `ssgg` paces both paths at `GG_GAP_MS` = 5 ms
  (the write's median answer latency) and polls each answer every `REPLY_POLL_MS` = 50 ms. GG's own 10 ms is a
  **device setting**: the engine's `hidconfig` writes command `6c` with three u32 - time-between-commands (default
  10, commented "10 ms is also the default in devicelib"), file-access (`FA_READWRITE`), max-usb-timeout (500) -
  `standard-boilerplate.lisp:8-10, 24`; the profile read spec writes it with 1 ms in front of the pulls and resets
  it in `api-finally` (`apex_7.lisp:217, 224`), the write spec writes it with 5000 in front of the file open and
  again with 500 (`:251, 253, 270`). The chain's `6c 00 01 …` frame is that write, not driver mode: driver mode is
  the same command byte with a 4-byte payload (`base_apex_legacy.lisp:562, 570`). Both are in
  `docs/development/apex-7-onboard-profile-read.md` §"What paces the read".
- 2026-10-19 (a protocol reply is not a keyboard input report): the write's acks (`05 01` to a push, `88 01` to
  a file open) are read as a **report on the profile collection, matched by report id**, with the read's own poll
  (`Apex7::await_write_ack` over the protocol read). The keyboard's interrupt input report carries key events: in
  the read capture 293 interrupt reports arrive against 750 payload reads (698 of them within 40 ms after a payload
  read, which is key chatter), and protocol replies are never key reports. `read_report_timeout` is not used by any
  on-board path.
- 2026-10-19 (no thread for the read, the write or the deploy): all three are plain blocking calls. They used
  `std::thread::scope`, which joined before the call returned, so the blocking work ran on a thread either way
  and nothing ran concurrently with it; the read's ~2 s and the write's frame loop simply block the caller. The
  `join().map_err(...)` panic mapping went with the scope. The engine's 250 ms pause behind the deploy frame is
  not slept and its 40 x 500 ms deploy poll is not run: measured 2026-10-19, the capture shows **no frame and no
  reply after the `89`**, so the deploy sends the frame and reads nothing back.
- 2026-10-19 (the deploy is its own step and the write's last frame): `onboard load` sends `89 <slot + 1>` on
  its own, and `onboard write` sends it right after the commit with no pause - the engine sleeps 250 ms behind the
  frame and polls 40 x 500 ms for the slot to come back; measured 2026-10-19, the capture has **no frame and no
  reply after the `89`**, so `deploy_profile` sends the frame and reads nothing back.
- 2026-10-19 (the write plan is built per family): the codec names the write geometry (`file_id`, region, block
  window, `FILE_BLOCKS`, the ack report ids) and the family module frames it, because the windows are family
  facts (`apex_7.lisp:217` `write-region` -> `windows`, `file-write`'s `min` clamp): the engine caps a
  `file-write` at 1024 bytes, and the buffer window is 512 (`DEVICE_RESOURCE_BUFFER_LEN` 1024 / 2), so a family
  that sets `FILE_BLOCKS` to a value not divisible by 1024 (a 2048-byte block on a legacy family) is an error
  naming the divisibility rule, not a frame stream that ignores the engine.

- 2026-10-19 (on-board frame names): every on-board command in `hid_reports.rs` is now named by the object it
  moves, and its `CommandCode` variant carries the same words as the type: `OnboardFileOpenCommand` /
  `OnboardFileCommitCommand` (the profile file), `OnboardBufferReadCommand` / `OnboardBufferWriteCommand` (the
  device resource buffer), `OnboardPayloadPullCommand` (read side) / `OnboardPayloadPushCommand` (write side) (the
  payload, on the same opcode `05`), `OnboardProfileDeployCommand` (the slot; "load" is what the board does, we
  deploy), plus `OnboardFirmwareVersionCommand` and `OnboardDriverModeCommand`. Renamed from `OnboardReadCommand`,
  `OnboardCommitCommand`, `OnboardPullCommand`, `OnboardPushCommand`, `OnboardLoadProfileCommand`,
  `ONBOARD_GET_WIRE_BYTES` (now `ONBOARD_READ_WIRE_BYTES`) and the bare `CommandCode::{FirmwareVersion, DriverMode,
  LoadProfile}` variants. The report-size constants keep the hidapi vocabulary: `ONBOARD_OUTPUT_REPORT_SIZE` (65)
  against `ONBOARD_WRITE_REPORT_KIND::Feature`'s `ONBOARD_FEATURE_REPORT_SIZE` (643), `ONBOARD_READ_WIRE_BYTES` (642)
  as the GET_REPORT length, `ONBOARD_MAX_READ_LEN` (636) as the most a read frame may ask for. `PLAN.md` history
  entries keep the names used at their date.

- 2026-10-19 (Apex 7 only, and the names say so): the on-board path is scoped to the board it was captured
  and measured on. `Apex7::onboard_capable` takes `0x1612` alone - it used to take the four PIDs GG lists on
  this platform (`0x1614` apex_pro_tkl, `0x1616` apex_150, `0x1618` apex_7_tkl, `base_apex_legacy.lisp:6`),
  and none of them has hardware here, so none is enabled: `ssgg onboard read --device 1616` now answers
  `no keyboard with product ID 1616 answers the on-board profile protocol`. `LegacyApexOnboard` is
  `Apex7Onboard`, its `family_name()` is `Apex 7` (was `legacy Apex (Apex 7, Apex Raw, Apex 2014)`) and its
  write banner says `Apex 7 write geometry`. Files: `apex7.rs` -> `apex_7.rs`, `apex_legacy_profile.rs` ->
  `apex_7_profile.rs`. Protocol evidence keeps the platform prose (the engine's `apex_7.lisp` covers the
  siblings, `docs/development/devices.md` keeps the registry names) - what is gone is the code that enabled
  boards nobody has read an answer from.

- 2026-10-19 (discovery never asks the on-board question): the catch-all arm in `discovery::open_keyboard`
  that wrapped an on-board-capable product ID in `Apex7` is deleted, so a keyboard is built from its HID
  collection and product ID alone and keeps its lighting path whatever it can read off the board. The scope is
  stated once, on the codec: `Apex7Onboard::knows_product` (`0x1612`, the board the frames are known from),
  asked by `Apex7::onboard_codec` and reported by `Keyboard::supports_onboard_profile`
  (`= onboard_codec().is_some()`), which is the gate `open_onboard_keyboard` and `onboard read --device <pid>`
  use. `onboard_capable` and `Apex7::supports_onboard` are deleted as duplicates of that one report.

- 2026-10-19 (probe: the `6c 00 01` frame is answered on interrupt EP2, and it is not the read's pacing):
  sent to the live Apex 7 - 3 copies of `6c 00 01` (feature, EP0 path) and 3 of `6c 01 01` (output) - the
  device answered **every copy with `81 01` on interrupt EP2** (24 bytes, the rest zero). So the frame the
  engine's profile sessions use for pacing gets an ack, and the ack is an **interrupt report, not a feature
  report**: it is the same report the read's pulls answer on, on the same endpoint. Two facts about it:
  `0x6c` is **not** a HID report ID (report 68 on this board is a 64-byte output report; the engine sent it
  as a feature report ID 0, like it does with `80`), and the read's pacing frames are **not** the gap between
  the pulls - all 40 `6c`-family frames of a GG read sit in the session's first 43 ms, with a ~50 ms gap
  between the pulls and no frame in it; the `6c 01 01` stream runs at ~2 ms intervals and stops when GG stops.
  `6c 00 01` closes the session. Nothing in the read is time-limited: a pull is answered by its own reply.
  **Reverted here**: the `read_pacing: 1 ms` field (it was a name for a GG frame I had mis-assigned to the
  read's 50 ms gap, and it drove a sleep after every pull, and it printed `1 ms` in read output as a
  protocol property of a frame nobody has measured the interval on).

- 2026-10-19 (scope change): `onboard_capable` is deleted (2026-10-19) - `Keyboard::supports_onboard_profile`
  is the one report a keyboard makes about reading a profile off the board (`onboard_codec().is_some()`,
  which is the codec's `knows_product` answer for this device's product ID), and `discovery` never asked the
  question anyway: `open_keyboard` decides the keyboard class from the collection and product ID, the class
  decides what it can read. A keyboard the manager enumerates is a keyboard with its lighting path, whatever
  it can read off the board. `docs/development/adding-hardware.md` (the `Onboard-capable keyboards` section
  and the checklist row), `ONBOARD.md` and the read doc follow. **Reverted here**: moving `onboard_capable`
  into `keyboards/onboard.rs` as a shared predicate - the user redirected it to deletion, because it was the
  same rule as `supports_onboard_profile` written in two places.

- 2026-10-19 (the device record is two fields): `OnboardDevice` keeps `product_id` + `product`; `vendor_id`
  (every SteelSeries device answers `0x1038`) and the `known` flag go, and with them `devices::known_product_id`
  and the codec's `knows_product` - a record with the model name already carries the registry's reading of the
  number, and the on-board scope lives in the device class (`Apex7` answers the codec; discovery builds `Apex7`
  for `0xffc0/0x9f` + `0x1612`, the board the frames were measured on), not in a product-ID table a codec keeps.
  The decode report's device line reads `Apex 7 (product 1612)`. **Reverted here**: keeping a per-codec product
  ID table as the scope statement, and the registry-known flag as a label on the name.

- 2026-10-19 (no device struct): `OnboardDevice` is deleted and the identity is **three plain fields on the
  record** - `product_id`, `product` (model name), `firmware` - declared **above** the profile fields in
  `ProfileImage` and in the decode report, so a record says which keyboard before what it holds (a test asserts
  that order in the serialized text). A read stamps all three (`ProfileImage::stamp_identity` copies them into
  the decode report the read writes); a file decode names no keyboard and writes none of them, and the read
  record writes them as `null` to say so. The encoder ignores them. `Keyboard::supports_onboard_profile`
  remains the gate, and the `[EXPERIMENTAL]` one-board label moved onto the fields. **Reverted here**: the
  `device` object itself (three values plus a name for them) and, in the previous step, the `vendor_id` +
  `known` fields - one vendor ID for the whole brand and a name derivable from its number are not data.

- 2026-10-19 (the class rule is where the scope lives, and it went missing): `ssgg onboard read` answered
  `DeviceNotFound` on a connected Apex 7 that `ssgg devices` lists. `Keyboard::supports_onboard_profile` is
  `onboard_codec().is_some()`, and `Apex7` is the only class that answers a codec - so the read runs only on a
  keyboard discovery built as `Apex7`. Deleting the `open_keyboard` arm that built it (with the `onboard_capable`
  predicate it keyed on) left `0x1612` in the catch-all arm: the keyboard opened as `GenericKeyboard`, answered no
  codec, and the selection found no candidate. The gate was intact and the class behind it was not, which is why
  the symptom is "not found" and not "not supported". The rule is now `discovery::keyboard_class(product_id)`,
  a pure function `open_keyboard` switches on: `0x1612` -> `Apex7`, the Apex Pro family -> `ApexProTkl2023`,
  `0x1A01` -> `Apex3Tkl`, everything else -> `Generic`. Two tests pin it: one names the board the on-board path
  is enabled on, one names the platform siblings it refuses (`0x1614`, `0x1616`, `0x1618`) - the refusal is the
  scope, not a bug. `open_onboard_keyboard`, `onboard_choice` and `--device` are untouched: the selection was
  never the broken part. The named-product message now also says which keyboard answers the protocol.
  **Reverted here**: re-adding a product-ID predicate (`onboard_capable` under any name) as a second copy of the
  rule - the class decision is the rule, and a predicate on a codec is a second place to say it.

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
