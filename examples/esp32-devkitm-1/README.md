# Espressif ESP32-DevKitM-1

**Demand rank:** ESP32 is #1 in the measured MCU-use list at 38%. This adds a
board-specific model profile for Espressif's breadboard-friendly
ESP32-DevKitM-1. It reuses the existing classic ESP32 model; this is a board
coverage extension, not a new MCU core.

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

The profile declares only UART0 and GPIO for the serial path and external
indicator. Wi-Fi, Bluetooth, the physical USB-to-UART bridge, USB,
reset/download buttons, regulator, power LED, and other peripherals are outside
the profile and are not claimed as modeled.

Live status and purchase evidence is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
