#!/usr/bin/env bash
# Build thirteen unmodified ch32fun (cnlohr/ch32fun, MIT) CH32V003 examples into
# examples/ch32v003-evt/ch32fun/:
#
#   blink.elf        examples/blink: PD0, PD4, PD6, PC0 toggled every
#                    Delay_Ms(250) at 48 MHz.
#   uartdemo.elf     examples/uartdemo: "Count: N" on USART1 (PD5, 115200)
#                    every 100 ms, PD0 toggling.
#   systick_irq.elf  examples/systick_irq: 1 kHz SysTick interrupt counting
#                    milliseconds, printed through the WCH-LinkE debug
#                    printf channel (ch32fun's default console).
#   adc_polled.elf   examples/adc_polled: software-started 10-bit ADC
#                    conversions of AIN7 (PD4) every 200 ms, printed as
#                    "Count: N adc: X" over the debug printf channel.
#   i2c_oled.elf     examples/i2c_oled: polled I2C1 master (PC1 SDA, PC2
#                    SCL, 1 MHz fast-mode setting) driving a 128x32 SSD1306
#                    at 0x3C through eight test patterns, 2 s apart.
#   tim2_pwm.elf     examples/tim2_pwm: TIM2 PWM mode 1, ARR 255, on CH1
#                    (PD4), CH2 (PD3) and CH4 (PD7); the duty steps by one
#                    count every Delay_Ms(5), CH2/CH4 180 degrees out of
#                    phase with CH1. CH1/CH2 are active low; CH4 stays
#                    active high (the example masks CC4P with 0xff).
#   exti_pin_change_isr.elf
#                    examples/exti_pin_change_isr: EXTI line 3 on PD3
#                    (AFIO_EXTICR = PD, rising edge, INTENR), the
#                    EXTI7_0 handler blipping PC1 high then low and
#                    clearing INTFR; main spins on nop.
#   spi_max7219.elf  examples/spi_max7219: 16-bit SPI1 frames (PC5 SCK,
#                    PC6 MOSI) to a MAX7219 with CS/LOAD on PD0.
#   uart_tx_dma.elf  examples/uart_tx_dma: "Hello World!\r\n" on USART1
#                    (PD5, 115200) moved by DMA1 channel 4 (CTLR3.DMAT)
#                    once per second; the channel's transfer-complete
#                    interrupt (PFIC 25) toggles PD6.
#   ws2812bdemo.elf  examples/ws2812bdemo: 191 WS2812/SK6805 LEDs (GRB) on
#                    PC6, driven by ch32fun's ws2812b_dma_spi_led_driver.h:
#                    16-bit SPI1 frames at 3 MHz (one LED bit = four SPI
#                    bits, 1000 or 1110) moved by DMA1 channel 3 in circular
#                    mode, its half/complete interrupts (PFIC 24) refilling
#                    a 48-halfword buffer from WS2812BLEDCallback().
#   tim1_pwm.elf     examples/tim1_pwm: TIM1 (advanced-control) PWM mode 1,
#                    ARR 255, MOE set, on the complementary output CH1N
#                    (PD0, CC1NE|CC1NP with CC1E clear) and on CH4 (PC4,
#                    CC4E|CC4P); CH1's compare steps one count every
#                    Delay_Ms(5), CH4's 128 counts ahead.
#   tim1_pwm_complementary_outputs.elf
#                    examples/tim1_pwm_complementary_outputs: TIM1 remapped
#                    by AFIO_PCFR1.TIM1_RM = 01 (partial remap 1), CH1 on
#                    PC6 and CH1N on PC3, PSC 9 / ATRLR 10000 (2.083 ms),
#                    50% duty, both outputs active low (CC1P|CC1NP); the
#                    main loop steps BDTR.DTG (dead time) 0..255 every
#                    Delay_Ms(20).
#   iwdg.elf         examples/iwdg: PD0, PD4 and PC0 on 1.5 s, off 1 s, then
#                    the IWDG started at PSCR /128, RLDR 0xFFF (~4.1 s at
#                    the 128 kHz LSI), on 3 s, one feed, then 250 ms blinks
#                    with no further feed until the watchdog resets the
#                    chip and the sequence starts again.
#
# The compile and link lines are the ones ch32fun.mk emits for
# TARGET_MCU=CH32V003 (rv32ec/ilp32e, -Os -flto, generated_ch32v003.ld),
# with the toolchain prefix made explicit. ch32fun is MIT licensed, so unlike
# the WCH SDK images the ELFs are committed; LICENSE-ch32fun carries the
# notice.
#
# Toolchain: riscv64-unknown-elf-gcc 13.2 with newlib headers (Ubuntu
# gcc-riscv64-unknown-elf). Override the tool prefix with RISCV_PREFIX
# (default /usr) and the newlib include directory with NEWLIB. The unpacked
# picolibc deb's include directory works too and gives the same bytes:
#   RISCV_PREFIX=~/tools/riscv64-deb/root/usr \
#   NEWLIB=~/tools/riscv64-deb/root/usr/lib/picolibc/riscv64-unknown-elf/include
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${repo_root}/examples/ch32v003-evt/ch32fun"
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

ld="${out_dir}/generated_ch32v003.ld"
"${prefix}gcc" -E -P -x c -DTARGET_MCU=CH32V003 -DMCU_PACKAGE=1 -DTARGET_MCU_LD=0 \
  -DTARGET_MCU_MEMORY_SPLIT= "${src}/ch32fun/ch32fun.ld" > "${ld}"

for example in blink uartdemo systick_irq adc_polled i2c_oled tim2_pwm exti_pin_change_isr \
    spi_max7219 uart_tx_dma ws2812bdemo tim1_pwm tim1_pwm_complementary_outputs iwdg; do
  "${prefix}gcc" -o "${out_dir}/${example}.elf" \
    "${src}/ch32fun/ch32fun.c" "${src}/examples/${example}/${example}.c" \
    -g -Os -flto -ffunction-sections -fdata-sections -fmessage-length=0 \
    -msmall-data-limit=8 -fno-tree-loop-distribute-patterns \
    -march=rv32ec -mabi=ilp32e -DCH32V003 -static-libgcc \
    -I"${newlib}" -I"${src}/extralibs" -I"${src}/ch32fun" -nostdlib \
    -I"${src}/examples/${example}" -Wall \
    -L"${src}/misc" -lgcc -T "${ld}" -Wl,--gc-sections -lgcc \
    -ffile-prefix-map="${src}"=ch32fun
  "${prefix}strip" --strip-debug "${out_dir}/${example}.elf"
  chmod 0644 "${out_dir}/${example}.elf"
  "${prefix}size" "${out_dir}/${example}.elf"
done
rm -f "${ld}"
