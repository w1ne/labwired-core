#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
"""Regenerate and check `PIC18_DEVICES` in crates/core/src/cpu/pic18.rs.

For each part, check that every SFR address and bit position the PIC18
model implements matches the SDCC 4.4.0 device header, and report the
flash size (gputils `<part>_g.lkr` code page) and the Access Bank split
(SDCC `pic16devices.txt`).

Needs a local upstream SDCC 4.4.0 (distribution packages often omit the
pic16 port) and gputils 1.4.0. Neither is vendored, so CI does not run
this script.

    export SDCC_SHARE=/opt/sdcc-4.4.0/share/sdcc
    export GPUTILS_LKR=/usr/share/gputils/lkr      # default

    # Re-verify every row already committed in pic18.rs:
    python3 scripts/pic18/check_sfr_map.py --check-table

    # Print table rows for the parts on stdin (`pic18f4520`, one per line):
    python3 scripts/pic18/check_sfr_map.py --table < parts.txt

    # JSON with the mismatches per part on stdin:
    python3 scripts/pic18/check_sfr_map.py < parts.txt
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
PIC18_RS = REPO / "crates" / "core" / "src" / "cpu" / "pic18.rs"

# SFRs the model implements, at their PIC18F4520 addresses.
SFRS = {
    "WREG": 0xFE8, "STATUS": 0xFD8, "BSR": 0xFE0,
    "FSR0L": 0xFE9, "FSR0H": 0xFEA, "FSR1L": 0xFE1, "FSR1H": 0xFE2,
    "FSR2L": 0xFD9, "FSR2H": 0xFDA,
    "INDF0": 0xFEF, "POSTINC0": 0xFEE, "POSTDEC0": 0xFED,
    "PREINC0": 0xFEC, "PLUSW0": 0xFEB,
    "INDF1": 0xFE7, "POSTINC1": 0xFE6, "POSTDEC1": 0xFE5,
    "PREINC1": 0xFE4, "PLUSW1": 0xFE3,
    "INDF2": 0xFDF, "POSTINC2": 0xFDE, "POSTDEC2": 0xFDD,
    "PREINC2": 0xFDC, "PLUSW2": 0xFDB,
    "PRODL": 0xFF3, "PRODH": 0xFF4, "TABLAT": 0xFF5,
    "TBLPTRL": 0xFF6, "TBLPTRH": 0xFF7, "TBLPTRU": 0xFF8,
    "PCL": 0xFF9, "PCLATH": 0xFFA, "PCLATU": 0xFFB,
    "STKPTR": 0xFFC, "TOSL": 0xFFD, "TOSH": 0xFFE, "TOSU": 0xFFF,
    "INTCON": 0xFF2, "INTCON2": 0xFF1, "INTCON3": 0xFF0,
    "PIE1": 0xF9D, "PIR1": 0xF9E, "IPR1": 0xF9F,
    "PIE2": 0xFA0, "PIR2": 0xFA1, "IPR2": 0xFA2,
    "RCON": 0xFD0,
    "RCSTA": 0xFAB, "TXSTA": 0xFAC, "TXREG": 0xFAD, "SPBRG": 0xFAF,
    "PORTA": 0xF80, "PORTB": 0xF81, "PORTC": 0xF82,
    "LATA": 0xF89, "LATB": 0xF8A, "LATC": 0xF8B,
    "TRISA": 0xF92, "TRISB": 0xF93, "TRISC": 0xF94,
}

# Header names that are the same register on parts with two EUSARTs.
SFR_ALIASES = {
    "TXREG": ["TXREG", "TXREG1"],
    "TXSTA": ["TXSTA", "TXSTA1"],
    "RCSTA": ["RCSTA", "RCSTA1"],
    "SPBRG": ["SPBRG", "SPBRG1"],
    "INTCON": ["INTCON", "INTCON1"],
}

# Bits the model implements: register -> {bit: position}.
BITS = {
    "INTCON": {"GIE": 7, "PEIE": 6, "TMR0IE": 5, "INT0IE": 4,
               "RBIE": 3, "TMR0IF": 2, "INT0IF": 1, "RBIF": 0},
    "INTCON2": {"TMR0IP": 2, "RBIP": 0},
    "INTCON3": {"INT2IP": 7, "INT1IP": 6, "INT2IE": 4, "INT1IE": 3,
                "INT2IF": 1, "INT1IF": 0},
    "RCON": {"IPEN": 7, "TO": 3, "PD": 2},
    "TXSTA": {"TXEN": 5, "TRMT": 1},
    "RCSTA": {"SPEN": 7},
    "PIR1": {"TXIF": 4},
    "STKPTR": {"STKFUL": 7, "STKUNF": 6},
}

# Alternative header spellings of the same bit.
BIT_ALIASES = {
    "GIE": ["GIE", "GIEH"],
    "PEIE": ["PEIE", "GIEL"],
    "TMR0IE": ["TMR0IE", "T0IE"],
    "TMR0IF": ["TMR0IF", "T0IF"],
    "TMR0IP": ["TMR0IP", "T0IP"],
    "INT0IE": ["INT0IE", "INT0E"],
    "INT0IF": ["INT0IF", "INT0F"],
    "INT1IE": ["INT1IE", "INT1E"],
    "INT1IF": ["INT1IF", "INT1F"],
    "INT2IE": ["INT2IE", "INT2E"],
    "INT2IF": ["INT2IF", "INT2F"],
    "RBIE": ["RBIE", "IOCIE"],
    "RBIF": ["RBIF", "IOCIF"],
    "RBIP": ["RBIP", "IOCIP"],
    "TO": ["TO", "NOT_TO", "nTO"],
    "PD": ["PD", "NOT_PD", "nPD"],
    "STKFUL": ["STKFUL", "STKOVF"],
    "TXEN": ["TXEN", "TXEN1"],
    "TRMT": ["TRMT", "TRMT1"],
    "SPEN": ["SPEN", "SPEN1"],
    "TXIF": ["TXIF", "TX1IF"],
}


def sdcc_share() -> pathlib.Path:
    share = os.environ.get("SDCC_SHARE")
    if not share:
        sys.exit("error: set SDCC_SHARE to an SDCC 4.4.0 share/sdcc directory")
    path = pathlib.Path(share)
    if not (path / "include" / "pic16" / "pic16devices.txt").is_file():
        sys.exit(f"error: {path} has no include/pic16/pic16devices.txt")
    return path


def gputils_lkr() -> pathlib.Path:
    return pathlib.Path(os.environ.get("GPUTILS_LKR", "/usr/share/gputils/lkr"))


def devices() -> dict[str, dict[str, int]]:
    """Parse pic16devices.txt into {"18f4520": {"ramsize": .., "split": ..}}."""
    out: dict[str, dict[str, int]] = {}
    cur = None
    for line in (sdcc_share() / "include" / "pic16" / "pic16devices.txt").open():
        line = line.split("#")[0].strip()
        if not line:
            continue
        key, *vals = line.split()
        if key == "name":
            cur = vals[0]
            out[cur] = {}
        elif key == "using":
            out[cur].update(out[vals[0]])
        elif key in ("ramsize", "split"):
            out[cur][key] = int(vals[0], 0)
    return out


def flash_of(dev: str) -> int | None:
    """Program flash bytes: end of the gputils linker script's code page."""
    path = gputils_lkr() / f"{dev}_g.lkr"
    if not path.is_file():
        return None
    for line in path.open():
        m = re.match(r"\s*CODEPAGE\s+NAME=page\s+START=0x0+\s+END=(0x[0-9A-Fa-f]+)", line)
        if m:
            return int(m.group(1), 16) + 1
    return None


