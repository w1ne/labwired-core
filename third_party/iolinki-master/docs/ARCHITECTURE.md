# iolinki-master Architecture

`iolinki-master` is a standalone, hardware-independent IO-Link **master** protocol
library. The design goal is a portable embedded master core with clean boundaries:
no heap, caller-owned storage, a public API that hides private state, and no
coupling to the device-stack singleton. This document describes the layering. It
does **not** restate the build order — that is [`ROADMAP.md`](ROADMAP.md) — and it
cross-references the PHY contract in [`PHY_BOUNDARY.md`](PHY_BOUNDARY.md) rather
than duplicating it.

## 1. Repository and shared-helper boundary

The master is a *sibling* of the device stack, not a fork of it. It compiles only
the narrow shared helper sources it needs from a local `iolinki` checkout — `crc.c`
and `frame.c` (CRC6/checksum and frame encode/decode) — into its own build via
`CMakeLists.txt`. It must never `add_subdirectory()` the full device stack or call
device singleton entry points (`iolink_init()`, `iolink_process()`). The device
checkout is located through `-DIOLINKI_DEVICE_DIR` and defaults to a sibling
`../iolinki`.

## 2. Public API vs private state

The central architectural move: **callers own opaque storage; private state lives
under `src/`.**

- Public users allocate `iolink_master_port_t` (per port) or
  `iolink_master_controller_t` (per multi-port controller). These are unions sized
  by audited budgets — `IOLINK_MASTER_PORT_STORAGE_SIZE` (1280 B) and
  `IOLINK_MASTER_CONTROLLER_STORAGE_SIZE` (32 B) — giving embedded integrators a
  fixed, auditable RAM ceiling and keeping the ABI heap-free and caller-owned.
- The real layout lives in `src/master_internal.h` and is reached only through
  `iolink_master_port_state()` inside `src/master_*.c`; the public header
  (`include/iolinki_master/master.h`) exposes none of it.
- Every public function has a documented return contract expressed as named result
  codes (`IOLINK_MASTER_STATUS_OK`, `IOLINK_MASTER_STATUS_PENDING`,
  `IOLINK_MASTER_ERR_*`, and the per-domain `..._ISDU_ERR_*` / `..._SIO_ERR_*` /
  `..._PARAM_ERR_*` enums), never bare magic integers.

This boundary matters because every later feature becomes harder to change once
external users compile against the storage sizes and result codes.

## 3. Protocol core

`src/master_port.c` is the core engine. It owns:

- **Port lifecycle**: `INACTIVE → STARTUP → PREOPERATE → OPERATE`, plus `ERROR`
  (`iolink_master_state_t`), driven by `iolink_master_process` /
  `iolink_master_poll_rx`.
- **Startup**: wake-up request, Type-0 idle exchange, operate transition command,
  and OPERATE entry; fixed baudrate or auto-baud scan across COM3→COM2→COM1 with a
  configurable per-baud `wake_retry_limit`.
- **Cyclic process data**: configured PD in/out for M-sequence Types 0, 1_1/1_2/1_V,
  2_1/2_2/2_V (`iolink_master_m_seq_type_t`), exposed via `iolink_master_set_pd_out`
  / `iolink_master_get_pd_in` / `iolink_master_get_od_status`.
- **RX/retry**: byte accumulation, checksum/CRC verification, bounded retry, and the
  diagnostics counters (`checksum_errors`, `response_timeouts`, `cycle_slips`,
  jitter, derived link quality).

Frame encode/decode and CRC come from the shared `../iolinki` helpers; the core adds
master-side sequencing on top.

## 4. Timing and scheduler

The core does **not** own a clock — the caller does. Timing is expressed as explicit
monotonic 100µs inputs so it is testable without wall-clock sleeps:

- `iolink_master_tick` / `iolink_master_tick_event` take an explicit tick event
  (`IOLINK_MASTER_TICK_CYCLE_DUE`, `..._RESPONSE_TIMEOUT`, `..._NONE`).
- `iolink_master_tick_at(port, event, now_100us)` applies `min_cycle_time` pacing
  and response-deadline scheduling against caller-supplied time.
- `iolink_master_get_next_tick_time(port, now_100us, out_next_100us)` tells the
  caller-owned hardware timer when the port is next due.
- `iolink_master_get_timing` exposes a read-only snapshot of scheduler-visible state
  (`iolink_master_timing_t`).

Response timeout (`response_timeout_100us`) is kept distinct from cycle spacing
(`min_cycle_time`); a zero response timeout falls back to `min_cycle_time`. The
MasterCycleTime octet (time-base + multiplier) is decoded to 100µs units by
`iolink_master_decode_min_cycle_time_100us` for both validation and pacing. This is
the current architecture-priority layer — see [`ROADMAP.md`](ROADMAP.md) §4.

## 5. PHY adapter boundary

The core is board-agnostic. Board support enters only through `iolink_phy_api_t`
(shared PHY struct: `init`/`set_mode`/`set_baudrate`/`send`/`recv_byte`, plus
optional `set_cq_line`/`detect_wakeup`/`get_voltage_mv`/`is_short_circuit`) and the
fallible adapter hooks carried in `iolink_master_config_t`
(`set_mode_checked`, `set_baudrate_checked`, `flush_rx`, `prepare_tx`, `prepare_rx`,
`read_cq_line` / `read_cq_line_checked`, `wake_up`). The PHY is retained **by
pointer** and must outlive the port. `iolink_master_validate_phy_contract` checks
that a PHY/config pair is complete enough for real hardware. The physical 80µs WURQ
pulse and `t_WU`/`t_REN`/`TDMT` startup timing are the adapter's responsibility and
are unverified on silicon. Full contract: [`PHY_BOUNDARY.md`](PHY_BOUNDARY.md);
implementing an adapter: [`PORTING.md`](PORTING.md).

## 6. Controller / multi-port runtime

`src/master_controller.c` initializes and drives an array of ports. It fans one
controller timestamp out to per-port cycle pacing
(`iolink_master_controller_tick_at`), applies per-port response deadlines, supports
independent per-port modes/timings, and exposes port count and per-port access
(`iolink_master_controller_get_port_count` / `..._get_port`). A failing port
returns its result without corrupting siblings. The controller computes the earliest
next due time across ports (`iolink_master_controller_get_next_tick_time`). It is a
fan-out helper, not yet a full port-policy scheduler.

## 7. Services layer

Services sit **above** cyclic transport in `src/master_isdu.c` (with identity in
`src/master_parameters.c`), kept independent of startup policy:

- **ISDU** read/write with segmentation into fixed buffers.
- **Direct Parameter Page 1** parse/apply/get/validate and capability-driven config
  selection (`src/master_parameters.c`), plus VendorID/DeviceID inspection.
- **Events**: code/detail read and ack, with optional rising-edge event-pending and
  per-event dispatch callbacks.
- **Data Storage**: read/write/restore with readback verification.
- **Block parameterization**: download/upload/store system commands and a
  write-with-readback sequence.
- **SIO DI/DQ** and dynamic mode transitions in `src/master_sio.c`.

## 8. Design principles

- **No dynamic memory** anywhere in `src/` or `include/` — all state is in
  caller-owned fixed storage.
- **No board headers in `src/master_*.c`**; no sleeping inside core calls.
- **Fixed-width types** (`<stdint.h>`) throughout for portability.
- **Named result codes**, checked at every call site.

Note on maturity: the core is protocol- and simulation-validated only. It is not
timing-certified, hardware-validated, or IO-Link-conformance validated — see
[`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md).
