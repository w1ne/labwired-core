# Multi-node worlds

LabWired is not limited to one MCU per run. A **world** is several machines
stepped together, linked by **interconnects**, driven by an **environment
manifest** and optional **environment test script**.

This page maps **what already ships** (not a wishlist). Use it as the product
story: *env YAML → N nodes → link → oracle*.

---

## Three layers

| Layer | What | Entry |
|-------|------|--------|
| **1. Environment manifest** | Topology: nodes + interconnects | `EnvironmentManifest` YAML |
| **2. World runner** | Load ELFs, lockstep, tick links | `World::from_manifest`, CLI env test |
| **3. Proofs** | Real firmware crossing a link | Examples + e2e tests below |

Single-board Playground labs and multi-node **worlds** share the same per-node
`system.yaml` shape. Connectivity is **never** implied by CLI flags — only by
explicit interconnects.

---

## Layer 1 — Environment manifest

```yaml
schema_version: "1.0"
name: "two-node-smoke"
nodes:
  - id: alpha
    system: "path/to/system.yaml"
    firmware: "path/to/a.elf"
  - id: beta
    system: "path/to/system.yaml"
    firmware: "path/to/b.elf"
interconnects:
  - type: uart_cross_link   # or can_bus, egress, …
    nodes: [alpha, beta]
    # config: { … }          # type-specific, closed schema
```

**Shipped interconnect types** (validated before the world starts):

| Type | Nodes | Role |
|------|-------|------|
| `uart_cross_link` | exactly 2 | Cross-wire named UARTs (default `uart2`) |
| `can_bus` | ≥ 2 | Shared CAN; `config.peripheral` required |
| `egress` | exactly 1 | Host-facing UART egress (TCP / MQTT / HTTP) |

Schema details and assertion rules: [CI test runner — environment scripts](../ci_test_runner.md).

!!! note "Arch contract for env scripts"
    Documented environment-test path is **Cortex-M** oriented for
    node-qualified `memory_value` assertions (see test runner). Multi-node
    **RISC-V / ESP32-C3** proofs exist as dedicated examples and e2e gates
    (below); use those entry points when not on the Cortex-M env contract.

---

## Layer 2 — How to run

### CLI (environment test)

```yaml
# test.yaml
schema_version: "1.0"
inputs:
  env: "two-node-env.yaml"
limits:
  max_steps: 100000
  wall_time_ms: 5000
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
      size: 8
```

```bash
cargo run -p labwired-cli -- test \
  --script path/to/test.yaml \
  --output-dir out/world-run
```

Artifacts: environment `result.json` / snapshot with **per-node** provenance
(`run_type: environment`). See [CLI](../cli_reference.md) and
[CI integration](../ci_integration.md).

### In-tree smoke fixture

| File | Role |
|------|------|
| [`examples/ci/two-node-env.yaml`](../../examples/ci/two-node-env.yaml) | Two fixture nodes, no interconnect (topology smoke) |
| [`examples/ci/two-node-inputs-env.yaml`](../../examples/ci/two-node-inputs-env.yaml) | Test script pointing at that env |

### Engine API

`World::from_manifest` / `add_machine` / `add_interconnect` / `step_all` —
`crates/core/src/world.rs`.

---

## Layer 3 — Shipped multi-node proofs

These are **not** “blinky on N boards.” They exercise links and stacks.

### Wired multi-MCU

| Proof | Location | What it proves |
|-------|----------|----------------|
| Two ESP32-C3s, UART PING/PONG | [`examples/ci-two-c3-link`](../../examples/ci-two-c3-link), [`world_esp32c3_pingpong`](../../crates/core/tests/world_esp32c3_pingpong.rs) | Cross-chip serial + C3 UART model |
| Human-readable lab | [`examples/esp32c3-pingpong`](../../examples/esp32c3-pingpong) | Same idea with Arduino + OLED |
| IO-Link multi-chip station | [`world_multichip`](../../crates/core/tests/world_multichip.rs), `examples/iolink-station` | N Cortex-M nodes + UART links |
| CAN multi-node | [`world_can_bus`](../../crates/core/tests/world_can_bus.rs) | FDCAN traffic across machines |

### Wireless / radio

| Proof | Location | What it proves |
|-------|----------|----------------|
| **ESP32-C3 BLE two-node e2e** | [`e2e_esp32c3_ble_two_node`](../../crates/cli/tests/e2e_esp32c3_ble_two_node.rs) | Real Arduino-ESP32 flash both ways over **BLE air** (adv → stack → app) |
| BLE air model | `peripherals/ble_air.rs` | Channel + access-address select, broadcast |
| nRF52 virtual air | `peripherals/nrf52/radio.rs` `VirtualAirBus` | Cross-instance RADIO TX/RX, MODE/address match |
| **RfMedium** (path loss) | `peripherals/rf_medium.rs` | Seeded path loss, capture, PER, frame trace |
| nRF RADIO + medium | optional `VirtualAirBus::attach_medium` | Distance can **drop** frames; RSSI tracks distance |
| Wi‑Fi twin | `wifi_mac`, `virtual_wifi*`, [`e2e_labwired_wifi`](../../crates/core/tests/e2e_labwired_wifi.rs) | Associate + HTTP against **in-sim AP** (feature `wifi-thunks`) |
| Wi‑Fi docs | [ESP32-C3 Wi‑Fi MAC bridge](../esp32c3_wifi_mac_bridge.md) | Fidelity notes |

