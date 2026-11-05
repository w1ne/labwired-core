# ESP32-S2-DevKitM-1-N4R2 onboarding example

This profile boots a freestanding ESP32-S2 ELF, emits a UART0 marker, and
drives the board's on-board WS2812 RGB LED on GPIO18. The firmware bit-bangs a
GRB frame and sets the pixel red.

The model claim is limited to the single Xtensa LX7 core, the S2 XIP and SRAM
windows used by this ELF, UART0 TX capture, GPIO18 output, and the attached
NeoPixel model. ROM boot, flash MMU setup, Wi-Fi, USB OTG, PSRAM, buttons,
analog behavior, and other S2 peripherals are outside this smoke. GPIO model
reuse is limited to the S2/S3 common register path exercised here.

## Quick run

Install Espressif's `xtensa-esp32s2-elf-gcc` and make it available on `PATH`,
then run from the repository root:

```bash
scripts/build-esp32s2-devkitm1-smoke.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32s2-devkitm1-n4r2/onboarding-smoke.yaml \
  --output-dir out/esp32s2-devkitm1-n4r2 \
  --no-uart-stdout
```

The smoke asserts the UART marker, GPIO18 edge count, and the decoded red
pixel. Its committed run is under [`evidence/`](evidence/).

## Source and status

- Board wiring and exact buyable variant: [`evidence/vendor-status.md`](evidence/vendor-status.md)
- Local Core CLI result: [`evidence/onboarding-summary.md`](evidence/onboarding-summary.md)
- Hosted verifier availability: [`evidence/labwired-verification.md`](evidence/labwired-verification.md)
- Model boundary and register sources: [`VALIDATION.md`](VALIDATION.md)
