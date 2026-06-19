# FDCAN multi-node CI gate — Implementation Plan (Sub-project A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let two firmware-running STM32H5 nodes be built from one manifest, wired over a virtual FDCAN bus, run headless by `labwired test`, and report a deterministic per-node pass/fail that drives the CI exit code.

**Architecture:** Reuse the existing `network::CanBus` interconnect and `Fdcan` peripheral. Add a post-construction `attach_bus` path (peripherals are built before interconnects wire), a `can_bus` arm in `World::from_manifest` mirroring the existing `uart_cross_link` arm, a per-node selector on the existing assertion structs, and a multi-node test-script variant in the CLI runner. A throwaway-but-kept two-firmware ping/pong proves the whole headless path.

**Tech Stack:** Rust (labwired-core workspace: `crates/core`, `crates/config`, `crates/cli`), `goblin` for ELF, `std::sync::mpsc` for the bus, `arm-none-eabi-gcc` for the proof firmwares.

## Global Constraints

- Repo: **labwired-core**, MIT-licensed; license header on every new `.rs` file (copy the header block from `crates/core/src/world.rs:1-5`).
- Branch: `feat/fdcan-multinode-cigate`, based on `feat/iolink-multichip-station` (where `from_manifest` lives). PRs target **`main`**; no `develop` branch exists. Merges to `main` with/after the IO-Link multichip branch.
- No competitor/Claude/AI references in commits or files. Commit author identity is the machine default (w1ne noreply).
- Existing single-node behavior must stay green: the `node:` assertion selector and the multi-node test variant are **additive** — absent `node` ⇒ current single-machine path unchanged.
- The CLI free-tier path (no `LABWIRED_API_KEY`) must run this gate with no HTTP and no cycle-quota gate. Do not require a key.
- FDCAN model caveat (carry into firmware): `CanBus` echoes each TX frame back to its sender; firmware must ignore frames bearing its own TX id.
- Scope boundary: **no udslib here.** The proof firmwares are a tiny ping/pong, not UDS.

---

### Task 0: Spike — second-firmware FDCAN TX reaches a peer over `CanBus`, headless (go/no-go)

**Exploratory, not TDD.** Goal: kill the one unproven link before building anything else. Existing `h563-uds-ecu` proves RX-from-static-injector; `tests/fdcan.rs` proves register-level two-node exchange. Unproven: a *firmware* driving FDCAN TX, delivered to a *second firmware* node's RX over the `CanBus` interconnect, run by the multi-node `World`.

**Files (spike, may be hacky; deleted or promoted in Task 6):**
- Scratch: `crates/core/tests/spike_fdcan_firmware_twonode.rs`
- Reuse firmware: `examples/h563-uds-ecu/firmware/` as the receiver; a 30-line TX-only sender firmware (hand-write or trim the ECU `main.c`).

- [ ] **Step 1:** Write a Rust test that builds two `Machine`s from `configs/chips/stm32h563.yaml` (as `from_manifest` does at `crates/core/src/world.rs:140-160`), constructs a `CanBus`, and for each machine binds its `fdcan1` to a bus endpoint. Since `attach_bus` does not exist yet, for the spike construct the FDCAN via `Fdcan::new_with_bus(tx, rx)` directly inside a minimal `SystemBus` or poke the peripheral after build — whatever is fastest to get a frame across. Sender firmware: on boot, write FDCAN TXBC/TX buffer to emit ID `0x123` payload `[0xDE,0xAD]`. Receiver: the ECU firmware (or a trimmed copy) that, on RX, writes a byte to a known SRAM location.
- [ ] **Step 2:** Step the world ~50k cycles. Assert the receiver machine's SRAM marker shows the frame arrived (`machine.read_u8(addr)`).
- [ ] **Step 3 (go/no-go):** If the frame crosses firmware→bus→firmware, **GO** — record in the commit message the exact addresses/IDs that worked, then proceed to Task 1. If it does **not** cross, STOP and report: the gap is in the firmware↔bus drain (`crates/core/src/peripherals/fdcan.rs:474` TX push, `:637` RX drain), not in the higher-level wiring — fix that first.
- [ ] **Step 4: Commit** the spike under a clear name so it can be promoted or deleted in Task 6.

