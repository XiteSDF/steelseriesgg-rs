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
| `onboard_profile::CMD_PULL_PAYLOAD` (legacy Apex) | `0x85` | Experimental — resource-buffer pull, sent as an **output report** (`SET_REPORT wValue 0x0200 wIndex 1 wLength 64`: report number 0, command byte in the data stage); frame `85 00 <fs u16 LE> <file offset u32 LE> <size u16 LE>`; 121 pulls decoded from the engine capture 2 URBs; accepted on an Apex 7 `0xFFC1` collection on 2026-10-15 |
| `onboard_profile::CMD_READ_PAYLOAD` (legacy Apex) | `0x83` | Experimental — payload read, sent as a **feature report** (`SET_REPORT wValue 0x0300 wIndex 1 wLength 642`), frame `83 00 <buffer offset u16 LE> <size u16 LE>`, size 512 (2 for the schema probe). The answer is the matching `GET_REPORT` reply: 642 bytes, report-ID byte `B5`-`B7`, then 20 framing bytes before the payload. That framing offset is `ReadConfig::reply_header` / `--reply-header`, and `onboard schema --sweep` finds it on real hardware. Linux: request 642 data bytes — 643 makes the control transfer time out. Full read never completed on hardware |
| `onboard_profile::config_frame` (legacy Apex) | pacing frame, output report 0 | Experimental — `SET_REPORT wValue 0x0200 wIndex 1 wLength 13` with data `00 \| 01 00 00 02 00 f4 01 60 00` = pacing `u8` 1 ms, `u16` 0, file access `u16` 2, USB timeout `u16` 500 ms, trailing `u16` 96 (meaning unknown); accepted on the Apex 7 `0xFFC1` collection on 2026-10-15, `EPIPE` on the other collections |
| `0x1612`-`0x1615` platform (`apex_legacy_onboard.rs`, `onboard_profile.rs`, feature `onboard`) | — | Experimental — on-board **read** for the legacy Apex family (Apex 7, Apex Raw, Apex 2014), reverse-engineered from the GG capture; transfer map in `docs/development/apex-7-onboard-profile-read.md`. Schema word `0x000B` for the reference keyboard's profile files; profile name at payload offset 18 in GG's own 12288-byte assembly and at offset 2 in the Windows engine decoder's layout; schema word at payload offset 0 reads `0x0000`, the assembly tail is zeros, and the CRC-32/STM32 of every region candidate (11648, 11696, 12288 bytes, as u32 and u16 words) matches none of them — the checksum region is unverified, so `onboard read` prints both words and `decode` never claims a profile is verified. **Never** write a profile: no read-back is known for this family and writes are not implemented |
| `onboard_profile::CMD_PUSH_PAYLOAD` (legacy Apex) | `0x05` | Placeholder — named for completeness; nothing sends it, there is no read-back for on-board writes. GG's write direction in the capture is report `03 00 <buffer offset u16> <size u16> <payload>` on the 642-byte feature report (144 transfers) |

## Rules

**Adding new protocol code:** Label it `[EXPERIMENTAL]` or `[PLACEHOLDER]` — silence implies confirmed.

**Promoting experimental to confirmed:** Only after testing on physical hardware. Include device model and date in the commit. Remove the label.

**Never:** State experimental protocol as production-ready in docs or user-facing output. The `experimental-apex-2023`
feature flag was removed on 2026-10-15; experimental paths are labelled in code and docs instead of feature-gated.
Writes that could change persistent device state (the on-board profile) stay unwired until a read-back exists and the
user approves the first write. The same rule covers the legacy Apex family: `ssgg onboard` reads the
on-board profile and writes nothing, `read` and `schema` require `--yes`, and `CMD_PUSH_PAYLOAD` stays unsent.
