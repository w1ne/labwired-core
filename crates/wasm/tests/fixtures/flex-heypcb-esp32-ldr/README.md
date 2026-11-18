# HeyPCB ESP32-S3 LDR Flex behavior slice

This fixture exercises the generic Flex runtime with a portable Rust-to-Wasm behavior program and the existing catalog-backed `ldr` PeripheralKit. The pinned source board is **HeyPCB ESP32-S3 LDR - GL5528 photoresistor light sensor**, revision A, around ESP32-S3-WROOM-1-N16R8.

## Source and import evidence

The source is [HeyPCB/esp32-sensors-1 at immutable revision `57a2122a19ac1aa9c092e43fb5b123c461508752`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/ldr), licensed [CERN-OHL-S-2.0](https://github.com/HeyPCB/esp32-sensors-1/blob/57a2122a19ac1aa9c092e43fb5b123c461508752/LICENSE). Exact Git blob IDs for the schematic, PCB, `board.json`, README, and license are in [source-map.json](source-map.json). The source [README](https://github.com/HeyPCB/esp32-sensors-1/blob/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/ldr/README.md) states that the board has not been fabricated, assembled, or powered. The pinned board folder contains no original firmware for this design.

Live MCP discovery and imports are summarized in [mcp-import.json](mcp-import.json). Discovery found the source MCU configuration, its GPIO1 ADC function and GPIO48 pin, plus the existing `ldr` model backed by a reusable PeripheralKit. The current generic schematic import returned 18 mapped, 1 ignored, and 3 dropped parts across 27 wire edges. It drops U4 GL5528(10-20) with an explicit gap: two-terminal, lux-dependent conductance is not modeled. Its imported graph retains C7.1-to-U3.GPIO1 and C7.1-to-R6.1, but has no U4 pin or edge. The current PCB import returned 17 mapped, 1 ignored, and 4 dropped parts across 22 wire edges. It maps U4 as a fixed resistor without reporting that as a model gap; the source part remains a GL5528 and the resistor mapping is not an equivalent behavior model. Other gaps include U1 (USBLC6-2SC6), U2 (AP2112K-3.3), and, on the PCB import, SW1/SW2. Both imports report `twin_ready`; that status does not mean complete board behavior coverage. These are separate schematic and PCB imports, not a paired full-board runtime import.

The Flex manifest is a separate, source-aligned behavior slice built from the pinned net names; it is not the imported board diagram. The portable firmware reads `source.U3.GPIO1` and drives `source.U3.GPIO48`, while the existing `ldr` PeripheralKit provides a generic illuminance-to-millivolt output at the GPIO1 endpoint. This is an explicit voltage-behavior approximation. It does not model U4 as a two-terminal photoconductive device, the R6 divider, or C7 transient behavior, and it does not resolve the importer gap. Public verified Part Knowledge returned `NOT_FOUND` for GL5528, so the model's typical 10 kΩ-at-10-lux CdS curve is an explicit generic assumption, not a GL5528-specific verified characteristic. The LED output is logic state only. Other power, USB, boot/reset, RF, MCU and board behavior is outside this slice.

## What runs

The portable Wasm firmware applies dark (`0 lx`) and bright (`100000 lx`) stimuli, reads the model's source.U3.GPIO1 millivolts, computes a 12-bit ADC count, and drives source.U3.GPIO48 based on a half-scale threshold. The modeled D1 logic output is read back. The dark case is the negative control; the bright case checks LED on and off transitions. The existing oscilloscope trace now includes the registered LDR output as `source-u4-gl5528.voltage_mv` in `mV`, sampled at its 1 µs model boundaries; both digital and analog snapshots label their timestamps as `ns`. Native Flex observations and the actual browser Wasm run are recorded in [native.json](native.json) and [browser.json](browser.json).

The oscilloscope samples show the generic LDR kit's modeled millivolt output at the selected signal. They do not show the GL5528's two-terminal conductance or the board's R6/C7 analog network. The board's other power, USB, boot/reset, RF, MCU and board behavior is outside this slice.

`WasmFlexRuntime.run_report()` returns the model name, named ports, model IDs/types, complete validated model configurations, signal bindings and resolutions, declared assumptions and unsupported checks, and the selected `flex_portable_wasm` path with `target_cpu_emulated: false` and `time_unit: ns`. It also returns `manifest_sha256`, the SHA-256 digest of the exact YAML bytes passed to `new_from_yaml()`, so the existing project UI can retain the precise model input with a run. Older manifests without both scope lists report `scope_notes_complete: false`; an empty list is not presented as evidence that there are no gaps. The browser evidence records the Core commit, pinned source revision and license, source-file blob IDs, and SHA-256 digests for the model manifest and portable firmware source/artifact so the observed run can be reproduced from the checked-in inputs.

## Existing Playground instrument handoff

The Library lane owns source import, project selection, and browser launch. It can use the existing Core Wasm surface without another importer or runtime: create `WasmFlexRuntime` with `new_from_yaml(system)`, instantiate the selected portable firmware with `labwired.read`, `labwired.write`, and `labwired.advance_ns`, then read `ports()` and `run_report()` for project scope. The report identifies `flex_portable_wasm`, `target_cpu_emulated: false`, nanosecond time, named read/write ports, the validated model configs, assumptions, and unsupported checks.

The existing instruments can poll `digital_trace_snapshot(cursor)` and `analog_trace_snapshot(cursor)` using their native safe-integer `Number` cursors. Digital batches expose `time_unit`, the imported signal path plus matching `port_names` aliases for firmware-facing Boolean ports, timestamped edges, `cursor`, and `dropped`; internal signals remain visible with an empty alias list. Analog batches expose `time_unit`, named channels, engineering units, numeric read-port aliases such as `sensor_mv`, timestamped samples, `next_cursor`, and `dropped`. Flex caps simulated time at the largest safe JavaScript integer. The host `advance_ns(delta)` accepts a nonnegative safe-integer `Number`, so a single UI-driven advance may exceed `u32`; the separate portable-firmware import remains a Wasm `u32` call. The Wasm boundary rejects negative, fractional, nonfinite, and unsafe deltas and cursors. The browser regression checks that LDR drive/state aliases resolve to their source-mapped signals, verifies the scope channel carries `sensor_mv`, validates timestamps and cursors, and verifies polling from a returned cursor neither repeats nor loses observations. Convert the batch field names to the existing instrument DTO at the Playground boundary.

The current browser proof still uses the checked-in fixture harness rather than a Library/Playground launch. The handoff exposes the functional portable behavior slice only; it does not upgrade the imported source to a full-board or chip-fidelity claim.

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

The README, source pins, live discovery, schematic import, PCB import, and behavior-slice proof above are the reproducible onboarding steps for this board. On the 18 Nov merge head `95bedb576a93996125faf3763e64c03d47d3c875`, the targeted native run `cargo test -p labwired-wasm --lib flex_heypcb_ldr -- --nocapture` passed 3/3; the first compile took 12m08s with concurrent shared Cargo builds, and the tests then executed in 0.06s. Its initial run caught a stale expected scope-channel record that omitted `sensor_mv`; the refreshed native reference and added browser assertion now keep that alias covered. `wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web` passed in 11m32s on the shared host. Chromium 156.0.8078.79 with Playwright 1.61.1 ran the checked-in portable module and both controls; `browser.json.measured_browser_ms` records 468.1ms for page-side Wasm initialization and checks. This timing excludes onboarding, compilation, and the surrounding application.

These are lane measurements for reproducing this fixture, not a comparison of development effort. The page-side WebAssembly timing is not the total onboarding time. No comparable chip-specific Rust model onboarding run was measured, so no speed claim is made.

On the run-report fingerprint commit `1024575c862962ce44d3f515fc045e4753e6042f`, `cargo test -p labwired-wasm --lib flex_heypcb_ldr -- --nocapture` passed 3/3. Chromium 156.0.8078.79 / Playwright 1.61.1 passed the same pinned portable firmware and controls; `browser.json` records the exact `9c78f66d7564bfb8027fea1515141b6e1273ca8918b168280915c82fdf1dacd8` model digest returned by `run_report()` and 643.2 ms page-side execution. This measurement is fixture-specific and excludes compile time and onboarding.

On Core commit `d8b62f413f387167ac8c261e904901daf3849c4e`, the targeted native test again passed 3/3. Chromium 156.0.8078.79 / Playwright 1.61.1 passed the pinned browser workload and a host API regression that advances by 4,294,967,296 ns in one call, above `u32`, while rejecting negative, fractional, nonfinite, and unsafe deltas. Bright output remained ADC 4087 with four named digital edges; the dark negative control remained ADC 0 with no edges. Analog samples retained the `sensor_mv` alias, nanosecond timestamps, and zero dropped samples. `browser.json` records this run and its 388.2 ms page-side timing; the measurement excludes compilation and onboarding.
