# Arduino Nano 33 IoT (Microchip SAMD21G18A) — UART/LED smoke

A Cortex-M0+ board in the Arduino Nano form factor, built on the same
ATSAMD21G18A silicon as the Zero. On-board NINA-W102 WiFi, ATECC608A crypto
and the LSM6DS3 IMU are **not attached**; this page covers what the engine
models: the core, PORT, MCLK/PM+GCLK, SERCOM5 as Serial1, SysTick, and the
external-input ADC slice.

## Status at a glance

| Aspect             | Status                                                              |
|--------------------|---------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/atsamd21.yaml`](../../configs/chips/atsamd21.yaml)   |
| System yaml        | [`configs/systems/nano-33-iot.yaml`](../../configs/systems/nano-33-iot.yaml) |
| Committed ELF      | `tests/fixtures/atsamd21-nano33-smoke.elf`                           |
| Validation         | `firmware_survival::test_atsamd21_nano33_smoke_survival`; [`cosim_nano_33_iot_analog.rs`](../../crates/core/tests/cosim_nano_33_iot_analog.rs) |
| Tier               | **smoke-manual** — boots firmware and prints, **no silicon diff**    |

## What is proven

A bare-metal firmware performs the real SAM D21 bring-up in datasheet order —
NVMCTRL wait states, SYSCTRL.PCLKSR ready poll, GCLK SYNCBUSY spins,
PM.APBCMASK, WRCONFIG, then `CTRLB` before `CTRLA.ENABLE` on SERCOM5 — and its
`OK\n` console reaches the capture sink. The chip descriptor builds through
`Session`/`SystemBus::from_config` (`board_config_gates`), and the board LED is
declared on PA17 per ArduinoCore-samd `variants/nano_33_iot/variant.cpp`.

The mixed-signal test drives a Beta NTC divider on PA02/AIN0 and a separate
Shockley diode load line on PB08/AIN2. It checks the solved voltages, the
function-B pad mux, 12/10/8-bit ADC codes, and the adjacent undriven AIN3.
Both device types use the shared analog adapter and SAMD21 ADC route.

## What is not proven

No silicon diff (no Nano 33 IoT has been benched against an SWD oracle), and
no executing-fidelity differential of its own. Clock gating is a register
bank, so firmware that forgets `APBCMASK` works here and fails on hardware.
SERCOM SPI/I²C modes, EIC, USB, TCC/TC and DMAC are unmodelled. The generic
SAMD21 ADC slice is now attached to this profile and covered through an
end-to-end MNA test; this does not claim a silicon comparison.

## How to run

```bash
labwired test --system configs/systems/nano-33-iot.yaml \
  --firmware tests/fixtures/atsamd21-nano33-smoke.elf
```

The onboarding smoke lives in [`examples/nano-33-iot/`](../../examples/nano-33-iot/) —
the same bring-up a user runs without a toolchain.

## Related

- [ATSAMD21G18A (Arduino Zero)](atsamd21g18a.md) — the deep SAM D21 model
