# steelseriesgg-rs — Agent Handbook

## Project context

Open-source SteelSeries GG replacement for Linux. Controls SteelSeries keyboards and headsets via USB HID: RGB lighting effects, a GameSense-compatible HTTP server (port 27301), device profiles, and optional PulseAudio/PipeWire mixer and Sonar integration.

- **Language**: Rust 2024 edition (MSRV and toolchain: see Source-of-truth files)
- **Primary binary**: `ssgg` (`src/main.rs`, clap CLI)
- **Library crate**: `steelseries_gg` (`src/lib.rs`)
- **Platform**: Linux-first; Windows build supported but not primary
- **Config**: `$XDG_CONFIG_HOME/ssgg/config.toml`
- **Repo**: `https://github.com/Ven0m0/steelseriesgg-rs`

---

## Quick orientation

Use `rg --files src/` for the full tree. Non-obvious entry points:

```
src/main.rs                    main() and most cmd_* handlers
src/cli.rs                     clap types (Cli, Commands, *Action) and argument parse helpers
src/daemon.rs                  daemon loop, hot-plug monitoring, GameSense startup
src/error.rs                   crate::error::Error (thiserror) + Result alias
src/devices/hid_reports.rs     HidReportBuilder and typed HID commands - use this, not raw bytes
src/devices/discovery.rs       DeviceManager: enumeration, hot-plug, fingerprinting
src/devices/keyboards/         Keyboard impls (apex.rs, apex_pro_tkl_2023.rs = experimental)
src/gamesense/server.rs        Axum HTTP server (port 27301)
src/devices/keyboards/apex7.rs Apex 7 legacy family (`0x1612`-`0x1615`): on-board profile read, write, deploy
src/devices/keyboards/onboard.rs OnboardProfileCodec trait - the read/write geometry a family implements
src/devices/keyboards/apex_legacy_profile.rs Apex 7 codec: decode, encode, checksum, write plan
src/audio/                     feature `audio` (pulse.rs) and `sonar` (sonar.rs)
src/bin/                       helper binaries, not part of `ssgg` (discover_actuation probes actuation firmware)
examples/                      device_readback.rs (hardware read-back example)
scripts/capture-device.ps1     Windows USBPcap capture helper for new-device reports (see docs/development/adding-hardware.md)
assets/                        udev rules, systemd user unit
tests/                         integration tests
docs/development/              reverse-engineering notes; index and per-file state in docs/development/README.md
```

---

## Source-of-truth files

When prose docs and code disagree, always trust these files. Do not quote toolchain, MSRV, or pinned dependency versions from memory or from this document; read them.

| File | What it controls |
|------|-----------------|
| `Cargo.toml` | dependency versions and pins, features, MSRV, edition |
| `rust-toolchain.toml` | toolchain channel and components |
| `.github/workflows/ci.yml` | exact CI commands, pinned CI toolchain, feature matrix |
| `src/devices/hid_reports.rs` | HID command codes and report layouts |

---

## Toolchain and CI

### Local commands

Run the smallest relevant subset for any given change.

| Step | Command |
|------|---------|
| Format check | `cargo fmt --all -- --check` |
| Lint | `cargo clippy --all-targets --locked -- -D warnings` |
| Test | `cargo test --locked` |
| Release build | `cargo build --release --locked` |

Add `--features <flag>` when touching feature-gated code. The CI feature matrix is in
`.github/workflows/ci.yml`; the `ci-matrix` skill runs it locally. The `audio` feature needs `libpulse-dev` on
Debian/Ubuntu. On Windows, `--features audio` does not link (`LNK1181: cannot open input file ':libpulse-0.dll.lib'`);
this is expected, CI builds and lints `audio` on Linux only.

---

## Feature flags

| Flag | Enables | Extra dep |
|------|---------|-----------|
| *(none)* | Core RGB, GameSense, profiles | — |
| `audio` | AudioMixer, PulseAudio/PipeWire backend | `libpulse-dev` |
| `sonar` | SonarClient HTTP integration | — (reqwest already in tree) |

