# NUCLEO-L476RG (STM32L476RG, Cortex-M4F)

> Hardware-validated reference board. Every peripheral here has been
> exercised against real silicon (NUCLEO-L476RG with J-Link OB, UART
> capture on `/dev/ttyACM1`) and the simulator reproduces the byte
> stream verbatim.

## Quick start

```bash
# 1. Build the demo firmware (cross-compiles to thumbv7em-none-eabihf)
cargo build --release -p firmware-l476-demo --target thumbv7em-none-eabihf

# 2. Run it in the simulator
cargo run --release -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabihf/release/firmware-l476-demo \
  --system examples/nucleo-l476rg/system.yaml \
  --max-steps 2000000
```

Expected output:

```
L476-DEMO BOOT
DEV=10076415
SPI1 OK
I2C1 OK
ADC1 OK
DMA1 OK
LED ON
LED OFF
BTN=1
DONE
```

The same firmware, flashed to a real NUCLEO-L476RG with `JLinkExe`, emits
the identical byte stream over USART2 → ST-LINK / J-Link OB Virtual COM
Port. Drift between the two breaks `test_nucleo_l476rg_demo_survival` in
`crates/core/tests/firmware_survival.rs`.

## Board pinout (UM1724)

| Signal       | Pin   | Alt-fn | Wired to                              |
|--------------|-------|--------|---------------------------------------|
| USART2_TX    | PA2   | AF7    | ST-LINK / J-Link OB Virtual COM Port  |
| USART2_RX    | PA3   | AF7    | (same)                                |
| LD2 LED      | PA5   | output | green user LED, active high           |
| B1 button    | PC13  | input  | blue user button, active low          |
| SPI1_SCK     | PA5   | AF5    | (shared with LD2 — careful)           |
| SPI1_MISO    | PA6   | AF5    | header CN5 D12                        |
| SPI1_MOSI    | PA7   | AF5    | header CN5 D11                        |
| I2C1_SCL     | PB6   | AF4    | header CN10 D5                        |
| I2C1_SDA     | PB7   | AF4    | header CN7 D7                         |
| ADC1_IN5     | PA0   | analog | header CN8 A0                         |

## What's modelled

| Peripheral | Status      | Hardware-validated against           |
|------------|-------------|--------------------------------------|
| Cortex-M4F core | ✅ full | full Thumb-2 + VFPv4 single-precision |
| SysTick    | ✅ full     | system-exception path                |
| RCC        | ✅ L4 layout| AHB1ENR/AHB2ENR/APB1ENR1/APB2ENR     |
| GPIO (A,B,C,D,E,H) | ✅ V2 layout | MODER/AFR latching                |
| USART2     | ✅ V2 layout| 115200 8N1 byte-for-byte             |
| SPI1/2/3   | ✅ register-fidelity | CR1/CR2/SR latching, no-loopback semantics |
| I2C1/2/3   | ✅ L4 layout| TIMINGR/ISR/ICR/RXDR/TXDR            |
| ADC1       | ✅ L4 layout| DEEPPWD/ADVREGEN/ADCAL semantics     |
| DMA1/2     | ✅ full     | mem-to-mem CMAR→CPAR, GIF/HTIF/TCIF  |
| DBGMCU     | ✅ IDCODE   | configurable per chip yaml           |
| NVIC       | ✅ ISER/ISPR routing | low IRQs route correctly through NVIC |

## What's NOT modelled

These exist in real silicon but the simulator doesn't ship a full
register model. Firmware that touches them will read zeros and writes
will be silently dropped — _not_ a fault.

- USB (OTG_FS)
- CAN
- LCD-TFT, DMA2D
- TSC (touch-sensing controller)
- LPUART, LPTIM (low-power timers/UART)
- RTC, DAC
- HASH, AES (crypto blocks)
- Comparators, op-amps
- Quad-SPI / OctoSPI
- Independent / window watchdog

## Hardware setup

Plug the NUCLEO-L476RG into a host USB port. The on-board ST-LINK V2-1
shows up as either an ST-LINK (USB ID `0483:374b`) or a SEGGER J-Link
(USB ID `1366:0105`) depending on which firmware is flashed onto the
debug MCU. Either is fine — the demo build is identical, only the
flash command differs.

### Flashing with J-Link OB

```bash
arm-none-eabi-objcopy -O ihex \
  target/thumbv7em-none-eabihf/release/firmware-l476-demo \
  firmware.hex

cat > flash.jlink <<EOF
halt
erase
loadfile firmware.hex
r
g
qc
EOF

JLinkExe -NoGui 1 -AutoConnect 1 -Device STM32L476RG -If SWD -Speed 4000 \
  -CommanderScript flash.jlink
```

### Flashing with ST-LINK V2-1

```bash
arm-none-eabi-objcopy -O binary \
  target/thumbv7em-none-eabihf/release/firmware-l476-demo \
  firmware.bin

st-flash write firmware.bin 0x08000000
```

### Capturing UART output

The Virtual COM Port appears as `/dev/ttyACM1` (or `/dev/ttyACM0` if
no other ACM device is present). At 115200 8N1:

```bash
stty -F /dev/ttyACM1 115200 cs8 -cstopb -parenb -ixon -ixoff \
  -icanon -echo raw
cat /dev/ttyACM1
```

## Debugging via GDB

The simulator ships a GDB stub. Launch with `--gdb 3333`, then attach:

```bash
cargo run --release -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabihf/release/firmware-l476-demo \
  --system examples/nucleo-l476rg/system.yaml \
  --gdb 3333

# In another shell
arm-none-eabi-gdb target/thumbv7em-none-eabihf/release/firmware-l476-demo \
  -ex "target remote :3333"
```

## How the survival tests work

`crates/core/tests/firmware_survival.rs` runs each of the L476 fixtures
through the simulator and asserts on the byte-for-byte UART output. The
`expected_uart_output` field for each case is the raw capture from real
silicon. CI runs this on every commit; any drift in the L4 chip config,
peripheral models, or Thumb-2 decoder breaks the build.

Five register-level traces (`smoke`, `spi`, `i2c`, `adc`, `dma`) plus
the comprehensive `demo` cover the full peripheral surface.
