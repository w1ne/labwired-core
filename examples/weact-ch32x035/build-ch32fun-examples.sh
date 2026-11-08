#!/usr/bin/env bash
# Build unmodified ch32fun CH32X035 examples into examples/weact-ch32x035/ch32fun/:
#
#   blink.elf               examples_x035/blink: PB12 (the WeAct board LED),
#                           PA2, PA4 and PA7 toggled every Delay_Ms(250).
#   debugprintfdemo.elf     examples_x035/debugprintfdemo: "+N" / "-N[c]"
#                           through the WCH-Link debug printf channel
#                           (DMDATA0/1 at 0xE0000380).
#   exti_pin_change_isr.elf examples_x035/exti_pin_change_isr: PC14 follows
#                           PC7 through the EXTI7_0 interrupt.
#
# The compile and link lines are the ones ch32fun.mk emits for
# TARGET_MCU=CH32X035 with the default TARGET_MCU_PACKAGE=CH32X035F8U6:
# rv32imac/ilp32, -DCH32X03x=1, MCU_PACKAGE=1, TARGET_MCU_LD=4 (62 KB flash
# at 0, 20 KB SRAM). No source depends on the package: the WeAct board's
# CH32X035F8U6 runs blink and debugprintfdemo, the LQFP48 CH32X035C8T6 the
# EXTI example (QFN20 does not bond PC7). ch32fun's misc/libgcc.a is the
# RV32E one ch32fun.mk links only for the CH32V00x; the toolchain's own
# rv32imac libgcc is used here. ch32fun is MIT licensed, so the ELFs are
# committed; LICENSE-ch32fun carries the notice.
#
# Toolchain: riscv64-unknown-elf-gcc 13.2 (Ubuntu gcc-riscv64-unknown-elf), as
# for examples/weact-bluepill-plus-ch32v203/build-ch32fun-examples.sh:
#   RISCV_PREFIX=~/tools/riscv64-deb/root/usr \
#   NEWLIB=~/tools/riscv64-deb/root/usr/lib/picolibc/riscv64-unknown-elf/include
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
example_root="${repo_root}/examples/weact-ch32x035"
out_dir="${example_root}/ch32fun"
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
"${prefix}gcc" -E -P -x c -DTARGET_MCU=CH32X035 -DMCU_PACKAGE=1 -DTARGET_MCU_LD=4 \
  "${src}/ch32fun/ch32fun.ld" > "${ld}"

build() {
  local name="$1" dir="${src}/examples_x035/$1"
  "${prefix}gcc" -o "${out_dir}/${name}.elf" \
    "${src}/ch32fun/ch32fun.c" "${dir}/${name}.c" \
    -g -Os -flto -ffunction-sections -fdata-sections -fmessage-length=0 \
    -msmall-data-limit=8 -fno-tree-loop-distribute-patterns \
    -march=rv32imac -mabi=ilp32 -DCH32X03x=1 -static-libgcc \
    -I"${newlib}" -I"${src}/extralibs" -I"${src}/ch32fun" -nostdlib \
    -I"${dir}" -Wall \
    -T "${ld}" -Wl,--gc-sections -lgcc \
    -ffile-prefix-map="${src}"=ch32fun -ffile-prefix-map="${repo_root}"=.
  "${prefix}strip" --strip-debug "${out_dir}/${name}.elf"
  chmod 0644 "${out_dir}/${name}.elf"
  "${prefix}size" "${out_dir}/${name}.elf"
}

build blink
build debugprintfdemo
build exti_pin_change_isr
