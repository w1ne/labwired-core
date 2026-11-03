# Seeed Studio XIAO nRF54L15

This profile covers the standard, non-Sense XIAO nRF54L15 (SKU 101991421). It
boots a Cortex-M33 image and exercises the active-low on-board user LED on P2.00
through the nRF54L GPIO2 model. The smoke asserts GPIO output transitions.

The chip descriptor maps the XIAO D0–D15 header labels to their nRF54L15 GPIO
pads and exposes the user key on P0.00. It only declares GPIO behavior. UART,
I2C, SPI, ADC, the radio, and the battery measurement path are not part of this
profile. The board's SAMD11 USB/debug bridge, USB serial, charging circuitry,
antenna switch, and the Sense variant's IMU and microphone are also outside the
profile. No wireless or sensor behavior is claimed.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id xiao-nrf54l15 \
  --crate firmware-xiao-nrf54l15-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/xiao-nrf54l15/io-smoke.yaml \
  --system configs/systems/xiao-nrf54l15.yaml \
  --out-dir examples/xiao-nrf54l15/evidence
```

The committed build and simulator output is under `evidence/`. Seeed's live
stock check is in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- Seeed's [XIAO nRF54L15 guide](https://wiki.seeedstudio.com/xiao_nrf54l15_sense_getting_started/) maps D0–D15 to the nRF54L15 pads and lists the user key on P0.00 and user LED on P2.00. The [board schematic](https://files.seeedstudio.com/wiki/XIAO_nRF54L15/Getting_Start/nRF54L15_Schematic.pdf) shows the key pulled up and the LED driven active-low; the guide's example turns the LED on by driving it low.
- Nordic's [nRF54L15 product page](https://www.nordicsemi.com/Products/nRF54L15) describes the Cortex-M33 device and links its current documentation.
- The board-specific chip descriptor reuses the local `nrf54l` GPIO model and only exposes the three GPIO ports needed by the header and user IO. The local smoke asserts at least 16 output edges on GPIO2:0; the key mapping is declared but is not exercised in this smoke.

The run is digital-twin evidence, not a physical board capture.
