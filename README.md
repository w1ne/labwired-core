# LabWired Core

> High-performance, declarative firmware simulator for ARM Cortex-M, RISC-V, and Xtensa LX7 (ESP32-S3) microcontrollers.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![LabWired Core CI](https://github.com/w1ne/labwired-core/actions/workflows/ci.yml/badge.svg)](https://github.com/w1ne/labwired-core/actions/workflows/ci.yml)
[![Documentation](https://img.shields.io/badge/docs-latest-blue.svg)](https://labwired.com/docs/index.html)

## Highlights

- **🚀 [Demos & Examples](../DEMOS.md)** - Central portal for all LabWired demos.
- **🤖 [Agentic Hardware Fix](../ai/tests/autonomous_fix_demo.py)** - WATCH: AI agent autonomously fixing a peripheral model.
- **v0.1.0 Demo**: [Blinky + I2C Sensor](examples/demo-blinky/README.md)
- **NUCLEO-H563ZI Showcase**: [Human Demo Example](examples/nucleo-h563zi/README.md)
- **NUCLEO-H563ZI Runbook**: [Reproducible Validation Steps](examples/nucleo-h563zi/VALIDATION.md)
- **ESP32-S3 hello-world**: [Plan 2 case study](docs/case_study_esp32s3_plan2.md) — real esp-hal Rust binary printing via USB_SERIAL_JTAG.
- **ESP32-S3 blinky**: [Plan 3 case study](docs/case_study_esp32s3_plan3.md) — GPIO toggle from a SYSTIMER alarm ISR, observed via the new `GpioObserver` trait.
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

- [Architecture Overview](./docs/architecture.md)
- [Architecture Guide](./docs/architecture_guide.md)
- [Board Onboarding Playbook](./docs/board_onboarding_playbook.md) (config-first)
- [CI Integration Guide](./docs/ci_integration.md)
- [GDB Integration](./docs/gdb_integration.md)
- [Release Strategy](./docs/release_strategy.md)
- [VS Code Debugging](./docs/vscode_debugging.md)

## License
MIT
