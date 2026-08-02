# LabWired Core Documentation

Welcome to the **LabWired Core** documentation. LabWired is a deterministic firmware simulation platform designed to replace physical hardware in CI pipelines.

## 🚀 Getting Started

If you are new to LabWired, start here:

- **[Running Your Firmware](getting_started_firmware.md)**: Learn how to load ELF binaries and execute them in the simulator.
- **[Per-Board Coverage](boards/)**: What's modeled per chip — see e.g.
  [`stm32f401`](boards/stm32f401.md),
  [`stm32f407` (I²C onboarding-in-flight)](boards/stm32f407.md),
  [`stm32h563`](boards/stm32h563.md),
  [`stm32l476` (gold reference)](boards/nucleo-l476rg.md),
  [`nrf52840`](boards/nrf52840.md),
  [`seeed-xiao-nrf52840-sense`](boards/seeed-xiao-nrf52840-sense.md),
  [`rp2040`](boards/rp2040.md),
  [`esp32c3`](boards/esp32c3.md).
  Check here before pointing firmware at a new chip.
- **[Board Onboarding](board_onboarding_playbook.md)**: Steps to add support for a new microcontroller or board.

## 🧠 Core Concepts

Understand how LabWired achieves deterministic simulation:

- **[Architecture Overview](architecture.md)**: Explains the split between the CPU Core, System Bus, and Peripherals.
- **[Configuration Reference](configuration_reference.md)**: Detailed schema for defining chips and systems (YAML).

## 🛠 Developer Guides

For contributors extending the core engine or adding new peripherals:

- **[Peripheral Modeling](peripherals.md)**: How to model and validate a peripheral — declarative YAML and Rust paths, the silicon-validation loop, and the merge bar.
- **[Declarative Registers](declarative_registers.md)**: The register-map YAML schema.
- **[CI Integration](ci_integration.md)**: How to run LabWired in GitHub Actions or GitLab CI.
- **[Coverage Scoreboard](coverage_scoreboard.md)**: Top-target smoke coverage and deterministic status tracking.
- **Onboarding Smoke CI**: `core-onboarding-smoke.yml` publishes time-to-first-smoke metrics and scoreboard artifacts.
- **[Catalog Validation Structure](catalog_validation.md)**: Separation of PR smoke vs full target sweep and catalog metadata ownership.
- **[Target Support Rubric](target_support_rubric.md)**: Objective support levels and promotion criteria.

## 🔍 Debugging

- **[VS Code Debugging](vscode_debugging.md)**: Recipes for `launch.json`.
- **[Native DAP](debugging.md)**: Architecture of the built-in Debug Adapter.
- **[GDB Integration](gdb_integration.md)**: Using standard GDB clients.

## 🤖 AI Agents

- **[Core Agents Manual](./agents.md)**: Essential onboarding for AI coding agents working in this repository.

## 📚 Examples and Case Studies

Practical walkthroughs and technical deep-dives:

- **[I2C Sensor Simulation](examples/i2c_sensor_example.md)**: Verify driver code against a mock I2C device.
- **[DMA & Interrupts](examples/dma_exti_example.md)**: Understanding the two-phase execution model.
