# Live vendor availability — ESP32-S3-DevKitC-1-N16R8

Checked 2026-11-10 UTC.

- Espressif's live [DevKits catalog](https://www.espressif.com/products/devkits)
  lists **ESP32-S3-DevKitC-1-N16R8** and links the exact SKU to Mouser.
- Espressif's current [ESP32-S3-DevKitC-1 v1.1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
  lists N16R8 as an ordering variant and explicitly marks older N8 and N8R2
  variants EOL. N16R8 is not marked EOL.
- The same guide maps J1 pin 4 / GPIO4 to ADC1_CH3 and documents that the
  board's I/O is broken out for jumper-wire peripherals.

Demand rank: ESP32 family **#1** (38% reported MCU use; 42% ESP32 kits). This
round extends the existing DevKitC board path with an ADC input example; it is
not a new MCU or board demand pick. Espressif's catalog and current guide are
the live lifecycle and buyability evidence for the selected N16R8 SKU.
