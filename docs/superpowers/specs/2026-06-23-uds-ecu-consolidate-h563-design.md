# Consolidate UDS ECU app + extend h563 to full session — Design

**Date:** 2026-06-23
**Status:** Approved (design), pending spec review
**Branch:** `feat/uds-ecu-consolidate-h563` (off `origin/main` ab8fc6ec)

## Problem

Two issues with the UDS ECU examples:

1. **Asymmetry.** After PR #343, `f103-uds-ecu` answers the full everyday UDS
   diagnostic session, but `h563-uds-ecu` (the CAN-FD / FDCAN counterpart) only
   handles a single DID read (VIN 0xF190).
2. **h563 is broken.** Its firmware still uses the pre-instance udslib ISO-TP
   API and **does not compile** against current udslib (verified: 6 errors —
   `uds_tp_isotp_init`/`uds_isotp_rx_callback`/`uds_tp_isotp_process`/
   `uds_tp_isotp_set_fd` signatures, `fn_tp_send` type, missing `address_mode`
   service-entry field). The firmware build is not a CI gate, so it rotted.
3. **Duplication.** f103's handler set (DID table, DTC store, routine/IO/comm,
   security seed, reset) lives inline in its `main.c`; bringing h563 to parity
   would duplicate all of it.

This work consolidates the board-agnostic UDS application into one shared file,
restores h563 to build against current udslib, and extends it to the same full
tester-driven session as f103 — over FDCAN instead of bxCAN.

## Goals / non-goals

In scope: a shared `examples/common/uds_ecu_app.{c,h}`; f103 refactored to use
it (re-verified); h563 ported to current udslib + converted to a pure
tester-driven ECU running the full session over FDCAN; a Rust regression for the
scriptable tester driving an FDCAN peripheral.

Out of scope: `h563-uds-bootloader` (separate OTA example); new UDS services
beyond the #343 set; any chip-model feature work unless a concrete gap blocks
the FDCAN tester path.

## Decisions (approved)

- **h563 becomes a pure tester-driven ECU** (like f103): drop the self-send /
  self-check loop and the `UDS_OK`/`UDS_FAIL` UART verdict; print `ECU_READY`
  and answer the bus; the external scriptable `uds-tester` drives and asserts.
- **VIN is parameterized per board**: the shared DID table reads a VIN the board
  passes via `uds_ecu_app_fill_config(cfg, vin)`. f103 keeps
  `LABWIRED-F103-UDS`; h563 keeps `LABWIRED-H563-UDS`.

## Architecture

### 1. Shared application (`examples/common/uds_ecu_app.{c,h}`)

Board-agnostic, transport-agnostic. Holds exactly what #343 put in f103's
`main.c`, made reusable:

- Storage: `g_scratch[4]` (DID 0x0123), `g_lamp[1]` (IO 0xA001), the DTC backing
  array + `uds_dtc_store`, and a VIN pointer set at config time.
- DID table entries for `0xF190` (VIN, read-only, any session — storage set from
  the `vin` arg), `0x0123` (scratch, read/write, `UDS_SESSION_EXTENDED`),
  `0xA001` (IO point).
- Handlers: `ecu_routine` (routine 0x0203, extended-only), `ecu_io` (0xA001),
  `ecu_comm` (accept), `security_seed` (DE AD BE EF), `ecu_reset` (AIRCR
  `0xE000ED0C` VECTKEY|SYSRESETREQ — works on both M3 and M33).
- `void uds_ecu_app_fill_config(uds_config_t *cfg, const char *vin);` — seeds the
  DTC store and sets `did_table`, `app_data`, `fn_dtc_list`, `fn_dtc_clear`,
  `fn_routine_control`, `fn_io_control`, `fn_comm_control`, `fn_security_seed`,
  `fn_reset`. The board's `main` sets the board-specific fields it must not own:
  `ecu_address`, `get_time_ms`, `fn_tp_send`, `rx_buffer(_size)`,
  `tx_buffer(_size)`, `p2_ms`, `p2_star_ms`.
- The shared file declares its own freestanding `memcpy`/`memset` shims? **No** —
  to avoid duplicate-symbol clashes, the shims stay in each board's `main.c`
  (both already define them). The shared file uses only what udslib needs.

The header exposes `uds_ecu_app_fill_config` and the VIN-length contract; nothing
else is public.

### 2. f103 refactor

`examples/f103-uds-ecu/firmware/main.c`: remove the inline DID/DTC/IO storage and
the `ecu_routine`/`ecu_io`/`ecu_comm`/`security_seed`/`ecu_reset` definitions
(they move to the shared file), `#include "uds_ecu_app.h"`, and replace the
hand-written `cfg` handler fields with one `uds_ecu_app_fill_config(&cfg,
"LABWIRED-F103-UDS")` call plus the board-specific field assignments. Keep all
bxCAN/RCC/USART/ISO-TP bring-up unchanged. **Re-verify the existing f103 smokes**
(`uds-session-smoke.yaml`, `uds-reset-smoke.yaml`, `uds-smoke.yaml`) pass
unchanged — this refactor must be behavior-preserving.

