#!/usr/bin/env python3
"""Flatten Espressif's esp32c2_rev200_rom.elf into the C2 mask-ROM window.

Writes the 576 KiB image the `esp32c2` chip's `rom` region loads
(0x4000_0000..0x4009_0000, `LABWIRED_ESP32C2_ROM`).

Two passes, mirroring `crates/core/src/boot/esp32s3_rom.rs::build_window`:
  1. PT_LOAD file bytes placed at their LOAD (physical) address inside the
     window. For code VMA == LMA. The `.data_*` initialisers (VMA in DRAM,
     LMA 0x4008_23B0..) and `.rodata` (VMA 0x3FF4_2C00 on the DROM bus, LMA
     0x4008_2C00) are mask-ROM bytes too: the ROM unpacks the first through
     `_rom_store_table` and reads the second through the DROM alias.
  2. PROGBITS sections inside the window that no PT_LOAD carries. For the
     C2 rev 2 ROM every window section (.fixed.text, .init.text, .bt_text,
     .text, 0x4000_0000..0x4008_23B0) is in a PT_LOAD already, so this pass
     changes nothing; it is kept so a later ROM ELF cannot drop a section.

Usage: make_esp32c2_rom_bin.py <esp32c2_rev200_rom.elf> <out.bin>
The ELF ships in espressif/esp-rom-elfs (release 20260528 was used).
"""
import struct
import sys

BASE, SIZE = 0x4000_0000, 0x9_0000


def main(src: str, dst: str) -> None:
    b = open(src, "rb").read()
    out = bytearray(SIZE)
    phoff, = struct.unpack_from("<I", b, 28)
    phentsz, phnum = struct.unpack_from("<HH", b, 42)
    for i in range(phnum):
        p_type, off, _va, pa, fsz, _msz, _fl, _al = struct.unpack_from("<8I", b, phoff + i * phentsz)
        if p_type == 1 and fsz and BASE <= pa < BASE + SIZE:
            n = min(fsz, BASE + SIZE - pa)
            out[pa - BASE:pa - BASE + n] = b[off:off + n]
    shoff, = struct.unpack_from("<I", b, 32)
    shentsz, shnum = struct.unpack_from("<HH", b, 46)
    for i in range(shnum):
        _name, sh_type, _flags, addr, off, sz = struct.unpack_from("<6I", b, shoff + i * shentsz)
        if sh_type == 1 and sz and BASE <= addr < BASE + SIZE:
            n = min(sz, BASE + SIZE - addr)
            out[addr - BASE:addr - BASE + n] = b[off:off + n]
    open(dst, "wb").write(out)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
