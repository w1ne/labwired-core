#!/usr/bin/env python3
"""Generate the dsPIC33FJ32MC204 M0 smoke fixture ELF.

There is no open-source PIC24 assembler in CI, so the fixture is assembled by
hand from the encodings in DS70000157G (see docs/pic24-m0-reference.md):

    MOV  #'H', W0        ; 0020 0480
    MOV  W0, 0x0224      ; 8811 20   (U1TXREG)
    NOP x6
    BRA  $-1             ; 0037 FFFF (self-loop → run ends at max_steps)

The ELF is a minimal ELF32 little-endian (ET_EXEC) with e_machine = 118
(EM_DSPIC30F, binutils) and one PT_LOAD segment at paddr 0. Each 24-bit
instruction occupies one 4-byte little-endian slot — the provisional layout
documented in crates/core/src/cpu/pic24.rs (to be replaced by real XC16 ELF
mapping when a toolchain is available).

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
CODE = b"".join(w.to_bytes(4, "little")[:3] + b"\x00" for w in WORDS)
CODE_OFF = 0x1000

ehdr = struct.pack(
    "<16sHHIIIIIHHHHHH",
    b"\x7fELF\x01\x01\x01" + b"\x00" * 9,
    2, EM_DSPIC30F, 1,
    0, 52, 0, 0, 52, 32, 1, 0, 0, 0,
)
phdr = struct.pack(
    "<IIIIIIII",
    1,                       # PT_LOAD
    CODE_OFF,                # p_offset
    0,                       # p_vaddr
    0,                       # p_paddr
    len(CODE),               # p_filesz
    len(CODE),               # p_memsz
    5,                       # PF_R | PF_X
    4,                       # p_align
)

blob = bytearray(ehdr + phdr)
blob.extend(b"\x00" * (CODE_OFF - len(blob)))
blob.extend(CODE)

out = Path(__file__).parent / "dspic33fj32mc204-uart-smoke.elf"
out.write_bytes(blob)
print(f"wrote {out} ({len(blob)} bytes, {len(WORDS)} instructions)")
