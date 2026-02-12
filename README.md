# LabWired Core

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![LabWired Core CI](https://github.com/w1ne/labwired-core/actions/workflows/ci.yml/badge.svg)](https://github.com/w1ne/labwired-core/actions/workflows/ci.yml)

High-performance, declarative firmware simulator for ARM Cortex-M and RISC-V microcontrollers.

## Highlights
- **v0.1.0 Ready**: [Blinky + I2C Sensor Demo](examples/demo-blinky/README.md)
- **Case Study**: [Debugging STM32 Without Hardware](docs/case_study_stm32.md)
- **Roadmap**: [Future of LabWired](ROADMAP.md)

## Features
- **Multi-Architecture**: ARM Cortex-M (M0, M3, M4) and RISC-V (RV32I) support.
- **Declarative Hardware**: Define chips and peripherals using YAML (No Rust coding required for basic IP).
- **CI Test Runner**: Headless `labwired test` with JSON/JUnit reports for continuous verification.
- **IDE Integration**: GDB and Debug Adapter Protocol (DAP) support for VS Code integration.
- **High Performance**: Native Rust implementation achieving hundreds of thousands of IPS.

## Community & Governance
We welcome contributions! Please see our community guides:
- [Contributing](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)

## Quick Start
1. **Prerequisites**: [Rust 1.75+](https://rustup.rs/) and target support:
   ```bash
   rustup target add thumbv6m-none-eabi thumbv7m-none-eabi riscv32i-unknown-none-elf
   ```
2. **Build**: `cargo build --release`
3. **Run**: `cargo run -p labwired-cli -- --firmware <path/to/elf> --system system.yaml`

Full setup guide: [Getting Started](./docs/getting_started_firmware.md)

## Documentation
- [Architecture](./docs/architecture.md)
- [CI Integration](./docs/ci_integration.md)
- [GDB Integration](./docs/GDB_INTEGRATION.md)
- [VS Code Debugging](./docs/vscode_debugging.md)

## License
MIT
