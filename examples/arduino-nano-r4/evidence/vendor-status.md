# Arduino Nano R4 — live vendor status (2026-11-14)

Both pages were read live on 2026-11-14 in a real Chrome tab (the Arduino
store returns HTTP 429 to plain HTTP clients).

| Item | Source | What the page said |
|---|---|---|
| Board | https://store.arduino.cc/products/nano-r4 | "Arduino® Nano R4", "SKU ABX00142", "€13,40", an "ADD TO CART" button. No retired/NRND notice. |
| MCU | https://www.renesas.com/en/products/ra4m1 | "RA4M1", "Active", "Product Longevity: 2041". |
| Board docs | https://docs.arduino.cc/hardware/nano-r4/ | Renesas RA4M1, 48 MHz Cortex-M4; no lifecycle notice. |

Pin map source: ArduinoCore-renesas 1.6.0 `variants/NANOR4`
(`pins_arduino.h`, `variant.cpp`, `pinmux.inc`): LED_BUILTIN = pin 22 = P204;
Serial1 = UART1_TX_PIN 1 / UART1_RX_PIN 0 = P302 / P301 = SCI2 TXD2 / RXD2.