`audio` and `sonar` are independent; do not couple them. Experimental protocol paths are not feature-gated; they carry
`[EXPERIMENTAL]` labels instead (see `.claude/rules/experimental-protocol.md`).

---

## Hard constraints

1. **HID reports**: always use `HidReportBuilder` and typed helpers in `src/devices/hid_reports.rs`. Never build raw byte arrays by hand.
2. **hidapi pin**: keep the exact `=` pin on `hidapi` in `Cargo.toml`; changing it requires explicit task justification.
3. **GameSense CORS**: enforce localhost-only origin; the policy may only be tightened, never relaxed.
4. **Propagate errors**: use `?` or return an explicit `Err` value — no `unwrap` or `expect` in production paths.
5. **Error types**: `crate::error::Error` with `thiserror` at library boundaries; `anyhow` with `context()` in `src/main.rs` and binaries.
6. **Protocol accuracy**: the direct-mode frames (`0x40`, `0x61`, `0x3A`), the profile blob and `PerKeyRgb (0x23)` are reverse-engineered; only one solid-colour `0x40` frame was replayed on hardware. Label new protocol work clearly; never state it as confirmed unless tested on hardware.
7. **Minimal scope**: fix what the task asks, nothing more. No opportunistic refactors.
8. **No mutable global state**, no panics in non-test code (unless explicitly fatal and documented), no ignored `Result`s.
9. **`unsafe` blocks** must document the safety invariant inline.
10. **Platform-gated deps**: Unix-only crates (`rustix`) live under `[target.'cfg(unix)'.dependencies]`; keep uses behind matching `cfg`.

---

## Rust conventions

- Naming: `snake_case` for functions and modules, `PascalCase` for types and traits, `SCREAMING_SNAKE_CASE` for constants.
- `rustfmt.toml`: edition 2024 style, max 120 columns, 4-space indentation.
- Prefer `&T` borrows over owned values when ownership is not needed.
- New dependencies require justification; avoid heavy transitive deps.
- No comments that restate what the code does — comment only non-obvious invariants or workarounds.

---

## Working rules

- **Search before reading**: `rg 'TypeName|fn_name' src/`, then read only the relevant lines. Large files: `src/main.rs`, `src/devices/hid_reports.rs`.
- **Documents**: only create planning or analysis documents when explicitly requested.
- **Commit messages**: one short imperative subject, no body; details belong in `PLAN.md` and `docs/`.

---

## Protocol research notes

- `docs/development/` was checked against source on 2026-10-20 (`docs/development/README.md` lists each file and
  its state). Source still wins; verify before acting on a doc claim.
- Command-code status (confirmed / experimental / placeholder) lives in `.claude/rules/experimental-protocol.md`.

### Apex Pro TKL 2023, wired (PID `0x1628`): SteelSeries GG wire protocol

Captured from official GG on Windows with USBPcap on 2026-10-14 (firmware `1.19.7`). Full write-up, slot map and
offsets: `docs/development/apex-pro-tkl-2023-gg-capture.md`. All of it is `[EXPERIMENTAL]`; only the lighting frame
was replayed from `ssgg` (one solid colour, see below).

- **Where GG keeps the protocol**: the Electron UI (`app.asar`) only talks to the local core. HID code lives in
  `C:\Program Files\SteelSeries\GG\apps\engine`: `SteelSeriesEngine.exe` (Go) -> `SSEdevice.dll` -> `HIDDLL.dll`.
  `deviceSpecifications\*.edevice` are encrypted and were not decrypted.
- **Transport**: HID class SET_REPORT to USB interface 1 (`wIndex 1`). Feature report ID 0, 644 wire bytes
  (`wValue 0x0300`) for lighting and profile upload. Output report ID 0, 64 bytes (`wValue 0x0200`) for commands.
  Replies on interrupt EP `0x82`.
- **Lighting**: `0x40`, count `0x54` (84), then `[usage][R][G][B]` x84 (USB HID usages), zero padded. GG streams
  about 19 fps and overwrites other programs while its engine runs; with GG closed the keyboard keeps the last frame.
  GG brightness is software scaling of the RGB values (brightness 0 = all-black frame).
