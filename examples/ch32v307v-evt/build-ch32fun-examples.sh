#!/usr/bin/env bash
# Build firmware for the WCH CH32V307V-EVT (CH32V307VCT6, QingKe V4F) into
# examples/ch32v307v-evt/ch32fun/:
#
#   blink.elf            ch32fun examples_v30x/blink, unmodified: PA3 toggled
#                        every Delay_Ms(1000) on the QingKe V4 64-bit SysTick.
#   debugprintfdemo.elf  ch32fun examples_v30x/debugprintfdemo, unmodified:
#                        "+N" / "-N[c]" through the WCH-Link debug printf
#                        channel (DMDATA0/1 at 0xE0000380).
#   fpu_selftest.elf     fpu-selftest/fpu_selftest.c on ch32fun: every RV32F
#                        instruction under RNE/RTZ/RDN/RUP with fflags, hashed
#                        per vector and printed on the debug printf channel.
#
# and fpu-selftest/expected.txt: the same fpu_selftest.c built for this host
# (x86-64 SSE + FMA3 through fenv.h) and run, which is the oracle the smoke
# compares against. It needs an x86-64 host with FMA.
#
# The compile and link lines are the ones ch32fun.mk emits for
# TARGET_MCU=CH32V307 with TARGET_MCU_PACKAGE=CH32V307VCT6: rv32imafc/ilp32f
# (ENABLE_FPU=1), -DCH32V30x_D8C, MCU_PACKAGE=1, TARGET_MCU_LD=3 and
# TARGET_MCU_MEMORY_SPLIT=3, which SystemInit writes to FLASH->OBR and the
# linker script maps as 288 KB flash + 32 KB SRAM. The examples' own Makefiles
# name the CH32V305RBT6; the CH32V307VCT6 is the part on the EVT, and neither
# source depends on the package. fpu_selftest adds -ffp-contract=off (on both
# builds) so the compiler-generated float code rounds the same way the host
# does. ch32fun is MIT licensed, so the ELFs are committed; LICENSE-ch32fun
# carries the notice.
#
# Toolchain: riscv64-unknown-elf-gcc 13.2 (Ubuntu gcc-riscv64-unknown-elf), as
# for examples/weact-bluepill-plus-ch32v203/build-ch32fun-examples.sh:
#   RISCV_PREFIX=~/tools/riscv64-deb/root/usr \
#   NEWLIB=~/tools/riscv64-deb/root/usr/lib/picolibc/riscv64-unknown-elf/include
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
example_root="${repo_root}/examples/ch32v307v-evt"
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
"${prefix}gcc" -E -P -x c -DTARGET_MCU=CH32V307 -DMCU_PACKAGE=1 -DTARGET_MCU_LD=3 \
  -DTARGET_MCU_MEMORY_SPLIT=3 "${src}/ch32fun/ch32fun.ld" > "${ld}"

build() {
  local name="$1" dir="$2"
  shift 2
  "${prefix}gcc" -o "${out_dir}/${name}.elf" \
    "${src}/ch32fun/ch32fun.c" "${dir}/${name}.c" \
    -g -Os -flto -ffunction-sections -fdata-sections -fmessage-length=0 \
    -msmall-data-limit=8 -fno-tree-loop-distribute-patterns \
    -march=rv32imafc -mabi=ilp32f -DCH32V30x=1 -DTARGET_MCU_MEMORY_SPLIT=3 \
    -DCH32V30x_D8C -static-libgcc "$@" \
    -I"${newlib}" -I"${src}/extralibs" -I"${src}/ch32fun" -nostdlib \
    -I"${dir}" -Wall \
    -L"${src}/misc" -lgcc -T "${ld}" -Wl,--gc-sections -lgcc \
    -ffile-prefix-map="${src}"=ch32fun -ffile-prefix-map="${repo_root}"=.
  "${prefix}strip" --strip-debug "${out_dir}/${name}.elf"
  chmod 0644 "${out_dir}/${name}.elf"
  "${prefix}size" "${out_dir}/${name}.elf"
}

build blink "${src}/examples_v30x/blink"
build debugprintfdemo "${src}/examples_v30x/debugprintfdemo"
build fpu_selftest "${example_root}/fpu-selftest" -ffp-contract=off

host_bin="$(mktemp)"
trap 'rm -f "${ld}" "${host_bin}"' EXIT
gcc -O1 -frounding-math -fsignaling-nans -mfma -fno-math-errno -ffp-contract=off \
  -o "${host_bin}" "${example_root}/fpu-selftest/fpu_selftest.c" -lm
"${host_bin}" > "${example_root}/fpu-selftest/expected.txt"
tail -n 6 "${example_root}/fpu-selftest/expected.txt"
