# Validation record

- Demand position: ESP32 family rank 1 (38%); no C61-specific survey share.
- Vendor check: exact ESP32-C61-DevKitC-1-N8R2 listed by Espressif and in stock
  at Mouser on 2026-11-17. Bosch lists the BME280 for 10 years of delivery;
  Adafruit's BME280 breakout listing showed it in stock.
- Firmware: bare-metal RV32IMAC image, fast-loaded at the C61 flash entry point.
- Test: firmware configures the GPIO6/GPIO22 I²C0 route, reads the BME280 ID
  (`0x60`) and raw-temperature vector (`0x082DC0`), prints a UART0 marker, then
  reaches the configured step limit.
- Scope gaps: C61 mask-ROM/bootloader, physical USB-UART, electrical IO_MUX,
  GPIO26..29, other GPIO-matrix routes, radio, PSRAM, GPIO8 RGB protocol,
  sensor conversion behavior, and physical board behavior are not verified.

Reproduction command: [`ci/test.sh`](ci/test.sh). Fresh generated logs and
structured simulator output are in [`evidence/run/`](evidence/run/).
