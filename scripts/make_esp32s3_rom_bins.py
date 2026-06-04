#!/usr/bin/env python3
"""Extract flat ROM (IROM) and DROM images from the genuine Espressif
ESP32-S3 boot-ROM ELF, for LabWired's faithful `--rom-boot` path.

The ESP32-S3 boot ROM is dual-mapped on silicon:
  * instruction bus 0x4000_0000..0x4006_0000  (384 KiB)  -> LABWIRED_ESP32S3_ROM
  * data bus        0x3FF0_0000..0x3FF2_0000  (128 KiB)  -> LABWIRED_ESP32S3_DROM

`configure_xtensa_esp32s3` loads each as a flat image based at its window
start, so this script walks the ELF program headers and lays every PT_LOAD
segment whose vaddr falls in a window at the window-relative offset.

The ROM blob is Espressif's copyright; it is NOT vendored. Point this at the
copy shipped with the ESP toolchain, e.g.
  ~/.platformio/tools/tool-esp-rom-elfs/esp32s3_rev0_rom.elf

Usage:
  make_esp32s3_rom_bins.py <esp32s3_rev0_rom.elf> [out_dir]
Writes <out_dir>/esp32s3_rom.bin and <out_dir>/esp32s3_drom.bin.
"""
import struct
import sys
from pathlib import Path

WINDOWS = {
    "esp32s3_rom.bin": (0x4000_0000, 0x6_0000),
    "esp32s3_drom.bin": (0x3FF0_0000, 0x2_0000),
}


def load_segments(elf: bytes):
    """Yield (p_vaddr, p_paddr, data) for each PT_LOAD with file content."""
    if elf[:4] != b"\x7fELF":
        raise SystemExit("not an ELF")
    e_phoff = struct.unpack_from("<I", elf, 0x1C)[0]
    e_phentsize = struct.unpack_from("<H", elf, 0x2A)[0]
    e_phnum = struct.unpack_from("<H", elf, 0x2C)[0]
    for i in range(e_phnum):
        off = e_phoff + i * e_phentsize
        p_type, p_offset, p_vaddr, p_paddr, p_filesz = struct.unpack_from(
            "<5I", elf, off
        )
        if p_type == 1 and p_filesz:  # PT_LOAD with file bytes
            yield p_vaddr, p_paddr, elf[p_offset : p_offset + p_filesz]


def progbits_sections(elf: bytes):
    """Yield (sh_addr, bytes) for every SHT_PROGBITS section with addr+content."""
    e_shoff = struct.unpack_from("<I", elf, 0x20)[0]
    e_shentsize = struct.unpack_from("<H", elf, 0x2E)[0]
    e_shnum = struct.unpack_from("<H", elf, 0x30)[0]
    for i in range(e_shnum):
        off = e_shoff + i * e_shentsize
        sh_type = struct.unpack_from("<I", elf, off + 4)[0]
        sh_addr = struct.unpack_from("<I", elf, off + 0x0C)[0]
        sh_offset = struct.unpack_from("<I", elf, off + 0x10)[0]
        sh_size = struct.unpack_from("<I", elf, off + 0x14)[0]
        if sh_type == 1 and sh_size and sh_addr:
            yield sh_addr, elf[sh_offset : sh_offset + sh_size]


def populate_data_copy_sources(elf: bytes, irom: bytearray, irom_base: int):
    """Reconstruct the ROM's data-init copy sources in the IROM image.

    The boot ROM's startup copies `.data` from IROM-resident load addresses
    (LMAs) into DRAM via a table of (dst_start, dst_end, src, 0) quadruples.
    But Espressif's ROM ELF stores many `.data.interface.*` init values in
    SECTIONS that are in no PT_LOAD segment, so those copy-source LMAs read 0
    in a PT_LOAD-only image — and the ROM faithfully copies 0 over pointers
    like rom_cache_internal_table_ptr (0x3FCEFFC4 → 0x3FF1E2B4), then calls a
    null cache vtable method. We walk the in-image copy table and fill each
    `src` LMA with the genuine bytes the matching DRAM `dst` section holds, so
    the ROM's own copy lands the real values. Faithful: the bytes are the
    ROM's, just relocated to where its copy loop reads them.
    """
    # VMA -> bytes lookup across all PROGBITS sections (covers .data.interface.*).
    sections = sorted(progbits_sections(elf))

    def vma_read(addr, n):
        out = bytearray(n)
        for sa, data in sections:
            if sa <= addr + n and addr < sa + len(data):
                lo = max(addr, sa)
                hi = min(addr + n, sa + len(data))
                out[lo - addr : hi - addr] = data[lo - sa : hi - sa]
        return bytes(out)

    DRAM_LO, DRAM_HI = 0x3FC8_8000, 0x3FD0_0000
    IROM_HI = irom_base + len(irom)
    entries = 0
    off = 0
    # Scan the IROM image for the contiguous copy table (16-byte quads).
    while off + 16 <= len(irom):
        dst_s, dst_e, src, term = struct.unpack_from("<4I", irom, off)
        ok = (
            DRAM_LO <= dst_s < DRAM_HI
            and dst_s <= dst_e < DRAM_HI
            and irom_base <= src < IROM_HI
            and term == 0
            and (dst_e - dst_s) < 0x1_0000
        )
        if ok:
            n = dst_e - dst_s
            if n:
                vals = vma_read(dst_s, n)
                if any(vals):  # only fill when the section actually has data
                    rel = src - irom_base
                    irom[rel : rel + n] = vals[: max(0, min(n, len(irom) - rel))]
            entries += 1
            off += 16
        else:
            off += 4
    print(f"  data-copy table: populated {entries} source entries")


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    elf = Path(sys.argv[1]).read_bytes()
    out = Path(sys.argv[2]) if len(sys.argv) > 2 else Path("/tmp")
    out.mkdir(parents=True, exist_ok=True)
    segs = list(load_segments(elf))
    # The IROM window must be laid out by LOAD ADDRESS (p_paddr): code segments
    # have paddr==vaddr, but the ROM's `.data` is stored at an IROM LMA
    # (e.g. 0x400577A8, vaddr 0x3FCD7E00 in DRAM) and the ROM's own startup
    # data-init table copies from those LMAs into DRAM at boot. Keying on vaddr
    # would drop that `.data` source, so the copy writes zeros — e.g.
    # rom_cache_internal_table_ptr (0x3FCEFFC4) stays 0 and the cache routines
    # call a null vtable method. The DROM (data-bus) window is keyed by vaddr.
    KEY_BY_PADDR = {"esp32s3_rom.bin"}
    for name, (base, size) in WINDOWS.items():
        img = bytearray(size)
        placed = 0
        for vaddr, paddr, data in segs:
            addr = paddr if name in KEY_BY_PADDR else vaddr
            if base <= addr < base + size:
                rel = addr - base
                n = min(len(data), size - rel)
                img[rel : rel + n] = data[:n]
                placed += 1
        # Fill the boot ROM's `.data` copy-source LMAs from section content so
        # the ROM's own startup copy lands real values (see function docstring).
        if name in KEY_BY_PADDR:
            populate_data_copy_sources(elf, img, base)
        (out / name).write_bytes(img)
        print(f"{out / name}: {size} bytes, {placed} segments (base 0x{base:08x})")


if __name__ == "__main__":
    main()
