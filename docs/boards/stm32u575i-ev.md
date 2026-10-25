# STM32U575I-EV (STM32U575AII6Q)

The **STM32U575I-EV** is ST's evaluation board (**MB1550**) for the
**STM32U575AII6Q**: Arm Cortex-M33 with FPU and TrustZone, 2 MB flash,
768 KiB main SRAM + 16 KiB SRAM4. It shares the U5 register map and memory
geometry with the [NUCLEO-U575ZI-Q](stm32u575.md), so both boards use the same
chip descriptor, [`configs/chips/stm32u575.yaml`](../../configs/chips/stm32u575.yaml).

> **Fidelity: SIM-DERIVED.** There is no U575I-EV bench part. Board facts come
> from **UM2854 Rev 2** and ST's **`stm32u575i-ev-bsp` v1.4.1**; the MCU model
> is the shared U575 descriptor (RM0456 / DS13736 / the vendor SVD). Nothing
> here is a silicon capture. The community **60North `renode-stm32u5`** package
> (commit `9cef86b9a`) is a Renode-derived reference, **not silicon** — its
> `stm32u575i-ev-st-examples.robot` FDCAN loopback is reproduced by the
> `stm32u575_60north` target. See the
> [60North runbook](../../validation/stm32u5-60north/README.md).

## Status at a glance

> **Live status:** the table below is a hand-maintained snapshot. For the
> authoritative, auto-generated view see the
> [validation status](VALIDATION_STATUS.md) and
> [chip conformance scoreboard](../coverage/chip-conformance.md).

| Aspect | Status |
|--------|--------|
| Chip yaml | [`configs/chips/stm32u575.yaml`](../../configs/chips/stm32u575.yaml) (shared with the NUCLEO) |
| System yaml | [`configs/systems/stm32u575i-ev.yaml`](../../configs/systems/stm32u575i-ev.yaml) |
| Example | [`examples/stm32u575i-ev/`](../../examples/stm32u575i-ev/) (`VALIDATION.md` = runbook + evidence) |
| Reference firmware | `crates/firmware-stm32u575i-ev-smoke` (bare-metal Rust, LED5/LED6 + USART1) |
| Validation | committed io-smoke with ODR read-back; PR-gated `firmware_survival`; shared U575 conformance; 60North reference |
| Tier | **L1 smoke, SIM-DERIVED** |
| Core | Arm Cortex-M33 (TrustZone present, factory-disabled) |

## Pins (from UM2854 / ST BSP v1.4.1)

| Board label | MCU pin | Notes |
|-------------|---------|-------|
| LED5 (green) | PB7 | active high (`BSP_LED_On` drives high); `BSP_LED_GREEN` |
| LED6 (red) | PH7 | active high; `BSP_LED_RED` |
| LED7 (blue) | MFX I/O expander | not a direct GPIO — not modelled |
| LED8 (yellow) | MFX I/O expander | not a direct GPIO — not modelled |
| USER button | PC13 | active high, external pull-down |
| COM1 TX / RX | PA9 / PA10 | USART1 AF7, 115200 8N1 (ST-LINK-V3E VCP) |

The STM32CubeU5 `STM32U575I-EV/Examples/FDCAN/FDCAN_Loopback` example lights
LED5 on success and LED6 on error; the 60North reference checks exactly those
pins (PB7 / PH7).

## What the model covers

The MCU surface is the U575's; see the
[peripheral table in stm32u575.md](stm32u575.md#peripherals-from-chip-yaml).
This board package specifically proves:

| Surface | State | Evidence |
|---------|-------|----------|
| RCC AHB2ENR1 GPIOB/GPIOH + APB2ENR USART1 gates | ✅ tested in-sim | firmware enables them before any GPIO/USART access |
| GPIOB PB7 / GPIOH PH7 output path | ✅ tested in-sim | `odr_b=` / `odr_h=` read back after each write |
| USART1 COM1 console | ✅ tested in-sim | deterministic banner + heartbeat on the TX sink |
| Memory map (2 MB flash, 768 KiB SRAM) | ✅ tested in-sim | ELF links and executes at `0x0800_0000` |
| LED7/LED8 (MFX expander) | ❌ not modelled | see `EXTERNAL_COMPONENTS.md` |
| LCD, camera, Octo-SPI, microSD, audio, EEPROM | ❌ not modelled | board peripherals out of scope |

## Firmware artifacts

| Use | Artifact | Notes |
|-----|----------|-------|
| io-smoke (PR gate) | `tests/fixtures/stm32u575i-ev-smoke.elf` | committed, built from `crates/firmware-stm32u575i-ev-smoke`; source build in the onboarding workflow |
| Survival gate | `firmware_survival::test_stm32u575i_ev_smoke_survival` | asserts `STM32U575I-EV SMOKE\n` on USART1 |
| 60North reference | external 60North ELFs | `stm32u575_60north`, model branch |

## How to run

```bash
# Source build + run
cargo build -p firmware-stm32u575i-ev-smoke --release --target thumbv8m.main-none-eabi
labwired --firmware target/thumbv8m.main-none-eabi/release/firmware-stm32u575i-ev-smoke \
  --system configs/systems/stm32u575i-ev.yaml --max-steps 4000000
# → STM32U575I-EV SMOKE / led5=on odr_b=00000080 ... / hb=0 ...

# Committed-fixture smoke
labwired test --script examples/stm32u575i-ev/io-smoke.yaml
# → PASS  3/3 checks · io-smoke · 4000000 steps
```

## Known omissions

- The MFX I/O expander, its LED7/LED8, LCD, camera, Octo-SPI memories, microSD,
  audio codec and EEPROM are not modelled or attached.
- TrustZone/GTZC, OCTOSPI/HSPI, USB and ADC4 are not modelled (same U575 gaps).
- No bench part, no SWD capture and no silicon differential. Passing the 60North
  reference means matching a second software model, not silicon.

## Related systems & examples

- [`examples/stm32u575i-ev/VALIDATION.md`](../../examples/stm32u575i-ev/VALIDATION.md)
- [`examples/stm32u575i-ev/REQUIRED_DOCS.md`](../../examples/stm32u575i-ev/REQUIRED_DOCS.md)
- [`examples/stm32u575i-ev/EXTERNAL_COMPONENTS.md`](../../examples/stm32u575i-ev/EXTERNAL_COMPONENTS.md)
- [60North runbook](../../validation/stm32u5-60north/README.md)
