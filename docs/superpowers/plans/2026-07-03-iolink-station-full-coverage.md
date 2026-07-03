# IO-Link Station Full Master-Stack Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the LabWired iolink-station model so the on-wire CI gate covers the full iolinki-master feature surface: ISDU read/write, PD output, events, data storage, CRC-corruption survival, and device-mute → ERROR → restart recovery.

**Architecture:** One new firmware pair (`master-fw-svc` + `device-fw-svc`) runs a phased "service script" after OPERATE, mirroring every result into `volatile` globals that Rust integration tests read by ELF symbol. Fault injection gets a small core addition: a byte-corruption counter on `UartCrossLink` reachable from tests via a new `as_any_mut` on the `Interconnect` trait; device mute is pure test-side (step machines selectively).

**Tech Stack:** C (arm-none-eabi-gcc, STM32L476 bare metal, CMSIS from STM32CubeL4), Rust (labwired-core integration tests), iolinki device stack + iolinki-master stack from `third_party/`.

## Global Constraints

- Repo: `/home/andrii/projects/labwired-core-model-coverage` (worktree, branch `feat/iolink-station-full-coverage`).
- Firmware builds need `export STM32CUBE_L4_DIR=$HOME/projects/STM32CubeL4`.
- Tests run with `export LABWIRED_REQUIRE_IOLINK_ELFS=1` so missing ELFs hard-fail.
- Never edit `third_party/iolinki` (pinned submodule) — if a device-stack gap blocks a phase, STOP and report; the fix belongs in the iolinki repo.
- `third_party/iolinki-master` may be patched ONLY for confirmed on-wire bugs (it is overlaid by the current iolinki-master checkout in that repo's CI, so any patch must also be reported for an iolinki-master PR). Prefer adapting firmware/test.
- All new C firmware mirrors observable state into `volatile` globals — the Rust test's only window.
- Do not use designated initializers that skip fields on the device config: `memset` first (GCC short-enums garbage gotcha, see `examples/iolink-dido/firmware/main.c:64`).
- clippy must stay clean: `cargo clippy --release -p labwired-core`.

## Verified API facts (from source, do not re-derive)

**Master (third_party/iolinki-master/include/iolinki_master/master.h):**
- States: INACTIVE=0, STARTUP=1, PREOPERATE=2, OPERATE=3, ERROR=4.
- Poll idiom: every ISDU-family call returns 0=OK, 1=PENDING, negative=error; call repeatedly with identical args between ticks.
- `int iolink_master_read_isdu(port, uint16_t index, uint8_t subindex, uint8_t* data, uint8_t* len);` len is in/out.
- `int iolink_master_write_isdu(port, uint16_t index, uint8_t subindex, const uint8_t* data, uint8_t len);`
- `int iolink_master_set_pd_out(port, const uint8_t* data, uint8_t len);` — len MUST equal cfg.pd_out_len.
- `int iolink_master_get_pd_in(port, uint8_t* buffer, uint8_t buffer_len, uint8_t* out_len);`
- `int iolink_master_get_diagnostics(const port, iolink_master_diagnostics_t* d);` fields used: `event_pending` (bool), `checksum_errors` (u32), `response_timeouts` (u32), `last_event_code` (u16).
- `int iolink_master_read_event_details(port, iolink_master_event_t* events, uint8_t max, uint8_t* out_count);` — reads ISDU index 0x001C, expects len%3==0, 3-byte records {qualifier, code_hi, code_lo} → `iolink_master_event_t{qualifier, type, code}`.
- `int iolink_master_read_data_storage(port, uint8_t* data, uint8_t* len);` / `int iolink_master_write_data_storage(port, const uint8_t* data, uint8_t len);` — ISDU index 0x0003 sub 0.
- Retry limit hardcoded 2: 3rd consecutive timeout/checksum error → ERROR + ERR_RETRY_LIMIT. Recovery from ERROR only via restart/re-init. VERIFY the exact restart API in master.h (search "restart"; if absent, re-run `iolink_master_init`).
- Timeouts in the model firmware: paced mode `tick_at(port, now)`? NO — existing firmwares call `iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, now)` (1-port) / `iolink_master_controller_tick_at(&ctrl, now)` (4-port), now += 20 per loop (100µs units). Keep the 1-port pattern. Check master.h:352 signature before use.

**Device (third_party/iolinki/include/iolinki/):**
- `iolink_device_config_t` fields: `phy`, `stack{m_seq_type,min_cycle_time,pd_in_len,pd_out_len,t_pd_us}`, `app_callbacks`, `device_info`, `ds_storage`.
- `iolink_device_info_t` (device_info.h:25-52): `vendor_name` (ISDU 0x0010), `product_name` (0x0012), `vendor_id` (0x000A), `device_id` (0x000B), plus others. Serving is built-in once `cfg.device_info` is non-NULL.
- `int iolink_device_pd_output_read(ctx, uint8_t* data, size_t len);` (device.h:59) — poll in main loop.
- `int iolink_device_pd_input_update(ctx, const uint8_t* data, size_t len, bool valid);`
- `iolink_events_ctx_t* iolink_device_get_events_ctx(ctx);` + `void iolink_event_trigger(events_ctx, uint16_t code, iolink_event_type_t type);` types: IOLINK_EVENT_TYPE_{NOTIFICATION,WARNING,ERROR} (events.h).
- `iolink_device_set_timing_enforcement(&device, false);` required in the model (frozen clock).
- **RISK (must verify in Task 3 before wiring events):** master reads event details at index 0x001C but device_info.function_id is also documented at 0x001C. Grep `third_party/iolinki/src/isdu.c` + `include/iolinki/protocol.h` for how index 0x001C is served and where the event queue is exposed via ISDU. If the device serves events elsewhere (or not at all), record the mismatch in the task report — assert only `event_pending` + `last_event_code`/`read_event_code` if that path works, and file the mismatch as a cross-stack bug.

**Rust test infra:**
- Symbols: `labwired_loader::resolve_symbol_in_elf(&std::fs::read(&elf)?, "g_name") -> Option<u32>`.
- Memory: `world.machines.get("master").unwrap().read_u8(addr as u64)` / `get_mut(...).write_u8(...)`.
- World: `World::from_manifest(env_manifest, &root)`; `world.step_all()`; `world.machines` and `world.interconnects` are pub. Selective stepping = call `.step()` on individual machines instead of `step_all()`, then tick interconnects — NOTE: `Interconnect::tick()` is how bytes move; when hand-stepping you MUST also iterate `world.interconnects` and call `.tick()` each loop, or no bytes flow.
- Skip gate: copy `require_iolink_elfs()`/`skip_or_fail_missing_elfs()` pattern from `crates/core/tests/world_multichip.rs:31-47`.
- Existing test loop bounds: OPERATE reached well within 5_000_000 step_all iterations (2-node).

---

### Task 1: UartCrossLink fault injection + Interconnect downcast

**Files:**
- Modify: `crates/core/src/network/mod.rs` (UartCrossLink struct, tick, Interconnect trait)
- Test: unit tests appended to `crates/core/src/network/mod.rs` (existing `#[cfg(test)] mod tests` if present, else create)

**Interfaces:**
- Produces: `Interconnect::as_any_mut(&mut self) -> Option<&mut dyn std::any::Any>` (default None); `UartCrossLink::set_corrupt_a_to_b(n: u32)`, `UartCrossLink::set_corrupt_b_to_a(n: u32)` — next n forwarded bytes in that direction are XORed with 0xFF.

- [ ] **Step 1: Write failing unit test** in `crates/core/src/network/mod.rs` tests module:

```rust
#[test]
fn crosslink_corrupts_next_n_bytes_then_forwards_clean() {
    let (mut link, ep_a, ep_b) = UartCrossLink::new("a".into(), "b".into());
    let mut ep_a = ep_a;
    let mut ep_b = ep_b;
    link.set_corrupt_a_to_b(1);
    ep_a.on_tx_byte(0x55);
    ep_a.on_tx_byte(0x66);
    link.tick().unwrap();
    assert_eq!(ep_b.poll(0), Some(0xAA)); // 0x55 ^ 0xFF
    assert_eq!(ep_b.poll(0), Some(0x66)); // clean again
}

#[test]
fn interconnect_downcasts_to_crosslink() {
    let (link, _a, _b) = UartCrossLink::new("a".into(), "b".into());
    let mut boxed: Box<dyn Interconnect> = Box::new(link);
    let any = boxed.as_any_mut().expect("crosslink exposes as_any_mut");
    assert!(any.downcast_mut::<UartCrossLink>().is_some());
}
```
(Adjust `poll` signature/`UartWireEndpoint` field access to the real API at network/mod.rs:90-102 — `poll(&mut self, elapsed_us: u32) -> Option<u8>`.)

- [ ] **Step 2:** `cargo test -p labwired-core --lib network` → both FAIL to compile (methods missing).
- [ ] **Step 3: Implement.** In `Interconnect` trait add default method `fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> { None }`. In `UartCrossLink` add fields `corrupt_a_to_b: u32, corrupt_b_to_a: u32` (init 0 in `new`), setters, and in `tick()` replace the two forward loops with:

```rust
while let Ok(byte) = self.a_out.try_recv() {
    let byte = if self.corrupt_a_to_b > 0 { self.corrupt_a_to_b -= 1; byte ^ 0xFF } else { byte };
    let _ = self.b_in.send(byte);
}
while let Ok(byte) = self.b_out.try_recv() {
    let byte = if self.corrupt_b_to_a > 0 { self.corrupt_b_to_a -= 1; byte ^ 0xFF } else { byte };
    let _ = self.a_in.send(byte);
}
```
And `impl` block: `fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> { Some(self) }` inside `impl Interconnect for UartCrossLink`.

- [ ] **Step 4:** `cargo test -p labwired-core --lib network` → PASS; `cargo clippy --release -p labwired-core` clean.
- [ ] **Step 5:** Commit: `feat(network): fault-injection counters + Interconnect downcast for UartCrossLink`

### Task 2: device-fw-svc firmware (service-rich device)

**Files:**
- Create: `examples/iolink-station/device-fw-svc/{main.c,Makefile}`; copy `phy_labwired.c`, `phy_labwired.h`, `debug_uart.c`, `debug_uart.h`, `.gitignore` verbatim from `examples/iolink-dido/firmware/`.
- Modify: `examples/iolink-station/ci/build.sh` (add make + test -f lines)
- Create: `examples/iolink-station/device-svc/system.yaml` (chip-only, no external devices) and `examples/iolink-station/env-svc.yaml`

**Interfaces:**
- Produces: `device.elf` exporting `volatile uint8_t g_device_state`. Device identity: vendor_name "LABWIRED", product_name "SVCDEV", vendor_id 0x1234, device_id 0x00056789. PD: 1 in / 1 out, mirror loop (pd_in := last pd_out). Event: when received pd_out byte == 0xE7 and not yet fired, `iolink_event_trigger(ev_ctx, 0x8CA0, IOLINK_EVENT_TYPE_WARNING)` once.

- [ ] **Step 1: main.c** — start from `examples/iolink-dido/firmware/main.c`, remove all SPI code, then:

```c
static const iolink_device_info_t DEVICE_INFO = {
    .vendor_name = "LABWIRED",
    .product_name = "SVCDEV",
    .vendor_id = 0x1234u,
    .device_id = 0x00056789u,
};
volatile uint8_t g_device_state = 0xFFu;
volatile uint8_t g_event_fired = 0u;

/* in main(), after memset of cfg: */
cfg.stack.m_seq_type = IOLINK_M_SEQ_TYPE_1_1;
cfg.stack.min_cycle_time = 0;
cfg.stack.pd_in_len = 1;
cfg.stack.pd_out_len = 1;   /* <-- now consumes master PD-out */
cfg.stack.t_pd_us = 0;
cfg.device_info = &DEVICE_INFO;
/* keep iolink_device_set_timing_enforcement(&device, false); */

/* main loop replaces the SPI read: */
uint8_t out = 0u;
if (iolink_device_pd_output_read(&device, &out, 1) == 0) {
    (void)iolink_device_pd_input_update(&device, &out, 1, true); /* mirror */
    if (out == 0xE7u && !g_event_fired) {
        iolink_event_trigger(iolink_device_get_events_ctx(&device), 0x8CA0u,
                             IOLINK_EVENT_TYPE_WARNING);
        g_event_fired = 1u;
        dbg_puts("EVENT FIRED\r\n");
    }
} else {
    uint8_t idle = 0x00u;
    (void)iolink_device_pd_input_update(&device, &idle, 1, true);
}
iolink_device_process(&device);
```
Keep the state-change debug print block and `g_device_state` update. Include `iolinki/events.h`, `iolinki/device_info.h`. Verify exact return convention of `iolink_device_pd_output_read` (0 == success? check device.h comment / src/device.c) — adjust the `== 0` check to reality.

- [ ] **Step 2: Makefile** — copy `examples/iolink-dido/firmware/Makefile`, change `ELF := device.elf`, fix `IOLINKI_DIR ?= ../../../third_party/iolinki` (one level deeper than iolink-dido? NO — device-fw-svc sits at examples/iolink-station/device-fw-svc, so `../../../third_party/iolinki` is correct, same depth as master-fw's Makefile paths; verify against `master-fw/Makefile`).
- [ ] **Step 3: manifests.** `device-svc/system.yaml`: copy `sensor/system.yaml`, delete the `external_devices` entry (empty list) — chip path stays `../../../configs/chips/stm32l476.yaml`. `env-svc.yaml`:

```yaml
name: iolink-station-svc
nodes:
  - id: master
    system: master/system.yaml
    firmware: master-fw-svc/master.elf
  - id: device1
    system: device-svc/system.yaml
    firmware: device-fw-svc/device.elf
interconnects:
  - type: uart_cross_link
    nodes: [master, device1]
    config: { node_a_uart: uart2, node_b_uart: uart2 }
```
(Match exact YAML shape of `env.yaml` — copy it and edit.)
- [ ] **Step 4: build.sh** — add `make -C "$ROOT/examples/iolink-station/device-fw-svc" STM32CUBE_L4_DIR="$STM32CUBE_L4_DIR"` and `test -f "$ROOT/examples/iolink-station/device-fw-svc/device.elf"` (master-fw-svc lines land in Task 3; do NOT add them here or build.sh breaks until then — add only the device lines).
- [ ] **Step 5:** `export STM32CUBE_L4_DIR=$HOME/projects/STM32CubeL4 && make -C examples/iolink-station/device-fw-svc` → device.elf exists; `arm-none-eabi-nm device.elf | grep -E "g_device_state|g_event_fired"` shows both.
- [ ] **Step 6:** Commit: `feat(iolink-station): service-rich device firmware (device_info + PD mirror + event)`

### Task 3: master-fw-svc firmware (phased service script)

**Files:**
- Create: `examples/iolink-station/master-fw-svc/{main.c,Makefile}`; copy `phy_labwired.c`, `phy_labwired.h`, `debug_uart.c`, `debug_uart.h`, `.gitignore` from `examples/iolink-station/master-fw/`.
- Modify: `examples/iolink-station/master-fw-svc/phy_labwired.c` — config gains `pd_out_len = 1` (everything else unchanged).
- Modify: `examples/iolink-station/ci/build.sh` (master-fw-svc make + test -f lines)

**Interfaces:**
- Produces: `master.elf` exporting volatile globals (all uint8_t unless noted): `g_master_state` (raw state), `g_phase`, `g_isdu_ok`, `g_isdu_vendor[8]`, `g_isdu_vendor_len`, `g_pd_echo_ok`, `g_event_ok`, `g_event_code_hi`, `g_event_code_lo`, `g_ds_ok`, `g_svc_done`, `g_error_seen`, `g_restart_count`, `g_diag_ck_errors` (low byte), `g_diag_timeouts` (low byte), `g_diag_event_pending`.
- Consumes: device from Task 2 (mirror + 0xE7 event trigger + device_info strings).

- [ ] **Step 1: verify open API questions** (read, don't guess): (a) restart API in `third_party/iolinki-master/include/iolinki_master/master.h` (grep restart); (b) whether `M_SEQ_TYPE_1_1` supports pd_out_len=1 on both stacks — grep `pd_out` in `third_party/iolinki-master/src/master_port.c` encode path and `third_party/iolinki/src/dll.c`; if 1_1 cannot carry PD-out, switch BOTH configs to the matching TYPE_2_x and note it; (c) the 0x001C event-details question from the header block — grep `0x001C`/`0x1C` in `third_party/iolinki/src/isdu.c` and `include/iolinki/protocol.h`. Record findings as comments at the top of main.c.
- [ ] **Step 2: main.c** — start from `master-fw/main.c`. Keep init + tick pattern (`iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, now); now += 20u;`). After the state/pd update, add the phased script (runs each loop iteration):

```c
switch (g_phase) {
case 0: /* wait OPERATE */
    if (g_master_state == 3u) { g_phase = 1u; dbg_puts("SVC PHASE 1 ISDU\r\n"); }
    break;
case 1: { /* ISDU read vendor name 0x0010 */
    uint8_t len = sizeof vendor_buf;
    int r = iolink_master_read_isdu(&port, 0x0010u, 0u, vendor_buf, &len);
    if (r == 0) {
        uint8_t n = len < 8u ? len : 8u;
        for (uint8_t i = 0; i < n; i++) g_isdu_vendor[i] = vendor_buf[i];
        g_isdu_vendor_len = n;
        g_isdu_ok = 1u; g_phase = 2u; dbg_puts("SVC PHASE 2 PDOUT\r\n");
    } else if (r < 0) { g_isdu_ok = 0xEEu; g_phase = 2u; }
    break; }
case 2: { /* PD echo: send 0x42, wait for mirror */
    uint8_t v = 0x42u;
    (void)iolink_master_set_pd_out(&port, &v, 1u);
    if (g_master_pd0 == 0x42u) { g_pd_echo_ok = 1u; g_phase = 3u; dbg_puts("SVC PHASE 3 EVENT\r\n"); }
    break; }
case 3: { /* trigger + observe event */
    uint8_t v = 0xE7u;
    (void)iolink_master_set_pd_out(&port, &v, 1u);
    iolink_master_diagnostics_t d;
    if (iolink_master_get_diagnostics(&port, &d) == 0 && d.event_pending) {
        g_diag_event_pending = 1u; g_phase = 4u; dbg_puts("SVC PHASE 4 EVREAD\r\n");
    }
    break; }
case 4: { /* read event details */
    iolink_master_event_t evs[4]; uint8_t cnt = 0u;
    int r = iolink_master_read_event_details(&port, evs, 4u, &cnt);
    if (r == 0 && cnt >= 1u) {
        g_event_code_hi = (uint8_t)(evs[0].code >> 8);
        g_event_code_lo = (uint8_t)(evs[0].code & 0xFFu);
        g_event_ok = 1u; g_phase = 5u; dbg_puts("SVC PHASE 5 DS\r\n");
    } else if (r < 0) { g_event_ok = 0xEEu; g_phase = 5u; }
    break; }
case 5: { /* data storage write + readback */
    static const uint8_t DS[4] = {0xD5, 0x01, 0xBE, 0xEF};
    int r = iolink_master_write_data_storage(&port, DS, 4u);
    if (r == 0) { g_phase = 6u; }
    else if (r < 0) { g_ds_ok = 0xEEu; g_phase = 7u; }
    break; }
case 6: { /* readback */
    uint8_t buf[16]; uint8_t len = sizeof buf;
    int r = iolink_master_read_data_storage(&port, buf, &len);
    if (r == 0) {
        g_ds_ok = (len >= 4u) ? 1u : 0xEEu; /* content check refined per DS record format found in Step 1 */
        g_phase = 7u;
    } else if (r < 0) { g_ds_ok = 0xEEu; g_phase = 7u; }
    break; }
case 7: g_svc_done = 1u; break;
default: break;
}
/* every loop: mirror diagnostics + ERROR restart policy */
{
    iolink_master_diagnostics_t d;
    if (iolink_master_get_diagnostics(&port, &d) == 0) {
        g_diag_ck_errors = (uint8_t)d.checksum_errors;
        g_diag_timeouts = (uint8_t)d.response_timeouts;
        if (d.event_pending) g_diag_event_pending = 1u;
    }
}
if (g_master_state == 4u) { /* ERROR: count once, then restart */
    g_error_seen = 1u;
    /* use the restart API found in Step 1 (or re-init) */
    <RESTART_CALL>;
    g_restart_count++;
    g_phase = 0u; /* re-run the script after recovery */
    dbg_puts("SVC RESTART\r\n");
}
```
Replace `<RESTART_CALL>` with the verified API. If ISDU error codes surface (0xEE paths), the phase advances so the world doesn't hang — the Rust test decides pass/fail per flag. Semantics: 1 = proven on wire, 0xEE = service returned an error (test fails with the flag value visible), 0 = never reached.
- [ ] **Step 3:** build.sh gains the master-fw-svc lines (same shape as Task 2 Step 4).
- [ ] **Step 4:** `make -C examples/iolink-station/master-fw-svc` → master.elf; `arm-none-eabi-nm` shows all g_ symbols above.
- [ ] **Step 5:** Commit: `feat(iolink-station): phased service-script master firmware`

### Task 4: happy-path services integration test

**Files:**
- Create: `crates/core/tests/world_station_services.rs`
- Modify: `examples/iolink-station/ci/test.sh` (add `cargo test -p labwired-core --release --test world_station_services -- --nocapture` line)

**Interfaces:**
- Consumes: env-svc.yaml world; symbols from Task 3.

- [ ] **Step 1: test skeleton** (copy the helpers from world_multichip.rs — station_root, skip gate, manifest load):

```rust
// crates/core/tests/world_station_services.rs
use labwired_core::world::World;
use std::path::PathBuf;

fn station_root() -> PathBuf { PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/iolink-station")) }

fn sym(elf_bytes: &[u8], name: &str) -> u64 {
    labwired_loader::resolve_symbol_in_elf(elf_bytes, name)
        .unwrap_or_else(|| panic!("symbol {name} not in ELF")) as u64
}

#[test]
fn master_services_isdu_pdout_event_ds_all_pass_on_wire() {
    let root = station_root();
    let master_elf = root.join("master-fw-svc/master.elf");
    let device_elf = root.join("device-fw-svc/device.elf");
    if !master_elf.exists() || !device_elf.exists() { /* skip gate copied from world_multichip.rs */ return; }
    let env = labwired_config::EnvironmentManifest::from_file(&root.join("env-svc.yaml")).unwrap();
    let mut world = World::from_manifest(env, &root).unwrap();
    let mb = std::fs::read(&master_elf).unwrap();
    let a_done = sym(&mb, "g_svc_done");
    let a_isdu = sym(&mb, "g_isdu_ok");
    let a_vlen = sym(&mb, "g_isdu_vendor_len");
    let a_vbuf = sym(&mb, "g_isdu_vendor");
    let a_pd = sym(&mb, "g_pd_echo_ok");
    let a_ev = sym(&mb, "g_event_ok");
    let a_ev_hi = sym(&mb, "g_event_code_hi");
    let a_ev_lo = sym(&mb, "g_event_code_lo");
    let a_ds = sym(&mb, "g_ds_ok");
    let a_phase = sym(&mb, "g_phase");
    let mut done = false;
    for _ in 0..60_000_000u64 {
        world.step_all();
        if world.machines.get("master").unwrap().read_u8(a_done).unwrap() == 1 { done = true; break; }
    }
    let m = world.machines.get("master").unwrap();
    let phase = m.read_u8(a_phase).unwrap();
    assert!(done, "service script never finished; stuck at phase {phase}");
    assert_eq!(m.read_u8(a_isdu).unwrap(), 1, "ISDU vendor-name read failed on wire");
    let vlen = m.read_u8(a_vlen).unwrap() as usize;
    let vendor: Vec<u8> = (0..vlen.min(8)).map(|i| m.read_u8(a_vbuf + i as u64).unwrap()).collect();
    assert_eq!(&vendor, b"LABWIRED", "vendor name mismatch: {vendor:02x?}");
    assert_eq!(m.read_u8(a_pd).unwrap(), 1, "PD-out echo failed");
    assert_eq!(m.read_u8(a_ev).unwrap(), 1, "event read failed");
    let code = ((m.read_u8(a_ev_hi).unwrap() as u16) << 8) | m.read_u8(a_ev_lo).unwrap() as u16;
    assert_eq!(code, 0x8CA0, "event code mismatch");
    assert_eq!(m.read_u8(a_ds).unwrap(), 1, "data-storage write/readback failed");
    eprintln!("services on wire: ISDU+PDOUT+EVENT+DS all green (phase {phase})");
}
```
(Adapt `EnvironmentManifest::from_file` + skip-gate to the exact forms in world_multichip.rs; the 60M bound trims down once real cycle counts are known — target <60s test runtime, tune with early exit.)
- [ ] **Step 2:** Build both firmwares, run `LABWIRED_REQUIRE_IOLINK_ELFS=1 cargo test -p labwired-core --release --test world_station_services -- --nocapture`. THIS IS THE INTEGRATION CRUNCH: expect iteration. Debug order when a phase sticks: read master debug prints (now `[master]`-prefixed), check device prints, re-check Step-1 findings of Task 3 (M-seq PD-out support, 0x001C serving, DS record format). Firmware fixes go in Tasks 2/3 files; core/stack bugs get reported per Global Constraints.
- [ ] **Step 3:** Add the test.sh line. Run full `examples/iolink-station/ci/test.sh` green.
- [ ] **Step 4:** Commit: `test(iolink-station): on-wire ISDU + PD-out + event + data-storage gate`

### Task 5: fault-injection tests (CRC corruption + device mute/recovery)

**Files:**
- Modify: `crates/core/tests/world_station_services.rs` (two more tests)

**Interfaces:**
- Consumes: Task 1 (`as_any_mut` + `set_corrupt_b_to_a`), Task 3 symbols (`g_diag_ck_errors`, `g_diag_timeouts`, `g_error_seen`, `g_restart_count`, `g_master_state`).

- [ ] **Step 1: CRC corruption test.** Build world, step_all until master state==3 (bound 10M). Then:

```rust
let link = world.interconnects[0].as_any_mut().unwrap()
    .downcast_mut::<labwired_core::network::UartCrossLink>().unwrap();
link.set_corrupt_b_to_a(2); // corrupt next 2 device->master bytes: one bad frame
for _ in 0..2_000_000u64 { world.step_all(); }
let m = world.machines.get("master").unwrap();
assert!(m.read_u8(a_ck).unwrap() >= 1, "checksum error not counted");
assert_eq!(m.read_u8(a_state).unwrap(), 3, "master did not survive one corrupt frame (retry-in-place)");
```
- [ ] **Step 2: mute/recovery test.** Reach OPERATE via step_all. Then hand-step: for N iterations step ONLY master + tick interconnects (device muted → timeouts); poll until `g_error_seen==1 && g_master_state==4` observed or `g_master_state==3→4` transition seen (bound 20M; remember the ERROR handler restarts immediately, so assert `g_error_seen==1 || g_restart_count>=1` rather than catching state 4 live). Then resume step_all for both and assert master returns to state 3 with `g_restart_count >= 1`. Also assert `g_diag_timeouts >= 3` was reached (mirror is low byte — read while muted).

```rust
// mute: step master only
for _ in 0..30_000_000u64 {
    world.machines.get_mut("master").unwrap().step().unwrap();
    for ic in world.interconnects.iter_mut() { ic.tick().unwrap(); }
    if world.machines.get("master").unwrap().read_u8(a_restarts).unwrap() >= 1 { break; }
}
assert_eq!(world.machines.get("master").unwrap().read_u8(a_err_seen).unwrap(), 1, "mute never drove master to ERROR");
// unmute: resume full stepping until OPERATE again
let mut recovered = false;
for _ in 0..30_000_000u64 {
    world.step_all();
    if world.machines.get("master").unwrap().read_u8(a_state).unwrap() == 3 { recovered = true; break; }
}
assert!(recovered, "master did not re-OPERATE after device unmute");
```
(Caveat discovered in infra map: the muted-master hand-step loop steps 1 machine step per interconnect tick — much slower wall-clock per sim-time than step_all; tune bounds by observed timeout pace. The master needs ~3 response timeouts at `response_timeout_100us=3`, `now += 20` per loop — timeouts come fast; expect <1M iterations.)
- [ ] **Step 3:** Run both new tests + full test.sh. All green, clippy clean.
- [ ] **Step 4:** Commit: `test(iolink-station): CRC-corruption survival + device-mute ERROR/restart recovery`

### Task 6: CI + docs closure

**Files:**
- Modify: `.github/workflows/core-iolink-native.yml` — add `crates/core/tests/world_station_services.rs` and `crates/core/src/network/mod.rs` to the path filter lists; add a `cargo test -p labwired-core --test world_station_services -- --nocapture` step next to the existing world_multichip one (skips cleanly without ELFs).
- Modify: `examples/iolink-station/README.md` — document the svc pair + what the gate now proves.

- [ ] **Step 1:** Make both edits (mirror the world_multichip entries exactly).
- [ ] **Step 2:** `bash examples/iolink-station/ci/build.sh && bash examples/iolink-station/ci/test.sh` end-to-end green from a clean `make -C ... clean` (or rm the ELFs first).
- [ ] **Step 3:** Commit: `ci(iolink-station): wire service-coverage tests into board CI + docs`

### Task 7 (separate repo, after labwired-core PR merges): iolinki-master status update

**Files:** in `/home/andrii/projects/iolinki-master` (branch off master): `docs/IMPLEMENTATION_STATUS.md` — update Evidence column for ISDU read/write, Cyclic PD (output), Events, Data Storage, RX path/retries rows to cite the LabWired on-wire tests; note any cross-stack bugs found. PR to master (protected: cmake-ctest + labwired-real-firmware-model must pass).

## Self-Review notes
- Spec coverage: item 1 (ISDU) → Tasks 2/3/4; item 2 (PD out) → Tasks 2/3/4; item 3 (faults) → Tasks 1/5; item 4 (events+DS) → Tasks 2/3/4. CI/docs → Task 6. Cross-repo reporting → Task 7 + Global Constraints.
- Known unknowns are explicitly assigned: restart API, M-seq PD-out capability, 0x001C event serving, DS record format, pd_output_read return convention — all in Task 3 Step 1 / Task 2 Step 1 with named files to check.
- Type consistency: symbol names in Task 4/5 match Task 3's export list; fault API names in Task 5 match Task 1.
