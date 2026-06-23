# f103-uds-ecu Full Typical-Session Coverage — Design

**Date:** 2026-06-23
**Status:** Approved (design), pending spec review
**Branch:** `feat/f103-uds-full-coverage` (off `origin/main`)

## Problem

The emulator demonstrates only a third of typical ECU diagnostic operation
end-to-end. A coverage audit (UDS services 0x10–0x3E) found that the everyday
diagnostic services are *compiled into* the f103-uds-ecu firmware but have **no
registered handler and no runnable scenario** — so a real request would NRC.
Today only 0x27 (SecurityAccess), 0x11 (ECUReset), and 0x22 (one DID on h563)
actually run on the bus via the scriptable tester.

This work makes the f103 ECU answer the full typical diagnostic session, proves
it with one runnable showcase scenario, and locks per-service framing as
always-on Rust regression.

## Scope

In scope (the everyday diagnostic loop), all on `examples/f103-uds-ecu`:

| SID  | Service                       | Work                                   |
|------|-------------------------------|----------------------------------------|
| 0x10 | DiagnosticSessionControl      | built-in; proven by scenario           |
| 0x3E | TesterPresent                 | built-in; proven by scenario           |
| 0x22 | ReadDataByIdentifier          | DID table (VIN + scratch)              |
| 0x2E | WriteDataByIdentifier         | DID table (scratch), extended-gated    |
| 0x14 | ClearDTC                      | `fn_dtc_clear` via dtc_store           |
| 0x19 | ReadDTCInformation            | `fn_dtc_list` via dtc_store            |
| 0x31 | RoutineControl                | `fn_routine_control`, extended-gated   |
| 0x2F | InputOutputControlByIdentifier| `fn_io_control` (DID 0xA001)           |
| 0x28 | CommunicationControl          | `fn_comm_control`                      |

Out of scope: 0x23/0x3D memory access, 0x29 Authentication, 0x84 Secured,
0x86 ROE, 0x87 LinkControl, 0x34/0x36/0x37 transfer (already shown by the h563
OTA example). h563-uds-ecu is unchanged.

## Architecture

Three layers, no changes to the Rust tester FSM (its prefix-matching SF +
multi-frame send/receive already covers every shape these services need).

### 1. Firmware handlers (`examples/f103-uds-ecu/firmware/main.c`)

Extend the existing `uds_config_t cfg` (today only `fn_security_seed` +
`fn_reset`). Mirror the canonical udslib reference
`examples/h5_uds_ecu_full/firmware/main.c`. Exact udslib surface
(`include/uds/uds_config.h`):

- **DID table** — `cfg.did_table = { .entries = g_dids, .count = N }` with
  `uds_did_entry_t { id, size, session_mask, security_mask, read, write, storage }`:
  - `0xF190` VIN, size 17, `session_mask 0` (all), storage = const
    `"LABWIRED-F103-UDS"`. Read-only (no `write`).
  - `0x0123` scratch, size 4, `session_mask = UDS_SESSION_EXTENDED` (0x02),
    storage = `g_scratch[4]`. Read+write via storage pointer (no callbacks
    needed — udslib reads/writes storage directly when `read`/`write` are NULL).
  - `0xA001` IO point ("test lamp"), size 1, `session_mask 0`, storage =
    `g_lamp[1]`. Required so 0x2F accepts the ID.
- **DTC** — seed via `uds_dtc_store` (`include/uds/uds_dtc_store.h`):
  `uds_dtc_store_init(&g_store, g_dtc_backing, CAP, AGING)`,
  `uds_dtc_store_register(&g_store, 0x123456, severity, funit, fgroup)`,
  `uds_dtc_store_report_test(&g_store, 0x123456, true)` to set status bits.
  Wire `cfg.app_data = &g_store; cfg.fn_dtc_list = uds_dtc_store_list_cb;
  cfg.fn_dtc_clear = uds_dtc_store_clear_cb;`.
- **RoutineControl** — `cfg.fn_routine_control = ecu_routine`:
  `int ecu_routine(uds_ctx_t*, uint8_t type, uint16_t id, const uint8_t* data,
  uint16_t len, uint8_t* out, uint16_t max)`. Routine `0x0203`: if
  `ctx->active_session != extended` return `-UDS_NRC_REQUEST_OUT_OF_RANGE`
  (enforcement); on start (type 0x01) write `out[0]=0x00` (status), return 1;
  unknown id → `-0x31`.
- **IOControl** — `cfg.fn_io_control = ecu_io`:
  `int ecu_io(uds_ctx_t*, uint16_t id, uint8_t type, const uint8_t* data,
  uint16_t len, uint8_t* out, uint16_t max)`. id `0xA001`: store `data[0]` into
  `g_lamp[0]`, echo `out[0]=g_lamp[0]`, return 1; else `-0x31`.
