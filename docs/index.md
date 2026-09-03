# LabWired documentation

Run **real firmware** on a **digital twin** of the board — in the browser, from an agent, or in CI. Same models. Deterministic results.

---

## Start here

| Door | You want to… | Start |
|------|----------------|--------|
| **Playground** | Try without installing | [Playground first run](tutorials/playground.md) · [app.labwired.com](https://app.labwired.com) |
| **Agent (MCP)** | Let Claude / Codex / Cursor drive the twin | [Connect MCP](agent/mcp.md) · [First agent run](agent/first-run.md) |
| **CLI / CI** | Run locally or in a pipeline | [Run firmware](getting_started_firmware.md) · [CI](ci_integration.md) |
| **Onboard hardware** | Add a sensor, actuator, or board | [Pick a track](howto/onboard-hardware.md) · [Onboard a part](howto/onboard-part.md) |

**What does a green pass mean?** → [Fidelity](fidelity.md)

---

## Onboard hardware (quick pick)

| Track | Examples | Guide |
|-------|----------|--------|
| **Part** (most common) | I²C sensor, SPI chip, servo, motor, buzzer | [Onboard a part](howto/onboard-part.md) |
| **Board / MCU** | New chip or Nucleo / Pico / ESP board | [Board playbook](board_onboarding_playbook.md) |
| **On-chip peripheral** | New timer / UART model inside a chip | [Peripheral modeling](peripherals.md) |

Parts catalog: [Parts](parts/index.md) · Support levels: [Target rubric](target_support_rubric.md)

---

## Boards (popular)

[ESP32-C3](boards/esp32c3.md) · [ESP32-S3](boards/esp32s3.md) · [nRF52840](boards/nrf52840.md) · [RP2040](boards/rp2040.md) · [RP2350-Zero](boards/rp2350-zero.md) · [STM32F401](boards/stm32f401.md)

---

## More (when you need it)

| Topic | Link |
|-------|------|
| Agent tools | [Tool reference](agent/tools.md) |
| CLI flags | [CLI reference](cli_reference.md) |
| Troubleshooting | [Troubleshooting](troubleshooting.md) |
| YAML config | [Configuration reference](configuration_reference.md) |
| Architecture (engine) | [Architecture](architecture.md) |
| Agents working *in* this repo | [Core agents manual](agents.md) |

---

## Product

Hosted Playground and MCP use the same core models as the open CLI. Plans and tokens: [labwired.com](https://labwired.com).
