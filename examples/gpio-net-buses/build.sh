#!/usr/bin/env bash
# Rebuild the example's firmware with clang + lld (no vendor SDK). The ELFs are
# committed so the tests run without a toolchain.
set -euo pipefail
cd "$(dirname "$0")"
CC=${CC:-clang}
COMMON=(-mthumb -Os -ffreestanding -fno-builtin -nostdlib -Wall -Wextra -fuse-ld=lld
        -Xlinker --build-id=none -Xlinker -z -Xlinker max-page-size=4096)
G0=(--target=thumbv6m-none-eabi -mcpu=cortex-m0plus "${COMMON[@]}" -T src/g0.ld)
for fw in spi_master spi_slave spi_dma_master spi_dma_slave i2c_controller i2c_target \
          i2c_bulk_controller i2c_bulk_target; do
    "$CC" "${G0[@]}" "src/$fw.c" -o "firmware/${fw//_/-}.elf"
done
# A target too slow for the clock: the controller must be held by the stretch.
"$CC" "${G0[@]}" -DSLOW=1 src/i2c_target.c -o firmware/i2c-target-slow.elf
# SPI frame shapes: modes 1-3, LSB-first and CR2.DS frame sizes (16, 12 and
# 4 bits), master and slave alike.
for variant in "mode1 -DCPHA=1" "mode2 -DCPOL=1" "mode3 -DCPOL=1 -DCPHA=1" "lsb -DLSBFIRST=1" \
               "ds16 -DDS=16" "ds12 -DDS=12" "ds4 -DDS=4"; do
    read -r name defs <<<"$variant"
    read -ra defs <<<"$defs"
    "$CC" "${G0[@]}" "${defs[@]}" src/spi_master.c -o "firmware/spi-master-$name.elf"
    "$CC" "${G0[@]}" "${defs[@]}" src/spi_slave.c -o "firmware/spi-slave-$name.elf"
done
# An 8-bit master that leaves CR2.FRXTH at reset: on the G0's FIFO SPI
# RXNE then waits for 16 bits, so it stalls after its first frame.
"$CC" "${G0[@]}" -DFRXTH=0 src/spi_master.c -o firmware/spi-master-nofrxth.elf
"$CC" "${G0[@]}" -DTXE_IRQ=1 src/spi_slave.c -o firmware/spi-slave-txe.elf
"$CC" "${G0[@]}" -DUNDERRUN=1 src/spi_slave.c -o firmware/spi-slave-underrun.elf
# STM32F401 (Cortex-M4): 16-bit SPI frames.
F4=(--target=thumbv7em-none-eabi -mcpu=cortex-m4 "${COMMON[@]}" -T src/g0.ld)
for fw in f4_spi16_master f4_spi16_slave; do
    "$CC" "${F4[@]}" "src/$fw.c" -o "firmware/${fw//_/-}.elf"
    # Hardware NSS: the master's SSOE drives it, the slave (SSM=0) reads it.
    "$CC" "${F4[@]}" -DHW_NSS=1 "src/$fw.c" -o "firmware/${fw//_/-}-nss.elf"
done
# STM32F401 DMA2: stream priority (two memory-to-memory streams), then SPI1
# both ways through the stream FIFOs with half-word INCR4 bursts, against the
# G071 DMA slave.
"$CC" "${F4[@]}" src/f4_spi_dma_master.c -o firmware/f4-spi-dma-master.elf
# Legacy I2C (CR1/SR1/SR2/CCR) on the STM32F103 (Cortex-M3) and STM32F401.
F1=(--target=thumbv7m-none-eabi -mcpu=cortex-m3 "${COMMON[@]}" -T src/f1.ld)
for fw in legacy_i2c_controller legacy_i2c_target; do
    "$CC" "${F1[@]}" -DCHIP_F1 "src/$fw.c" -o "firmware/f1-${fw//_/-}.elf"
    "$CC" "${F4[@]}" -DCHIP_F4 "src/$fw.c" -o "firmware/f4-${fw//_/-}.elf"
