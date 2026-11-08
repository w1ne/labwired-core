# ESP32-S2-DevKitM-1-N4R2 validation boundary

## Modelled in this profile

- ESP32-S2 single-core Xtensa LX7 at 240 MHz, entering a freestanding ELF at
  its linked XIP address as a fast boot handoff.
- 4 MiB flash XIP window at `0x4008_0000` and 320 KiB SRAM at
  `0x3FFB_0000`, matching the ESP32-S2 datasheet memory map used by this image.
- UART0 FIFO writes captured by the local CLI console.
- GPIO18 output through `GPIO_FUNC18_OUT_SEL_CFG`, `GPIO_ENABLE_W1TS`,
  `GPIO_OUT_W1TS`, and `GPIO_OUT_W1TC` at the ESP32-S2 GPIO block
  (`0x6000_4000`).
- One NeoPixel attached to GPIO18, decoding the emitted GRB waveform.

## Explicitly outside this profile

The model does not claim a real ROM boot, SPI flash MMU aliases, PSRAM, Wi-Fi,
USB OTG, native USB, the board's USB-to-UART bridge, button behavior, analog
electrical behavior, or unlisted ESP32-S2 peripherals. UART0 is observed at the
simulator console; board-level USB bridge propagation is not simulated.

The GPIO implementation reuses the ESP32-S3 register model for the documented
common GPIO output register head only. This profile's smoke covers direct GPIO
output selection and GPIO18 transitions. It makes no claim about S2 GPIO
interrupt routing, alternate-function signal indices, package-specific pin
behavior, or S3-only pads.

## Reproduce

```bash
scripts/build-esp32s2-devkitm1-smoke.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32s2-devkitm1-n4r2/onboarding-smoke.yaml \
  --output-dir out/esp32s2-devkitm1-n4r2 \
  --no-uart-stdout
```

Hardware facts come from Espressif's [ESP32-S2 datasheet](https://documentation.espressif.com/esp32-s2_datasheet_en.html),
[ESP32-S2 technical reference manual](https://www.espressif.com/sites/default/files/documentation/esp32-s2_technical_reference_manual_en.pdf),
and [ESP32-S2-DevKitM-1 user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s2/esp32-s2-devkitm-1/user_guide.html).