- **CommunicationControl** — `cfg.fn_comm_control = ecu_comm`:
  `int ecu_comm(uds_ctx_t*, uint8_t ctrl_type, uint8_t comm_type,
  uint16_t node_id)`. Return `UDS_OK` (accept). (Built-in validates the
  control/comm type ranges before calling us.)

NRC names come from udslib (`UDS_NRC_REQUEST_OUT_OF_RANGE` = 0x31, etc.);
negative-return convention is `-<nrc>`. Read-path NRC for a session-gated DID
in the wrong session is `0x31` (confirmed in `uds_service_data.c`).

Makefile already compiles every needed service `.c` (data, maintenance, io,
dtc_store) — no build change.

### 2. Combined showcase scenario

Two files mirroring the existing reset smoke (`script` in SYSTEM yaml, assertion
in SCENARIO yaml). Reuse the locally-built f103 ELF; **not** a clean-checkout CI
gate (consistent with `uds-reset-smoke.yaml`).

- `examples/f103-uds-ecu/uds-session.yaml` (system) — same machine as
  `system-reset.yaml` plus a `uds-tester` whose `script` chains the full session.
  Tester `send`/`expect` are UDS-payload bytes; `expect` is a prefix match;
  negative steps use `expect_nrc`.

  | # | send                | expect / expect_nrc | proves                          |
  |---|---------------------|---------------------|---------------------------------|
  | 1 | `22 F1 90`          | `62 F1 90`          | DID read, open session          |
  | 2 | `2E 01 23 DE AD BE EF` | `expect_nrc: 0x31` | write rejected in default       |
  | 3 | `10 03`             | `50 03`             | enter extended session          |
  | 4 | `3E 00`             | `7E 00`             | tester present                  |
  | 5 | `2E 01 23 DE AD BE EF` | `6E 01 23`       | write accepted in extended      |
  | 6 | `22 01 23`          | `62 01 23 DE AD BE EF` | read-back proves the write   |
  | 7 | `19 01 09`          | `59 01`             | DTC count by status mask        |
  | 8 | `14 FF FF FF`       | `54`                | clear all DTC                   |
  | 9 | `31 01 02 03`       | `71 01 02 03`       | routine start (extended)        |
  |10 | `2F A0 01 03 01`    | `6F A0 01`          | IO control shortTermAdjustment  |
  |11 | `28 00 01`          | `68 00`             | communication control           |
  |12 | `11 01`             | `51 01`             | reset (51 01 before reboot)     |

- `examples/f103-uds-ecu/uds-session-smoke.yaml` (scenario) — assertions:
  `uds_tester: { id: <tester-id>, result: done }` and
  `uart_contains: ECU_READY` (reprints on the real reboot from step 12).

### 3. Always-on Rust regression (CI)

Per-service framing isolation lives as `CanUdsTester` FSM unit tests in
`crates/core/src/bus/mod.rs` (no ELF, runs in `core-integrity`). Each builds a
one/multi-step script, feeds canned ECU frames, asserts the FSM frames the
request and matches the reply correctly. Cover the shapes the combined scenario
exercises but the existing tests do not:

- multi-byte SF DID write (`2E 01 23 DE AD BE EF` → `6E 01 23`)
- DTC read reply (`19 01 09` → `59 01 ..`), incl. a multi-frame DTC reply path
- routine reply with output bytes (`31 01 02 03` → `71 01 02 03 00`)
- IO control reply (`2F A0 01 03 01` → `6F A0 01 03 01`)
- negative response via `expect_nrc` for the session-gated write
  (`2E ...` → `7F 2E 31`)

These test the **tester**, not firmware session gating (which only the local
smoke proves) — that division is intentional and noted in the smoke README.

## Testing

- Rust: `cargo test -p labwired-core` (FSM tests) + `cargo fmt --check` +
  `cargo clippy --all-targets -- -D warnings` (default-members) — the
  `core-integrity` gate.
- Smoke (local, after building the ELF with
  `make -C examples/f103-uds-ecu/firmware UDSLIB_DIR=$HOME/projects/udslib`):
  `cargo run -p labwired-cli --bin labwired -- test --script
  examples/f103-uds-ecu/uds-session-smoke.yaml` → exit 0, `result: done`,
  `ECU_READY` twice. Negative control: corrupt one `expect` → exit non-zero.
- README in the example documents the new scenario and the CI-vs-local split.

## Risks / decisions

- **Session enforcement** (approved): scratch DID 0x0123 and routine 0x0203 are
  extended-only; the `10 03` switch is load-bearing and the scenario shows a
  default-session write rejected with `7F 2E 31`.
- **Per-service coverage** (approved): one combined yaml showcase + Rust FSM
  tests, *not* ~14 per-service yaml pairs.
- DTC status/severity constants and dtc_store capacity are taken verbatim from
  `uds_dtc_store.h` / the `examples/dtc_store` reference; the plan pins exact
  values.
- No Rust tester FSM changes — if a service needs a frame shape the FSM cannot
  produce, that is a plan-stopping surprise to escalate, not silently patch.
