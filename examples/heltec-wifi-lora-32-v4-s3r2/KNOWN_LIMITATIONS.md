# Known limits

- This profile models the ESP32-S3 boot path, the active-high white status LED on GPIO35, and the SX1262 command interface on GP-SPI3.
- The onboard OLED is SSD1315 and is omitted because the engine does not currently model that controller. No OLED initialization or display capability is claimed.
- The SX1262 model covers selected SPI commands and IRQ status only. It reports `TxDone` immediately; RF transmission/reception, packet payload buffering, airtime, radio timing, antenna, FEM, and board FEM controls are not modelled.
- The physical SX1262 reset line (GPIO12), BUSY timing, and DIO2/DIO3 functions are not exercised by this profile. The model starts at its reset state when the simulation starts.
- The profile does not model 2 MB PSRAM, GNSS, battery management, solar input, USB, or the Vext power switch.
- The 16 MiB flash map matches the V4 S3R2 product listing. Firmware in this smoke does not exercise flash reads beyond the boot image.
- The repository's local simulator smoke checks the declared GPIO and SX1262 command behavior; it is not physical-board confirmation. The hosted LabWired result verifies GPIO35 on a generic ESP32-S3 model, not the exact Heltec system. The hosted component catalog currently has no SX1262 entry, so the radio smoke has no hosted model-verification result.
