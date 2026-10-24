#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail
fixture_source=$1
fixture_output=$2
mkdir -p "$fixture_output"
for fixture_case in positive restore-enabled; do
  fixture_dir="$fixture_output/$fixture_case"
  mkdir -p "$fixture_dir/include"
  cp "$fixture_source/simulation/include/InterruptMask.h" "$fixture_dir/include/"
  if [ "$fixture_case" = restore-enabled ]; then
    python3 - "$fixture_dir/include/InterruptMask.h" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1])
s = p.read_text()
original = '__asm__ volatile("msr primask, %0" : : "r"(saved) : "memory");'
assert s.count(original) == 1
p.write_text(s.replace(original, '(void)saved;\n        __asm__ volatile("msr primask, %0" : : "r"(0u) : "memory");'))
PY
  fi
  arm-none-eabi-g++ -mcpu=cortex-m4 -mthumb -std=c++11 -O0 \
    -ffreestanding -fno-exceptions -fno-rtti -fno-unwind-tables \
    -fno-asynchronous-unwind-tables -Wall -Wextra -Werror -pedantic \
    -I "$fixture_dir/include" -c "$fixture_source/simulation/arm/interrupt-mask-probe.cpp" \
    -o "$fixture_dir/probe.o"
  arm-none-eabi-gcc -mcpu=cortex-m4 -mthumb \
    -Dtransfer_state_probe=interrupt_mask_probe \
    -c "$fixture_source/simulation/arm/startup.S" -o "$fixture_dir/startup.o"
  arm-none-eabi-g++ -mcpu=cortex-m4 -mthumb -nostdlib -nostartfiles \
    -Wl,--no-undefined,-Map="$fixture_dir/control.map" \
    -T "$fixture_source/simulation/arm/control.ld" \
    "$fixture_dir/startup.o" "$fixture_dir/probe.o" -o "$fixture_dir/control.elf"
  test -z "$(arm-none-eabi-nm -u "$fixture_dir/control.elf")"
  arm-none-eabi-size "$fixture_dir/control.elf"
  arm-none-eabi-nm "$fixture_dir/control.elf" > "$fixture_dir/symbols.txt"
  sha256sum "$fixture_dir/control.elf" "$fixture_dir/control.map" "$fixture_dir/include/InterruptMask.h"
done
