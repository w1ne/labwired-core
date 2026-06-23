# f103-uds-ecu Full Typical-Session Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the f103-uds-ecu example answer the full everyday UDS diagnostic session (0x10/0x3E/0x22/0x2E/0x14/0x19/0x31/0x2F/0x28, plus the existing 0x27/0x11), prove it with one runnable showcase scenario, and lock per-service tester framing as always-on Rust regression.

**Architecture:** Three layers. (1) Rust `CanUdsTester` FSM unit tests in `crates/core/src/bus/mod.rs` cover each new request/response shape in CI (no firmware ELF). (2) The f103 firmware (`examples/f103-uds-ecu/firmware/main.c`) registers udslib handlers for the in-scope services. (3) One combined 12-step scenario drives the whole session over the bus and asserts each reply; it needs the locally-built ELF and is not a clean-checkout CI gate (same as the existing reset smoke). No changes to the Rust tester FSM.

**Tech Stack:** Rust (labwired-core workspace), C against udslib (`~/projects/udslib`, ISO-14229), arm-none-eabi firmware, YAML scenarios.

## Global Constraints

- Git identity: `w1ne <14119286+w1ne@users.noreply.github.com>`; every commit `Signed-off-by` (DCO) via `git commit -s`.
- No "Claude", "AI", or assistant references in commits or code.
- Integrate with `git merge`, never rebase. Branch is `feat/f103-uds-full-coverage` off `origin/main`.
- Pre-push gate `core-integrity` = `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` (default-members, NOT `--workspace`) + tests. Run all three before declaring a Rust task done.
- Never stage `third_party/iolinki` (pre-existing submodule divergence, unrelated).
- udslib NRC convention in handlers: return a negative literal NRC with a trailing comment, matching `examples/h5_uds_ecu_full/firmware/main.c` style (e.g. `return -0x31; /* requestOutOfRange */`). `UDS_OK` is `0`.
- The firmware includes only public headers (`uds/uds_core.h`, `uds/uds_isotp.h`, `uds/uds_dtc.h`, `uds/uds_dtc_store.h`). Session-id `0x03` (extended) is internal-only; use a local `#define ECU_SESSION_EXTENDED 0x03u` with a comment, not an internal header.
- udslib config field names (verbatim from `include/uds/uds_config.h`): `did_table` (`{ const uds_did_entry_t *entries; uint16_t count; }`), `app_data` (`void *`), `fn_dtc_list`, `fn_dtc_clear`, `fn_routine_control`, `fn_io_control`, `fn_comm_control`. Keep `restrict_sessions` unset (false) so 0x11/0x27 stay reachable in the default session as today.
- Tester `expect` is a **prefix** match (response may be longer); `expect_nrc` matches `[0x7F, sid, nrc]` exactly; wildcard token in an expect string is `..`.

---

### Task 1: Rust FSM regression tests for the new service shapes

Locks the tester behavior the combined scenario depends on. Pure Rust, CI-gated, fully independent of firmware.

**Files:**
- Modify: `crates/core/src/bus/mod.rs` (test module — the `#[cfg(test)] mod tests` block containing `bus_with_script`, `inject_ecu_reply`, and the existing `uds_tester_*` tests around lines 3725–3850).