def parse_header(dev: str):
    """Return ({sfr: address}, {register: {bit: position}}) or (None, None)."""
    path = sdcc_share() / "non-free" / "include" / "pic16" / f"pic{dev}.h"
    if not path.is_file():
        return None, None
    text = path.read_text()
    addr = {
        m.group(2): int(m.group(1), 16)
        for m in re.finditer(r"extern __at\((0x[0-9A-Fa-f]+)\) __sfr (\w+);", text)
    }
    bits: dict[str, dict[str, int]] = {}
    for m in re.finditer(r"typedef (union|struct)\s*\{(.*?)\}\s*__(\w+)bits_t;", text, re.S):
        kind, body, reg = m.groups()
        if kind == "struct":
            structs = [body]
        else:
            structs = [s.group(1) for s in re.finditer(r"struct\s*\{(.*?)\}", body, re.S)]
        positions: dict[str, int] = {}
        for struct in structs:
            pos = 0
            for f in re.finditer(r"unsigned\s*(\w*)\s*:\s*(\d+)\s*;", struct):
                name, width = f.group(1), int(f.group(2))
                if name and width == 1:
                    positions.setdefault(name, pos)
                pos += width
        bits[reg] = positions
    return addr, bits


def check(dev: str) -> list[str]:
    """Mismatches between the model's SFR map and the part's header."""
    addr, bits = parse_header(dev)
    if addr is None:
        return ["no SDCC header"]
    errs = []
    for name, want in SFRS.items():
        got = [addr[c] for c in SFR_ALIASES.get(name, [name]) if c in addr]
        if not got:
            errs.append(f"{name} missing")
        elif want not in got:
            errs.append(f"{name}@{got[0]:#x}")
    for reg, want_bits in BITS.items():
        reg_bits = next(
            (bits[c] for c in SFR_ALIASES.get(reg, [reg]) if c in bits), None)
        if reg_bits is None:
            errs.append(f"{reg}bits missing")
            continue
        for bit, want in want_bits.items():
            got = [reg_bits[c] for c in BIT_ALIASES.get(bit, [bit]) if c in reg_bits]
            if not got:
                errs.append(f"{reg}.{bit} missing")
            elif want not in got:
                errs.append(f"{reg}.{bit}@{got[0]}")
    return errs


