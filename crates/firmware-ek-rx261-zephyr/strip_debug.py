#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
"""Drop the `.debug*` sections of a 32-bit little-endian ELF, keep the rest.

`rx-zephyr-elf-strip --strip-debug` (binutils 2.43, Zephyr SDK 1.0.1)
rewrites a Zephyr RX image's `.exvectors` segment to file offset 0 ("section
.exvectors lma 0xffffff80 adjusted to 0x100000054"): BFD's segment layout
wraps at the top of the 32-bit address space, where the RX fixed vector table
lives. This does the one thing the fixtures need instead: every byte a
program header loads stays at its offset, the non-debug non-loaded sections
(.symtab, .strtab, .shstrtab, .comment) are packed after them, and the
section header table is rewritten. The output is a function of the input
only, so a rebuild stays byte-identical.

Usage: strip_debug.py <in.elf> <out.elf>
"""
import struct
import sys

EHDR = "<16sHHIIIIIHHHHHH"
SHDR = "<IIIIIIIIII"
SHT_NOBITS = 8


def main(src, dst):
    data = open(src, "rb").read()
    (ident, e_type, e_machine, e_version, e_entry, e_phoff, e_shoff, e_flags,
     e_ehsize, e_phentsize, e_phnum, e_shentsize, e_shnum, e_shstrndx) = \
        struct.unpack_from(EHDR, data, 0)
    if ident[:4] != b"\x7fELF" or ident[4] != 1 or ident[5] != 1:
        sys.exit(f"{src}: not a 32-bit little-endian ELF")
    shdrs = [list(struct.unpack_from(SHDR, data, e_shoff + i * e_shentsize))
             for i in range(e_shnum)]
    names = shdrs[e_shstrndx]
    strtab = data[names[4]:names[4] + names[5]]

    def name(sh):
        return strtab[sh[0]:strtab.index(b"\0", sh[0])].decode()

    # Everything up to the end of the last loaded byte (and the program
    # header table) is kept verbatim.
    keep_end = e_phoff + e_phnum * e_phentsize
    for i in range(e_phnum):
        p_type, p_offset, _, _, p_filesz = struct.unpack_from(
            "<IIIII", data, e_phoff + i * e_phentsize)
        if p_type == 1 and p_filesz:
            keep_end = max(keep_end, p_offset + p_filesz)

    dropped = {i for i, sh in enumerate(shdrs) if name(sh).startswith(".debug")}
    remap, kept = {}, []
    for i, sh in enumerate(shdrs):
        if i not in dropped:
            remap[i] = len(kept)
            kept.append(sh)

    out = bytearray(data[:keep_end])
    for sh in kept:
        sh_type, sh_offset, sh_size = sh[1], sh[4], sh[5]
        if sh_type == SHT_NOBITS or sh_size == 0 or sh_offset + sh_size <= keep_end:
            continue
        align = max(sh[8], 1)
        out += b"\0" * (-len(out) % align)
        out += data[sh_offset:sh_offset + sh_size]
        sh[4] = len(out) - sh_size
    for sh in kept:
        # sh_link indexes another section (SYMTAB -> STRTAB).
        if sh[6] in dropped:
            sh[6] = 0
        elif sh[6]:
            sh[6] = remap[sh[6]]
    # Symbols name their section by index: follow the renumbering.
    for sh in kept:
        if sh[1] != 2:  # SHT_SYMTAB
            continue
        for off in range(sh[4], sh[4] + sh[5], 16):
            shndx = struct.unpack_from("<H", out, off + 14)[0]
            if 0 < shndx < 0xFF00:
                new = remap.get(shndx, 0)
                struct.pack_into("<H", out, off + 14, new)
    out += b"\0" * (-len(out) % 4)
    new_shoff = len(out)
    for sh in kept:
        out += struct.pack(SHDR, *sh)
    struct.pack_into(EHDR, out, 0, ident, e_type, e_machine, e_version, e_entry,
                     e_phoff, new_shoff, e_flags, e_ehsize, e_phentsize, e_phnum,
                     e_shentsize, len(kept), remap[e_shstrndx])
    open(dst, "wb").write(out)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
