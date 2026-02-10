# LabWired Core - Firmware Simulation Engine

> High-performance, declarative firmware simulator for ARM Cortex-M and RISC-V microcontrollers.

## Highlights

- **v0.1.0 Demo**: [Blinky + I2C Sensor](examples/demo-blinky/README.md)
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

- [Architecture](./docs/architecture.md)
- [Implementation Plan](./docs/plan.md)
- [CI Integration Guide](./docs/ci_integration.md)
- [GDB Integration](./docs/GDB_INTEGRATION.md)
- [VS Code Debugging](./docs/vscode_debugging.md)

## License

MIT
