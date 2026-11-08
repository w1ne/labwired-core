#!/usr/bin/env bash
# Build unmodified ch32fun CH32V006 examples into examples/ch32v006f8u7/ch32fun/:
#
#   blink.elf            examples_x00x/blink: PD0, PD4, PD6 and PC0 toggled
#                        every Delay_Ms(250).
#   dual_usart.elf       examples_x00x/dual_usart: a banner and "tick N" lines
#                        on USART1 (PD5 TX) and on USART2 (AFIO_PCFR1.USART2_RM
#                        = 3, PD2 TX), 115200 8N1.
#   debugprintfdemo.elf  examples/debugprintfdemo: "+N" / "-N[c]" through the
#                        WCH-Link debug printf channel (DMDATA0/1 at
#                        0xE00000F4).
#
# The compile and link lines are the ones ch32fun.mk emits for
# TARGET_MCU=CH32V006 with gcc >= 13: rv32ec_zmmul/ilp32e, -DCH32V00x=1
# -DCH32V006, MCU_PACKAGE=1, TARGET_MCU_LD=7 (62 KB flash at 0, 8 KB SRAM),
# linked against ch32fun's RV32E misc/libgcc.a as ch32fun.mk does for every
# CH32V00x. No source depends on the package. ch32fun is MIT licensed, so
# the ELFs are committed; LICENSE-ch32fun carries the notice.
#
# Toolchain: riscv64-unknown-elf-gcc 13.2 (Ubuntu gcc-riscv64-unknown-elf), as
# for examples/ch32v003-evt/build-ch32fun-examples.sh:
#   RISCV_PREFIX=~/tools/riscv64-deb/root/usr \
#   NEWLIB=~/tools/riscv64-deb/root/usr/lib/picolibc/riscv64-unknown-elf/include
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${repo_root}/examples/ch32v006f8u7/ch32fun"
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

ld="$(mktemp --suffix=.ld)"
trap 'rm -f "${ld}"' EXIT
"${prefix}gcc" -E -P -x c -DTARGET_MCU=CH32V006 -DMCU_PACKAGE=1 -DTARGET_MCU_LD=7 \
  "${src}/ch32fun/ch32fun.ld" > "${ld}"

build() {
  local dir="${src}/$1" name
  name="$(basename "$1")"
  "${prefix}gcc" -o "${out_dir}/${name}.elf" \
    "${src}/ch32fun/ch32fun.c" "${dir}/${name}.c" \
    -g -Os -flto -ffunction-sections -fdata-sections -fmessage-length=0 \
    -msmall-data-limit=8 -fno-tree-loop-distribute-patterns \
    -march=rv32ec_zmmul -mabi=ilp32e -DCH32V00x=1 -DCH32V006 -static-libgcc \
    -I"${newlib}" -I"${src}/extralibs" -I"${src}/ch32fun" -nostdlib \
    -I"${dir}" -Wall \
    -L"${src}/misc" -lgcc -T "${ld}" -Wl,--gc-sections -lgcc \
    -ffile-prefix-map="${src}"=ch32fun -ffile-prefix-map="${repo_root}"=.
  "${prefix}strip" --strip-debug "${out_dir}/${name}.elf"
  chmod 0644 "${out_dir}/${name}.elf"
  "${prefix}size" "${out_dir}/${name}.elf"
}

build examples_x00x/blink
build examples_x00x/dual_usart
build examples/debugprintfdemo
