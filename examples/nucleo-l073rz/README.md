# NUCLEO-L073RZ (STM32L073RZ, Cortex-M0+)

> **Tier: hardware-validated (smoke).** The boot + USART2 path was verified
> against a physical NUCLEO-L073RZ over SWD (on-board ST-LINK V2, 2026-06-03):
> the device identity (`DEV=20086447`) was read off the real DBGMCU, the
> firmware's clock/GPIO/USART bring-up was confirmed by reading registers back
> over SWD, and the board's captured UART matches the simulator **byte-for-byte**
> (see [`captures/`](captures/)). Peripherals beyond the GPIO/USART smoke path
> (RCC clock tree, I2C/SPI/ADC) remain family-model approximations. See
> [`VALIDATION.md`](VALIDATION.md) for the full evidence and fidelity limits,
> and [`REQUIRED_DOCS.md`](REQUIRED_DOCS.md) for the sources.

## Quick start

```bash
# 1. Build the demo firmware (Cortex-M0+ → thumbv6m-none-eabi)
rustup target add thumbv6m-none-eabi   # one-time
cargo build --release -p firmware-l073-demo --target thumbv6m-none-eabi

# 2. Run it in the simulator
cargo run --release -p labwired-cli -- \
  --firmware target/thumbv6m-none-eabi/release/firmware-l073-demo \
  --system examples/nucleo-l073rz/system.yaml \
  --max-steps 200000
```

Expected UART output (USART2 → stdout):

```
L073-DEMO BOOT
DEV=20086447
LED ON
LED OFF
LED ON
LED OFF
LED ON
LED OFF
DONE
```

This is identical to the real board's UART (captured at 9600 8N1 over the
ST-LINK VCP — see [`captures/silicon-uart-boot.txt`](captures/silicon-uart-boot.txt)).
`DEV=20086447` is the firmware reading **DBGMCU_IDCODE** back from the
hardware oracle: REV_ID `0x2008`, DEV_ID `0x447` (STM32L0x3) — the exact value
read off the silicon. On the L0 this
register lives at `0x40015800` on the APB bus — **not** the `0xE0042000`
location used by Cortex-M3/M4 parts. Getting `0x447` back proves the M0+
debug-block placement in the chip yaml is correct.

## Board pinout (UM1724)

| Signal       | Pin   | Alt-fn | Wired to                              |
|--------------|-------|--------|---------------------------------------|
| USART2_TX    | PA2   | AF4    | ST-LINK V2-1 Virtual COM Port         |
| USART2_RX    | PA3   | AF4    | (same)                                |
| LD2 LED      | PA5   | output | green user LED, active high           |
| B1 button    | PC13  | input  | blue user button, active low          |
| SWDIO        | PA13  | SWD    | ST-LINK V2-1 (debug)                  |
| SWCLK        | PA14  | SWD    | ST-LINK V2-1 (debug)                  |

> **Note on "JTAG":** the Cortex-M0+ core has **no JTAG TAP** — the only
> debug transport is 2-wire **SWD**. The on-board ST-LINK/V2-1 exposes SWD
> (SWDIO=PA13, SWCLK=PA14) plus the USART2 Virtual COM Port. There is no
> boundary-scan / JTAG path on this part; references to "JTAG onboarding"
> map to SWD here.

## What differs from the L476 reference

The L0 is a smaller, lower-power part on a different core. The chip yaml
captures these L0-specific facts that an L4 copy-paste would get wrong:

| Item            | STM32L073 (this board)         | STM32L476 (reference)        |
|-----------------|--------------------------------|------------------------------|
| Core            | Cortex-M0+ (ARMv6-M, thumbv6m) | Cortex-M4F (ARMv7E-M)        |
| Flash / RAM     | 192 KB / 20 KB                 | 1 MB / 96 KB                 |
| GPIO bus base   | `0x50000000` (IOPORT)          | `0x48000000` (AHB2)          |
| DBGMCU          | `0x40015800` (APB)             | `0xE0042000` (M3/M4 region)  |
| DEV_ID          | `0x447`                        | `0x415`                      |
| USART2 alt-fn   | AF4                            | AF7                          |
| EXTI            | single-bank (`stm32f1` layout) | two-bank (`stm32l4`)         |
| Debug transport | SWD only                       | SWD + JTAG                   |

## What's modelled

See [`docs/boards/nucleo-l073rz.md`](../../docs/boards/nucleo-l073rz.md) for
the full fidelity table. In short: core + SysTick + GPIO + USART (smoke
path) are exercised; RCC/I2C/SPI/ADC/timers are present as register models
reused from the L4 family and are **not** L0-tuned; USB/LCD/COMP/crypto are
stubs.

## Hardware setup (real board)

Plug the NUCLEO-L073RZ into USB. The on-board ST-LINK/V2-1 enumerates as a
USB CDC Virtual COM Port (typically `/dev/ttyACM0`) and an SWD debug probe.

### Flashing with ST-LINK

```bash
arm-none-eabi-objcopy -O binary \
  target/thumbv6m-none-eabi/release/firmware-l073-demo firmware.bin
st-flash write firmware.bin 0x08000000
```

### Capturing UART output (115200 8N1)

```bash
stty -F /dev/ttyACM0 115200 cs8 -cstopb -parenb -ixon -ixoff -icanon -echo raw
cat /dev/ttyACM0
```

> The demo's USART2 BRR assumes a 16 MHz clock. After reset the L0 runs on
> the ~2.1 MHz MSI, so for an exact 115200 baud on real silicon you must
> first switch the system clock to HSI16 (or adjust BRR). The simulator
> ignores baud and emits each TDR byte directly, so it prints regardless.

## Debugging via the LabWired GDB stub

```bash
cargo run --release -p labwired-cli -- \
  --firmware target/thumbv6m-none-eabi/release/firmware-l073-demo \
  --system examples/nucleo-l073rz/system.yaml \
  --gdb 3333

# another shell
arm-none-eabi-gdb target/thumbv6m-none-eabi/release/firmware-l073-demo \
  -ex "target remote :3333"
```
