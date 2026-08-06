# ESP32-C3

Espressif **ESP32-C3** — single-core **RISC-V (RV32IMC)**, ~400 KB SRAM, external QSPI flash, **Wi-Fi** (+ BLE on silicon; see matrix). LabWired's reference **RISC-V / Espressif** target: run the **same binary** you flash to hardware in the oracle, CI, and playground.

This page is the **board-page template** for public docs: status → artifact → pins → support matrix → how to run → honest limits. Copy the section order for other boards.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match
    - [Tier-1 matrix](../coverage/tier1-scoreboard.md) — per-peripheral pass/fail
    - [Target support rubric](../target_support_rubric.md) — modeled / stub / silicon-verified

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/esp32c3.yaml`](../../configs/chips/esp32c3.yaml) |
| Example system | [`configs/systems/esp32c3-devkit.yaml`](../../configs/systems/esp32c3-devkit.yaml) |
| Playground board id | `esp32-c3-supermini` (labs such as `esp32c3-oled-lab`) |
| Reference firmware | [`crates/firmware-esp32c3-demo/`](../../crates/firmware-esp32c3-demo/) |
| Examples | `examples/esp32c3-blinky`, `esp32c3-oled-demo`, weather / thermal workshops under `examples/` |
| Wi-Fi path | Register-level MAC + bridge — [ESP32-C3 Wi-Fi MAC bridge](../esp32c3_wifi_mac_bridge.md) |
| Tier (snapshot) | Full documented SVD estate wired; behavioral depth varies by block (matrix below) |

---

## Flash / firmware artifact

| Use | Artifact | Notes |
|-----|----------|--------|
| **ESP-IDF / esptool path** | Merged flash **`.bin`** (bootloader + partition table + app) | Same image you would write to silicon |
| **Bare / demo crates** | **ELF** where the example links for the C3 map | Demo crates under `crates/firmware-esp32c3-*` |
| **Arduino / PlatformIO** | Build output for the board's compile profile | Hosted compile: board id `esp32-c3-supermini` |

!!! warning "Not STM32-shaped"
    Do not assume a single `0x0800_0000` ELF like Cortex-M. C3 apps use **IROM/DROM/IRAM** windows and often a **flash image**. See chip yaml `memory_regions` and [Running firmware](../getting_started_firmware.md).

**ROM dumps (optional):** mask ROM via `LABWIRED_ESP32C3_ROM` / `LABWIRED_ESP32C3_ROM_DATA` (see `esp32c3.yaml`). Without them those windows stay zero; many user apps still boot.

---

## Pins (Super Mini / common playground mapping)

Canonical playground MCU: **ESP32-C3 Super Mini** (`esp32-c3-supermini`). Silkscreen **D0–D10** map to GPIO; firmware should use the **GPIO matrix**.

| Board label | GPIO | Typical default | Notes |
|-------------|------|-----------------|--------|
| D0 | GPIO2 | A0 / GPIO | |
| D1 | GPIO3 | A1 / GPIO | |
| D2 | GPIO4 | A2 / GPIO | |
| D3 | GPIO5 | A3 / GPIO | |
| D4 | GPIO6 | I2C SDA (common) | Remappable via matrix |
| D5 | GPIO7 | I2C SCL (common) | Remappable |
| D6 | GPIO21 | UART TX | |
| D7 | GPIO20 | UART RX | |
| D8 | GPIO8 | SPI SCK / **user LED** (active-low on Super Mini) | |
| D9 | GPIO9 | SPI MISO | |
| D10 | GPIO10 | SPI MOSI | |
| 3V3 / 5V / GND | — | Power | Digital levels in sim — not SPICE |

---

## Support matrix

| Mark | Meaning |
|------|---------|
| ✅ | Modeled well enough for real drivers / demos we ship |
| ⚠️ | Present but partial, stubbed, or easy to misuse |
| ❌ | Not simulated — use the bench |

### Core & boot

| Block | Status | Notes |
|-------|--------|--------|
| RV32IMC CPU | ✅ | Instruction-level; unsupported ops audited |
| Memory map (SRAM, IROM, DROM, IRAM, RTC FAST) | ✅ | Matches IDF link layout |
| Mask ROM | ⚠️ | Optional image via env |
| Reset / clocks | ✅ / ⚠️ | Enough for boot + timers |

### Console, GPIO, timers

| Block | Status | Notes |
|-------|--------|--------|
| UART0 / UART1 | ✅ | Espressif UART IP + FIFOs (`esp32c3_uart`) — not STM32 UART |
| GPIO + IO MUX | ✅ | Super Mini LED on GPIO8 active-low |
| TIMG0 / TIMG1 | ✅ | |
| SYSTIMER | ✅ | FreeRTOS-style tick source |
| USB device (USB-Serial/JTAG) | ⚠️ | Do not assume full USB enumeration |

### Buses

| Block | Status | Notes |
|-------|--------|--------|
| I2C0 | ✅ | Wire external sensors in system yaml / playground |
| SPI0 / SPI1 / SPI2 | ⚠️ | Controller modeled; not every external SPI device fully bridged |
| I2S0, RMT, LEDC, TWAI, UHCI, GDMA | ⚠️ | Wired in yaml; check tier-1 scoreboard |

### Radio & network

| Block | Status | Notes |
|-------|--------|--------|
| Wi-Fi (MAC + FE path) | ✅ / ⚠️ | [Wi-Fi MAC bridge](../esp32c3_wifi_mac_bridge.md); sim network, not RF PHY cert |
| BLE / BT | ❌ | On silicon; not a C3 LabWired deliverable today |
| Packet-level multi-node RF room | ⚠️ | Separate product work; do not over-claim here |

### ADC, crypto, misc

| Block | Status | Notes |
|-------|--------|--------|
| SAR ADC | ⚠️ | No full analog conversion from graph voltage |
| Crypto blocks / eFuse | ⚠️ | Present in map; verify scoreboard before CI reliance |

---

## What it catches vs what needs a bench

**Sim is strong for:** logic/state-machine bugs; UART/GPIO/timer/I2C bring-up; deterministic CI; Wi-Fi **stack** along the documented bridge.

**Still use silicon for:** analog, power, antenna/EMI, BLE, USB edge cases, flash wear, production sign-off.

See [Fidelity](../fidelity.md), [Hardware-sim parity](../golden_reference.md), [Run limits](../limits.md).

---

## How to run

### CLI (oracle / CI)

```bash
cargo run -p labwired-cli -- test \
  --script path/to/esp32c3-test.yaml \
  --output-dir out/esp32c3-run
