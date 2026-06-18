# IO-Link Multi-Chip Station Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Simulate a master chip running real `iolinki-master` firmware wired point-to-point to N sensor chips each running real `iolinki` device firmware, orchestrated by `World`.

**Architecture:** Finish the in-tree multi-node architecture: a real `UartCrossLink` interconnect built on the proven `UartStreamDevice` seam, plus `World::from_manifest()` to build N `Machine`s from an `EnvironmentManifest`. Each device is its own simulated MCU (own address space), so the device stack's singleton globals are a non-issue. Prove 1 master ↔ 1 sensor first (Phase 1), then scale the same master FW to 4 ports (Phase 2).

**Tech Stack:** Rust `labwired-core` (`world.rs`, `network/mod.rs`, `peripherals/uart.rs`, `system/builder.rs`), C firmware built with `arm-none-eabi-gcc` (cortex-m4), `iolinki-master` + `iolinki` device stacks, `EnvironmentManifest` YAML, Cargo integration tests.

## Global Constraints

- Worktree: `/home/andrii/projects/labwired-core-multichip`, branch `feat/iolink-multichip-station`, based on `origin/main`. Do not push to `main`.
- Each device is one chip; IO-Link is point-to-point (one device per master port). Never host two device stacks in one address space.
- Reuse the proven `UartStreamDevice` (`poll(elapsed_us) -> Option<u8>`, `on_tx_byte(u8)`) seam; do not invent a parallel UART path.
- Firmware needs `arm-none-eabi-gcc` (cortex-m4, `-mfloat-abi=soft`, `--specs=nano.specs --specs=nosys.specs`). ELFs are build artifacts; check the prebuilt ELF in if CI lacks the toolchain.
- Device-stack `frame.c` lives in commit `aec4803` (not on the submodule's pinned `develop`); firmware/build must point `IOLINKI_DIR` at a checkout that has it.
- Commit style: w1ne noreply identity; no AI/Claude references in commit messages.
- Do not regress existing `CanBus`/`WirelessBus` `World` tests.

---

## Task 1: Real `UartCrossLink` interconnect on `UartStreamDevice` endpoints

**Files:**
- Modify: `crates/core/src/network/mod.rs` (replace the stub `UartCrossLink`)
- Test: `crates/core/src/network/mod.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: `Interconnect` trait (`fn tick(&mut self) -> SimResult<()>`), `UartStreamDevice` (`poll`, `on_tx_byte`).
- Produces:
  - `pub struct UartWireEndpoint` implementing `UartStreamDevice`, constructed in pairs via `UartCrossLink::new() -> (UartCrossLink, UartWireEndpoint, UartWireEndpoint)`.
  - `UartCrossLink` implementing `Interconnect`; `tick()` moves bytes A→B and B→A.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn uart_cross_link_moves_bytes_both_directions() {
    use crate::peripherals::uart::UartStreamDevice;
    let (mut link, mut a, mut b) = UartCrossLink::new("nodeA".into(), "nodeB".into());

    // Firmware on A transmits 0x55; B should receive it after a tick.
    a.on_tx_byte(0x55);
    link.tick().unwrap();
    assert_eq!(b.poll(1000), Some(0x55));
    assert_eq!(b.poll(1000), None);

    // And the reverse direction.
    b.on_tx_byte(0xAA);
    link.tick().unwrap();
    assert_eq!(a.poll(1000), Some(0xAA));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core --lib network::tests::uart_cross_link_moves_bytes_both_directions`
Expected: FAIL — `UartCrossLink::new` returns `()` / `UartWireEndpoint` not found.

- [ ] **Step 3: Replace the stub implementation**

Replace the stub `UartCrossLink` block in `crates/core/src/network/mod.rs` with:

```rust
use std::sync::mpsc::{channel, Receiver, Sender};

/// One end of the point-to-point UART wire, attached to a chip's UART via
/// `UartStreamDevice`. Bytes the firmware transmits land in `out` (drained by
/// the link); bytes the link delivers land in `inbox` (fed to the chip RX).
pub struct UartWireEndpoint {
    out: Sender<u8>,
    inbox: Receiver<u8>,
}

impl crate::peripherals::uart::UartStreamDevice for UartWireEndpoint {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        self.inbox.try_recv().ok()
    }
    fn on_tx_byte(&mut self, byte: u8) {
        let _ = self.out.send(byte);
    }
}

/// Point-to-point full-duplex UART link between two nodes' UARTs.
pub struct UartCrossLink {
    pub node_a: String,
    pub node_b: String,
    a_out: Receiver<u8>, // bytes A transmitted
    b_in: Sender<u8>,    // -> B inbox
    b_out: Receiver<u8>, // bytes B transmitted
    a_in: Sender<u8>,    // -> A inbox
}

impl UartCrossLink {
    pub fn new(node_a: String, node_b: String) -> (Self, UartWireEndpoint, UartWireEndpoint) {
        let (a_tx, a_out) = channel(); // A firmware TX
        let (a_in, a_inbox) = channel(); // -> A RX
        let (b_tx, b_out) = channel(); // B firmware TX
        let (b_in, b_inbox) = channel(); // -> B RX
        let endpoint_a = UartWireEndpoint { out: a_tx, inbox: a_inbox };
        let endpoint_b = UartWireEndpoint { out: b_tx, inbox: b_inbox };
        let link = Self { node_a, node_b, a_out, b_in, b_out, a_in };
        (link, endpoint_a, endpoint_b)
    }
}

impl Interconnect for UartCrossLink {
    fn tick(&mut self) -> SimResult<()> {
        while let Ok(byte) = self.a_out.try_recv() {
            let _ = self.b_in.send(byte);
        }
        while let Ok(byte) = self.b_out.try_recv() {
            let _ = self.a_in.send(byte);
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-core --lib network::tests::uart_cross_link_moves_bytes_both_directions`
Expected: PASS.

- [ ] **Step 5: Run existing world/network tests (no regression)**

Run: `cargo test -p labwired-core --lib world:: network::`
Expected: existing `test_can_bus_transmission`, `test_wireless_bus_transmission`, `test_multi_node_basic_sync` still PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/network/mod.rs
git commit -m "feat(net): real UartCrossLink interconnect on UartStreamDevice endpoints"
```

---

## Task 2: UART-by-id attach seam on `SystemBus` + `MachineTrait`

The wire endpoints must attach to a named UART inside an already-built machine. Today only the kit `AttachCtx` reaches a UART (`ctx.uart()`); there is no post-build lookup.

**Files:**
- Modify: `crates/core/src/bus.rs` (add `attach_uart_stream_by_id`)
- Modify: `crates/core/src/world.rs` (extend `MachineTrait`)
- Test: `crates/core/src/bus.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `SystemBus::attach_uart_stream_by_id(&mut self, uart_id: &str, dev: Box<dyn UartStreamDevice>) -> anyhow::Result<()>`
  - `MachineTrait::attach_uart_stream(&mut self, uart_id: &str, dev: Box<dyn UartStreamDevice>) -> SimResult<()>`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn attach_uart_stream_by_id_feeds_rx() {
    use crate::peripherals::uart::UartStreamDevice;
    struct One(Option<u8>);
    impl UartStreamDevice for One {
        fn poll(&mut self, _us: u32) -> Option<u8> { self.0.take() }
    }
    // Build a bus that has at least uart2 (use the same chip the L476 system uses).
    let chip = labwired_config::ChipDescriptor::from_file("configs/chips/stm32l476.yaml").unwrap();
    let manifest = labwired_config::SystemManifest { /* minimal: name+chip+empty vecs */
        ..minimal_manifest("configs/chips/stm32l476.yaml")
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.attach_uart_stream_by_id("uart2", Box::new(One(Some(0x42)))).unwrap();
    // Drive the bus tick so the stream device's byte reaches the UART RX register.
    // (Exact RX-read path mirrors existing uart.rs stream tests.)
}
```

> NOTE for implementer: model this test on the existing `attach_stream` test in `crates/core/src/peripherals/uart.rs` (search `attach_stream`). Use that test's exact RX-read assertion path; `minimal_manifest` is a 3-line local helper building a `SystemManifest` with empty `external_devices`/`board_io`/`peripherals`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core --lib bus::tests::attach_uart_stream_by_id_feeds_rx`
Expected: FAIL — no method `attach_uart_stream_by_id`.

- [ ] **Step 3: Implement the bus lookup**

In `crates/core/src/bus.rs`, add a method that finds the UART peripheral registered under `uart_id` and calls its existing `attach_stream`. Mirror how `SystemBus::from_config`/`AttachCtx` already resolves a peripheral by id (search `fn uart(` / the peripheral registry map in `bus.rs`). Implementation shape:

```rust
pub fn attach_uart_stream_by_id(
    &mut self,
    uart_id: &str,
    dev: Box<dyn crate::peripherals::uart::UartStreamDevice>,
) -> anyhow::Result<()> {
    let uart = self
        .uart_by_id_mut(uart_id) // existing or thin new helper over the peripheral map
        .ok_or_else(|| anyhow::anyhow!("no uart peripheral '{uart_id}'"))?;
    uart.attach_stream(dev);
    Ok(())
}
```

- [ ] **Step 4: Extend `MachineTrait`**

In `crates/core/src/world.rs`, add to `trait MachineTrait` and its `impl<C: Cpu> for Machine<C>`:

```rust
fn attach_uart_stream(
    &mut self,
    uart_id: &str,
    dev: Box<dyn crate::peripherals::uart::UartStreamDevice>,
) -> SimResult<()>;
```

```rust
fn attach_uart_stream(&mut self, uart_id: &str, dev: Box<dyn crate::peripherals::uart::UartStreamDevice>) -> SimResult<()> {
    self.bus.attach_uart_stream_by_id(uart_id, dev)
        .map_err(|e| /* map to SimError variant used elsewhere in world.rs */ )
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p labwired-core --lib bus::tests::attach_uart_stream_by_id_feeds_rx world::`
Expected: PASS, world tests still green.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/bus.rs crates/core/src/world.rs
git commit -m "feat(core): attach UART stream device by id on bus and MachineTrait"
```

---

## Task 3: `World::from_manifest()` for CortexM nodes + UART interconnects

**Files:**
- Modify: `crates/core/src/world.rs` (`from_manifest`)
- Test: `crates/core/tests/world_multichip.rs` (new integration test)

**Interfaces:**
- Consumes: `EnvironmentManifest { nodes: Vec<NodeConfig>, interconnects: Vec<InterconnectConfig> }`, `NodeConfig { id, system, firmware, config_overrides }`, `InterconnectConfig { type, nodes, config }`, `SystemBus::from_config`, `Machine::new`, `UartCrossLink::new`, `MachineTrait::attach_uart_stream`.
- Produces: a populated `World` whose `step_all()` advances every node and ticks every `uart_cross_link`.

- [ ] **Step 1: Write the failing test** (uses two trivial CortexM nodes + an empty firmware; asserts construction + a step runs)

```rust
#[test]
fn from_manifest_builds_two_cortexm_nodes_and_uart_link() {
    let env = labwired_core::__test_env_manifest_2node(); // helper builds an EnvironmentManifest in code
    let root = std::path::Path::new("examples/iolink-station");
    let mut world = labwired_core::world::World::from_manifest(env, root).unwrap();
    assert_eq!(world.machines.len(), 2);
    let results = world.step_all();
    assert!(results.values().all(|r| r.is_ok()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core --test world_multichip from_manifest_builds_two_cortexm_nodes_and_uart_link`
Expected: FAIL — `from_manifest` returns `bail!("not yet implemented")`.

- [ ] **Step 3: Implement `from_manifest`**

Replace the `bail!` body. For each `NodeConfig`: resolve `system` path under `root_dir`, parse `SystemManifest`, parse its chip, `SystemBus::from_config`, load the `firmware` ELF into the bus (mirror the ELF-load path in `crates/cli/src/main.rs:~1148`), build `Machine::new(CortexM::new(), bus)`, `add_machine(node.id, ...)`. For each `InterconnectConfig` with `type == "uart_cross_link"`: read `config.node_a_uart` / `config.node_b_uart` + the two node ids from `nodes`, call `UartCrossLink::new`, attach each endpoint to the matching machine via `attach_uart_stream`, `add_interconnect(link)`.

```rust
pub fn from_manifest(manifest: EnvironmentManifest, root_dir: &Path) -> anyhow::Result<Self> {
    let mut world = World::new(manifest.name.clone());
    for node in &manifest.nodes {
        let sys_path = root_dir.join(&node.system);
        let sysman = labwired_config::SystemManifest::from_file(&sys_path)?;
        let chip = labwired_config::ChipDescriptor::from_file(
            sys_path.parent().unwrap().join(&sysman.chip))?;
        let mut bus = SystemBus::from_config(&chip, &sysman)?;
        load_elf_into_bus(&mut bus, &root_dir.join(&node.firmware))?; // factor from cli
        let machine = Machine::new(crate::cpu::cortex_m::CortexM::new(), bus);
        world.add_machine(node.id.clone(), Box::new(machine));
    }
    for ic in &manifest.interconnects {
        if ic.r#type == "uart_cross_link" {
            let a = &ic.nodes[0]; let b = &ic.nodes[1];
            let a_uart = ic.config.get("node_a_uart").and_then(|v| v.as_str()).unwrap_or("uart1");
            let b_uart = ic.config.get("node_b_uart").and_then(|v| v.as_str()).unwrap_or("uart2");
            let (link, ea, eb) = crate::network::UartCrossLink::new(a.clone(), b.clone());
            world.machines.get_mut(a).unwrap().attach_uart_stream(a_uart, Box::new(ea))?;
            world.machines.get_mut(b).unwrap().attach_uart_stream(b_uart, Box::new(eb))?;
            world.add_interconnect(Box::new(link));
        }
    }
    Ok(world)
}
```

> NOTE: `load_elf_into_bus` should be factored from the existing CLI ELF-load path so both share one implementation (DRY). If that path is entangled in `main.rs`, extract a `pub fn` into `crates/loader` or `system/builder.rs` as part of this task.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-core --test world_multichip from_manifest_builds_two_cortexm_nodes_and_uart_link`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/world.rs crates/core/tests/world_multichip.rs
git commit -m "feat(world): build multi-node CortexM environments from manifest with UART links"
```

---

## Task 4: One-port master firmware crate

**Files:**
- Create: `examples/iolink-station/master-fw/Makefile`
- Create: `examples/iolink-station/master-fw/main.c`
- Create: `examples/iolink-station/master-fw/phy_labwired.c` + `.h`
- Create: `examples/iolink-station/master-fw/startup.c`, `l476.ld`, `debug_uart.c/.h` (copy from al2205 firmware; identical board)

**Interfaces:**
- Produces: `examples/iolink-station/master-fw/master.elf` — boots on STM32L476, drives `iolinki-master` over USART2, reaches OPERATE against a device peer, and signals state+PD to an observable location (debug UART byte and/or a known RAM address read via `MachineTrait::read_u8`).

- [ ] **Step 1: Copy the al2205 board scaffolding**

```bash
mkdir -p examples/iolink-station/master-fw
cp examples/al2205-iolink-dido/firmware/{startup.c,l476.ld,debug_uart.c,debug_uart.h} examples/iolink-station/master-fw/
```

- [ ] **Step 2: Write the master Makefile**

Mirror the al2205 Makefile but point at the master stack + device frame/crc:

```make
CC := arm-none-eabi-gcc
IOLINKI_MASTER_DIR ?= ../../../third_party/iolinki-master
IOLINKI_DIR ?= ../../../third_party/iolinki
CFLAGS  := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -ffreestanding -O0 -g3 -Wall -Wextra \
           -ffunction-sections -fdata-sections \
           -I$(IOLINKI_MASTER_DIR)/include -I$(IOLINKI_DIR)/include -I.
LDFLAGS := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostartfiles \
           --specs=nano.specs --specs=nosys.specs -T l476.ld -Wl,--gc-sections
MASTER_SRC := \
  $(IOLINKI_MASTER_DIR)/src/master_port.c \
  $(IOLINKI_MASTER_DIR)/src/master_controller.c \
  $(IOLINKI_MASTER_DIR)/src/master_isdu.c \
  $(IOLINKI_MASTER_DIR)/src/master_parameters.c \
  $(IOLINKI_MASTER_DIR)/src/master_sio.c \
  $(IOLINKI_DIR)/src/frame.c $(IOLINKI_DIR)/src/crc.c
APP_SRC := startup.c debug_uart.c phy_labwired.c main.c
ELF := master.elf
all: $(ELF)
$(ELF): $(APP_SRC) $(MASTER_SRC) l476.ld
	$(CC) $(CFLAGS) $(LDFLAGS) $(APP_SRC) $(MASTER_SRC) -o $@
clean:
	rm -f $(ELF)
.PHONY: all clean
```

> Requires `third_party/iolinki-master` vendored (as on `feat/iolink-real-stack`) or added as a submodule. If absent, add it first (rsync-vendor or submodule) — same source as that branch's Task 0.

- [ ] **Step 3: Write `phy_labwired.c/.h`** — implement the `iolink_phy_api_t` (`send`, `recv_byte`, optional `init`/`set_mode`/`set_baudrate`) over the L476 USART2 registers. Model on al2205's `phy_labwired.c` (same USART layout), but this is the master side: `send` writes bytes to USART DR; `recv_byte` reads from USART RX when RXNE is set. Provide the `iolink_master_config_t` callbacks (`set_mode_checked`, `set_baudrate_checked`, `flush_rx`, `prepare_tx`, `prepare_rx`, `wake_up`) as thin no-op/USART helpers (mirror the native bridge's bridge_* shims from `feat/iolink-real-stack`).

- [ ] **Step 4: Write `main.c`**

```c
#include "iolinki_master/master.h"
#include "phy_labwired.h"
#include "debug_uart.h"
#include <stdint.h>

static volatile uint8_t g_state;   /* observable at a known RAM address */
static volatile uint8_t g_pd0;

int main(void) {
    dbg_uart_init();
    iolink_master_port_t port;
    iolink_master_config_t cfg = phy_labwired_master_config(); /* type 2_1, COM3, port mode IOLINK, callbacks wired */
    iolink_phy_api_t phy = phy_labwired_master_phy();
    if (iolink_master_init(&port, &phy, &cfg) != 0) { for(;;){} }
    uint32_t now = 0;
    for (;;) {
        iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, now);
        now += 20; /* 2ms cycles in 100us units */
        g_state = (uint8_t) iolink_master_get_state(&port);
        uint8_t pd[1] = {0}, n = 0;
        if (iolink_master_get_pd_in(&port, pd, 1, &n) == 0 && n >= 1) g_pd0 = pd[0];
        dbg_uart_putc(g_state); /* state observable on debug UART */
    }
}
```

> Place `g_state`/`g_pd0` at a linker-known symbol (or emit via debug UART) so the Rust test can observe OPERATE (`3`) and PD without parsing UART framing. Document the address/symbol in the example README.

- [ ] **Step 5: Build the ELF**

Run: `make -C examples/iolink-station/master-fw`
Expected: produces `master.elf` with no errors (requires `arm-none-eabi-gcc`).

- [ ] **Step 6: Commit**

```bash
git add examples/iolink-station/master-fw
git commit -m "feat(example): one-port iolinki-master firmware for STM32L476"
```

---

## Task 5: Phase-1 proof — 2-node env + master↔sensor PD exchange test

**Files:**
- Create: `examples/iolink-station/env.yaml`
- Create: `examples/iolink-station/master/system.yaml`
- Create: `examples/iolink-station/sensor/system.yaml`
- Reuse: al2205 device firmware ELF for the sensor node (build it)
- Test: `crates/core/tests/world_multichip.rs` (add the proof test)

**Interfaces:**
- Consumes: `World::from_manifest`, `MachineTrait::read_u8`/`step`, the master `g_state`/`g_pd0` observable.

- [ ] **Step 1: Build the sensor (device) ELF**

```bash
make -C examples/al2205-iolink-dido/firmware  # produces al2205_dido.elf
```

- [ ] **Step 2: Write the two `system.yaml`s + `env.yaml`**

`env.yaml`:
```yaml
name: "iolink-station-2node"
nodes:
  - id: "master"
    system: "master/system.yaml"
    firmware: "master-fw/master.elf"
  - id: "sensor1"
    system: "sensor/system.yaml"
    firmware: "../al2205-iolink-dido/firmware/al2205_dido.elf"
interconnects:
  - type: "uart_cross_link"
    nodes: ["master", "sensor1"]
    config: { node_a_uart: "uart2", node_b_uart: "uart2" }
```

`master/system.yaml` and `sensor/system.yaml`: `name` + `chip: "../../../configs/chips/stm32l476.yaml"` + empty `external_devices`/`board_io` (the master's IO-Link peer is now the real sensor node over the wire, not a host model).

- [ ] **Step 3: Write the failing proof test**

```rust
#[test]
fn master_chip_reaches_operate_with_real_sensor_chip() {
    let root = std::path::Path::new("examples/iolink-station");
    let env = labwired_config::EnvironmentManifest::from_file(root.join("env.yaml")).unwrap();
    let mut world = labwired_core::world::World::from_manifest(env, root).unwrap();
    const STATE_ADDR: u64 = 0x2000_0000; // g_state symbol address from the master map
    let mut reached = false;
    for _ in 0..200_000 {
        world.step_all();
        if world.machines.get("master").unwrap().read_u8(STATE_ADDR).unwrap() == 3 { reached = true; break; }
    }
    assert!(reached, "master node never reached OPERATE talking to the real sensor chip");
}
```

- [ ] **Step 4: Run it (iterate on cycle budget / wiring until green)**

Run: `cargo test -p labwired-core --test world_multichip master_chip_reaches_operate_with_real_sensor_chip -- --nocapture`
Expected: PASS — the master chip reaches OPERATE (state 3) driving the real device-FW sensor chip over the wire.

> If it stalls: confirm the wire endpoints are on the right UARTs, the master wake-up byte handling matches the device's framing (the device firmware's PHY consumes the wake-up as on `feat/iolink-real-stack`), and `step_all` ordering ticks the link after both machines step.

- [ ] **Step 5: Commit**

```bash
git add examples/iolink-station crates/core/tests/world_multichip.rs
git commit -m "feat(example): 2-node IO-Link station — master chip drives real sensor chip"
```

---

## Task 6 (Phase 2): Scale master FW to 4 ports + 4-sensor station

**Files:**
- Modify: `examples/iolink-station/master-fw/main.c` + `phy_labwired.c` (4 USARTs via `iolink_master_controller_*`)
- Create: `examples/iolink-station/sensor{2,3,4}/system.yaml`
- Modify: `examples/iolink-station/env.yaml` (4 sensor nodes + 4 uart_cross_links)
- Test: `crates/core/tests/world_multichip.rs`

- [ ] **Step 1: Extend master FW to a 4-port controller** — use `iolink_master_controller_init(&ctrl, ports, 4, phys, cfgs)` with one `iolink_phy_api_t` per USART (USART1–USART3 + UART4); per-port PHY holds its USART base. Expose `g_state[4]`/`g_pd0[4]`. Rebuild ELF.

- [ ] **Step 2: Write 4-node env + per-sensor PD stimulus** — each sensor node reuses the device ELF; vary the published PD via the 74HC165 `inputs` preset in each sensor `system.yaml` (proximity/pressure/distance byte values).

- [ ] **Step 3: Write the 4-port proof test** — `World::from_manifest` on the 4-node env; step until `g_state[0..4]` all == 3 and each `g_pd0[i]` matches its sensor's preset.

- [ ] **Step 4: Run + commit**

```bash
cargo test -p labwired-core --test world_multichip four_port_station_all_sensors_operate -- --nocapture
git add examples/iolink-station crates/core/tests/world_multichip.rs
git commit -m "feat(example): 4-port IO-Link station with four real sensor chips"
```

---

## Task 7: Example README + docs

**Files:**
- Create: `examples/iolink-station/README.md`

- [ ] **Step 1: Write the README** — explain the topology (1 master chip + N sensor chips, real FW both sides, point-to-point UART cross-links), how to build the ELFs (`make -C master-fw`, `make -C ../al2205-iolink-dido/firmware`), how to run the tests, and that this is functional simulation evidence (not electrical/PHY conformance). Link the design spec.

- [ ] **Step 2: Commit**

```bash
git add examples/iolink-station/README.md
git commit -m "docs(example): IO-Link multi-chip station README"
```

---

## Verification Gates

- `cargo test -p labwired-core --lib network:: world::` — interconnect + world unit tests green (incl. existing CAN/wireless).
- `cargo test -p labwired-core --test world_multichip` — 2-node (and Phase 2 4-node) proofs green.
- `make -C examples/iolink-station/master-fw` and `make -C examples/al2205-iolink-dido/firmware` — ELFs build.
- `cargo check -p labwired-core` and `cargo check -p labwired-wasm` — non-multichip + browser builds unaffected.
- `git diff --check` — no whitespace errors.

## Execution Stop Points

- Stop after Task 5 if the master chip cannot reach OPERATE against the real sensor chip over the wire (the chip-to-chip mechanism is unproven — report rather than scaling).
- Stop before Task 4/5 if `arm-none-eabi-gcc` is unavailable (no ELFs can be built).
- Do not start Task 6 until the Task 5 2-node proof is green.
