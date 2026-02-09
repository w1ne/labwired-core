# LabWired Firmware Simulation Platform

> A cloud-native, high-performance, standalone firmware simulator for ARM Cortex-M microcontrollers.

## 📖 Overview
LabWired is a next-generation simulation platform designed to bridge the gap between hardware dependency and software velocity. It enables developers to run, debug, and test firmware binaries without physical hardware, leveraging a portable Rust-based execution engine.

**Key Features:**
- **Declarative Configuration**: Define Chips and Boards in YAML (including memory maps and peripherals).
- **Declarative Register Maps**: Model new peripherals via YAML descriptors without writing Rust code.
- **System Services**: Full support for SysTick, Vector Table Boot, and Exception Handling.
- **Core Peripheral Ecosystem**: STM32F1-compatible GPIO, RCC, Timers, I2C, and SPI models.
- **Interactive Debugging**: Standard DAP support with DWARF-based source mapping for VS Code.
- **CI Test Runner**: Deterministic `labwired test` mode with YAML scripts, JSON/JUnit outputs, and artifact bundles.
- **Functional Stubbing**: Mock external sensors and devices without complex emulation.
- **High Performance**: Native Rust implementation (`labwired-core`).
- **HAL Compatible**: Supports running binaries built with standard `stm32f1xx-hal` (see `crates/firmware-hal-test`).

## 🏗 Architecture
The project is organized as a Rust Workspace:

- **`crates/cli`**: The command-line interface entry point.
- **`crates/config`**: YAML-based hardware and project descriptors.
- **`crates/loader`**: ELF binary parsing and symbol resolution.
- **`crates/core`**: The execution engine (CPU, Dynamic Bus, Peripherals).
- **`crates/dap`**: Debug Adapter Protocol (DAP) server.
- **`editors/vscode`**: LabWired VS Code extension.

See [Architecture Documentation](docs/architecture.md) for details.

## 🚀 Getting Started

### Prerequisites
- **Rust**: Latest stable toolchain (1.75+).
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

### Building
```bash
# Build all crates
cargo build

# Run tests
cargo test
```

### Running the Simulator (Firmware Mode)

**1. Install ARM Target**
The firmware is built for the `thumbv7m-none-eabi` target (Cortex-M3).
```bash
rustup target add thumbv7m-none-eabi
```

**2. Build the Firmware**
Compile the demo application located in `crates/firmware`.
```bash
cargo build --release -p firmware --target thumbv7m-none-eabi
```

**3. Run the Simulator**
Pass the path to the firmware and the **System Manifest** defining the hardware.
```bash
# Run with prototype STM32F103 configuration
cargo run -p labwired-cli -- --firmware target/thumbv7m-none-eabi/release/firmware --system system.yaml
```

**Expected Output:**
```text
INFO labwired: Starting LabWired Simulator
INFO labwired: Loading system manifest: "system.yaml"
INFO labwired: Loading chip descriptor: "configs/chips/stm32f103.yaml"
INFO labwired: Loading firmware: "..."
INFO labwired: Firmware Loaded Successfully!
INFO labwired: Entry Point: 0x8000000
INFO labwired: Starting Simulation...
INFO labwired: Initial PC: 0x8000000, SP: 0x20002000
INFO labwired: Running for 20000 steps...
INFO labwired: Simulation loop finished (demo).
INFO labwired: Final PC: 0x8000010
INFO labwired: Total Instructions: 1540
INFO labwired: Total Cycles: 1540
INFO labwired: Average IPS: 125432.12
```

### CI-Friendly Test Runner (`labwired test`)

Use the deterministic runner mode to drive simulations from a YAML test script and emit machine-readable artifacts:

```bash
cargo build --release -p labwired-cli
./target/release/labwired test --script examples/ci/uart-ok.yaml --output-dir out/artifacts --no-uart-stdout
```

See `docs/ci_test_runner.md` for schema, exit codes, and artifact formats.

## 🔄 CI Integration

LabWired integrates seamlessly into your CI/CD pipeline, replacing physical hardware with deterministic simulation.

### Quick Start

**GitHub Actions:**
```yaml
- uses: w1ne/labwired/.github/actions/labwired-test@main
  with:
    script: tests/firmware-test.yaml
    output_dir: test-results
```

**GitLab CI:**
```yaml
test:
  script:
    - labwired test --script tests/firmware-test.yaml --output-dir results
  artifacts:
    reports:
      junit: results/junit.xml
```

**Docker (when published):**
```bash
docker run --rm -v $PWD:/workspace ghcr.io/w1ne/labwired:latest \
  test --script tests/firmware-test.yaml
```

### Resources

- **[CI Integration Guide](docs/ci_integration.md)** - Complete setup instructions
- **[Workflow Templates](examples/workflows/)** - Ready-to-use GitHub Actions & GitLab CI templates
- **[Test Examples](examples/ci/)** - Sample test scripts

### Benefits

- ✅ No physical hardware required in CI
- ✅ Deterministic, reproducible results
- ✅ Parallel testing across multiple targets
- ✅ Fast feedback (no flashing delays)
- ✅ Fault injection testing support


## 🤝 Development Workflow
We follow **Gitflow** and enforce strict quality gates.

- **Main Branch**: `main` (Production tags only).
- **Development**: `develop` (Feature integration).
- **Feature Branches**: `feature/xyz`.

**Quality Gates:**
- All PRs must pass CI (Format, Lint, Test, Audit).
- Code coverage goal: >80%.

See [Release & Merging Strategy](docs/release_strategy.md) for the full protocol.

## 📄 Documentation
- [Implementation Plan](docs/plan.md)
- [Architecture](docs/architecture.md)
- [Release Strategy](docs/release_strategy.md)
- [CI Integration Guide](docs/ci_integration.md)
- [Interactive Debugging](docs/debugging.md)


## ⚖️ License
MIT
