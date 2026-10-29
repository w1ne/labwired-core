# NUCLEO-L552ZE-Q

The **NUCLEO-L552ZE-Q** with an **STM32L552ZET6Q** (Cortex-M33, Armv8-M,
110 MHz max): 512 KiB of flash at `0x08000000`, 192 KiB SRAM1 at `0x20000000`
plus 64 KiB SRAM2 at `0x20030000`, three user LEDs on `PC7` (LD1, green),
`PB7` (LD2, blue) and `PA9` (LD3, red), a user button on `PC13`, and the
ST-LINK/V3E Virtual COM Port on **LPUART1** `PG7`/`PG8` (AF8).

This is the **first STM32L5 part in the engine**. It is an **L1 (smoke)**
target: boot, the new L5 RCC clock gates, the LPUART1 console and the
GPIO LED/button path (with `ODR` read-back) are proven by a committed smoke
fixture and an `io-smoke.yaml`. There is no silicon capture: **SIM-DERIVED**,
no executing-fidelity differential.

!!! warning "Not claimed yet"
    TrustZone / SAU / GTZC and the secure/non-secure split, MPU/PMSAv8
    enforcement and `TT`/`TTA` are **not modelled**. The Cortex-M33 core stores
    the MPU registers but does not enforce them
    ([ISA coverage](../isa_coverage.md)). The L5 non-secure FLASH controller,
    EXTI, ADC, FDCAN and USB are declared stubs. Do not advertise the Zephyr
    protection/userspace test cases for this part.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/stm32l552.yaml`](../../configs/chips/stm32l552.yaml) |
| Example system | [`configs/systems/nucleo-l552ze-q.yaml`](../../configs/systems/nucleo-l552ze-q.yaml) |
| Reference firmware | [`crates/firmware-l552ze-smoke/`](../../crates/firmware-l552ze-smoke/) |
| Example | [`examples/nucleo-l552ze-q/`](../../examples/nucleo-l552ze-q/) |
| Committed artifact | `tests/fixtures/nucleo-l552ze-q-smoke.elf` (`NUCLEO-L552ZE-Q SMOKE`) |
| Build target | `thumbv8m.main-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + LPUART1 + GPIO); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| Flash | `0x0800_0000`, 512 KiB | modelled — the smoke links and runs here |
| SRAM1 / SRAM2 | `0x2000_0000` 192 KiB / `0x2003_0000` 64 KiB | modelled — data/stack |
| RCC | `0x4002_1000` | modelled (`stm32l5` profile) — AHB1ENR/AHB2ENR/AHB3ENR/APB1ENR1/APB1ENR2/APB2ENR, MSI boot, PLL handshake |
| LPUART1 (console) | `0x4000_8000` | modelled — USART v2 layout, `APB1ENR2` bit 0 |
| USART1–3 / UART4–5 | `0x4001_3800` / `0x4000_4400`… | modelled — USART v2 layout |
| GPIOA–H | `0x4202_0000` + `0x400`×n | modelled — `stm32v2` MODER/ODR/BSRR/AFR |
| LD1 / LD2 / LD3 | PC7 / PB7 / PA9 | modelled GPIO — active high |
| B1 user button | PC13 | modelled GPIO — active high |
| VCP console | PG7 (TX) / PG8 (RX), AF8 | simulator captures the LPUART1 stream; USB bridge not modelled |
| SPI1–3 | `0x4001_3000` / `0x4000_3800` / `0x4000_3C00` | modelled — SPI v2 FIFO, unexercised |
| I2C1–2 | `0x4000_5400` / `0x4000_5800` | modelled — v2 I2C, unexercised |
| TIM1–8, TIM15–17 | see descriptor | modelled — generic TIMx, unexercised |
| DMA1 / DMA2 / DMAMUX1 | `0x4002_0000` / `0x4002_0400` / `0x4002_0800` | modelled — unexercised |
| RNG / CRC / RTC / IWDG / WWDG / PWR | see descriptor | modelled — unexercised |
| FLASH controller, EXTI, ADC1, FDCAN1, USB | see descriptor | `type: stub` — zeros |
| TrustZone / SAU / GTZC / MPU / TT | — | **not modelled** |

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point
(vector table at the flash base, `0x0800_0000`) with SP seeded at the top of
SRAM1 (`0x2003_0000`). The STM32 system-memory bootloader and TrustZone
secure-boot path are not modelled.

## Validation

```bash
# from the repository root
cargo build -p firmware-l552ze-smoke --release --target thumbv8m.main-none-eabi
./target/release/labwired test --script examples/nucleo-l552ze-q/io-smoke.yaml --no-uart-stdout
```

See [`examples/nucleo-l552ze-q/VALIDATION.md`](../../examples/nucleo-l552ze-q/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

RM0432 (STM32L55x/L56x reference manual) for the RCC/GPIO/USART/SPI/I2C
register maps; DS12380 (STM32L552xx/L562xx datasheet) for the memory
boundaries and pin map; UM2623 (NUCLEO-144 user manual) and the Zephyr
`nucleo_l552ze_q` board dts for the LED/button/console wiring — detailed in
[`examples/nucleo-l552ze-q/REQUIRED_DOCS.md`](../../examples/nucleo-l552ze-q/REQUIRED_DOCS.md).
