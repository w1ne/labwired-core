# Adafruit Metro M4 Express (Microchip SAMD51J19A) — UART/LED smoke

A Cortex-M4 board on the SAMD51 family, running at 120 MHz from the DFLL.
On-board QSPI flash, USB and NeoPixel are **not attached**; this page covers
the modelled slice: core, PORT A/B, MCLK+GCLK, SERCOM3 as Serial1, SysTick.

## Status at a glance

| Aspect             | Status                                                              |
|--------------------|---------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/atsamd51.yaml`](../../configs/chips/atsamd51.yaml)   |
| System yaml        | [`configs/systems/metro-m4.yaml`](../../configs/systems/metro-m4.yaml) |
| Committed ELF      | `tests/fixtures/atsamd51-metro-m4-smoke.elf`                         |
| Validation         | `firmware_survival::test_atsamd51_metro_m4_smoke_survival`           |
| Tier               | **smoke-manual** — boots firmware and prints, **no silicon diff**    |

## What is proven

A bare-metal firmware enables `MCLK.APBBMASK` bit 10 and `GCLK PCHCTRL[24]`
(SERCOM3_CORE), then writes `OK\n` on SERCOM3 DATA and toggles PA16 (D13 per
Adafruit `variants/metro_m4/variant.cpp`). The chip descriptor builds through
`SystemBus::from_config` (`atsamd51_config`). MCLK APB\*MASK and GCLK PCHCTRL
are behavioural models, not stubs.

## What is not proven

No silicon diff and no executing-fidelity differential; QSPI, USB, NeoPixel
and unused SERCOMs are deliberate `stub` windows (they answer zeros).
DMA/ADC/DAC/EIC and the compare channels are unmodelled.

## How to run

```bash
labwired test --system configs/systems/metro-m4.yaml \
  --firmware tests/fixtures/atsamd51-metro-m4-smoke.elf
```

The onboarding smoke lives in [`examples/metro-m4/`](../../examples/metro-m4/).

## Related

- [ATSAMD21G18A (Arduino Zero)](atsamd21g18a.md) — the SAM D21 sibling
- [Nano 33 IoT](nano-33-iot.md) — the D21 maker board
