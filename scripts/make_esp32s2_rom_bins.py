#!/usr/bin/env python3
"""Extract flat ROM images from the genuine Espressif ESP32-S2 boot-ROM ELF.

The ESP32-S2 mask ROM has three CPU-visible pieces (ESP32-S2 TRM, "System and
Memory", and the ROM ELF's own section table):
  * instruction bus 0x4000_0000..0x4002_0000 (128 KiB)  -> esp32s2_rom.bin
  * data bus        0x3FFA_0000..0x3FFB_0000 (64 KiB)   -> esp32s2_drom.bin
  * the ROM's initialised `.data_*` sections, which live at the top of
    internal SRAM, 0x3FFF_E000..0x4000_0000 (8 KiB)      -> esp32s2_rom_data.bin

The ROM's reset handler copies `.data_*` from their load addresses in IROM to
SRAM. LabWired enters the app at its ELF entry and skips that reset, so the
third image lands the same bytes the copy would have, at the same addresses.
The `.data_*` sections are PROGBITS but not ALLOC, so no PT_LOAD carries them:
this script walks the section table, not the program headers.

Source: `esp32s2_rev0_rom.elf` from espressif/esp-rom-elfs (release 20260528).
The ROM contents are Espressif's copyright; see crates/core/roms/esp32s2/README.md.

Usage:
  make_esp32s2_rom_bins.py <esp32s2_rev0_rom.elf> [out_dir]
"""
import struct
import sys
from pathlib import Path

WINDOWS = {
    "esp32s2_rom.bin": (0x4000_0000, 0x2_0000),
    "esp32s2_drom.bin": (0x3FFA_0000, 0x1_0000),
    "esp32s2_rom_data.bin": (0x3FFF_E000, 0x2000),
}


def progbits_sections(elf: bytes):
    """Yield (name, sh_addr, bytes) for every SHT_PROGBITS section with content."""
    if elf[:4] != b"\x7fELF":
        raise SystemExit("not an ELF")
    e_shoff = struct.unpack_from("<I", elf, 0x20)[0]
    e_shentsize = struct.unpack_from("<H", elf, 0x2E)[0]
    e_shnum = struct.unpack_from("<H", elf, 0x30)[0]
    e_shstrndx = struct.unpack_from("<H", elf, 0x32)[0]
    stroff = struct.unpack_from("<I", elf, e_shoff + e_shstrndx * e_shentsize + 0x10)[0]
    for i in range(e_shnum):
        off = e_shoff + i * e_shentsize
        sh_name, sh_type = struct.unpack_from("<2I", elf, off)
        sh_addr, sh_offset, sh_size = struct.unpack_from("<3I", elf, off + 0x0C)
        if sh_type == 1 and sh_size and sh_addr:
            end = elf.index(b"\0", stroff + sh_name)
            name = elf[stroff + sh_name : end].decode()
            yield name, sh_addr, elf[sh_offset : sh_offset + sh_size]


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    elf = Path(sys.argv[1]).read_bytes()
    out_dir = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(".")
    out_dir.mkdir(parents=True, exist_ok=True)
    images = {name: bytearray(size) for name, (_, size) in WINDOWS.items()}
    placed = {name: 0 for name in WINDOWS}
    for sec, addr, data in progbits_sections(elf):
        for name, (base, size) in WINDOWS.items():
            if base <= addr and addr + len(data) <= base + size:
                images[name][addr - base : addr - base + len(data)] = data
                placed[name] += len(data)
                break
        else:
            # `.static_dram_start` is a 4-byte linker marker in ROM bss space,
            # not data the ROM copies; nothing else may fall outside.
            if sec != ".static_dram_start":
                raise SystemExit(f"section {sec} @ {addr:#010x} fits no window")
    for name, img in images.items():
        (out_dir / name).write_bytes(img)
        print(f"{name}: {len(img)} bytes, {placed[name]} from the ELF")
    return 0


if __name__ == "__main__":
    sys.exit(main())
