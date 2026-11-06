# Live vendor status and availability

Checked 2026-11-06 UTC for Heltec WiFi LoRa 32 V4.3, selecting the OLED,
ESP32-S3R2, 863–870 MHz, and China warehouse options.

- Heltec's [live product listing](https://heltec.org/project/wifi-lora-32-v4/?attribute_display=OLED&attribute_mcu=ESP32-S3R2&attribute_pa_band=863-870mhz&attribute_warehouse=CN+%28China%29+Warehouse) identifies the product as V4.3. Its selected S3R2/OLED variation (SKU `ZC-310-1-CN`) returned `is_in_stock: true`, `is_purchasable: true`, and a $19.90 price, with Add to cart available. The selected warehouse row did not give a stock count; the same page listed a separate S3R2/OLED 902–928 MHz US variation with 12 in stock.
- Heltec's [V4 documentation](https://wiki.heltec.org/docs/devices/open-source-hardware/esp32-series/lora-32/wifi-lora-32-v4/) says the board uses an ESP32-S3R2 and SX1262, with 16 MB flash and 2 MB PSRAM. Its V4 pin table maps the built-in LED to GPIO35 (GPIO46 is the R8 variant).
- Heltec's [V4.3 schematic](https://resource.heltec.cn/download/WiFi_LoRa_32_V4/Schematic/HTIT-WB32LAF_V4.3.pdf) shows the GPIO-controlled white LED (LED3) with a 330 Ω series resistor to ground, confirming active-high polarity. Heltec's [V4.2 datasheet](https://resource.heltec.cn/download/WiFi_LoRa_32_V4/datasheet/WiFi_LoRa_32_V4.2.0.pdf) identifies the OLED controller as SSD1315, with OLED SDA on GPIO17, SCL on GPIO18, and reset on GPIO21. The SSD1315 is outside this smoke profile because the engine does not yet have that model.

Demand rank: ESP32 family **#1** (38% MCU use in the standing survey). This is
current board coverage in the highest-demand MCU family, not a new chip. The
survey does not rank this exact board.
