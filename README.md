# LabWired Core - Firmware Simulation Engine


> High-performance, declarative firmware simulator for ARM Cortex-M and RISC-V microcontrollers.

[![Documentation](https://img.shields.io/badge/docs-latest-blue.svg)](https://labwired.com/docs/)

## CI Dashboard

| Indicator | Status | Link |
|---|---|---|
| Core Integrity (PR Gate) | ![Core Integrity](https://github.com/w1ne/labwired-core/actions/workflows/core-ci.yml/badge.svg?branch=main) | [Workflow](https://github.com/w1ne/labwired-core/actions/workflows/core-ci.yml) |
| Coverage Gate (Scheduled/Manual) | ![Coverage](https://github.com/w1ne/labwired-core/actions/workflows/core-coverage.yml/badge.svg?branch=main) | [Workflow](https://github.com/w1ne/labwired-core/actions/workflows/core-coverage.yml) |
| Unsupported Audit (Scheduled/Manual) | ![Unsupported Audit](https://github.com/w1ne/labwired-core/actions/workflows/core-unsupported-audit.yml/badge.svg?branch=main) | [Workflow](https://github.com/w1ne/labwired-core/actions/workflows/core-unsupported-audit.yml) |
| Nightly Validation (Scheduled/Manual) | ![Nightly](https://github.com/w1ne/labwired-core/actions/workflows/core-nightly.yml/badge.svg?branch=main) | [Workflow](https://github.com/w1ne/labwired-core/actions/workflows/core-nightly.yml) |

### Board Model Dashboard

| Board Model | Status | Verification Coverage | Instruction Support % (runtime) | Workflow |
|---|---|---|---|---|
| `ci-fixture-uart1` (ARM Cortex-M) | ![ARM Board](https://github.com/w1ne/labwired-core/actions/workflows/core-board-ci-fixture-arm.yml/badge.svg?branch=main) | smoke + max UART + no-progress | published in workflow summary/artifacts | [ARM Board CI](https://github.com/w1ne/labwired-core/actions/workflows/core-board-ci-fixture-arm.yml) |
| `ci-fixture-riscv-uart1` (RISC-V) | ![RISC-V Board](https://github.com/w1ne/labwired-core/actions/workflows/core-board-ci-fixture-riscv.yml/badge.svg?branch=main) | smoke | published in workflow summary/artifacts | [RISC-V Board CI](https://github.com/w1ne/labwired-core/actions/workflows/core-board-ci-fixture-riscv.yml) |
| `nucleo-h563zi` | ![H563 Board](https://github.com/w1ne/labwired-core/actions/workflows/core-board-nucleo-h563zi.yml/badge.svg?branch=main) | io-smoke + fullchip-smoke | published in workflow summary/artifacts | [NUCLEO-H563ZI CI](https://github.com/w1ne/labwired-core/actions/workflows/core-board-nucleo-h563zi.yml) |

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

### Prerequisites
- Rust 1.75+ ([Install](https://rustup.rs/))
- ARM/RISC-V targets:
  ```bash
  rustup target add thumbv6m-none-eabi thumbv7m-none-eabi riscv32i-unknown-none-elf
  ```

### Building
```bash
cargo build --release
```

### Running a Simulation
```bash
cargo run -p labwired-cli -- --firmware path/to/firmware.elf --system system.yaml
```

### Testing
```bash
cargo test --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture
```

## CI Integration

Use `labwired test` for deterministic CI testing:

```bash
labwired test --script tests/uart-ok.yaml --output-dir results
```

See [`docs/ci_integration.md`](./docs/ci_integration.md) for details.

## Documentation

- [Architecture Overview](./docs/architecture.md)
- [Architecture Guide](./docs/architecture_guide.md)
- [Board Onboarding Playbook](./docs/board_onboarding_playbook.md) (config-first)
- [CI Integration Guide](./docs/ci_integration.md)
- [GDB Integration](./docs/gdb_integration.md)
- [Release Strategy](./docs/release_strategy.md)
- [VS Code Debugging](./docs/vscode_debugging.md)

## License

MIT
