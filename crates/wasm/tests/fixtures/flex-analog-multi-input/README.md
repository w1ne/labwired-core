# Reusable multi-input analog source in Flex

This held-out workload exercises the catalog's registered `lipo_charger` analog source through the generic Flex runtime. The source descriptor is [`configs/devices/lipo_charger.yaml`](../../../../../configs/devices/lipo_charger.yaml), in the pinned Core commit. It uses two inputs with different units: state of charge (`%`) and a dimensionless charger-present value (`1`).

The descriptor leaves `usb_present`'s catalog unit empty because it is dimensionless. Flex requires explicit numeric port units, so the existing analog-source adapter exposes that catalog input as conventional dimensionless `1`; no value conversion occurs. This also checks the model's declared defaults (50% and disconnected) and its existing `mV` output on the oscilloscope trace.

The portable Wasm program samples 50% state of charge with the charger connected and disconnected, then checks the 100% state-of-charge clamp. The native Flex result is the checked reference. This is functional simulation of the reusable catalog equation, not TP4056 controller, battery chemistry, physical safety, or target-MCU firmware simulation.

Reproduce from the repository root:

```sh
cargo test -p labwired-wasm --lib flex_lipo_charger -- --nocapture
sh crates/wasm/tests/fixtures/flex-analog-multi-input/build.sh
wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
  node scripts/test_browser_flex.mjs --browser \
  --fixture crates/wasm/tests/fixtures/flex-analog-multi-input \
  --check runMultiInputAnalogSourceChecks
```
