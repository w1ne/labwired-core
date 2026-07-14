# LabWired Core

LabWired Core runs embedded firmware in a deterministic simulated hardware lab.

It loads real firmware ELFs and executes them against modeled chips, boards,
peripherals, buses, sensors, displays, and protocol devices. The goal is not to
replace every hardware test. The goal is to move the fast, repeatable part of
firmware bring-up and regression testing into a local and CI-friendly simulator.

[![Documentation](https://img.shields.io/badge/docs-latest-blue.svg)](https://labwired.com/docs/)

## What You Can Do

| Use case | Where to start | What LabWired provides |
| --- | --- | --- |
| Run firmware without a board | [`labwired run`](docs/cli_reference.md) | ELF loading, system manifests, UART/GPIO output, traces, snapshots |
| Gate firmware behavior in CI | [`labwired test`](docs/ci_test_runner.md) | YAML test scripts, assertions, exit codes, artifacts, JUnit output |
| Debug firmware without hardware | [Debugging](docs/debugging.md), [GDB](docs/gdb_integration.md) | GDB RSP and VS Code DAP support for breakpoints and register inspection |
| Model board-level I/O | [`examples/demo-blinky`](examples/demo-blinky/README.md) | External I2C/SPI-style device attachment, board I/O mapping, deterministic checks |
| Validate simulator behavior against hardware | [`examples/nucleo-h563zi`](examples/nucleo-h563zi/README.md) | Hardware and simulator traces, UART logs, reproducible validation reports |
| Add or audit chip support | [Board onboarding](docs/board_onboarding_playbook.md), [coverage](docs/coverage_scoreboard.md) | Chip/system YAML, target support rubric, smoke coverage, catalog metadata |

Supported targets are intentionally uneven. ARM Cortex-M and RISC-V have the
deepest CI coverage today; selected ESP32/Xtensa paths exist for specific
examples. Check the per-board docs before assuming a peripheral is modeled:
[docs/boards](docs/boards/).

## Quick Start

### Install the CLI

Pinned release:

```sh
curl -fsSL https://labwired.com/install.sh | LABWIRED_VERSION=v0.19.0 sh
labwired --version
```

Prefer to inspect the installer first:

```sh
curl -fsSL https://labwired.com/install.sh -o install.sh
# review install.sh, then:
LABWIRED_VERSION=v0.19.0 sh install.sh
```

Supported host environments:

- Linux
- macOS
- Windows via WSL2

Install options:

- `LABWIRED_VERSION=v0.19.0` pins the current documented release for a
  reproducible install.
- `LABWIRED_FROM_SOURCE=1` forces a source build.
- `LABWIRED_INSTALL_DIR=~/.local/bin` changes the install directory.

### Run the CI smoke example from a source checkout

The bundled smoke script expects the fixture firmware to exist in `target/`.
From the repository root:

```sh
rustup target add thumbv6m-none-eabi
cargo build -p firmware-ci-fixture --release --target thumbv6m-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/ci/uart-ok.yaml \
  --output-dir /tmp/labwired-readme-smoke \
  --no-uart-stdout
```

That script runs the fixture firmware against
[`configs/systems/ci-fixture-uart1.yaml`](configs/systems/ci-fixture-uart1.yaml)
and asserts that UART output contains `OK`.

### Build the simulator from source

```sh
rustup target add thumbv6m-none-eabi thumbv7m-none-eabi riscv32i-unknown-none-elf
cargo build --release -p labwired-cli
./target/release/labwired --version
```

## Examples

Start with examples that have a clear system manifest, firmware path, and smoke
test:

- [CI UART smoke](examples/ci/README.md): minimal `labwired test` scripts for
  deterministic pass/fail behavior.
- [Blinky + TMP102](examples/demo-blinky/README.md): STM32F103 firmware talking
  to a virtual TMP102 sensor over I2C.
- [NUCLEO-H563ZI](examples/nucleo-h563zi/README.md): same demo story on the
  simulator and a physical STM32H563ZI Nucleo board, with committed validation
  artifacts.
- [NUCLEO-L476RG](examples/nucleo-l476rg/README.md): survival and validation
  traces for an STM32 Nucleo target.
- [Seeed XIAO nRF52840 Sense](examples/seeed-xiao-nrf52840-sense/README.md):
  nRF52840 board coverage with UART/GPIO/SPI smoke paths.
- [UDS on STM32F103](examples/f103-uds-ecu/README.md) and
  [UDS on STM32H563](examples/h563-uds-ecu/README.md): CAN/UDS-oriented
  firmware examples.
- [IO-Link DIDO](examples/iolink-dido/README.md): IO-Link device-oriented
  system wiring and smoke test.

For the broader list, see [docs/demos.md](docs/demos.md). Treat each example's
README and validation file as the source of truth for what is actually modeled.

## Validation Model

LabWired distinguishes between three levels of confidence:

- **Modeled**: a chip, peripheral, bus, or external device has simulator logic
  behind it and can execute firmware behavior.
- **Smoke-tested**: a repository test or example script exercises that model
  and checks observable output.
- **Hardware-compared**: captured hardware behavior is compared with simulator
  behavior for a documented scope.

The H563 example is the best place to inspect hardware-comparison artifacts:

- [`examples/nucleo-h563zi/golden-reference/determinism_report_h563.json`](examples/nucleo-h563zi/golden-reference/determinism_report_h563.json)
- [`examples/nucleo-h563zi/VALIDATION.md`](examples/nucleo-h563zi/VALIDATION.md)
- [`docs/golden_reference.md`](docs/golden_reference.md)

Those reports are evidence for the scope they describe. They are not a blanket
claim that every instruction, peripheral, or timing path matches hardware.

## Repository Scope

This repository owns the core simulator and its validation assets:

- CPU, bus, memory, peripheral, and external device execution.
- Chip and system descriptors in [`configs/chips`](configs/chips/) and
  [`configs/systems`](configs/systems/).
- CLI, test runner, debug adapters, and snapshot/trace tooling.
- Hardware-target validation metadata consumed by catalog and app surfaces.

Application UI, hosted playground behavior, and product-specific surfaces live
outside this core package.

## CI and Release Signals

The main merge gate is [`.github/workflows/core-ci.yml`](.github/workflows/core-ci.yml):
formatting, linting, build, and integration tests.

Additional workflows publish narrower signals:

- [`core-board-ci.yml`](.github/workflows/core-board-ci.yml): board/example
  smoke coverage.
- [`core-coverage.yml`](.github/workflows/core-coverage.yml): coverage checks.
- [`core-unsupported-audit.yml`](.github/workflows/core-unsupported-audit.yml):
  unsupported instruction audits.
- [`core-nightly.yml`](.github/workflows/core-nightly.yml): broader scheduled
  validation.
- [`core-validate-hw-targets.yml`](.github/workflows/core-validate-hw-targets.yml):
  onboarding target sweep and catalog metadata.

For release mechanics, see [RELEASE_PROCESS.md](RELEASE_PROCESS.md) and
[RELEASE_READINESS_CHECKLIST.md](RELEASE_READINESS_CHECKLIST.md).

## Documentation

- [Docs index](docs/index.md)
- [Architecture overview](docs/architecture_overview.md)
- [Engine architecture](docs/architecture.md)
- [CLI reference](docs/cli_reference.md)
- [CI test runner](docs/ci_test_runner.md)
- [Configuration reference](docs/configuration_reference.md)
- [Board onboarding playbook](docs/board_onboarding_playbook.md)
- [Target support rubric](docs/target_support_rubric.md)
- [Debugging](docs/debugging.md)
- [PlatformIO integration](docs/platformio_integration.md)
- [Agents manual](docs/agents.md)

## Contributing

Use [CONTRIBUTING.md](CONTRIBUTING.md) for repository workflow and
[docs/agents.md](docs/agents.md) for AI-agent-specific guidance. For security
issues, see [SECURITY.md](SECURITY.md).

## License

MIT. See [LICENSE](LICENSE).