### 3. h563 port + extend

`examples/h563-uds-ecu/firmware/main.c`:
- Port to the instance ISO-TP API: allocate `static uds_isotp_ctx_t g_iso;` and
  `static uint8_t g_iso_tx_sdu[64];`; call `uds_tp_isotp_init(&g_iso, can_send,
  0x7E8, 0x7E0, g_iso_tx_sdu, sizeof(g_iso_tx_sdu))`, `uds_tp_isotp_set_fd(&g_iso,
  true)`, `uds_isotp_rx_callback(&g_iso, &ctx, id, data, len)`,
  `uds_tp_isotp_process(&g_iso, t)`, and a `fn_tp_send` adapter
  `int isotp_send_adapter(struct uds_ctx*, const uint8_t*, uint16_t)` that calls
  `uds_isotp_send(&g_iso, data, len)` (matching f103).
- Replace the single `user_services`/`app_read_data_by_id` handler with
  `uds_ecu_app_fill_config(&cfg, "LABWIRED-H563-UDS")`.
- Ensure FDCAN runs in **normal mode** (not loopback) and accepts 0x7E0 into the
  RX FIFO so external tester frames arrive (today's `fdcan_start` configured for
  the self-test path; confirm/adjust filtering for external RX).
- Convert `main` to a pure ECU loop: `ECU_READY` banner, then
  `poll → uds_isotp_rx_callback → uds_process → uds_tp_isotp_process` forever.
  Remove `fdcan_send_frame(request)`, `positive_vin_response_seen`,
  `g_positive_response_sent`, and the `UDS_OK`/`UDS_FAIL` verdict.
- Reset: `ecu_reset` (shared) writes AIRCR; the M33 machine reboots via the same
  core-implicit SCB path as f103.

`examples/h563-uds-ecu/firmware/Makefile`: add `$(UDSLIB_DIR)/src/services/
uds_dtc_store.c` to `UDS_SRCS`, add the shared file to the build (VPATH
`../../common` + `uds_ecu_app.c` in `APP_SRCS`, or an explicit path), and add
`-I../../common` to the C flags.

`examples/h563-uds-ecu/system.yaml`: replace the `can-diagnostic-tester` device
with a scriptable `uds-tester` on `fdcan1`, `request_id: 0x7E0`,
`reply_id: 0x7E8`, running the same 12-step script as f103's `uds-session.yaml`
(adjusted only for ids). `examples/h563-uds-ecu/uds-session-smoke.yaml`:
assertions `uart_contains: ECU_READY` + `uds_tester: { id: <id>, result: done }`.
Keep or update `uds-smoke.yaml` to the new model (the old self-test UART asserts
no longer apply).

### 4. Rust regression (FDCAN tester path)

No example has driven the scriptable `uds-tester` over FDCAN before (f103 used
bxCAN). The path exists (`service_can_uds_testers` handles `Fdcan` via
`tx_frames`/`receive_frame`) but is unexercised — exactly where #343 found the
bxCAN FlowControl-drop bug. Add an FSM test in `crates/core/src/bus/mod.rs`
mirroring `uds_tester_completes_against_real_bxcan` but against a real `Fdcan`
peripheral in normal mode: a multi-frame ECU response so the FC-delivery path is
covered on FDCAN too. If end-to-end bring-up reveals an FDCAN-tester gap (normal
mode, filtering, FD framing), fix it minimally and guard it with a test — same
discipline as the #343 FC fix.

## Testing

- Rust gate: `cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
  cargo test -p labwired-core --lib` (the `core-integrity` gate).
- Build both ELFs: `make -C examples/f103-uds-ecu/firmware
  UDSLIB_DIR=$HOME/projects/udslib` and the h563 equivalent — both clean under
  `-Wall -Wextra -Werror`.
- Smokes (local, not clean-checkout gates): f103 `uds-session-smoke.yaml` /
  `uds-reset-smoke.yaml` still pass (refactor regression); h563
  `uds-session-smoke.yaml` passes (exit 0, `result: done`, `ECU_READY` ×2 from
  the reset). Negative control on the h563 scenario.
- READMEs for both examples updated; the CI-vs-local split restated.

## Risks

- **FDCAN tester path unexercised** (primary risk). Budget for a minimal Rust fix
  + regression test if normal-mode FDCAN drive reveals a gap (FC delivery,
  filtering, FD DLC). Reset on M33 is confirmed working (SCB core-implicit).
- **h563 currently doesn't build** — porting to the instance ISO-TP API is the
  first step; the design mirrors f103's known-good transport wiring verbatim.
- **Refactoring the just-shipped f103** could regress it; mitigated by re-running
  all three f103 smokes after the refactor.
- **Shared-file symbol clashes** (memcpy/memset shims): keep the shims in each
  board's `main.c`, not the shared file.
- VIN length must match each board's DID `size` field; the shared table derives
  size from the passed VIN to avoid a mismatch.
