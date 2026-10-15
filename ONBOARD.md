Above all, respect the original repository *.md files guidelines.

Task: implement onboard profile read packet for Apex 7 keyboard on the `main` branch.

Git status: `main` branch is checked out. `onboard-configs`, `onboard-configs-dev` and `qwen` branches should be ignored.

Useful data:
`docs/adding-hardware.md` - guidelines for adding hardware.
capture notes - capture as instructed in guidelines.
`lisp` - lisp descriptors for various steelseries devices dumped from Windows engine (should not be referenced in code or comments).
`src/devices/keyboards/apex_pro_tkl_2023_profile.rs` - experimental implementation for Apex Pro TKL 2022 keyboard (year not a typo, that's how it's named in Windows engine files).

Additional environment:
Ghidra running MCP server with SteelSeries Engine Windows executable and device firmware loaded.

Moved from `PLAN.md`:

## Apex 7 on-board profile read (`onboard` feature, hardware validation)

Implemented 2026-10-15 from the GG capture: pacing frame, `0x85` pulls, `0x83` payload reads,
checksum, payload decode, `ssgg onboard plan|read|schema|decode`. Every protocol byte in it is
`[EXPERIMENTAL]` - nothing has been replayed on hardware. Read-only by design: the push command is named
and never sent. Wire format, layout table and open questions: `docs/development/apex-7-onboard-profile-read.md`.

1. **[HW] Path probe (Linux, Apex 7, GG stopped):** `ssgg onboard schema --yes`. It answers "is the
   profile protocol on the control collection?" If every candidate path times out, the protocol is on a
   different collection and the ranking in `apex_legacy_onboard::candidate_paths` is wrong.
2. **[HW] Schema words of all five slots:** `ssgg onboard schema --yes` prints the word of slot 0 plus the
   lighting file; sweep `--profile 0..4`. Schema `0x000A` / `0x000B` means the decode applies; anything
   else is a third schema - record it, do not guess.
3. **[HW] Full read of one slot:** `ssgg onboard read --yes --profile 0 --out /tmp/p0.bin --json /tmp/p0.json`.
   Check `checksum: stored ..., computed ..., match: true` - a match validates the CRC and the 12 x 1024 split.
4. **[HW] Reproducibility:** `ssgg onboard read --yes --profile 0 --out /tmp/p0b.bin --verify`, then
   `--gap-ms 0`, `2`, `10` to find the lowest gap the keyboard tolerates; keep the measured value as the default.
5. **[HW] Layout check:** decode `/tmp/p0.bin` (`ssgg onboard decode`) and compare name, lighting colours,
   key mappings and macro count with what GG shows for that slot. This is the only way to settle the inferred
   offsets (GUID, OLED, Rapid Tap, macro) and the 260-byte tail.
6. **[HW] Windows:** same read through the hidapi path (no udev rule needed); confirms the interface choice for
   the control collection.
7. Only after 1-4: extend to the lighting files (`0x0301`-`0x0305`, reply report ID `0x03`), then promote what
   worked to confirmed in `.claude/rules/experimental-protocol.md` with device model and date.