### Agent path

Use MCP on a **single** board today for describe/run/verify; multi-node worlds
are primarily **CLI / CI / engine** today. Connecting world runs to MCP is a
product follow-up — the twin already supports multi-node offline.

[Connect MCP](../agent/mcp.md) · [Verify habit](../agent/first-run.md)

---

## Mental model vs peers

| Capability | LabWired today |
|------------|----------------|
| Multi-machine lockstep | **Yes** — `World` |
| UART / CAN interconnect | **Yes** — env interconnect types |
| Two real C3 stacks talking BLE | **Yes** — e2e gate |
| Path-loss RF science | **Yes** — `RfMedium` (+ optional nRF attach) |
| One YAML “RF room” in env manifests | **Not yet** — topic: manifest `rf:` |
| One medium for nRF + BLE PDU + Wi‑Fi frames | **Partial** — separate airs; unify next |
| Electrical / analog board physics | **Not claimed** |

---

## Operator checklist

1. Pick a **proof** from the tables (UART C3, CAN, BLE two-node, or env smoke).  
2. Prefer **oracle / assertions** over “Serial looked fine.”  
3. For radio work: read the module headers (what is faithful vs idealized).  
4. For CI: environment scripts write **environment** result schema — don’t mix with single-machine assumptions.

---

## Related

- [CI test runner](../ci_test_runner.md) — env script contract  
- [CI integration](../ci_integration.md)  
- [Configuration](../configuration_reference.md)  
- [Fidelity](../fidelity.md)  
- [ESP32-C3 board](../boards/esp32c3.md) · [nRF52840](../boards/nrf52840.md)  

---

## Topic: env-manifest `rf:` (path loss)

Optional block on the environment manifest. Seeds a shared **`RfMedium`** on the
`World` (path loss / RSSI floor / node positions).

```yaml
schema_version: "1.0"
name: "two-radio"
nodes:
  - id: alpha
    system: "…"
    firmware: "…"
  - id: beta
    system: "…"
    firmware: "…"
rf:
  seed: 42
  rssi_floor_dbm: -70.0        # optional
  path_loss_exponent: 2.0      # optional
  ref_loss_db: 40.0            # optional
  nodes:
    alpha: { x: 0.0, y: 0.0 }
    beta:  { x: 15.0, y: 0.0 } # metres
```

- Unknown `rf.nodes` ids are **rejected** at validate time.
- `World.rf_medium` holds the medium when `rf:` is present.
- **nRF RADIO** can attach the same medium via `VirtualAirBus::attach_medium`
  (unit-tested path-loss drop). Full automatic attach of every radio in a world
  from this block is the next product wire-up.

---

## Topic: three airs (unification map)

Today there are **three** RF-ish media — intentionally different frame types:

| Medium | Module | Frame | Used by |
|--------|--------|-------|---------|
| nRF virtual air | `nrf52/radio.rs` `VirtualAirBus` | Whitened RADIO buffer + MODE/addr | nRF52 RADIO |
| BLE PDU air | `ble_air.rs` | BLE PDU + access address | ESP32-C3 BT |
| Wi‑Fi MAC / virtual AP | `wifi_mac`, `virtual_wifi*` | 802.11 / host-side services | ESP Wi‑Fi |
| Cellular AT (CSQ) | `components/bg770a.rs` | No air frames — reports path-loss CSQ | Quectel BG770A |

**Unification goal:** one **`RfMedium`** decides path loss / collision /
seeded PER; each air remains the correct **frame type** but asks the medium
before deliver. nRF optional attach is step 1; BLE + Wi‑Fi frame path next.

**Cellular (shipped):** BG770A shares the VirtualAirBus medium slot via
`attach_lab_air` (or spins a local medium for single-board labs). `AT+CSQ` /
`AT+QCSQ` map UE↔`cell` distance to CSQ steps; SimInput `range_m` moves the
UE. Optional `rssi` CSQ override is for scripts only.

Do **not** force one bit layout across RADIO / BLE / Wi‑Fi.

---

## Topic: electrical / analog

| Claim | Status |
|-------|--------|
| Digital buses, register twins, sensor **digital** models | **Shipped** |
| Seeded sensor noise / thermal lag | **Shipped** (noise layer / parity pack) |
| SPICE / board-level electrical / EMI | **Not claimed** |
| Full ADC from graph voltage | **Partial / stub** on many chips |

Honest product line: we catch **logic, protocol, multi-node link, and
radio-stack** bugs; **analog and power** stay bench unless a board page says
otherwise. See [Fidelity](../fidelity.md).
