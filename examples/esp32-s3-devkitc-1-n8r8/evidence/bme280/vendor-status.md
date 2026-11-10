# ESP32-S3-DevKitC-1-N8R8 availability check

Checked live against Espressif's current ESP32-S3-DevKitC-1 user guide on
2026-11-10:

- Espressif lists `ESP32-S3-DevKitC-1-N8R8` with an ESP32-S3-WROOM-1-N8R8,
  8 MB quad flash, 8 MB octal PSRAM, and 3.3 V SPI voltage in the current
  ordering table.
- The same guide gives a retail sample-order path. The N8R8 line is not marked
  EOL in the current table.

Source: [ESP32-S3-DevKitC-1 User Guide — Ordering Information and Retail
Orders](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp-dev-kits-en-master-esp32s3.pdf)
(checked 2026-11-10; lines 157–171 in the live HTML/PDF view).

This supports choosing the N8R8 board for a current design. It is not a claim
that the simulator models its flash/PSRAM protocol: this narrow profile models
the firmware-visible 8 MB flash capacity and the exercised I2C0 path only.
