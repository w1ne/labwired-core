# Hosted SPI clock verification

On 2026-11-10, the hosted `labwired_verify` ran the same firmware source on its
catalogued classic ESP32 board and returned `status: model_verified` for this
oracle:

```json
{"gpio_edges":[{"pin":"GPIO18","min_edges":16}]}
```

The verifier observed 16 GPIO18 transitions and stopped after the assertion
passed. This hosted catalog does not include the SPI SSD1306 component, so the
hosted result verifies the SPI clock waveform only. The repository-local smoke
attaches `oled-ssd1306-spi` to SPI3 and separately asserts the panel's full
128x8 top-page framebuffer region.

The hosted result reports one non-blocking model gap: no power net is routed,
so the MCU uses an ideal 3.3 V rail and power-on/brown-out reset are not
simulated. This is model evidence, not a physical-board run.
