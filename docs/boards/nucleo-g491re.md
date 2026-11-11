# NUCLEO-G491RE

ST's STM32G491RE Nucleo-64 board, modelled for Cortex-M4 firmware that drives
the user LED on PA5. A separate system profile adds an Adafruit BME280 on I2C1.

!!! tip "Live status"
    The status below is a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — model estate and conformance level
    - [Tier-1 matrix](../coverage/tier1-scoreboard.md) — per-peripheral pass/fail
    - [Target support rubric](../target_support_rubric.md) — modeled / stub / silicon-verified

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/stm32g491re.yaml`](../../configs/chips/stm32g491re.yaml) |
| Board system | [`configs/systems/nucleo-g491re.yaml`](../../configs/systems/nucleo-g491re.yaml) |
| Copy-paste smoke | [`examples/nucleo-g491re/`](../../examples/nucleo-g491re/) |
| BME280 add-on smoke | [`examples/nucleo-g491re-bme280/`](../../examples/nucleo-g491re-bme280/) |
| Playground board | Not registered in the hosted catalog |
| Validation | Local GPIO and BME280 I2C smokes; no silicon capture or hosted verifier verdict |

## Flash / firmware artifact

The example builds a bare-metal Cortex-M4 ELF for `thumbv7em-none-eabi`:

```bash
cargo build -p firmware-nucleo-g491re-smoke --release --target thumbv7em-none-eabi
```

## Pins

| Board label | MCU pin | Model use | Smoke coverage |
|-------------|---------|-----------|----------------|
| LD2 user LED | PA5 | GPIOA output, active high | Driven and checked for transitions |
| B1 user button | PC13 | GPIOC input, active low | Wiring declared; not read by this smoke |
| BME280 SDA | PB9 / Arduino D14 | I2C1 SDA, AF4 | Add-on smoke checks pad activity and sensor reads |
| BME280 SCL | PB8 / Arduino D15 | I2C1 SCL, AF4 | Add-on smoke checks pad activity and sensor reads |

## Support matrix

| Block | Status | Notes |
|-------|--------|-------|
| Cortex-M4 execution | ✅ | Boots the committed no-std firmware in the local CLI smoke |
| Main flash and SRAM | ✅ | 512 KiB flash and 96 KiB main SRAM; separate CCM SRAM is not mapped |
| RCC / GPIOA | ✅ | G4 RCC and GPIO models exercise the PA5 LED path |
| GPIOC / B1 mapping | ⚠️ | Board wiring and GPIOC are described; button behavior is not asserted |
| GPIOB / I2C1 | ✅ | Declared for the separately scoped BME280 add-on smoke |
| ST-LINK, USB, oscillators, analog | ❌ | Outside this model |
| Other G491 peripherals | ❌ | Not declared by this intentionally narrow model |

## What it catches vs what needs a bench

The smoke catches firmware boot, GPIO clock enable, PA5 configuration, output
set/reset, and LED-level readback. The add-on smoke also reads BME280 ID and raw
temperature over I2C1. Neither smoke establishes silicon equivalence, electrical
behavior, analog performance, USB or ST-LINK behavior, or other G491 peripherals.
See the [base validation](../../examples/nucleo-g491re/VALIDATION.md) and
[BME280 validation](../../examples/nucleo-g491re-bme280/VALIDATION.md) for their
separate boundaries and evidence.

## How to run

### CLI

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-g491re/io-smoke.yaml \
  --output-dir out/nucleo-g491re/io-smoke --no-uart-stdout
```

### Playground and MCP

This board does not have a hosted catalog entry yet, so it cannot currently be
selected in Playground or addressed through the hosted board tools.

## Related systems and examples

- [`configs/systems/nucleo-g491re.yaml`](../../configs/systems/nucleo-g491re.yaml) — board wiring
- [`examples/nucleo-g491re/README.md`](../../examples/nucleo-g491re/README.md) — quick start and copy-paste commands
- [`examples/nucleo-g491re/vendor-status.md`](../../examples/nucleo-g491re/vendor-status.md) — live ST availability evidence
- [`examples/nucleo-g491re-bme280/README.md`](../../examples/nucleo-g491re-bme280/README.md) — BME280 wiring and copy-paste example
