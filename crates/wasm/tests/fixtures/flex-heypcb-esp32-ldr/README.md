# HeyPCB ESP32-S3 LDR Flex behavior slice

This fixture exercises the generic Flex runtime with a portable Rust-to-Wasm behavior program and the existing catalog-backed `ldr` PeripheralKit. The pinned source board is **HeyPCB ESP32-S3 LDR - GL5528 photoresistor light sensor**, revision A, around ESP32-S3-WROOM-1-N16R8.

## Source and import evidence

The source is [HeyPCB/esp32-sensors-1 at immutable revision `57a2122a19ac1aa9c092e43fb5b123c461508752`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/ldr), licensed [CERN-OHL-S-2.0](https://github.com/HeyPCB/esp32-sensors-1/blob/57a2122a19ac1aa9c092e43fb5b123c461508752/LICENSE). Exact Git blob IDs for the schematic, PCB, `board.json`, README, and license are in [source-map.json](source-map.json). The source [README](https://github.com/HeyPCB/esp32-sensors-1/blob/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/ldr/README.md) states that the board has not been fabricated, assembled, or powered. The pinned board folder contains no original firmware for this design.

Live MCP discovery and imports are summarized in [mcp-import.json](mcp-import.json). Discovery found the source MCU configuration, its GPIO1 ADC function and GPIO48 pin, plus the existing `ldr` model backed by a reusable PeripheralKit. The generic schematic import returned 19 mapped, 1 ignored, and 2 dropped parts across 29 wire edges. In this direct hosted `labwired_import` result, the SNS_AN1 connection graph is present, but U4 GL5528(10-20) is mapped as a generic resistor with imported value `220`. The generic PCB import returned 17 mapped, 1 ignored, and 4 dropped parts across 22 wire edges. It also maps U4 as a resistor. Schematic gaps are U1 (USBLC6-2SC6) and U2 (AP2112K-3.3); PCB gaps are SW1, SW2, U1 and U2. Both imports report `twin_ready`; that status does not mean complete board behavior coverage. These are separate schematic and PCB imports, not a paired full-board runtime import.

The runtime source map uses the pinned source's U3.GPIO1 sensor net and U3.GPIO48/D1 user LED path. It replaces the imported resistor's behavior with the already-registered `ldr` PeripheralKit model; it adds no board-specific kit or importer. The reusable LDR model represents a typical 10 kΩ-at-10-lux CdS response and returns static divider millivolts. It does not model GL5528 production spread or source capacitor C7's transient smoothing. The LED output is logic state only. Other power, USB, boot/reset, RF, MCU and board behavior is outside this slice.

## What runs

The portable Wasm firmware applies dark (`0 lx`) and bright (`100000 lx`) stimuli, reads the model's source.U3.GPIO1 millivolts, computes a 12-bit ADC count, and drives source.U3.GPIO48 based on a half-scale threshold. The modeled D1 logic output is read back. The dark case is the negative control; the bright case checks LED on and off transitions. The existing oscilloscope trace now includes the registered LDR output as `source-u4-gl5528.voltage_mv` in `mV`, sampled at its 1 µs model boundaries; the existing logic-analyzer trace carries the named digital signals in nanoseconds. Native Flex observations and the actual browser Wasm run are recorded in [native.json](native.json) and [browser.json](browser.json).

The oscilloscope samples show the catalog lookup output. They do not model a GL5528 transient response or C7's analog smoothing; the board's other power, USB, boot/reset, RF, MCU and board behavior is outside this slice.

`WasmFlexRuntime.run_report()` returns the model name, named ports, model IDs/types, signal bindings and resolutions, declared assumptions and unsupported checks, and the selected `flex_portable_wasm` path with `target_cpu_emulated: false` and `time_unit: ns`. Older manifests without both scope lists report `scope_notes_complete: false`; an empty list is not presented as evidence that there are no gaps. The full YAML model, pinned source/license and firmware artifact hashes remain project provenance for the existing Library/Playground UI to display alongside this compact runtime report.

This proves the functional portable behavior program against the existing generic Flex runtime in the local ephemeral browser harness. It is not a public Library/Playground launch. It does **not** execute original ESP32 firmware, emulate ESP32-S3 instructions or ADC peripherals, prove chip parity, cover the full board, or provide physical hardware evidence. The source project had no board firmware to run.

## Reproduce

```sh
cargo test -p labwired-wasm --lib flex_heypcb_ldr -- --nocapture
cargo test -p labwired-wasm --lib error_boundary_ratchet -- --nocapture
crates/wasm/tests/fixtures/flex-heypcb-esp32-ldr/build.sh
wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
  node scripts/test_browser_flex.mjs --browser \
  --fixture crates/wasm/tests/fixtures/flex-heypcb-esp32-ldr \
  --check runHeypcbLdrChecks
```

## Measured onboarding steps

The README, source pins, live discovery, schematic import, PCB import, and runtime proof above are the reproducible steps for this board. A parallel GitHub fetch + LabWired discovery + two imports completed in 3.541s. On this lane, the first native compile for the targeted source-map/runtime tests took 2m13s including dependency compilation; those two tests executed in 0.09s. The latest full eight-test Flex suite run took 5.87s including a small test recompilation (0.06s test execution). The error-boundary ratchet completed in 0.11s (0.01s test execution). Building the portable module from `firmware.rs` took 0.06s. `wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web` took 2m26.78s, and the browser fixture completed in 6.153s in Chrome 156.0.8078.79 with Playwright 1.58.2.

These are lane measurements for reproducing this fixture, not a comparison of development effort. No comparable chip-specific Rust model onboarding run was measured, so no speed claim is made.

The updated `browser.json.measured_browser_ms` records the page-side WebAssembly initialization and positive/negative checks in the current Chromium run; it is not onboarding time.
