"""Survey the macro record region of every Apex 7 profile payload captured on this machine.

Ground truth for the decoder: region start/length, packed record chain, per-record header and action
counts, the zero run behind the chain, the CRC field, and what each macro binding resolves to under the
binding rule the codec uses. Run: python3 -I scripts/analyze-macro-region.py [dir]
"""

import pathlib
import sys

# Region, mapping tables and payload tail, mirrored from the offsets in apex_7_profile.rs.
SCHEMA_A, SCHEMA_B = 0x000A, 0x000B
PAYLOAD_LEN_A, PAYLOAD_LEN_B = 11648, 11696
NAME_OFFSET, NAME_LEN = 2, 24
LIGHTING_LEN = 57
KEY_MAPPING_LEN, BUTTON_MAPPING_COUNT, WHEEL_MAPPING_COUNT = 5, 128, 2
BUTTON_MAPPINGS_OFFSET = NAME_OFFSET + NAME_LEN + 1 + LIGHTING_LEN + 1
BUTTON_META_MAPPINGS_OFFSET = BUTTON_MAPPINGS_OFFSET + (BUTTON_MAPPING_COUNT + WHEEL_MAPPING_COUNT) * KEY_MAPPING_LEN
WHEEL_META_MAPPINGS_OFFSET = BUTTON_META_MAPPINGS_OFFSET + BUTTON_MAPPING_COUNT * KEY_MAPPING_LEN
MACRO_REGION_START_B = 2076
MACRO_DATA_LEN = 9600
# The six reads surveyed packed their records from 3600 inside the block, 608 bytes of them.
SURVEYED_CHAIN_START, SURVEYED_CHAIN_END = 3600, 4208
MARKER = 0x77
ACTION_KEY, ACTION_MOUSE, ACTION_DELAY = 0x02, 0x00, 0x04
# Every action is 4 bytes: key push 02 key 01 01, key release 02 key 01 00, mouse press 00 mask 00 01,
# mouse release 00 mask 00 00 (mask 0 is the zero pad, not an action), wait 04 00 ms.
ACTION_LEN = 4
# A binding byte names a record by its offset in the block in 4-byte units.
INDEX_UNIT = 4
BINDING_BYTE_MAX = 255
MOUSE_BITS = [(0x01, 1), (0x02, 2), (0x04, 3), (0x08, 4), (0x10, 5), (0x20, 6)]


def mask_words(mask):
    return "+".join(str(n) for bit, n in MOUSE_BITS if mask & bit) or f"mask 0x{mask:02X}"


def header(payload, at, limit):
    """Record header at `at`, or None when the bytes there are not one."""
    if at + 8 > limit or payload[at] != MARKER:
        return None
    mode = payload[at + 1]
    if mode not in (1, 2, 3, 4, 5, 6, 7, 8):
        return None
    return {
        "mode": mode,
        "header_word": int.from_bytes(payload[at + 2 : at + 4], "little"),
        "flags": int.from_bytes(payload[at + 4 : at + 6], "little"),
        "interval": int.from_bytes(payload[at + 6 : at + 8], "little"),
    }


def actions(payload, at, limit):
    """Actions packed behind a record header. Returns (counts, masks, ms, stop, end)."""
    counts = {"press": 0, "release": 0, "press_mouse": 0, "release_mouse": 0, "delay": 0}
    masks = []
    ms = []
    pos = at
    while pos + ACTION_LEN <= limit:
        unit = payload[pos : pos + ACTION_LEN]
        if unit[0] == ACTION_KEY and unit[2] == 0x01 and unit[3] in (0, 1):
            counts["press" if unit[3] == 1 else "release"] += 1
        elif unit[0] == ACTION_MOUSE and unit[2] == 0x00 and unit[1] != 0:
            counts["press_mouse" if unit[3] == 1 else "release_mouse"] += 1
            masks.append(unit[1])
        elif unit[0] == ACTION_DELAY and unit[1] == 0x00:
            counts["delay"] += 1
            ms.append(int.from_bytes(unit[2:4], "little"))
        else:
            return counts, masks, ms, f"byte {unit[0]:#04x} at {pos}", pos
        pos += ACTION_LEN
    return counts, masks, ms, "", pos


def walk(payload, start, limit):
    """Walk the block the way the codec walks it: every 0x77 header whose actions parse is a record,
    the bytes that are not are counted and skipped, so a chain packed past GG's own structures is found.
    Returns (records, other_bytes, stop_reason)."""
    records = []
    other = 0
    stop = ""
    at = start
    while at < limit:
        found = header(payload, at, limit)
        if found is None:
            other += payload[at] != 0
            at += 1
            continue
        counts, masks, ms, why, end = actions(payload, at + 8, limit)
        if not sum(counts.values()) or end >= limit:
            other += sum(1 for byte in payload[at:end] if byte != 0)
            stop = stop or why
            at = max(end, at + 1)
            continue
        records.append(
            {
                "offset": at,
                "index": (at - start) // INDEX_UNIT,
                **found,
                "counts": counts,
                "masks": masks,
                "ms": ms,
                "actions": sum(counts.values()),
                "bytes": end - at,
                "stop": why,
            }
        )
        at = end
    return records, other, stop


