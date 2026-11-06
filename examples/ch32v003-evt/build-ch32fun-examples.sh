#!/usr/bin/env bash
# Build three unmodified ch32fun (cnlohr/ch32fun, MIT) CH32V003 examples into
# examples/ch32v003-evt/ch32fun/:
#
#   blink.elf        examples/blink: PD0, PD4, PD6, PC0 toggled every
#                    Delay_Ms(250) at 48 MHz.
#   uartdemo.elf     examples/uartdemo: "Count: N" on USART1 (PD5, 115200)
#                    every 100 ms, PD0 toggling.
#   systick_irq.elf  examples/systick_irq: 1 kHz SysTick interrupt counting
#                    milliseconds, printed through the WCH-LinkE debug
#                    printf channel (ch32fun's default console).
#
# The compile and link lines are the ones ch32fun.mk emits for
# TARGET_MCU=CH32V003 (rv32ec/ilp32e, -Os -flto, generated_ch32v003.ld),
# with the toolchain prefix made explicit. ch32fun is MIT licensed, so unlike
# the WCH SDK images the ELFs are committed; LICENSE-ch32fun carries the
# notice.
#
# Toolchain: riscv64-unknown-elf-gcc 13.2 with newlib headers (Ubuntu
# gcc-riscv64-unknown-elf). Override the tool prefix with RISCV_PREFIX
# (default /usr) and the newlib include directory with NEWLIB.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${repo_root}/examples/ch32v003-evt/ch32fun"
mkdir -p "${out_dir}"

ch32fun_commit="b28d285460ce052a179dba8f4a22c6f73ff8e328"
src="${CH32FUN_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/labwired/ch32fun-${ch32fun_commit}}"
prefix="${RISCV_PREFIX:-/usr}/bin/riscv64-unknown-elf-"
newlib="${NEWLIB:-/usr/include/newlib}"

if [[ ! -f "${src}/ch32fun/ch32fun.c" ]]; then
  rm -rf "${src}"
  git init -q "${src}"
  git -C "${src}" fetch -q --depth 1 https://github.com/cnlohr/ch32fun.git "${ch32fun_commit}"
  git -C "${src}" checkout -q FETCH_HEAD
fi
cp "${src}/LICENSE" "${out_dir}/LICENSE-ch32fun"

ld="${out_dir}/generated_ch32v003.ld"
"${prefix}gcc" -E -P -x c -DTARGET_MCU=CH32V003 -DMCU_PACKAGE=1 -DTARGET_MCU_LD=0 \
  -DTARGET_MCU_MEMORY_SPLIT= "${src}/ch32fun/ch32fun.ld" > "${ld}"

for example in blink uartdemo systick_irq; do
  "${prefix}gcc" -o "${out_dir}/${example}.elf" \
    "${src}/ch32fun/ch32fun.c" "${src}/examples/${example}/${example}.c" \
    -g -Os -flto -ffunction-sections -fdata-sections -fmessage-length=0 \
    -msmall-data-limit=8 -fno-tree-loop-distribute-patterns \
    -march=rv32ec -mabi=ilp32e -DCH32V003 -static-libgcc \
    -I"${newlib}" -I"${src}/extralibs" -I"${src}/ch32fun" -nostdlib \
    -I"${src}/examples/${example}" -Wall \
    -L"${src}/misc" -lgcc -T "${ld}" -Wl,--gc-sections -lgcc \
    -ffile-prefix-map="${src}"=ch32fun
  "${prefix}strip" --strip-debug "${out_dir}/${example}.elf"
  chmod 0644 "${out_dir}/${example}.elf"
  "${prefix}size" "${out_dir}/${example}.elf"
done
rm -f "${ld}"
