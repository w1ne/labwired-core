# Raspberry Pi Pico 2 (RP2350)

**Demand rank:** RP2350 is part of the demand list's #2 RP2040/RP2350 target.
The survey measures RP2040 at #5 (12%); RP2350 is the current flagship and
was not reported as a separate survey row.

The board profile boots UART0 firmware, toggles the onboard GP25 LED through
SIO, and runs a PIO0 state machine on the same LED. The PIO smoke exercises
`SET PINDIRS`, `SET PINS`, and `JMP` on the RP2350 Pico 2 profile.

This system profile is intentionally limited to peripherals with a runnable
model and smoke: the RP2350's UART0 data-register path, IO_BANK0 function
selection for GP25, and SIO/PIO0 output driving the onboard user LED. The
working chip slice is [`configs/chips/rp2350-pico2-smoke.yaml`](../../configs/chips/rp2350-pico2-smoke.yaml);
the broader RP2350 descriptor remains separate and has additional gaps.

## Run

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id pico2 \
  --crate firmware-pico2-demo \
  --target thumbv8m.main-none-eabi \
  --script examples/pico2/io-smoke.yaml \
  --system configs/systems/pico2.yaml \
  --firmware-build-dir crates/firmware-pico2-demo \
  --out-dir examples/pico2/evidence
```

The smoke expects the `PICO2_SMOKE_OK` UART transcript and at least eight GP25
edges. The committed `evidence/` directory records the build and simulation.

The PIO-specific board smoke runs the PIO0 SET/JMP loop on GP25:

```bash
scripts/onboarding_smoke.sh \
  --target-id pico2-pio \
  --crate firmware-pico2-demo \
  --target thumbv8m.main-none-eabi \
  --script examples/pico2/pio-smoke.yaml \
  --system configs/systems/pico2.yaml \
  --firmware-build-dir crates/firmware-pico2-demo \
  --out-dir examples/pico2/evidence/pio
```

## Vendor status checked 2026-11-06 UTC

Raspberry Pi's [Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
says **“Available now from $5”** and that the series will remain in production
until at least January 2040. The exact SC1631 listing was also shown in stock;
see [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Scope and limitations

- GP25 is the only board-level pin represented here, as the onboard LED.
- UART0 output is captured from PL011 data-register writes. USB CDC, the
  physical UART waveform, and USB boot/flash are not proven.
- The smoke system uses the narrow RP2350 slice. It omits POWMAN/TICKS, TBMAN,
  PADS_BANK0 electrical controls, TrustZone, the second core, Hazard3 RISC-V,
  PIO2 and RP2350-specific PIO-v2 behavior, and USB enumeration.
- This is repository simulation evidence. No physical Pico 2 capture exists.
