# NUCLEO-L073RZ

[![tier: hardware-validated (smoke)](https://img.shields.io/badge/tier-hardware--validated%20(smoke)-brightgreen)](#)

The NUCLEO-L073RZ (STM32L073RZ, **Cortex-M0+**, 192 KB flash, 20 KB SRAM,
6 KB data EEPROM) is an ultra-low-power STM32L0 Nucleo-64 board. The boot +
USART2 smoke path is **validated against physical silicon** over SWD
(2026-06-03): device identity read off the real DBGMCU (`0x20086447`),
clock/GPIO/USART bring-up confirmed by SWD register reads, and the board's
captured UART matches the simulator byte-for-byte. Peripherals beyond the
GPIO/USART smoke path are register-map approximations not yet diffed against
silicon (see VALIDATION.md).

For build/run instructions see
[`examples/nucleo-l073rz/README.md`](../../examples/nucleo-l073rz/README.md);
for the validation evidence and the full list of fidelity limits see
[`examples/nucleo-l073rz/VALIDATION.md`](../../examples/nucleo-l073rz/VALIDATION.md).

## Status at a glance

| Aspect             | Status                                          |
|--------------------|-------------------------------------------------|
| Chip yaml          | `configs/chips/stm32l073.yaml`                  |
| System yaml        | `configs/systems/nucleo-l073rz.yaml`            |
| Reference firmware | `crates/firmware-l073-demo/` (thumbv6m)         |
| Survival tests     | 0 (UART parity captured; survival case TODO)    |
| Hardware-validated | **Yes (smoke)** — SWD identity + byte-for-byte UART vs silicon |
| Debug transport    | SWD only (Cortex-M0+ has no JTAG TAP)           |

## Peripherals

| Peripheral | Status        | Notes                                                       |
|------------|---------------|-------------------------------------------------------------|
| Cortex-M0+ | ⚠️ partial    | ARMv6-M **not** enforced by engine; toolchain (`thumbv6m`) is the ISA gate. Bit-band wrongly left enabled. |
| SysTick    | ✅            | system-exception timekeeping path                           |
| GPIO       | ✅            | A,B,C,D,E,H on the **IOPORT bus @0x50000000**; `Stm32V2` MODER/AFR layout |
| USART2     | ✅ silicon    | `Stm32V2` layout; TDR@0x28 byte stream matches real board byte-for-byte (AF4, 9600 8N1) |
| USART1/4/5, LPUART1 | ⚠️ present | mapped, not exercised                              |
| DBGMCU     | ✅ silicon    | IDCODE = `0x20086447` @ **0x40015800** (M0+ APB; read off real chip) |
| CRC        | ✅ silicon    | CRC-32 of fixed input matches real chip **byte-for-byte** (`B874177A`) |
| DMA1       | ✅ silicon    | mem-to-mem copy verified on silicon (CMAR→CPAR, TCIF)      |
| TIM21      | ✅ silicon    | free-running counter advances on both (behavioural)        |
| SPI1       | ✅ flag-level | TXE behaviour matches; data round-trip needs MOSI→MISO jumper |
| I2C1       | ⚠️ flag-level | sim+silicon agree (no NACK on bare bus); round-trip needs a slave |
| RCC        | ✅ silicon    | dedicated `stm32l0` layout; CFGR@0x0C, CR reset `0x300`, SW→SWS clock-switch readback matches silicon (`0x04`) |
| ADC1       | ⚠️ analog     | silicon converts VREFINT (`~0x86`); sim returns deterministic mock |
| RNG        | ⚠️ by design  | sim deterministic (`0xCAFEBABE`); real TRNG can't/shouldn't match |
| I2C2/3, SPI2 | ⚠️ present  | mapped, not exercised                                     |
| TIM2/3/6/7/22, LPTIM1 | ⚠️ present | TIM2 32-bit; others 16-bit; not exercised        |
| RTC/IWDG/WWDG/DAC | ⚠️ present | mapped, not exercised                              |
| EXTI       | ⚠️ approx     | single-bank `stm32f1` layout (L0 has <32 lines)             |
| USB FS, LCD, COMP, SYSCFG | ⛔ stub | reads 0, writes dropped — no fault                          |

Legend: ✅ silicon-validated · ⚠️ present/approximated/unvalidated · ⛔ stub.
Full per-peripheral evidence + the 3 documented divergences:
[`examples/nucleo-l073rz/VALIDATION.md`](../../examples/nucleo-l073rz/VALIDATION.md) §6.

## Pin map (UM1724)

| Signal     | Pin   | Alt-fn | Wired to                          |
|------------|-------|--------|-----------------------------------|
| USART2_TX  | PA2   | AF4    | ST-LINK V2-1 Virtual COM Port     |
| USART2_RX  | PA3   | AF4    | (same)                            |
| LD2 LED    | PA5   | output | green user LED, active high       |
| B1 button  | PC13  | input  | blue user button, active low      |
| SWDIO      | PA13  | SWD    | ST-LINK V2-1 debug                |
| SWCLK      | PA14  | SWD    | ST-LINK V2-1 debug                |

## How L073 differs from the L476 reference

| Item            | L073 (M0+)                | L476 (M4F)                |
|-----------------|---------------------------|---------------------------|
| GPIO bus        | `0x50000000` (IOPORT)     | `0x48000000` (AHB2)       |
| DBGMCU          | `0x40015800` (APB)        | `0xE0042000` (M3/M4)      |
| DEV_ID          | `0x447`                   | `0x415`                   |
| USART2 alt-fn   | AF4                       | AF7                       |
| EXTI            | single-bank (`stm32f1`)   | two-bank (`stm32l4`)      |
| Debug           | SWD only                  | SWD + JTAG                |
| Flash / RAM     | 192 KB / 20 KB            | 1 MB / 96 KB              |

## Run the demo

```bash
rustup target add thumbv6m-none-eabi
cargo build --release -p firmware-l073-demo --target thumbv6m-none-eabi
cargo run --release -p labwired-cli -- \
  --firmware target/thumbv6m-none-eabi/release/firmware-l073-demo \
  --system examples/nucleo-l073rz/system.yaml \
  --max-steps 200000
```
