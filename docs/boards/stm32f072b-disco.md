# STM32F072B-DISCO

The **STM32F072B-DISCO** (ST order code STM32F072B-DISCO): an **STM32F072RBT6** (Cortex-M0, 48 MHz), 128 KiB of flash at `0x08000000`, 16 KiB of SRAM at `0x20000000`, four user LEDs on `PC6` (LD3, red), `PC8` (LD4, yellow), `PC9` (LD5, green) and `PC7` (LD6, blue), a user button on `PA0` (active low), and the ST-LINK/V2-1 virtual COM port on **USART1** (`PA9`/`PA10`, AF1).

This is the **first STM32F0 part in the engine**. It is an **L1 (smoke)** target: boot, the F0 clock gates (`AHBENR`/`APB2ENR`), the USART1 console and the four-LED GPIO walk (with `GPIOC_ODR` readback) are proven by a committed smoke fixture and an `io-smoke.yaml`. No silicon capture exists: **SIM-DERIVED**, no executing-fidelity differential.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/stm32f072.yaml`](../../configs/chips/stm32f072.yaml) |
| Example system | [`configs/systems/stm32f072b-disco.yaml`](../../configs/systems/stm32f072b-disco.yaml) |
| Reference firmware | [`crates/firmware-stm32f072b-disco-smoke/`](../../crates/firmware-stm32f072b-disco-smoke/) |
| Example | [`examples/stm32f072b-disco/`](../../examples/stm32f072b-disco/) |
| Committed artifact | `tests/fixtures/stm32f072b-disco-smoke.elf` (`STM32F072B-DISCO SMOKE`) |
| Build target | `thumbv6m-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + USART1 + GPIO); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| Flash | `0x0800_0000`, 128 KiB | modelled — the smoke links and runs here |
| SRAM | `0x2000_0000`, 16 KiB | modelled — data/stack |
| RCC (F0 layout) | `0x4002_1000` | modelled — `stm32f0` profile (RM0360), incl. `AHBENR`/`APB2ENR`/`CFGR3` |
| GPIOA–F | `0x4800_0000` + 0x400 steps | modelled — `stm32v2` (MODER/OTYPER/OSPEEDR/PUPDR/IDR/ODR/BSRR/AFRx), `AHBENR` gates |
| USART1–4 | `0x4001_3800` / `0x4000_4400` / `0x4000_4800` / `0x4000_4C00` | modelled — `stm32v2` (ISR/TDR), `APB2ENR`/`APB1ENR` gates |
| SPI1/2, I2C1/2 | `0x4001_3000` / `0x4000_3800`, `0x4000_5400` / `0x4000_5800` | modelled — legacy SPI and legacy I2C |
| TIM1/2/3/14/15/16/17 | see descriptor | modelled — generic timer |
| DMA1, EXTI, SYSCFG, PWR, CRC | see descriptor | modelled |
| bxCAN1, DAC1, RTC, IWDG/WWDG | see descriptor | modelled |
| ADC1 | `0x4001_2400` | modelled — `stm32l4`-family ADC profile (unexercised) |
| USB FS, TSC, CRS | `0x4000_5C00` / `0x4002_4000` / `0x4000_6C00` | `type: stub` — zeros |
| LD3/LD4/LD5/LD6 | PC6 / PC8 / PC9 / PC7 | modelled GPIO — active high |
| User button B1 | PA0 | modelled GPIO — active low (internal pull-up) |
| USART1 VCP | PA9 / PA10 (AF1) | modelled |

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point
(the flash base, `0x0800_0000`) with SP seeded at the top of SRAM. The
STM32F0 bootloader/DFU path is not modelled.

## Validation

```bash
cd core
cargo build -p firmware-stm32f072b-disco-smoke --release --target thumbv6m-none-eabi
./target/release/labwired test --script examples/stm32f072b-disco/io-smoke.yaml --no-uart-stdout
```

See [`examples/stm32f072b-disco/VALIDATION.md`](../../examples/stm32f072b-disco/VALIDATION.md) for the claims, runtime evidence and explicit non-claims.

## Sources

RM0360 (STM32F0x1/2/8 reference manual) and DS9826 (STM32F072xB datasheet)
for the chip, and the Zephyr `stm32f072b_disco` board dts for the board
wiring — detailed in
[`examples/stm32f072b-disco/REQUIRED_DOCS.md`](../../examples/stm32f072b-disco/REQUIRED_DOCS.md).
