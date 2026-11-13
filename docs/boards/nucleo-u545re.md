# NUCLEO-U545RE-Q (STM32U545RET6Q)

The **STM32U545RE** (Arm Cortex-M33 with FPU, up to 160 MHz, 512 KB flash in two
banks, 256 KB SRAM + 16 KB SRAM4, LQFP64) on the ST Nucleo-64 board **MB1841**.
It is the 64-pin sibling of the [STM32U575 / NUCLEO-U575ZI-Q](stm32u575.md): same
U5 IP family, smaller memory map, fewer peripherals, one user LED.

[![NUCLEO-U545RE-Q top illustration: ST-LINK USB-C at the top, user USB-C at the bottom, B1 USER and B2 RESET buttons, STM32U545 LQFP64 below centre, Arduino and Morpho headers](../assets/boards/nucleo-u545re/board.svg)](../assets/boards/nucleo-u545re/board.svg)

> **Fidelity: SIM-DERIVED.** There is no U545 bench part. Reset values and
> behaviour come from ST's U545 SVD and the U575-family models; nothing is a
> silicon capture. No Renode differential has been run: upstream Renode has
> no STM32U5 platform, and the third-party 60North `renode-stm32u5`
> platform (STM32U575, MIT) has not been compared against this model.

!!! note "Sources"
    ST's own documents (datasheet, RM0456, UM3062) could not be fetched when this
    port was written (st.com was unreachable from the build host). Every fact
    below is cited to an ST-published repository instead; see
    [`REQUIRED_DOCS.md`](../../examples/nucleo-u545re/REQUIRED_DOCS.md).

## Status at a glance

> **Live status:** the table is a hand-maintained snapshot. The authoritative views
> are the [chip conformance scoreboard](../coverage/chip-conformance.md) and
> [validation status](VALIDATION_STATUS.md).

| Aspect | Status |
|--------|--------|
| Chip yaml | [`configs/chips/stm32u545.yaml`](../../configs/chips/stm32u545.yaml) |
| System yaml | [`configs/systems/nucleo-u545re.yaml`](../../configs/systems/nucleo-u545re.yaml) |
| Example | [`examples/nucleo-u545re/`](../../examples/nucleo-u545re/) (`VALIDATION.md` = commands + evidence) |
| Reference firmware | bare-metal blinky (committed ELF), STM32CubeU5 HAL smoke (160 MHz PLL1) |
| Validation | machine-run pin tests, PR-gated survival, Cube HAL run, Arduino matrix L0-L8; **Zephyr not validated** (see `VALIDATION.md`) |
| Tier | **sim-validated** |
| Core | Arm Cortex-M33 (TrustZone present, factory-disabled) |

## Differences from the U575 chip descriptor

`stm32u545.yaml` is a self-contained derivative of `stm32u575.yaml`. A drift guard
(`crates/core/tests/u545_u575_drift_guard.rs`) checks every shared peripheral;
its explicit peripheral-field allow-list is `dbgmcu.config.idcode` and
`sysmem.config.flash_size_kb`. Top-level flash/RAM sizes and package pin maps
also differ, and USART2/GPIOF/GPIOI are absent on U545.

| Field | U545 | Source |
|-------|------|--------|
| Flash | 512 KiB, two 256 KiB banks | open pin data `STM32U545RETxQ.xml`, `stm32u545xx.h` (`FLASH_BANK_SIZE = FLASH_SIZE >> 1`) |
| SRAM | SRAM1 192 KiB + SRAM2 64 KiB (256 KiB contiguous at `0x20000000`), SRAM4 16 KiB at `0x28000000`, no SRAM3 | `stm32u545xx.h`, ST `STM32U545xx_FLASH.ld` |
| Removed peripherals | USART2, GPIOF, GPIOI | absent from the ST U545 SVD |
| DBGMCU IDCODE | `0x10026455` | ST U545 SVD |
| System-memory flash-size word | 512 KiB (U575: 2048 KiB) | U545/U575 chip descriptors; checked by the drift guard |

## Pins (NUCLEO-U545RE-Q)

