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
  is the backing store; every other range in it aliases that one),
* own SFRs: the bank-2/3 mirror addresses of TMR0/PORTB/OPTION_REG/TRISB
  (0x101/0x106/0x181/0x186) at which the header defines a register of the
  part's own (ANSELB at 0x186 on the PIC16F72x), so the model must not
  mirror them.

Needs a local upstream SDCC 4.4.0 (distribution packages often omit the
PIC ports) and gputils 1.4.0. Neither is vendored, so CI does not run this
script.

    export SDCC_SHARE=/opt/sdcc-4.4.0/share/sdcc
    export GPUTILS_LKR=/usr/share/gputils/lkr      # default

    # Re-verify every row already committed in pic16.rs:
    python3 scripts/pic16/check_sfr_map.py --check-table

    # Print table rows for the parts on stdin (`pic16f877a`, one per line);
    # add --enhanced for PIC16F1xxx parts (checked against the PIC16E_*_MAP
    # that the part's PIR1 address and OSCSTAT ready bits pick):
    python3 scripts/pic16/check_sfr_map.py --table < parts.txt
    python3 scripts/pic16/check_sfr_map.py --enhanced --table < parts.txt

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
# Bank-2/3 mirrors of TMR0, PORTB, OPTION_REG and TRISB in the classic model
# (`Pic16::canonical`). A part that defines its own register at one of them
# lists it in its row (`dev_own`).
MIRRORS = (0x101, 0x106, 0x181, 0x186)

SFRS = {
    "INDF": 0x00, "TMR0": 0x01, "PCL": 0x02, "STATUS": 0x03, "FSR": 0x04,
    "PORTA": 0x05,
    "PCLATH": 0x0A, "INTCON": 0x0B,
    "OPTION_REG": 0x81, "TRISA": 0x85,
}
# PIR1/PIE1 and the PEIE bit. A part with neither (PIC16F84A: GPR from 0x0C)
# gets `PIC16_CLASSIC_F84A_MAP`: no PIR/PIE, no USART, and only PORTA/PORTB
# decoded, so the addresses of PORTC-E/TRISC-E (EEDATA/EEADR, EECON1/2 on
# the 84A) are the part's own.
PIR_SFRS = {"PIR1": 0x0C, "PIE1": 0x8C}
PIR_BITS = ("PEIE",)
PORTS_CDE = ("PORTC", "PORTD", "PORTE", "TRISC", "TRISD", "TRISE")
# The USART (`PIC16_CLASSIC_MAP`). A part with none of these registers gets
# `PIC16_CLASSIC_NO_USART_MAP`, which leaves 0x18/0x19/0x98/0x99 and PIR1.TXIF
# as plain registers; a part with any of them must have all four here.
USART_SFRS = {"RCSTA": 0x18, "TXREG": 0x19, "TXSTA": 0x98, "SPBRG": 0x99}
USART_BITS = ("TXIF", "TXEN", "TRMT", "SPEN")
# Optional SFRs: if the part has them they must be at these addresses.
OPTIONAL_SFRS = {
    "PORTD": 0x08, "PORTE": 0x09, "TRISD": 0x88, "TRISE": 0x89,
    "PIR2": 0x0D, "PIE2": 0x8D,
}
# Optional ports the model always decodes (PORTx reads latch & !TRISx &
# mask). A part without one (no PORTC: PIC16F627A/628A/648A, PIC16F87/88;
# no PORTB: the 14-pin PIC16F610/616/630/676/684) gets a zero port mask, so
# the port reads 0 and ignores writes, as on silicon; the address must then
# be free in the header.
OPTIONAL_FREE_SFRS = {"PORTB": 0x06, "PORTC": 0x07, "TRISB": 0x86, "TRISC": 0x87}
# Bit masks the model decodes.
BITS = {
    "C": 0x01, "DC": 0x02, "Z": 0x04, "NOT_PD": 0x08, "NOT_TO": 0x10,
    "RP0": 0x20, "RP1": 0x40, "IRP": 0x80,
    "GIE": 0x80, "PEIE": 0x40,
    "TXIF": 0x10, "TXEN": 0x20, "TRMT": 0x02, "SPEN": 0x80,
}
# Classic map names in pic16.rs, by whether the part has the USART.
# Classic map names in pic16.rs, by (has the USART, has PIR1/PIE1).
MAPS = {
    (True, True): "PIC16_CLASSIC_MAP",
    (False, True): "PIC16_CLASSIC_NO_USART_MAP",
    (False, False): "PIC16_CLASSIC_F84A_MAP",
}


