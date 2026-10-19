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

## Apex 7 on-board profile write

Task: implement `ssgg onboard write` and `ssgg onboard load` commands.

1. `ssgg onboard write` mirrors the current state of `ssgg onboard read` command and writes the onboad config to the keyboard. Input is decoded json by default, profile slot is mandatory, --bin optionally allow to upload a binary dump instead.
1.1. The shape of write is inverted compared to read - do two Write commands then Push command in a loop. Write command and Push commad have a similar signature to Read command and Pull command. Their codes are `0x03` and `0x05`, respectively.
1.2. The engine writes more than it reads. This is observable in packet dumps and may be observable in list files. However, the specifics are unknown at the time, so in this version we write what we have.
1.3. Verify implementation against lisp files and Ghidra disassembly.
1.4. After writing, a LoadProfile command should be issued to the newly written profile.
1.5. Command should support a `--dry-run` option where it prints all commands that it issues to write the profile to keyboard without actually issuing these commands.
1.6. Acceptance is `ssgg onboard write --dry-run config_1.json --profile 1` issuing the same write sequence as the GG write capture until the `05 00 01 …` packet.

2. `ssgg onboard load` issues a LoadProfile command to load a profile slot specified by user. Same restrictions apply to profile slot as `ssgg onboard write`.

**Result (2026-10-20, software part done, hardware part open).** `ssgg onboard write` takes the decode report by
default, `--bin` takes the payload bytes as they are, `--profile N` is mandatory (no default: a write names the slot
it rewrites) and the frames are the capture's: `88 00 01 02` open, 40 windows of `03 00` buffer 0 +
`03 00` buffer 512 + `05 00 01 02 <file offset> 00 04` push, `eb 01` commit, then `89 01` deploy (GG sleeps 250 ms
behind that frame; the write path does not).
The geometry is the keyboard family's (`OnboardProfileCodec::write_plan`, `FILE_BLOCKS` of `apex_7.lisp:217` and the
engine write API `base_apex_legacy.lisp:1993-2022`), the pacing is the capture's (7.1 ms average per frame, the
100 ms the engine inserts every 32 frames, 5 ms of answer drain after each push; the engine's 250 ms sleep behind
the deploy frame is not slept), and a
write answers `88 01 …` after the open and `05 01 …` after every push (202/202 in the capture; buffer writes and the
commit are not answered). 1.6 is met: the plan of `config_1.json` byte-equals the capture's frames, its frame 103 is
the capture's frame 103 (`05 00 01 …`, file offset 33792), and a 12,288-byte image plans the
same 122 frames, padded past its end with zeros. `--dry-run` prints the plan and opens no keyboard; without it
`onboard write` and `onboard load` send - there is no `--yes` gate (dropped 2026-10-20: asking for the command is
the authorisation, `--dry-run` is the safe mode).
`onboard decode` of the capture's own image answers `Config 1`, schema `0x000B`, and its stored checksum equals the
one `encode` computes (`0EB1C3D1`). Fixture: `tests/fixtures/onboard-write-apex7.manifest.txt`
(generator `scripts/gg-write-frames.py`), asserted frame for frame by
`apex_7_profile::tests::the_write_plan_of_the_read_report_is_the_capture_write_sequence`; fixture bytes are the
wire data stage (no USBPcap header, no report-ID byte). **No test touches a keyboard and no write, open, buffer
write, push, commit or deploy frame has been replayed on a keyboard** - 1.2 ("write what we have": 11,696/12,288
payload bytes padded to the 40-window region, the region behind the payload never written), 1.4 (deploy after a write)
and 1.5 (`--dry-run`) are implemented, the write→read→decode→diff cycle and every hardware claim stay open with the
user's keyboard, and the first hardware write is a deliberate, user-approved act reported with the model and date.
Write-up: `docs/development/apex-7-onboard-profile-write.md`.

## Apex 7 on-board profile read (`ssgg onboard`, hardware validation)

Implemented 2026-10-15 from the GG capture: a `0x90` firmware-version poll, `0x85` pulls,
`0x83` payload reads, checksum, payload decode, `ssgg onboard read|decode`. Only the profile slot is
selectable (`--profile 1..5`, GG's `Config 1` .. `Config 5`: `--profile N` reads slot `N - 1`, the file
GG pulls as `0x020N`); the geometry is compile-time constants, and since 2026-10-19 the read **pulls 40
fills per file** (file offsets `0..=0x9C00`, a 40,960-byte image) where GG's own read walks 12: GG's
**write** transfers reach `0x9C00`, so the read collects the region behind the 11,696-byte payload,
records it (`extra_len`, `extra_fnv1a_64`, `--bin` writes the payload and `<out>.extra.bin` the region)
and decodes none of it (§Known facts (2026-10-19, the write side: `0x05` is the pull frame flipped)). The 13-byte "pacing frame" earlier
versions sent is not in the capture and was removed. GG's six-frame opening chain (`90 00`,
`90 01`, `f4 00`, `6c 00 01`, `90 00`, `90 01`) was removed on 2026-10-17, restored on 2026-10-18 when a read
with only the `90 00` poll answered with scrambled payload bytes, and **removed again the same day: measured
on hardware, the read collects a profile slot with no chain and no closing sequence, so ssgg sends neither**
(§Known facts (2026-10-18, sequences the read does not need)). `f4 00` and `6c 00 01` have no known meaning.
Every protocol byte in it is `[EXPERIMENTAL]`. **The read changes nothing on the keyboard**: it sends the
firmware poll, the probe, the pulls and the payload reads. Wire format, layout table and open questions:
`docs/development/apex-7-onboard-profile-read.md`.

