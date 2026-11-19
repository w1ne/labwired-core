# Held-out NTC alarm workload in Flex

This portable workload uses the existing [`ntc-thermistor` analog source](../../../../../configs/devices/ntc_thermistor.yaml), which describes a 10 kΩ, B=3950 thermistor above a 10 kΩ pull-down divider. The exact beta equation and 25 °C midpoint are documented in that catalog model. Flex runs a newly compiled portable WebAssembly program that reads the resulting millivolts and drives a simple digital alarm above 2000 mV.

The proof exercises the existing oscilloscope and logic analyzer through their nanosecond trace APIs. Native and Chromium browser output are compared at 0 °C, 25 °C, and 50 °C. Cold and ambient inputs keep the alarm off; the hot input asserts it.

This is functional behavior against a reusable analog source, not original target firmware, target ADC fidelity, component tolerance, a physical measurement, or a full-board simulation.

The run report carries structured `execution_scope` fields: it marks the firmware as portable behavior, says the original target firmware and detailed target path were not executed, and leaves full-board coverage and chip parity unestablished.

Reproduce from the repository root:

```sh
cargo test -p labwired-wasm --lib flex_ntc -- --nocapture
sh crates/wasm/tests/fixtures/flex-analog-ntc/build.sh
wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
  node scripts/test_browser_flex.mjs --browser \
  --fixture crates/wasm/tests/fixtures/flex-analog-ntc \
  --check runNtcAnalogSourceChecks
```
