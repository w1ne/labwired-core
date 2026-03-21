[← Back to Hub](../README.md)

# Getting Started with LabWired

This tutorial walks you through installing LabWired, running your first firmware simulation, and inspecting the results.

## Prerequisites

- Rust toolchain (stable) — install via [rustup](https://rustup.rs/)
- ARM cross-compilation target: `rustup target add thumbv7m-none-eabi`
- Python 3.10+ (optional, for AI features)

## 1. Clone and Build

```bash
git clone https://github.com/labwired/labwired.git
cd labwired/core
cargo build -p labwired-cli --release
```

The binary is at `target/release/labwired`.

## 2. Run Your First Simulation

LabWired ships with example firmware and configs. Run the STM32F103 blinky demo:

```bash
cargo run -p labwired-cli -- test \
  --script examples/ci/uart-ok.yaml \
  --output-dir out/first-run \
  --no-uart-stdout
```

This runs unmodified firmware on a virtual STM32 and checks UART assertions.

## 3. Inspect Results

The output directory contains machine-readable artifacts:

```bash
ls out/first-run/
# result.json    — pass/fail status, cycles, assertions
# snapshot.json  — final CPU register state
# uart.log       — captured UART output
# junit.xml      — JUnit-compatible test report (for CI)
```

View the result:

```bash
cat out/first-run/result.json | python3 -m json.tool
```

```json
{
  "status": "pass",
  "stop_reason": "max_steps",
  "cycles": 4523,
  "instructions": 1000,
  "assertions": [
    {"assertion": "uart_contains OK", "passed": true}
  ]
}
```

## 4. Enable Instruction Tracing

Add `--trace` for cycle-level visibility:

```bash
cargo run -p labwired-cli -- test \
  --script examples/ci/uart-ok.yaml \
  --output-dir out/traced-run \
  --trace \
  --no-uart-stdout
```

This produces `trace.json` with per-instruction data (PC, opcode, register deltas, memory writes).

## 5. Generate VCD Waveforms

For signal-level analysis in GTKWave or PulseView:

```bash
cargo run -p labwired-cli -- test \
  --script examples/ci/uart-ok.yaml \
  --output-dir out/vcd-run \
  --vcd out/vcd-run/trace.vcd \
  --no-uart-stdout
```

Open `trace.vcd` in any VCD-compatible viewer.

## 6. Write Your Own Test Script

Create a YAML test script for your firmware:

```yaml
schema_version: "1.0"
inputs:
  firmware: "path/to/your/firmware.elf"
  system: "path/to/system.yaml"
limits:
  max_steps: 10000
  max_cycles: 1000000
  wall_time: 30
assertions:
  - uart_contains: "Hello"
  - expected_stop_reason: max_steps
```

Run it:

```bash
labwired test --script my-test.yaml --output-dir out/my-test
```

## 7. Debug in VS Code

Install the LabWired VS Code extension, then:

1. Open your firmware project in VS Code
2. Run **LabWired: Launch Config Wizard** from the command palette
3. Select your ELF binary and system config
4. Press F5 to start debugging

You get breakpoints, register inspection, memory views, timeline visualization, and UART output — all without physical hardware.

## Next Steps

- [CI Integration Guide](./ci-integration.md) — run LabWired in GitHub Actions
- [NUCLEO-H563ZI Demo](../guides/NUCLEO_H563ZI_DEMO.md) — advanced multi-peripheral example
- [Compatibility Matrix](../specs/compatibility_matrix.md) — supported chips and peripherals