def has_usart(addrs: dict[str, int]) -> bool:
    return any(name in addrs for name in USART_SFRS)


def has_pir(addrs: dict[str, int]) -> bool:
    return any(name in addrs for name in PIR_SFRS)


def classic_map(addrs: dict[str, int]) -> str | None:
    return MAPS.get((has_usart(addrs), has_pir(addrs)))


# Enhanced mid-range (PIC16F1xxx, 32 banks): the PIC16F182x/F193x layout
# the model implements (`PIC16E_F182X_MAP`, DS40001440E Table 3-8). Each
# entry lists the header names that may carry the register.
E_SFRS = {
    ("INDF0",): 0x000, ("INDF1",): 0x001, ("PCL",): 0x002, ("STATUS",): 0x003,
    ("FSR0L",): 0x004, ("FSR0H",): 0x005, ("FSR1L",): 0x006, ("FSR1H",): 0x007,
    ("BSR",): 0x008, ("WREG",): 0x009, ("PCLATH",): 0x00A, ("INTCON",): 0x00B,
    ("PORTA",): 0x00C, ("TMR0",): 0x015,
    ("TRISA",): 0x08C, ("OPTION_REG",): 0x095, ("PCON",): 0x096,
    ("LATA",): 0x10C,
    ("TXREG", "TX1REG"): 0x19A, ("SPBRGL", "SPBRG", "SP1BRGL"): 0x19B,
    ("RCSTA", "RC1STA"): 0x19D, ("TXSTA", "TX1STA"): 0x19E,
    ("STATUS_SHAD",): 0xFE4, ("WREG_SHAD",): 0xFE5, ("BSR_SHAD",): 0xFE6,
    ("PCLATH_SHAD",): 0xFE7, ("FSR0L_SHAD",): 0xFE8, ("FSR0H_SHAD",): 0xFE9,
    ("FSR1L_SHAD",): 0xFEA, ("FSR1H_SHAD",): 0xFEB,
    ("STKPTR",): 0xFED, ("TOSL",): 0xFEE, ("TOSH",): 0xFEF,
}
# The model touches these addresses on every enhanced part (reset values,
# the PIRx/PIEx interrupt scan, ports, OSCSTAT ready bits). A part may lack
# the register, but then no other SFR may sit at the address.
E_OPTIONAL_SFRS = {"OSCSTAT": 0x09A}
# The two enhanced layouts, keyed by where PIR1 sits: how many PORT/TRIS/LAT
# addresses the model decodes from PORTA up (`Pic16SfrMap::ports`), and its
# PIR/PIE list. PIC16F161x has PORTA-C only, so its PIR1-5/PIE1-5 start one
# address lower than the F182x's PIR1-4/PIE1-4.
E_LAYOUTS = {
    0x011: {"ports": 5, "pir": 4},
    0x010: {"ports": 3, "pir": 5},
}


def e_layout_sfrs(pir1: int) -> tuple[dict, dict]:
    """(required, optional) layout-dependent SFRs for the layout with PIR1
    at `pir1`: PIR1/PIE1 must be there; the other ports and PIRx/PIEx are
    optional but nothing else may sit at their addresses."""
    lay = E_LAYOUTS[pir1]
    required = {("PIR1",): pir1, ("PIE1",): pir1 + 0x80}
    optional = {}
    for n in range(1, lay["ports"]):
        port = "ABCDE"[n]
        optional[f"PORT{port}"] = 0x00C + n
        optional[f"TRIS{port}"] = 0x08C + n
        optional[f"LAT{port}"] = 0x10C + n
    for n in range(1, lay["pir"]):
        optional[f"PIR{n + 1}"] = pir1 + n
        optional[f"PIE{n + 1}"] = pir1 + 0x80 + n
    return required, optional

