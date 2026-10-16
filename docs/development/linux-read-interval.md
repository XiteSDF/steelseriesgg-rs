# Interrupt-endpoint interval table — Linux run capture (measured 2026-10-16)

Generated 2026-10-16 from the Linux run capture: the Linux `ssgg onboard read` run on an Apex 7 (PID `0x1612`, firmware
`4.16.8`, `the hidraw node`), 1428 packets, bus 1 device 24, 4.77 s. **That capture is not in the tree** - only a
66-packet excerpt of a related run remained (sha256 <digest recorded outside the repo>),
so these numbers need a new capture to be re-verified. tshark 4.4.1, fields:
`tshark -r Linux run capture -T fields -E separator='|' -e frame.number -e frame.time_relative -e usb.address
-e usb.endpoint_address -e usb.urb_type -e usb.ev_type -e usb.setup.wValue -e usb.setup.wIndex -e usb.data`

Summary of the 14 profile reads (`0x83` request → read-back → poll):

- 14 reads, 14 GET_REPORT control transfers (13 with a 643-byte data stage, 1 with 530 bytes)
- 14 input-report polls: no report URB lands inside any of them (windows of 114-150 ms), and 7 `read(2)` calls ended
  in `EAGAIN` (frames 364, 431, 557, 765, 811, 1020, 1092) - the queue was empty when `read` was issued
- device answer latency, frame → reply URB: 1.0 ms, 3.4 ms, 2.9 ms, 1.3 ms, 1.9 ms, 1.3 ms, 1.5 ms, 1.3 ms
- poll windows: 144 ms, 146 ms, 150 ms, 114 ms, 114 ms, 147 ms, 148 ms, 149 ms
- 5 GET_REPORT transfers with no URB: 6.6 ms, 3.2 ms, 2.4 ms, 1.9 ms, 1.3 ms — all at `wIndex 3`
- interrupt URBs in flight for 263 ms of 1030 ms; 1133 gaps with no URB in flight, 52 s in total

Interval between consecutive interrupt-endpoint URBs (all 1678 of them):

| bucket [ms] | count |
|---|---|
| 10-20 | 737 |
| 40-50 | 200 |
| 30-40 | 196 |
| 20-30 | 163 |
| 2-3 | 156 |
| 3-4 | 72 |
| 4-5 | 58 |
| 5-10 | 58 |
| 1-2 | 33 |
| 0.1-1 | 5 |
| 50-100 | 1 |

737 of the 10-20 ms gaps are 11.4-11.5 ms: `usbhid` polls a collection's interrupt endpoint at 1 ms
(`HID_HUB_POLL_INTERVAL`) only while a descriptor on that node is open. The 30-50 ms gaps (396) are the
`read_timeout` windows of a descriptor that is closed again.

Per transfer type (count / median gap / p90 gap / max gap, ms):

| transfer | n | median | p90 | max |
|---|---|---|---|---|
| ep2desc OUT | 124 | 7.25 | 12.25 | 64.36 |
| ep2desc IN | 10 | 2.26 | 22.90 | 22.90 |
| ep4desc IN | 9 | 3.96 | 45.47 | 45.47 |
| ep6desc IN | 11 | 4.69 | 46.55 | 46.55 |
| ep8desc IN | 9 | 7.27 | 47.97 | 47.97 |
| ep12desc IN | 10 | 4.90 | 11.74 | 11.74 |
| ep14desc IN | 1418 | 11.45 | 44.07 | 119.11 |
| ep16desc IN | 12 | 4.63 | 47.87 | 47.87 |
| SET_REPORT wValue 0x0200 | 124 | 7.25 | 12.25 | 64.36 |
| GET_REPORT wValue 0x0300 | 14 | 74.50 | 151.61 | 164.20 |
| SET_REPORT wValue 0x0300 | 14 | 75.25 | 151.96 | 165.21 |

Read the mechanism in `apex-7-onboard-profile-read.md`, section "Input reports: the node must be open when
the report arrives".

## 5. Read sizes: why 642, and where the off-by-one was

Measured 2026-10-18 from the transport, not from a capture. The two directions of the profile transfer are
asymmetric, and the asymmetry is the kernel's:

- **Write** (`SET_REPORT`): the buffer carries a leading report-number byte, the kernel strips it and the
  wire number is the rest. `ONBOARD_FEATURE_REPORT_SIZE` (643) goes out as `wLength 642`, matching GG's
  642-byte read requests.
- **Read** (`GET_REPORT`): the requested length goes to the device verbatim (hidapi passes the slice length
  straight to `HIDIOCGFEATURE`, `src/linux_native/ioctl.rs:26`), and the kernel writes the device's report-ID
  byte into byte 0 of the same buffer. The ask is `READ_REPORT_LEN` = 642, which decodes as
  `wValue 0x0300 wLength 642` - GG's number.

The run put `GET wLength 641` on the wire (24 submits in the Linux run capture): the raw-ioctl code
that has since been deleted subtracted the report-ID byte from the length it handed the kernel, and its
"643 produces `ETIMEDOUT`" observation is the same subtraction seen from the other side. Under hidapi, asking
643 would put 643 on the wire, which is the length the keyboard has never answered.

A reply therefore arrives as `[report ID][641 bytes]`, and a header-echoing reply spends 5 of those 641 on its
header. The most a read frame may ask for is **636** payload bytes: `MAX_READ_LEN` and `ONBOARD_MAX_READ_LEN`
are both `642 - 1 - 5`, and the code asserts they are equal. The constant that bounded read frames said 641
until 2026-10-18: it subtracted the report-ID byte but not the header, allowing a read to ask for a size no
reply can carry.
