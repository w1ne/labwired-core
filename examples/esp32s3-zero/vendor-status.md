# Live vendor and availability check — Waveshare ESP32-S3-Zero

Checked live in Chrome on 2026-11-11 at 13:39 UTC.

- Waveshare's [official product page](https://www.waveshare.com/product/esp32-s3-zero.htm) showed Product 25081 at **$5.49–$7.99** with an active **Add to Cart** button. The version selector includes the 4 MB flash / 2 MB PSRAM board.
- Waveshare's [official product documentation](https://docs.waveshare.com/ESP32-S3-Zero) identifies the standard ESP32-S3-Zero as 4 MB flash and 2 MB PSRAM and maps its onboard WS2812 data line to **GPIO21**.
- The [official schematic](https://files.waveshare.com/wiki/ESP32-S3-Zero/ESP32-S3-Zero-Sch.pdf) connects the WS2812 DIN net to GPIO21.

This is an ESP32 family **#1 demand-rank (38%)** coverage correction, not a new MCU-family pick. The smoke checks UART0 output and decodes the onboard WS2812 frame. It does not exercise Wi-Fi, BLE, USB download/CDC, or PSRAM.
