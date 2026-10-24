#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Turn the one-section relocatable object llvm-mc assembles for Xtensa into
an executable ELF loaded at a fixed address.

LLVM 18 assembles Xtensa (llvm-mc -triple=xtensa) but neither generates code
for it nor links it (ld.lld crashes on an EM_XTENSA object), so the
gpio-net-buses Xtensa firmware is hand-written assembly with every branch and
literal resolved by the assembler: the object has one `.text` section and no
relocations. This places that section at `base` as one PT_LOAD segment and
points e_entry at `_start`.

    xtensa_elf.py in.o out.elf 0x40378000
"""
import struct
import sys

EM_XTENSA = 94


def sections(data):
    shoff, = struct.unpack_from("<I", data, 0x20)
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", data, 0x2E)
    hdrs = [struct.unpack_from("<10I", data, shoff + i * shentsize) for i in range(shnum)]
    strtab = hdrs[shstrndx]
    def name(off):
        start = strtab[4] + off
        return data[start:data.index(b"\0", start)].decode()
    return [(name(h[0]), h) for h in hdrs]


def main(src, dst, base):
    data = open(src, "rb").read()
    if data[:4] != b"\x7fELF" or struct.unpack_from("<H", data, 0x12)[0] != EM_XTENSA:
        sys.exit(f"{src}: not an Xtensa ELF object")
    secs = sections(data)
    text = None
    entry = None
    for name, h in secs:
        sh_type, sh_flags, sh_offset, sh_size = h[1], h[2], h[4], h[5]
        if sh_type in (4, 9) and sh_size:  # SHT_RELA / SHT_REL
            sys.exit(f"{src}: {name} holds relocations; resolve them in the source")
        if sh_flags & 2 and sh_size:  # SHF_ALLOC
            if name != ".text" or text is not None:
                sys.exit(f"{src}: unexpected allocated section {name}")
            text = data[sh_offset:sh_offset + sh_size]
        if sh_type == 2:  # SHT_SYMTAB
            strtab = secs[h[6]][1]
            for k in range(sh_size // 16):
                st_name, st_value = struct.unpack_from("<II", data, sh_offset + 16 * k)
                start = strtab[4] + st_name
                if data[start:data.index(b"\0", start)] == b"_start":
                    entry = st_value
    if text is None or entry is None:
        sys.exit(f"{src}: needs a .text section and a _start symbol")

    ehsize, phsize, shsize = 52, 32, 40
    text_off = ehsize + phsize
    shstr = b"\0.text\0.shstrtab\0"
    shstr_off = text_off + len(text)
    sh_off = (shstr_off + len(shstr) + 3) & ~3
    out = bytearray()
    out += b"\x7fELF" + bytes([1, 1, 1, 0]) + bytes(8)
    out += struct.pack("<HHIIIIIHHHHHH", 2, EM_XTENSA, 1, base + entry, ehsize, sh_off,
                       0, ehsize, phsize, 1, shsize, 3, 2)
    # PT_LOAD, R+X
    out += struct.pack("<8I", 1, text_off, base, base, len(text), len(text), 5, 4)
    out += text + shstr
    out += bytes(sh_off - len(out))
    out += bytes(shsize)
    out += struct.pack("<10I", 1, 1, 6, base, text_off, len(text), 0, 0, 4, 0)
    out += struct.pack("<10I", 7, 3, 0, 0, shstr_off, len(shstr), 0, 0, 1, 0)
    open(dst, "wb").write(out)


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2], int(sys.argv[3], 0))
