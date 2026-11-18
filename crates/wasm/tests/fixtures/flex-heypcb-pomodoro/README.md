# HeyPCB Pomodoro Pal Flex behavior slice

This fixture exercises the existing CPU-free Flex runtime and declarative behavior adapter against a source-grounded part of the pinned Pomodoro Pal schematic. It is a new portable WASM behavior program, not the repository's original CH32V003 HEX.

The exact source is [HeyPCB Pomodoro Pal at revision `1d6e50ff9998978e6f4521895042ae240fff7c42`](https://github.com/HeyPCB/pomodoro-pal/tree/1d6e50ff9998978e6f4521895042ae240fff7c42), whose repository declares [CERN-OHL-S-2.0](https://github.com/HeyPCB/pomodoro-pal/blob/1d6e50ff9998978e6f4521895042ae240fff7c42/LICENSE). File blob IDs and generic `labwired_import` results are recorded in `source-map.json`. The schematic import returned 36 mapped parts from 41 references and 51 connections, with five ignored connector/test-point references and no dropped parts. The separate PCB import returned 35 mapped parts, one dropped/design-only part (BZ1), five ignored references, and a 41-component/42-net board file. It reports a 69.6 × 66.39 mm outline, zero cutouts and two holes. These are two imports of files at the same immutable project revision; they are not represented as a paired, full-board runtime import.

The schematic maps U1.PD6 through R5/R6 to the anodes of green LEDs D1/D2. The PCB file names that shared trace `LEAF` and connects it to footprint pad U1.3, R5.1 and R6.1. The fixture exposes that source-derived signal and two logic-state outputs through `FlexManifest` ports and the existing `CosimRunner` behavior engine.

The observation is limited to digital logic state. The LabWired catalog describes generic `led` as `netlist-only`; this fixture does not claim LED current, optical output, analog fidelity, full-board coverage, or physical hardware behavior. Discovery reports a 22-pin `ch32v003f4p6` model, while Part Knowledge returned `MODEL_ONLY` for its pin data, not vendor-backed silicon evidence. Part Knowledge returned `NOT_FOUND` for the source buzzer MPN `PKMCS0909E4000`, and the PCB importer marks BZ1 design-only, so the buzzer is not simulated. The original HEX's prior 746-cycle stop at unmapped `0xe000f000` remains an unsupported original-firmware result. `labwired_validate` currently rejects the imported board before validation because its tool schema does not admit `ch32v003f4p6`; that is not a validation pass.

The source import is not currently a saved LabWired project. `labwired_project_list(q: "Pomodoro")` returned no projects. The browser Library owner retains the paired-source importer and browser launch flow.

`browser.json` records the actual Chromium run: portable Wasm returned `status=passed`, emitted six positive logic edges and zero negative-control edges in Chromium 156.0.8078.79. The test completed by matching the browser observations to `native.json`.

## Reproduce

```sh
cargo test -p labwired-wasm --lib flex_heypcb_pomodoro_source_mapped_pd6_pair_has_native_reference_and_negative_control -- --nocapture
crates/wasm/tests/fixtures/flex-heypcb-pomodoro/build.sh
wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
  node scripts/test_browser_flex.mjs --browser \
  --fixture crates/wasm/tests/fixtures/flex-heypcb-pomodoro \
  --check runPomodoroPd6Checks
```

The test compares the compiled portable firmware's positive pulse and inactive negative control with a native `FlexRuntime` reference. It makes no claim about executing the original firmware, CH32V003 instruction/peripheral fidelity, chip parity, or complete-board behavior.

## Measured effort

- First native test build: 41.86 s; the cached native reference rerun: 0.35 s.
- Portable firmware module build: 0.18 s.
- First `wasm-pack` browser-runtime build: 5m56s, including the initial Wasm dependency build.
- Chromium fixture run: about 1.4 s.

These are reproducible lane measurements for this fixture, not a speed comparison. No comparable CH32V003 Rust-model onboarding run was measured, so no speed claim is made.
