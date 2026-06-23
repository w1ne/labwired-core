# Consolidate UDS ECU app + extend h563 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Factor the board-agnostic UDS ECU application into one shared file used by both f103 and h563, restore h563 to build against current udslib, and extend it to the same full tester-driven diagnostic session as f103 — over FDCAN.

**Architecture:** New `examples/common/uds_ecu_app.{c,h}` holds the DID table, seeded DTC store, and the routine/IO/comm/security-seed/reset handlers, plus `uds_ecu_app_fill_config(cfg, vin)`. A board-provided `uds_ecu_app_log()` hook lets the shared `security_seed` emit `UDS_SEED_SERVED` via each board's UART. f103 is refactored onto it (behavior-preserving, re-verified by its three smokes). h563 is ported to the instance ISO-TP API, converted to a pure tester-driven ECU, and driven by a scriptable `uds-tester` over FDCAN running the 12-step session.

**Tech Stack:** Rust (labwired-core), C against udslib (`~/projects/udslib`, ISO-14229), arm-none-eabi firmware (Cortex-M3 bxCAN + Cortex-M33 FDCAN), YAML scenarios.

## Global Constraints

- Git identity `w1ne <14119286+w1ne@users.noreply.github.com>`; every commit `Signed-off-by` via `git -c user.name=w1ne -c user.email=14119286+w1ne@users.noreply.github.com commit -s`.
- No "Claude"/"AI"/assistant references in commits, code, YAML, or docs.
- Integrate with `git merge`, never rebase. Branch `feat/uds-ecu-consolidate-h563` off `origin/main`.
- Pre-push Rust gate (`core-integrity`): `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` (default-members, NOT `--workspace`) + tests. Run all three before declaring a Rust task done.
- Firmware uses only public udslib headers (`uds/uds_core.h`, `uds/uds_isotp.h`, `uds/uds_dtc.h`, `uds/uds_dtc_store.h`). Extended-session id `0x03` via a local `#define ECU_SESSION_EXTENDED 0x03u` (internal-only otherwise). NRC convention: negative literal with a trailing comment. `UDS_OK` = 0. Do not set `restrict_sessions`.
- Both VINs are exactly 17 bytes: `LABWIRED-F103-UDS`, `LABWIRED-H563-UDS`. The shared DID table uses VIN `size = 17` and its `storage` pointer is set to the passed `vin` at config time.
- Tester `expect` is a prefix match; `expect_nrc` matches `[0x7F, sid, nrc]` exactly.
- Commit only intended source. The built ELF/`build/`/`Cargo.lock` are git-ignored. Never stage `third_party/iolinki`.
- Freestanding `memcpy`/`memset` shims stay in each board's `main.c` (both already define them); the shared file must NOT define them (duplicate-symbol clash).

---

### Task 1: Shared UDS ECU app file + f103 refactor

Extract f103's inline handlers into a shared file and refactor f103 to use it. Behavior-preserving: proven by f103's existing ELF build + three smokes.

**Files:**
- Create: `examples/common/uds_ecu_app.h`
- Create: `examples/common/uds_ecu_app.c`
- Modify: `examples/f103-uds-ecu/firmware/main.c` (remove inline handlers/storage, add log glue + fill_config call)
- Modify: `examples/f103-uds-ecu/firmware/Makefile` (build the shared file + `-I../../common`)

**Interfaces:**
- Produces: `void uds_ecu_app_fill_config(uds_config_t *cfg, const char *vin);` and the board-provided contract `void uds_ecu_app_log(const char *msg);`. `fill_config` sets `did_table`, `app_data`, `fn_dtc_list`, `fn_dtc_clear`, `fn_routine_control`, `fn_io_control`, `fn_comm_control`, `fn_security_seed`, `fn_reset`. The board sets `ecu_address`, `get_time_ms`, `fn_tp_send`, `rx_buffer(_size)`, `tx_buffer(_size)`, `p2_ms`, `p2_star_ms`, and defines `uds_ecu_app_log`.

- [ ] **Step 1: Create the header `examples/common/uds_ecu_app.h`**

