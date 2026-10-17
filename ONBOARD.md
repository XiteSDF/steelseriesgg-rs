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
versions sent is not in the capture and was removed. GG's six-frame opening chain (`90 00`,
`90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01`) was removed on 2026-10-17 as unnecessary (only the `90`
polls were known to answer) and **restored on 2026-10-18: the read sends all six (`ONBOARD_INIT_CHAIN`),
because a read that sends only the `90 00` poll is answered on the wire and its payload replies come back as
scrambled bytes.** `f4 00` and `6c 00 01` have no known meaning, and which frame of the six the keyboard
needs is open. Every protocol byte in it is `[EXPERIMENTAL]`. Read-only by design: the push
command is named and never sent. Wire format, layout table and open questions:
`docs/development/apex-7-onboard-profile-read.md`.

**Read status (2026-10-18):** `ssgg onboard read --yes --profile N` completes a full slot read on an Apex 7
under Linux. That closes the transport questions (steps 1, 1a, 1b) and the framing question (step 2). It is
reported without the run log, so record the model, firmware, slot contents and the checksum line from the run
here before anything is promoted to confirmed - the label stays until then. The chain is part of that
finding: the read only produces readable payload bytes with `ONBOARD_INIT_CHAIN` sent, and it opens with GG's
2-byte file-head probe as a schema gate (`Apex7::probe_schema`, step 0c). Steps 3-5 (slot by
slot, the double read, the layout compare with GG) and the chain question (which frame arms the resource
buffer) are what is left.

