# Arduino UNO R4 Minima (Renesas R7FA4M1AB) — I2C smoke

A Cortex-M4 board on the Renesas RA4M1, the first RA part in the engine.
USB CDC Serial is not attached; the smoke uses SCI2 on D0/D1 and RIIC IIC1 on
A4/SDA=P101 and A5/SCL=P100 with an external BME280 at `0x76`.

## Status at a glance

| Aspect             | Status                                                              |
|--------------------|---------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/ra4m1.yaml`](../../configs/chips/ra4m1.yaml)         |
| System yaml        | [`configs/systems/arduino-uno-r4-minima.yaml`](../../configs/systems/arduino-uno-r4-minima.yaml) |
| Firmware           | `firmware-ra4m1-bme280-smoke` |
| Smoke              | [`bme280-smoke.yaml`](../../examples/arduino-uno-r4-minima/bme280-smoke.yaml) |
| Evidence           | [`examples/arduino-uno-r4-minima/evidence/`](../../examples/arduino-uno-r4-minima/evidence/) |
| Tier               | **local simulator smoke** — no silicon diff |

## What is proven

Bare-metal firmware waits for HOCO, configures IIC1 and reads BME280 chip ID
`0x60` from register `0xD0`; the smoke checks the RA IIC1 bus trace and reports
the result on SCI2. It also sets PFS IIC1 mux values for P100/P101 and toggles
P111 (D13). The controller transaction is modeled, while the simulator does
not electrically route IIC1 through the header pads.

## What is not proven

No silicon diff and no executing-fidelity differential. USBFS is a stub
window; PFS IIC mux routing to physical pads is not modeled. The analog blocks
and SCI's other modes are unmodelled, and clock gating fidelity is limited to
what the smoke exercises. A real BME280 breakout must be compatible with the
UNO R4 Minima's 5 V I/O, with suitable level shifting and pull-ups.

## How to run

```bash
./scripts/onboarding_smoke.sh \
  --target-id arduino-uno-r4-minima-bme280 \
  --crate firmware-ra4m1-bme280-smoke --target thumbv7em-none-eabi \
  --script examples/arduino-uno-r4-minima/bme280-smoke.yaml \
  --system configs/systems/arduino-uno-r4-minima.yaml \
  --out-dir examples/arduino-uno-r4-minima/evidence
```

The onboarding smoke lives in
[`examples/arduino-uno-r4-minima/`](../../examples/arduino-uno-r4-minima/).

## Related

- [Arduino Uno (ATmega328P)](arduino-uno.md) — the AVR board of the same name
