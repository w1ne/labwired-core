# EOS S3 QuickFeather validation status

## Current measurement (2026-11-09)

The QuickFeather system booted `firmware-eos-s3-smoke` for **100,000
instructions** and passed **6/6 smoke assertions** (`EOS S3 checks PPPPPP
DONE`, summary 0xE053003F, 6 edges on GPIO 4). Evidence:
[`docs/coverage/renode-onboarding/metrics/eos-s3-quickfeather/`](../coverage/renode-onboarding/metrics/eos-s3-quickfeather/).

The chip descriptor includes the `eos-s3` profile whole; the checks, the
memory map fix, the claimed peripherals and the negative controls are in
[`eos-s3.md`](eos-s3.md).

Board parts not declared:

- LEDs and the user button. Renode `eos-s3-quickfeather.repl` wires the LEDs
  to GPIO 4/5/6; qorc-sdk `BSP/quickfeather/src/leds.c` drives IO_OUTPUT
  bits 1/4/6. The two sources disagree, so no board LED is claimed.
- MC3635 accelerometer: it sits behind the FFE I2C master, which has no
  model.

The profile's `verified` field remains false. This result is limited to the
local model behaviour; it is not datasheet validation or hardware
confirmation.
