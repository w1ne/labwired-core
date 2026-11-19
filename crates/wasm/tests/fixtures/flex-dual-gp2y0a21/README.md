# Two independent Sharp GP2Y0A21 sensors in one Flex project

This held-out workload instantiates the same registered, source-backed
`gp2y0a21` catalog model twice. One portable Wasm firmware module samples both
sensor ports, drives two independent indicators, and is executed by the existing
Flex browser runtime. The left and right sensor use independent distance inputs
and produce separate oscilloscope channels. The logic analyzer records both
indicators, including one-at-a-time positive controls and far-range negative
controls.

The model reuses Sharp's GP2Y0A21YK0F typical white-paper graph read-offs in
[Fig. 2](https://global.sharp/products/device/lineup/data/pdf/datasheet/gp2y0a21yk_e.pdf).
It does not model optical geometry, ambient light, cross-talk, device spread,
noise, ADC calibration, measurement cadence or output delay. The clamping rules
outside 10–80 cm are model assumptions. Original target firmware, MCU execution,
ADC peripheral fidelity, full-board behavior and physical measurements are not
established.

## Reproduce

```sh
cargo test -p labwired-wasm --lib flex_dual_sensor -- --nocapture
crates/wasm/tests/fixtures/flex-dual-gp2y0a21/build.sh
wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
  node scripts/test_browser_flex.mjs --browser \
  --fixture crates/wasm/tests/fixtures/flex-dual-gp2y0a21 \
  --check runDualGp2y0a21Checks
```
