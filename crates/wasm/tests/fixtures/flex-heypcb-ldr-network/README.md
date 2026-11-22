# HeyPCB ESP32-S3 LDR sensor network in Flex

This fixture solves the light-sensor network of the pinned **HeyPCB ESP32-S3 LDR** board, revision A, in the existing in-core analog engine. Portable Rust-to-Wasm firmware reads the SNS_AN1 voltage on the board's GPIO1 endpoint and drives the IO48 user LED. It extends the sibling [`flex-heypcb-esp32-ldr`](../flex-heypcb-esp32-ldr/README.md) slice, which injects millivolts from the generic `ldr` kit and does not model U4, R6, or C7.

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a19ac1aa9c092e43fb5b123c461508752`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/ldr), asset `boards/ldr/kicad/esp32_ldr`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs. A regression checks them against the sibling fixture's source map.
- Network: the pad nets come from the pinned PCB blob and the values from the pinned BOM blob. U4 (GL5528(10-20), LCSC C10081) connects 3V3 to SNS_AN1. R6 (10 kΩ) and C7 (100 nF) connect SNS_AN1 to GND. SNS_AN1 goes to U3 GPIO1. A regression compares each netlist line with these nets.
- Photoresistor data: the GL5528 row of the *GL55 Series CdS Photoresistor Manual* (Senba Optical & Electronic), PDF SHA-256 `99b0c494c8695f36461cadaaee50dcc91fba7d6c2f782f323e190e456458b230`. The row gives 10-20 kΩ at 10 lx, γ = 0.6 between 10 lx and 100 lx, 1 MΩ minimum dark resistance, and 20 ms / 30 ms response time.

The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

## Model

```
V3V3 3v3 0 dc 3.3
RU4 3v3 sns_an1 15k      routed: R = 15 kΩ * (lux / 10)^-0.6, 1..100 lx
R6 sns_an1 0 10k
C7 sns_an1 0 100n
```

`config.photoresistors` is a generic key of the `analog` co-simulation adapter. It binds a numeric lux input to one netlist resistor with a bounded power law. The manifest supplies every coefficient and the valid range. No code in this repository names the board or the part.

Assumptions and limits, also carried in `run_report()`:

- R10 is 15 kΩ, the midpoint of the manual's 10-20 kΩ range. The manual gives no typical value. The native regression also runs the 10 kΩ and 20 kΩ corners.
- The manual defines γ between 10 lx and 100 lx. The 1-10 lx part of the range follows the straight band of the manual's Fig. 3 and is an assumption.
- Input below 1 lx or above 100 lx stops the run with `photoresistor illuminance must be within 1..100 lx`. Darkness has no modeled reading.
- The modeled transient is C7 charging through R6 in parallel with the static U4 resistance. The GL5528's own 20 ms / 30 ms response, light history, spectral response, and temperature dependence are not modeled.
- 3V3 is an ideal source. GPIO1 is an ideal probe. No ADC conversion is modeled; the firmware computes a 12-bit count from volts.
- The LCSC listing for C10081 named JCHL with 8-20 kΩ and γ 0.65 on 2026-11-19. Those seller values differ from the manual and are not used. LabWired Part Knowledge returned `NOT_FOUND` for GL5528 on the same date.
- The generic importer does not produce this network. The schematic import drops U4 and the PCB import maps it as a fixed resistor. The netlist was written from the pinned source nets.

## Measured result

Native reference ([native.json](native.json)), 50 µs analog step, 4 ms settle:

| Case | Lux | SNS_AN1 after 4 ms | Divider formula | ADC count | LED edges |
| --- | --- | --- | --- | --- | --- |
| Start | 10 | 1.3200 V | 1.3200 V | - | - |
| Bright | 100 | 2.3969 V | 2.3969 V | 2974 | 4 |
| Dim (negative control) | 1 | 0.4819 V | 0.4734 V | 597 | 0 |

- The dim case has about 4.7 time constants in 4 ms, so about 1 % of the step remains.
- The oscilloscope trace holds 81 samples per case in volts with the `sensor_v` port alias and zero dropped samples. The bright step reaches 63.2 % at the 350 µs sample. The expected value is 50 µs input latch plus 274 µs (100 nF × 2.737 kΩ).
- The logic analyzer shows the bright LED edges at 4,000,000 ns, 4,001,000 ns (two), and 4,002,000 ns.
- At the 10 kΩ and 20 kΩ corners the bright case stays above half scale and the dim case stays below it.
- 0 lx, 0.5 lx, and 100,000 lx stop the run at time 0 with the range error. A stopped run answers no further read or advance.

Browser run ([browser.json](browser.json)): Chromium 156.0.8078.79 with Playwright ran the compiled `firmware.wasm` against the web Wasm runtime built from Core commit `3c3d63480`. Both cases equal the native reference sample for sample, the 63.2 % crossing is at 350,000 ns, and 0 lx and 100,000 lx stop the run with the range error. Page-side Wasm initialization and checks took 276.2 ms. This time excludes compilation and onboarding.

Command-line run: `labwired test --script test-bright.yaml` and `test-dim.yaml` run the same `system.yaml` and the same `firmware.wasm` bytes through the existing test runner, with `inputs.flex` as the explicit choice of portable execution. `crates/cli/tests/flex_test_runner.rs` checks that the digital edges and the 81 analog samples of both cases equal the native reference, and that the dim case fails the bright assertions.

Time-constant check (2026-11-19): the scripts assert the time at which SNS_AN1 passes 63.2 % of its step with `analog_crossings`. Both times were calculated from the pinned values before the run. Bright: U4 is 3.77 kΩ at 100 lx, the time constant with R6 and C7 is 274 µs, and with the 50 µs input latch the first sample at or above 2.001 V is at 350,000 ns. Dim: U4 is 59.7 kΩ at 1 lx, the time constant is 857 µs, and the first sample at or below 0.785 V is at 950,000 ns. The run gave both times. A crossing time is a sample time (50 µs step); the runner does not interpolate. `analog_window` asserts that the node stays between 2.395 V and 2.398 V after 2 ms in bright light.

## Scope

This is functional portable behavior against a source-pinned analog sub-network. It does not execute ESP32-S3 firmware, emulate target instructions or the ADC peripheral, cover the full board, establish chip parity, or provide physical-hardware evidence. It is not a Library or Playground launch.

## Reproduce

```sh
cargo test -p labwired-core --test analog -- photoresistor
cargo test -p labwired-config --lib flex
cargo test -p labwired-wasm --lib flex_heypcb_ldr_network -- --nocapture
cargo test -p labwired-cli --test flex_test_runner
cargo run -p labwired-cli --bin labwired -- test --no-key \
  --script crates/wasm/tests/fixtures/flex-heypcb-ldr-network/test-bright.yaml \
  --output-dir /tmp/flex-ldr-bright --vcd /tmp/flex-ldr-bright/digital.vcd \
  --analog-trace /tmp/flex-ldr-bright/analog.csv
crates/wasm/tests/fixtures/flex-heypcb-ldr-network/build.sh
wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
  node scripts/test_browser_flex.mjs --browser \
  --fixture crates/wasm/tests/fixtures/flex-heypcb-ldr-network \
  --check runLdrNetworkChecks
```
