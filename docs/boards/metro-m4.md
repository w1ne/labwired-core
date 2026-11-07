# Adafruit Metro M4 Express (Microchip SAMD51J19A) — UART/LED smoke

The smoke profile keeps the MCU on its reset DFLL48M clock at 48 MHz; it does
not configure the Metro M4's higher-speed DPLL clock. The onboarding profile
covers OSCCTRL/MCLK/GCLK, PORTA, SERCOM3 Serial1 on PA22/PA23, the D13 LED on
PA16, and SysTick. It omits the board's QSPI flash, native USB, NeoPixel, and
unused MCU peripherals.

## Status at a glance

| Aspect             | Status                                                              |
|--------------------|---------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/atsamd51-metro-m4-smoke.yaml`](../../configs/chips/atsamd51-metro-m4-smoke.yaml) |
| System yaml        | [`configs/systems/metro-m4.yaml`](../../configs/systems/metro-m4.yaml) |
| Committed ELF      | `tests/fixtures/atsamd51-metro-m4-smoke.elf`                         |
| Validation         | `firmware_survival::test_atsamd51_metro_m4_smoke_survival`           |
| Onboarding evidence | [`examples/metro-m4/evidence/`](../../examples/metro-m4/evidence/)   |
| Tier               | **smoke-manual** — simulator evidence, **no silicon diff**           |

## What is proven

A bare-metal firmware enables `MCLK.APBBMASK` bit 10 and `GCLK PCHCTRL[24]`,
muxes PA22/PA23 to SERCOM3 PAD0/PAD1, configures 115200-baud USART, transmits
`METRO_M4_OK`, and toggles PA16. The smoke asserts both the SERCOM3 transcript
and a PORTA edge. The descriptor builds through `SystemBus::from_config` and
omits USB/QSPI windows rather than declaring zero-answering models for them.

## What is not proven

No silicon diff or executing-fidelity differential is recorded. QSPI, USB,
NeoPixel, and unused SERCOMs are omitted from the focused board profile.
DMA/ADC/DAC/EIC and the compare channels are unmodelled.

## How to run

```bash
scripts/onboarding_smoke.sh \
  --target-id metro-m4 \
  --crate firmware-atsamd51-demo \
  --target thumbv7em-none-eabi \
  --script examples/metro-m4/onboarding-smoke.yaml \
  --system configs/systems/metro-m4.yaml \
  --out-dir examples/metro-m4/evidence
```

The onboarding smoke lives in [`examples/metro-m4/`](../../examples/metro-m4/).

## Related

- [ATSAMD21G18A (Arduino Zero)](atsamd21g18a.md) — the SAM D21 sibling
- [Nano 33 IoT](nano-33-iot.md) — the D21 maker board
