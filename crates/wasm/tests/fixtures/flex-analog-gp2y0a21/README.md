# Held-out GP2Y0A21 Flex workload

This fixture runs portable Wasm firmware against the existing registered
`gp2y0a21` analog source and a generic digital indicator model. It does not
use a board-specific simulator path or execute target firmware.

## Source-backed curve correction

The source is Sharp's [GP2Y0A21YK0F datasheet](https://global.sharp/products/device/lineup/data/pdf/datasheet/gp2y0a21yk_e.pdf),
Fig. 2, the typical output graph for white paper with 90% reflectance. At
VCC=5 V, the characteristics table gives a typical 0.4 V output at 80 cm and
a typical 1.9 V difference between 10 cm and 80 cm. Those values anchor the
curve at 400 mV and 2300 mV. The remaining model points are rounded read-offs
from the typical graph, not guaranteed limits.

This corrects the prior catalog values, which had assigned the roughly 3.1 V
peak near 5–6 cm to 10 cm and had shifted the rest of the curve upward. The
fixture exercises all nine catalog points from 10 to 80 cm. It checks 2.3 V at
10 cm, 1.65 V at 15 cm, 1.3 V at 20 cm, and 0.4 V at 80 cm in the generic
runtime. The portable firmware turns on a modeled indicator at 1500 mV; the
logic analyzer records rising at 2000 ns and falling at 6000 ns.

The fixture declares the limits: reflectance and optical geometry, ambient
light, sensor variation, noise, ADC calibration, 38 ms measurement cadence,
and the output delay are not modeled. Below 10 cm the source graph rises to a
peak and then falls; the catalog clamps to its 10 cm endpoint. Beyond 80 cm it
holds the 80 cm endpoint. Those clamps are model assumptions outside Sharp's
10–80 cm measuring range.

## Runtime and instruments

`run_report()` identifies the `gp2y0a21` catalog model, its signal bindings,
portable behavior execution, time units, a direct link to Sharp's Fig. 2 source,
unsupported checks, and the exact model YAML digest. The existing oscilloscope
reports the named `distance_mv` channel in mV with nanosecond samples. The
existing logic analyzer reports the named `near_led` channel and nanosecond
transitions. Both captures are checked against the native Flex reference with
zero dropped samples.

This proves one functional portable firmware workload using the reusable
catalog source. It does not establish target-MCU fidelity, sensor measurement
timing, full-board coverage, chip parity, or physical behavior.

## Reproduce

```sh
cargo test -p labwired-wasm --lib flex_gp2y0a21 -- --nocapture
crates/wasm/tests/fixtures/flex-analog-gp2y0a21/build.sh
wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
  node scripts/test_browser_flex.mjs --browser \
  --fixture crates/wasm/tests/fixtures/flex-analog-gp2y0a21 \
  --check runGp2y0a21Checks
```
