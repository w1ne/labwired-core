#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
"""Wrap a PIC18 Intel HEX image in a minimal ELF32 container.

SDCC/gputils emit Intel HEX (and COFF), not ELF. LabWired boots PIC18
firmware from ELF images stamped EM_MCHP_PIC (204), the machine value
Microchip's XC8 uses. This tool copies the HEX data records byte for byte
into one PT_LOAD segment per contiguous run (p_paddr = p_vaddr = the HEX
byte address, entry 0 = the reset vector). It adds no code and changes no
byte; `crates/core/tests/pic18f4520_smoke.rs` checks that the ELF payload
equals the HEX.

Usage: hex2elf.py IN.hex OUT.elf
"""
import struct
import sys

EM_MCHP_PIC = 204


def parse_ihex(text):
    data = {}
    upper = 0
    for lineno, line in enumerate(text.splitlines(), 1):
        line = line.strip()
        if not line:
            continue
        if not line.startswith(":"):
            raise SystemExit(f"line {lineno}: not an Intel HEX record")
        raw = bytes.fromhex(line[1:])
        if sum(raw) & 0xFF:
            raise SystemExit(f"line {lineno}: bad checksum")
        count, addr, rtype = raw[0], (raw[1] << 8) | raw[2], raw[3]
        payload = raw[4 : 4 + count]
        if rtype == 0x00:
            for i, b in enumerate(payload):
                data[upper + addr + i] = b
        elif rtype == 0x01:
            break
        elif rtype == 0x04:
            upper = ((payload[0] << 8) | payload[1]) << 16
        else:
            raise SystemExit(f"line {lineno}: unsupported record type {rtype:#04x}")
    return data


def runs(data):
    out = []
    for addr in sorted(data):
        if out and out[-1][0] + len(out[-1][1]) == addr:
            out[-1][1].append(data[addr])
        else:
            out.append((addr, bytearray([data[addr]])))
    return out


def build_elf(segments):
    ehsize, phentsize = 52, 32
    phoff = ehsize
    offset = phoff + phentsize * len(segments)
    phdrs, blobs = b"", b""
    for addr, blob in segments:
        # PT_LOAD, offset, vaddr, paddr, filesz, memsz, flags R|X, align 1
        phdrs += struct.pack("<IIIIIIII", 1, offset, addr, addr, len(blob), len(blob), 5, 1)
        blobs += bytes(blob)
        offset += len(blob)
    ident = b"\x7fELF" + bytes([1, 1, 1, 0]) + bytes(8)
    header = ident + struct.pack(
        "<HHIIIIIHHHHHH",
        2,  # ET_EXEC
        EM_MCHP_PIC,
        1,  # EV_CURRENT
        0,  # e_entry: reset vector
        phoff,
        0,  # no section headers
        0,
        ehsize,
        phentsize,
        len(segments),
        40,
        0,
        0,
    )
    return header + phdrs + blobs


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    with open(sys.argv[1]) as f:
        segments = runs(parse_ihex(f.read()))
    with open(sys.argv[2], "wb") as f:
        f.write(build_elf(segments))


if __name__ == "__main__":
    main()