0. **[HW] Opening chain and firmware poll:** `ssgg onboard read --yes --profile 0` starts with GG's six-frame
   chain (`90 00`, `90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01`, 11 ms apart) and prints the firmware version
   answered by the first `90 00` in the `firmware:` line. **Measured 2026-10-17:** an Apex 7 wired answered
   `90 00` and `90 01` the same way. **Measured 2026-10-18:** the chain is required - with only the `90 00`
   poll the read is answered and the payload comes back scrambled, so all six frames are sent; the meanings of
   `f4 00` and `6c 00 01` are still unknown. **Measured 2026-10-18 as well:** the chain is answered **one frame
   at a time** - a chain run that drained only the first answer collected the answers of `90 00` and `90 01`
   and nothing from frame 3 on, with all six frames on the wire - so `send_init_chain` reads one input report
   after every frame (GG's `read-chunk HIDIO 65`, `GG_INPUT_READ_BYTES`). Why an unread answer silences the
   frames after it is inference, not measured. **The write path is part of this step (2026-10-18):** the chain
   goes out through `GenericKeyboard::write_profile_packet` - the write with no cache - precisely so that all six
   frames reach the device. The keyboard's ordinary output write, `send_report`, goes through
   `write_padded_report` + `HidOptimizer`, which drops any report byte-identical to one sent in the last 50 ms,
   and chain frames 5 and 6 (`90 00`, `90 01`, 20 ms behind their first copy) fall in that window: on
   `send_report` the keyboard sees a four-frame chain and `--debug` logs the drop as "Skipping duplicate HID
   report". `send_report` was tried for the read on 2026-10-18 and reverted the same day. Test the frame
   sub-question with `write_profile_packet` - that is the write that puts the frames on the wire as measured.

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

0b. **[HW] Which chain frame arms the resource buffer:** drop the chain frames one at a time (a code change
    in `ONBOARD_INIT_CHAIN`) on a run that reads a slot whose contents you know from GG, and record here
    which frame the keyboard needs. Today all six frames are sent, and the answer is what lets the read drop
    the ones that do nothing. **Drain one input report after every frame while testing:** a frame set tested
    without draining looks dead when the only fault is the unread answer (step 0, 2026-10-18). Which frame
    arms the buffer is still unmeasured.

0c. **[HW] Schema gate (done 2026-10-18):** the read runs GG's 2-byte file-head probe as its first step
    (`Apex7::probe_schema`: 2-byte pull at file offset 0, then one window read at buffer offset 0, exactly
    the pair the capture's transfer map lists) and **refuses the read when the schema word at payload offset 0
    is not `0x000B`**: schema B is what `apex_legacy_profile` decodes and what has been measured, so a device
    answering schema A (or anything else) is stopped before anything is pulled, rather than later as a profile
    whose checksum never matches. Measured: the probe's schema word is read on an Apex 7. **Unmeasured:** a
    schema A device's behaviour, and whether the probe is load-bearing for the read at all (drop it and see -
    the same kind of test as step 0b).

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
   number **0**, 642 bytes (the size a reply can carry: `MAX_READ_LEN` 636 payload bytes behind the report-ID
   byte, plus the 5-byte header a header-echoing reply spends - see `docs/development/linux-read-interval.md` §5).
   The schema gate uses the same read on the probe's 2-byte pull (step 0c). `HIDIOCGFEATURE` builds `wValue` from the
   buffer's first byte, and since 2026-10-17 every report buffer built in `src/devices/hid_reports.rs`
   starts with report number **0** (`READ_REPLY_ID = 0x00`), which is GG's `wValue 0x0300` / `wValue 0x0200`;
   the run asked the command byte as the report number (`0x0290`, `0x0285`, `0x0383`) and got no
   read-back, and
   the kernel validates the request against that interface's report descriptor. **642**, never 643 (643 is
   the `ETIMEDOUT` of 2026-10-15). The read-back is what the 2026-10-18 profile reads collected.
   The length rules differ between the two directions and the kernel is why: a feature-report **write**
   buffer carries a leading report-number byte the kernel strips, so the 643-byte `ONBOARD_FEATURE_REPORT_SIZE`
   buffer goes out as `wLength 642`; a feature-report **read** sends the requested length verbatim
   (`HIDIOCGFEATURE` gets the slice length from hidapi, `src/linux_native/ioctl.rs:26`), so `READ_REPORT_LEN`
   (642) is both the ask and the wire number, and the reply arrives as `[report ID][641 bytes]`. The read's
   5-byte header lives inside those 641, so the payload a read may ask for tops out at **636**
   (`MAX_READ_LEN` = `ONBOARD_MAX_READ_LEN`): an earlier constant said 641, which subtracted the report-ID
   byte but not the header.
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
| Wire types: `0x85` pull, `0x83` read, `0x90` firmware poll, the opening chain, the 643-byte read buffer | `src/devices/hid_reports.rs` (`OnboardPullCommand`, `OnboardReadCommand`, `OnboardFirmwareVersionCommand`, `OnboardInitCommand` + `ONBOARD_INIT_CHAIN`) — not feature-gated, no raw byte arrays outside it. The chain is `90 00`, `90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01` and the read sends all of it: a read with only the `90 00` poll answers with scrambled payload bytes (Apex 7, 2026-10-18). `CommandCode::EngineStatus` (`f4 00`) and `CommandCode::OpenFiles` (`6c 00 01`) name the chain's two unexplained frames; they were removed 2026-10-17 and restored 2026-10-18 |
| Read geometry, reply parsing, assembly, decode | `src/devices/keyboards/apex_legacy_profile.rs` (`PULLS_PER_FILE`, `DEVICE_RESOURCE_BUFFER_LEN`, `PAYLOAD_READ_LEN`, `PROFILE_FILE_BYTES`, `READS_PER_PULL`, `profile_file_id`, `validate_profile`, `parse_read_reply`, `ProfileImage`, `decode`), core code. It was `src/onboard_profile.rs` until 2026-10-17, when it moved into the legacy Apex namespace: its constants are all `apexui.dll` facts of the legacy ("macho") platform - reply framing, resource-buffer size, payload sizes, schema words, struct offsets - and there is nothing there another family can share. The only knob is the profile slot: pull count, buffer size, window size, pacing and reply framing are constants, the read loop builds every frame from the typed commands, and it is also what logs the geometry - the CLI prints none, so no legacy constant appears in `src/main.rs`. Renamed 2026-10-18 so the names say **fact**, not **default**: `DEFAULT_CHUNKS` → `PULLS_PER_FILE`, `READ_LEN` → `PAYLOAD_READ_LEN`, `RESOURCE_BUFFER_LEN` → `DEVICE_RESOURCE_BUFFER_LEN`, `PROFILE_FILE_LEN` → `PROFILE_FILE_BYTES`, `SCHEMA_B_FIRMWARE` → `SCHEMA_B_FIRMWARE_VERSION` (documentation only - the read takes its schema from the word it reads). Deleted as unused: `PROFILE_FS_LEN` (the 0xA000 flash region), `PULL_REPORT_LEN`, `INIT_REPORT_LEN`, `LIGHTING_FS_SHARED` |
| Packet I/O | `src/devices/keyboards/mod.rs`: `GenericKeyboard` does the profile packet I/O on the `device` handle it already owns - `connected_device` (the one handle accessor, shared with the lighting sends, same "Device not connected" message), `write_profile_packet` (the write the read uses: no cache, so the chain's repeated frames go out; `send_report` runs the hid optimizer, which drops a report identical to one sent in the last 50 ms - see the write-path note in step 0), `send_feature`, `read_profile_feature_packet`, `read_profile_input_packet`, core code; `Apex7` runs the read plan - `read_image`, `send_init_chain`, `probe_schema`, `pull_window`, `read_window` - on its own handle on a scoped thread (`std::thread::scope`, 2026-10-18, which replaced `spawn_blocking` fed by an `unsafe` transmute to `&'static` - the scoped borrow is checked, and the read blocks the task's thread for its ~2 s). The plan takes the slot only: `strict` (no flag fed it) and the `deadline` reply-budget plumbing (always `None`) are gone, one payload read has one fixed `REPLY_BUDGET_MS`. No transport type: `OnboardTransport` with its `HandleTransport` and `PathTransport`, and the per-call path-open helpers (`send_feature_report_raw`, `read_feature_report_raw`, `write_output_report_raw`, `read_input_report_raw`) were all removed on 2026-10-17; Apex 7 wired works through hidapi |
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
