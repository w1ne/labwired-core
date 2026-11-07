# Known limits

- The profile covers only the onboard ST7789V2 over GP-SPI3, its reset/D-C/chip-select/backlight GPIOs, and UART0 output. It does not claim that PSRAM, the keyboard, BMI270 IMU, audio codec, speaker amplifier, microphone, infrared LED, microSD, battery, charger, Grove port, radio, USB, or expansion header is modelled.
- The ESP32-S3 SYSTEM clock/reset window remains an explicit placeholder. Firmware leaves it at reset state and does not exercise dynamic peripheral clock or reset gating.
- The smoke checks controller frame memory, the configured visible RAM crop, display-on state, and backlight GPIO state. It does not test color optics, brightness, physical display capture, or cold-start timing.
- The panel's GPIO38 backlight is also documented by M5Stack as the Stamp-S3A RGB-LED power enable. The LED itself is not attached to this narrow profile.
- The smoke uses the local repository simulator; it is not a physical-board test or a hosted `labwired_verify` verdict. See [`evidence/labwired-verification.md`](evidence/labwired-verification.md).
- The S3 engine's generic fast-boot options still attach an 8 MiB PSRAM model by default even though this ESP32-S3FN8 board has no external PSRAM. The profile and smoke do not use that window; selecting per-board PSRAM capacity remains an engine configuration gap.