**Interfaces:**
- Consumes (existing test helpers in the same module):
  - `fn bus_with_script(steps: &[(&str, &str)]) -> SystemBus` — builds a bxCAN at `0x4000_6400` (name `bxcan1`) with a normal-mode bank-0 mask filter accepting `0x111` into FIFO0, attaches a `CanUdsTester` (`request_id 0x111`, `reply_id 0x222`) whose script is the `(send, expect)` tuples with `expect_nrc: None`, and runs the first service tick (sends step 0's request).
  - `fn inject_ecu_reply(bus: &mut SystemBus, id: u32, data: &[u8])` — pushes a classic CAN frame into `bxcan1`'s `tx_frames` so the next `bus.service_can_uds_testers()` drains it.
  - `CanUdsTester { script: Vec<UdsStep>, state: CanUdsTesterState, step_idx, .. }`, `UdsStep { send: Vec<u8>, expect: Vec<Option<u8>>, expect_nrc: Option<u8> }`, `CanUdsTesterState::{Done, Failed, AwaitResp, AwaitMultiResp, ..}`, `SystemBus::{yaml_bytes, parse_expect}`.
- Produces: a new helper `fn bus_with_steps(steps: Vec<UdsStep>) -> SystemBus` and five `#[test]` fns. No production code changes.

- [ ] **Step 1: Refactor `bus_with_script` to delegate to a `bus_with_steps(Vec<UdsStep>)` core**

In the test module, extract the bxCAN-setup + tester-attach + first-tick body of `bus_with_script` into a new `bus_with_steps`, and make `bus_with_script` build the `Vec<UdsStep>` from tuples and call it. The bxCAN register setup must be byte-identical to the current `bus_with_script` body (constants `MCR/BTR/FMR/FS1R/FM1R/FFA1R/FA1R/FBANK/VALID_BTR/BASE`, the filter writes for `0x111`, and the final `bus.service_can_uds_testers()`).

```rust
fn bus_with_steps(script: Vec<UdsStep>) -> SystemBus {
    use crate::peripherals::bxcan::BxCan;
    const MCR: u64 = 0x000;
    const BTR: u64 = 0x01C;
    const FMR: u64 = 0x200;
    const FM1R: u64 = 0x204;
    const FS1R: u64 = 0x20C;
    const FFA1R: u64 = 0x214;
    const FA1R: u64 = 0x21C;
    const FBANK: u64 = 0x240;
    const VALID_BTR: u32 = 0x00DC_0009;
    const BASE: u64 = 0x4000_6400;

    let mut bus = SystemBus::empty();
    bus.add_peripheral("bxcan1", BASE, 0x400, None, Box::new(BxCan::new()));

    bus.write_u32(BASE + MCR, 1).unwrap();
    bus.write_u32(BASE + BTR, VALID_BTR).unwrap();
    bus.write_u32(BASE + FMR, 1).unwrap();
    bus.write_u32(BASE + FS1R, 0x1).unwrap();
    bus.write_u32(BASE + FM1R, 0x0).unwrap();
    bus.write_u32(BASE + FFA1R, 0x0).unwrap();
    bus.write_u32(BASE + FBANK, (0x111u32) << 21).unwrap();
    bus.write_u32(BASE + FBANK + 4, (0x111u32) << 21).unwrap();
    bus.write_u32(BASE + FA1R, 0x1).unwrap();
    bus.write_u32(BASE + FMR, 0x0).unwrap();
    bus.write_u32(BASE + MCR, 0).unwrap();

    let mut tester = CanUdsTester::new("uds".into(), "bxcan1".into());
    tester.script = script;
    bus.can_uds_testers.push(tester);
    bus.service_can_uds_testers();
    bus
}

fn bus_with_script(steps: &[(&str, &str)]) -> SystemBus {
    let script: Vec<UdsStep> = steps
        .iter()
        .map(|(send_str, expect_str)| UdsStep {
            send: SystemBus::yaml_bytes(
                Some(&serde_yaml::Value::String(send_str.to_string())),
                &[],
            ),
            expect: SystemBus::parse_expect(expect_str),
            expect_nrc: None,
        })
        .collect();
    bus_with_steps(script)
}
```

- [ ] **Step 2: Run the existing tester tests to confirm the refactor is behavior-preserving**

Run: `cargo test -p labwired-core --lib uds_tester_`
Expected: PASS — all existing `uds_tester_*` tests still green (the refactor changed no behavior).

- [ ] **Step 3: Write the five new failing tests**

Append to the test module:

```rust
/// 0x2E WriteDataByIdentifier: single-frame multi-byte request (7 bytes) →
/// positive 6E echo. Covers DID-write framing the existing tests lack.
#[test]
fn uds_tester_did_write_sf_completes() {
    let mut bus = bus_with_script(&[("2E 01 23 DE AD BE EF", "6E 01 23")]);
    inject_ecu_reply(&mut bus, 0x222, &[0x04, 0x6E, 0x01, 0x23]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// 0x31 RoutineControl: reply carries an output byte after the echo; the
/// prefix match must accept the longer response.
#[test]
fn uds_tester_routine_reply_with_output_byte() {
    let mut bus = bus_with_script(&[("31 01 02 03", "71 01 02 03")]);
    inject_ecu_reply(&mut bus, 0x222, &[0x05, 0x71, 0x01, 0x02, 0x03, 0x00]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// 0x2F IOControl: shortTermAdjustment request, reply echoes DID + state.
#[test]
fn uds_tester_io_control_reply_completes() {
    let mut bus = bus_with_script(&[("2F A0 01 03 01", "6F A0 01")]);
    inject_ecu_reply(&mut bus, 0x222, &[0x05, 0x6F, 0xA0, 0x01, 0x03, 0x01]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// 0x19 ReadDTCInformation: a multi-frame ECU reply (FF + 1 CF) must be
/// reassembled (AwaitResp → AwaitMultiResp → Done) and prefix-matched.
#[test]
fn uds_tester_dtc_read_multiframe_reply_completes() {
    let mut bus = bus_with_script(&[("19 02 09", "59 02")]);
    // FF declares 10-byte response, carries first 6 bytes (59 02 09 01 23 45).
    inject_ecu_reply(&mut bus, 0x222, &[0x10, 0x0A, 0x59, 0x02, 0x09, 0x01, 0x23, 0x45]);
    bus.service_can_uds_testers(); // tester replies FlowControl, enters AwaitMultiResp
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitMultiResp);
    // CF carries the remaining bytes; total >= 10 → complete.
    inject_ecu_reply(&mut bus, 0x222, &[0x21, 0x67, 0xAA, 0xBB, 0xCC, 0xDD]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// Session-gated write rejected in the default session: the tester must
/// accept a negative response when the step declares `expect_nrc`.
#[test]
fn uds_tester_expect_nrc_negative_response_completes() {
    let steps = vec![UdsStep {
        send: SystemBus::yaml_bytes(
            Some(&serde_yaml::Value::String("2E 01 23 DE AD BE EF".to_string())),
            &[],
        ),
        expect: Vec::new(),
        expect_nrc: Some(0x31),
    }];
    let mut bus = bus_with_steps(steps);
    inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x7F, 0x2E, 0x31]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}
```

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cargo test -p labwired-core --lib uds_tester_`
Expected: PASS — the five new tests plus all pre-existing `uds_tester_*` tests.

- [ ] **Step 5: Run the full Rust gate**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test -p labwired-core --lib`
Expected: fmt clean, clippy clean (no `-D warnings` hits), tests PASS. (If fmt reports diffs, run `cargo fmt` and re-stage.)

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/bus/mod.rs
git commit -s -m "test(core): cover DID-write, routine, IO, multiframe-DTC, and NRC tester shapes"
```

---

### Task 2: f103 firmware handlers for the full diagnostic session

Register udslib handlers so the ECU answers 0x22/0x2E/0x14/0x19/0x31/0x2F/0x28 (0x10/0x3E are built-in). Deliverable: the firmware compiles cleanly to an ELF. Runtime proof is Task 3.

**Files:**
- Modify: `examples/f103-uds-ecu/firmware/main.c` (add includes, DID/DTC/IO storage, three handler fns, and the extended `uds_config_t` initializer in `main`).

**Interfaces:**
- Consumes (udslib public API, `include/uds/`):
  - `uds_did_entry_t { uint16_t id; uint16_t size; uint8_t session_mask; uint16_t security_mask; uds_did_read_fn read; uds_did_write_fn write; void *storage; }`; `uds_did_table_t { const uds_did_entry_t *entries; uint16_t count; }`.
  - `UDS_SESSION_EXTENDED` = `(1<<1)` (DID `session_mask` value for extended-only).
  - `uds_dtc_record_t`, `uds_dtc_store_t`; `void uds_dtc_store_init(uds_dtc_store_t*, uds_dtc_record_t* backing, uint16_t capacity, uint8_t aging_threshold)`; `int uds_dtc_store_register(uds_dtc_store_t*, uint32_t dtc, uint8_t severity, uint8_t functional_unit, uint8_t functional_group)`; `void uds_dtc_store_report_test(uds_dtc_store_t*, uint32_t dtc, bool failed)`; callbacks `uds_dtc_store_list_cb`, `uds_dtc_store_clear_cb`.
  - `UDS_DTC_SEVERITY_CHECK_IMMEDIATELY` (0x80), `UDS_DTC_FGID_EMISSIONS` (0x33) from `uds/uds_dtc.h`.
  - config fields `did_table`, `app_data`, `fn_dtc_list`, `fn_dtc_clear`, `fn_routine_control`, `fn_io_control`, `fn_comm_control`; `ctx->active_session` (raw session id, `0x03` = extended); `UDS_OK` = 0.
- Produces (for Task 3's scenario): an ECU that, given the request/response table in Task 3, returns 0x62/0x6E/0x59/0x54/0x71/0x6F/0x68 positives, gates DID `0x0123` write and routine `0x0203` to the extended session, and serves VIN DID `0xF190`.

- [ ] **Step 1: Add the two udslib DTC headers next to the existing UDS includes**

In `examples/f103-uds-ecu/firmware/main.c`, after the existing `#include "uds/uds_isotp.h"` (line ~24), add:

```c
#include "uds/uds_dtc.h"
#include "uds/uds_dtc_store.h"
```

- [ ] **Step 2: Add DID/DTC/IO storage and the extended-session literal**

Immediately before `static int security_seed(...)` (line ~243), add:

```c
/* --- Diagnostic data: DIDs, one seeded DTC, one IO point --- */
#define ECU_SESSION_EXTENDED 0x03u /* UDS_SESSION_ID_EXTENDED (internal id) */

/* VIN reported by ReadDataByIdentifier 0xF190 (read-only, any session). */
static const uint8_t g_vin[17] = {'L', 'A', 'B', 'W', 'I', 'R', 'E', 'D', '-',
                                  'F', '1', '0', '3', '-', 'U', 'D', 'S'};
/* Writable scratch DID 0x0123 (read+write, EXTENDED session only). */
static uint8_t g_scratch[4];
/* IO-controlled point 0xA001 ("test lamp") for InputOutputControl 0x2F. */
static uint8_t g_lamp[1];

static const uds_did_entry_t g_dids[] = {
    {0xF190u, 17u, 0u, 0u, NULL, NULL, (void *) g_vin},
    {0x0123u, 4u, UDS_SESSION_EXTENDED, 0u, NULL, NULL, g_scratch},
    {0xA001u, 1u, 0u, 0u, NULL, NULL, g_lamp},
};

/* Reference DTC store, seeded with one failing DTC (0x123456). */
static uds_dtc_record_t g_dtc_backing[4];
static uds_dtc_store_t g_dtc_store;
```

- [ ] **Step 3: Add the routine, IO, and comm-control handlers**

After `security_seed` (line ~255, before `int main(void)`), add:

```c
/* UDSLib fn_routine_control: routine 0x0203, startRoutine in EXTENDED only. */
static int ecu_routine(uds_ctx_t *ctx, uint8_t type, uint16_t id, const uint8_t *data,
                       uint16_t len, uint8_t *out, uint16_t max)
{
    (void) data;
    (void) len;
    (void) max;
    if (id != 0x0203u) {
        return -0x31; /* requestOutOfRange */
    }
    if (ctx->active_session != ECU_SESSION_EXTENDED) {
        return -0x31; /* requestOutOfRange: routine requires extended session */
    }
    if (type == 0x01u) { /* startRoutine */
        out[0] = 0x00u;  /* routine status: OK */
        return 1;
    }
    return -0x31; /* requestOutOfRange: unsupported routine control type */
}

/* UDSLib fn_io_control: IO point 0xA001 (test lamp) — store and echo state. */
static int ecu_io(uds_ctx_t *ctx, uint16_t id, uint8_t type, const uint8_t *data,
                  uint16_t len, uint8_t *out, uint16_t max)
{
    (void) ctx;
    (void) type;
    (void) max;
    if (id != 0xA001u) {
        return -0x31; /* requestOutOfRange */
    }
    if (len >= 1u) {
        g_lamp[0] = data[0];
    }
    out[0] = g_lamp[0];
    return 1;
}

/* UDSLib fn_comm_control: accept the requested communication mode. */
static int ecu_comm(uds_ctx_t *ctx, uint8_t ctrl_type, uint8_t comm_type, uint16_t node_id)
{
    (void) ctx;
    (void) ctrl_type;
    (void) comm_type;
    (void) node_id;
    return UDS_OK;
}
```

- [ ] **Step 4: Seed the DTC store and extend the `uds_config_t` initializer**

In `main`, right before the `uds_config_t cfg = { ... };` declaration (line ~269), seed the store:

```c
    uds_dtc_store_init(&g_dtc_store, g_dtc_backing, 4u, 40u);
    uds_dtc_store_register(&g_dtc_store, 0x123456u, UDS_DTC_SEVERITY_CHECK_IMMEDIATELY, 0x10u,
                           UDS_DTC_FGID_EMISSIONS);
    uds_dtc_store_report_test(&g_dtc_store, 0x123456u, true); /* set testFailed status */
```

Then add these fields to the `cfg` initializer (between `.fn_reset = ecu_reset,` and `.rx_buffer = g_rx_buf,`):

```c
        .did_table = {.entries = g_dids, .count = (uint16_t) (sizeof(g_dids) / sizeof(g_dids[0]))},
        .app_data = &g_dtc_store,
        .fn_dtc_list = uds_dtc_store_list_cb,
        .fn_dtc_clear = uds_dtc_store_clear_cb,
        .fn_routine_control = ecu_routine,
        .fn_io_control = ecu_io,
        .fn_comm_control = ecu_comm,
```

- [ ] **Step 5: Build the firmware ELF and verify it compiles clean**

Run: `make -C examples/f103-uds-ecu/firmware UDSLIB_DIR=$HOME/projects/udslib`
Expected: build succeeds, produces `examples/f103-uds-ecu/firmware/build/f103_uds_ecu.elf`, no compiler errors and no new warnings (the handlers and config compile against udslib).

If `make` reports an undefined reference to a `uds_dtc_store_*` or `uds_service_*` symbol, confirm `uds_dtc_store.c`/`uds_service_data.c`/`uds_service_io.c`/`uds_service_maintenance.c` are in `UDS_SRCS` (they are on `main`) and that `UDSLIB_DIR` points at the udslib checkout.

- [ ] **Step 6: Commit (the built ELF is git-ignored; commit only source)**

```bash
git add examples/f103-uds-ecu/firmware/main.c
git commit -s -m "feat(f103-uds): handlers for DID, DTC, routine, IO, and comm control"
```

---

### Task 3: Combined showcase scenario, smoke run, and docs

Drive the full session over the bus against the Task 2 ELF and assert each reply; document the scenario and the CI-vs-local split.

**Files:**
- Create: `examples/f103-uds-ecu/uds-session.yaml` (system: machine + scripted tester).
- Create: `examples/f103-uds-ecu/uds-session-smoke.yaml` (scenario: inputs + assertions).
- Modify: `examples/f103-uds-ecu/README.md` (document the new scenario + CI/local split).

**Interfaces:**
- Consumes: the firmware behavior from Task 2 (positive responses + extended-session gating) and the existing `system-reset.yaml` shape (`type: uds-tester`, `connection: bxcan1`, `config: { request_id: 0x111, reply_id: 0x222, script: [...] }`, `id: uds-tester`); the CLI runner `labwired test --script <scenario>` with assertions `uart_contains` and `uds_tester: { id, result: done }`.
- Produces: a runnable end-to-end demo of typical ECU operation.

- [ ] **Step 1: Write the combined system file with the full scripted session**

Create `examples/f103-uds-ecu/uds-session.yaml`:

```yaml
name: "f103-uds-ecu"
chip: "../../configs/chips/stm32f103.yaml"
external_devices:
  # A real second CAN node: a virtual UDS tester scripted to walk the full
  # everyday diagnostic session and assert each ECU reply on the bus. Sends are
  # UDS-payload bytes; the tester frames ISO-TP. `expect` is a prefix match;
  # `expect_nrc` matches a [7F sid nrc] negative response exactly.
  - type: "uds-tester"
    id: "uds-tester"
    connection: "bxcan1"
    config:
      request_id: 0x111
      reply_id: 0x222
      script:
        - send: "22 F1 90"            # ReadDataByIdentifier VIN (open session)
          expect: "62 F1 90"
        - send: "2E 01 23 DE AD BE EF" # WriteDataByIdentifier rejected in default
          expect_nrc: 0x31
        - send: "10 03"               # DiagnosticSessionControl -> extended
          expect: "50 03"
        - send: "3E 00"               # TesterPresent
          expect: "7E 00"
        - send: "2E 01 23 DE AD BE EF" # WriteDataByIdentifier accepted in extended
          expect: "6E 01 23"
        - send: "22 01 23"            # read back proves the write persisted
          expect: "62 01 23 DE AD BE EF"
        - send: "19 01 09"            # ReadDTCInformation reportNumberOfDTCByStatusMask
          expect: "59 01"
        - send: "14 FF FF FF"         # ClearDiagnosticInformation (all groups)
          expect: "54"
        - send: "31 01 02 03"         # RoutineControl startRoutine 0x0203 (extended)
          expect: "71 01 02 03"
        - send: "2F A0 01 03 01"      # InputOutputControl shortTermAdjustment
          expect: "6F A0 01"
        - send: "28 00 01"            # CommunicationControl enableRxAndTx / normal
          expect: "68 00"
        - send: "11 01"              # ECUReset softReset (51 01 before reboot)
          expect: "51 01"
board_io: []
```

- [ ] **Step 2: Write the scenario file with assertions**

Create `examples/f103-uds-ecu/uds-session-smoke.yaml`:

```yaml
schema_version: "1.0"
inputs:
  system: "./uds-session.yaml"
  firmware: "./firmware/build/f103_uds_ecu.elf"
limits:
  max_steps: 2000000
assertions:
  # The scripted tester walks the full diagnostic session (read/write DID,
  # session switch, tester present, DTC count/clear, routine, IO control, comm
  # control) and finishes with 0x11 ECUReset. ECU_READY reprints on the real
  # AIRCR-triggered reboot, so the banner appears twice.
  - uart_contains: "ECU_READY"
  - uds_tester: { id: "uds-tester", result: done }
```

- [ ] **Step 3: Run the smoke scenario and verify it passes**

Run: `cargo run -p labwired-cli --bin labwired -- test --script examples/f103-uds-ecu/uds-session-smoke.yaml`
Expected: exit code 0; output reports the `uds-tester` assertion as `done` and the `ECU_READY` uart assertion satisfied. (If the ELF is missing, build it first per Task 2 Step 5.)

- [ ] **Step 4: Negative control — prove the assertion is live**

Temporarily change step 1's `expect: "62 F1 90"` to `expect: "62 F1 91"` in `uds-session.yaml`, then run:

Run: `cargo run -p labwired-cli --bin labwired -- test --script examples/f103-uds-ecu/uds-session-smoke.yaml; echo "exit=$?"`
Expected: non-zero exit; the `uds-tester` assertion reports a failure mentioning step 0. Then revert the byte back to `62 F1 90` and re-run Step 3 to confirm it passes again.

- [ ] **Step 5: Document the scenario in the example README**

In `examples/f103-uds-ecu/README.md`, add a section describing the three scenarios (`uds-smoke.yaml` SecurityAccess, `uds-reset-smoke.yaml` ECUReset, `uds-session-smoke.yaml` full session) and state explicitly: these smokes need the locally-built ELF (`make -C firmware UDSLIB_DIR=$HOME/projects/udslib`) and are not clean-checkout CI gates; the always-on regression for tester framing lives in the `crates/core/src/bus/mod.rs` `uds_tester_*` tests. Note the full-session scenario gates DID `0x0123` write and routine `0x0203` to the extended session (entered via `10 03`).

Use this exact section text (append under the existing scenarios documentation; if no such section exists, add it after the intro):

```markdown
## Scenarios

- `uds-smoke.yaml` — multi-frame SecurityAccess (0x27) seed handshake.
- `uds-reset-smoke.yaml` — ECUReset (0x11): `51 01` lands before the real reboot.
- `uds-session-smoke.yaml` — the full everyday diagnostic session driven by the
  scripted tester: ReadDataByIdentifier (0x22), session switch (0x10) and
  TesterPresent (0x3E), WriteDataByIdentifier (0x2E, extended-session gated),
  ReadDTCInformation/ClearDTC (0x19/0x14), RoutineControl (0x31, extended-gated),
  InputOutputControl (0x2F), CommunicationControl (0x28), and ECUReset (0x11).
  The default-session write is asserted to return `7F 2E 31`, then succeeds after
  `10 03`.

These smokes drive a locally-built ELF and are **not** clean-checkout CI gates.
Build first:

    make -C firmware UDSLIB_DIR=$HOME/projects/udslib

Run:

    cargo run -p labwired-cli --bin labwired -- test --script examples/f103-uds-ecu/uds-session-smoke.yaml

The always-on regression for the scriptable tester's framing (single/multi-frame
requests and responses, wildcard and NRC matching) lives in the
`uds_tester_*` tests in `crates/core/src/bus/mod.rs`, which run in CI.
```

- [ ] **Step 6: Commit**

```bash
git add examples/f103-uds-ecu/uds-session.yaml examples/f103-uds-ecu/uds-session-smoke.yaml examples/f103-uds-ecu/README.md
git commit -s -m "feat(f103-uds): full diagnostic-session scenario and docs"
```

---

## Final verification (after all tasks)

- [ ] Rust gate green: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test -p labwired-core --lib`
- [ ] Smoke green against the built ELF: `cargo run -p labwired-cli --bin labwired -- test --script examples/f103-uds-ecu/uds-session-smoke.yaml` (exit 0, `result: done`, `ECU_READY` twice).
- [ ] `git status` shows no staged `third_party/iolinki`.
- [ ] All commits `Signed-off-by` and free of assistant references.
