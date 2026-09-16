# Arduino Uno R4 Minima (Renesas R7FA4M1AB) — UART/LED smoke

A Cortex-M4 board on the Renesas RA4M1, the first RA part in the engine.
USB CDC Serial is not attached; the smoke uses SCI2 on the D0/D1 header.

## Status at a glance

| Aspect             | Status                                                              |
|--------------------|---------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/ra4m1.yaml`](../../configs/chips/ra4m1.yaml)         |
| System yaml        | [`configs/systems/arduino-uno-r4-minima.yaml`](../../configs/systems/arduino-uno-r4-minima.yaml) |
| Committed ELF      | `tests/fixtures/ra4m1-uno-r4-smoke.elf`                              |
| Validation         | `firmware_survival::test_ra4m1_uno_r4_smoke_survival`                |
| Tier               | **smoke-manual** — boots firmware and prints, **no silicon diff**    |

## What is proven

A bare-metal firmware completes the RA clock bring-up (HOCO, OSCSF ready),
configures SCI2 for the D0/D1 header, writes `OK\n` three times, and toggles
P111 (D13 per ArduinoCore-renesas `variants/MINIMA`) through the RA PORT
registers — a behavioural model, not a stub.

## What is not proven

No silicon diff and no executing-fidelity differential. USBFS is a stub
window, the analog blocks and SCI's other modes are unmodelled, and clock
gating fidelity is limited to what the smoke exercises.

## How to run

```bash
labwired test --system configs/systems/arduino-uno-r4-minima.yaml \
  --firmware tests/fixtures/ra4m1-uno-r4-smoke.elf
```

The onboarding smoke lives in
[`examples/arduino-uno-r4-minima/`](../../examples/arduino-uno-r4-minima/).

## Related

- [Arduino Uno (ATmega328P)](arduino-uno.md) — the AVR board of the same name
