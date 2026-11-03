# Raspberry Pi Pico 2 (RP2350)

This system profile is intentionally limited to peripherals with a runnable
model and smoke: the RP2350's UART0 data-register path, IO_BANK0 function
selection for GP25, and SIO output driving the onboard user LED. The working
chip slice is [`configs/chips/rp2350-pico2-smoke.yaml`](../../configs/chips/rp2350-pico2-smoke.yaml);
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

## Vendor status checked 2026-11-03

Raspberry Pi's [Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
said **“Available now from $5”** and **“will remain in production until at
least January 2040.”**

## Scope and limitations

- GP25 is the only board-level pin represented here, as the onboard LED.
- UART0 output is captured from PL011 data-register writes. USB CDC, the
  physical UART waveform, and USB boot/flash are not proven.
- The smoke system uses the narrow RP2350 slice. It omits POWMAN/TICKS, TBMAN,
  PADS_BANK0 electrical controls, TrustZone, the second core, Hazard3 RISC-V,
  PIO v2 behavior, and USB enumeration.
- This is repository simulation evidence. No physical Pico 2 capture exists.
