#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
"""Regenerate and check `PIC16_DEVICES` in crates/core/src/cpu/pic16.rs.

For each classic mid-range part, check that every SFR address and bit
position the PIC16 model implements matches the SDCC 4.4.0 `pic14` device
header, then derive the rest of the row:

* flash words: the end of the last gputils `<part>_g.lkr` code page,
* port masks: the `RA0..RE7` bit fields the header declares for PORTA-E,
* memory aliases: the lkr `SHAREBANK` groups (the first range of a group
  is the backing store; every other range in it aliases that one).

Needs a local upstream SDCC 4.4.0 (distribution packages often omit the
PIC ports) and gputils 1.4.0. Neither is vendored, so CI does not run this
script.

    export SDCC_SHARE=/opt/sdcc-4.4.0/share/sdcc
    export GPUTILS_LKR=/usr/share/gputils/lkr      # default

    # Re-verify every row already committed in pic16.rs:
    python3 scripts/pic16/check_sfr_map.py --check-table

    # Print table rows for the parts on stdin (`pic16f877a`, one per line):
    python3 scripts/pic16/check_sfr_map.py --table < parts.txt

    # JSON with the mismatches per part on stdin:
    python3 scripts/pic16/check_sfr_map.py < parts.txt
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
PIC16_RS = REPO / "crates" / "core" / "src" / "cpu" / "pic16.rs"

# SFRs the model implements, at their PIC16F877A addresses (DS39582B).
SFRS = {
    "INDF": 0x00, "TMR0": 0x01, "PCL": 0x02, "STATUS": 0x03, "FSR": 0x04,
    "PORTA": 0x05, "PORTB": 0x06, "PORTC": 0x07,
    "PCLATH": 0x0A, "INTCON": 0x0B, "PIR1": 0x0C,
    "RCSTA": 0x18, "TXREG": 0x19,
    "OPTION_REG": 0x81, "TRISA": 0x85, "TRISB": 0x86, "TRISC": 0x87,
    "PIE1": 0x8C, "TXSTA": 0x98, "SPBRG": 0x99,
}
# Optional SFRs: if the part has them they must be at these addresses.
OPTIONAL_SFRS = {
    "PORTD": 0x08, "PORTE": 0x09, "TRISD": 0x88, "TRISE": 0x89,
    "PIR2": 0x0D, "PIE2": 0x8D,
}
# Bit masks the model decodes.
BITS = {
    "C": 0x01, "DC": 0x02, "Z": 0x04, "NOT_PD": 0x08, "NOT_TO": 0x10,
    "RP0": 0x20, "RP1": 0x40, "IRP": 0x80,
    "GIE": 0x80, "PEIE": 0x40,
    "TXIF": 0x10, "TXEN": 0x20, "TRMT": 0x02, "SPEN": 0x80,
}


def sdcc_include() -> pathlib.Path:
    share = os.environ.get("SDCC_SHARE")
    if not share:
        sys.exit("error: set SDCC_SHARE to an SDCC 4.4.0 share/sdcc directory")
    return pathlib.Path(share) / "non-free" / "include" / "pic14"


def lkr_dir() -> pathlib.Path:
    return pathlib.Path(os.environ.get("GPUTILS_LKR", "/usr/share/gputils/lkr"))


def header(part: str) -> str:
    return (sdcc_include() / f"{part}.h").read_text()


def check(part: str) -> list[str]:
    """Mismatches between the model's SFR map and the part's header."""
    h = header(part)
    addrs = {m[0]: int(m[1], 16)
             for m in re.findall(r"#define\s+(\w+)_ADDR\s+0x([0-9A-Fa-f]+)", h)}
    bad = []
    if "BSR" in addrs:
        bad.append("enhanced mid-range (BSR)")
    for name, want in SFRS.items():
        if addrs.get(name) != want:
            bad.append(f"{name} at {addrs.get(name)!r}, model {want:#x}")
    for name, want in OPTIONAL_SFRS.items():
        if name in addrs and addrs[name] != want:
            bad.append(f"{name} at {addrs[name]:#x}, model {want:#x}")
    for name, want in BITS.items():
        found = re.search(rf"#define\s+_{name}\s+0x([0-9A-Fa-f]+)", h)
        if not found or int(found[1], 16) != want:
            bad.append(f"bit {name} {found[1] if found else None}, model {want:#x}")
    return bad


def geometry(part: str):
    """(flash words, port masks, aliases) from the lkr and the header."""
    lkr = (lkr_dir() / f"{part[3:]}_g.lkr").read_text()
    pages = re.findall(
        r"CODEPAGE\s+NAME=page\d*\s+START=(0x\w+)\s+END=(0x\w+)", lkr)
    flash_words = max(int(end, 16) for _, end in pages) + 1
    groups: dict[str, list[tuple[int, int]]] = {}
    for name, start, end in re.findall(
            r"SHAREBANK\s+NAME=(\w+)\s+START=(0x\w+)\s+END=(0x\w+)", lkr):
        groups.setdefault(name, []).append((int(start, 16), int(end, 16)))
    aliases = []
    for ranges in groups.values():
        target = ranges[0][0]
        for start, end in ranges[1:]:
            aliases.append((start, end, target))
    h = header(part)
    masks = []
    for port in "ABCDE":
        mask = 0
        for bit in range(8):
            if re.search(rf"unsigned\s+R{port}{bit}\s*:\s*1", h):
                mask |= 1 << bit
        masks.append(mask)
    return flash_words, masks, sorted(aliases)


def row(part: str) -> str:
    flash_words, masks, aliases = geometry(part)
    m = ", ".join(f"0x{x:02X}" for x in masks)
    a = ", ".join(f"(0x{s:03X}, 0x{e:03X}, 0x{t:03X})" for s, e, t in aliases)
    return (f'    dev("{part}", {flash_words}, [{m}], &[{a}]),')


def committed_rows() -> list[str]:
    return re.findall(r'^    dev\("pic\w+".*$', PIC16_RS.read_text(), re.M)


def main() -> int:
    if "--check-table" in sys.argv:
        rows = committed_rows()
        bad = 0
        for committed in rows:
            part = re.match(r'    dev\("(\w+)"', committed)[1]
            mism = check(part)
            want = row(part)
            if mism or want != committed:
                bad += 1
                print(f"{part}: {mism or ''}\n  committed {committed}\n  derived   {want}")
        print(f"{len(rows)} rows, {bad} mismatches")
        return 1 if bad else 0
    parts = [p.strip().lower() for p in sys.stdin if p.strip()]
    if "--table" in sys.argv:
        for part in parts:
            mism = check(part)
            if mism:
                print(f"    // {part}: {'; '.join(mism)}")
            else:
                print(row(part))
        return 0
    print(json.dumps({p: check(p) for p in parts}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
