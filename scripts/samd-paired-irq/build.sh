#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Prospective harness: require an explicitly qualified immutable source pin.
set -euo pipefail
fixture_source=$1
fixture_output=$2
qualified_source=${LABWIRED_PAIRED_QUALIFIED_SOURCE:?Set the reviewed, qualified CODAL commit explicitly}
[[ "$qualified_source" =~ ^[0-9a-f]{40}$ ]]
test "$(git -C "$fixture_source" rev-parse HEAD)" = "$qualified_source"
test -z "$(git -C "$fixture_source" status --porcelain --untracked-files=all)"
script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
export LABWIRED_PAIRED_SOURCE_DIR="$fixture_source"
mkdir -p "$fixture_output"
python3 "$script_directory/record.py" --snapshot > "$fixture_output/source-before.json"
for mode in 0 1 2 3 4; do
  fixture_dir="$fixture_output/$mode"
  mkdir -p "$fixture_dir"
  arm-none-eabi-g++ -mcpu=cortex-m4 -mthumb -std=c++11 -O0 \
    -ffreestanding -fno-exceptions -fno-rtti -fno-unwind-tables \
    -fno-asynchronous-unwind-tables -Wall -Wextra -Werror -pedantic \
    -DBW_SIMULATION_STRICT_PAIRED_IRQ=1 \
    -I "$fixture_source/paired-irq/include" -I "$fixture_source/simulation/include" \
    -c "$fixture_source/paired-irq/profile.cpp" -o "$fixture_dir/profile.o"
  arm-none-eabi-g++ -mcpu=cortex-m4 -mthumb -std=c++11 -O0 \
    -ffreestanding -fno-exceptions -fno-rtti -fno-unwind-tables \
    -fno-asynchronous-unwind-tables -Wall -Wextra -Werror -pedantic \
    -DBW_PAIRED_IRQ_ADVERSARY="$mode" \
    -I "$fixture_source/simulation/include" \
    -c "$fixture_source/paired-irq/arm-probe.cpp" -o "$fixture_dir/probe.o"
  arm-none-eabi-gcc -mcpu=cortex-m4 -mthumb \
    -c "$fixture_source/simulation/arm/irq-startup.S" -o "$fixture_dir/startup.o"
  arm-none-eabi-g++ -mcpu=cortex-m4 -mthumb -nostdlib -nostartfiles \
    -Wl,--no-undefined,-Map="$fixture_dir/control.map" \
    -T "$fixture_source/simulation/arm/control.ld" \
    "$fixture_dir/startup.o" "$fixture_dir/profile.o" "$fixture_dir/probe.o" \
    -o "$fixture_dir/control.elf"
  test -z "$(arm-none-eabi-nm -u "$fixture_dir/control.elf")"
  if arm-none-eabi-readelf -SW "$fixture_dir/control.elf" | grep -Eq '\.(init_array|ctors)'; then
    echo 'Constructor section requires separately qualified startup support' >&2
    exit 1
  fi
  arm-none-eabi-nm -S "$fixture_dir/control.elf" > "$fixture_dir/symbols.txt"
  sha256sum "$fixture_dir/control.elf" "$fixture_dir/control.map"
done
python3 "$script_directory/record.py" --snapshot > "$fixture_output/source-after.json"
cmp "$fixture_output/source-before.json" "$fixture_output/source-after.json"
