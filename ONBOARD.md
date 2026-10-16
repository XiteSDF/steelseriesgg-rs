Above all, respect the original repository *.md files guidelines.

Task: implement onboard profile read packet for Apex 7 keyboard on the `main` branch.

Git status: `onboard-configs-dev` branch is checked out. `onboard-configs` and `qwen` branches should be ignored.

Useful data:
`docs/adding-hardware.md` - guidelines for adding hardware.
`ssgg-capture-1612-20261015-004400` - capture as instructed in guidelines.
`lisp` - lisp descriptors for various steelseries devices dumped from Windows engine (should not be referenced in code or comments).
`src/devices/keyboards/apex_pro_tkl_2023_profile.rs` - experimental implementation for Apex Pro TKL 2022 keyboard (year not a typo, that's how it's named in Windows engine files).

Additional environment:
Ghidra running MCP server with SteelSeries Engine Windows executable and device firmware loaded.

Moved from `PLAN.md`:

## Apex 7 on-board profile read (`onboard` feature, hardware validation)

Implemented 2026-10-15 from the 2026-10-15 GG capture: GG's six-frame opening chain (`90 00`,
`90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01` - the `90` poll answers the firmware version), `0x85`
pulls, `0x83` payload reads, checksum, payload decode, `ssgg onboard plan|read|schema|decode`. The 13-byte
"pacing frame" earlier versions sent is not in the capture and was removed. Every protocol byte in it is
`[EXPERIMENTAL]` - nothing has been replayed on hardware. Read-only by design: the push command is named
and never sent. Wire format, layout table and open questions: `docs/development/apex-7-onboard-profile-read.md`.

