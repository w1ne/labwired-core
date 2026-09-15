# Run firmware with the CLI

Install the LabWired CLI, run a real firmware image on a virtual board, and get serial output or a pass/fail test.

LabWired does **not** ship a firmware HAL. Build with the same vendor SDK you use on silicon (ESP-IDF, Arduino, Zephyr, STM32 HAL, Pico SDK, bare metal). The twin models memory-mapped peripherals. There is no `labwired.h`.

Supported ISAs include **ARM Cortex-M**, **RISC-V**, and **Xtensa** (per board). See the [board pages](boards/esp32c3.md) for what each chip models.

---

## 1. Install the CLI

```bash
curl -fsSL https://labwired.com/install.sh | LABWIRED_VERSION=v0.21.0 sh
labwired --help
```

Linux, macOS, and Windows via WSL2. Pin `LABWIRED_VERSION` for reproducible CI.

Optional: clone the repo for examples and configs:

```bash
git clone https://github.com/w1ne/labwired-core && cd labwired-core
```

---

## 2. Quick smoke (no toolchain)

From a clone of `labwired-core`:

```bash
labwired test --script examples/nrf54l15-dk/io-smoke.yaml
```

**Expected:** the script prints board identity lines and exits **0**. Artifacts land next to the run (`result.json`, `uart.log` when UART is used).

---

## 3. Run your firmware

### Interactive run

```bash
labwired run \
  --firmware path/to/firmware.elf \
  --system configs/systems/<board>.yaml
```

Use a system YAML from `configs/systems/` that matches your board. Some ESP targets need a **merged flash image** instead of a lone app ELF — see the board page (example: [ESP32-C3](boards/esp32c3.md)).

### Deterministic test (CI gate)

```bash
labwired test \
  --script path/to/test.yaml \
  --junit report.xml
```

Exit code **0** = assertions passed. Exit **non-zero** = fail. See [Test script schema](ci_test_runner.md) and [CI integration](ci_integration.md).

---

## 4. What you need for a custom board path

| Piece | Role |
|-------|------|
| **Firmware** | ELF (or board-specific flash image) from your normal build |
| **System YAML** | Board wiring: chip + pins + external parts (`configs/systems/`) |
| **Chip YAML** | MCU memory map and peripherals (`configs/chips/`) — already present for supported boards |

Start from an existing system under `configs/systems/` or an `examples/*/`. Do not invent addresses; copy a nearby board and change only what you need.

To **add a new chip or board**, use [Onboard hardware](howto/onboard-hardware.md) and the [Board playbook](board_onboarding_playbook.md).

To **add a sensor or actuator**, use [Onboard a part](howto/onboard-part.md).

---

## 5. Common problems

| Symptom | Likely cause | What to do |
|---------|----------------|------------|
| `MemoryAccessViolation` | Firmware hit an unmapped address | Check board matrix; fix linker script or model gaps |
| Immediate HardFault | Vector table / flash base wrong | Match flash base on the chip page |
| Empty serial | Wrong UART peripheral or pins | Compare system YAML to firmware UART setup |
| Stuck loop | Polling a stub / unmodeled status bit | See ✅/⚠️/❌ on the board page |

More: [Troubleshooting](troubleshooting.md).

---

## Next

| Path | Doc |
|------|-----|
| CI | [CI integration](ci_integration.md) |
| Agent | [Connect MCP](agent/mcp.md) |
| CLI flags | [CLI reference](cli_reference.md) |
| Fidelity | [What a green pass means](fidelity.md) |
