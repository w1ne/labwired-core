#!/usr/bin/env bash
# Build the two CH32V003 PFIC interrupt images against WCH's EVT SDK
# (openwch/ch32v003, pinned commit) into target/wch-sdk/:
#
#   ch32v003f4p6-wch-sdk-systick-interrupt.elf
#       WCH's unmodified SYSTICK/SYSTICK_Interrupt example.
#   ch32v003f4p6-pfic-smoke.elf
#       firmware_pfic.c (this directory) as main, with the same example's
#       SystemInit, interrupt stubs, startup and SDK drivers.
#
# As with build-sdk-gpio-toggle.sh, the SDK carries no open-source license,
# so its sources are fetched into a cache directory and the ELFs stay under
# the git-ignored target/ tree.
#
# One compile flag stands in for WCH's MounRiver GCC: WCH marks handlers
# __attribute__((interrupt("WCH-Interrupt-fast"))), an attribute only WCH's
# GCC fork understands. Upstream GCC warns and drops the whole attribute,
# compiling the handler as a plain function that returns with `ret` — wrong
# on silicon too. -D'interrupt(x)=interrupt' turns it into the standard
# RISC-V machine interrupt attribute (software register save + mret), the
# form ch32v003fun uses. The sources are unchanged.
#
# Toolchain: riscv64-unknown-elf-gcc 13.2 with picolibc (Ubuntu
# gcc-riscv64-unknown-elf + picolibc-riscv64-unknown-elf). Override the
# install prefix with RISCV_PREFIX (default /usr).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
example_dir="${repo_root}/examples/ch32v003-evt"
out_dir="${repo_root}/target/wch-sdk"
mkdir -p "${out_dir}"

sdk_commit="4e1b7437ada5d3ae8831a6f4b811a6320e4e23c7"
sdk_dir="${WCH_SDK_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/labwired/openwch-ch32v003-${sdk_commit}}"
prefix="${RISCV_PREFIX:-/usr}"
gcc="${prefix}/bin/riscv64-unknown-elf-gcc"
picolibc="${prefix}/lib/picolibc/riscv64-unknown-elf"

user=SYSTICK/SYSTICK_Interrupt/User
base="https://raw.githubusercontent.com/openwch/ch32v003/${sdk_commit}/EVT/EXAM"
files=(
  "${user}/main.c"
  "${user}/ch32v00x_conf.h"
  "${user}/ch32v00x_it.c"
  "${user}/ch32v00x_it.h"
  "${user}/system_ch32v00x.c"
  "${user}/system_ch32v00x.h"
  SRC/Startup/startup_ch32v00x.S
  SRC/Ld/Link.ld
  SRC/Core/core_riscv.c
  SRC/Core/core_riscv.h
  SRC/Debug/debug.c
  SRC/Debug/debug.h
)
for name in adc dbgmcu dma exti flash gpio i2c iwdg misc opa pwr rcc spi tim usart wwdg; do
  files+=("SRC/Peripheral/inc/ch32v00x_${name}.h" "SRC/Peripheral/src/ch32v00x_${name}.c")
done
files+=(SRC/Peripheral/inc/ch32v00x.h)

for f in "${files[@]}"; do
  if [[ ! -f "${sdk_dir}/${f}" ]]; then
    mkdir -p "$(dirname "${sdk_dir}/${f}")"
    curl -sfL "${base}/${f}" -o "${sdk_dir}/${f}"
  fi
done

build() {
  local main="$1" output="$2"
  "${gcc}" \
    -march=rv32ec_zicsr_zifencei -mabi=ilp32e -Os \
    -ffunction-sections -fdata-sections -msmall-data-limit=0 -Wno-attributes \
    -D'interrupt(x)=interrupt' \
    -nostdinc -isystem "${picolibc}/include" -isystem "$("${gcc}" -print-file-name=include)" \
    -I"${sdk_dir}/SRC/Core" -I"${sdk_dir}/SRC/Debug" \
    -I"${sdk_dir}/SRC/Peripheral/inc" -I"${sdk_dir}/${user}" \
    "${sdk_dir}/SRC/Startup/startup_ch32v00x.S" \
    "${main}" \
    "${sdk_dir}/${user}/ch32v00x_it.c" "${sdk_dir}/${user}/system_ch32v00x.c" \
    "${sdk_dir}/SRC/Core/core_riscv.c" "${sdk_dir}/SRC/Debug/debug.c" \
    "${sdk_dir}"/SRC/Peripheral/src/*.c \
    "${example_dir}/picolibc_stdout.c" \
    -nostartfiles -nostdlib -T "${sdk_dir}/SRC/Ld/Link.ld" \
    -Wl,--gc-sections -Wl,--defsym=vfprintf=__i_vfprintf \
    -Wl,--build-id=none \
    "${picolibc}/lib/rv32e/ilp32e/libc.a" \
    "$("${gcc}" -march=rv32e -mabi=ilp32e -print-libgcc-file-name)" \
    -o "${output}"
  "${prefix}/bin/riscv64-unknown-elf-size" "${output}"
}

build "${sdk_dir}/${user}/main.c" "${out_dir}/ch32v003f4p6-wch-sdk-systick-interrupt.elf"
build "${example_dir}/firmware_pfic.c" "${out_dir}/ch32v003f4p6-pfic-smoke.elf"
