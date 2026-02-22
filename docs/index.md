# LabWired Core Documentation

Welcome to the **LabWired Core** documentation. LabWired is a deterministic firmware simulation platform designed to replace physical hardware in CI pipelines.

## 🚀 Getting Started

If you are new to LabWired, start here:

- **[Running Your Firmware](getting_started_firmware.md)**: Learn how to load ELF binaries and execute them in the simulator.
- **[Board Onboarding](board_onboarding_playbook.md)**: Steps to add support for a new microcontroller or board.

## 🧠 Core Concepts

Understand how LabWired achieves deterministic simulation:

- **[Architecture Overview](architecture.md)**: Explains the split between the CPU Core, System Bus, and Peripherals.
- **[Configuration Reference](configuration_reference.md)**: Detailed schema for defining chips and systems (YAML).

## 🛠 Developer Guides

For contributors extending the core engine or adding new peripherals:

- **[Peripheral Development](peripheral_development.md)**: How to implement custom peripheral models in Rust.
- **[Declarative Registers](declarative_registers.md)**: Defining register maps using simple YAML files.
- **[CI Integration](ci_integration.md)**: How to run LabWired in GitHub Actions or GitLab CI.
- **[Coverage Scoreboard](coverage_scoreboard.md)**: Top-target smoke coverage and deterministic status tracking.
- **Onboarding Smoke CI**: `core-onboarding-smoke.yml` publishes time-to-first-smoke metrics and scoreboard artifacts.
- **[Target Support Rubric](target_support_rubric.md)**: Objective support levels and promotion criteria.

## 🔍 Debugging

- **[VS Code Debugging](vscode_debugging.md)**: Recipes for `launch.json`.
- **[Native DAP](debugging.md)**: Architecture of the built-in Debug Adapter.
- **[GDB Integration](gdb_integration.md)**: Using standard GDB clients.

## 📚 Examples

Practical walkthroughs of specific features:

- **[I2C Sensor Simulation](examples/i2c_sensor_example.md)**: Verify driver code against a mock I2C device.
- **[DMA & Interrupts](examples/dma_exti_example.md)**: Understanding the two-phase execution model.
