# EOS S3 Qomu validation status

## Current measurement (2026-11-09)

The Qomu system booted `firmware-eos-s3-smoke` for **100,000 instructions**
and passed **6/6 smoke assertions** (`EOS S3 checks PPPPPP DONE`, summary
0xE053003F, 6 edges on GPIO 4). Evidence:
[`docs/coverage/renode-onboarding/metrics/eos-s3-qomu/`](../coverage/renode-onboarding/metrics/eos-s3-qomu/).

The chip descriptor includes the `eos-s3` profile whole; the checks, the
memory map fix, the claimed peripherals and the negative controls are in
[`eos-s3.md`](eos-s3.md).

Board limits:

- Renode `eos-s3-qomu.repl` detaches the PL011 (`uart: @ none`) because the
  board routes no UART pins. The silicon PL011 is present and the smoke
  reads it, but a Qomu user cannot see that UART on the board.
- Not declared: the USB serial port in the FPGA fabric (0x40020000, no
  model) and the LEDs (GPIO 0/1/3 per Renode only; no second source).

The profile's `verified` field remains false. This result is limited to the
local model behaviour; it is not datasheet validation or hardware
confirmation.