def bindings(payload, records, start):
    """Every 0x71 (macro) binding row in the base and META mapping tables, resolved by the binding rule.

    Yields (table, slot, binding byte, record, record the superseded 7 x position rule would name).
    """
    by_offset = {rec["offset"]: rec for rec in records}
    for table, offset, count in (
        ("base", BUTTON_MAPPINGS_OFFSET, BUTTON_MAPPING_COUNT),
        ("meta", BUTTON_META_MAPPINGS_OFFSET, BUTTON_MAPPING_COUNT),
        ("wheel meta", WHEEL_META_MAPPINGS_OFFSET, BUTTON_MAPPING_COUNT),
    ):
        for slot in range(count):
            row = payload[offset + slot * KEY_MAPPING_LEN : offset + slot * KEY_MAPPING_LEN + KEY_MAPPING_LEN]
            if len(row) < KEY_MAPPING_LEN or row[0] != 0x71:
                continue
            byte = row[3]
            if byte == 0 and not any(by_offset.get(INDEX_UNIT * value) for value in range(byte, byte + 1)):
                continue
            here = start + INDEX_UNIT * byte
            # The superseded rule read the byte as 7 x the record's position in the chain.
            legacy = records[byte // 7] if byte // 7 < len(records) else None
            yield table, slot, byte, by_offset.get(here), legacy


def describe(rec):
    if rec is None:
        return "no record"
    counts = rec["counts"]
    parts = [f"{counts[k]} {k}" for k in counts if counts[k]]
    masks = ", ".join(f"mouse {mask_words(mask)}" for mask in rec["masks"])
    return (
        f"@{rec['offset']} mode={rec['mode']} header_word={rec['header_word']} "
        f"interval={rec['interval']} {', '.join(parts)}"
        + (f" [{masks}]" if masks else "")
    )


def main():
    paths = sorted(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/images").glob("*.bin"))
    for path in paths:
        payload = path.read_bytes()
        schema = int.from_bytes(payload[0:2], "little")
        # The block is the tail of the payload; the chain the six reads packed sits inside it.
        start = MACRO_REGION_START_B
        limit = min(len(payload), start + MACRO_DATA_LEN)
        if schema == SCHEMA_A:
            limit = min(limit, PAYLOAD_LEN_A)
        records, other, stop = walk(payload, start, limit)
        region = payload[start:limit]
        used = max((rec["offset"] + rec["bytes"] for rec in records), default=start) - start
        zeros = 0
        at = start + used
        while at < len(payload) and payload[at] == 0:
            zeros += 1
            at += 1
        name = payload[NAME_OFFSET : NAME_OFFSET + 10].split(b"\0")[0].decode("ascii", "replace")
        print(f"== {path.name} payload={len(payload)} schema={schema:#06x} name={name!r}")
        print(
            f"   block {start}..{limit} chain={SURVEYED_CHAIN_START}..{SURVEYED_CHAIN_END} used={used}"
            f" records={len(records)} other_bytes={other} zeros_behind_chain={zeros}"
            f" content_resumes_at={at} stop={stop!r}"
        )
        for rec in records:
            counts = " ".join(f"{n}={rec['counts'][n]}" for n in rec["counts"] if rec["counts"][n])
            masks = " ".join(f"{mask_words(mask):#>1}" for mask in rec["masks"])
            print(
                f"   rec@{rec['offset']} index={rec['index']} mode={rec['mode']}"
                f" header_word={rec['header_word']} flags={rec['flags']:#06x} interval={rec['interval']}"
                f" bytes={rec['bytes']} actions={rec['actions']} {counts}"
                + (f" masks={masks}" if masks else "")
                + (f" ms={set(rec['ms'])}" if rec["ms"] else "")
                + (f" stop={rec['stop']!r}" if rec["stop"] else "")
            )
        markers = sum(1 for byte in region if byte == MARKER)
        print(f"   0x77 bytes in block: {markers}")
        block_start = start - records[0]["offset"] if records else start
        for table, slot, byte, rec, legacy in bindings(payload, records, block_start):
            if byte == 0 and rec is None:
                continue
            print(
                f"   {table} slot {slot}: binding byte {byte} -> {describe(rec)}"
                f" | 7 x position would name {describe(legacy)}"
            )


main()
