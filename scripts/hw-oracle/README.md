# HW oracle spot-check scripts

Each decoder task dumps a handful of representative encoded instruction bytes
through `xtensa-esp-elf-objdump` (Espressif's binutils disassembler) and
diffs the mnemonic + operands against our decoder's `Instruction` variant.

## Prerequisites
- `espup install` has run (Xtensa binutils on PATH via `. ~/export-esp.sh`)
- Running in an environment with the esp-rs toolchain sourced.

## Usage
```
. /home/andrii/export-esp.sh
./scripts/hw-oracle/b7-sweep.sh
```

Expected: 22+/25 ✓ (3 rows may show ✗ for decimal-vs-hex formatting of
small immediates — not a correctness issue; inspect manually if unclear).

## Adding a new family sweep
Create `scripts/hw-oracle/bN-sweep.sh` with cases in the form:
```
"<u32_hex_big_endian>|<expected_objdump_mnemonic_and_operands>"
```
`<u32_hex_big_endian>` is the full 32-bit instruction value. The script
emits low 3 bytes little-endian into a temp `.bin` and disassembles.

## Rationale (digital-twin requirement)
Cross-referencing our decoder against an authoritative external disassembler
catches encoding bugs that prose-only ISA-RM verification misses (e.g., the
B7 pre-rework bugs that would have produced wrong offsets on every real
program).
