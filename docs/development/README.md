# Development notes

Reverse-engineering and reference notes. Source code wins over every file here; command status lives in
`.claude/rules/experimental-protocol.md`, open work in `PLAN.md`.

| File | Content | State |
|------|---------|-------|
| [adding-hardware.md](adding-hardware.md) | How to capture a new device with GG and turn it into a PR | Current |
| [apex-pro-tkl-2023-gg-capture.md](apex-pro-tkl-2023-gg-capture.md) | Apex Pro TKL 2023 wired (`0x1628`) GG wire protocol: lighting, profile blob, slot map | Current (2026-10) |
| [apex-7-onboard-profile-read.md](apex-7-onboard-profile-read.md) | Legacy Apex on-board profile read: transfer map decoded from the capture's URBs, `0x85` pulls, `0x83` reads, reply framing, payload layout, Linux collection map | `[EXPERIMENTAL]`, capture (decoded from the engine capture 2 URBs); requests accepted on an Apex 7 on 2026-10-15; report builders moved to `src/devices/hid_reports.rs` (report number 0) 2026-10-17; read path lives in `keyboards/apex7.rs` (`Apex7`, reads on the device's own handle) + `apex_legacy_onboard.rs`, not replayed since; reply framing and checksum region unresolved |
| [devices.md](devices.md) | GG firmware product IDs, keyboard control endpoint, command codes with status | Checked against source 2026-10-15 |
| [gamesense-api.md](gamesense-api.md) | ssgg GameSense server endpoints and CORS; observed GG responses | Checked against source 2026-10-15; GG part from 2026-05 |
| [database-schemas.md](database-schemas.md) | GG engine, Prism and Sonar SQLite schemas | Read 2026-05; per-key fields corrected 2026-10-15 |
