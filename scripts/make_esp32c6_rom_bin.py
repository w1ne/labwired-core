#!/usr/bin/env python3
"""Flatten Espressif's esp32c6_rev0_rom.elf into the C6 mask-ROM window.

Writes the 320 KiB image the `esp32c6` chip's `rom` region loads
(0x4000_0000..0x4005_0000, `LABWIRED_ESP32C6_ROM`).

Two passes, mirroring `crates/core/src/boot/esp32s3_rom.rs::build_window`:
  1. PT_LOAD file bytes inside the window (code, `.rodata`).
  2. PROGBITS sections inside the window that no PT_LOAD carries. The ROM
     ELF's `.rodata.interface` (0x4004_FD40) is not ALLOC, so its segment has
     a zero file size; it holds `ets_rom_layout_p` (0x4004_FFFC), which
     ESP-IDF 5 heap init dereferences. Without it the reserved-region table
     reads 0xFFFF_FFFF and `soc_get_available_memory_regions` aborts.

Usage: make_esp32c6_rom_bin.py <esp32c6_rev0_rom.elf> <out.bin>
The ELF ships in espressif/esp-rom-elfs (release 20260528 was used).
"""
import struct
import sys

BASE, SIZE = 0x4000_0000, 0x5_0000


def main(src: str, dst: str) -> None:
    b = open(src, "rb").read()
    out = bytearray(SIZE)
    phoff, = struct.unpack_from("<I", b, 28)
    phentsz, phnum = struct.unpack_from("<HH", b, 42)
    for i in range(phnum):
        p_type, off, va, _pa, fsz, _msz, _fl, _al = struct.unpack_from("<8I", b, phoff + i * phentsz)
        if p_type == 1 and fsz and BASE <= va < BASE + SIZE:
            n = min(fsz, BASE + SIZE - va)
            out[va - BASE:va - BASE + n] = b[off:off + n]
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
