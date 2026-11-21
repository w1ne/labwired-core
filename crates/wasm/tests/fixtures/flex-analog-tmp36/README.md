# Held-out formula-backed analog source in Flex

This portable workload exercises the registered [`tmp36` source model](../../../../../configs/devices/tmp36.yaml) through the generic Flex runtime. The model's declared input is temperature in `°C`; its output is millivolts. The saved manifest exposes those as named ports and includes the model's assumptions and unsupported checks in the run report.

Portable Wasm firmware reads the declared 25 °C default, then samples -40 °C, 0 °C, 25 °C and 125 °C. Native and Chromium browser observations include the existing oscilloscope trace at 0 ns and 1000 ns. Cold and hot cases are negative and positive controls around the default.

This checks the registered catalog equation as functional portable behavior. It does not execute original target firmware, emulate an MCU or ADC, verify sensor tolerance, or measure a physical part.

Reproduce from the repository root:

```sh
cargo test -p labwired-wasm --lib flex_tmp36 -- --nocapture
sh crates/wasm/tests/fixtures/flex-analog-tmp36/build.sh
wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
  node scripts/test_browser_flex.mjs --browser \
  --fixture crates/wasm/tests/fixtures/flex-analog-tmp36 \
  --check runTmp36AnalogSourceChecks
```
