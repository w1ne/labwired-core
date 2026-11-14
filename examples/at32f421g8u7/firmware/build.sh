#!/usr/bin/env bash
set -euo pipefail
# Reuse a pinned SDK checkout; do not modify or download dependencies implicitly.
: "${AT32_SDK:?Set AT32_SDK to ArteryTek/AT32F421_Firmware_Library checkout}"
expected=2fd0bfbc812d7e4984203039cba838345d3058f1
[[ $(git -C "$AT32_SDK" rev-parse HEAD) == "$expected" ]] || { echo 'SDK commit mismatch' >&2; exit 1; }
# A matching HEAD alone does not prove that the source is unmodified.
git -C "${AT32_SDK}" diff --quiet --exit-code || { echo 'Source checkout has tracked edits' >&2; exit 1; }
git -C "${AT32_SDK}" diff --cached --quiet --exit-code || { echo 'Source checkout has staged edits' >&2; exit 1; }
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
output=${1:-"$here/../../../tests/fixtures/at32f421g8u7-smoke.elf"}
mkdir -p "$here/build"
export TMPDIR="$here/build"
arm-none-eabi-gcc -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -DAT32F421G8U7 -DUSE_STDPERIPH_DRIVER \
 -ffile-prefix-map="$here"=examples/at32f421g8u7/firmware -ffile-prefix-map="$AT32_SDK"=artery-sdk \
 -Os -g -ffreestanding -ffunction-sections -fdata-sections -Wall -Wextra -Werror -nostdlib \
 -I"$here" -I"$AT32_SDK/libraries/drivers/inc" -I"$AT32_SDK/libraries/cmsis/cm4/core_support" \
 -I"$AT32_SDK/libraries/cmsis/cm4/device_support" \
 "$here/smoke.c" "$AT32_SDK/libraries/cmsis/cm4/device_support/system_at32f421.c" \
 "$AT32_SDK/libraries/drivers/src/at32f421_crm.c" "$AT32_SDK/libraries/drivers/src/at32f421_gpio.c" \
 "$AT32_SDK/libraries/drivers/src/at32f421_usart.c" "$AT32_SDK/libraries/drivers/src/at32f421_tmr.c" \
 "$AT32_SDK/libraries/drivers/src/at32f421_flash.c" -T"$here/link.ld" -Wl,--gc-sections -lgcc -o "$output"
arm-none-eabi-size "$output"
sha256sum "$output"