```c
#ifndef UDS_ECU_APP_H
#define UDS_ECU_APP_H

#include "uds/uds_core.h"

/* Board-provided: emit a NUL-terminated trace string via the board UART. */
void uds_ecu_app_log(const char *msg);

/* Populate the board-agnostic UDS handlers, DID table, and a seeded DTC store
 * into `cfg`. Call AFTER setting the board-specific cfg fields and BEFORE
 * uds_init. `vin` must point to a 17-byte VIN (reported by DID 0xF190). */
void uds_ecu_app_fill_config(uds_config_t *cfg, const char *vin);

#endif /* UDS_ECU_APP_H */
```

- [ ] **Step 2: Create `examples/common/uds_ecu_app.c` with the shared handlers**

```c
#include <stddef.h>
#include <stdint.h>

#include "uds/uds_core.h"
#include "uds/uds_dtc.h"
#include "uds/uds_dtc_store.h"
#include "uds_ecu_app.h"

#define REG32(addr) (*(volatile uint32_t *) (addr))
#define ECU_SESSION_EXTENDED 0x03u /* UDS_SESSION_ID_EXTENDED (internal id) */

/* Writable scratch DID 0x0123 (read+write, EXTENDED session only). */
static uint8_t g_scratch[4];
/* IO-controlled point 0xA001 ("test lamp") for InputOutputControl 0x2F. */
static uint8_t g_lamp[1];

/* DID table. The 0xF190 VIN storage pointer is set in fill_config (per board);
 * the table is mutable for that reason. */
static uds_did_entry_t g_dids[] = {
    {0xF190u, 17u, 0u, 0u, NULL, NULL, NULL},
    {0x0123u, 4u, UDS_SESSION_EXTENDED, 0u, NULL, NULL, g_scratch},
    {0xA001u, 1u, 0u, 0u, NULL, NULL, g_lamp},
};

/* Reference DTC store, seeded with one failing DTC (0x123456). */
static uds_dtc_record_t g_dtc_backing[4];
static uds_dtc_store_t g_dtc_store;

static int security_seed(struct uds_ctx *ctx, uint8_t level, uint8_t *seed, uint16_t max_len)
{
    (void) ctx;
    (void) level;
    (void) max_len;
    uds_ecu_app_log("UDS_SEED_SERVED\n");
    seed[0] = 0xDE;
    seed[1] = 0xAD;
    seed[2] = 0xBE;
    seed[3] = 0xEF;
    return 4;
}

/* fn_reset hook: faithful CMSIS NVIC_SystemReset via AIRCR (works on M3 + M33).
 * udslib calls this only AFTER the 0x11 positive response (51 01) is on the
 * transport, so the reply is on the bus before SYSRESETREQ latches. */
static void ecu_reset(uds_ctx_t *ctx, uint8_t type)
{
    (void) ctx;
    (void) type;
    __asm volatile("dsb 0xF" ::: "memory");
    REG32(0xE000ED0Cu) = (0x05FAu << 16) | (1u << 2); /* AIRCR: VECTKEY | SYSRESETREQ */
    __asm volatile("dsb 0xF" ::: "memory");
    for (;;) {
    }
}

/* fn_routine_control: routine 0x0203, startRoutine in EXTENDED session only. */
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

/* fn_io_control: IO point 0xA001 (test lamp) — store and echo state. */
static int ecu_io(uds_ctx_t *ctx, uint16_t id, uint8_t type, const uint8_t *data, uint16_t len,
                  uint8_t *out, uint16_t max)
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

/* fn_comm_control: accept the requested communication mode. */
static int ecu_comm(uds_ctx_t *ctx, uint8_t ctrl_type, uint8_t comm_type, uint16_t node_id)
{
    (void) ctx;
    (void) ctrl_type;
    (void) comm_type;
    (void) node_id;
    return UDS_OK;
}

void uds_ecu_app_fill_config(uds_config_t *cfg, const char *vin)
{
    g_dids[0].storage = (void *) vin; /* 17-byte VIN reported by 0xF190 */

    uds_dtc_store_init(&g_dtc_store, g_dtc_backing, 4u, 40u);
    uds_dtc_store_register(&g_dtc_store, 0x123456u, UDS_DTC_SEVERITY_CHECK_IMMEDIATELY, 0x10u,
                           UDS_DTC_FGID_EMISSIONS);
    uds_dtc_store_report_test(&g_dtc_store, 0x123456u, true); /* set testFailed status */

    cfg->did_table.entries = g_dids;
    cfg->did_table.count = (uint16_t) (sizeof(g_dids) / sizeof(g_dids[0]));
    cfg->app_data = &g_dtc_store;
    cfg->fn_dtc_list = uds_dtc_store_list_cb;
    cfg->fn_dtc_clear = uds_dtc_store_clear_cb;
    cfg->fn_routine_control = ecu_routine;
    cfg->fn_io_control = ecu_io;
    cfg->fn_comm_control = ecu_comm;
    cfg->fn_security_seed = security_seed;
    cfg->fn_reset = ecu_reset;
}
```

