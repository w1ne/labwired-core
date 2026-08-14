<a href="https://labwired.com">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/w1ne/labwired-core/main/docs/assets/brand/labwired-logo-dark.png">
    <img alt="LabWired" src="https://raw.githubusercontent.com/w1ne/labwired-core/main/docs/assets/brand/labwired-logo.png" width="300">
  </picture>
</a>

# LabWired Core

Run your firmware on a virtual instance of a real chip, from your terminal, your CI, or your
AI coding agent. No board on your desk.

[![Rust Core CI](https://github.com/w1ne/labwired-core/actions/workflows/core-ci.yml/badge.svg?branch=main)](https://github.com/w1ne/labwired-core/actions/workflows/core-ci.yml)
[![Board CI](https://github.com/w1ne/labwired-core/actions/workflows/core-board-ci.yml/badge.svg?branch=main)](https://github.com/w1ne/labwired-core/actions/workflows/core-board-ci.yml)
[![HW target validation](https://github.com/w1ne/labwired-core/actions/workflows/core-validate-hw-targets.yml/badge.svg?branch=main)](https://github.com/w1ne/labwired-core/actions/workflows/core-validate-hw-targets.yml)
[![Unsupported instruction audit](https://github.com/w1ne/labwired-core/actions/workflows/core-unsupported-audit.yml/badge.svg?branch=main)](https://github.com/w1ne/labwired-core/actions/workflows/core-unsupported-audit.yml)
[![Onboarding smoke](https://github.com/w1ne/labwired-core/actions/workflows/core-onboarding-smoke.yml/badge.svg?branch=main)](https://github.com/w1ne/labwired-core/actions/workflows/core-onboarding-smoke.yml)
[![Arduino matrix](https://github.com/w1ne/labwired-core/actions/workflows/core-arduino-matrix-smoke.yml/badge.svg?branch=main)](https://github.com/w1ne/labwired-core/actions/workflows/core-arduino-matrix-smoke.yml)
[![IO-Link native](https://github.com/w1ne/labwired-core/actions/workflows/core-iolink-native.yml/badge.svg?branch=main)](https://github.com/w1ne/labwired-core/actions/workflows/core-iolink-native.yml)

[![Latest release](https://img.shields.io/github/v/release/w1ne/labwired-core?label=release&color=0b7285)](https://github.com/w1ne/labwired-core/releases/latest)
[![Website](https://img.shields.io/badge/web-labwired.com-0b7285.svg)](https://labwired.com/)
[![Documentation](https://img.shields.io/badge/docs-latest-blue.svg)](https://docs.labwired.com/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

<img alt="labwired test booting an nRF54L15 and gating on the result" src="https://raw.githubusercontent.com/w1ne/labwired-core/main/docs/assets/labwired-demo.gif" width="100%">

<sub>Recorded from [`docs/assets/demo.tape`](docs/assets/demo.tape) against a real binary. You
can re-run it. The second half breaks an assertion on purpose, so you can see the gate
fail.</sub>

## What is LabWired Core?

LabWired Core loads a firmware ELF and executes it against modeled silicon: CPU, buses,
peripherals, sensors, displays, and protocol devices. You get UART output, GPIO and bus
traces, register state, and a pass/fail exit code.

Runs are deterministic. The same ELF and the same board manifest give the same trace on
every machine. That is what makes a simulator run usable as a CI gate.

Supported cores:

* ARM Cortex-M0+, Cortex-M3, Cortex-M4, Cortex-M7, Cortex-M33
* RISC-V
* Xtensa LX6 and LX7 (selected ESP32 paths)

This repository is the engine behind [labwired.com](https://labwired.com/). The hosted
browser [Playground](https://app.labwired.com/) and the hosted MCP connector run the same
models. To look before installing anything, open a lab:
[SSD1306 hello](https://app.labwired.com/?lab=ssd1306-hello-lab),
[BME280 weather](https://app.labwired.com/?lab=bme280-weather-lab), or
[IO-Link DI/DO](https://app.labwired.com/?lab=iolink-dido).

## Why use LabWired Core?

Testing firmware on real boards is slow. You wait for hardware, you flash it, and when
something breaks you get a dark LED and no trace. Do that for every board in a product line
and a hardware CI bench stops being worth the upkeep.

LabWired runs the same binary you would flash. You assert on what the firmware made the
hardware do.

Two things separate it from a CPU emulator.

**The board is modeled, not only the chip.** Sensors, displays, and bus devices live in the
system manifest. You can assert on I2C traffic, SPI framing, or what a panel was told to
draw.

**We publish where the model is thin.** Every capability sits in a named tier. Anywhere the
simulator short-circuits real hardware, we write it down in the
[Fidelity Ledger](FIDELITY.md). You cannot gate on a simulator you cannot audit.

## Quickstart

Clone the repository, install the CLI, run a firmware. No cross-toolchain needed.

```sh
git clone https://github.com/w1ne/labwired-core && cd labwired-core
curl -fsSL https://labwired.com/install.sh | LABWIRED_VERSION=v0.22.1 sh
labwired test --script examples/nrf54l15-dk/io-smoke.yaml
```

```
nRF54L15 boot OK
core=cortex-m33 rram=1524K ram=256K
uarte20@0x500C6000 gpio2@0x50050400
regs from MDK/SVD, not nRF52
PASS  4/4 checks · io-smoke · 200000 steps · 0.04s
```

That is a committed bare-metal ELF booting on an nRF54L15-DK profile in under a second.
Nothing is compiled on your machine.

The four checks are the three UART lines plus the stop reason.

The firmware prints those lines from string literals. The text proves nothing by itself.
What it proves is that the bytes arrived. To send them, the firmware had to boot from RRAM
at `0x0`, set up RAM at `0x20000000`, and drive UARTE20 over EasyDMA. Get the UARTE base
wrong in the chip model and you get no output at all.

GPIO is checked at the pin, not in the banner, by
`firmware_survival::test_nrf54l15_lights_dk_led0`. The
[example README](examples/nrf54l15-dk/README.md) says why both tests are needed.

The verdict goes to stderr. The exit code is the gate: `0` passes, anything else fails.
UART output and `--json` stay on stdout, so pipes keep working.

## Installation

The install script covers Linux, macOS, and Windows via WSL2.

```sh
curl -fsSL https://labwired.com/install.sh | LABWIRED_VERSION=v0.22.1 sh
```

| Variable | Effect |
| --- | --- |
| `LABWIRED_VERSION=` | pin a release |
| `LABWIRED_INSTALL_DIR=` | choose the install directory |
| `LABWIRED_FROM_SOURCE=1` | build from source instead of downloading |

To read the installer before running it:

```sh
curl -fsSL https://labwired.com/install.sh -o install.sh   # review it, then:
sh install.sh
```

Or build it yourself:

```sh
cargo build --release -p labwired-cli
```

## Running LabWired

### From your terminal

```sh
labwired run  --chip configs/chips/<chip>.yaml --firmware path/to/firmware.elf
labwired test --script  path/to/test.yaml --junit report.xml
```

`run` is interactive: UART, GPIO, traces, snapshots.

`test` is the gate. It writes `result.json`, `uart.log`, and JUnit, and returns an exit code.
Assertions cover UART content, memory and register values, stop reasons, and step and
wall-time limits.

See the [CLI reference](docs/cli_reference.md) and the
[test runner reference](docs/ci_test_runner.md).

### In CI

The same YAML scripts are the merge gate, with no HIL bench to maintain:

```yaml
- run: labwired test --script examples/ci/uart-ok.yaml --junit report.xml
```

See [CI integration](docs/ci_integration.md) and
[labwired.com/ci](https://labwired.com/ci.html).

### From an AI coding agent (MCP)

LabWired speaks MCP. An agent can assemble a board, run firmware, and read the result back.
You never touch the toolchain.

```sh
claude mcp add labwired --transport http https://api.labwired.com/mcp
codex   mcp add labwired --url https://api.labwired.com/mcp
```

Your client opens a browser to sign in on first use. Other MCP clients take the standard
block:

```json
{ "mcpServers": { "labwired": { "type": "http", "url": "https://api.labwired.com/mcp" } } }
```

Then ask for what you want:

> *"Connect LabWired over MCP. Load a virtual STM32 LED + UART board, run the firmware,
> check the UART output, and give me the Playground URL."*

Agents working inside this repository should read [docs/agents.md](docs/agents.md).

## How a board is described

A board is data, not code.

A chip descriptor in [`configs/chips`](configs/chips/) declares memory geometry, cores, and
peripheral bases. A system manifest in [`configs/systems`](configs/systems/) wires up pins,
buses, and the devices on them. Firmware writes to modeled registers. The modeled hardware
drives the firmware back.

To add a board you write those two files and an example. You do not patch the engine. See
the [board onboarding playbook](docs/board_onboarding_playbook.md) and the
[configuration reference](docs/configuration_reference.md).

## What we validate

Every capability sits in one of three tiers, and we say which:

* **Modeled** — simulator logic exists and firmware can execute against it.
* **Smoke-tested** — a committed test or example exercises the model and checks output.
* **Hardware-compared** — captured silicon behavior is diffed against simulator behavior for
  a documented scope.

The NUCLEO-H563ZI example is the reference case. The same firmware runs on a physical board
and on the simulator, and the artifacts are committed. See
[`VALIDATION.md`](examples/nucleo-h563zi/VALIDATION.md),
[`determinism_report_h563.json`](examples/nucleo-h563zi/golden-reference/determinism_report_h563.json),
and the [golden reference method](docs/golden_reference.md).

Each report covers the scope it describes. None of them claim that every instruction and
timing path matches silicon. The [Fidelity Ledger](FIDELITY.md) has both sides: where we
short-circuit hardware, and where we hold up. One e-paper panel matches silicon on 19033 of
19033 SPI transfers.

## Boards and examples

ARM Cortex-M and RISC-V have the deepest coverage. Some ESP32/Xtensa paths exist for
specific examples.

Check the per-board status before you assume a peripheral is modeled. It is in
[docs/boards](docs/boards/), the
[validation status matrix](docs/boards/VALIDATION_STATUS.md), and the browsable catalog at
[app.labwired.com/validation](https://app.labwired.com/validation).

| To see | Run |
| --- | --- |
| A deterministic pass/fail gate | [CI UART smoke](examples/ci/README.md) |
| Firmware driving a sensor over I2C | [Blinky + TMP102](examples/demo-blinky/README.md) |
| Simulator compared against a physical board | [NUCLEO-H563ZI](examples/nucleo-h563zi/README.md) |
| CAN/UDS diagnostics | [UDS on STM32H563](examples/h563-uds-ecu/README.md) |
| An IO-Link device | [IO-Link DIDO](examples/iolink-dido/README.md) |
| GDB or VS Code debugging without a probe | [Debugging](docs/debugging.md), [GDB](docs/gdb_integration.md) |

The full list is in [docs/demos.md](docs/demos.md). Each example's README and validation file
is the source of truth for what that example actually models.

## The LabWired ecosystem

This repository is the engine. Everything below runs on it, and all of it is public.

**Put it in your workflow**

| Project | What it gives you |
| --- | --- |
| [firmware-test](https://github.com/LabWired/firmware-test) | GitHub Action. Run a LabWired test script as a merge gate — no hardware, no cross-toolchain on the runner. |
| [firmware-ci-starter](https://github.com/LabWired/firmware-ci-starter) | Template repo. Firmware, test script, and workflow already wired — [generate](https://github.com/LabWired/firmware-ci-starter/generate) it and your first push runs green. |
| [labwired-zephyr](https://github.com/w1ne/labwired-zephyr) | Zephyr west runner. `west simulate`, or `west flash -r labwired`. |
| [labwired-vscode](https://github.com/w1ne/labwired-vscode) | VS Code extension. Run and debug firmware from the editor without a probe. |
| [labwired-lab-template](https://github.com/w1ne/labwired-lab-template) | Bare template for a repo whose merges gate on a simulation run. |

**Drive it from an agent**

| Project | What it gives you |
| --- | --- |
| [agent](https://github.com/LabWired/agent) | The LabWired Firmware Agent — writes firmware and checks it on a virtual board. |
| [skills](https://github.com/LabWired/skills) | Agent Skills for firmware work against the simulator as a hardware oracle. |
| [`docs/agents.md`](docs/agents.md) | The MCP surface and the rules an agent working in this repo should follow. |

**See it doing real work**

| Project | What it shows |
| --- | --- |
| [labwired-cra-evidence](https://github.com/LabWired/labwired-cra-evidence) | CRA-style secure-boot and signed-OTA evidence, regenerated in CI on a virtual nRF52840 + ATECC608A. Evidence, not a certificate. |
| [smart-ring-digital-twin](https://github.com/w1ne/smart-ring-digital-twin) | An nRF54L15 smart ring, register-level sensor models, and an honest BLE-contention demonstration. |
| [labwired-nokia-ci-demo](https://github.com/w1ne/labwired-nokia-ci-demo) | STM32L476 driving a Nokia 5110 (PCD8544) and an HC-SR04, gated in CI. |
| [labwired-demo-stm32f1xx](https://github.com/w1ne/labwired-demo-stm32f1xx) | Rust `embedded-hal` on STM32F1 under the simulator. |

**Firmware stacks that use LabWired as their test bench**

[iolinki](https://github.com/w1ne/iolinki) (IO-Link device stack for Zephyr) ·
[iolinki-master](https://github.com/w1ne/iolinki-master) (IO-Link master stack) ·
[udslib](https://github.com/w1ne/udslib) (ISO 14229 UDS for embedded ECUs) ·
[thermal-io-link-condition-sensor](https://github.com/w1ne/thermal-io-link-condition-sensor) (MLX90640 + ESP32-C3 condition monitoring)

Using LabWired for something public? [Open an issue][new-issue] and it goes on this list.

## Missing a chip? Peripheral behaving wrong?

[Open an issue][new-issue] for a wrong register, an unsupported instruction, a board you
need, or a peripheral you would model differently. Attach a firmware ELF and a system
manifest if you can — those usually become regression tests.

Adding a board is the easiest way to contribute: follow the
[board onboarding playbook](docs/board_onboarding_playbook.md), mirror an existing example,
and check the result against the [target support rubric](docs/target_support_rubric.md).

See [ROADMAP.md](ROADMAP.md) for what is planned, [CONTRIBUTING.md](CONTRIBUTING.md) for
repository workflow, and [SECURITY.md](SECURITY.md) for security issues.

[new-issue]: https://github.com/w1ne/labwired-core/issues/new

## Repository scope

This repository owns the core simulator and its validation assets: CPU, bus, memory,
peripheral, and external device execution; chip and system descriptors; CLI, test runner,
debug adapters, and snapshot/trace tooling; and hardware-target validation metadata.
Application UI, hosted Playground behavior, and product surfaces live outside this package.

The merge gate is [`core-ci.yml`](.github/workflows/core-ci.yml). Narrower signals come from
board smoke coverage ([`core-board-ci.yml`](.github/workflows/core-board-ci.yml)), coverage
([matrix smoke](.github/workflows/core-coverage-matrix-smoke.yml),
[weekly](.github/workflows/core-coverage-weekly.yml)),
[unsupported instruction audits](.github/workflows/core-unsupported-audit.yml),
[nightly validation](.github/workflows/core-nightly.yml),
[hardware target sweeps](.github/workflows/core-validate-hw-targets.yml), and per-board
throughput ([`core-perf.yml`](.github/workflows/core-perf.yml)). Release mechanics are in
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

## Project links

[labwired.com](https://labwired.com/) ·
[Playground](https://app.labwired.com/) ·
[Docs](https://docs.labwired.com/) ·
[Validation](https://app.labwired.com/validation) ·
[For CI](https://labwired.com/ci.html) ·
[Blog](https://labwired.com/blog/) ·
[Pricing](https://labwired.com/pricing.html)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for repository workflow and
[docs/agents.md](docs/agents.md) for AI-agent guidance. For security issues, see
[SECURITY.md](SECURITY.md).

## License

MIT. See [LICENSE](LICENSE).
