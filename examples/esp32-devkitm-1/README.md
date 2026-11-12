# Espressif ESP32-DevKitM-1

**Demand rank:** ESP32 is #1 in the measured MCU-use list at 38%. This adds a
system fixture for Espressif's breadboard-friendly ESP32-DevKitM-1. It uses
the existing [classic ESP32 descriptor](../../configs/chips/esp32.yaml) and
adds coverage for this board's UART0 and external GPIO23 indicator.

The exact board uses an ESP32-MINI-1 module with 4 MB flash and no PSRAM. The
system profile covers UART0 and an external indicator LED attached to header
GPIO23 through a 330 Ω series resistor. Espressif maps UART0 TX/RX to GPIO1/GPIO3
and connects those lines to the onboard USB-to-UART bridge. The simulator
captures UART0 output at the MCU model; the USB bridge and USB connection
themselves are not simulated. GPIO23 is not an onboard LED.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id esp32-devkitm-1 \
  --firmware-build-script examples/esp32-devkitm-1/build-firmware.sh \
  --script examples/esp32-devkitm-1/onboarding-smoke.yaml \
  --system configs/systems/esp32-devkitm-1.yaml \
  --out-dir examples/esp32-devkitm-1/evidence
```

The smoke requires the `ESP32_DEVKITM1_UART0_OK` UART0 marker and at least 16
GPIO23 edges. Connect a physical LED anode to GPIO23 through a 330 Ω resistor
and its cathode to GND to reproduce the external indicator wiring. The run log
and full simulator result are committed under `evidence/`. The hosted verifier
separately model-verifies the shared ESP32 GPIO23 path; its catalog does not
contain this exact DevKitM-1 profile and assumes an ideal 3.3 V rail.

## Model boundary

The system reuses the existing classic ESP32 model and memory map. This smoke
validates UART0 and GPIO23 for the serial path and external indicator. It does
not validate Wi-Fi, Bluetooth, the physical USB-to-UART bridge, USB,
reset/download buttons, regulator, power LED, or other peripherals.

## Evidence provenance

The committed receipts in `evidence/` were captured on 2026-11-10 with the
original narrow `esp32-devkitm-1-smoke` descriptor. Their logs remain unchanged
to preserve that provenance. The current system uses the canonical `esp32.yaml`
descriptor, which has the same CPU, flash, RAM, UART0, and GPIO map needed by
this fixture.

Live status and purchase evidence is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
