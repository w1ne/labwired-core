# SSD1306 0.91″ 128×32 OLED lab

Wires a 0.91-inch **128×32** SSD1306 OLED module to the STM32F103 `i2c1` bus at
address `0x3C`. Same controller and command set as the 0.96″ 128×64 panel — the
model just tracks a 4-page (512-byte) GDDRAM framebuffer instead of 8 pages.

The playground surfaces the panel's pixels through the WASM display bridge, so
firmware that draws to the SSD1306 renders verbatim in the browser.
