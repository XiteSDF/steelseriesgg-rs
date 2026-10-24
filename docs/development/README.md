# Development notes

Reverse-engineering and reference notes. Source code wins over every file here; command status lives in
`.claude/rules/experimental-protocol.md`, open work in `PLAN.md`.

| File | Content | State |
|------|---------|-------|
| [adding-hardware.md](adding-hardware.md) | How to capture a new device with GG and turn it into a PR | Current |
| [apex-7-gg-capture.md](apex-7-gg-capture.md) | Apex 7 wired (`0x1612`) GG wire protocol: read/write frames, payload layout, checksum; the other Apex keyboards of this legacy platform share it | Frames confirmed on hardware 2026-10-20; payload decoded and encoded by the crate's codec; the macro record grammar measured 2026-10-28 (four-byte key and mouse actions, a key binds the record at block offset 4 x its binding byte, the mouse mask named to mouse 8 on the engine converter's `log2(mask) + 1` rule) and `[EXPERIMENTAL]`, macro header fields and action kinds read out of the engine binary 2026-10-28 (mode = GG playback type + 1, accepted `1..=5`; the repeat pair is used by modes 2, 3, 4), the UI name of each mode open, the OLED array uninterpreted by the codec (the GUI rasterises it and decodes an uploaded BMP into those bytes, GUI-side only, no hardware claim); the 4.16.8 firmware package read 2026-10-23 (container layout, the five profile slots and the five `0x030N` blocks shipped zeroed, the board's own menu vocabulary naming its illumination and display options and the timeout ladder `idle_timeout_ms` carries), the six OLED illumination bytes at payload offset 27 still unnamed, and they are the board's own settings - GG's Apex 7 UI offers no OLED option beyond the image upload (operator-reported 2026-10-23), the keyboard's `DISPLAY` menu carries brightness, icons and timeout |
| [apex-pro-tkl-2023-gg-capture.md](apex-pro-tkl-2023-gg-capture.md) | Apex Pro TKL 2023 wired (`0x1628`) GG wire protocol: lighting, profile blob, slot map | Current (2026-10) |
| [devices.md](devices.md) | GG firmware product IDs, keyboard control endpoint, command codes with status | Checked against source 2026-10-20 |
| [gamesense-api.md](gamesense-api.md) | ssgg GameSense server endpoints and CORS; observed GG responses | Checked against source 2026-10-15; GG part from 2026-05 |
| [database-schemas.md](database-schemas.md) | GG engine, Prism and Sonar SQLite schemas | Read 2026-05; per-key fields corrected 2026-10-15 |
