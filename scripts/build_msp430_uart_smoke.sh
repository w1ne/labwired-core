#!/usr/bin/env bash
# Build the MSP430 onboarding smoke for each imported MSP430 part.
#
# Needs the TI MSP430 GCC (msp430-gcc 9.3.1.11) and its support files:
#   MSP430_GCC=<toolchain root>  MSP430_SUPPORT=<msp430-gcc-support-files/include>
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${MSP430_GCC:?set MSP430_GCC to the msp430-gcc root}"
: "${MSP430_SUPPORT:?set MSP430_SUPPORT to msp430-gcc-support-files/include}"
source_file="${repo_root}/examples/renode-onboarding/msp430-uart-smoke.c"

build() {
    local mcu="$1" part="$2" model="$3"
    local output_file="${repo_root}/tests/fixtures/${mcu}-uart-smoke.elf"
    "${MSP430_GCC}/bin/msp430-elf-gcc" \
        -mmcu="${mcu}" ${model} \
        -I"${MSP430_SUPPORT}" -L"${MSP430_SUPPORT}" \
        -Os -Wall -Wextra -Werror \
        -ffunction-sections -fdata-sections -Wl,--gc-sections \
        -DPART="\"${part}\"" \
        -o "${output_file}" "${source_file}"
    "${MSP430_GCC}/bin/msp430-elf-strip" --strip-debug "${output_file}"
    chmod 0644 "${output_file}"
}

build msp430f149 MSP430F149 ""
build msp430f1611 MSP430F1611 ""
build msp430f2617 MSP430F2617 "-mlarge"
build msp430f2619 MSP430F2619 "-mlarge"
build msp430f5437 MSP430F5437 "-mlarge"
