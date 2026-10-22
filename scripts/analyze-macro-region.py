"""Survey the macro record region of every Apex 7 profile payload captured on this machine.

Ground truth for the decoder: region start/length, packed record chain, per-record header and action
counts, the zero run behind the chain, and the CRC field. Run: python3 -I scripts/analyze-macro-region.py
"""

import binascii
import pathlib
import sys

REGION_START = 3600
REGION_LEN = 608
MARKER = 0x77
PRESS_LEN = 8
RELEASE_LEN = 4
DELAY_LEN = 4


def walk(payload, start, limit):
    """Walk the packed record chain from `start`. Returns (records, used_bytes, stop_reason)."""
    records = []
    at = start
    while payload[at] == MARKER:
        mode = payload[at + 1]
        rec_id = int.from_bytes(payload[at + 2 : at + 4], "little")
        flags = int.from_bytes(payload[at + 4 : at + 6], "little")
        interval = int.from_bytes(payload[at + 6 : at + 8], "little")
        pos = at + 8
        press = release = delay = 0
        stop = ""
        while pos < limit and payload[pos] not in (0, MARKER):
            if payload[pos] == 0x02:
                if payload[pos + 3] == 0x01:
                    press += 1
                    pos += PRESS_LEN
                else:
                    release += 1
                    pos += RELEASE_LEN
            elif payload[pos] == 0x04:
                delay += 1
                pos += DELAY_LEN
            else:
                stop = f"byte {payload[pos]:#04x} at {pos}"
                break
        records.append(
            {
                "offset": at,
                "mode": mode,
                "id": rec_id,
                "flags": flags,
                "interval": interval,
                "press": press,
                "release": release,
                "delay": delay,
                "bytes": pos - at,
                "stop": stop,
            }
        )
        at = pos
    return records, at - start, records[-1]["stop"] if records and records[-1]["stop"] else ""


def main():
    paths = sorted(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/images").glob("*.bin"))
    for path in paths:
        payload = path.read_bytes()
        region = payload[REGION_START : REGION_START + REGION_LEN]
        records, used, stop = walk(payload, REGION_START, len(payload))
        content_end = REGION_START + used
        zeros = 0
        at = content_end
        while at < len(payload) and payload[at] == 0:
            zeros += 1
            at += 1
        crc_stored = int.from_bytes(payload[11688:11692], "little")
        crc_computed = binascii.crc32(payload[4:11688]) & 0xFFFFFFFF
        name = payload[2:12].split(b"\0")[0].decode("ascii", "replace")
        print(f"== {path.name} payload={len(payload)} name={name!r}")
        print(
            f"   region {REGION_START}..{REGION_START + REGION_LEN} used={used} free={REGION_LEN - used}"
            f" records={len(records)} zeros_behind_chain={zeros} content_resumes_at={at}"
        )
        print(f"   crc_field={crc_stored:08X} computed={crc_computed:08X} match={crc_stored == crc_computed}")
        for rec in records:
            print(
                f"   rec@{rec['offset']} mode={rec['mode']} id={rec['id']} flags={rec['flags']:#06x}"
                f" interval={rec['interval']} press={rec['press']} release={rec['release']}"
                f" delay={rec['delay']} bytes={rec['bytes']} stop={rec['stop']!r}"
            )
        markers = sum(1 for byte in region if byte == MARKER)
        print(f"   0x77 bytes in region: {markers}")


main()
