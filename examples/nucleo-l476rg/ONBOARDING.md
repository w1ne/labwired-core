# NUCLEO-L476RG onboarding smoke

This smoke uses the existing `stm32l476rg` chip model and the NUCLEO-L476RG
system mapping. Firmware boots, sends its marker through USART2 to the ST-LINK
Virtual COM Port, checks SPI1/I2C1/ADC1/DMA1 readiness, and toggles the onboard
LD2 LED on PA5. The firmware's UART output and PA5 edge count are asserted by
[`io-smoke.yaml`](io-smoke.yaml).

Run and refresh the committed evidence with:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-l476rg \
  --crate firmware-l476-demo \
  --target thumbv7em-none-eabihf \
  --script examples/nucleo-l476rg/io-smoke.yaml \
  --system configs/systems/nucleo-l476rg.yaml \
  --out-dir examples/nucleo-l476rg/evidence
```

The captured board path is USART2 PA2/PA3 and LD2 PA5. The chip descriptor
also declares broader STM32L476 blocks; SYSCFG is explicitly a `stub`, and
USB OTG, CAN, TSC and other blocks listed as not modelled in the existing
[board guide](README.md#whats-not-modelled) are outside this smoke's claims.
The smoke does not access SYSCFG or those unsupported board features.

An additional LabWired MCP oracle run checked the same USART2 marker and PA5
toggle with a small C fixture; its request and `status: model_verified` result
are saved in `evidence/mcp-oracle-check.json` and
`evidence/mcp-oracle-check.c`. That separate run assumes an ideal 3.3 V rail
because no power net was routed, so it does not verify power-on or brown-out
behavior. It is not the Rust binary used by the committed local smoke.

Vendor lifecycle and availability evidence is in
[`evidence/vendor-status.md`](evidence/vendor-status.md). Captured build and
simulation logs, UART output, simulator result, and onboarding metrics are
committed under [`evidence/`](evidence/).
