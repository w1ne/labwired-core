# LabWired Core

Deterministic firmware simulation engine for ARM Cortex-M and RISC-V targets.

[![Documentation](https://img.shields.io/badge/docs-latest-blue.svg)](https://labwired.com/docs/)

## What This Repo Owns

- Simulation engine correctness and determinism.
- Chip/system model execution (`configs/chips`, `configs/systems`).
- Hardware-target validation metadata for catalog consumers.

## Quick Start

### Install

Pinned release (recommended):

```sh
curl -fsSL https://labwired.com/install.sh | LABWIRED_VERSION=v0.13.0 sh
labwired --version
```

Prefer to read the script first:

```sh
curl -fsSL https://labwired.com/install.sh -o install.sh
# review install.sh, then:
LABWIRED_VERSION=v0.13.0 sh install.sh
```

Install options:

- `LABWIRED_VERSION=v0.13.0` pins a release (omit for latest).
- `LABWIRED_FROM_SOURCE=1` forces source build.
- `LABWIRED_INSTALL_DIR=~/.local/bin` overrides install dir.

### Run a deterministic test script

```sh
labwired test --script examples/ci/uart-ok.yaml --output-dir results
```

### Build from source

```sh
rustup target add thumbv6m-none-eabi thumbv7m-none-eabi riscv32i-unknown-none-elf
cargo build --release -p labwired-cli
./target/release/labwired --version
```

## CI At A Glance

### Required merge gate

- `.github/workflows/core-ci.yml`: fmt, clippy, build, and integration tests on every PR to `main`.

### Quality signals

- `core-coverage.yml`: coverage verification.
- `core-unsupported-audit.yml`: unsupported instruction audits.
- `core-nightly.yml`: broader nightly validation.
- `core-validate-hw-targets.yml`: full onboarding target sweep, emits `out/hw-target-validation/summary.{json,md}`, and refreshes onboarding validation metadata.

### Board model signals

- `core-board-ci-fixture-arm.yml`: ARM fixture smoke coverage.
- `core-board-ci-fixture-riscv.yml`: RISC-V fixture smoke coverage.
- `core-board-nucleo-h563zi.yml`: H563 io-smoke and fullchip-smoke.

## Validation Structure

- PR smoke and scoreboard: `core-onboarding-smoke.yml`.
- Full target sweep for catalog metadata: `core-validate-hw-targets.yml`.
- Policy and downstream contract: [docs/catalog_validation.md](./docs/catalog_validation.md).

## Key Docs

- [Docs Index](./docs/index.md)
- [Architecture](./docs/architecture.md)
- [Board Onboarding Playbook](./docs/board_onboarding_playbook.md)
- [CI Integration Guide](./docs/ci_integration.md)
- [Release Strategy](./docs/release_strategy.md)
- [Agents Manual](./docs/agents.md)

## Highlights

- [Demos & Examples](../DEMOS.md)
- [Blinky + I2C Sensor Demo](examples/demo-blinky/README.md)
- [NUCLEO-H563ZI Showcase](examples/nucleo-h563zi/README.md)
- [STM32 Case Study](docs/case_study_stm32.md)

## License

MIT
