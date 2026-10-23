#!/usr/bin/env python3
"""Generate the dsPIC33FJ32MC204 smoke fixture ELF in XC16 layout.

There is no open-source PIC24 assembler in CI, so the fixture is assembled by
hand from the encodings in DS70000157G (see docs/pic24-m0-reference.md):

    .reset (PC 0x0000):  GOTO 0x200
    .text  (PC 0x0200):  MOV  #'H', W0        ; 0020 0480
                         MOV  W0, 0x0224      ; 8811 20   (U1TXREG)
                         NOP x6
                         BRA  $-1             ; 0037 FFFF (self-loop)

The ELF follows the real XC16 v1.70 layout, verified against a compiler-built
image (examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-uart-smoke.elf):
addresses are **PC units** (2 per instruction word), executable sections are
arrays of **4-byte slots** (little-endian 24-bit word + one pad byte), and the
reset vector at address 0 holds the 2-word GOTO into user code at 0x200.
DS50002071F's "2 program counter units are equivalent to 3 bytes" describes
the assembler's conceptual addressing, not ELF file storage.

Run:  python3 make-fixture-elf.py            # writes dspic33fj32mc204-uart-smoke.elf
"""
import struct
from pathlib import Path

EM_DSPIC30F = 118
# XC16 linker scripts put user code at 0x200 (PC units).
CODE_PC = 0x200
# GOTO 0x200 is a 2-word instruction: 0000 0100 nnnn... + high-bits word.
RESET_WORDS = [0x040200, 0x000000]
PROGRAM_WORDS = [
    0x00200480,  # MOV #0x48, W0      ('H')
    0x00881120,  # MOV W0, 0x0224     (U1TXREG)
    0x00000000,  # NOP
    0x00000000,  # NOP
    0x00000000,  # NOP
    0x00000000,  # NOP
    0x00000000,  # NOP
    0x00000000,  # NOP
    0x0037FFFF,  # BRA $-1            (self-loop)
]


def slots(words):
    """XC16 executable-section bytes: each 24-bit word in a 4-byte slot."""
    return b"".join(w.to_bytes(4, "little") for w in words)


RESET = slots(RESET_WORDS)
CODE = slots(PROGRAM_WORDS)

RESET_OFF = 0x200
CODE_OFF = 0x300
SHSTR_OFF = 0x400
SHOFF = 0x500
SHSTR = b"\x00.reset\x00.text\x00.shstrtab\x00"

ehdr = struct.pack(
    "<16sHHIIIIIHHHHHH",
    b"\x7fELF\x01\x01\x01" + b"\x00" * 9,
    2, EM_DSPIC30F, 1,
    CODE_PC,                 # e_entry: PC units
    52,                      # e_phoff
    SHOFF,                   # e_shoff
    0, 0,                    # e_flags, e_ehsize
    32, 2,                   # e_phentsize, e_phnum
    40, 4, 3,                # e_shentsize, e_shnum, e_shstrndx
)
# One PT_LOAD per section (the core PIC24 parser reads sections, but keeping
# the segments consistent keeps generic ELF tooling happy).
ph_reset = struct.pack("<IIIIIIII", 1, RESET_OFF, 0, 0, len(RESET), len(RESET), 5, 2)
ph_code = struct.pack(
    "<IIIIIIII", 1, CODE_OFF, CODE_PC, CODE_PC, len(CODE), len(CODE), 5, 2
)


def sh(name_off, typ, flags, addr, off, size, align):
    return struct.pack(
        "<IIIIIIIIII", name_off, typ, flags, addr, off, size, 0, 0, align, 0
    )


shdrs = (
    sh(0, 0, 0, 0, 0, 0, 0)
    + sh(1, 1, 0x6, 0, RESET_OFF, len(RESET), 2)          # .reset, AX
    + sh(8, 1, 0x6, CODE_PC, CODE_OFF, len(CODE), 2)      # .text, AX
    + sh(14, 3, 0, 0, SHSTR_OFF, len(SHSTR), 1)           # .shstrtab
)

blob = bytearray(ehdr + ph_reset + ph_code)
blob.extend(b"\x00" * (RESET_OFF - len(blob)))
blob.extend(RESET)
blob.extend(b"\x00" * (CODE_OFF - len(blob)))
blob.extend(CODE)
blob.extend(b"\x00" * (SHSTR_OFF - len(blob)))
blob.extend(SHSTR)
blob.extend(b"\x00" * (SHOFF - len(blob)))
blob.extend(shdrs)

out = Path(__file__).parent / "dspic33fj32mc204-uart-smoke.elf"
out.write_bytes(blob)
print(
    f"wrote {out} ({len(blob)} bytes, reset GOTO + {len(PROGRAM_WORDS)} instructions, "
    f"entry 0x{CODE_PC:x}, {len(RESET)} reset bytes, {len(CODE)} code bytes)"
)
