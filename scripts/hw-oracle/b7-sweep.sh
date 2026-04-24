#!/bin/bash
set -e
# Correct rri8_pack(op0,t,s,r,imm8) = op0 | (t<<4) | (s<<8) | (r<<12) | (imm8<<16)
cases=(
  "00101237|beq a2, a3, 0x14"
  "00059237|bne a2, a3, 0x9"
  "0005A237|bge a2, a3, 0x9"
  "00052237|blt a2, a3, 0x9"
  "00053237|bltu a2, a3, 0x9"
  "0005B237|bgeu a2, a3, 0x9"
  "00048237|bany a2, a3, 0x8"
  "00044237|ball a2, a3, 0x8"
  "00040237|bnone a2, a3, 0x8"
  "0004C237|bnall a2, a3, 0x8"
  "00045237|bbc a2, a3, 0x8"
  "0004D237|bbs a2, a3, 0x8"
  "00046277|bbci a2, 0x7, 0x8"
  "00047277|bbci a2, 0x17, 0x8"
  "0004E277|bbsi a2, 0x7, 0x8"
  "00010216|beqz a2, 0x14"
  "00010256|bnez a2, 0x14"
  "00010296|bltz a2, 0x14"
  "000102D6|bgez a2, 0x14"
  "00105226|beqi a2, 5, 0x14"
  "0010F2E6|bgei a2, 0x100, 0x14"
  "001052B6|bltui a2, 5, 0x14"
  "001002F6|bgeui a2, 0x8000, 0x14"
  "00000006|j 0x4"
  "00000406|j 0x14"
)
pass=0; fail=0
for case in "${cases[@]}"; do
  u32=${case%%|*}
  expected=${case##*|}
  b0=$(printf '%02x' $((0x$u32 & 0xFF)))
  b1=$(printf '%02x' $(((0x$u32 >> 8) & 0xFF)))
  b2=$(printf '%02x' $(((0x$u32 >> 16) & 0xFF)))
  printf "\\x$b0\\x$b1\\x$b2" > /tmp/b7-oracle/insn.bin
  actual=$(xtensa-esp-elf-objdump -D -b binary -mxtensa /tmp/b7-oracle/insn.bin 2>&1 \
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