def table_rows() -> list[tuple[str, int, int]]:
    text = PIC18_RS.read_text()
    return [
        (name, int(flash), int(split, 16))
        for name, flash, split in re.findall(
            r'^    \("(pic18\w+)", (\d+), 0x([0-9A-F]+)\),$', text, re.M)
    ]


def check_table() -> int:
    devs = devices()
    rows = table_rows()
    bad = 0
    for name, flash, split in rows:
        dev = name.replace("pic", "", 1)
        errs = check(dev)
        if errs or flash_of(dev) != flash or devs.get(dev, {}).get("split") != split:
            bad += 1
            print(name, "MISMATCH", errs, flash_of(dev), devs.get(dev, {}).get("split"))
    print(f"{len(rows)} rows checked, {bad} mismatches")
    return 1 if bad or not rows else 0


def main() -> int:
    if "--check-table" in sys.argv:
        return check_table()
    devs = devices()
    res = {}
    for part in (line.strip() for line in sys.stdin):
        if not part:
            continue
        dev = part.replace("pic", "", 1)
        info = devs.get(dev, {})
        res[part] = {
            "split": info.get("split"),
            "ram": info.get("ramsize"),
            "flash": flash_of(dev),
            "errs": check(dev),
        }
    if "--table" in sys.argv:
        ok = sorted((p for p, v in res.items() if not v["errs"]),
                    key=lambda p: (p.replace("lf", "f"), p))
        for p in ok:
            print(f'    ("{p}", {res[p]["flash"]}, 0x{res[p]["split"]:X}),')
    else:
        json.dump(res, sys.stdout, indent=1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
