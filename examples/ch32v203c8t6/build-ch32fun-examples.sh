#!/usr/bin/env bash
# Build unmodified ch32fun (cnlohr/ch32fun, MIT) examples for the
# CH32V203C8T6 into examples/ch32v203c8t6/ch32fun/:
#
#   blink.elf            examples_v20x/blink: PA0, PA2, PA4, PA7 toggled every
#                        Delay_Ms(250) at the 144 MHz SystemInit leaves (HSI
#                        x18 PLL), timed by the QingKe V4 64-bit SysTick.
#   debugprintfdemo.elf  examples_v20x/debugprintfdemo: "+N" / "-N[c]" lines
#                        through the WCH-LinkE debug printf channel (DMDATA0/1
#                        at 0xE0000380) every Delay_Ms(100).
#   exti_blink.elf       examples_v20x/exti_blink: EXTI1 (PA1, rising edge,
#                        PFIC interrupt 23) toggles PA10 in the handler.
#   systick_irq.elf      examples/systick_irq (the generic example, built with
#                        TARGET_MCU=CH32V203): SysTick_Handler through PFIC
#                        interrupt 12 once per millisecond, advancing the
#                        64-bit CMP; main prints the millisecond count.
#   spi_dma_tx.elf       local smoke fixture using ch32fun startup: SPI2
#                        16-bit master TX fed by DMA1 channel 5 (circular,
#                        half-word); the channel 5 handler (PFIC interrupt
#                        31) logs HT/TC for three laps, then USART1 prints it.
#   adc_irq.elf          local smoke fixture using ch32fun startup: ADC1
#                        IN0 and IN17 (VREFINT) through the end-of-conversion
#                        interrupt (PFIC interrupt 34), ADC2 IN1 polled;
#                        USART1 prints the three 12-bit codes.
#   i2c_bme280.elf       local smoke fixture using ch32fun startup: I2C1
#                        polled 1-byte and 3-byte reads of a BME280, and an
#                        I2C2 read stepped by the event interrupt (PFIC
#                        interrupt 49); USART1 prints the bytes.
#   tim_irq.elf          local smoke fixture using ch32fun startup: TIM2
#                        1 kHz update on PFIC interrupt 44 and TIM1 CH1
#                        compare on PFIC interrupt 43; USART1 prints the
#                        counts and the SysTick-measured 10 update periods.
#   usart_rx_irq.elf     local smoke fixture using ch32fun startup: USART1
#                        RXNE interrupt 53 captures injected "wch!", then
#                        USART1 TX prints the received bytes.
#
# The compile and link lines are the ones ch32fun.mk emits for
# TARGET_MCU=CH32V203 with TARGET_MCU_PACKAGE=CH32V203C8T6 (rv32imac/ilp32,
# -Os -flto, MCU_PACKAGE=1, TARGET_MCU_LD=2: 64 KB flash, 20 KB RAM), with the
# toolchain prefix made explicit. ch32fun.mk links its RV32E misc/libgcc.a
# for the CH32V00x only; the V20x gets the toolchain's rv32imac multilib.
# ch32fun is MIT licensed, so the ELFs are committed; LICENSE-ch32fun
# carries the notice.
#
# Toolchain: riscv64-unknown-elf-gcc 13.2 (Ubuntu gcc-riscv64-unknown-elf):
#   RISCV_PREFIX=~/tools/riscv64-deb/root/usr \
#   NEWLIB=~/tools/riscv64-deb/root/usr/lib/picolibc/riscv64-unknown-elf/include
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${repo_root}/examples/ch32v203c8t6/ch32fun"
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

ld="$(mktemp)"
trap 'rm -f "${ld}"' EXIT
"${prefix}gcc" -E -P -x c -DTARGET_MCU=CH32V203 -DMCU_PACKAGE=1 -DTARGET_MCU_LD=2 \
  -DTARGET_MCU_MEMORY_SPLIT= "${src}/ch32fun/ch32fun.ld" > "${ld}"

for example in examples_v20x/blink examples_v20x/debugprintfdemo examples_v20x/exti_blink \
  examples/systick_irq; do
  example_dir="${src}/${example}"
  name="$(basename "${example}")"
  "${prefix}gcc" -o "${out_dir}/${name}.elf" \
    "${src}/ch32fun/ch32fun.c" "${example_dir}/${name}.c" \
    -g -Os -flto -ffunction-sections -fdata-sections -fmessage-length=0 \
    -msmall-data-limit=8 -fno-tree-loop-distribute-patterns \
    -march=rv32imac -mabi=ilp32 -DCH32V20x=1 -static-libgcc \
    -I"${newlib}" -I"${src}/extralibs" -I"${src}/ch32fun" -nostdlib \
    -I"${example_dir}" -Wall \
    -lgcc -T "${ld}" -Wl,--gc-sections -lgcc \
    -ffile-prefix-map="${src}"=ch32fun
  "${prefix}strip" --strip-debug "${out_dir}/${name}.elf"
  chmod 0644 "${out_dir}/${name}.elf"
  "${prefix}size" "${out_dir}/${name}.elf"
done

for name in usart_rx_irq spi_dma_tx tim_irq adc_irq i2c_bme280; do
  "${prefix}gcc" -o "${out_dir}/${name}.elf" \
    "${src}/ch32fun/ch32fun.c" "${repo_root}/examples/ch32v203c8t6/${name}.c" \
    -g -Os -flto -ffunction-sections -fdata-sections -fmessage-length=0 \
    -msmall-data-limit=8 -fno-tree-loop-distribute-patterns \
    -march=rv32imac -mabi=ilp32 -DCH32V20x=1 -static-libgcc \
    -I"${newlib}" -I"${src}/extralibs" -I"${src}/ch32fun" \
    -I"${src}/examples_v20x/exti_blink" -nostdlib \
    -Wall -lgcc -T "${ld}" -Wl,--gc-sections -lgcc \
    -ffile-prefix-map="${src}"=ch32fun
  "${prefix}strip" --strip-debug "${out_dir}/${name}.elf"
  chmod 0644 "${out_dir}/${name}.elf"
  "${prefix}size" "${out_dir}/${name}.elf"
done
