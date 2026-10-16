Above all, respect the original repository *.md files guidelines.

Task: implement onboard profile read packet for Apex 7 keyboard on the `main` branch.

Git status: `onboard-configs-dev` branch is checked out. `onboard-configs` and `qwen` branches should be ignored.

Useful data:
`docs/adding-hardware.md` - guidelines for adding hardware.
capture notes - capture as instructed in guidelines.
`lisp` - lisp descriptors for various steelseries devices dumped from Windows engine (should not be referenced in code or comments).
`src/devices/keyboards/apex_pro_tkl_2023_profile.rs` - experimental implementation for Apex Pro TKL 2022 keyboard (year not a typo, that's how it's named in Windows engine files).

Additional environment:
Ghidra running MCP server with SteelSeries Engine Windows executable and device firmware loaded.

Moved from `PLAN.md`:

## Apex 7 on-board profile read (`ssgg onboard`, hardware validation)

Implemented 2026-10-15 from the GG capture: a `0x90` firmware-version poll, `0x85` pulls,
`0x83` payload reads, checksum, payload decode, `ssgg onboard read|decode`. Only the profile slot is
selectable (`--profile 0..4`); the geometry is compile-time constants. The 13-byte "pacing frame" earlier
versions sent is not in the capture and was removed, and so was GG's six-frame opening chain (`90 00`,
`90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01`), removed 2026-10-17: the read sends the single `90 00`
poll it needs, because on hardware `90 00` and `90 01` answer identically and `f4 00` and `6c 00 01`
have no known meaning. Every protocol byte in it is `[EXPERIMENTAL]`. Read-only by design: the push
command is named and never sent. Wire format, layout table and open questions:
`docs/development/apex-7-onboard-profile-read.md`.

**Read status (2026-10-18):** `ssgg onboard read --yes --profile N` completes a full slot read on an Apex 7
under Linux. That closes the transport questions (steps 1, 1a, 1b) and the framing question (step 2). It is
reported without the run log, so record the model, firmware, slot contents and the checksum line from the run
here before anything is promoted to confirmed - the label stays until then. Steps 3-5 (slot by slot, the
double read, the layout compare with GG) are what is left.

