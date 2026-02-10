# LabWired Platform

> A complete firmware simulation and debugging platform for ARM Cortex-M and RISC-V microcontrollers.

## 📖 Overview
LabWired is a next-generation simulation platform designed to bridge the gap between hardware dependency and software velocity. It enables developers to run, debug, and test firmware binaries without physical hardware.

## 🏗️ Monorepo Structure

This repository contains three independent components:

### [`core/`](./core/) - The Emulator Engine
Rust-based simulation engine with:
- **Declarative Configuration**: YAML-based chip and peripheral definitions
- **Multi-Architecture**: ARM Cortex-M and RISC-V support
- **CI Test Runner**: Deterministic `labwired test` with JSON/JUnit outputs
- **GDB/DAP Servers**: Standard debugging protocols
- **High Performance**: Native Rust implementation

### [`vscode/`](./vscode/) - VS Code Extension
Professional debugging interface with:
- **Timeline View**: Execution history visualization
- **Register Inspector**: Peripheral register viewer with bit-field expansion
- **Memory View**: Hex/ASCII memory inspector
- **DAP Integration**: Seamless connection to the emulator

### [`ai/`](./ai/) - AI Asset Generation
Tools for automated peripheral model generation:
- **Datasheet Ingestion**: PDF → YAML conversion
- **Schematic Analysis**: Component extraction from images
- **Logic Synthesis**: Behavioral model generation

## 🚀 Quick Start

### Building the Core Emulator
```bash
cd core
cargo build --release
```

### Running a Simulation
```bash
cd core
cargo run -p labwired-cli -- --firmware path/to/firmware.elf --system system.yaml
```

### Development Setup
See [`DEVELOPMENT.md`](./DEVELOPMENT.md) for complete setup instructions for all three components.

## 📚 Documentation

- **[Core Emulator](./core/README.md)** - Detailed emulator documentation
- **[VS Code Extension](./vscode/README.md)** - Extension features and usage
- **[AI Tools](./ai/README.md)** - Asset generation tools
- **[Development Guide](./DEVELOPMENT.md)** - Contributing and building
- **[Platform Strategy](./docs/spec/)** - Business roadmaps and market analysis

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
