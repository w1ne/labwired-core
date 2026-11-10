# NUCLEO-G491RE

ST's STM32G491RE Nucleo-64 board, modelled for Cortex-M4 firmware that drives
the user LED on PA5.

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
| Playground board | Not registered in the hosted catalog |
| Validation | Local deterministic GPIO smoke; no silicon capture or hosted verifier verdict |

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

## Support matrix

| Block | Status | Notes |
|-------|--------|-------|
| Cortex-M4 execution | ✅ | Boots the committed no-std firmware in the local CLI smoke |
| Main flash and SRAM | ✅ | 512 KiB flash and 96 KiB main SRAM; separate CCM SRAM is not mapped |
| RCC / GPIOA | ✅ | G4 RCC and GPIO models exercise the PA5 LED path |
| GPIOC / B1 mapping | ⚠️ | Board wiring and GPIOC are described; button behavior is not asserted |
| ST-LINK, USB, oscillators, analog | ❌ | Outside this model |
| Other G491 peripherals | ❌ | Not declared by this intentionally narrow model |

## What it catches vs what needs a bench

The smoke catches firmware boot, GPIO clock enable, PA5 configuration, output
set/reset, and LED-level readback. It does not establish silicon equivalence,
electrical behavior, analog performance, USB or ST-LINK behavior, or the other
G491 peripherals. See [`examples/nucleo-g491re/VALIDATION.md`](../../examples/nucleo-g491re/VALIDATION.md)
for the model boundary and committed run evidence.

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