done
# A slave too slow for the clock: the master must be held by the stretch.
"$CC" "${F4[@]}" -DCHIP_F4 -DSLOW=1 src/legacy_i2c_target.c -o firmware/f4-legacy-i2c-target-slow.elf
# RP2040 (PL022 SPI0, DW_apb_i2c I2C0), against the G071.
RP=(--target=thumbv6m-none-eabi -mcpu=cortex-m0plus "${COMMON[@]}" -T src/rp2040.ld)
for fw in rp2040_spi_master rp2040_spi_slave rp2040_i2c_controller rp2040_i2c_target; do
    "$CC" "${RP[@]}" "src/$fw.c" -o "firmware/${fw//_/-}.elf"
done
"$CC" "${RP[@]}" -DSLOW=1 src/rp2040_i2c_target.c -o firmware/rp2040-i2c-target-slow.elf
# nRF52840 (Cortex-M4, serial instance 0: SPIM0 / SPIS0 / TWIM0 / TWIS0),
# against the G071.
NRF=(--target=thumbv7em-none-eabi -mcpu=cortex-m4 "${COMMON[@]}" -T src/nrf52.ld)
for fw in nrf_spim_master nrf_spis_slave nrf_twim_controller nrf_twis_target; do
    "$CC" "${NRF[@]}" "src/$fw.c" -o "firmware/${fw//_/-}.elf"
done
"$CC" "${NRF[@]}" -DSLOW=1 src/nrf_twis_target.c -o firmware/nrf-twis-target-slow.elf
# nRF52840 SPIM3 with its hardware chip select and D/CX, CSN active low and
# (-DCSNPOL=1) active high.
"$CC" "${NRF[@]}" src/nrf_spim3_master.c -o firmware/nrf-spim3-master.elf
"$CC" "${NRF[@]}" -DCSNPOL=1 src/nrf_spim3_master.c -o firmware/nrf-spim3-master-csnpol.elf
# STM32L476 and STM32G474 (Cortex-M4, FIFO SPI, same SPI1/GPIOA/RCC
# addresses): SPI1 hardware NSS, the master's SSOE driving it (-DNSSP=1:
# pulse mode), the slave (SSM=0) reading it. One image runs on both parts.
L4=(--target=thumbv7em-none-eabi -mcpu=cortex-m4 "${COMMON[@]}" -T src/g0.ld)
"$CC" "${L4[@]}" src/l4_spi_nss_master.c -o firmware/l4-spi-nss-master.elf
"$CC" "${L4[@]}" -DNSSP=1 src/l4_spi_nss_master.c -o firmware/l4-spi-nss-master-nssp.elf
"$CC" "${L4[@]}" src/l4_spi_nss_slave.c -o firmware/l4-spi-nss-slave.elf
# ESP32-C3 and ESP32-C6 (GP-SPI2, I2C0) against the G071, register level, no
# IDF: the same sources for both chips (-DCHIP_C6 picks the C6's bases and
# GPIO-matrix numbers). The esp-hal 1.x masters live in esp32c3-esp-hal/
# (README there); their ELFs are committed beside these.
for chip in c3 c6; do
    if [ "$chip" = c6 ]; then arch=rv32imac; defs=(-DCHIP_C6); else arch=rv32imc; defs=(); fi
    ESP=(--target=riscv32-unknown-elf "-march=$arch" -mabi=ilp32 -Os -ffreestanding -fno-builtin -nostdlib
         -Wall -Wextra -fuse-ld=lld -Xlinker --build-id=none -Xlinker --no-relax -T src/esp32c3.ld)
    for fw in spi_master spi_slave spi_dma_master spi_dma_slave i2c_controller i2c_target; do
        "$CC" "${ESP[@]}" "${defs[@]}" "src/esp32c_$fw.c" -o "firmware/esp32${chip}-${fw//_/-}.elf"
    done
    "$CC" "${ESP[@]}" "${defs[@]}" -DSLOW=1 src/esp32c_i2c_target.c -o "firmware/esp32${chip}-i2c-target-slow.elf"