0. **[HW] Opening chain:** `ssgg onboard read --yes --profile 0 --chunks 1 --resource-len 512 --read-len 512`
   first reports what the chain answered (the `firmware:` line is the `0x90` poll's answer). If the keyboard
   answers `4.16.x`, the chain is accepted on our framing; `--no-init-chain` compares the same read without it.

Measured 2026-10-16 by comparing the GG capture with the ssgg run of 2026-10-15 on the same keyboard: the
chain, the chain answers, the pull header and the `83` header all match, and the frames we send are answered
exactly like GG's. What we never sent is the **read-back**: GG issues `GET_REPORT 0xa1 0x01 wValue 0x0300
wIndex 1 wLength 642` after every read request and takes the window from that control-IN data stage; our
run sent none, so it collected nothing but 64-byte interrupt status. That is step 1a.

1. **[HW] Path probe (Linux, Apex 7, GG stopped):** `ssgg --debug onboard read --yes --profile 0`. It
   answers "is the profile protocol on the collection the device manager opened for the keyboard?" - the
   read runs on the `Apex7` device handle, and `ssgg devices` shows which collection that is. The
   2026-10-15 probe answered on interface 4 (`/dev/hidraw3`, `0xFFC1`), which is where the device manager
   lands on that box. If a keyboard needs interface 1 (`wIndex 1` in the capture) and the manager opened a
   different collection, that collection has to be discovered as its own device: the read no longer probes
   nodes by path.
1a. **[HW] Feature-report read-back:** the read path must ask for the reply the way GG does — report
   number **0**, 642 bytes, on the interface-1 collection. `HIDIOCGFEATURE` builds `wValue` from the
   buffer's first byte, and since 2026-10-17 every report buffer built in `src/devices/hid_reports.rs`
   starts with report number **0** (`READ_REPLY_ID = 0x00`), which is GG's `wValue 0x0300` / `wValue 0x0200`;
   the 2026-10-15 run asked the command byte as the report number (`0x0290`, `0x0285`, `0x0383`) and got no
   read-back, and
   the kernel validates the request against that interface's report descriptor. **642**, never 643 (643 is
   the `ETIMEDOUT` of 2026-10-15). Confirm with `--debug` that a control-IN transfer with 642 bytes of data
   appears in the capture we take, and that the first bytes behind the report-ID byte are the schema word
   (`0b 00`).
1b. **[HW] hidapi as the transport (code 2026-10-17, hardware unverified):** the hand-rolled raw-ioctl and
   `write(2)` paths are deleted; every transfer is a hidapi call (`send_feature_report`, `get_feature_report`,
   `write`, `read_timeout`) on the handle the device manager opened for the keyboard. On Linux the `linux-native` backend issues
   the kernel's own `HIDIOCSFEATURE`/`HIDIOCGFEATURE`/`HIDIOCSOUTPUT` (`hidapi-2.6.7/src/linux_native/ioctl.rs:20,26,38`),
   the direction-bit bug was fixed in 2.6.6 and 2.6.7 is pinned, so the `#[cfg(unix)]` split is gone. Output frames
   therefore leave as `SET_REPORT wValue 0x0200` control transfers, matching the reference capture, instead of a
   `write(2)` to an OUT endpoint the profile collection does not have. **Done 2026-10-17**: the per-call
   path-open helpers (`*_report_raw`) are removed and the read runs on the device's own handle, which is open for the whole
   read; `usbhid` polls the interrupt endpoint only while a descriptor on the node is open, which is why per-report opens
   missed the chain and pull answers (measured in
   `docs/development/apex-7-onboard-profile-read.md`, "Input reports: the node must be open when the report arrives").
   The 2026-10-15 text `Inappropriate ioctls (errno 110)` is not evidence of what the kernel answered - that path
   formatted `last_os_error()` after unrelated calls on the same thread.
2. **[HW] Reply framing:** one short read per candidate framing -
   `ssgg --debug onboard read --yes --profile 0 --chunks 1 --resource-len 512 --read-len 512
   --payload-len 512 --reply-header N` for `N` in `0, 1, 2, 5, 6, 14, 19, 20, 21, 36`. The framing that
   yields schema `0x000B` with a readable name (`Config 1` .. `Config 5`) is where the payload really
   starts on this keyboard.
3. **[HW] Full read of one slot, then slot by slot:** `ssgg onboard read --yes --profile 0 --reply-header
   <the framing from step 2> --out /tmp/p0.bin --json /tmp/p0.json`, then `--profile 1..4` - every read
   prints that slot's schema word. Schema `0x000A` / `0x000B` means the decode applies; anything else is a
   third schema - record it, do not guess. A `checksum: stored ..., computed ..., match:
   true` would validate the CRC and the 12 x 1024 split at once; the capture does not predict it, because the
   payload-read reply is a file head that repeats the head (`0b 00` + the name) and never the requested
   window - 241 GG reads give 33 distinct payloads, 179 of them all zero. Record what the answers look like, with `--reply-header 0` (the default, the framing the
   capture shows) and with the framing found in step 2.
4. **[HW] Reproducibility:** `ssgg onboard read --yes --profile 0 --out /tmp/p0b.bin --verify`, then
   `--gap-ms 0`, `2`, `10` to find the lowest gap the keyboard tolerates; keep the measured value as the default.
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
| Wire types: `0x85` pull, `0x83` read, `0x03` init chain, the pacing frame `0x01 0x00 00 02 00 f4 01 60 00`, the 643-byte read buffer | `src/devices/hid_reports.rs` (`OnboardPullCommand`, `OnboardReadCommand`, `OnboardInitCommand`, `ONBOARD_INIT_CHAIN`) — not feature-gated, no raw byte arrays outside it |
| Read plan, reply parsing, assembly, decode | `src/onboard_profile.rs` (`ReadConfig`, `plan_json`, `parse_read_reply`, `ProfileImage`, `decode`), feature `onboard` |
| Packet I/O | `src/devices/keyboards/apex_legacy_onboard.rs`: `ProfileHandle` (`Arc<Mutex<HidDevice>>`) with module-internal transfers on it (`write_output`, `send_feature`, `read_feature`, `read_input`, `device_label`) - `Apex7` passes its own handle, no transport type. The trait `OnboardTransport`, its `HandleTransport` and `PathTransport` implementations, and the per-call path-open helpers (`send_feature_report_raw`, `read_feature_report_raw`, `write_output_report_raw`, `read_input_report_raw`) were removed on 2026-10-17; Apex 7 wired works through hidapi |
| Keyboard device | `src/devices/keyboards/apex7.rs`: `Apex7` wraps `GenericKeyboard` (its `HidReportBuilder` and the open `HidDevice`) and runs the read on its own handle; `onboard_family(pid)` selects `0x1612`, `0x1614`, `0x1616`, `0x1618` and `discovery::open_keyboard` wraps those PIDs in it **with or without the `onboard` feature** — only the read (`read_profile`, the transport, the read plan) is gated, so a family board keeps standard lighting in a default build |
| Device API | `Keyboard::supports_onboard_profile`, `Keyboard::read_onboard_profile` (feature `onboard`, `[EXPERIMENTAL]`); every other keyboard answers "not supported" |
| CLI | `ssgg onboard plan / read / decode` (`src/cli.rs`, `src/main.rs`); `read` runs on the `Apex7` device (its own handle, `--device vid:pid` selects the keyboard), and fails with a clear message when the target is not on the family |

Two facts from the first Linux run (2026-10-15, Apex 7 `/dev/hidraw3`) that shape the transport:

- Discovery opens the interface-4 collection (`usage 0xFFC1`) for this keyboard, not the interface-1
  collection the capture's transfers ran on: the 2026-10-15 probe tried `1-3:1.4/input5`
  (`is_collection = true`), and the interface-1 node had no `input*` at all that day.
- `usbhid` polls a collection's interrupt endpoint only while a descriptor on that node is open. A
  per-report open (`/dev/hidrawN` opened for one report) has nowhere to deliver the opening-chain and
  pull answers, so `read_interrupt_report` saw none. The read avoids that: `Apex7` does its own packet
  I/O on the handle the device manager opened, and that handle stays open for the whole read.
