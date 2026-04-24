# LabWired Core

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![LabWired Core CI](https://github.com/w1ne/labwired-core/actions/workflows/ci.yml/badge.svg)](https://github.com/w1ne/labwired-core/actions/workflows/ci.yml)

Declarative firmware simulator for ARM Cortex-M and RISC-V microcontrollers.

## Highlights
- **v0.1.0 Ready**: [Blinky + I2C Sensor Demo](examples/demo-blinky/README.md)
- **Case Study**: [Debugging STM32 Without Hardware](docs/case_study_stm32.md)
- **Roadmap**: [Future of LabWired](ROADMAP.md)

## Features
- **Multi-Architecture**: ARM Thumb / Thumb-2 (ARMv6-M core + selected ARMv7-M bit-field and
  data-processing ops) and RISC-V RV32I. Full matrix in
  [docs/isa_coverage.md](docs/isa_coverage.md); M4 FPU / DSP and RV32M/A/C are not yet supported.
- **Declarative Hardware**: Define chips and peripherals using YAML — the `GenericPeripheral`
  engine models register banks, reset values, bitfield triggers, and periodic events from
  a descriptor file. Arbitrary protocol state machines still require a native Rust peripheral.
- **CI Test Runner**: Headless `labwired test` with JSON/JUnit reports for continuous verification.
- **IDE Integration**: GDB and Debug Adapter Protocol (DAP) support for VS Code integration.
- **Performance**: baseline measured with `cargo bench -p labwired-core`. See
  [crates/core/benches/fetch_ips.rs](crates/core/benches/fetch_ips.rs); numbers are published
  per release — treat unverified IPS claims elsewhere as stale.

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
- [ISA Coverage Matrix](./docs/isa_coverage.md)
- [CI Integration](./docs/ci_integration.md)
- [GDB Integration](./docs/GDB_INTEGRATION.md)
- [VS Code Debugging](./docs/vscode_debugging.md)

## License
MIT
