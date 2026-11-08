# RAMN (Toyota Resistant Automotive Miniature Network) imported profile

## Measured profile

LabWired's onboarding profile for the imported Renode `ramn` target models
one RAMN ECU, narrowed to what the engine executes and the smoke exercises.
It follows Renode's
[`boards/ramn.repl`](https://github.com/renode/renode/blob/master/platforms/boards/ramn.repl),
which includes
[`cpus/stm32l552.repl`](https://github.com/renode/renode/blob/master/platforms/cpus/stm32l552.repl),
and the RAMN firmware's own bring-up
([`firmware/RAMNV1/Core/Src/main.c`](https://github.com/ToyotaInfoTech/RAMN/blob/main/firmware/RAMNV1/Core/Src/main.c),
`stm32l5xx_hal_msp.c`):

- **CPU:** Cortex-M33 (STM32L552).
- **Memory:** 512 KiB flash at `0x08000000` and 256 KiB SRAM at `0x20000000`
  (SRAM1 + SRAM2 as one contiguous region, as in the RAMN memory layout).
- **RCC** at `0x40021000` on the `stm32l5` profile (RM0438 map, MSI 4 MHz
  reset clock).
- **LPUART1** at `0x40008000` on the `stm32v2` USART profile. This is the
  RAMN console: PA2/PA3 AF8, 115200 8N1, FIFO off. Gated by
  `RCC_APB1ENR2.LPUART1EN`.
- **GPIOA / GPIOB** at `0x42020000` / `0x42020400` on the `stm32v2` GPIO
  profile, with the per-port MODER/OSPEEDR/PUPDR reset values from the repl
  (`0xABFFFFFF` / `0xFFFFFEBF` MODER). The smoke uses PA9 (`HW_TYPE`, input
  with pull-up) and PB6 (`FDCAN1_STB`, push-pull output), as the RAMN
  firmware configures them. Gated by `RCC_AHB2ENR`.

The descriptor uses the same entries as `configs/chips/stm32l552.yaml`.

Not claimed and not mapped: FDCAN1, ADC1, SPI1-3, I2C1, USART1-3, UART4/5,
GPIOC-H, DMA/DMAMUX, EXTI, RTC, timers, IWDG, RNG, USB, the flash controller,
PWR and the OTP/unique-ID window (Renode fakes PWR and OTP with Python
peripherals). Board wiring (LCD, ECU enables, BOOT0 lines) and the four-ECU
CAN network are not claimed either.

The earlier report on this page described a 2026-10-18 run of a generic
`uart-ok-thumbv7m.elf` fixture that only showed the CPU stepping, and listed
the Renode peripherals as coverage. It never touched a RAMN peripheral and is
withdrawn.

## Smoke evidence

The firmware crate `crates/firmware-ramn-smoke` (`thumbv8m.main-none-eabi`)
enables the GPIOA/GPIOB and LPUART1 clocks, sets PA2/PA3 to AF8 and PA9 to an
input with pull-up, programs LPUART1 `BRR` = 8888 (115200 from MSI 4 MHz) and
`CR1` = UE | RE | TE, drives PB6 through `BSRR`, prints its readbacks polled
on `ISR.TXE`, waits for `ISR.TC`, then toggles PB6 as a heartbeat.

It ran **400,000 instructions** and passed 6/6 checks: the banner
`RAMN LPUART1 SMOKE`, the ordered readback lines (`brr=000022b8`,
PA `MODER` `0xABF3FFAF` / `AFRL` `0x00008800`, `hw_type=1`, PB6 set 1 /
clear 0, `pass=3f`), at least 3 PB6 edges, the MUL word `0x143` at
`0x20001000`, the result word `0x5A4E003F` at `0x20001004`, and `max_steps`.
The idle `ISR` reads `0x006000C0` (REACK | TEACK | TXE | TC), as RM0438
describes.

With LPUART1 removed from the descriptor, the same firmware takes a precise
BusFault (BFAR `0x4000800C`) on its first LPUART1 access and passes 2/6
(`docs/coverage/renode-onboarding/attempt-history/ramn/2026-11-08-lpuart1-negative-control/`).

The exact system manifest is `configs/systems/onboarding/ramn.yaml` and the
smoke script is `examples/renode-onboarding/ramn-smoke.yaml`. Per-target
metrics, JUnit, UART output, simulator result, snapshot and logs are under
`docs/coverage/renode-onboarding/metrics/ramn/`.

This is local CLI simulation evidence for the smoke assertions. It does not
show whole-SoC coverage, Renode equivalence or hardware behaviour. See the
[generated scoreboard](../coverage/renode-onboarding-scoreboard.md) for the
current global totals.
