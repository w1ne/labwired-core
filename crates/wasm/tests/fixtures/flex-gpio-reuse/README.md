# Flex GPIO behavior reuse across two board descriptions

This fixture checks the same declarative gpio_device behavior on two distinct
hardware descriptions through the existing CPU-free Flex runtime. The native
test compares the parsed behavior blocks exactly, and the browser harness runs
the same portable Wasm module against both bindings.

| Binding | Hardware path | Workload |
|---|---|---|
| HeyPCB Pomodoro Pal | U1.PD6 to D1.logic_state and D2.logic_state | Asymmetric 1 µs + 2 µs pulses, held out from the single-pulse proof |
| Arduino Uno LabWired model | portb.pb5 to status_led.logic_state | Same held-out pulse pattern |

The Pomodoro identity, immutable source revision
HeyPCB/pomodoro-pal@1d6e50ff9998978e6f4521895042ae240fff7c42, declared
CERN-OHL-S-2.0 license, and generic MCP import evidence remain pinned in
../flex-heypcb-pomodoro/{README.md,source-map.json,mcp-import.json}. The
contract here is a narrower view of that source graph. It instantiates the
same data-driven `gpio_device` behavior twice, once for each imported LED logic
output, so the portable firmware must observe both source-mapped outputs.
The source import counted 36 mapped / 41 schematic references (51 wire
edges, 5 ignored, 0 dropped) and 35 mapped / 41 PCB components (5 ignored,
BZ1 dropped/design-only; source file 42 nets). This fixture binds one drive
port to both source-mapped LED logic outputs. BZ1, LED electrical/optical
behavior, the whole board, original HEX execution, and CH32V003 fidelity remain
uncovered.

The second binding uses the existing LabWired system description in
configs/systems/arduino-uno.yaml, which maps status_led to portb bit 5.
examples/arduino-uno-blinky/EXTERNAL_COMPONENTS.md labels that twin output
D13/PB5. The labwired_part read for arduino-uno returned MODEL_ONLY: this
fixture tests the LabWired model path and does not upgrade it to a vendor-backed
pin fact. Its tested slice has one write port, one read port, and one behavior
adapter; no new import or full-board inventory was produced. The original Uno
AVR firmware was not executed in Flex.

The portable firmware imports only labwired.read, labwired.write, and
labwired.advance_ns. Its asymmetric pulse workload runs on both contracts; the
positive result is compared to native FlexRuntime evidence, and a low-drive
run lasting 5 µs must leave all mapped outputs inactive. Pomodoro produces 12
edges: four each on PD6, D1 and D2. The single-output Uno contract produces 8
edges. Both negative controls emit zero edges, and both positive traces require
zero dropped events. Each adapter samples at its declared 1,000 ns step; those
timestamps are model scheduler boundaries, not a silicon latency claim.

This checks digital indicator logic at the declared model boundary. It does not
simulate LED current, optical output, board power, the original CH32V003 HEX,
Arduino AVR firmware, MCU instructions/peripherals, or full-board coverage. It
is not chip parity. No importer, catalog registry, simulator, orchestration,
MCP command/schema, or public tool is added.

## Reproduce

    cargo test -p labwired-wasm --lib flex_gpio_reuse -- --nocapture
    crates/wasm/tests/fixtures/flex-gpio-reuse/build.sh
    wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
    LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
      node scripts/test_browser_flex.mjs --browser \
      --fixture crates/wasm/tests/fixtures/flex-gpio-reuse \
      --check runGpioReuseChecks

Effort measurements and actual browser/runtime versions are recorded in
browser.json. These measurements cover this reusable behavior binding only;
no comparable chip-specific Rust onboarding task was measured, so no speed
claim is made.