- **Code state**: `0x1628` uses the direct path with GG's 84-key table `WIRED_2023_KEYS`, a 645-byte buffer
  (report-ID byte + 644 wire bytes) and skips the `0x4B` init (`apex_pro_tkl_2023.rs`). Gen3 models keep `TKL_KEYS`
  (112) and 643. `send_feature` sends on the device's own hidapi handle on every platform; the path-based
  special case added for this model on 2026-10-15 was reverted on 2026-10-17 (**test on hardware** whether
  hidapi reaches it: `ssgg rgb color FF0000` on `0x1628` with GG stopped).
  Per-key (`rgb perkey`) sends the same 84-key frame with unlisted keys black (no read-back exists).
  `ssgg rgb` warns on Windows when `SteelSeriesEngine.exe` runs (`warn_if_gg_engine_running`, `src/main.rs`).
- **Hardware evidence**: on 2026-10-14, with GG stopped, `ssgg rgb color FF0000` turned the keyboard solid red
  (user-confirmed); USBPcap showed `wValue 0x0300, wIndex 1, wLength 644, 40 54 04 ff 00 00 05 ...`. Nothing else
  (other colours, per-key, Linux hidraw, actuation) has been tested.
- **On-board profile**: GG rewrites 12,500 bytes in 25 chunks after every change, chunk header
  `03 01 80 | len u16 LE (500) | offset u16 LE | 00 00`. Per-key tables of 70 slots: actuation at 1084, reset at
  1154, release mode at 1446 (0 off / 2 Rapid Trigger / 3 Key Guard), RT sensitivity at 1516; Rapid Tap enable at
  2700. Slot map is `WIRED_2023_PROFILE_SLOTS` / `profile_slot()`. Actuation raw values: 0.1 mm = `05`,
  1.0 = `15`, 2.0 = `39`, 3.0 = `86` (non-linear).
- **Profile checksum (solved 2026-10-15)**: little-endian u32 at blob offset 12320 = STM32 hardware CRC-32
  (poly `0x04C11DB7`, init `0xFFFFFFFF`, no reflection, no xorout) over bytes `0..12320` read as LE 32-bit words.
  Matches all 26 captured blobs (8 distinct). Code: `keyboards/apex_pro_tkl_2023_profile.rs` (checksum, actuation
  patch, chunk split; nothing sends it). **Still never write the profile**: no read-back command is known, so any
  write would replace the board's other settings with a stale or synthetic blob.
- **64-byte commands seen**: poll set `02 01 80`, `b1`, `b2`, `34`, `90` (`90` returns firmware version);
  `38 66 01/00` with Rapid Tap on/off; `38 67 0a 00 04 07 <n> ...` in an uncontrolled run. Meanings unknown except `90`.
- **GG's stored config**: per-key JSON in `C:\ProgramData\SteelSeries\GG\apps\engine\db\database.db`, table
  `configurations`, column `settings`, `device_id` 242 (`apex_pro_tkl_2022`). Open read-only with GG stopped.

### Apex 7, wired (PIDs `0x1612`-`0x1615`): on-board profile read and write

Apex 7 (`0x1612`), Apex Pro (`0x1614`), Apex 2014 (`0x1616`), Apex Raw (`0x1618`). All of it is `[EXPERIMENTAL]`:
no byte of the read side is confirmed on hardware per the promotion rule, and **no byte of the write side has been
sent to a keyboard at all**. Full write-ups: `docs/development/apex-7-onboard-profile-read.md` (read) and
`docs/development/apex-7-onboard-profile-write.md` (write); `ONBOARD.md` carries the open hardware steps.