- [ ] **Step 3: Refactor `examples/f103-uds-ecu/firmware/main.c`**

Delete these now-shared definitions from main.c: the `ECU_SESSION_EXTENDED` define, `g_vin`, `g_scratch`, `g_lamp`, `g_dids`, `g_dtc_backing`, `g_dtc_store`, and the functions `security_seed`, `ecu_reset`, `ecu_routine`, `ecu_io`, `ecu_comm`. (Keep `g_iso`, `g_iso_tx_sdu`, `g_rx_buf`, `g_tx_buf`, `g_now_ms`, `get_time_ms`, `isotp_send_adapter`, and all bxCAN/RCC/USART bring-up.)

Add the include near the other UDS includes:
```c
#include "uds_ecu_app.h"
```

Add the board log glue (near `uart_puts`, after it is defined):
```c
void uds_ecu_app_log(const char *msg) { uart_puts(msg); }
```

In `main`, the `cfg` initializer keeps only board fields; the handler fields come from `fill_config`. Replace the `uds_config_t cfg = { ... };` so it reads:
```c
    uds_config_t cfg = {
        .ecu_address = 0x10u,
        .get_time_ms = get_time_ms,
        .fn_tp_send = isotp_send_adapter,
        .rx_buffer = g_rx_buf,
        .rx_buffer_size = sizeof(g_rx_buf),
        .tx_buffer = g_tx_buf,
        .tx_buffer_size = sizeof(g_tx_buf),
        .p2_ms = 50u,
        .p2_star_ms = 2000u,
    };
    uds_ecu_app_fill_config(&cfg, "LABWIRED-F103-UDS");
```
(The DTC-store seeding lines that #343 placed before `cfg` are removed — `fill_config` now does the seeding.)

- [ ] **Step 4: Wire the shared file into the f103 Makefile**

In `examples/f103-uds-ecu/firmware/Makefile`: add `-I../../common` to `COMMON_CFLAGS` (line 8-9 region), add a `VPATH = .:../../common` line, and add `uds_ecu_app.c` to `APP_SRCS` (so it builds as `$(BUILD_DIR)/uds_ecu_app.o` via the existing `$(BUILD_DIR)/%.o: %.c` rule, found through VPATH).

```make
COMMON_CFLAGS := $(CPUFLAGS) -ffreestanding -fno-builtin -ffunction-sections \
                 -fdata-sections -Os -g -I$(UDSLIB_DIR)/include -I$(UDSLIB_DIR)/src/core \
                 -I../../common
VPATH = .:../../common
...
APP_SRCS := startup.c main.c uds_ecu_app.c
```
(Note: f103's APP_SRCS may differ slightly; preserve existing entries and append `uds_ecu_app.c`. If f103 has no `startup.c`, keep whatever it lists and add `uds_ecu_app.c`.)

- [ ] **Step 5: Build the f103 ELF clean**

Run: `make -C examples/f103-uds-ecu/firmware UDSLIB_DIR=$HOME/projects/udslib`
Expected: builds clean under `-Wall -Wextra -Werror`, produces `build/f103_uds_ecu.elf`, no duplicate-symbol or undefined-reference errors.

- [ ] **Step 6: Re-verify all three f103 smokes (behavior-preserving)**

Run each and confirm exit 0:
```
cargo run -p labwired-cli --bin labwired -- test --script examples/f103-uds-ecu/uds-session-smoke.yaml
cargo run -p labwired-cli --bin labwired -- test --script examples/f103-uds-ecu/uds-reset-smoke.yaml
cargo run -p labwired-cli --bin labwired -- test --script examples/f103-uds-ecu/uds-smoke.yaml
```
Expected: all exit 0. `uds-session` reports `uds_tester` done + `ECU_READY`; `uds-reset` reports done + `ECU_READY`; `uds-smoke` shows `F103-UDS-ECU`, `ECU_READY`, and `UDS_SEED_SERVED` (proves the shared `security_seed` + log hook works). If `UDS_SEED_SERVED` is missing, the log hook is not wired — fix before continuing.

- [ ] **Step 7: Commit**

```bash
git add examples/common/uds_ecu_app.h examples/common/uds_ecu_app.c examples/f103-uds-ecu/firmware/main.c examples/f103-uds-ecu/firmware/Makefile
git commit -s -m "refactor(uds-ecu): extract shared UDS app; f103 uses it"
```

---

### Task 2: Rust regression — scriptable tester over FDCAN

The `uds-tester` has only ever driven bxCAN. Validate it drives an `Fdcan` peripheral in normal mode (where #343 found the bxCAN FlowControl-drop bug) and lock it with a test. If bring-up exposes a gap, fix it minimally and guard it.

**Files:**
- Modify: `crates/core/src/bus/mod.rs` (test module; production code only if a real gap is found)

**Interfaces:**
- Consumes: `service_can_uds_testers` (handles `Fdcan` via `tx_frames` drain + `receive_frame` inject), `CanUdsTester`, `CanUdsTesterState`, the `Fdcan` peripheral (`crates/core/src/peripherals/fdcan.rs`: `receive_frame`, `tx_frames`, normal-mode = `CCCR_INIT` cleared, non-loopback). Mirror the existing `uds_tester_completes_against_real_bxcan` test (around mod.rs:2350) and the firmware's `fdcan_start` register sequence (CCCR INIT|CCE → TEST=0 → CCCR=0) for the bring-up.

- [ ] **Step 1: Write a failing/validating test driving the tester over a real Fdcan**

Add a `#[test] uds_tester_completes_against_real_fdcan` to the test module. Build a `SystemBus` with an `Fdcan` peripheral named `fdcan1` at its base, bring it to normal mode (mirror `fdcan.rs`), attach a `CanUdsTester` with a script whose ECU reply is **multi-frame** (so the FC-delivery path is exercised on FDCAN). Inject the ECU FirstFrame via the Fdcan's `tx_frames` (the tester drains it), then assert the tester delivered a FlowControl frame to the Fdcan (via `receive_frame` / the Fdcan RX path), inject the CF, and assert the exchange reaches `Done`. Read `fdcan.rs` to use the exact RX-observation method (mirror how `uds_tester_completes_against_real_bxcan` observes injected frames, adapted for Fdcan).

The assertion must verify FC **delivery** on FDCAN (not merely final `Done`), matching the discrimination standard from #343's `uds_tester_multiframe_ecu_response_injects_flowcontrol`.

- [ ] **Step 2: Run the test**

Run: `cargo test -p labwired-core --lib uds_tester_completes_against_real_fdcan -- --nocapture`
Expected: PASS if the FDCAN tester path already works. If it FAILS (the tester can't drive Fdcan — e.g. normal-mode delivery, FD framing, or FC not injected), that is a real gap: go to Step 3.

- [ ] **Step 3: If a gap was found, fix it minimally in production code**

Apply the smallest fix to `service_can_uds_testers` / `Fdcan` that makes the path work (e.g. the FDCAN analogue of the #343 `AwaitMultiResp` FC forwarding, or a normal-mode delivery/filter fix in `fdcan.rs`). Keep the test from Step 1 as the guard; if you removed the fix the test must go RED. Document the gap + fix in the report. If NO gap was found, skip this step and note "FDCAN tester path worked unmodified."

- [ ] **Step 4: Full Rust gate**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test -p labwired-core --lib`
Expected: fmt clean, clippy clean, all tests pass (including the new FDCAN test).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/bus/mod.rs
git commit -s -m "test(bus): cover scriptable uds-tester driving FDCAN in normal mode"
```
(If Step 3 applied a production fix, say so in the commit body.)

---

### Task 3: Port + extend the h563 ECU firmware

Port h563's firmware to the instance ISO-TP API, convert it to a pure tester-driven ECU, and wire the shared handlers. Deliverable: clean ELF build (runtime proof is Task 4).

**Files:**
- Modify: `examples/h563-uds-ecu/firmware/main.c`
- Modify: `examples/h563-uds-ecu/firmware/Makefile`

**Interfaces:**
- Consumes: `uds_ecu_app_fill_config` / `uds_ecu_app_log` (Task 1); current udslib instance ISO-TP API: `uds_tp_isotp_init(uds_isotp_ctx_t*, can_send, tx_id, rx_id, tx_sdu_buf, tx_sdu_size)`, `uds_tp_isotp_set_fd(uds_isotp_ctx_t*, bool)`, `uds_isotp_rx_callback(uds_isotp_ctx_t*, struct uds_ctx*, id, data, len)`, `uds_tp_isotp_process(uds_isotp_ctx_t*, time)`, `int uds_isotp_send(uds_isotp_ctx_t*, const uint8_t*, uint16_t)`. Keep all existing FDCAN/USART helpers (`fdcan_start`, `fdcan_send_frame`, `fdcan_poll_rx_frame`, `write_payload`/`read_payload`, `len_to_dlc`/`dlc_to_len`, `can_send`, `uart_*`, `get_time_ms`).
- Produces: an h563 ECU that answers the full session via the shared handlers, reachable by an external tester on request id 0x7E0 / reply 0x7E8, and reboots via AIRCR on 0x11.

- [ ] **Step 1: Remove the self-test + stale UDS plumbing from main.c**

Delete: `g_positive_response_sent`, `g_dids`, `g_did_table`, `app_read_data_by_id`, `g_user_services`, `pump_one_tester_request`, `positive_vin_response_seen`, and the old `main` body's self-send/verify loop and `UDS_OK`/`UDS_FAIL`/`VIN=...` prints. Keep `g_rx_buffer`, `g_tx_buffer`, `g_vin`? — remove `g_vin` (VIN now passed to fill_config). Keep `g_now_ms`, `get_time_ms`, `can_send`, and all FDCAN/UART helpers.

- [ ] **Step 2: Add the instance ISO-TP plumbing + shared includes + log glue**

Add includes near the existing UDS includes:
```c
#include "uds_ecu_app.h"
```
Add ISO-TP instance state (near the buffers):
```c
static uds_isotp_ctx_t g_iso;
static uint8_t g_iso_tx_sdu[64];
```
Add the transport adapter and the log glue:
```c
static int isotp_send_adapter(struct uds_ctx *ctx, const uint8_t *data, uint16_t len)
{
    (void) ctx;
    return uds_isotp_send(&g_iso, data, len);
}

void uds_ecu_app_log(const char *msg) { uart_puts(msg); }
```

- [ ] **Step 3: Rewrite `main` as a pure tester-driven ECU**

```c
int main(void)
{
    uart_init();
    uart_puts("H563-UDS-ECU\n");

    fdcan_start();
    uds_tp_isotp_init(&g_iso, can_send, 0x7E8u, 0x7E0u, g_iso_tx_sdu, sizeof(g_iso_tx_sdu));
    uds_tp_isotp_set_fd(&g_iso, true);
    uart_puts("ECU_READY\n");

    uds_config_t cfg = {
        .ecu_address = 0x10u,
        .get_time_ms = get_time_ms,
        .fn_tp_send = isotp_send_adapter,
        .rx_buffer = g_rx_buffer,
        .rx_buffer_size = sizeof(g_rx_buffer),
        .tx_buffer = g_tx_buffer,
        .tx_buffer_size = sizeof(g_tx_buffer),
        .p2_ms = 50u,
        .p2_star_ms = 2000u,
    };
    uds_ecu_app_fill_config(&cfg, "LABWIRED-H563-UDS");

    uds_ctx_t ctx;
    if (uds_init(&ctx, &cfg) != UDS_OK) {
        uart_puts("UDS_INIT_FAIL\n");
        for (;;) {
        }
    }

    for (;;) {
        can_frame_t frame;
        if (fdcan_poll_rx_frame(&frame)) {
            uds_isotp_rx_callback(&g_iso, &ctx, frame.id, frame.data, frame.len);
        }
        uds_process(&ctx);
        uds_tp_isotp_process(&g_iso, g_now_ms);
        ++g_now_ms;
    }
}
```
Ensure `fdcan_start()` leaves the controller in **normal mode** (it already clears TEST and INIT) and that the FDCAN RX accepts frames with id 0x7E0 into FIFO0. If the emulated FDCAN requires an acceptance-filter/RXGFC setup to receive 0x7E0 (verified in Task 4's smoke), add the minimal filter config in `fdcan_start` mirroring what the model needs; otherwise leave as-is.

- [ ] **Step 4: Update the h563 Makefile**

Add `$(UDSLIB_DIR)/src/services/uds_dtc_store.c` to `UDS_SRCS`. Add `-I../../common` to `COMMON_CFLAGS`, a `VPATH = .:../../common` line, and `uds_ecu_app.c` to `APP_SRCS`.

- [ ] **Step 5: Build the h563 ELF clean (this currently fails on `main` — must now pass)**

Run: `make -C examples/h563-uds-ecu/firmware UDSLIB_DIR=$HOME/projects/udslib`
Expected: builds clean under `-Wall -Wextra -Werror`, produces the h563 ELF, no API-signature errors (the six pre-existing errors are gone), no duplicate-symbol/undefined-reference.

- [ ] **Step 6: Commit**

```bash
git add examples/h563-uds-ecu/firmware/main.c examples/h563-uds-ecu/firmware/Makefile
git commit -s -m "feat(h563-uds): port to instance ISO-TP, pure tester-driven ECU on shared app"
```

---

### Task 4: h563 full-session scenario, smoke, and docs

Drive the h563 ECU through the full session over FDCAN and assert each reply; document. "Actually use what you ship": run the smoke + negative control.

**Files:**
- Modify: `examples/h563-uds-ecu/system.yaml` (replace `can-diagnostic-tester` with scriptable `uds-tester`)
- Create: `examples/h563-uds-ecu/uds-session-smoke.yaml`
- Modify: `examples/h563-uds-ecu/uds-smoke.yaml` (update to the new tester-driven model)
- Modify: `examples/h563-uds-ecu/README.md`

**Interfaces:**
- Consumes: the Task 3 h563 firmware behavior; the scriptable `uds-tester` device on `fdcan1` (validated in Task 2); the CLI runner `labwired test --script` with `uart_contains` + `uds_tester: {id, result: done}` assertions.

- [ ] **Step 1: Replace the tester in `system.yaml` with the scriptable full session**

```yaml
name: "h563-uds-ecu"
chip: "../../configs/chips/stm32h563.yaml"
external_devices:
  - type: "uds-tester"
    id: "uds-tester"
    connection: "fdcan1"
    config:
      request_id: 0x7E0
      reply_id: 0x7E8
      script:
        - send: "22 F1 90"
          expect: "62 F1 90"
        - send: "2E 01 23 DE AD BE EF"
          expect_nrc: 0x31
        - send: "10 03"
          expect: "50 03"
        - send: "3E 00"
          expect: "7E 00"
        - send: "2E 01 23 DE AD BE EF"
          expect: "6E 01 23"
        - send: "22 01 23"
          expect: "62 01 23 DE AD BE EF"
        - send: "19 01 09"
          expect: "59 01"
        - send: "14 FF FF FF"
          expect: "54"
        - send: "31 01 02 03"
          expect: "71 01 02 03"
        - send: "2F A0 01 03 01"
          expect: "6F A0 01"
        - send: "28 00 01"
          expect: "68 00"
        - send: "11 01"
          expect: "51 01"
board_io: []
```

- [ ] **Step 2: Create `examples/h563-uds-ecu/uds-session-smoke.yaml`**

```yaml
schema_version: "1.0"
inputs:
  system: "./system.yaml"
  firmware: "./firmware/build/h563_uds_ecu.elf"
limits:
  max_steps: 2000000
assertions:
  - uart_contains: "ECU_READY"
  - uds_tester: { id: "uds-tester", result: done }
```
(Use the actual h563 ELF filename produced by Task 3's Makefile; check `firmware/build/*.elf` and match it here.)

- [ ] **Step 3: Build the ELF (if needed) and run the smoke**

```
make -C examples/h563-uds-ecu/firmware UDSLIB_DIR=$HOME/projects/udslib
cargo run -p labwired-cli --bin labwired -- test --script examples/h563-uds-ecu/uds-session-smoke.yaml
```
Expected: exit 0; `uds-tester` assertion `done`; `ECU_READY` satisfied (appears twice — the 0x11 ECUReset reboots the ECU). If the smoke FAILS at a step and it is not a scenario typo, do NOT tweak bytes to force a pass — STOP and report BLOCKED with the failing step + tester failure message (it may be an FDCAN RX-filter gap in Task 3 or an FDCAN tester gap for Task 2 to fix).

- [ ] **Step 4: Negative control — prove the assertion is live**

Temporarily change step 1's `expect: "62 F1 90"` to `"62 F1 91"`, re-run the smoke, confirm non-zero exit with a step-0 failure, then revert and confirm pass again. Leave the file passing.

- [ ] **Step 5: Reconcile `uds-smoke.yaml` to the new model**

The old `uds-smoke.yaml` asserted the self-test UART strings (`UDS_REQ_22_F190`, `UDS_RESP_62_F190`, `VIN=...`, `UDS_OK`), which no longer exist. Update it to a minimal tester-driven VIN-read smoke (its own small system file or reusing the session system), asserting `uart_contains: "H563-UDS-ECU"`, `uart_contains: "ECU_READY"`, and `uds_tester: {id, result: done}` for a single `22 F1 90 → 62 F1 90` script — OR remove `uds-smoke.yaml` if `uds-session-smoke.yaml` supersedes it. Pick the smaller change; do not leave a scenario asserting strings the firmware no longer prints.

- [ ] **Step 6: Update `examples/h563-uds-ecu/README.md`**

Describe the new tester-driven full-session scenario (FDCAN), state the CI-vs-local split (smoke needs a locally-built ELF via `make -C firmware UDSLIB_DIR=$HOME/projects/udslib`, not a clean-checkout gate; always-on regression is the `uds_tester_*` tests including the FDCAN one), and that the ECU shares its UDS application with f103 via `examples/common/uds_ecu_app.c`. Mirror the structure of the f103 README's scenarios section.

- [ ] **Step 7: Commit**

```bash
git add examples/h563-uds-ecu/system.yaml examples/h563-uds-ecu/uds-session-smoke.yaml examples/h563-uds-ecu/uds-smoke.yaml examples/h563-uds-ecu/README.md
git commit -s -m "feat(h563-uds): full diagnostic-session scenario over FDCAN and docs"
```

---

## Final verification (after all tasks)

- [ ] Rust gate: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test -p labwired-core --lib`
- [ ] Both ELFs build clean: f103 and h563 firmware `make` under `-Werror`.
- [ ] Smokes green: f103 `uds-session`/`uds-reset`/`uds-smoke` (refactor regression) and h563 `uds-session-smoke` (exit 0, `done`, `ECU_READY` ×2).
- [ ] `git status` shows no staged `third_party/iolinki`; only intended files changed; all commits signed-off, no assistant references.
