# Experimental Protocol Rules

Protocol work has three states. Never mix them up or upgrade a status without hardware evidence.

## Status table

| Status | Meaning | Doc label |
|--------|---------|-----------|
| Confirmed | Tested on real hardware | (none) |
| Experimental | Reverse-engineered, untested | `[EXPERIMENTAL]` |
| Placeholder | Code exists, actual command unknown | `[PLACEHOLDER]` |

## Current command code status

| Constant | Code | Status |
|----------|------|--------|
| `CommandCode::PerKeyRgb` | `0x23` | Placeholder — actual per-key command unknown |
| `CommandCode::Apex2023Direct` | `0x40` | Experimental — one solid-colour frame replayed on `0x1628` (2026-10-14); per-key, other colours untested |
| `APEX_GEN1_PACKET_ID_DIRECT` (`apex_pro_tkl_2023.rs`) | `0x3A` | Experimental — original Apex Pro `0x1610`, from OpenRGB, untested |
| `Apex8ZoneBrightnessCommand` | `0x23` | Experimental — Apex 3 TKL brightness, from OpenRGB, untested |
| `CommandCode::ProfileChunk` / `ProfileChunkCommand` | `0x03` (`03 01 80`) | Experimental — profile upload chunk; checksum matches captures; never sent (no read-back) |
| `CommandCode::ActuationControl` | `0x2D` | Experimental — firmware command unknown |
| `hid_reports::OnboardPullCommand` / `CommandCode::OnboardPull` (legacy Apex) | `0x85` | Experimental — resource-buffer pull, built by the typed command and sent as an **output report** (`SET_REPORT wValue 0x0200 wIndex 1 wLength 64`: report number 0, command byte in the data stage); frame `85 00 <fs u16 LE> <file offset u32 LE> <size u16 LE>`; 121 pulls decoded from the engine capture 2 URBs; accepted on an Apex 7 `0xFFC1` collection on 2026-10-15 |
| `hid_reports::OnboardReadCommand` / `CommandCode::OnboardPayloadRead` (legacy Apex) | `0x83` | Experimental — payload read, built by the typed command and sent as a **feature report** (`SET_REPORT wValue 0x0300 wIndex 1 wLength 642`), frame `83 00 <buffer offset u16 LE> <size u16 LE>`, size 512 (2 for the schema probe). **The window is read back by a separate control-IN transfer the host must issue**: GG sends `GET_REPORT bmRequestType 0xa1 bRequest 0x01 wValue 0x0300 wIndex 1 wLength 642` after **every** `83` request (241/241 in the engine session capture), and the device answers with a 643-byte data stage (report-ID byte `03`, payload directly behind it — framing length 0 = `READ_REPLY_FRAMING_LEN` = `ReadConfig::reply_header` / `--reply-header`; the candidate framings 0, 1, 2, 5, 6, 14, 19, 20, 21, 36 are probed on real hardware with short `onboard read --reply-header N` runs). Linux: request **642** data bytes (hidapi `get_feature_report`, i.e. the kernel's `HIDIOCGFEATURE(642)`; the getter exists in the `linux-native` backend this crate builds, `hidapi-2.6.7/src/linux_native/ioctl.rs:26` — the 2026-10-17 note "hidapi 2.6.7 exposes no getter on Linux" was wrong) — 643 makes the control transfer time out. Report number matters: the kernel builds `wValue` from the buffer's first byte, so since 2026-10-17 every onboard buffer built in `hid_reports` starts with report number **0** (`ONBOARD_OUTPUT_REPORT_SIZE` 65 = report number + 64 data bytes → `wValue 0x0200 wLength 64`; `ONBOARD_FEATURE_REPORT_SIZE` 643 → `wValue 0x0300 wLength 642`; `READ_REPLY_ID = 0x00` asks GG's `wValue 0x0300`), which is what GG asks. Before that the command byte was used as the report number (`0x0290`, `0x0285`, `0x0383`) and the run issued no read-back at all; the kernel validates GET_REPORT against the interface's report descriptor, and asking interface 1 (`0xFFC0`) is not measured yet. Full read never completed on hardware |
| `hid_reports::ONBOARD_INIT_CHAIN` (`OnboardInitCommand`, `CommandCode::{FirmwareVersion,EngineStatus,OpenFiles}`) (legacy Apex) | `0x90`, `0x90`, `0xf4`, `0x6c`, `0x90`, `0x90` | Experimental — GG's six 64-byte `wValue 0x0200 wIndex 1` output reports, 11 ms apart, decoded from the engine session capture URBs: `90 00 …`, `90 01 …`, `f4 00 …`, `6c 00 01 …`, `90 00 …`, `90 01 …`. The `90` polls are answered on the interrupt endpoint with the ASCII firmware version (`4.16.8` in the capture), `f4 00` answers `00 01 00 …`, `6c 00 01` is not answered; meanings unknown, `ssgg onboard read` sends the chain and prints the version it gets, `--no-init-chain` skips it. **No 13-byte "pacing frame" exists on the wire** — an earlier version of this table listed `01 00 00 …` at `wLength 13`; it was a misreading of the capture's text export, and `config_frame` was removed 2026-10-16 |
| `0x1612`-`0x1615` platform (`keyboards/apex7.rs` device, `apex_legacy_onboard.rs`, `onboard_profile.rs`, feature `onboard`) | — | Experimental — on-board **read** for the legacy Apex family (Apex 7, Apex Raw, Apex 2014), reverse-engineered from the GG capture; transfer map in `docs/development/apex-7-onboard-profile-read.md`. Schema word `0x000B` for the reference keyboard's profile files, name at payload offset 2 (the Windows engine decoder and the replies agree). The payload-read reply is a **file head, not a window**: 241 reads return only 33 distinct payloads — 179 have an all-zero payload (180 with at most two non-zero bytes), the schema word `0b 00` appears in 6 replies and a profile name in five (`Config 1`..`Config 5`, once each, not `Config 1` for every file), and the requested buffer and file offsets do not change the answer; GG never walks the profile either (its 241 requests are three byte-identical shapes: file 1 window 2 121×, file 2 window 2 120×) and the assembly has no checksum word (CRC-32/STM32 over the 11648, 11696 and 12288-byte regions, as u32 and u16 words, matches nothing) — `onboard read` prints both words and `decode` never claims a profile is verified. **Never** write a profile: no read-back is known for this family and writes are not implemented |
| `onboard_profile::CMD_PUSH_PAYLOAD` (legacy Apex) | `0x05` | Placeholder — named for completeness; nothing sends it, there is no read-back for on-board writes. GG's write direction in the capture is report `03 00 <buffer offset u16> <size u16> <payload>` on the 642-byte feature report (144 transfers) |

## Rules

**HID transport:** transfers go through hidapi (`write`, `send_feature_report`, `get_feature_report`, `read_timeout`).
The `linux-native` backend in the pinned 2.6.7 issues the kernel's own `HIDIOCSOUTPUT` / `HIDIOCSFEATURE` /
`HIDIOCGFEATURE` (`hidapi-2.6.7/src/linux_native/ioctl.rs:20,26,38`), and the direction-bit bug the earlier native backend
sent was fixed in 2.6.6 — do not add hand-rolled `ioctl` / `write(2)` paths. Known open issue: `usbhid` polls a
collection's interrupt endpoint only while a descriptor on that node is open, so per-call opens lose interrupt reports
that arrive while the node is closed (`docs/development/apex-7-onboard-profile-read.md`, "Input reports: the node must be
open when the report arrives"); a session handle was tried and reverted 2026-10-17.

**Adding new protocol code:** Label it `[EXPERIMENTAL]` or `[PLACEHOLDER]` — silence implies confirmed.

**Promoting experimental to confirmed:** Only after testing on physical hardware. Include device model and date in the commit. Remove the label.

**Never:** State experimental protocol as production-ready in docs or user-facing output. The `experimental-apex-2023`
feature flag was removed on 2026-10-15; experimental paths are labelled in code and docs instead of feature-gated.
Writes that could change persistent device state (the on-board profile) stay unwired until a read-back exists and the
user approves the first write. The same rule covers the legacy Apex family: `ssgg onboard` reads the
on-board profile and writes nothing, `read` and `schema` require `--yes`, and `CMD_PUSH_PAYLOAD` stays unsent.
