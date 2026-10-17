# Development notes

Reverse-engineering and reference notes. Source code wins over every file here; command status lives in
`.claude/rules/experimental-protocol.md`, open work in `PLAN.md`.

| File | Content | State |
|------|---------|-------|
| [adding-hardware.md](adding-hardware.md) | How to capture a new device with GG and turn it into a PR | Current |
| [apex-pro-tkl-2023-gg-capture.md](apex-pro-tkl-2023-gg-capture.md) | Apex Pro TKL 2023 wired (`0x1628`) GG wire protocol: lighting, profile blob, slot map | Current (2026-10) |
| [apex-7-onboard-profile-read.md](apex-7-onboard-profile-read.md) | Legacy Apex on-board profile read: transfer map decoded from the capture's URBs, `0x85` pulls, `0x83` reads, reply framing, payload layout, Linux collection map | `[EXPERIMENTAL]`, capture 2026-10-15 (decoded from `hub-2.pcap` URBs); requests accepted on an Apex 7 on 2026-10-15; report builders moved to `src/devices/hid_reports.rs` (report number 0) 2026-10-17; read path lives in `keyboards/apex7.rs` (`Apex7`: read plan, transfers and polling on the device's own handle; the `apex_legacy_onboard` module it lived in separately was folded into `Apex7` on 2026-10-19); **a full slot read completes on an Apex 7 under Linux, 2026-10-17/18** (model, firmware and decoded bytes not recorded in the repo); the pull's answer is decoded (pacing token, 2026-10-17), the decoded payload layout and the checksum region are not verified; the read ends with a four-frame closing sequence (`69` OLED off, `89 <slot + 1>` deploy, `41 00` volatile, `6c 00 00` close driver mode), sent since 2026-10-18 (the driver-mode close since 2026-10-19) and never tested on hardware - engine-side reading, including the fact that **driver mode is a different frame in the Apex 3 family** (`0x03` in a 33-byte report on Apex 3, `0x6C` with a different opening byte on Apex 3 TKL), is in §Driver mode; the capture's single 64-byte `ff` frame is unaccounted for (kit's "idle" step) and is not sent |
| [devices.md](devices.md) | GG firmware product IDs, keyboard control endpoint, command codes with status | Checked against source 2026-10-15 |
| [linux-read-interval.md](linux-read-interval.md) | Interrupt-endpoint interval table from the 2026-10-16 Linux run capture (`linux-read.pcapng`): poll windows, answer latency, the gaps that per-report opens miss | Measured 2026-10-16; **the capture it measures is not in the tree** (only a 66-packet excerpt), so re-verification needs a new capture |
| [gamesense-api.md](gamesense-api.md) | ssgg GameSense server endpoints and CORS; observed GG responses | Checked against source 2026-10-15; GG part from 2026-05 |
| [database-schemas.md](database-schemas.md) | GG engine, Prism and Sonar SQLite schemas | Read 2026-05; per-key fields corrected 2026-10-15 |
