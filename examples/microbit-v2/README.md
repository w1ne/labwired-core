# micro:bit v2 Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for the BBC micro:bit v2
(Nordic nRF52833) using the minimal supported subset:

1. `uart0` — UARTE0 EasyDMA console bridged to the interface MCU (TX = P0.06)
2. `gpio0` / `gpio1` — buttons A (P0.14) and B (P0.23) are declared as inputs
3. `clock` — HFCLK/LFCLK behavioural model

The example and production system attach the row/column-multiplexed 5x5 LED
matrix. Radio/BLE stacks, USB protocols, speaker and microphone are not qualified.

**SIM-DERIVED.** Not silicon-verified. No executing-fidelity differential
exists for this part yet; the smoke proves the UARTE EasyDMA console path
end-to-end; the distinct source-built board guest below qualifies display and
button behavior, not on-board sensor protocols or electrical timing.

## Executable display and button proof

`board-io.S` is independently authored MIT firmware that scans a diagonal
across all five rows/columns using silicon P0/P1 addresses. It reads active-low
buttons into RAM `0x20000000` (A=1, B=2) and increments a completed-scan counter
at `0x20000004`. Host tests inject only physical button levels; pixels and
button receipts come from guest execution.

```sh
cargo test --release -p labwired-core --features microbit-board-io-test \
  --test microbit_v2_board_io_guest
cargo test --release -p labwired-core --features microbit-board-io-test \
  --test microbit_v2_board_io_guest active_display_button_workload_throughput \
  -- --ignored --nocapture
```

ARM GCC must be installed. The second command warms up for 8 million steps,
then measures five active display/button windows of 64 million steps each
(one modeled second at 64 MHz), using actual simulated cycles and elapsed wall
time. The longer windows reduce scheduling noise compared with the original
4-million-step samples. Scores from different hosts or window sizes are not
evidence of an engine optimization.
`LABWIRED_REQUIRE_REALTIME=1` enables a median >=1.0x gate; absence of that
environment variable records results without claiming real-time performance.
This is native event-scheduler throughput, not a browser-WASM measurement.

## Quick Run

```bash
cargo build -p firmware-nrf52833-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/microbit-v2/uart-smoke.yaml --output-dir out/microbit-v2/uart-smoke --no-uart-stdout
```

Expected result:

1. smoke test passes
2. UART contains `OK`

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `REQUIRED_DOCS.md`: source-grounding references (nRF52833 PS, micro:bit docs).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.
6. `KNOWN_LIMITATIONS.md`: the L2/L3 known-limitations boundary (proven at L3,
   partially modelled, not modelled, evidence gaps).
