# LabWired Core

> Run your firmware on a virtual instance of a real chip, from your terminal, your CI, or
> your AI coding agent. No board on your desk.

[![Documentation](https://img.shields.io/badge/docs-latest-blue.svg)](https://docs.labwired.com/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

LabWired Core loads a real firmware ELF and executes it against modeled silicon: CPU,
buses, peripherals, sensors, displays, and protocol devices. You get UART, GPIO, bus
traces, and pass/fail, deterministically and without hardware.

We publish what we model, what is smoke-tested, and what has been compared against real
hardware, including [where we still cheat](FIDELITY.md).

This repository is the engine. There is also a hosted browser
[Playground](https://app.labwired.com/) and a hosted MCP connector, both running the same
models.

## Quickstart

Clone the repo, install the CLI, run a firmware. No cross-toolchain needed.

```sh
git clone https://github.com/w1ne/labwired-core && cd labwired-core
curl -fsSL https://labwired.com/install.sh | LABWIRED_VERSION=v0.21.0 sh
labwired test --script examples/nrf54l15-dk/io-smoke.yaml
```

```
nRF54L15 boot OK
core=cortex-m33 rram=1524K ram=256K
uarte20@0x500C6000 gpio2@0x50050400
regs from MDK/SVD, not nRF52
```

That is a committed bare-metal ELF booting on an nRF54L15-DK profile in under a second.
The firmware read its own memory geometry and peripheral bases out of the modeled chip,
and the script asserts all three lines. The register map comes from the MDK/SVD data for
this part, so the output is a check on the chip model, not a printed constant. Nothing is
compiled on your machine.

Linux, macOS, and Windows via WSL2. `LABWIRED_VERSION=` pins a release,
`LABWIRED_INSTALL_DIR=` sets the install directory, `LABWIRED_FROM_SOURCE=1` builds from
source. To read the installer first:
`curl -fsSL https://labwired.com/install.sh -o install.sh`, review it, then
`sh install.sh`.

## Three ways to drive it

<details>
<summary><b>From your terminal</b></summary>

```sh
labwired run  --firmware path/to/firmware.elf --system configs/systems/<board>.yaml
labwired test --script  path/to/test.yaml --junit report.xml
```

`run` is interactive: UART, GPIO, traces, snapshots. `test` is the deterministic gate. It
emits `result.json`, `uart.log`, and JUnit, and returns an exit code. See the
[CLI reference](docs/cli_reference.md) and [test runner](docs/ci_test_runner.md).
</details>

<details>
<summary><b>From your AI coding agent (MCP)</b></summary>

LabWired speaks MCP, so an agent can assemble a board, run firmware, and read back the
result without you driving the toolchain.

```sh
claude mcp add labwired --transport http https://api.labwired.com/mcp
codex   mcp add labwired --url https://api.labwired.com/mcp
```

Other MCP clients take the standard block:

```json
{ "mcpServers": { "labwired": { "type": "http", "url": "https://api.labwired.com/mcp" } } }
```

Then tell your agent:

> *"Connect LabWired over MCP. Load a virtual STM32 LED + UART board, run the firmware,
> check the UART output, and give me the Playground URL."*

Agents working inside this repository should read [docs/agents.md](docs/agents.md).
</details>

<details>
<summary><b>In CI</b></summary>

The same YAML scripts are the merge gate, with no HIL bench to maintain:

```yaml
- run: labwired test --script examples/ci/uart-ok.yaml --junit report.xml
```

Assertions cover UART content, memory and register values, stop reasons, and step and
wall-time limits. See [CI integration](docs/ci_integration.md).
</details>

## How it works

Your firmware ELF runs against a modeled chip and board, produces real peripheral traffic,
and gives you output you can assert on. Runs are deterministic: the same inputs give the
same trace on every machine.

A board is described in data rather than code. A chip descriptor in
[`configs/chips`](configs/chips/) and a system manifest in
[`configs/systems`](configs/systems/) wire up memory, pins, buses, and attached devices.
Firmware writes to modeled registers and the modeled hardware drives the firmware back.
The blinky example above passes because the firmware set the GPIO enable bit, not because
the result was stubbed.

## What we validate

Every capability here sits in one of three tiers, and we say which:

- **Modeled**: simulator logic exists and firmware can execute against it.
- **Smoke-tested**: a committed test or example exercises the model and checks output.
- **Hardware-compared**: captured silicon behavior is diffed against simulator behavior
  for a documented scope.

The NUCLEO-H563ZI example is the reference case, with the same firmware on a physical
board and on the simulator and the artifacts committed:
[`VALIDATION.md`](examples/nucleo-h563zi/VALIDATION.md),
[`determinism_report_h563.json`](examples/nucleo-h563zi/golden-reference/determinism_report_h563.json),
and the [golden reference method](docs/golden_reference.md).

Those reports are evidence for the scope they describe, not a claim that every instruction
and timing path matches silicon. Where we short-circuit hardware, it is recorded in the
[Fidelity Ledger](FIDELITY.md), along with the cases that hold up, such as an e-paper
panel matching silicon on 19033 of 19033 SPI transfers.

## Boards and examples

ARM Cortex-M and RISC-V have the deepest coverage. Selected ESP32/Xtensa paths exist for
specific examples. Per-board status is in [docs/boards](docs/boards/) and the
[validation status matrix](docs/boards/VALIDATION_STATUS.md). Check it before assuming a
peripheral is modeled.

| To see | Run |
| --- | --- |
| A deterministic pass/fail gate | [CI UART smoke](examples/ci/README.md) |
| Firmware driving a sensor over I2C | [Blinky + TMP102](examples/demo-blinky/README.md) |
| Simulator compared against a physical board | [NUCLEO-H563ZI](examples/nucleo-h563zi/README.md) |
| CAN/UDS diagnostics | [UDS on STM32H563](examples/h563-uds-ecu/README.md) |
| An IO-Link device | [IO-Link DIDO](examples/iolink-dido/README.md) |
| GDB or VS Code debugging without a probe | [Debugging](docs/debugging.md), [GDB](docs/gdb_integration.md) |

The full list is in [docs/demos.md](docs/demos.md). Each example's README and validation
file is the source of truth for what that example actually models.

## Missing a chip? Peripheral behaving wrong? Tell us here.

[Open an issue][new-issue] for a wrong register, an unsupported instruction, a board you
need, or a peripheral you would model differently. Wrong-behavior reports are most useful
with a firmware ELF and a system manifest attached, since those usually become regression
tests.

Adding a board is the easiest way to contribute: follow the
[board onboarding playbook](docs/board_onboarding_playbook.md), mirror an existing
example, and check the result against the
[target support rubric](docs/target_support_rubric.md). See [ROADMAP.md](ROADMAP.md) for
what is planned.

[new-issue]: https://github.com/w1ne/labwired-core/issues/new

## Repository scope

This repository owns the core simulator and its validation assets: CPU, bus, memory,
peripheral, and external device execution; chip and system descriptors; CLI, test runner,
debug adapters, and snapshot/trace tooling; and hardware-target validation metadata.
Application UI, hosted Playground behavior, and product surfaces live outside this
package.

The merge gate is [`core-ci.yml`](.github/workflows/core-ci.yml). Narrower signals come
from board smoke coverage ([`core-board-ci.yml`](.github/workflows/core-board-ci.yml)),
coverage ([matrix smoke](.github/workflows/core-coverage-matrix-smoke.yml),
[weekly](.github/workflows/core-coverage-weekly.yml)),
[unsupported instruction audits](.github/workflows/core-unsupported-audit.yml),
[nightly validation](.github/workflows/core-nightly.yml),
[hardware target sweeps](.github/workflows/core-validate-hw-targets.yml), and per-board
throughput ([`core-perf.yml`](.github/workflows/core-perf.yml)). For release mechanics see
[RELEASE_PROCESS.md](RELEASE_PROCESS.md) and
[RELEASE_READINESS_CHECKLIST.md](RELEASE_READINESS_CHECKLIST.md).

## Documentation

[Docs index](docs/index.md) ·
[Architecture overview](docs/architecture_overview.md) ·
[Engine architecture](docs/architecture.md) ·
[CLI reference](docs/cli_reference.md) ·
[CI test runner](docs/ci_test_runner.md) ·
[Configuration reference](docs/configuration_reference.md) ·
[Board onboarding playbook](docs/board_onboarding_playbook.md) ·
[Target support rubric](docs/target_support_rubric.md) ·
[Debugging](docs/debugging.md) ·
[PlatformIO integration](docs/platformio_integration.md) ·
[Agents manual](docs/agents.md)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for repository workflow and
[docs/agents.md](docs/agents.md) for AI-agent guidance. For security issues, see
[SECURITY.md](SECURITY.md).

## License

MIT. See [LICENSE](LICENSE).
