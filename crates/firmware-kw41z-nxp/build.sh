#!/usr/bin/env bash
# Build the MKW41Z4 (KW41Z) NXP-clock-bring-up fixture ELF.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"

CC=arm-none-eabi-gcc
OUT=build
ELF="$OUT/kw41z-nxp.elf"

CPUFLAGS="-mcpu=cortex-m0plus -mthumb -mfloat-abi=soft"
CFLAGS="$CPUFLAGS -ffunction-sections -fdata-sections -Os -g -std=gnu11 -Wall \
        -DCPU_MKW41Z512VHT4 -Ivendor -I."
# The vendor startup zeroes .bss and branches straight to main (no crt0/_start).
ASFLAGS="$CPUFLAGS -x assembler-with-cpp -D__STARTUP_CLEAR_BSS -D__START=main -Ivendor"
LDFLAGS="$CPUFLAGS -nostartfiles -Wl,--gc-sections -Wl,-Map=$OUT/kw41z-nxp.map \
         -T linker.ld --specs=nano.specs --specs=nosys.specs"

SRCS=(
    main.c
    vendor/clock_config.c
    vendor/system_MKW41Z4.c
    vendor/fsl_clock.c
    vendor/fsl_lpuart.c
    vendor/fsl_common.c
    vendor/fsl_common_arm.c
    vendor/fsl_smc.c
)
ASRC=vendor/startup_MKW41Z4.S

rm -rf "$OUT"
mkdir -p "$OUT"

OBJS=()
for s in "${SRCS[@]}"; do
    o="$OUT/$(basename "${s%.c}").o"
    echo "CC  $s"
    $CC $CFLAGS -c "$s" -o "$o"
    OBJS+=("$o")
done

echo "AS  $ASRC"
$CC $ASFLAGS -c "$ASRC" -o "$OUT/startup_MKW41Z4.o"
OBJS+=("$OUT/startup_MKW41Z4.o")

echo "LD  $ELF"
$CC "${OBJS[@]}" $LDFLAGS -o "$ELF"

arm-none-eabi-size "$ELF"
echo "Built $ELF"

# Publish the committed fixture consumed by the firmware_survival /
# kw41z_clock_boot tests. build/ is gitignored; this ELF is the source of truth.
FIXTURE="$HERE/../../tests/fixtures/kw41z-nxp.elf"
cp "$ELF" "$FIXTURE"
echo "Published $FIXTURE"
