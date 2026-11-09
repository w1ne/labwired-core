# Raspberry Pi Pico 2 (RP2350)

**Demand rank:** RP2350 is part of the demand list's #2 RP2040/RP2350 target.
The survey measures RP2040 at #5 (12%); RP2350 is the current flagship and
was not reported as a separate survey row.

The board profiles boot UART0 firmware and toggle the onboard GP25 LED through
SIO. The PIO smoke exercises `SET PINDIRS`, `SET PINS`, and `JMP` on the
RP2350 Pico 2 profile. The ADC smoke reads GP26 / ADC0 through a modeled
potentiometer input.

These system profiles are intentionally limited to peripherals with runnable
models and smokes. The shared narrow chip slice is
[`configs/chips/rp2350-pico2-smoke.yaml`](../../configs/chips/rp2350-pico2-smoke.yaml);
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

## ADC0 potentiometer smoke

The Pico 2's GP26 header pad is ADC0. The example uses a modeled external
linear potentiometer, with its wiper on GP26 (header pin 31), its high terminal
on ADC_VREF (pin 35), and its low terminal on AGND (pin 33). The default
midpoint is 1.65 V, which the model's ideal 12-bit ADC converts to 2048 counts
against 3.3 V. The system profile also maps GP27/ADC1 and GP28/ADC2, but this
smoke exercises GP26/ADC0 only.

The exact bare-metal firmware source is
[`crates/firmware-pico2-adc-smoke/src/main.rs`](../../crates/firmware-pico2-adc-smoke/src/main.rs).
Its ADC sample and the onboard GP25 LED are checked by the committed smoke:

```rust
const ADC_CS: *mut u32 = 0x400A_0000 as *mut u32;
const ADC_RESULT: *const u32 = 0x400A_0004 as *const u32;

unsafe fn read_gp26_adc0() -> u16 {
    core::ptr::write_volatile(ADC_CS, 1 | (1 << 2)); // EN | START_ONCE
    (core::ptr::read_volatile(ADC_RESULT) & 0x0fff) as u16
}
```

The complete `no_std` firmware also reports the result on UART0 and toggles
the onboard LED. Its default potentiometer fixture reads 2048 in the ideal
model.

```bash
scripts/onboarding_smoke.sh \
  --target-id pico2-adc \
  --crate firmware-pico2-adc-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/pico2/adc-smoke.yaml \
  --system configs/systems/pico2-adc.yaml \
  --firmware-build-dir crates/firmware-pico2-adc-smoke \
  --out-dir examples/pico2/evidence/adc
```

The committed metrics and one-target scoreboard are in
[`evidence/adc/`](evidence/adc/).

This smoke exercises the ADC register conversion path and board-level analog
input routing; it does not validate Pico SDK `adc_*` calls, USB, or a physical
Pico 2 capture. The hosted LabWired verification catalog currently has no
Pico 2 target, so this is a repository-local smoke result rather than a hosted
`labwired_verify` verdict. Availability for this exact board is recorded in
[`evidence/adc/vendor-status.md`](evidence/adc/vendor-status.md).

## Vendor status checked 2026-11-09 UTC

Raspberry Pi's [Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
says **“Available now from $5”** and that the series will remain in production
until at least January 2040. DigiKey marks exact order code SC1631 Active and
showed stock when checked; see [`evidence/adc/vendor-status.md`](evidence/adc/vendor-status.md).

## Scope and limitations

- The base system models GP25 as the onboard LED. The ADC system exposes GP26
  as an analog input and maps GP27/GP28 to ADC1/ADC2; only ADC0 is exercised.
- ADC reference variation, silicon offset/noise, and calibration are not
  modeled, so `2048` is the smoke's ideal-model result, not a physical-board
  calibration claim.
- UART0 output is captured from PL011 data-register writes. USB CDC, the
  physical UART waveform, and USB boot/flash are not proven.
- The smoke system uses the narrow RP2350 slice. It omits POWMAN/TICKS, TBMAN,
  PADS_BANK0 electrical controls, TrustZone, the second core, Hazard3 RISC-V,
  PIO2 and RP2350-specific PIO-v2 behavior, and USB enumeration.
- This is repository simulation evidence. No physical Pico 2 capture exists.
