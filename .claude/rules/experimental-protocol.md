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

## Rules

**Adding new protocol code:** Label it `[EXPERIMENTAL]` or `[PLACEHOLDER]` — silence implies confirmed.

**Promoting experimental to confirmed:** Only after testing on physical hardware. Include device model and date in the commit. Remove the label.

**Never:** State experimental protocol as production-ready in docs or user-facing output. The `experimental-apex-2023`
feature flag was removed on 2026-10-15; experimental paths are labelled in code and docs instead of feature-gated.
Writes that could change persistent device state (the on-board profile) stay unwired until a read-back exists and the
user approves the first write.