**Read status (2026-10-18):** `ssgg onboard read --profile N` completes a full slot
read on an Apex 7 under Linux, and the slot-0 read image (`p0.bin`, wired, firmware 4.16.3) **decodes with a
checksum match** (`AE57B9DC` over 11692 bytes) once the layout was corrected to GG's payload struct: 5-byte
mapping entries, OLED at 1385, Rapid Tap at 2025/2026 with 5-byte pairs, macro at 2076, GUID at 11676, the
checksum word at the payload tail (11692) and not at the tail of the 12288-byte image. The read decodes as
112 self-bound keys, 12 empty slots and 4 shipped media-cluster bindings on the base layer. That closes the transport questions (steps 1, 1a, 1b) and the framing question
(step 2). It is reported without the run log, so record the model, firmware, slot contents and the checksum
line from the run here before anything is promoted to confirmed - the label stays until then. The 2026-10-18
run had GG's chain and closing sequence in it; the runs have neither, and the chain question
(step 0b) is closed. Steps 3-5 (slot by slot, the double read, the layout compare with GG) are what is left.

0. **[HW] Opening chain and firmware poll - superseded 2026-10-18, the chain is not sent any more:** the
build described below starts with GG's six-frame
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

0b. **[HW] Which chain frame arms the resource buffer - closed 2026-10-18: none of them does** (the read
works with the chain not sent, §Known facts (2026-10-18, sequences the read does not need), and the code no
longer sends it, so this probe is moot). History: drop the chain frames one at a time (a code change
    in `ONBOARD_INIT_CHAIN`) on a run that reads a slot whose contents you know from GG, and record here
    which frame the keyboard needs. Today all six frames are sent, and the answer is what lets the read drop
    the ones that do nothing. **Drain one input report after every frame while testing:** a frame set tested
    without draining looks dead when the only fault is the unread answer (step 0, 2026-10-18). Which frame
    arms the buffer is still unmeasured.

0c. **[HW] Schema gate (done 2026-10-18):** the read runs GG's 2-byte file-head probe as its first step
    (`Apex7::probe_schema`: 2-byte pull at file offset 0, then one window read at buffer offset 0, exactly
    the pair the capture's transfer map lists) and **refuses the read when the schema word at payload offset 0
    is not `0x000B`**: schema B is what `apex_7_profile` decodes and what has been measured, so a device
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
3. **[HW] Slot by slot, then the checksum line:** the read of one slot works (2026-10-17/18) and its checksum
matches (2026-10-18, slot 0). Remaining:
`--profile 2..5` - every read
   prints that slot's schema word. Schema `0x000A` / `0x000B` means the decode applies; schema `0x0000` is a
   slot GG never saved and `decode` says so. A `checksum: ... match: true` validates the CRC and the
   12 x 1024 split at once (slot 0: true).
4. **[HW] Reproducibility:** `ssgg onboard read --profile 1 --bin /tmp/p1.bin --verify`. Pacing is
   `GG_GAP_MS`, a constant: a gap ladder needs a code change, and whatever the keyboard tolerates is recorded
   there.
5. **[HW] Layout check:** the offsets are settled (GG's payload struct, hardware-verified by the CRC, 2026-10-18,
   §Payload layout in `docs/development/apex-7-onboard-profile-read.md`). Settled since 2026-10-19: the **macro
   block** - the 97 non-zero bytes of the reference read form a 136-byte packed record stream (2076..2211) that
   decodes to 2 records, and record 0 is the macro **GG
   shows** on that keyboard (META+Insert, "Repeat every 5000 ms", pressing the `4` key), so `mode`,
   `interval_ms`, `flags`, `record_bytes` and the action words are read against GG's screen; and the OLED block's
   orientation (128 x 40, 1 bpp, **bit 7 = the left pixel**) was confirmed on a 0x1628 screen on 2026-10-18, so
   the decoder and the GUI write and draw it that way with no switch. Remaining: compare name, lighting colours
   and key mappings with what **GG shows** for the same slot, and the `flags` word (`0x0101` on both records,
   named "repeats" by the engine) plus the mode values other than `1` (play once) and `4` (repeat every
   interval). A record is an 8-byte header (marker `0x77`, mode, id, flags, interval u16 ms) plus its actions; the
   list has no stride and no length word - the payload region behind it is zero - so the decoder walks the actions
   to the zero pad. The record decode is `[EXPERIMENTAL]`:
   §The macro block is a packed record list in `docs/development/apex-7-onboard-profile-read.md`.
   **The binding index is measured 2026-10-19** on a user's own Config 1 read (`some_rebounds.json`, 5 records, 14
   bindings, no device involved): the third code byte of a `0x71` slot is **7 x the record's position**, and the
   record a key binds is `floor(index / 7)`, so the 7 indices of a group bind the same record - C, V, X carry 0, 7,
   14 and bind the records that tap C, V, X with LCtrl held 20 ms; indices whose ordinal is past the list (38, 43,
   46, 63, 92, 96, 117 in that read) are the macros GG deleted with the binding left on the key. The factor 7 is
   measured from that one profile's indices and content; *why* 7 is open.
6. **[HW] Windows:** same read through the hidapi path (no udev rule needed); confirms the interface choice for
   the control collection.
