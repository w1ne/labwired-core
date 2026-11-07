#!/usr/bin/env bash
# Build WCH's unmodified CH32V003 GPIO_Toggle example (openwch/ch32v003 EVT
# SDK, pinned commit) into target/wch-sdk/ch32v003f4p6-wch-sdk-gpio-toggle.elf.
#
# The SDK carries no open-source license (its headers restrict use to WCH
# microcontrollers), so neither its sources nor the built image are committed:
# the sources are fetched into a cache directory and the ELF goes under the
# git-ignored target/ tree. The only
# local addition is picolibc_stdout.c: the SDK's debug.c targets newlib and
# provides _write(); picolibc's printf writes through a `stdout` FILE, which
# the shim routes to that same _write().
#
# Toolchain: riscv64-unknown-elf-gcc 13.2 with picolibc (Ubuntu
# gcc-riscv64-unknown-elf + picolibc-riscv64-unknown-elf). Override the
# install prefix with RISCV_PREFIX (default /usr).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
example_dir="${repo_root}/examples/ch32v003-evt"
output="${repo_root}/target/wch-sdk/ch32v003f4p6-wch-sdk-gpio-toggle.elf"
mkdir -p "$(dirname "${output}")"

sdk_commit="4e1b7437ada5d3ae8831a6f4b811a6320e4e23c7"
sdk_dir="${WCH_SDK_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/labwired/openwch-ch32v003-${sdk_commit}}"
prefix="${RISCV_PREFIX:-/usr}"
gcc="${prefix}/bin/riscv64-unknown-elf-gcc"
picolibc="${prefix}/lib/picolibc/riscv64-unknown-elf"

base="https://raw.githubusercontent.com/openwch/ch32v003/${sdk_commit}/EVT/EXAM"
files=(
  GPIO/GPIO_Toggle/User/main.c
  GPIO/GPIO_Toggle/User/ch32v00x_conf.h
  GPIO/GPIO_Toggle/User/ch32v00x_it.c
  GPIO/GPIO_Toggle/User/ch32v00x_it.h
  GPIO/GPIO_Toggle/User/system_ch32v00x.c
  GPIO/GPIO_Toggle/User/system_ch32v00x.h
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

"${gcc}" \
  -march=rv32ec_zicsr_zifencei -mabi=ilp32e -Os \
  -ffunction-sections -fdata-sections -msmall-data-limit=0 -Wno-attributes \
  -nostdinc -isystem "${picolibc}/include" -isystem "$("${gcc}" -print-file-name=include)" \
  -I"${sdk_dir}/SRC/Core" -I"${sdk_dir}/SRC/Debug" \
  -I"${sdk_dir}/SRC/Peripheral/inc" -I"${sdk_dir}/GPIO/GPIO_Toggle/User" \
  "${sdk_dir}/SRC/Startup/startup_ch32v00x.S" \
  "${sdk_dir}"/GPIO/GPIO_Toggle/User/*.c \
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