```

```bash
cargo run -p labwired-cli -- run \
  --firmware path/to/firmware.elf \
  --system configs/systems/esp32c3-devkit.yaml
```

### Playground

1. Open a C3 lab on [app.labwired.com](https://app.labwired.com).
2. Board: **ESP32-C3 Super Mini**.
3. Build or upload the correct artifact.
4. Assert on GPIO / UART / display / oracle — not on ❌ features.

### Agent (MCP)

1. [Connect MCP](../agent/mcp.md).
2. `labwired_describe` → `esp32-c3-supermini` or `esp32c3`.
3. `labwired_compile` (hosted) / artifact (stdio) → `labwired_run` → **`labwired_verify`**.

Details: [First agent run](../agent/first-run.md) · [Tools](../agent/tools.md).

---

## Related systems & examples

| Path | What |
|------|------|
| `configs/systems/esp32c3-devkit.yaml` | Baseline system |
| `configs/systems/esp32c3-oled-demo.yaml` | Display lab |
| `examples/esp32c3-blinky` | Minimal bring-up |
| `docs/esp32c3_wifi_mac_bridge.md` | Wi-Fi fidelity |
| [Parts index](../parts/index.md) | External components |

---

## Template checklist (other board pages)

1. Status at a glance · 2. Flash/artifact · 3. Pins · 4. Support matrix · 5. Catch vs bench · 6. CLI · Playground · MCP · 7. Related

No competitor names, pricing strategy, or internal roadmap on board pages.