| Board label | MCU pin | Notes |
|-------------|---------|-------|
| LD2 (green) | PA5 | the only user LED; Arduino D13 / SPI SCK; BSP `LED2` |
| B1 USER | PC13 | active high, internal pull-down; BSP `BUTTON_USER` |
| VCP TX / RX | PA9 / PA10 | USART1, AF7, 115200 8N1 (ST-LINK virtual COM port); BSP `COM1` |
| Arduino serial | PA2 / PA3 | LPUART1 (D1 / D0) |
| Arduino I2C | PB7 / PB6 | I2C1 SDA / SCL (D14 / D15) |
| Arduino SPI | PA5 / PA6 / PA7, CS PC9 | SPI1 SCK / MISO / MOSI (D13 / D12 / D11) |
| Arduino analog | A0 PA0, A1 PA1, A2 PA4, A3 PB0, A4 PC1, A5 PC0 | |
| Arduino digital | D2 PC8, D3 PB3, D4 PB5, D5 PB4, D6 PB10, D7 PA8, D8 PC7, D9 PC6, D10 PC9 | |
| USB / FDCAN | PA11 / PA12 | USB DM / DP; FDCAN1 RX / TX shares them |

[![NUCLEO-U545RE-Q pinout: Arduino header pin labels with MCU pins, LD2 PA5, B1 PC13, VCP USART1 PA9/PA10](../assets/boards/nucleo-u545re/pinout.svg)](../assets/boards/nucleo-u545re/pinout.svg)

Illustration positions are measured from ST's product photo (ST CAD files were not
reachable); see [`images/gen_images.py`](../../examples/nucleo-u545re/images/gen_images.py).

## Peripherals

The model set is the U575's (see [stm32u575.md](stm32u575.md) for the per-block
table), minus USART2/GPIOF/GPIOI. The shared peripheral descriptors are checked
against an explicit drift allow-list: DBGMCU IDCODE and the system-memory flash
size word differ by part (512 vs 2048 KiB). Top-level flash/RAM/pin maps also
reflect the smaller package and memory capacity. Declared: RCC, PWR, FLASH (two 256 KiB banks),
GPIOA-E/G/H, EXTI, USART1/3, UART4, LPUART1, I2C1-3, SPI1-3, TIM1/2/3/6/7, FDCAN1,
GPDMA1, ADC1, RTC, IWDG, CRC, CRS, RNG, ICACHE (limited register contract;
cache contents are not simulated), DBGMCU, SysTick, NVIC, and a 2 KiB STM32U5
system-memory window at `0x0BFA0000`. Its FLASHSIZE word is seeded read-only to
512 KiB for U545; OTP/package/UID bytes are simplified backing storage, not
factory identity behavior.
Not modeled: TrustZone/GTZC, USB FS, ADC4, DAC, OCTOSPI, SDMMC, AES/PKA/HASH.

## What this catches (and what it cannot)

Catches U5 boot-path regressions on a 64-pin memory map: PLL1 bring-up, clock
gating, flash bank geometry, USART1 VCP bytes, LD2 and B1 pin behaviour. It cannot
catch a divergence from silicon that the validating firmware also avoids; no
silicon-parity claim is made.

## How to run

```bash
# bare-metal blinky smoke (committed ELF)
labwired test --script examples/nucleo-u545re/io-smoke.yaml

# Cube HAL firmware (needs an STM32CubeU5 checkout, see EXTERNAL_COMPONENTS.md)
make -C examples/nucleo-u545re/board_firmware
labwired run --system configs/systems/nucleo-u545re.yaml \
  --firmware examples/nucleo-u545re/board_firmware/build/u545_hal_smoke.elf \
  --max-steps 20000000
# -> U545-HAL OK / BLINK 0 LD2=1 B1=0
```

## Related

- [`examples/nucleo-u545re/VALIDATION.md`](../../examples/nucleo-u545re/VALIDATION.md)
- [`crates/core/tests/nucleo_u545re.rs`](../../crates/core/tests/nucleo_u545re.rs) - pin proofs
- [`crates/core/tests/u545_u575_drift_guard.rs`](../../crates/core/tests/u545_u575_drift_guard.rs)
