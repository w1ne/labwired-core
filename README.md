# LabWired Core - Firmware Simulation Engine


> High-performance, declarative firmware simulator for ARM Cortex-M and RISC-V microcontrollers.

[![Documentation](https://img.shields.io/badge/docs-latest-blue.svg)](https://labwired.com/docs/)

## CI Dashboard

### Merge Gate

- `core-integrity` (required on PRs to `main`): ![Core Integrity](https://github.com/w1ne/labwired-core/actions/workflows/core-ci.yml/badge.svg?branch=main)  
[Workflow](https://github.com/w1ne/labwired-core/actions/workflows/core-ci.yml)

### Quality Signals

- `coverage` (PR + push + scheduled/manual): ![Coverage](https://github.com/w1ne/labwired-core/actions/workflows/core-coverage.yml/badge.svg?branch=main)  
[Workflow](https://github.com/w1ne/labwired-core/actions/workflows/core-coverage.yml)
- `unsupported-audit` (scheduled/manual): ![Unsupported Audit](https://github.com/w1ne/labwired-core/actions/workflows/core-unsupported-audit.yml/badge.svg?branch=main)  
[Workflow](https://github.com/w1ne/labwired-core/actions/workflows/core-unsupported-audit.yml)
- `nightly-validation` (scheduled/manual): ![Nightly](https://github.com/w1ne/labwired-core/actions/workflows/core-nightly.yml/badge.svg?branch=main)  
[Workflow](https://github.com/w1ne/labwired-core/actions/workflows/core-nightly.yml)

### Board Model Signals

- `ci-fixture-uart1` (ARM Cortex-M): ![ARM Board](https://github.com/w1ne/labwired-core/actions/workflows/core-board-ci-fixture-arm.yml/badge.svg?branch=main)  
Coverage: smoke + max UART + no-progress  
[ARM Board CI](https://github.com/w1ne/labwired-core/actions/workflows/core-board-ci-fixture-arm.yml)
- `ci-fixture-riscv-uart1` (RISC-V): ![RISC-V Board](https://github.com/w1ne/labwired-core/actions/workflows/core-board-ci-fixture-riscv.yml/badge.svg?branch=main)  
Coverage: smoke  
[RISC-V Board CI](https://github.com/w1ne/labwired-core/actions/workflows/core-board-ci-fixture-riscv.yml)
- `nucleo-h563zi`: ![H563 Board](https://github.com/w1ne/labwired-core/actions/workflows/core-board-nucleo-h563zi.yml/badge.svg?branch=main)  
Coverage: io-smoke + fullchip-smoke  
[NUCLEO-H563ZI CI](https://github.com/w1ne/labwired-core/actions/workflows/core-board-nucleo-h563zi.yml)

## Highlights

- **🚀 [Demos & Examples](../DEMOS.md)** - Central portal for all LabWired demos.
- **v0.1.0 Demo**: [Blinky + I2C Sensor](examples/demo-blinky/README.md)
- **NUCLEO-H563ZI Showcase**: [Human Demo Example](examples/nucleo-h563zi/README.md)
- **Case Study**: [Debugging STM32 Without Hardware](docs/case_study_stm32.md)

## Features

- **Multi-Architecture**: ARM Cortex-M (M0, M3, M4) and RISC-V (RV32I) support
- **Declarative Configuration**: YAML-based chip and peripheral definitions
- **CI Test Runner**: Deterministic `labwired test` with JSON/JUnit outputs
- **Debug Protocols**: GDB Remote Serial Protocol and Debug Adapter Protocol (DAP)
- **High Performance**: Native Rust implementation with cycle-accurate simulation
- **HAL Compatible**: Run binaries built with standard HALs (stm32f1xx-hal, etc.)

## Quick Start

### Install (one-liner)

```sh
curl -fsSL https://labwired.com/install.sh | sh
```

This detects your platform (Linux / macOS, x86_64 / ARM64), downloads a prebuilt binary from the
latest [GitHub Release](https://github.com/w1ne/labwired-core/releases), and adds `labwired` to
your `$PATH`. If no prebuilt is available for your platform it falls back to compiling from source
via `cargo install`.

```sh
labwired --version
```

> **Options** (env vars):
> - `LABWIRED_VERSION=v0.12.0` — pin a specific release
> - `LABWIRED_FROM_SOURCE=1` — always build from source
> - `LABWIRED_INSTALL_DIR=~/.local/bin` — override install directory

### Running a Simulation

```bash
labwired test --script tests/uart-ok.yaml --output-dir results
```

### From Source

If you prefer to build manually:

```bash
# Prerequisites: Rust 1.75+ and target toolchains
rustup target add thumbv6m-none-eabi thumbv7m-none-eabi riscv32i-unknown-none-elf

git clone https://github.com/w1ne/labwired-core.git
cd labwired-core
cargo build --release -p labwired-cli
./target/release/labwired --version
```

## CI Integration

Use `labwired test` for deterministic CI testing:

```bash
labwired test --script tests/uart-ok.yaml --output-dir results
```

See [`docs/ci_integration.md`](./docs/ci_integration.md) for details.

## Documentation

- [Agents Manual](./docs/agents.md) 🤖
- [Architecture Overview](./docs/architecture.md)
- [Architecture Guide](./docs/architecture_guide.md)
- [Board Onboarding Playbook](./docs/board_onboarding_playbook.md) (config-first)
- [CI Integration Guide](./docs/ci_integration.md)
- [GDB Integration](./docs/gdb_integration.md)
- [Release Strategy](./docs/release_strategy.md)
- [VS Code Debugging](./docs/vscode_debugging.md)

## License

MIT