```bash
git add crates/core/tests/spike_fdcan_firmware_twonode.rs
git commit -m "spike: prove firmware-driven FDCAN TX crosses CanBus to a peer firmware"
```

---

### Task 1: `Fdcan::attach_bus` — post-construction bus binding

**Files:**
- Modify: `crates/core/src/peripherals/fdcan.rs` (add method near `new_with_bus`, ~`:266`)
- Test: `crates/core/src/peripherals/fdcan.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: existing fields `bus_tx: Option<Sender<CanFrame>>` (`:215`), `bus_rx: Option<Receiver<CanFrame>>` (`:217`).
- Produces: `pub fn attach_bus(&mut self, tx: Sender<CanFrame>, rx: Receiver<CanFrame>)` — used by Task 2.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn attach_bus_binds_endpoints_after_construction() {
    use crate::network::CanBus;
    let mut bus = CanBus::new();
    let (tx, rx) = bus.attach();
    let mut dev = Fdcan::new();          // standalone, no bus
    assert!(dev.bus_tx.is_none());
    dev.attach_bus(tx, rx);
    assert!(dev.bus_tx.is_some());
    assert!(dev.bus_rx.is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core attach_bus_binds_endpoints -- --nocapture`
Expected: FAIL — `no method named attach_bus`.

- [ ] **Step 3: Write minimal implementation**

```rust
/// Bind this FDCAN to a `CanBus` interconnect after construction. Mirrors
/// `new_with_bus` but for peripherals already built by `SystemBus::from_config`
/// (interconnects are wired after the bus is built).
pub fn attach_bus(&mut self, tx: Sender<CanFrame>, rx: Receiver<CanFrame>) {
    self.bus_tx = Some(tx);
    self.bus_rx = Some(rx);
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-core attach_bus_binds_endpoints`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/peripherals/fdcan.rs
git commit -m "feat(fdcan): post-construction attach_bus for CanBus binding"
```

---

### Task 2: Bus + `MachineTrait` CAN attach by id

**Files:**
- Modify: `crates/core/src/bus/mod.rs` (add `attach_can_bus_by_id`, near the fdcan handling at `:393`)
- Modify: `crates/core/src/world.rs` (add `attach_can_bus` to `MachineTrait` ~`:29`, impl ~`:67`)
- Test: `crates/core/src/bus/mod.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `Fdcan::attach_bus` (Task 1); `find_peripheral_index_by_name` (`crates/core/src/bus/mod.rs`, used at `:1290`); `downcast_mut::<Fdcan>` (pattern at `:393-396`).
- Produces:
  - `SystemBus::attach_can_bus_by_id(&mut self, can_id: &str, tx: Sender<CanFrame>, rx: Receiver<CanFrame>) -> anyhow::Result<()>`
  - `MachineTrait::attach_can_bus(&mut self, can_id: &str, tx: Sender<CanFrame>, rx: Receiver<CanFrame>) -> anyhow::Result<()>` — used by Task 3.

- [ ] **Step 1: Write the failing test** (in `bus/mod.rs` tests, mirroring `test_from_config_can_diagnostic_tester_injects_frame_into_fdcan` at `:1249`)

```rust
#[test]
fn attach_can_bus_by_id_binds_named_fdcan() {
    use crate::network::CanBus;
    // Build a bus from a minimal chip+system declaring fdcan1 (reuse the YAML
    // literal at bus/mod.rs:1262 in the existing injector test).
    let mut bus = build_test_bus_with_fdcan1();   // helper extracted from :1262 test
    let mut can = CanBus::new();
    let (tx, rx) = can.attach();
    bus.attach_can_bus_by_id("fdcan1", tx, rx).expect("attach");
    let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
    let fdcan = bus.peripherals[idx].dev.as_any()
        .and_then(|a| a.downcast_ref::<crate::peripherals::fdcan::Fdcan>()).unwrap();
    assert!(fdcan.bus_tx.is_some(), "fdcan1 must be bound to the bus");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core attach_can_bus_by_id_binds_named_fdcan`