0. **[HW] Firmware poll:** `ssgg onboard read --yes --profile 0` starts with the single `90 00` poll and
   prints its answer in the `firmware:` line. **Measured 2026-10-17:** an Apex 7 wired answered `90 00` and
   `90 01` the same way, so one poll is all the read needs; `f4 00` and `6c 00 01` (the other two frames of
   GG's chain) are not sent and their meanings are unknown.

0a. **[HW] The pull is pacing, not a data transfer:** **Measured 2026-10-17** at the endpoint level of the
   capture (engine session capture): every one of the 121 pulls is answered 1.7-3.7 ms later by the keyboard's ordinary
   input report `0x01` (63 bytes, all zero = no keys pressed), and the device's own read spec keeps **no**
   bytes of it (byte specifier `(0 0 0)` in `apex_7.lisp`). So the packet GG reads after a pull is a flow-control
   token; every profile byte arrives through the `0x83` window replies. `ssgg onboard read` does the same
   (`read_pull_ack` consumes one packet and inspects none of it, `PULL_ACK_LEN` bytes). The drain is not
   optional on Linux: a run that
   skipped it failed every payload read-back with `io::read_exact: unexpected end of file`. The capture kit's
   decoded response listing lists the
   121 pulls as 64-byte "control responses" starting `85 00 …` - those are the requests GG submitted, which is
   why pull replies were earlier described as constant 64-byte frames.

Measured 2026-10-16 by comparing the GG capture with the ssgg run of 2026-10-15 on the same keyboard: the
chain, the chain answers, the pull header and the `83` header all match, and the frames we send are answered
exactly like GG's. What we never sent is the **read-back**: GG issues `GET_REPORT 0xa1 0x01 wValue 0x0300
wIndex 1 wLength 642` after every read request and takes the window from that control-IN data stage; our
run sent none, so it collected nothing but 64-byte interrupt status. That is step 1a.

1. **[HW] Path probe (Linux, Apex 7, GG stopped) - answered 2026-10-17/18:** the profile protocol is on the
collection the device manager opened for the keyboard: the read runs on the `Apex7` device handle and reads the
profile there, and `ssgg devices` shows which collection that is (`0xFFC1`, interface 4, `the hidraw node` on that box -
not the interface 1 (`wIndex 1`) the Windows capture used). The read does not probe nodes by path.
1a. **[HW] Feature-report read-back - done 2026-10-17/18:** the read asks for the reply the way GG does — report
   number **0**, 642 bytes. `HIDIOCGFEATURE` builds `wValue` from the
   buffer's first byte, and since 2026-10-17 every report buffer built in `src/devices/hid_reports.rs`
   starts with report number **0** (`READ_REPLY_ID = 0x00`), which is GG's `wValue 0x0300` / `wValue 0x0200`;
   the run asked the command byte as the report number (`0x0290`, `0x0285`, `0x0383`) and got no
   read-back, and
   the kernel validates the request against that interface's report descriptor. **642**, never 643 (643 is
   the `ETIMEDOUT` of 2026-10-15). The read-back is what the 2026-10-18 profile reads collected.
1b. **[HW] hidapi as the transport (code 2026-10-17, verified by the 2026-10-17/18 reads):** the hand-rolled raw-ioctl and
   `write(2)` paths are deleted; every transfer is a hidapi call (`send_feature_report`, `get_feature_report`,
   `write`, `read_timeout`) on the handle the device manager opened for the keyboard. On Linux the `linux-native` backend issues
   the kernel's own `HIDIOCSFEATURE`/`HIDIOCGFEATURE`/`HIDIOCSOUTPUT` (`hidapi-2.6.7/src/linux_native/ioctl.rs:20,26,38`),
   the direction-bit bug was fixed in 2.6.6 and 2.6.7 is pinned, so the `#[cfg(unix)]` split is gone. Output frames
   therefore leave as `SET_REPORT wValue 0x0200` control transfers, matching the reference capture, instead of a
   `write(2)` to an OUT endpoint the profile collection does not have. **Done 2026-10-17**: the per-call
   path-open helpers (`*_report_raw`) are removed and the read runs on the device's own handle, which is open for the whole
   read; `usbhid` polls the interrupt endpoint only while a descriptor on the node is open, which is why per-report opens
   missed the firmware-poll and pull answers (measured in
   `docs/development/apex-7-onboard-profile-read.md`, "Input reports: the node must be open when the report arrives").
   The 2026-10-15 text `Inappropriate ioctls (errno 110)` is not evidence of what the kernel answered - that path
   formatted `last_os_error()` after unrelated calls on the same thread.
2. **[HW] Reply framing - answered 2026-10-17/18:** fixed at [`READ_REPLY_FRAMING_LEN`] = 0, the offset the
capture's replies show - `--reply-header` and the short-read framing sweep are gone, so a keyboard that anchors its payload
   elsewhere is a code change, not a flag. The profile bytes a read collects are the evidence: a schema word
   (`0b 00`) with a readable name (`Config 1` .. `Config 5`) behind the report-ID byte is framing 0; anything
   else is recorded here and becomes the constant.
3. **[HW] Slot by slot, then the checksum line:** the read of one slot works (2026-10-17/18). Remaining:
`--profile 1..4` - every read
   prints that slot's schema word. Schema `0x000A` / `0x000B` means the decode applies; anything else is a
   third schema - record it, do not guess. A `checksum: stored ..., computed ..., match:
   true` would validate the CRC and the 12 x 1024 split at once; the capture does not predict it, because the
   payload-read reply is a file head that repeats the head (`0b 00` + the name) and never the requested
   window - 241 GG reads give 33 distinct payloads, 179 of them all zero.
4. **[HW] Reproducibility:** `ssgg onboard read --yes --profile 0 --out /tmp/p0b.bin --verify`. Pacing is
   `GG_GAP_MS`, a constant: a gap ladder needs a code change, and whatever the keyboard tolerates is recorded
   there.
5. **[HW] Layout check:** decode `/tmp/p0.bin` (`ssgg onboard decode`) and compare name, lighting colours,
   key mappings and macro count with what GG shows for that slot. This is the only way to settle the inferred
   offsets (GUID, OLED, Rapid Tap, macro) and the 260-byte tail.
6. **[HW] Windows:** same read through the hidapi path (no udev rule needed); confirms the interface choice for
   the control collection.
7. Only after 1-4: extend to the lighting files (`0x0301`-`0x0305`, reply report ID `0x03`), then promote what
   worked to confirmed in `.claude/rules/experimental-protocol.md` with device model and date.

---

## Code layout (2026-10-17)

| Piece | Where |
|-------|-------|
| Wire types: `0x85` pull, `0x83` read, `0x90` firmware poll, the 643-byte read buffer | `src/devices/hid_reports.rs` (`OnboardPullCommand`, `OnboardReadCommand`, `OnboardFirmwareVersionCommand`) — not feature-gated, no raw byte arrays outside it. `OnboardInitCommand`/`ONBOARD_INIT_CHAIN` are gone (2026-10-17, see the firmware-poll row of `.claude/rules/experimental-protocol.md`): the chain's `f4 00` and `6c 00 01` stay documented as `CommandCode::EngineStatus`/`OpenFiles`, meanings unknown, sent by nothing |
| Read geometry, reply parsing, assembly, decode | `src/devices/keyboards/apex_legacy_profile.rs` (`PULLS_PER_FILE`, `DEVICE_RESOURCE_BUFFER_LEN`, `PAYLOAD_READ_LEN`, `PROFILE_FILE_BYTES`, `READS_PER_PULL`, `profile_file_id`, `validate_profile`, `parse_read_reply`, `ProfileImage`, `decode`), core code. It was `src/onboard_profile.rs` until 2026-10-17, when it moved into the legacy Apex namespace: its constants are all `apexui.dll` facts of the legacy ("macho") platform - reply framing, resource-buffer size, payload sizes, schema words, struct offsets - and there is nothing there another family can share. The only knob is the profile slot: pull count, buffer size, window size, pacing and reply framing are constants, the read loop builds every frame from the typed commands, and it is also what logs the geometry - the CLI prints none, so no legacy constant appears in `src/main.rs`. Renamed 2026-10-18 so the names say **fact**, not **default**: `DEFAULT_CHUNKS` → `PULLS_PER_FILE`, `READ_LEN` → `PAYLOAD_READ_LEN`, `RESOURCE_BUFFER_LEN` → `DEVICE_RESOURCE_BUFFER_LEN`, `PROFILE_FILE_LEN` → `PROFILE_FILE_BYTES`, `SCHEMA_B_FIRMWARE` → `SCHEMA_B_FIRMWARE_VERSION` (documentation only - the read takes its schema from the word it reads). Deleted as unused: `PROFILE_FS_LEN` (the 0xA000 flash region), `PULL_REPORT_LEN`, `INIT_REPORT_LEN`, `LIGHTING_FS_SHARED` |
| Packet I/O | `src/devices/keyboards/mod.rs`: `GenericKeyboard` does the profile packet I/O on the `device` handle it already owns - `connected_device` (the one handle accessor, shared with the lighting sends, same "Device not connected" message), `write_profile_packet` (its own output write - the lighting write path runs the hid optimizer, which would drop the repeated frames of a read), `send_feature`, `read_profile_feature_packet`, `read_profile_input_packet`, core code; `apex_legacy_onboard` runs the read plan against `&GenericKeyboard` on a scoped thread (`std::thread::scope`, 2026-10-18, which replaced `spawn_blocking` fed by an `unsafe` transmute to `&'static` - the scoped borrow is checked, and the read blocks the task's thread for its ~2 s). The plan takes the slot only: `strict` (no flag fed it) and the `deadline` reply-budget plumbing (always `None`) are gone, one payload read has one fixed `REPLY_BUDGET_MS`. No transport type: `OnboardTransport` with its `HandleTransport` and `PathTransport`, and the per-call path-open helpers (`send_feature_report_raw`, `read_feature_report_raw`, `write_output_report_raw`, `read_input_report_raw`) were all removed on 2026-10-17; Apex 7 wired works through hidapi |
| Keyboard device | `src/devices/keyboards/apex7.rs`: `Apex7` wraps `GenericKeyboard` (its `HidReportBuilder` and the open `HidDevice`) and runs the read on its own handle; `onboard_family(pid)` selects `0x1612`, `0x1614`, `0x1616`, `0x1618` and `discovery::open_keyboard` wraps those PIDs in it - the read path is core code, labelled `[EXPERIMENTAL]`, so a family board keeps standard lighting in every build |
| Device API | `Keyboard::supports_onboard_profile`, `Keyboard::read_onboard_profile` (`[EXPERIMENTAL]`, core code); every other keyboard answers "not supported" |
| CLI | `ssgg onboard read / decode` (`src/cli.rs`, `src/main.rs`); `read --profile 0..4` runs on the `Apex7` device (its own handle, `--device vid:pid` selects the keyboard) and takes `--out`, `--json`, `--hex`, `--verify`, `--yes` - every other number of the read is a protocol constant, not a flag, and the CLI prints none of them; the geometry - slot, file id, pulls, payload reads - is logged by the read loop at `info` level, so `ssgg --debug onboard read` shows it before the first transfer; `decode` works on a saved image and touches no device |

Two facts from the first Linux run (2026-10-15, Apex 7 `the hidraw node`) that shape the transport:

- Discovery opens the interface-4 collection (`usage 0xFFC1`) for this keyboard, not the interface-1
  collection the capture's transfers ran on: the 2026-10-15 probe tried `1-3:1.4/input5`
  (`is_collection = true`), and the interface-1 node had no `input*` at all that day.
- `usbhid` polls a collection's interrupt endpoint only while a descriptor on that node is open. A
  per-report open (`/dev/hidrawN` opened for one report) has nowhere to deliver the firmware-poll and
  pull answers, so `read_interrupt_report` saw none. The read avoids that: `Apex7` does its own packet
  I/O on the handle the device manager opened, and that handle stays open for the whole read.
