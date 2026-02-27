# Core Agents Manual

This is the operating manual for AI coding agents working inside the `core` repository of LabWired.

## 1) Context

- The `core` directory contains the main simulation engine, CLI, DAP (Debug Adapter Protocol), configuration files, and testing infrastructure.
- All code in this repo is written in Rust and is designed to create a deterministic, fast, and testable simulation environment for various MCU protocols and peripherals.

## 2) Key Documentation Links

Start your learning and reference with these key files:
- [README.md](./README.md) - The main entrypoint.
- [ARCHITECTURE.md](./docs/ARCHITECTURE.md) - Core engine internals and architecture.
- [CONTRIBUTING.md](./CONTRIBUTING.md) - General connection and contributing guidelines.
- [CONTRIBUTING_PERIPHERALS.md](./docs/CONTRIBUTING_PERIPHERALS.md) - How to implement and integrate new peripheral models.
- [board_onboarding_playbook.md](./docs/board_onboarding_playbook.md) - Complete playbook for onboarding new boards and MSUs.
- [ci_test_runner.md](./docs/ci_test_runner.md) - Details on how CI validation works and is triggered.

## 3) Standard Development Commands

Run all of these commands from the `core` root directory (i.e. `/home/andrii/Projects/labwired/core`).

**Building and Testing:**
```bash
# Build the workspace excluding firmware cross-compilation crates
EXCLUDES="--exclude firmware-armv6m-hello --exclude firmware-stm32f103-blinky --exclude firmware-stm32f103-uart --exclude firmware-armv6m-ci-fixture --exclude firmware-armv7m-benchmark --exclude firmware-f401-demo --exclude firmware-h563-demo --exclude firmware-h563-fullchip-demo --exclude firmware-h563-io-demo --exclude firmware-hil-showcase --exclude firmware-nrf52832-demo --exclude firmware-rp2040-pio-onboarding --exclude firmware-rv32i-ci-fixture --exclude firmware-rv32i-hello"
cargo build --workspace $EXCLUDES
cargo test --workspace $EXCLUDES
```

**Linting and Formatting:**
```bash
# Verify the formatting
cargo fmt --all -- --check
# Run the linter
cargo clippy --workspace $EXCLUDES -- -D warnings
```

**Running the Simulator:**
```bash
cargo run -p labwired-cli -- --firmware path/to/firmware.elf --system path/to/system.yaml
```

**Testing Simulator Accuracy (Unsupported Instruction Audit):**
```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/<firmware-crate> \
  --system configs/systems/<board>.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/<board>
```

## 4) Best Practices for Agent Work in Core

1. **Verify State Proactively**: Before assuming a code fix works, compile it using the standard development commands and review `cargo test` results. Do not infer correctness without validation.
2. **Deterministic Outputs**: Ensure your peripheral and bus implementation logic respects the deterministic execution model. Avoid unpredictable or non-reproducible states.
3. **Follow the Onboarding SOP**: If onboarding a board, follow all phase gates in `docs/board_onboarding_playbook.md`. It mandates adding test fixtures, examples, config YAML files, and reporting.
4. **Scope Your Commits**: Keep tasks strictly scoped to their definition. Avoid speculative refactors or out-of-bounds improvements to stable core files if not requested.
