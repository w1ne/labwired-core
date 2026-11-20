#!/usr/bin/env bash
# Rebuild examples/ek-rx261/firmware/ek-rx261-adc-ntc-diode.elf, the bare-metal
# S12AD proof firmware (adc-ntc-diode.c), with the Zephyr SDK 1.0.1 RX
# toolchain (GCC 14.3.0, see crates/firmware-ek-rx261-zephyr/build.sh for the
# download). CROSS_COMPILE defaults to ~/tools/rx-zephyr-elf-dl/rx-zephyr-elf/
# bin/rx-zephyr-elf-. The data section must stay empty: the start-up code
# copies nothing (the build fails if it is not).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CROSS_COMPILE="${CROSS_COMPILE:-$HOME/tools/rx-zephyr-elf-dl/rx-zephyr-elf/bin/rx-zephyr-elf-}"
OUT="$HERE/ek-rx261-adc-ntc-diode.elf"
"${CROSS_COMPILE}gcc" -Os -ffreestanding -nostdlib -nostartfiles \
  -ffunction-sections -Wl,--gc-sections -Wall -Wextra -Werror \
  -T "$HERE/link.ld" -o "$OUT" "$HERE/start.S" "$HERE/adc-ntc-diode.c"
data_size="$("${CROSS_COMPILE}size" -A "$OUT" | awk '$1 == ".data" { print $2 }')"
if [ "${data_size:-0}" != "0" ]; then
  echo "error: .data/.bss is ${data_size} bytes; the start-up code copies nothing" >&2
  exit 1
fi
"${CROSS_COMPILE}strip" --strip-debug "$OUT"
echo "built $OUT"
