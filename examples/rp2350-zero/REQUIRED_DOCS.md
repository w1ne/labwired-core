# Required Source Documents (Waveshare RP2350-Zero)

## MCU

1. RP2350 Datasheet (memory map, peripherals, dual M33):
   https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf
2. pico-sdk `hardware_regs` for RP2350 (moved APB map vs RP2040):
   https://github.com/raspberrypi/pico-sdk/tree/master/src/rp2350/hardware_regs
3. CHIP_ID = `0x30004927` (sysinfo.h / RP2350 datasheet SYSINFO).

## Board (this carrier — not Pico 2)

1. Waveshare RP2350-Zero product page:
   https://www.waveshare.com/rp2350-zero.htm
2. Waveshare wiki (UF2: BOOT then RESET; WS2812; dimensions 18.00 × 23.50 mm):
   https://www.waveshare.com/wiki/RP2350-Zero
3. Schematic `RP2350_Zero.pdf` (P1 23-pad header, P2/P3 solder GPIOs, WS2812 DIN = GPIO16):
   https://files.waveshare.com/wiki/RP2350-Zero/RP2350_Zero.pdf
4. pico-sdk board header `waveshare_rp2350_zero.h`:
   - `PICO_DEFAULT_UART` 0, TX GP0, RX GP1
   - `PICO_DEFAULT_WS2812_PIN` 16
   - `PICO_DEFAULT_I2C` 1, SDA GP6, SCL GP7
   - `PICO_DEFAULT_SPI` 1, SCK GP10, TX GP11, RX GP12, CSN GP13
   - `PICO_FLASH_SIZE_BYTES` 4 MiB
   - `PICO_RP2350A` 1
5. Zephyr `boards/waveshare/rp2350_zero` pinctrl (UART0 P0/P1, WS2812 PIO0_P16, ADC P26–P29).

## Live USB identity (this desk, 2026-09-03)

Not a source of pin facts. Records the native-USB path used while writing these docs:

- USB: Raspberry Pi product string `Pico`, VID `2e8a`, PID `0009`, serial `38EE7132FF1204D0`
- CDC: `/dev/cu.usbmodem11201`
- Firmware on the cable at capture: Waveshare factory GPIO/ADC self-test (CDC text). That stream is **not** a twin oracle.
