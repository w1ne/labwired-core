# Known limits

- This profile models only the ESP32-S3R2 boot path and the active-high white status LED on GPIO35.
- The onboard OLED is SSD1315 and is omitted because the engine does not currently model that controller. No OLED initialization or display capability is claimed.
- The onboard SX1262 LoRa transceiver is not modelled. Radio, RF front-end, antenna, Wi-Fi and BLE behavior are outside this profile.
- The profile does not model 2 MB PSRAM, GNSS, battery management, solar input, USB, or the Vext power switch.
- The 16 MiB flash map matches the V4 S3R2 product listing. Firmware in this smoke does not exercise flash reads beyond the boot image.
- The local simulator smoke checks GPIO edges only and is not physical-board confirmation. The separate hosted LabWired result verifies GPIO35 on the generic ESP32-S3 model, not the exact Heltec system or its omitted peripherals.
