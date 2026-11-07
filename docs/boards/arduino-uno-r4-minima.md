# Arduino Uno R4 Minima (Renesas R7FA4M1AB) — UART/LED and analog smoke

A Cortex-M4 board on the Renesas RA4M1, the first RA part in the engine.
USB CDC Serial is not attached; the smoke uses SCI2 on the D0/D1 header.

## Status at a glance

| Aspect             | Status                                                              |
|--------------------|---------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/ra4m1.yaml`](../../configs/chips/ra4m1.yaml)         |
| System yaml        | [`configs/systems/arduino-uno-r4-minima.yaml`](../../configs/systems/arduino-uno-r4-minima.yaml) |
| Committed ELF      | `tests/fixtures/ra4m1-uno-r4-smoke.elf`                              |
| Validation         | `firmware_survival::test_ra4m1_uno_r4_smoke_survival`                |
| Analog proof       | `cosim_ra4m1_analog::uno_r4_minima_ntc_and_diode_netlists_reach_a0_a1_and_adc14` |
| Tier               | **smoke-manual** — boots firmware and prints, **no silicon diff**    |

## What is proven

A bare-metal firmware completes the RA clock bring-up (HOCO, OSCSF ready),
configures SCI2 for the D0/D1 header, writes `OK\n` three times, and toggles
P111 (D13 per ArduinoCore-renesas `variants/MINIMA`) through the RA PORT
registers — a behavioural model, not a stub. Separately, a solved Beta NTC
divider on A0/P014/AN009 and a Shockley diode load line on A1/P000/AN000 reach
the ADC14 through the descriptor's analog pin map. The ADC converts both
channels in 12-bit and 14-bit modes, with an unconnected neighboring channel
remaining at zero.

## What is not proven

No silicon diff and no executing-fidelity differential. The analog proof is
sim-derived: it covers ADC14 group-A software scans and the six Uno header
analog pin mappings, not the comparator, DAC, internal temperature/reference
inputs, or advanced ADC modes. USBFS is a stub window, and SCI's other modes
and clock-gating fidelity are limited to what the smoke exercises.

## How to run

```bash
labwired test --system configs/systems/arduino-uno-r4-minima.yaml \
  --firmware tests/fixtures/ra4m1-uno-r4-smoke.elf
```

The onboarding smoke lives in
[`examples/arduino-uno-r4-minima/`](../../examples/arduino-uno-r4-minima/).

## Related

- [Arduino Uno (ATmega328P)](arduino-uno.md) — the AVR board of the same name
