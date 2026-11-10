# Onboarding Smoke: esp32-devkitc-32e-microsd

- Demand rank: ESP32 rank 1 (38% reported MCU use).
- Work type: coverage depth on the existing ESP32-DevKitC-32E, not a new board/MCU demand pick.
- Target: exact Espressif ESP32-DevKitC-32E / ESP32-WROOM-32E with Adafruit
  Micro SD SPI/SDIO breakout (PID 4682) and a separate SPI-capable card on
  VSPI/SPI3.
- Behavior: stock Arduino `SD.begin` mounts the read-only SDHC image;
  `SD.open("/README.TXT")` reads `LabWired SD card demo.\r\n`. GPIO17 marks a
  successful mount, and GPIO16 plus UART0 mark a matching file read.
- Repository-local smoke: `pass`, 3/3 assertions, 10,470,000 simulator steps.
  The committed run log, structured output, and build logs are in this
  directory.
- Hosted `labwired_verify`: `failed` for the filesystem smoke at the 225,000,000
  step limit; GPIO16 remained low. The hosted catalog describes its independent
  microSD model as initialization plus zero-filled CMD17 reads, without a
  filesystem. See [`hosted-verification.md`](hosted-verification.md).
- Limits: the local model uses a fixed 32 MiB read-only FAT16 image. Writes,
  custom card images, and physical board confirmation are not covered.
- Live availability: [`vendor-status.md`](vendor-status.md).
