# Teensy 4.1 (NXP i.MX RT106x) — UART/LED smoke

A Cortex-M7 board on the i.MX RT1060 family. The chip yaml is the RT1064-class
descriptor (MIMXRT1062 on the real Teensy 4.1 is the same RT1060 core with
different flash); soft-float images linked in DTCM are what the engine runs.

## Status at a glance

| Aspect             | Status                                                              |
|--------------------|---------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/imxrt1064.yaml`](../../configs/chips/imxrt1064.yaml) |
| System yaml        | [`configs/systems/teensy-41.yaml`](../../configs/systems/teensy-41.yaml) |
| Committed ELF      | `tests/fixtures/imxrt1064-teensy41-smoke.elf`                        |
| Validation         | `firmware_survival::test_imxrt1064_teensy41_smoke_survival`          |
| Tier               | **smoke-manual** — boots firmware and prints, **no silicon diff**    |

## What is proven

A bare-metal firmware un-gates the peripheral clocks in `CCM CCGR`, routes the
LED pin through IOMUXC, writes `OK\n` on LPUART6, and toggles GPIO2_IO03
(pin 13 / LED_BUILTIN) through the DR_TOGGLE register. CCM and IOMUXC are
behavioural models, not stubs.

## What is not proven

No silicon diff and no executing-fidelity differential; FlexSPI is a stub
window and the XIP path is skipped (images run from DTCM). GPT/PIT/DMA and
the USB blocks are unmodelled.

## How to run

```bash
labwired test --system configs/systems/teensy-41.yaml \
  --firmware tests/fixtures/imxrt1064-teensy41-smoke.elf
```

The onboarding smoke lives in [`examples/teensy-41/`](../../examples/teensy-41/).
