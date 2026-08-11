# LabWired Core Documentation

**Deterministic firmware simulation** — the same binary in the browser, in CI, and under an agent. The agent proposes; the **oracle disposes**.

---

## Three doors (start here)

| Path | For | Start |
|------|-----|--------|
| **Playground** | Humans, zero install | [Playground first run](tutorials/playground.md) · [app.labwired.com](https://app.labwired.com) |
| **Agent (MCP)** | Claude Code, Codex, Cursor, … | [Connect MCP](agent/mcp.md) · [First agent run](agent/first-run.md) · [Tools](agent/tools.md) |
| **CLI / CI** | Local sim, pipelines, oracle scripts | [Running firmware](getting_started_firmware.md) · [CI integration](ci_integration.md) |

**Fidelity:** [What a green pass means](fidelity.md) · [Scoreboards](coverage/chip-conformance.md)

---

## Boards & parts

- **[Boards](boards/esp32c3.md)** — per-MCU pins, artifact format, ✅/⚠️/❌ matrix (template: [ESP32-C3](boards/esp32c3.md))
- **[Parts](parts/index.md)** — external components (sensors, displays, …)
- **[Board onboarding](board_onboarding_playbook.md)** — add a new chip (contributors)

Popular boards: [ESP32-C3](boards/esp32c3.md) · [nRF52840](boards/nrf52840.md) · [RP2040](boards/rp2040.md) · [STM32F401](boards/stm32f401.md)

---

## Core concepts

- [Architecture overview](architecture.md) — CPU, bus, peripherals
- [Hardware–sim parity](golden_reference.md)
- [Configuration (YAML)](configuration_reference.md)
- [Resource metrics](resource_metrics.md) — flash / RAM footprint and main-stack budgets in `labwired test`
- [Target support rubric](target_support_rubric.md)

---

## Tutorials & examples

- [Simulating sensors (I²C)](examples/i2c_sensor_example.md)
- [DMA & interrupts](examples/dma_exti_example.md)
- [Integrated test walkthrough](examples/integrated_test_walkthrough.md)
- [Resource metrics examples](../examples/metrics/README.md) — STM32F103 blinky flash / stack budgets

---

## Debugging

- [VS Code](vscode_debugging.md) · [Native DAP](debugging.md) · [GDB](gdb_integration.md)

---

## Contributing (engine)

- [Core agents manual](agents.md) — AI agents working **in this repository**
- [Peripheral modeling](peripherals.md) · [Declarative registers](declarative_registers.md)
- [Release strategy](release_strategy.md)

---

## Product

Simulation for registered users is not cycle-metered on the product side; hosted agent tokens follow your plan on [labwired.com](https://labwired.com). This site documents **how the twin works** and **how to drive it**.
