# Raspberry Pi Pico (RP2040): PIO LED smoke

**Demand rank:** RP2040 is #5 in the measured MCU list (12%), per the
JetBrains State of Embedded 2025 survey cited in the onboarding demand brief.

This smoke exercises the Pico's PIO0 state machine against its onboard GP25
LED. Firmware selects PIO0 in IO_BANK0, configures SET pins and directions for
GP25, and toggles the LED from PIO0 SM0. PIO0 SM1 separately checks that the TX
FIFO drains after a blocking `PULL`. The smoke requires both the `PIO_OK` UART
marker and at least eight GP25 edges.

## Run

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id rp2040-pio \
  --crate firmware-rp2040-pio-onboarding \
  --target thumbv6m-none-eabi \
  --script examples/rp2040-pio/io-smoke.yaml \
  --system examples/rp2040-pio/system.yaml \
  --out-dir examples/rp2040-pio/evidence
```

Build and simulation logs are committed in [`evidence/`](evidence/).

## Vendor status checked 2026-11-05

Raspberry Pi's [RP2040 specifications](https://www.raspberrypi.com/products/rp2040/specifications/)
list Pico 1 boards as **available from $4** and say RP2040 remains in
production until at least **January 2041**. The Pico product line is active and
buyable; the chip is not obsolete or NRND.

## Scope

- PIO0/PIO1 SET/OUT pin and direction writes are published through the live
  IO_BANK0 function mux to the RP2040 GPIO pad path. Overlapping writes resolve
  with the highest-numbered state machine, as specified for RP2040.
- This smoke uses SET PINS/PINDIRS on PIO0 SM0. It does not establish support
  for WAIT GPIO/PIN, side-set, a complete PIO instruction set, or RP2350 PIO-v2
  differences.
- UART output is captured from PL011 data-register writes. The smoke makes no
  USB CDC or physical hardware claim.
