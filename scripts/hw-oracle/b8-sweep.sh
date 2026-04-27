#!/bin/bash
set -e
# B8 HW-oracle sweep: CALL/CALLX/RET/RETW/JX + exception returns.
#
# Encoding reference (all HW-verified via xtensa-esp-elf-objdump):
#
# CALLN format (op0=5): bits[5:4]=n, bits[23:6]=imm18 (signed word offset from PC+4).
#   CALL0 n=0, CALL4 n=1, CALL8 n=2, CALL12 n=3.
#
# ST0 format (op0=0, op1=0, op2=0): r at bits[15:12], s at bits[11:8], t at bits[7:4].
#   r=0, t=8       → RET          (s ignored)
#   r=0, t=9       → RETW         (s ignored)
#   r=0, s=<as>, t=0xA → JX as_
#   r=0, s=<as>, t=0xC → CALLX0 as_
#   r=0, s=<as>, t=0xD → CALLX4 as_
#   r=0, s=<as>, t=0xE → CALLX8 as_
#   r=0, s=<as>, t=0xF → CALLX12 as_
#   r=3, s=0,  t=0 → RFE
#   r=3, s=2,  t=0 → RFDE
#   r=3, s=4,  t=0 → RFWO
#   r=3, s=5,  t=0 → RFWU
#   r=3, s=<level>, t=1 → RFI level
#
# Vector format: "<24bit_hex_little_endian>|<expected_objdump_mnemonic_and_operands>"
# Note: offset shown by objdump is absolute (PC=0), so CALL imm18=0 → "0x4" (PC+4).
cases=(
  # CALL0/4/8/12 — representative imm18 values
  "000005|call0 0x4"       # CALL0  n=0, imm18=0     → offset=0 words → PC+4+0=4
  "000055|call4 0x8"       # CALL4  n=1, imm18=1     → offset=1 word  → PC+4+4=8
  "000425|call8 0x44"      # CALL8  n=2, imm18=0x10  → offset=16 words→ PC+4+64=0x44
  "fffff5|call12 0x0"      # CALL12 n=3, imm18=0x3FFFF (=-1) → PC+4-4=0

  # CALLX0/4/8/12 — as_=5
  "0005c0|callx0 a5"       # r=0, s=5, t=0xC
  "0005d0|callx4 a5"       # r=0, s=5, t=0xD
  "0005e0|callx8 a5"       # r=0, s=5, t=0xE
  "0005f0|callx12 a5"      # r=0, s=5, t=0xF

  # RET / RETW / JX
  "000080|ret "            # r=0, s=0, t=8
  "000090|retw "           # r=0, s=0, t=9
  "0004a0|jx a4"           # r=0, s=4, t=0xA

  # Exception / interrupt returns
  "003000|rfe "            # r=3, s=0, t=0
  "003200|rfde "           # r=3, s=2, t=0
  "003400|rfwo "           # r=3, s=4, t=0
  "003500|rfwu "           # r=3, s=5, t=0
  "003210|rfi 2"           # r=3, s=2, t=1 → RFI level=2
)

mkdir -p /tmp/b8-oracle
pass=0; fail=0
for case in "${cases[@]}"; do
  u32=${case%%|*}
  expected=${case##*|}
  b0=$(printf '%02x' $((0x$u32 & 0xFF)))
  b1=$(printf '%02x' $(((0x$u32 >> 8) & 0xFF)))
  b2=$(printf '%02x' $(((0x$u32 >> 16) & 0xFF)))
  printf "\\x$b0\\x$b1\\x$b2" > /tmp/b8-oracle/insn.bin
  actual=$(xtensa-esp-elf-objdump -D -b binary -mxtensa /tmp/b8-oracle/insn.bin 2>&1 \
    | awk -F'\t' '/^[ ]+0:/{print $3 " " $4}')
  if [[ "$actual" == "$expected" ]]; then
    printf "✓ %s → %s\n" "$u32" "$actual"
    pass=$((pass+1))
  else
    printf "✗ %s  want='%s'  got='%s'\n" "$u32" "$expected" "$actual"
    fail=$((fail+1))
  fi
done
echo
echo "Passed: $pass / $((pass+fail))"