# (register names, bit names, position) the model decodes.
E_BITS = [
    (("STATUS",), ("C",), 0), (("STATUS",), ("DC",), 1), (("STATUS",), ("Z",), 2),
    (("STATUS",), ("NOT_PD",), 3), (("STATUS",), ("NOT_TO",), 4),
    # INTCON<2:0> flags pair with INTCON<5:3> enables.
    (("INTCON",), ("IOCIF",), 0), (("INTCON",), ("INTF",), 1),
    (("INTCON",), ("TMR0IF",), 2), (("INTCON",), ("IOCIE",), 3),
    (("INTCON",), ("INTE",), 4), (("INTCON",), ("TMR0IE",), 5),
    (("INTCON",), ("PEIE",), 6), (("INTCON",), ("GIE",), 7),
    (("OPTION_REG",), ("PS0",), 0), (("OPTION_REG",), ("PSA",), 3),
    (("OPTION_REG",), ("TMR0CS", "T0CS"), 5),
    (("PIR1",), ("TXIF", "TX1IF"), 4),
    (("TXSTA", "TX1STA"), ("TRMT",), 1), (("TXSTA", "TX1STA"), ("TXEN",), 5),
    (("RCSTA", "RC1STA"), ("SPEN",), 7),
    (("PCON",), ("NOT_RI",), 2), (("PCON",), ("STKUNF",), 6),
    (("PCON",), ("STKOVF",), 7),
    (("BSR",), ("BSR4",), 4),
]
# OSCSTAT<4:0> read as "ready" (the `osc_ready` mask): each of those bits
# must be one of these clock-ready/stable flags, or unimplemented.
E_OSC_READY = {"HFIOFS": 0, "LFIOFR": 1, "MFIOFR": 2, "HFIOFL": 3, "HFIOFR": 4}
# The enhanced maps in pic16.rs, by (PIR1 address, OSCSTAT ready mask).
# Parts without the MFINTOSC (PIC16F145x/150x/151x) have no MFIOFR/HFIOFL.
E_MAPS = {
    (0x011, 0x1F): "PIC16E_F182X_MAP",
    (0x011, 0x13): "PIC16E_F151X_MAP",
    (0x010, 0x1F): "PIC16E_F161X_MAP",
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
    usart, pir = has_usart(addrs), has_pir(addrs)
    if classic_map(addrs) is None:
        bad.append("USART without PIR1/PIE1: no map")
    if not pir:
        bad += [f"{n} present, model decodes PORTA/PORTB only"
                for n in PORTS_CDE if n in addrs]
    required = {**SFRS, **(USART_SFRS if usart else {}), **(PIR_SFRS if pir else {})}
    for name, want in required.items():
        if addrs.get(name) != want:
            bad.append(f"{name} at {addrs.get(name)!r}, model {want:#x}")
    for name, want in OPTIONAL_SFRS.items():
        if not pir and name in PORTS_CDE:
            continue
        if name in addrs and addrs[name] != want:
            bad.append(f"{name} at {addrs[name]:#x}, model {want:#x}")
    for name, want in OPTIONAL_FREE_SFRS.items():
        if not pir and name in PORTS_CDE:
            continue
        if name in addrs:
            if addrs[name] != want:
                bad.append(f"{name} at {addrs[name]:#x}, model {want:#x}")
        else:
            others = sorted(n for n, a in addrs.items() if a == want)
            if others:
                bad.append(f"{want:#x} is {others[0]}, model {name}")
    for name, want in BITS.items():
        if (not usart and name in USART_BITS) or (not pir and name in PIR_BITS):
            continue
        found = re.search(rf"#define\s+_{name}\s+0x([0-9A-Fa-f]+)", h)
        if not found or int(found[1], 16) != want:
            bad.append(f"bit {name} {found[1] if found else None}, model {want:#x}")
    return bad


def bit_fields(h: str) -> dict[str, dict[str, int]]:
    """Register -> {1-bit field name: bit position} from the header's
    `__<REG>bits_t` unions/structs."""
    regs = {}
    for kind, body, reg in re.findall(
            r"typedef (union|struct)\s*\{(.*?)\}\s*__(\w+)bits_t;", h, re.S):
        structs = re.findall(r"struct\s*\{(.*?)\};", body, re.S) if kind == "union" else [body]
        fields = {}
        for st in structs:
            pos = 0
            for name, width in re.findall(r"unsigned\s+(\w*)\s*:\s*(\d+)", st):
                if name and width == "1":
                    fields.setdefault(name, pos)
                pos += int(width)
        regs[reg] = fields
    return regs


def osc_ready(h: str) -> int:
    """The OSCSTAT ready bits the part implements."""
    fields = bit_fields(h).get("OSCSTAT", {})
    return sum(1 << pos for name, pos in E_OSC_READY.items() if fields.get(name) == pos)


def check_enhanced(part: str) -> list[str]:
    """Mismatches between the enhanced model maps and the part's header."""
    h = header(part)
    addrs = {m[0]: int(m[1], 16)
             for m in re.findall(r"#define\s+(\w+)_ADDR\s+0x([0-9A-Fa-f]+)", h)}
    regs = bit_fields(h)
    bad = []
    if "BSR" not in addrs:
        return ["classic mid-range (no BSR)"]
    if not (lkr_dir() / f"{part[3:]}_g.lkr").exists():
        bad.append("no gputils 1.4.0 linker script (cannot cross-check or build)")
    pir1 = addrs.get("PIR1") if addrs.get("PIR1") in E_LAYOUTS else 0x011
    required, optional = e_layout_sfrs(pir1)
    for names, want in {**E_SFRS, **required}.items():
        if not any(addrs.get(n) == want for n in names):
            bad.append(f"{names[0]} at {addrs.get(names[0])!r}, model {want:#x}")
    for name, want in {**E_OPTIONAL_SFRS, **optional}.items():
        if name in addrs:
            if addrs[name] != want:
                bad.append(f"{name} at {addrs[name]:#x}, model {want:#x}")
        else:
            others = sorted(n for n, a in addrs.items() if a == want)
            if others:
                bad.append(f"{want:#x} is {others[0]}, model {name}")
    for reg_names, bit_names, want in E_BITS:
        reg = next((r for r in reg_names if r in regs), None)
        got = None if reg is None else next(
            (regs[reg][b] for b in bit_names if b in regs[reg]), None)
        if got != want:
            bad.append(f"bit {reg_names[0]}.{bit_names[0]} at {got!r}, model {want}")
    if "BSR5" in regs.get("BSR", {}):
        bad.append("64 banks (BSR5)")
    if "OSCSTAT" in addrs:
        osc = {pos: name for name, pos in regs.get("OSCSTAT", {}).items()}
        for name, pos in E_OSC_READY.items():
            if osc.get(pos) not in (name, None):
                bad.append(f"OSCSTAT bit {pos} is {osc[pos]!r}, model {name}")
        if (pir1, osc_ready(h)) not in E_MAPS:
            bad.append(f"PIR1 {pir1:#x} with OSCSTAT ready mask {osc_ready(h):#04x} has no map")
    cfg = {m[0]: int(m[1], 16)
           for m in re.findall(r"#define\s+(_CONFIG[12]|_STVREN_OFF|_STVREN_ON)\s+0x([0-9A-Fa-f]+)", h)}
    if cfg.get("_CONFIG1") != 0x8007 or cfg.get("_CONFIG2") != 0x8008:
        bad.append("CONFIG words not at 0x8007/0x8008")
    if cfg.get("_STVREN_ON", 0) ^ cfg.get("_STVREN_OFF", 0) != 1 << 9:
        bad.append("CONFIG2.STVREN not bit 9")
    elif not re.search(r"CONFIG2 Options.*?_STVREN_OFF", h, re.S) or re.search(
            r"CONFIG1 Options(?:(?!CONFIG2 Options).)*_STVREN_OFF", h, re.S):
        bad.append("STVREN not in CONFIG2")
    return bad


def geometry(part: str):
    """(flash words, port masks, aliases, own SFRs, map name) from the lkr
    and the header. Flash includes the protected `oscval` page: on the
    PIC16F630/676 the last word (0x3FF) holds the factory OSCCAL `retlw`,
    and the datasheet counts it in the 1K words."""
    lkr = (lkr_dir() / f"{part[3:]}_g.lkr").read_text()
    pages = re.findall(
        r"CODEPAGE\s+NAME=(?:page\d*|oscval)\s+START=(0x\w+)\s+END=(0x\w+)", lkr)
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
    defined = {int(a, 16)
               for a in re.findall(r"#define\s+\w+_ADDR\s+0x([0-9A-Fa-f]+)", h)}
    own = sorted(a for a in MIRRORS if a in defined)
    addrs = {m[0]: int(m[1], 16)
             for m in re.findall(r"#define\s+(\w+)_ADDR\s+0x([0-9A-Fa-f]+)", h)}
    return flash_words, masks, sorted(aliases), own, classic_map(addrs)


def row(part: str) -> str:
    flash_words, masks, aliases, own, map_name = geometry(part)
    m = ", ".join(f"0x{x:02X}" for x in masks)
    a = ", ".join(f"(0x{s:03X}, 0x{e:03X}, 0x{t:03X})" for s, e, t in aliases)
    if map_name != MAPS[(True, True)]:
        if own:
            sys.exit(f"error: {part}: no row form for own SFRs on {map_name}")
        return f'    dev_map("{part}", {flash_words}, [{m}], &[{a}], {map_name}),'
    if own:
        o = ", ".join(f"0x{x:03X}" for x in own)
        return f'    dev_own("{part}", {flash_words}, [{m}], &[{a}], &[{o}]),'
    return (f'    dev("{part}", {flash_words}, [{m}], &[{a}]),')


def device_db(part: str) -> dict[str, str]:
    """The part's entry in SDCC's `pic14devices.txt` (key -> raw value)."""
    text = (sdcc_include().parents[2] / "include" / "pic14" / "pic14devices.txt").read_text()
    short = part[3:]
    # Some entries are not blank-line separated, and some name lists miss a
    # comma: split on the `processor` lines and on commas or spaces.
    for block in re.split(r"\n(?=\s*processor\s)", text):
        m = re.match(r"\s*processor\s+(.*)", block)
        if m and short in re.split(r"[\s,]+", m[1].strip()):
            return {k: v.strip() for k, v in re.findall(r"^\s*(\w+)\s+(.*)$", block, re.M)}
    raise KeyError(f"{part} not in pic14devices.txt")


def port_masks(part: str) -> list[int]:
    h = header(part)
    masks = []
    for port in "ABCDE":
        mask = 0
        for bit in range(8):
            if re.search(rf"unsigned\s+R{port}{bit}\s*:\s*1", h):
                mask |= 1 << bit
        masks.append(mask)
    return masks


def enhanced_geometry(part: str):
    """(flash words, port masks, map name) for an enhanced part. Flash words
    come from SDCC's `pic14devices.txt`, cross-checked against the lkr
    (the part must have one: SDCC builds go through gputils)."""
    k = device_db(part)["program"]
    flash_words = int(k[:-1]) * 1024 if k.endswith("K") else int(k)
    lkr = lkr_dir() / f"{part[3:]}_g.lkr"
    if not lkr.exists():
        # Without gputils support the part cannot be cross-checked or built.
        sys.exit(f"error: {part}: no gputils linker script {lkr.name}")
    pages = re.findall(r"CODEPAGE\s+NAME=page\d*\s+START=(0x\w+)\s+END=(0x\w+)",
                       lkr.read_text())
    lkr_words = max(int(end, 16) for _, end in pages) + 1
    if lkr_words != flash_words:
        sys.exit(f"error: {part}: pic14devices.txt {flash_words} words, lkr {lkr_words}")
    h = header(part)
    pir1 = int(re.search(r"#define\s+PIR1_ADDR\s+0x([0-9A-Fa-f]+)", h)[1], 16)
    return flash_words, port_masks(part), E_MAPS[(pir1, osc_ready(h))]


def enhanced_row(part: str) -> str:
    flash_words, masks, map_name = enhanced_geometry(part)
    m = ", ".join(f"0x{x:02X}" for x in masks)
    return f'    edev("{part}", {flash_words}, [{m}], {map_name}),'


def committed_enhanced_rows() -> dict[str, tuple]:
    """The `edev(...)` rows in `PIC16_DEVICES`."""
    rows = {}
    for m in re.finditer(r'\bedev\(\s*"(\w+)",\s*(\d+),\s*\[([^\]]*)\],\s*(\w+),?\s*\)',
                         PIC16_RS.read_text(), re.S):
        masks = [int(x, 16) for x in re.findall(r"0x([0-9A-Fa-f]+)", m[3])]
        rows[m[1]] = (int(m[2]), masks, m[4])
    return rows


def committed_rows() -> dict[str, tuple]:
    """The `dev(...)` rows in `PIC16_DEVICES`, as (flash words, port masks,
    aliases), whatever line breaks rustfmt gave them."""
    src = PIC16_RS.read_text()
    table = re.search(r"const PIC16_DEVICES: &\[Pic16Device\] = &\[(.*?)\n\];", src, re.S)
    if not table:
        sys.exit("error: PIC16_DEVICES not found in pic16.rs")
    rows = {}
    for m in re.finditer(
            r'\bdev(_own|_map)?\(\s*"(\w+)",\s*(\d+),\s*\[([^\]]*)\],\s*&\[(.*?)\]'
            r'(?:,\s*&\[([^\]]*)\])?(?:,\s*(PIC16_\w+))?,?\s*\)',
            table[1], re.S):
        masks = [int(x, 16) for x in re.findall(r"0x([0-9A-Fa-f]+)", m[4])]
        aliases = [tuple(int(x, 16) for x in t)
                   for t in re.findall(r"\(0x(\w+),\s*0x(\w+),\s*0x(\w+)\)", m[5])]
        own = [int(x, 16) for x in re.findall(r"0x([0-9A-Fa-f]+)", m[6] or "")]
        rows[m[2]] = (int(m[3]), masks, aliases, own, m[7] or MAPS[(True, True)])
    return rows


def main() -> int:
    if "--check-table" in sys.argv:
        rows = committed_rows()
        if not rows:
            sys.exit("error: no dev(...) rows parsed from PIC16_DEVICES")
        bad = 0
        for part, committed in rows.items():
            mism = check(part)
            want = geometry(part)
            if mism or tuple(want) != committed:
                bad += 1
                print(f"{part}: {mism or ''}\n  committed {committed}\n  derived   {want}")
        erows = committed_enhanced_rows()
        if not erows:
            sys.exit("error: no edev(...) rows parsed from PIC16_DEVICES")
        for part, committed in erows.items():
            mism = check_enhanced(part)
            want = None if mism else enhanced_geometry(part)
            if mism or want != committed:
                bad += 1
                print(f"{part}: {mism or ''}\n  committed {committed}\n  derived   {want}")
        print(f"{len(rows)} classic + {len(erows)} enhanced rows, {bad} mismatches")
        return 1 if bad else 0
    parts = [p.strip().lower() for p in sys.stdin if p.strip()]
    enhanced = "--enhanced" in sys.argv
    checker, make_row = (check_enhanced, enhanced_row) if enhanced else (check, row)
    if "--table" in sys.argv:
        for part in parts:
            mism = checker(part)
            if mism:
                print(f"    // {part}: {'; '.join(mism)}")
            else:
                print(make_row(part))
        return 0
    print(json.dumps({p: checker(p) for p in parts}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
