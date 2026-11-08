#!/usr/bin/env bash
# Build unmodified ch32fun CH32L103 examples into examples/ch32l103c8t6/ch32fun/:
#
#   blink.elf            examples_l103/blink: PA0, PA2, PA4 and PA7 toggled
#                        every Delay_Ms(250).
#   debugprintfdemo.elf  examples/debugprintfdemo: "+N" / "-N[c]" through the
#                        WCH-Link debug printf channel (DMDATA0/1 at
#                        0xE0000380).
#
# The compile and link lines are the ones ch32fun.mk emits for
# TARGET_MCU=CH32L103 with the default TARGET_MCU_PACKAGE=CH32L103C8T6:
# rv32imac/ilp32, -DCH32L103=1, MCU_PACKAGE=1, TARGET_MCU_LD=4 (a 64 KB
# flash region at 0, 20 KB SRAM; the part has 62 KB of code flash, which the
# chip profile models, and these images use under 3 KB). ch32fun's
# misc/libgcc.a is the RV32E one ch32fun.mk links only for the CH32V00x; the
# toolchain's own rv32imac libgcc is used here. ch32fun is MIT licensed, so
# the ELFs are committed; LICENSE-ch32fun carries the notice.
#
# Toolchain: riscv64-unknown-elf-gcc 13.2 (Ubuntu gcc-riscv64-unknown-elf), as
# for examples/weact-ch32x035/build-ch32fun-examples.sh:
#   RISCV_PREFIX=~/tools/riscv64-deb/root/usr \
#   NEWLIB=~/tools/riscv64-deb/root/usr/lib/picolibc/riscv64-unknown-elf/include
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${repo_root}/examples/ch32l103c8t6/ch32fun"
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
"${prefix}gcc" -E -P -x c -DTARGET_MCU=CH32L103 -DMCU_PACKAGE=1 -DTARGET_MCU_LD=4 \
  "${src}/ch32fun/ch32fun.ld" > "${ld}"

build() {
  local dir="${src}/$1" name
  name="$(basename "$1")"
  "${prefix}gcc" -o "${out_dir}/${name}.elf" \
    "${src}/ch32fun/ch32fun.c" "${dir}/${name}.c" \
    -g -Os -flto -ffunction-sections -fdata-sections -fmessage-length=0 \
    -msmall-data-limit=8 -fno-tree-loop-distribute-patterns \
    -march=rv32imac -mabi=ilp32 -DCH32L103=1 -static-libgcc \
    -I"${newlib}" -I"${src}/extralibs" -I"${src}/ch32fun" -nostdlib \
    -I"${dir}" -Wall \
    -T "${ld}" -Wl,--gc-sections -lgcc \
    -ffile-prefix-map="${src}"=ch32fun -ffile-prefix-map="${repo_root}"=.
  "${prefix}strip" --strip-debug "${out_dir}/${name}.elf"
  chmod 0644 "${out_dir}/${name}.elf"
  "${prefix}size" "${out_dir}/${name}.elf"
}

build examples_l103/blink
build examples/debugprintfdemo