Expected: FAIL — `no method named attach_can_bus_by_id`.

- [ ] **Step 3: Write minimal implementation**

In `crates/core/src/bus/mod.rs` (mirror the downcast at `:393-396`):

```rust
/// Bind the named FDCAN peripheral to a `CanBus` interconnect endpoint.
pub fn attach_can_bus_by_id(
    &mut self,
    can_id: &str,
    tx: std::sync::mpsc::Sender<crate::network::CanFrame>,
    rx: std::sync::mpsc::Receiver<crate::network::CanFrame>,
) -> anyhow::Result<()> {
    let idx = self.find_peripheral_index_by_name(can_id)
        .ok_or_else(|| anyhow::anyhow!("no peripheral named '{can_id}'"))?;
    let fdcan = self.peripherals[idx].dev.as_any_mut()
        .and_then(|a| a.downcast_mut::<crate::peripherals::fdcan::Fdcan>())
        .ok_or_else(|| anyhow::anyhow!("peripheral '{can_id}' is not an FDCAN"))?;
    fdcan.attach_bus(tx, rx);
    Ok(())
}
```

In `crates/core/src/world.rs`, add to `MachineTrait` (mirror `attach_uart_stream` at `:29` / `:67`):

```rust
fn attach_can_bus(
    &mut self,
    can_id: &str,
    tx: std::sync::mpsc::Sender<crate::network::CanFrame>,
    rx: std::sync::mpsc::Receiver<crate::network::CanFrame>,
) -> anyhow::Result<()>;
```

and impl for `Machine<C>`:

```rust
fn attach_can_bus(
    &mut self,
    can_id: &str,
    tx: std::sync::mpsc::Sender<crate::network::CanFrame>,
    rx: std::sync::mpsc::Receiver<crate::network::CanFrame>,
) -> anyhow::Result<()> {
    self.bus.attach_can_bus_by_id(can_id, tx, rx)
}
```

