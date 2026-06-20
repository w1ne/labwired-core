# FDCAN multi-node CI gate — design (Sub-project A)

**Date:** 2026-06-20
**Repo:** labwired-core
**Status:** Approved design, ready for implementation plan
**Origin:** Enables udslib issue #58 — run a two-node UDS-over-CAN stack entirely in
simulation, headless, in CI, with no physical hardware.

## 1. Purpose

Let two firmware-running STM32H5 (CortexM) nodes be built from a single manifest,
wired together over a **virtual FDCAN bus**, run **headless** by `labwired test`,
and report a deterministic **pass/fail** that drives the process exit code for CI.

This is the labwired-core half of a two-part program. The downstream half
(Sub-project B, in the udslib repo) puts a real UDS **server (ECU)** firmware on
one node and a real UDS **client (tester)** firmware on the other, and adds a
`e2e-labwired` job to udslib CI. Sub-project A ships and merges to `main` first
and is independently verifiable; it must not depend on udslib.

## 2. What already exists (reused, not rebuilt)

Audited on `feat/iolink-multichip-station` (base branch) and core `main`:

- `network::CanBus` + `Interconnect` — multi-node virtual CAN bus; `tick()`
  broadcasts each transmitted `CanFrame` to all attached nodes
  (`crates/core/src/network/mod.rs`). On `main`.
- `Fdcan::new_with_bus(tx, rx)` — an H5 FDCAN peripheral bound to a bus
  (`crates/core/src/peripherals/fdcan.rs`). On `main`.
- `tests/fdcan.rs` — two H5 FDCAN nodes exchanging classic **and** CAN-FD frames
  over one `CanBus`, proven at the register-poke level. On `main`.
- `World::from_manifest(env, root)` — builds N CortexM nodes from an
  `EnvironmentManifest` and wires `uart_cross_link` interconnects
  (`crates/core/src/world.rs:124`). **Only on `feat/iolink-multichip-station`.**
- `crates/cli` `test --script` — headless runner, bounded `--max-steps` /
  `--max-cycles` / `--wall-time-ms`, CI exit codes. On `main`.
- Assertion primitives `MemoryValueAssertion`, `UartContainsAssertion`,
  `StopReasonAssertion` (`crates/config/src/lib.rs`) and `World::read_u8`.
  On `main` (assertions are currently single-machine).

The only genuinely missing capability is **wiring two firmware nodes' FDCAN
peripherals to a shared bus from a manifest and asserting a per-node result
headless.** Everything else is composition.

## 3. Base-branch dependency (explicit)

`World::from_manifest` lives on `feat/iolink-multichip-station` (10 commits ahead
of `main`, not yet merged). Sub-project A is therefore branched **off that branch**
(`feat/fdcan-multinode-cigate`) and adds CAN on top, so the forward-port is not
duplicated. **A merges to `main` together with, or immediately after, the IO-Link
multichip-station branch.** If that branch stalls, the fallback is to forward-port
only the `from_manifest` core (config types + `World::from_manifest` + the
`uart_cross_link` arm) to a `main`-based branch; the CAN work in §4 is unchanged
either way.

## 4. Components

### A2 — `can_bus` interconnect type
In `World::from_manifest`, handle `InterconnectConfig { type: "can_bus", nodes,
config }`: construct one `CanBus`, call `attach()` once per listed node, register
the bus as a `World` interconnect. Mirrors the existing `uart_cross_link` arm.
Config is currently empty (a future `config` key may carry bus name / bitrate; not
modeled now — YAGNI).

### A3 — FDCAN-to-bus binding at node construction
The node/SoC builder must construct the referenced node's FDCAN via
`new_with_bus(tx, rx)` (the `attach()` channels for that node) instead of the
standalone constructor. Binding is keyed by `(node_id, peripheral = FDCAN1)`. A
node not named by any `can_bus` interconnect keeps its standalone FDCAN
(loopback-capable) unchanged — no regression to single-node labs.

### A4 — Multi-node headless test + per-node assertions
1. **Test-script variant** referencing an `EnvironmentManifest` (multiple nodes +
   interconnects) instead of a single `inputs.firmware` + `inputs.system`. New
   `LoadedTestScript` variant; the existing single-node V1.0 path is untouched.
