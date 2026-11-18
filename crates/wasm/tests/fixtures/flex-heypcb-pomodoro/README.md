# HeyPCB Pomodoro Pal Flex behavior slice

This fixture exercises the existing CPU-free Flex runtime and declarative behavior adapter against a source-grounded part of the pinned Pomodoro Pal schematic. It is a new portable WASM behavior program, not the repository's original CH32V003 HEX.

The exact source is [HeyPCB Pomodoro Pal at revision `1d6e50ff9998978e6f4521895042ae240fff7c42`](https://github.com/HeyPCB/pomodoro-pal/tree/1d6e50ff9998978e6f4521895042ae240fff7c42), whose repository declares [CERN-OHL-S-2.0](https://github.com/HeyPCB/pomodoro-pal/blob/1d6e50ff9998978e6f4521895042ae240fff7c42/LICENSE). File blob IDs and inspected generic `labwired_import` output are recorded in `source-map.json` and `mcp-import.json`. A fresh pinned import returned 36 mapped parts from 41 schematic references and 51 wire edges, with five ignored connector/test-point references and no dropped parts. The separate PCB import returned 35 mapped parts, one dropped/design-only part (BZ1), five ignored references, and a 41-component/42-net board file. Its geometry has a 69.6 × 66.39 mm outline, zero cutouts and two holes. These are two imports of files at the same immutable project revision; they are not represented as a paired, full-board runtime import.

The imported schematic connects U1.PD6 to R5.1 and ties R5.1 to R6.1; R5.2 and R6.2 connect to D1.A and D2.A. The PCB file names that shared trace `LEAF` and connects it to footprint pad U1.3, R5.1 and R6.1. The schematic also ties D1.K and D2.K to BZ1's negative pin. The Flex fixture maps the GPIO signal and two LED logic-state outputs through `FlexManifest` ports and the existing `CosimRunner` behavior engine; it does not model current through the resistors, LED physics, BZ1, or that shared return path.

The observation is limited to digital logic state. The LabWired catalog describes generic `led` as `netlist-only`; this fixture does not claim LED current, optical output, analog fidelity, full-board coverage, or physical hardware behavior. Discovery lists a 22-pin `ch32v003f4p6` catalog model and its PD6 entry, while the fresh Part Knowledge lookup for that exact name returned `NOT_FOUND`; the model inventory is not vendor-backed Part Knowledge. Part Knowledge also returned `NOT_FOUND` for the source buzzer MPN `PKMCS0909E4000`; the schematic maps it to a generic buzzer, but the PCB importer marks BZ1 design-only and it is not simulated. The fresh `labwired_validate` call processed the imported schematic and returned one error (`BOARDIO_NOT_TO_MCU` for D2) and two warnings (`COMPONENT_DANGLING` for D2 and `BOARD_NOT_RUNNABLE` for the MCU); the board has no hosted compile/run lane, so this is not a validation pass. The original HEX's prior 746-cycle stop at unmapped `0xe000f000` remains an unsupported original-firmware result.

The source import is not currently a saved LabWired project. `labwired_project_list(q: "Pomodoro")` returned no projects. The saved board-project flow uses a board, diagram and firmware, and invokes hosted-twin verification during creation. This fixture runs portable Wasm through Flex and supplies no target-board firmware to that flow, so no saved board project exists for this slice. The browser Library owner retains the paired-source importer and browser launch flow.

`browser.json` records the actual Chromium run: portable Wasm returned `status=passed`, emitted six positive logic edges and zero negative-control edges in Chromium 149.0.7827.55. The test completed by matching the browser observations to `native.json`.

## Reproduce

```sh
cargo test -p labwired-wasm --lib flex_heypcb_pomodoro -- --nocapture
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
- This round's pinned-import graph and native behavior tests: 8.96 s including compilation; both tests executed in 0.00 s.
- Portable firmware module build: 0.18 s.
- First `wasm-pack` browser-runtime build: 5m56s, including the initial Wasm dependency build.
- This round's provenance-bound native tests: 5.89 s on the first run and 9.54 s on the final rerun, including compilation; both tests executed in 0.00 s.
- This round's warm `wasm-pack` browser-runtime build: 12.30 s.
- Chromium fixture run: 2.46 s in Chromium 149.0.7827.55 with Playwright 1.58.2; the browser returned six positive logic edges and zero negative-control edges.

These are reproducible lane measurements for this fixture, not a speed comparison. No comparable CH32V003 Rust-model onboarding run was measured, so no speed claim is made.
