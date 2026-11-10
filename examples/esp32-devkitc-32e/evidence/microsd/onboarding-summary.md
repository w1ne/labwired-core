# Onboarding Smoke: esp32-devkitc-32e-microsd

- Demand rank: ESP32 rank 1 (38% reported MCU use).
- Work type: coverage depth on the existing ESP32-DevKitC-32E; this is not a
  new board/MCU demand pick.
- Target: exact Espressif ESP32-DevKitC-32E / ESP32-WROOM-32E with Adafruit
  Micro SD SPI/SDIO breakout (PID 4682) and a separate SPI-capable card on
  VSPI/SPI3.
- Behavior: stock Arduino `SD.begin` mounts the seeded SDHC image;
  `SD.open("/README.TXT")` reads the expected demo bytes; firmware then creates
  `/ROUND54.TXT`, writes a marker, and reads it back. GPIO17 marks a successful
  mount; GPIO16 and UART0 mark the completed round trip.
- Repository-local smoke: `pass`, 3/3 assertions, 10,730,000 simulator steps.
  The committed run log, structured output, and build logs are in this
  directory.
- Hosted `labwired_verify`: `failed` at the 225,000,000-step limit; GPIO16
  remained low. The hosted catalog describes its independent microSD model as
  an SPI command shell whose CMD17 reads return zeros, without a filesystem.
  The repository-local model is the evidence for FAT16 read/write behavior;
  the hosted result is not a pass. See
  [`hosted-verification.md`](hosted-verification.md).
- Limits: the local model uses a fixed 32 MiB FAT16 image; single-block writes
  persist in simulation memory for one run, then the seeded image resets.
  Custom card images, flash wear, and physical board confirmation are not
  covered.
- Live availability: [`vendor-status.md`](vendor-status.md).