- **Read** (`ssgg onboard read`, core code since the `onboard` feature flag was removed 2026-10-17): `90 00` firmware
  poll (last byte selects firmware source: `00` keyboard, `01` LED controller), one `83 00` schema probe, then per
  file 40 `85 00 <fs> <file offset> 512` pulls; every pull is **paced** by draining the keyboard's all-zero input
  report, every `83` window read is read back after a ~1 ms wait, profile files `0x0201 + slot` (`--profile N` =
  slot `N - 1`, GG's `Config 1`..`Config 5`), lighting `0x0301 + slot`, 512-byte resource buffer, 643-byte reply
  (report-ID byte + 642 data bytes). GG's six-frame opening chain and closing sequence are sent by nothing (measured
  2026-10-19: a full slot is read without them), the read changes nothing on the keyboard. Payload layout and
  checksum (FNV-1a 64 over `0..len-4`, u32 LE word at the payload tail) are in the read doc; `decode`/`encode` are
  file-only and `tools/profile-gui` binds them.
- **Write** (`ssgg onboard write`, `ssgg onboard load`, added 2026-10-20 from the operator's GG write capture write
  capture): `88 00 <profile file id>` open, then **40 windows** of the 40,960-byte profile region - two 512-byte
  `03 00 <buffer offset> <size> <data>` feature writes (642 wire bytes, buffer offsets 0 and 512, data behind the
  6-byte header) plus one `05 00 <file id> <file offset> 1024` push (64-byte output report) - then `eb <slot + 1>`
  commit, then `89 <slot + 1>` deploy after a 250 ms pause. A push is answered with `05 01 …` (202/202) and the open
  with `88 01 …`; buffer writes and the commit are not answered. Geometry comes from the keyboard's family codec
  (`OnboardProfileCodec::write_plan`), a write pads the payload to the region, and the file's own size is never read
  from the board. `--dry-run` prints the 122-frame plan with no keyboard opened; **`--yes` is what authorises
  sending** - without it write and load send nothing. Fixture asserted frame for frame against the plan:
  `tests/fixtures/onboard-write-apex7.manifest.txt` (generator `scripts/gg-write-frames.py`).
- **No read-back exists for an on-board write** on this family, so a write cannot be verified from the device except
  by reading the slot back with the read path and diffing. That cycle, the first hardware write, and the keyboard
  effects of `89` are open steps in `ONBOARD.md`.

### Windows reverse-engineering environment

- Tools: Wireshark (`C:\Program Files\Wireshark\tshark.exe`), USBPcap (`C:\Program Files\USBPcap\USBPcapCMD.exe`).
  `USBPcapCMD` without arguments is interactive and hangs; always pass `-d \\.\<capture device> -o <file> -A`.
- The keyboard was the captured USB address in past captures; re-check with tshark after changing ports.
- Useful tshark fields: `-e frame.time_epoch -e usb.data_fragment`; filter
  `usb.setup.bRequest==9 && usb.bmRequestType==0x21 && usb.setup.wValue==0x0300` (or `0x0200` for output reports).
- Stop GG: kill every `SteelSeries*` process, or the engine respawns. Restart from `C:\Program Files\SteelSeries\GG`
  with `SteelSeriesGGEZ.exe -dataPath="C:\ProgramData\SteelSeries\GG" -dbEnv=production`
  (`SteelSeriesGG.exe` alone exits with code 2).
- Do not launch GUI capture prompters with `-WindowStyle Hidden`; it hides the form.
- `ctx_execute_file` refuses paths outside the project; run analysis scripts with `python -I <script> <args>`.
- Capture artifacts (outside the repo) in the artifact kit: `blobs.bin` / `blobs3.bin`
  (reassembled 12,500-byte profile blobs, 13 each, 8 distinct) and `profile_crc_check.py <blob files>`. The original
  pcaps (engine capture 2, engine capture 3), step CSVs, `an*.py` scripts and capture prompters were gone on 2026-10-15; the
  blobs were recovered from an old session scratchpad. The Apex 7 **write** GG write capture (2026-10-20,
  677,636 bytes, sha256 <digest recorded outside the repo>, Apex 7 `0x1612` firmware 4.16.3, the captured USB address - re-check with tshark
  after changing ports) decodes to `tests/fixtures/onboard-write-apex7.manifest.txt` via
  `scripts/gg-write-frames.py`.

---

## Backlog

Tracked in `PLAN.md` only. Do not duplicate status here.
