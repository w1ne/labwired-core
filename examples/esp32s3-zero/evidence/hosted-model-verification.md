# Hosted LabWired model-verification receipt

Checked 2026-11-11. This is simulator evidence, not a physical-board result.

- Submitted firmware: `firmware/src/main.cpp`, SHA-256 `0d30b9c0083e6ced7fed391469dcd8d445ee734d7bec8ce8c33beba8babaf920`.
- Board: `esp32-s3-zero`.
- Diagram: ESP32-S3-Zero plus a `neopixel` part; GPIO21 to DIN, 3V3 to VCC, and GND to GND.
- Oracle: UART marker `ESP32_S3_ZERO_GPIO21_WS2812_OK`; GPIO21 at least 48 transitions.
- Verdict: `model_verified`; both oracle clauses passed. The run observed 97 GPIO21 transitions and the WS2812 artifact decoded one lit pixel with GRB bytes `[64, 0, 0]` on data pin 21.
- Gap: the hosted diagram did not model a routed MCU power rail, so VDD was assumed ideal. Power-on and brownout behavior were not verified.
