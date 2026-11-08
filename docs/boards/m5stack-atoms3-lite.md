# M5Stack AtomS3-Lite

| Field | Coverage |
|---|---|
| Board SKU | M5Stack AtomS3-Lite, C124 |
| MCU | ESP32-S3FN8, Xtensa LX7, 8 MB flash |
| Demand rank | ESP32 family rank 1 (38% in JetBrains State of Embedded 2025); exact SKU not separately ranked |
| Chip/system profile | [`esp32s3-atoms3-lite-smoke`](../../configs/chips/esp32s3-atoms3-lite-smoke.yaml) · [`m5stack-atoms3-lite`](../../configs/systems/m5stack-atoms3-lite.yaml) |
| Modeled board peripheral | WS2812C-2020 RGB LED on GPIO35, driven with GPIO timing |
| Smoke | [Firmware, copy-paste example, and evidence](../../examples/m5stack-atoms3-lite/README.md) |
| Vendor status | [Dated availability record](../../examples/m5stack-atoms3-lite/evidence/vendor-status.md) |

This narrow profile covers the RGB LED data path. The button, IR transmitter,
USB, radio, and connector peripherals are outside the model slice. The local
simulator smoke is not a physical-board test.
