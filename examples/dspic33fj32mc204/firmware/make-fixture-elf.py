#!/usr/bin/env python3
"""Generate the dsPIC33FJ32MC204 smoke fixture ELF in XC16 layout.

There is no open-source PIC24 assembler in CI, so the fixture is assembled by
hand from the encodings in DS70000157G (see docs/pic24-m0-reference.md):

    MOV  #'H', W0        ; 0020 0480
    MOV  W0, 0x0224      ; 8811 20   (U1TXREG)
    NOP x6
    BRA  $-1             ; 0037 FFFF (self-loop → run ends at max_steps)

The ELF follows the XC16 layout documented in DS50002071F ("Program Counter
Unit"): the address space counts **2 PC units per instruction**, an executable
section stores **3 bytes per instruction** (little-endian 24-bit words), and
user code starts at PC address 0x200. Phantom bytes (a 4th pad byte) are a HEX
file concept and are deliberately not used here.

Run:  python3 make-fixture-elf.py            # writes dspic33fj32mc204-uart-smoke.elf
"""
import struct
from pathlib import Path

WORDS = [
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

EM_DSPIC30F = 118
# XC16 linker scripts put user code at 0x200 (PC units).
CODE_PC = 0x200
# Executable sections store 3 bytes per instruction (DS50002071F).
CODE = b"".join(w.to_bytes(3, "little") for w in WORDS)
CODE_OFF = 0x1000

ehdr = struct.pack(
    "<16sHHIIIIIHHHHHH",
    b"\x7fELF\x01\x01\x01" + b"\x00" * 9,
    2, EM_DSPIC30F, 1,
    CODE_PC,                 # e_entry: PC units
    52, 0, 0, 52, 32, 1, 0, 0, 0,
)
phdr = struct.pack(
    "<IIIIIIII",
    1,                       # PT_LOAD
    CODE_OFF,                # p_offset
    CODE_PC,                 # p_vaddr (PC units)
    CODE_PC,                 # p_paddr (PC units)
    len(CODE),               # p_filesz: 3 bytes per instruction
    len(CODE),               # p_memsz
    5,                       # PF_R | PF_X — an executable section
    2,                       # p_align: one PC unit
)

blob = bytearray(ehdr + phdr)
blob.extend(b"\x00" * (CODE_OFF - len(blob)))
blob.extend(CODE)

out = Path(__file__).parent / "dspic33fj32mc204-uart-smoke.elf"
out.write_bytes(blob)
print(
    f"wrote {out} ({len(blob)} bytes, {len(WORDS)} instructions, "
    f"entry 0x{CODE_PC:x}, {len(CODE)} code bytes)"
)