done
# ESP32-S3 (GP-SPI2, I2C0) and classic ESP32 (VSPI master, HSPI slave, I2C0)
# against the G071, register level, no IDF. LLVM 18 assembles Xtensa but
# neither compiles C for it nor links it, so these are hand-written assembly
# (src/esp32x.inc, one source per role, --defsym CHIP_ESP32=1 for the classic
# part): llvm-mc makes a one-section object with no relocations and
# src/xtensa_elf.py places it at IRAM, where the bare-ELF fast boot enters.
XT_OBJ=$(mktemp -d)
trap 'rm -rf "$XT_OBJ"' EXIT
for chip in esp32s3 esp32; do
    if [ "$chip" = esp32 ]; then base=0x40080000; defs=(--defsym CHIP_ESP32=1); else base=0x40378000; defs=(); fi
    for variant in "spi_master" "spi_slave" "i2c_controller" "i2c_target" "i2c_target_slow --defsym SLOW=1"; do
        read -r fw extra <<<"$variant"
        read -ra extra <<<"$extra"
        src=${fw%_slow}
        [ "$chip" = esp32 ] && [ "$fw" = i2c_target_slow ] && continue
        llvm-mc -triple=xtensa -filetype=obj -I src "${defs[@]}" "${extra[@]}" "src/esp32x_$src.S" -o "$XT_OBJ/$fw.o"
        python3 src/xtensa_elf.py "$XT_OBJ/$fw.o" "firmware/${chip}-${fw//_/-}.elf" "$base"
    done
done
# The second controller of a kind (--defsym INST1=1) on bank-1 pads
# (--defsym PADS_HI=1, GPIO32 and up): S3 GP-SPI3 and I2C1, classic I2C1; and
# the classic VSPI master / HSPI slave on bank-1 pads (the slave's SCK, MOSI
# and CS0 on the input-only GPIO34..36). Plus plain GPIO on bank-1 pads
# between a classic ESP32 and an ESP32-S3 (src/esp32x_gpio_hi.S), and RTC
# GPIO on pads the RTC IO MUX owns, with a hand-off back to the digital side
# (src/esp32x_rtc_gpio.S).
for build in "esp32s3 spi3-master spi_master INST1 PADS_HI" \
             "esp32s3 spi3-slave spi_slave INST1 PADS_HI" \
             "esp32s3 i2c1-controller i2c_controller INST1 PADS_HI" \
             "esp32s3 i2c1-target i2c_target INST1 PADS_HI" \
             "esp32s3 i2c1-target-slow i2c_target INST1 PADS_HI SLOW" \
             "esp32 i2c1-controller i2c_controller INST1 PADS_HI" \
             "esp32 i2c1-target i2c_target INST1 PADS_HI" \
             "esp32 spi-hi-master spi_master PADS_HI" \
             "esp32 spi-hi-slave spi_slave PADS_HI" \
             "esp32 gpio-hi gpio_hi" \
             "esp32s3 gpio-hi gpio_hi" \
             "esp32 rtc-gpio rtc_gpio" \
             "esp32s3 rtc-gpio rtc_gpio"; do
    read -r chip out src syms <<<"$build"
    read -ra syms <<<"$syms"
    if [ "$chip" = esp32 ]; then base=0x40080000; defs=(--defsym CHIP_ESP32=1); else base=0x40378000; defs=(); fi
    for sym in "${syms[@]}"; do defs+=(--defsym "$sym=1"); done
    llvm-mc -triple=xtensa -filetype=obj -I src "${defs[@]}" "src/esp32x_$src.S" -o "$XT_OBJ/$out.o"
    python3 src/xtensa_elf.py "$XT_OBJ/$out.o" "firmware/${chip}-${out}.elf" "$base"
done