7. Only after 1-4: extend to the lighting files (`0x0301`-`0x0305`, reply report ID `0x03`), then promote what
   worked to confirmed in `.claude/rules/experimental-protocol.md` with device model and date.
8. **[HW] Closing sequence - superseded 2026-10-18: the read sends none of it** (measured: the read works
   without it; `Apex7::send_end_sequence` and `onboard_end_sequence` are out of the code). History, including
   why the fourth frame was chosen:

8a. **[HW] Closing sequence (code and driver-mode close 2026-10-18, never sent on hardware):** the read ends with four frames
   (`onboard_end_sequence` in `hid_reports.rs`, sent by `Apex7::send_end_sequence`): `69` (Gamesense OLED off),
   `89 <slot + 1>` (deploy that profile - the engine's deploy id is 1-based, so slot 3 goes out as `89 04`),
   `41 00` (profile not volatile, sent as 0 for every real slot) and `6c 00 00` (**close driver mode**). The
   first three are in the capture's frame inventory (one deploy/volatile pair behind each profile file, two `69`);
   the engine sleeps 250 ms behind the deploy frame and the rest goes at the chain's 11 ms pacing. The fourth
   frame is **in no capture**: the engine writes it when it closes a session and GG was killed in the capture, so
   its closing frames never hit the wire. It is sent because the read's own opening chain opened driver mode with
   `6c 00 01` - the read closes what it opened. The engine's own session close also writes a lighting-reset frame
   (`0x3B`); the read deliberately does not. The capture's frame inventory has one `ff` frame, recorded by the kit
   under the "idle, do nothing" step (capture summary): driver mode in this family is `0x6C` (on Apex 3 it is
   `0x03`), so `ff` is not driver mode, no engine-side name covers it, its direction is not settled, and the read
   does not send it. **These frames write:** `89` makes the keyboard run the slot. Test on hardware, watching the
   keyboard - does the active profile change to the slot that was read, does the OLED go dark, does the keyboard still type
   normally after the read (driver mode is what puts some key combos into the keyboard report), and does the read
   complete with the frames sent (they are the last
   transfers, so a keyboard that refuses one fails a read whose image is already assembled).
   Engine-side detail, including the fact that **the driver-mode frame is a different frame in the Apex 3 family**
   (`0x03` in a 33-byte report on Apex 3, `0x6C` with a different opening byte on Apex 3 TKL), is in
   `docs/development/apex-7-onboard-profile-read.md` §Driver mode. The read path exists for the legacy Apex family
   only, and a port to the Apex 3 family cannot reuse these frames.
11. `onboard encode` writes a decode report back into payload bytes, no device involved:
    `ssgg onboard encode config_3.json -o config_3.enc.bin --check config_3.bin`. The output is the payload
    (length from the geometry table, checksum computed there - the report's stored word is never copied), and
    `--check` compares the payload with a file: the 2026-10-18 images are 12,288-byte read windows (the
    12-pull read), so it says what the 592 bytes behind the payload are (device buffer content) and compares
    only the payload; since 2026-10-19 a read's `--bin` is exactly the payload and the pulled region behind
    it goes to `<out>.extra.bin`, so a check against a current read image is payload against payload. Verified
    byte-identical over the 11,696-byte payload of `config_1.bin`, `config_1_2.bin` and `config_3.bin`,
    `config_1.json` included with its 2 macro records written back unchanged. A JSON it cannot write faithfully
    is refused: a macro block carrying a decode note, a mapping table whose offsets are not in payload order,
    Rapid Tap on schema `a` or missing on `b`, a block described where the schema does not put it, a hex field
    of the wrong length. **Nothing sends these bytes - the write path is not implemented** - and every offset,
    stride and field width encode writes carries the `[EXPERIMENTAL]` status of the decode that produced it
    (`docs/development/apex-7-onboard-profile-read.md` §Encode).

---

## Code layout (2026-10-17)

| Piece | Where |
|-------|-------|
| Wire types: `0x85` pull, `0x83` read, `0x90` firmware poll (source byte: `90 00` keyboard firmware, `90 01` LED firmware), the 642-byte read buffer | `src/devices/hid_reports.rs` (`OnboardPayloadPullCommand`, `OnboardBufferReadCommand`, `OnboardFirmwareVersionCommand` with its `FirmwareSource` byte, `OnboardDriverModeCommand`, `OnboardProfileDeployCommand`) — not feature-gated, no raw byte arrays outside it. The chain code (`OnboardInitCommand` + `ONBOARD_INIT_CHAIN`) and the closing sequence (`onboard_end_sequence`) are out of the code (2026-10-18, §Known facts (2026-10-18, sequences the read does not need)), and so are the command codes nothing sends (`EngineStatus 0xF4`, `DisableGamesenseOled 0x69`, `SetProfileVolatile 0x41`). `CommandCode::OnboardDriverMode` (`0x6C`) and `CommandCode::OnboardProfileDeploy` (`0x89`) stay, with their named builders, and **nothing in ssgg sends them**. Driver mode is a different frame in the Apex 3 family - `docs/development/apex-7-onboard-profile-read.md` §Driver mode |
| Read geometry, reply parsing, assembly, decode, encode | `src/devices/keyboards/apex_7_profile.rs` (`PULLS_PER_FILE` = 40 since 2026-10-19 - GG reads 12 fills of a profile file and **writes** to `0x9C00`, so the read pulls `PROFILE_FILE_BYTES` = 40,960 and keeps the region behind the payload: `payload_len`, `extra_len`, `extra_fnv1a_64` in the decode report, `region:` in the read record, `onboard_extra` in `src/main.rs` writes the side file, `CMD_PUSH_PAYLOAD` (`0x05`) names the write-direction twin of the pull (frame layout measured 2026-10-19, sent by nothing), `DEVICE_RESOURCE_BUFFER_LEN`, `PAYLOAD_READ_LEN`, `READS_PER_PULL`, `profile_file_id`, `validate_profile`, `parse_read_reply`, `ProfileImage`, `decode`, `encode_json` (2026-10-19, writes a decode report back to payload bytes; `payload_diff` for its `--check`), core code. It was `src/onboard_profile.rs` until 2026-10-17, when it moved into the legacy Apex namespace: its constants are all `apexui.dll` facts of the legacy ("macho") platform - reply framing, resource-buffer size, payload sizes, schema words, struct offsets - and there is nothing there another family can share. The only knob is the profile slot: pull count, buffer size, window size, pacing and reply framing are constants, the read loop builds every frame from the typed commands, and it is also what logs the geometry - the CLI prints none, so no legacy constant appears in `src/main.rs`. It also carries `Apex7Onboard` (2026-10-19): this family's implementation of `onboard::OnboardProfileCodec`, delegating the codec methods to these functions and owning the family's words (`banner_lines`, `image_lines`, `profile_lines`, `check_lines` - the read's print text moved out of `src/main.rs` the same day) and its numbering (`config_count`, `slot_for_config`). Renamed 2026-10-18 so the names say **fact**, not **default**: `DEFAULT_CHUNKS` → `PULLS_PER_FILE`, `READ_LEN` → `PAYLOAD_READ_LEN`, `RESOURCE_BUFFER_LEN` → `DEVICE_RESOURCE_BUFFER_LEN`, `PROFILE_FILE_LEN` → `PROFILE_FILE_BYTES`, `SCHEMA_B_FIRMWARE` → `SCHEMA_B_FIRMWARE_VERSION` (documentation only - the read takes its schema from the word it reads). Deleted as unused: `PROFILE_FS_LEN` (the 0xA000 flash region), `PULL_REPORT_LEN`, `INIT_REPORT_LEN`, `LIGHTING_FS_SHARED` |
| Packet I/O | `src/devices/keyboards/mod.rs`: `GenericKeyboard` does the profile packet I/O on the `device` handle it already owns - `connected_device` (the one handle accessor, shared with the lighting sends, same "Device not connected" message), no cache, so repeated frames go out; `send_report` runs the hid optimizer, which drops a report identical to one sent in the last 50 ms - see the write-path note in step 0), `send_feature`, `read_profile_feature_packet`, `read_profile_input_packet`, core code; `Apex7` runs the read plan - `read_profile`, `firmware_version`, `probe_schema`, `pull_window`, `read_window` - on its own handle, blocking the calling thread for its ~2 s (2026-10-19: the `std::thread::scope` wrapper added 2026-10-18 - which had replaced `spawn_blocking` fed by an `unsafe` transmute to `&'static` - was removed: it joined before the call returned, so the work blocked a thread either way, and the method is a plain sync body now). The plan takes the slot only: `strict` (no flag fed it) and the `deadline` reply-budget plumbing (always `None`) are gone, one payload read has one fixed `REPLY_BUDGET_MS`. No transport type: `OnboardTransport` with its `HandleTransport` and `PathTransport`, and the per-call path-open helpers (`send_feature_report_raw`, `read_feature_report_raw`, `write_output_report_raw`, `read_input_report_raw`) were all removed on 2026-10-17; Apex 7 wired works through hidapi |
| Keyboard device | `src/devices/keyboards/apex_7.rs`: `Apex7` wraps `GenericKeyboard` (its `HidReportBuilder` and the open `HidDevice`) and runs the read on its own handle; **discovery never asks the on-board question** (2026-10-19): `discovery::open_keyboard` builds a keyboard from its HID collection and product ID alone - the catch-all arm that wrapped on-board-capable PIDs in `Apex7` is gone - so every keyboard the manager enumerates is a keyboard with its lighting path, whatever it can read off the board. The on-board scope is stated once, in the device class: `Apex7` answers the Apex 7 codec, and discovery builds `Apex7` for the captured collection alone - `usage_page 0xFFC0, usage 0x9F` with product ID **`0x1612`**, the board the frames were captured on and measured on (narrowed 2026-10-19 from the four PIDs GG lists on this platform - `0x1614`, `0x1616`, `0x1618` - because none of them has hardware here and none has ever answered a frame of ours: an untested board is refused, not asked to run a frame stream nobody has seen answer). `Keyboard::supports_onboard_profile` is `onboard_codec().is_some()` and that is the only gate the on-board commands use - `onboard read --device 1616` says so: `no keyboard with product ID 1616 answers the on-board profile protocol` |
| On-board profile interface | `src/devices/keyboards/onboard.rs` (2026-10-19, core code): the per-keyboard interfaces. `Keyboard::onboard_codec` answers *which family protocol a keyboard speaks* (a `&'static dyn OnboardProfileCodec`; `None` = no protocol, and `Keyboard::supports_onboard_profile` is derived from it), `Keyboard::read_onboard_profile` pulls one slot on the keyboard's handle, and `OnboardProfileCodec` is that family's **device-free codec**: `decode`, `encode_json`, `profile_json`/`image_json`, `dump_name`, `config_count`/`slot_for_config`, `knows_schema`, `mapping_len`, `summary_line`, `hex`, `diff`, and the text a read prints (`banner_lines`, `image_lines`, `profile_lines`, `check_lines`). `codecs`/`codec_for_schema`/`codec_for_bytes`/`codec_for_json` is the registry a file-only command reads a file with, and `schema_of`, `hex_of`, `hex_dump`, `image_diff`, `payload_diff` - payload helpers no family owns - live here. The report types (`ProfileImage`, `OnboardProfile`, ...) are re-exported from the legacy family module: **they are that layout as data**. The next keyboard plugs in by implementing the codec for its own layout, extending those types where its profile has fields this one has, registering its codec, and returning it from `Keyboard::onboard_codec` - on-board support is per keyboard, and `src/main.rs` names no family module |
| Device API | `Keyboard::onboard_codec`, `Keyboard::supports_onboard_profile`, `Keyboard::read_onboard_profile` (`[EXPERIMENTAL]`, core code, all declared in `keyboards/mod.rs` with "no protocol" defaults); `DeviceManager::open_onboard_keyboard` selects a keyboard that answers - `--device <pid>` (product ID: every SteelSeries device shares the vendor ID `0x1038`, so the product ID identifies the device), or, with no flag, the only connected keyboard that answers (rule: `discovery::onboard_choice`, answer: `discovery::OnboardKeyboard::Keyboard` / `NotFound` / `Ambiguous`); every other keyboard answers "not supported" |
| CLI | `ssgg onboard read / decode` (`src/cli.rs`, `src/main.rs`); `read --profile` takes GG's config number (`Config 1` is `1`): clap only parses it as a number, and the **keyboard's family** answers how many configs exist and which slot is behind the number (`OnboardProfileCodec::config_count` / `slot_for_config`), so an out-of-range number is refused in the family's wording - the CLI has no `1..=5` bound since 2026-10-19 (it was a legacy-family fact sitting in clap). The slot is 0-based in the library API and the read's `--debug` geometry line prints both. The keyboard is `--device <pid>` (product ID, hex: `1612`, `0x1612`) - the CLI has no default product ID, names no vendor (all SteelSeries devices are `0x1038`) and no keyboard family; omitting the flag is accepted only when exactly one connected keyboard answers the protocol (rule: `discovery::onboard_choice`: several collections of one product ID count as one keyboard, distinct product IDs and no flag answer `Ambiguous` and the CLI asks for `--device`) and it sends no transfer to a candidate it opens to ask; a keyboard the caller did not name that cannot be opened is skipped with a log line, a keyboard named with `--device` answers its open failure as an error and takes `-o/--out` (JSON path, default: the profile name it reads - `Config 1` -> `config_1.json`; the file is the **decoded profile**, the same report `decode` writes - 2026-10-18, it used to be the read record, which `tools/profile-gui` could not open), `--bin` (the decoded **payload** bytes, only when a path is given; the region the read pulled behind it is written beside it as `<out>.extra.bin` when there is one, 2026-10-19), `--report` (the per-pull read record as JSON: chunk headers, framing stats, image hash - capture diagnostics), `--hex`, `--verify`; there is no `--yes` (2026-10-18: the read sends request frames only, so no confirmation gate) and no `--json` (the JSON dump is the default output). Every other number of the read is a protocol constant, not a flag, and the CLI prints none of them; the geometry - slot, file id, pulls, payload reads - is logged by the read loop at `info` level, so `ssgg --debug onboard read` shows it before the first transfer; `decode` works on a saved image and touches no device: which family a file belongs to is answered by the codec registry from the schema word inside it, and the `Config 1..5` numbering, the dump file name and the print text all come from the codec, not from the CLI |
| Capture kit | `scripts/capture-onboard-kit/`: the capture script (explicit device list) and the analysis script that rebuilds a slot from a control transfer and reports its checksum line; see its `README.md` |
| Profile JSON editor | `tools/profile-gui/`: browser page, no build step, no deps - opens the JSON dumps `read` and `decode` write by default (the decoded profile), edits them on a drawn ANSI board (keys bound through GG's `device-offset-lookup-table`, embedded as `HID2OFF`/`OFF2HID`), classifies every mapping as `self` / `none` / `custom` so a factory profile reads as having no remaps, and saves JSON with unknown fields preserved. **One profile open at a time** (a new load replaces it); `Save a copy` takes no name (the browser download already asks where to put the file): the copy name is derived - loaded name plus a `-copy` marker, marker not stacked, profile name as the fallback - so a save never offers the loaded file's own name. The board scales to its column (never scrolls), carries no offset numbers (they are in the cap tooltip and the key panel), centres legends with a shrink class, draws the arrows as glyphs, labels the numpad without the `Ke ` prefix, prints a sub line only for a remapped key (a factory board prints nothing), draws the two base `wheel_mappings` entries as half caps in the numpad `-` column, top on the function-row line, with a direction glyph over the mapping text inside the cap, and the on-board OLED as a 16:5 rect over the numpad's first three columns **rasterised from `oled.bytes_hex`** (128x40 1bpp, bit 7 = left pixel, confirmed on hardware, so no bit-order switch); the cap palette is hue-and-lightness separated by checks (neutral states clearly above the cap, macro green vs media magenta apart, legend swatches equal to `FILL_COLORS`); **the cap is the control and the cap is the hit area**: both cap rects carry `pointer-events: all` and every `text` in a cap carries `pointer-events: none`, every line the cap draws is clipped to the cap box by a per-key `clipPath` (`cap-clip-<hid>`), a sub line longer than its cap is cut with an ellipsis at the width its shrink class draws (`fitSubText`: the Rapid Tap marker is never cut, and when it fills the cap the marker is painted alone), and focus is drawn on the cap rect (`.key:focus { outline: none }` plus a cap stroke), because a focused control's default ring follows the group's bounding box, which the text lines widen - so a click on a long macro sub line, or the ring around it, is not a thing. Every cap is `role=button` `tabindex=0` and Enter/Space select it. `selftest.mjs` 179 checks (the pair: a decode report that carries a pulled region shows its `payload + extra` split and region hash in the file panel, a payload-only report shows no such row), consumer codes read as media keys under function `0x61` (`0xE2` Mute, `0xB5` Next, `0xB6` Previous, `0xCD` Play/Pause, `0xE9` Vol+, `0xEA` Vol-; the same numbers are keyboard usages under other functions, so a code is named by its function byte); prints the decoded **macro record list** in a `Macro records` panel (record keys, mode, interval, byte span, the index group that binds it, and which keys bind it), resolves a `0x71` binding by `floor(index / MACRO_STRIDE)` (`MACRO_STRIDE` = 7, mirroring the Rust constant - a binding index 5 lands on the same record as index 0, and the byte the page writes for record *n* is `7n`, not `n`; a dump from before the measurement carries its record `index` as the position, and the page writes the stride-scaled value in that case), names that record in the key panel and the bindings table, and marks the `0x71` bindings in the inspector; `selftest.mjs` (179 checks) and `preview.mjs` run it under node - the preview shims a canvas with a BMP encoder, so `board-preview.svg` carries the bitmap. **Fixtures:** `examples/sample-identity.json` is a decode of the **stock** slot (`config_3.bin`, macro block empty) with the profile name blanked, `examples/sample-remap.json` is that decode with edits, and the macro checks run on a synthetic record stream shared with the Rust test fixture (`SYNTH_MACRO_BLOCK`) - no bytes of a profile that carries macros are kept in the code, and the measured record content lives in `docs/development/apex-7-onboard-profile-read.md`. The hash the page recomputes uses the engine's short-prime FNV variant over the block bytes, so it matches what the decoder writes. JSON only: no device access, no checksum recompute - encode lives in the CLI (`ssgg onboard encode`, 2026-10-19); see its `README.md`. On-board support is per keyboard family: a family implements `onboard::OnboardProfileCodec` for its own layout, and `apex_7_profile::Apex7Onboard` (2026-10-19) is the legacy family's. **Model names and the per-model feature set are imported from Rust** (2026-10-19): `tools/profile-gui/sync-products.mjs` parses `pub mod product_ids`, `device_name_from_product_id` and `device_type_from_product_id` in `src/devices/mod.rs`, `onboard_capable` in `apex_7.rs` and the `codecs()` registry in `onboard.rs`, and writes `tools/profile-gui/products.generated.js`, which `index.html` loads **before** `app.js` (the page reads it as `globalThis.SsggProducts`): the file panel shows the registry's name for the recorded product ID - which **wins over the `device.product` string the file carries**, per the crate's own rule of trusting the number over the name, with the claimed name used only when the registry is silent - the device kind decides the mouse / headset lines, and a model outside `onboard_capable` is labelled `payload layout: not the one this page encodes` because it needs that family's own layout, encoder and feature set (the product ID is in the JSON for exactly this reason: encodings and features are per model). Regenerate with `node tools/profile-gui/sync-products.mjs` after touching those tables; `--check` fails on drift, `selftest.mjs` runs it as a check and also compares the page's hand-mirrored fallback table (`PRODUCT_NAMES`, `LEGACY_ONBOARD_PIDS`) line by line against the import so they cannot drift (179 checks, 2026-10-19). **round 2 (binding, name, wheel, macro
select):** a key binds through **one select** - `Deactivated` / `Mouse Key` (GG's mouse functions
`0x01`-`0x08` and `0x30`-`0x34`: buttons, CPI, wheel, pan) / `Keyboard Key` (four code bytes, as
`Key 1`..`Key 4`) / `Media Key` (the six consumer usages the page names) / `Meta` (the five meta
actions) / **`Meta Toggle`** / `Macro` (a record select) - and a function byte no group owns (a
capture byte, `0x77`) keeps the raw function and code fields visible with a
`0xNN <name> - this editor has no group for it` option, so it is still editable by byte; the byte list and the code selects write the same
five bytes and a binding type with no option list writes `00 00 00 00` there; the wheel caps bind
through **that same panel** (click a wheel cap: target `wheel_mappings[n]`, base row on the base
layer, META row on the META layer) and the `Wheel mappings` tab, its table and its function select
are gone; the file-info panel edits nothing (no dropdown, no text field). **The META toggle is two
fields**: `meta_toggle_hid` - the binding, one byte naming the key GG switches META mode with (`0xF0`
is GG's code for the **Meta cap**, any other key is named by its own usage, `0x00` is no key) - and the
entry GG marks with `0x62` and code 1 = `05`: a stock read marks LWin's META slot (slot 13) and carries
the byte `0xF0`, the user profile marks slots 13 and 35 and carries `0x10` (the M key). The Meta action
select of a key bound `0x62`, and of any base-layer key (the page fills the key with GG's factory `0x62`
binding), offers `Meta Toggle (0x05) - make this key the META toggle`: it writes `62 05 00 00 00` on the
base layer and moves the byte to that key's code, keeping the key's other codes. The option is **not
offered on the META layer** (a stock META layer has no `0x62` entry bound on the base layer; GG's marker
slots are shown as the file has them and are not written) **nor on a wheel entry** (GG switches a layer
with a key). Undo and factory reset restore the binding bytes and never move the profile byte, which is
listed in the file-info diff on its own. Both paths print a `[EXPERIMENTAL]` line in the status, because
only the factory shape of the toggle is measured. The **Meta cap** (`0x65`) has no slot under its own
usage (`hid_to_offset(0x65)` is `None`) and binds through slot 93, the slot whose table code is GG's
`0xF0` (`Fn F0`), so it rebinds like any other key - a stock read binds slot 93 to itself, the user
profile binds it to `0x65`, the Context Menu keystroke - and the board paints that cap as `meta` with
`META toggle` in it while the byte marks it. **The profile name is edited above the keyboard** (field, indicator,
`Revert the name`): `as the file - <name_hex>` while it matches, amber `changed - the file as loaded
had "<name>" (<hex>)` when it does not, revert disabled until then and restores the loaded name
(regenerating `name_hex` with it), typing repaints only the indicator so the caret stays in the
field and the full repaint lands on the field's `change`, and a name over the field's 24 printable
ASCII bytes is refused in those words and leaves the document alone. The bottom row is drawn in the
order the caps sit (LCtrl, LWin, LAlt, Space, RCtrl, RWin, Meta, RAlt) - drawing order only, the
bytes are untouched. A macro binding select lists **short** options (`Record 0 - repeat 5000 ms - 2
action(s) - index 0`, 49 characters, no keystrokes; an index the records do not cover reads
`Index 14 - record 2, past the 2 record(s) (a macro GG deleted, binding left on the key)`) and the **whole macro** is shown in a block under it:
mode, id, interval, action count, every keystroke on its own line, the raw bytes and the `Bound by`
line; the cap sub line reads `macro 0 (record 0) - 4` |

Two facts from the first Linux run (2026-10-15, Apex 7 `the hidraw node`) that shape the transport:

- Discovery opens the interface-4 collection (`usage 0xFFC1`) for this keyboard, not the interface-1
  collection the capture's transfers ran on: the 2026-10-15 probe tried `1-3:1.4/input5`
  (`is_collection = true`), and the interface-1 node had no `input*` at all that day.
- `usbhid` polls a collection's interrupt endpoint only while a descriptor on that node is open. A
  per-report open (`/dev/hidrawN` opened for one report) has nowhere to deliver the firmware-poll and
  pull answers, so `read_interrupt_report` saw none. The read avoids that: `Apex7` does its own packet
  I/O on the handle the device manager opened, and that handle stays open for the whole read.

## Known facts (2026-10-18, sequences the read does not need)

Measured on hardware 2026-10-18 (`ssgg onboard read` on an Apex 7, operator runs):

- **GG's opening chain is not needed.** A full profile slot is read with none of `90 00`, `90 01`, `f4 00`,
  `6c 00 01` sent as a chain. The 2026-10-18 "only the `90 00` poll → scrambled payload replies" result is
  what an **undrained answer** produced, not the missing frames - that reading is inference, not a measured
  fact.
- **GG's closing sequence is not needed either.** The read is complete without `69`, `89 <slot + 1>`,
  `41 00`, `6c 00 00`. The read therefore **changes nothing on the keyboard**, and step 8's keyboard-effect
  questions stay unanswered (the frames are not sent).
- **The firmware poll takes a source byte.** `90 00` answers the firmware running the keyboard, `90 01` the
  LED controller's firmware - both read from an Apex 7 on 2026-10-18. The capture has both forms
  on the wire (capture summary rows 97, 115).
- **A payload read needs ~1 ms after its request is written** before reading the reply back; without the wait
  the read races the device filling its resource buffer and answers with garbage (`Apex7::read_window`).
- Code after this: the read sends one `90 00`, the 2-byte schema probe, then `PULLS_PER_FILE` = **40** pulls
  of `DEVICE_RESOURCE_BUFFER_LEN` = 1024 bytes (file offsets `0..=0x9C00`), each fill drained by
  `READS_PER_PULL` = 2 payload reads of `PAYLOAD_READ_LEN` = 512 bytes. The 28 pulls past GG's 12 are
  2026-10-19 (GG's write reach, §Known facts (2026-10-19, the write side: `0x05` is the pull frame flipped))
  and are not tested on hardware. `OnboardDriverModeCommand` and `OnboardProfileDeployCommand` stay in
  `hid_reports` as named commands for the deploy/driver-mode work, sent by nothing. `CMD_PUSH_PAYLOAD`
  (`0x05`) is named there as the write-direction twin of the pull and is sent by nothing.

**Not recorded in this repo:** the model, firmware and run log of these runs. The chain and sequence facts are
reported by the operator and carry the date.

## Known facts (2026-10-19, the write side: `0x05` is the pull frame flipped)

Reported from the operator's Windows write capture and re-decoded the same day from the
capture in this repo (engine capture 2, control-endpoint SETUP
records and their data fragments). Everything here is `[EXPERIMENTAL]`: no push byte has been sent to a
keyboard.

- **`0x05` is `0x85` with bit 7 of the command byte cleared**, same 64-byte output report
  (`SET_REPORT wValue 0x0200 wIndex 1 wLength 64`): `05 00 <fs_id u16 LE> <file offset u32 LE>
  <size u16 LE>`. GG's writes run as `0x05` pushes **until the offset reaches `0x9C00`** (250 pushes in the
  capture); its reads walk 12 chunks to `0x2C00`.
- In the capture all **72 `0x05` frames are one shape**: `fs_id 0x0102`, offsets
  `0x0, 0x400, …, 0x2C00`, size 1024 - 12 chunks per run, 6 runs. This document called them "72 lighting
  frames" for four days; they are push frames declaring a file transfer, and the bytes travel in the
  `03 00 <buffer offset u16> <size u16> <payload>` 642-byte feature writes (the 144 payload writes).
- A push is paced like a pull: the packet that follows it on the profile collection's endpoint is the same
  all-zero report the read path drains (`01 00 …`), and the answer (`05 01 …`) comes on that endpoint as
  well. Keystrokes are on a different endpoint, so the write's answer drain never sees typing traffic.
  The old note in §What answers a pull (`apex-7-onboard-profile-read.md`) about "the 72 `0x05` pushes
  behave the same way" stays true - it is the same pacing token.
- **`fs_id 0x0102` is a file the read never pulls** (profile files `0x0201..0x0205`, lighting files
  `0x0301..0x0305`), so what GG writes with `0x05` is open, and so is what answers a push.
- `ssgg onboard read` now pulls that region: `PULLS_PER_FILE` = 40 fills = 40,960 bytes per file
  (`0..=0x9C00`), keeping the 29,264 bytes behind the 11,696-byte payload as `extra_len` +
  `extra_fnv1a_64` in the JSON, `region:` in the record, and `<out>.extra.bin` beside `--bin`. **Nothing
  decodes it**, and the 28 extra pulls per file have not been run on hardware. The reason to pull it is the
  operator's hypothesis that the board holds more macro storage than the payload the decoder walks; the
  reference reads show device-buffer content there (zeros, 80 bytes of `FF` behind slot 3), which is
  evidence, not an answer.
- Code: `apex_7_profile::CMD_PUSH_PAYLOAD` (`0x05`) is named and sent by nothing, same rule as the
  driver-mode and deploy frames (`ONBOARD.md` §Known facts (2026-10-18, sequences the read does not need)).
  `ssgg` writes **no** profile payload: no read-back is known for this family.

## Known facts (2026-10-18, media code names)

- **Under function `0x61` (`media_key`) the code bytes are consumer-page usages**, not keyboard usages:
  `0xE2` Mute, `0xB5` Scan Next Track, `0xB6` Scan Previous Track, `0xCD` Play/Pause,
  `0xE9` Volume Increment, `0xEA` Volume Decrement. Same numbers under a keyboard function
  (`0x51` combo) are **keyboard-page** usages - `0xE2` is LAlt - so a code byte is named by its
  entry's function byte, never by its number alone.
- Names: supplied by the operator 2026-10-18 for GG's media cluster, and they are the standard
  consumer-page usages of the USB HID Usage Tables (§10). **Not read back from hardware.**
- Where they land in a real read (`p0.json`, slot 0, wired Apex 7, firmware 4.16.3):
  `wheel_mappings` = `0xE9` / `0xEA` (the two wheel caps), and the media cluster keys
  `0xf1` = `0xE2`, `0xf5` = `0xCD`, `0xf6` = `0xB5`, `0xf7` = `0xB6`, all function `0x61`.
  Those five keys are the only remaps of the factory base layer.
- Shown by `tools/profile-gui/` (`MEDIA_NAMES`, `keyName(code, functionByte)`, 6 selftest checks
  against `p0.json`). The Rust decoder prints code bytes as **hex only** - no key-name table.

## Known facts (2026-10-18, driver mode)

- **Driver mode (engine side, superseded 2026-10-18: ssgg sends neither frame, the read needs neither -
  §Known facts (2026-10-18, sequences the read does not need))** - engine-side reading in
  `docs/development/apex-7-onboard-profile-read.md` §Driver mode. Driver mode is report 0, command `0x6C`, last
  byte 1 to open and 0 to close; the engine writes the opening frame at device init and the closing frame when it
  closes a session, and the capture shows only the opening frame (`6c 00 01`, the chain's fourth frame) because GG
  was killed in the capture. The engine's session close also writes a lighting-reset frame (`0x3B`) and the
  OLED-off frame (`0x69`). **None of these is sent by ssgg**, and what the close changes on the device is not
  measured.
- **Driver mode is not one frame across families** - engine-side descriptors (`apex_3.lisp:23`, `:32`,
  `apex_3_tkl.lisp:99`, `:100`, `:109`, `base_apex_legacy.lisp:562`, `:570`; `ONBOARD.md` line 10 keeps those
  descriptors out of code comments). Legacy Apex family (this read, the `0x1612` devices): command `0x6C`, opening
  `profile_driver_mode 0` + `macro_driver_mode 1`, closing both 0. **Apex 3: command `0x03` in a 33-byte report**,
  with a `reserved` byte, and its opening and closing frames are byte-identical (both `profile_button_driver_mode
  0`, `macro_button_driver_mode 1`). **Apex 3 TKL: command `0x6C` in a 65-byte report, opening with
  `profile_driver_mode 1` + `macro_driver_mode 1`**, closing both 0; its descriptor states the purpose - the mode
  is what makes SSKey combos appear in the keyboard report. **A port of the read to the Apex 3 family cannot
  reuse the closing frame.**
