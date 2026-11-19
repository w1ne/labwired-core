#!/usr/bin/env bash
# Rebuild the MSP430 lockstep fixtures and their golden traces.
#
# Needs the TI MSP430 GCC (msp430-gcc 9.3.1.11) and its support files:
#   MSP430_GCC=<toolchain root>  MSP430_SUPPORT=<msp430-gcc-support-files/include>
# The golden traces come from the GDB simulator in the same toolchain
# (`msp430-elf-gdb`, `target sim`), one register dump per executed instruction.
set -euo pipefail
cd "$(dirname "$0")"
: "${MSP430_GCC:?set MSP430_GCC to the msp430-gcc root}"
: "${MSP430_SUPPORT:?set MSP430_SUPPORT to msp430-gcc-support-files/include}"
gcc="$MSP430_GCC/bin/msp430-elf-gcc"
flags=(-I"$MSP430_SUPPORT" -L"$MSP430_SUPPORT" -Os -ffunction-sections -Wl,--gc-sections)

"$gcc" -mmcu=msp430f149 "${flags[@]}" -o torture-f149.elf torture.c
"$gcc" -mmcu=msp430f2619 -msmall "${flags[@]}" -o torture-f2619-small.elf torture.c
"$gcc" -mmcu=msp430f2619 -mlarge "${flags[@]}" -o torture-f2619-large.elf torture.c
"$gcc" -mmcu=msp430f149 "${flags[@]}" -nostartfiles -o isa-f149.elf isa.S
"$gcc" -mmcu=msp430f2619 -mlarge "${flags[@]}" -nostartfiles -o isa-f2619.elf isa.S

for elf in torture-f149 torture-f2619-small torture-f2619-large isa-f149 isa-f2619; do
    "$MSP430_GCC/bin/msp430-elf-strip" --strip-debug "$elf.elf"
    python3 trace.py "$MSP430_GCC/bin/msp430-elf-gdb" "$elf.elf" "$elf.trace.z"
done