(If `as_any_mut` does not exist on the peripheral wrapper, add it alongside the existing `as_any` used at `:317` — one-line mirror.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-core attach_can_bus_by_id_binds_named_fdcan`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/bus/mod.rs crates/core/src/world.rs
git commit -m "feat(bus): attach named FDCAN to a CanBus via attach_can_bus_by_id"
```

---

### Task 3: `can_bus` interconnect arm in `World::from_manifest`

**Files:**
- Modify: `crates/core/src/world.rs` (the `match ic.r#type.as_str()` block at `:175-205`)
- Test: `crates/core/tests/fdcan_multinode.rs` (new)

**Interfaces:**
- Consumes: `MachineTrait::attach_can_bus` (Task 2); `CanBus::new`/`attach`/as `Interconnect` (`crates/core/src/network/mod.rs`).
- Produces: support for `InterconnectConfig { type: "can_bus", nodes, config }` in `from_manifest`.

- [ ] **Step 1: Write the failing test** — a 2-node manifest over `can_bus`, frame crosses A→B, plus determinism (roast #8b).

```rust
// crates/core/tests/fdcan_multinode.rs
use labwired_config::{EnvironmentManifest, InterconnectConfig, NodeConfig};
use labwired_core::world::World;
use std::collections::HashMap;
use std::path::PathBuf;

fn root() -> PathBuf { PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/fdcan-twonode")) }

#[test]
fn from_manifest_builds_two_can_nodes_and_frame_crosses() {
    if !root().join("sender/firmware/build/sender.elf").exists() { eprintln!("SKIP: build firmwares first"); return; }
    let env = EnvironmentManifest {
        schema_version: "1".into(), name: "fdcan2".into(),
        nodes: vec![
            NodeConfig { id: "sender".into(),   system: "sender/system.yaml".into(),   firmware: "sender/firmware/build/sender.elf".into(),   config_overrides: HashMap::new() },
            NodeConfig { id: "receiver".into(), system: "receiver/system.yaml".into(), firmware: "receiver/firmware/build/receiver.elf".into(), config_overrides: HashMap::new() },
        ],
        interconnects: vec![InterconnectConfig { r#type: "can_bus".into(), nodes: vec!["sender".into(), "receiver".into()], config: HashMap::new() }],
    };
    let mut world = World::from_manifest(env, &root()).expect("build can world");
    assert_eq!(world.machines.len(), 2);
    // receiver firmware writes 0xA5 to RECV_MARKER (0x2000_0004) on RX of id 0x123.
    const RECV_MARKER: u64 = 0x2000_0004;
    let mut got = 0u8;
    for _ in 0..200_000 { world.step_all(); got = world.machines.get("receiver").unwrap().read_u8(RECV_MARKER).unwrap(); if got == 0xA5 { break; } }
    assert_eq!(got, 0xA5, "receiver never saw the sender's CAN frame over the bus");
}

#[test]
fn from_manifest_unknown_can_node_errors() {
    let env = EnvironmentManifest {
        schema_version: "1".into(), name: "bad".into(), nodes: vec![],
        interconnects: vec![InterconnectConfig { r#type: "can_bus".into(), nodes: vec!["ghost".into()], config: HashMap::new() }],
    };
    let err = World::from_manifest(env, &root()).unwrap_err().to_string();
    assert!(err.contains("ghost"), "must name the unknown node, got: {err}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core --test fdcan_multinode`
Expected: FAIL — `unsupported interconnect type 'can_bus'` (from the `other =>` arm at `world.rs:203`).

- [ ] **Step 3: Write minimal implementation** — add a `"can_bus"` arm before `other =>`, mirroring `uart_cross_link` (`:178-202`):

```rust
"can_bus" => {
    let mut can = crate::network::CanBus::new();
    for node_id in &ic.nodes {
        let can_uart = ic.config.get("peripheral").and_then(|v| v.as_str()).unwrap_or("fdcan1");
        let (tx, rx) = can.attach();
        world.machines.get_mut(node_id)
            .with_context(|| format!("can_bus: unknown node '{node_id}'"))?
            .attach_can_bus(can_uart, tx, rx)?;
    }
    world.add_interconnect(Box::new(can));
}
```

- [ ] **Step 4: Run test to verify it passes** (after Task 6 firmwares exist; until then `from_manifest_unknown_can_node_errors` must pass and the crossing test SKIPs cleanly)

Run: `cargo test -p labwired-core --test fdcan_multinode`
Expected: `from_manifest_unknown_can_node_errors` PASS; crossing test SKIP (no ELF yet) or PASS (after Task 6).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/world.rs crates/core/tests/fdcan_multinode.rs
git commit -m "feat(world): can_bus interconnect — wire N nodes' FDCAN to a shared bus from manifest"
```

---

### Task 4: Per-node assertion selector + ELF-symbol memory addressing

**Files:**
- Modify: `crates/config/src/lib.rs` (`MemoryValueDetails` ~`:597`, `UartContainsAssertion` ~`:565`)
- Test: `crates/config/src/lib.rs` `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `MemoryValueDetails.node: Option<String>` and `MemoryValueDetails.symbol: Option<String>` (resolve against that node's ELF; `address` becomes optional when `symbol` is set).
  - `UartContainsAssertion.node: Option<String>`.
  - All `#[serde(default)]` so existing single-node scripts parse unchanged.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn memory_value_accepts_node_and_symbol() {
    let yaml = r#"
memory_value:
  node: tester
  symbol: g_test_result
  expected_value: 0xA5
  size: 8
"#;
    let a: MemoryValueAssertion = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(a.memory_value.node.as_deref(), Some("tester"));
    assert_eq!(a.memory_value.symbol.as_deref(), Some("g_test_result"));
    assert_eq!(a.memory_value.expected_value, 0xA5);
}

#[test]
fn memory_value_legacy_address_still_parses() {
    let yaml = "memory_value:\n  address: 0x20000000\n  expected_value: 1\n";
    let a: MemoryValueAssertion = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(a.memory_value.address, Some(0x2000_0000));
    assert!(a.memory_value.node.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-config memory_value_accepts_node_and_symbol`
Expected: FAIL — unknown field `node`/`symbol` (or `address` type mismatch).

- [ ] **Step 3: Write minimal implementation**

```rust
pub struct MemoryValueDetails {
    #[serde(default)]
    pub node: Option<String>,
    /// Resolve from the node's ELF symbol table; takes precedence over `address`.
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub address: Option<u64>,
    pub expected_value: u64,
    #[serde(default)]
    pub mask: Option<u64>,
    #[serde(default)]
    pub size: Option<u8>,
}
```

Add `#[serde(default)] pub node: Option<String>` to `UartContainsAssertion`. Update the single-node assertion evaluator (`crates/cli/src/commands/test.rs`, the `&assertions` use at `:298`/`:425`) so a `None` node resolves to the sole machine (unchanged behavior); fix the now-`Option<u64>` `address` call sites in the evaluator to `address.expect("address or symbol required")` after symbol resolution.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-config memory_value`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add crates/config/src/lib.rs crates/cli/src/commands/test.rs
git commit -m "feat(config): per-node + ELF-symbol selectors on memory/uart assertions (additive)"
```

---

### Task 5: Multi-node test-script variant + CLI runner evaluates per-node assertions

**Files:**
- Modify: `crates/config/src/lib.rs` (`LoadedTestScript` enum ~`:697`)
- Modify: `crates/cli/src/commands/test.rs` (`run_test`, the loaded-script match at `:73-105`)
- Test: `crates/cli/tests/` (new integration test invoking the binary) or a unit test on the loader.

**Interfaces:**
- Consumes: `EnvironmentManifest`, `World::from_manifest` (Task 3), per-node assertions (Task 4), ELF symbol resolution (reuse `goblin` as in `world.rs:load_elf_image`).
- Produces: a `LoadedTestScript::Env` variant carrying `{ env: EnvironmentManifest path, limits, assertions }`; `run_test` builds a `World`, steps to limit, evaluates each assertion against `world.machines[assertion.node]`, returns `EXIT_PASS`/`EXIT_ASSERT_FAIL`.

- [ ] **Step 1: Write the failing test** — loader round-trips a multi-node script.

```rust
#[test]
fn loads_multinode_env_test_script() {
    let yaml = r#"
schema_version: "1.0"
inputs:
  env: "./env.yaml"
limits: { max_steps: 200000 }
assertions:
  - memory_value: { node: tester, symbol: g_test_result, expected_value: 0xA5, size: 8 }
  - uart_contains: { node: tester, value: "RESULT PASS" }
"#;
    match load_test_script_str(yaml).unwrap() {
        LoadedTestScript::Env(s) => { assert!(s.inputs.env.ends_with("env.yaml")); assert_eq!(s.assertions.len(), 2); }
        _ => panic!("expected Env variant"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-config loads_multinode_env_test_script`
Expected: FAIL — no `Env` variant / `inputs.env` unknown.

- [ ] **Step 3: Write minimal implementation** — add the `Env` variant (an `inputs.env` path instead of `inputs.firmware`+`inputs.system`), and in `run_test` branch on it: load the manifest, `World::from_manifest`, step `max_steps`, resolve each assertion's `node` (default = sole machine) and `symbol`→address via the node's ELF, evaluate, set exit code. Keep the existing `V1_0` arm byte-for-byte unchanged.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-config loads_multinode_env_test_script`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/config/src/lib.rs crates/cli/src/commands/test.rs
git commit -m "feat(cli): multi-node env test scripts with per-node assertions"
```

---

### Task 6: `fdcan-twonode` example + headless proof + determinism test

**Files:**
- Create: `examples/fdcan-twonode/{env.yaml, test.yaml, README.md}`
- Create: `examples/fdcan-twonode/sender/{system.yaml, firmware/{main.c,startup.c,minimal.ld,Makefile}}`
- Create: `examples/fdcan-twonode/receiver/{system.yaml, firmware/...}` (trim from `examples/h563-uds-ecu/firmware/`)
- Promote/replace: the Task 0 spike test → delete `spike_fdcan_firmware_twonode.rs`; the kept proof is `crates/core/tests/fdcan_multinode.rs` (Task 3) + the headless `test.yaml`.

**Interfaces:**
- Consumes: everything above. Receiver writes `0xA5` to `g_recv_marker` (`0x2000_0004`) on RX of id `0x123`; sender writes `g_test_result=0xA5` to `0x2000_0000` after it sees the receiver's echo, and prints `RESULT PASS` to UART. Both id-filter their own TX (self-echo caveat).

- [ ] **Step 1:** Write the sender/receiver firmwares (mirror `h563-uds-ecu/firmware` startup+linker; `minimal.ld` pins `g_test_result`/`g_recv_marker` to a fixed `.noinit` section at `0x2000_0000`/`0x2000_0004` so the raw-address assertion is stable even without symbol resolution). `make -C examples/fdcan-twonode/sender/firmware && make -C .../receiver/firmware`.
- [ ] **Step 2:** Write `env.yaml` (two nodes + one `can_bus` interconnect) and `test.yaml`:

```yaml
schema_version: "1.0"
inputs: { env: "./env.yaml" }
limits: { max_steps: 200000 }
assertions:
  - memory_value: { node: sender, symbol: g_test_result, expected_value: 0xA5, size: 8 }
  - uart_contains: { node: sender, value: "RESULT PASS" }
  - memory_value: { node: receiver, address: 0x20000004, expected_value: 0xA5, size: 8 }
```

- [ ] **Step 3: Run the headless gate**

Run: `cargo run -p labwired-cli -- test --script examples/fdcan-twonode/test.yaml`
Expected: exit `0`; logs show `RESULT PASS`.

- [ ] **Step 4: Negative check** — temporarily break the receiver (drop the RX handler), rebuild, rerun.
Expected: exit `1`, assertion failure names `g_test_result`/`receiver`. Revert the break.

- [ ] **Step 5:** Run the Rust integration test (now un-SKIPped) and the full suite to confirm no single-node regressions.

Run: `cargo test -p labwired-core --test fdcan_multinode && cargo test --workspace`
Expected: all PASS; delete the spike test file.

- [ ] **Step 6: Commit**

```bash
git add examples/fdcan-twonode crates/core/tests/fdcan_multinode.rs
git rm crates/core/tests/spike_fdcan_firmware_twonode.rs
git commit -m "feat(example): fdcan-twonode headless CI proof — two firmwares over a virtual CAN bus"
```

---

## Self-Review

**Spec coverage:** §4 A2 → Task 3; A3 → Tasks 1–2; A4 → Tasks 4–5; A5 → Task 6 + Task 3 test. §5 pass/fail (memory symbol + UART) → Tasks 4 & 6. §6 self-echo → Global Constraints + Task 6 firmware. §3 base-branch dependency → Global Constraints. §8 determinism test → Task 3 crossing test (deterministic step loop). Task 0 spike (roast #3) front-loaded. Free-tier pin (roast #4) → Global Constraints. Timebase/timeout-pacing tests are **Sub-project B** (firmware-level) and are intentionally deferred — noted here so B's spec carries them.

**Placeholder scan:** no TBD/TODO; every code step shows code; "mirror file:line" references point to read, existing code (not invented).

**Type consistency:** `attach_bus(tx, rx)` (Task 1) ← `attach_can_bus_by_id` (Task 2) ← `attach_can_bus` (Task 2) ← `can_bus` arm (Task 3); `MemoryValueDetails.{node,symbol,address:Option}` (Task 4) consumed by the `Env` runner (Task 5) and `test.yaml` (Task 6) consistently. `address` becoming `Option<u64>` is flagged for call-site fixes in Task 4 Step 3.

**Known follow-up for B:** ARM toolchain + pinned labwired CLI binary in udslib CI; the `e2e-labwired` job as a separate (non-merge-gate) job to respect the ≤2-min rule; firmware timebase ← labwired clock + one timeout + one FC-pacing test.
