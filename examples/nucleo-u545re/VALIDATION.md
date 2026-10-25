# NUCLEO-U545RE-Q Validation Runbook

Run from the repository root. Evidence captured 2026-10-03, branch
`feat/stm32u545-nucleo`.

Tier: **sim-validated**. SVD-derived; no bench part, no silicon capture, no
Renode differential. Upstream Renode has no built-in STM32U5 platform; the
separate 60North reference platform is not used for a differential here.

## Toolchain coverage

| Stack | Status | Evidence |
|-------|--------|----------|
| Bare-metal C | validated | sections A, B |
| STM32Cube HAL (CubeU5 `12d19a5`) | validated | section C |
| Arduino (stm32duino core 3.0.0, generic U545RETxQ variant) | validated, L0-L8 9/9 | section E |
| **Zephyr** | **NOT validated** | section F |

## A. io-smoke (committed ELF)

```bash
cargo run -q -p labwired-cli -- test --script examples/nucleo-u545re/io-smoke.yaml --no-uart-stdout
# PASS  2/2 checks · io-smoke · 20000 steps · 0.03s
```

ELF: `tests/fixtures/nucleo-u545re-blinky.elf` (arm-none-eabi-gcc 13.2.1,
`make -C examples/nucleo-u545re/firmware`).

## B. The twin drives the board's pins

`cargo test -p labwired-core --test nucleo_u545re` (8 pass, 1 ignored):

| Test | Proves |
|------|--------|
| `blinky_prints_ok_on_usart1_vcp` | USART1 TX (VCP, PA9) delivers `OK` |
| `blinky_configures_the_board_pins_and_toggles_ld2_on_pa5` | GPIOA ODR bit 5 toggles (>= 3 edges); PA5 MODER=output, PA9/PA10 MODER=AF with AFRH=AF7, PC13 input with pull-down |
| `button_pc13_press_and_release_reach_the_vcp` | driving PC13 high/low is read by firmware and reported `B1=1` / `B1=0` |
| `vcp_rx_pa10_is_echoed_back_on_tx_pa9` | a byte injected on USART1 RX is received and echoed on TX |
| `memory_map_is_the_u545_one` | 256 KiB SRAM at `0x20000000` (no SRAM3), SRAM4 16 KiB, flash ends at 512 KiB |
| `peripherals_the_u545_lacks_are_unmapped` | USART2, GPIOF, GPIOI are bus errors |
| `dbgmcu_idcode_is_the_u545_one` | IDCODE `0x10026455` |
| `flash_banks_are_256k_so_bker_erase_hits_the_second_half` | page erase with BKER lands at +256 KiB, bank 1 untouched |
| `cube_hal_firmware_reaches_160mhz_blinks_ld2_and_reports_b1` (`--ignored`, ~140 s) | HAL toggle of PA5 and `HAL_GPIO_ReadPin(PC13)` with B1 held |

Plus PR-gated `firmware_survival::test_stm32u545_blinky_survival`,
`u545_u575_drift_guard` (3 tests), `chip_conformance`, `svd_conformance`,
`register_coverage`, and the existing `u575_flash` tests (1 MiB banks unchanged).

## C. STM32CubeU5 HAL firmware

```bash
make -C examples/nucleo-u545re/board_firmware
RUST_LOG=off target/debug/labwired --firmware examples/nucleo-u545re/board_firmware/build/u545_hal_smoke.elf \
  --system examples/nucleo-u545re/system.yaml --max-steps 50000000
```

```
U545-HAL OK
BLINK 0 LD2=1 B1=0
BLINK 1 LD2=0 B1=0
```

Two 20M-step runs are byte-identical (`DETERMINISTIC`). Unsupported-instruction
audit at 200k steps: `unknown_thumb16: 0`, `unhandled_thumb32: 0`,
`unsupported_total: 0`.

## D. Chip drift guard

`stm32u545.yaml` is a standalone derivative of `stm32u575.yaml`.
`crates/core/tests/u545_u575_drift_guard.rs` checks every shared peripheral
(type, base, size, irq, clock gate, config) against an explicit allow-list:
DBGMCU `idcode` and `sysmem.config.flash_size_kb` (512 vs 2048 KiB). The
top-level allow-list covers name, flash/RAM sizes, and package pin maps; only
USART2, GPIOF and GPIOI are removed. Changing any other U575 field without
mirroring it fails with the peripheral and field named (verified by mutating
the U575 USART1 `irq`).

## E. Arduino matrix (fidelity engine)

PlatformIO has no `nucleo_u545re_q` board and the stm32duino core 3.0.0 has no
NUCLEO_U545RE_Q variant. The matrix uses the generic `U545RETxQ` variant through a
local board file (`validation/arduino-matrix/pio-boards/nucleo_u545re_q.json`) that
overrides `LED_BUILTIN=PA5`, `Serial` = USART1 PA9/PA10, Wire = PB7/PB6, SPI SCK =
PA5, and reuses the U575 `ldscript.ld` (the core ships no U535/U545 linker script;
RAM/flash sizes come from the board file).

```bash
python3 validation/arduino-matrix/run_matrix.py --boards stm32u545
```

Result: **9 pass / 0 skip / 0 fail** (L0 serial, L1 loop, L2 blink on `gpioa:5`,
L3 INA219 I2C, L4 MAX31855 SPI, L5 ADC, L6 PWM, L7 timer, L8 FDCAN1 loopback).
Two sketch tweaks were needed: L6 drives PA0 (TIM2_CH1) because LD2 is PA5 =
DAC1_OUT2 and the twin has no DAC window (first run: `unmodeled`); L8 selects the
U5 FDCAN1 path for `STM32U545xx` (first run: `no_can`).

## F. Zephyr: not validated

The local Zephyr workspace is 3.7.2 (`c66235fb7346`). It has no `nucleo_u545re_q`
board and no STM32U545 SoC (`dts/arm/st/u5` has U575/U585/U595/U599/U5A5/U5A9 only);
both exist on upstream `main`. Validating needs a `west update` to a newer Zephyr and
HAL (multi-GB) which did not fit the build host's free disk budget, and backporting
the SoC into 3.7.2 would not be stock Zephyr. No Zephyr result is claimed and there
is no committed Zephyr fixture or `zephyr-matrix` entry.

## G. Not done / uncertain

- No silicon capture; datasheet / RM0456 / UM3062 not read (st.com unreachable).
- Morpho (CN7/CN10) pin assignments are not documented (UM3062 only).
- `images/` positions are measured from ST's product photo, not ST's CAD files.
- No Arduino survival ELF is committed (the matrix lane covers Arduino).