2. **`node:` selector** (optional `String`) added to `MemoryValueAssertion` and
   `UartContainsAssertion`. Absent → current single-machine behavior. Present →
   resolved against `World.machines[node]` (`read_u8` for memory; the node's UART
   trace for UART). Unknown node id → config error.
3. **Runner**: `run_test` loads the manifest, builds the `World`, steps **all**
   nodes to the limit, then evaluates assertions. Pass → `EXIT_PASS` (0); any fail
   → `EXIT_ASSERT_FAIL` (1). Same exit-code contract CI already relies on.

### A5 — Self-contained proof (the A-level gate)
A minimal two-H5 firmware pair under `examples/fdcan-twonode/` — **not udslib**, a
tiny ping/pong: node `pinger` sends a known CAN frame; node `ponger` receives it,
echoes a transformed reply; `pinger` verifies the reply, writes a result symbol,
and prints a UART summary. Plus:
- a manifest (`env.yaml`) wiring `pinger` + `ponger` over one `can_bus`;
- a test script asserting both the memory result symbol **and** a UART substring;
- a Rust integration test (`tests/fdcan_multinode.rs`) asserting `from_manifest`
  builds the 2-node CAN world and a frame crosses A→B.

This proves the whole headless path before udslib exists.

## 5. Pass/fail seam (decided: BOTH)

The tester firmware reports two ways, by design:
- **Memory result symbol** — `g_test_result` at a fixed address
  (e.g. `0x2000_0000`): `0xA5` = pass, `0xEE` = fail, written once the scenario
  completes. The CLI `MemoryValueAssertion { node, addr, equals }` reads it and it
  **drives the exit code** — deterministic, no parsing, reuses the proven
  `world_multichip.rs` pattern.
- **UART summary** — a human-readable line (e.g. `RESULT PASS 7/7 services ok`)
  asserted via `UartContainsAssertion { node, substring }` and surfaced in CI logs.

Memory symbol is authoritative for the gate; UART is for human triage.

## 6. Data flow

```
tester FDCAN TX ─▶ CanBus.tick() broadcast ─▶ ECU FDCAN RX FIFO0
                                                     │
                              ECU firmware builds reply, FDCAN TX
                                                     │
tester FDCAN RX ◀─ CanBus.tick() broadcast ◀─────────┘
        │
tester verifies ─▶ writes g_test_result + UART summary
        │
CLI reads g_test_result (+ UART) ─▶ process exit code ─▶ CI
```

Note: `CanBus` currently echoes each frame back to its sender (documented harmless
deviation — no source tagging). Firmware must ignore frames with its own TX id;
the proof firmware in A5 does, and the spec calls it out for Sub-project B.

## 7. Out of scope (YAGNI)

- Acceptance filtering, bit-timing, CAN error states (not modeled; not needed for
  ISO-TP-level correctness).
- Dynamic host-driven CAN injection — unnecessary: both nodes run real firmware.
- Bus bitrate/name config keys, >2 nodes per bus, FDCAN2. The `can_bus` arm
  already supports N attachments; only the 2-node case is exercised now.
- Anything udslib-specific (lives in Sub-project B).

## 8. Testing

- `tests/fdcan_multinode.rs` — `from_manifest` builds the 2-node CAN world; a
  frame transmitted by A lands in B's RX (firmware-free, fast).
- The A5 `labwired test --script` run — the headless gate: exit 0 on pass, 1 when
  the ponger firmware is deliberately broken (negative check in the plan).
- Existing single-node tests must stay green (no `node:` selector → unchanged).

## 9. Risks

- **IO-Link branch merge coupling** (§3) — mitigated by the forward-port fallback.
- **FDCAN self-echo** (§6) — mitigated by firmware id-filtering, called out for B.
- **Step ordering** — both nodes step per `world.step_all()`; the bus `tick()`
  must run each step so a frame TX'd in step _n_ is visible to the peer at _n+1_
  (already the `world_multichip` UART contract; reuse it).
```
