# Porting Guide — implementing a master PHY adapter

`iolinki-master` is board-agnostic. To run it on real hardware you implement the
PHY contract and the fallible adapter hooks; the protocol core never includes board
headers and never sleeps. This guide is the how-to; the normative contract and
adapter rules live in [`PHY_BOUNDARY.md`](PHY_BOUNDARY.md). Nothing here has been
run on silicon yet — treat the timing responsibilities below as the spec you must
satisfy, not as validated behavior.

## Two surfaces to implement

Board support enters the core through **two** structures, not one:

1. **`iolink_phy_api_t`** (shared from `iolinki/phy.h`) — the transceiver/UART
   driver.
2. **Adapter hooks in `iolink_master_config_t`** (in
   `include/iolinki_master/master.h`) — the fallible, master-specific operations
   the core calls around startup and each frame.

### 1. The PHY struct (`iolink_phy_api_t`)

```c
typedef struct {
    void* user;                                             /* passed to every call */
    int  (*init)(void* user);                               /* 0 ok, <0 hw failure */
    void (*set_mode)(void* user, iolink_phy_mode_t mode);   /* INACTIVE/SIO/SDCI */
    void (*set_baudrate)(void* user, iolink_baudrate_t b);  /* COM1/COM2/COM3 */
    int  (*send)(void* user, const uint8_t* data, size_t len); /* exact len, or <0 */
    int  (*recv_byte)(void* user, uint8_t* byte);           /* 1 got, 0 none, <0 err */
    int  (*detect_wakeup)(void* user);                      /* optional */
    void (*set_cq_line)(void* user, uint8_t state);         /* optional, DQ mode */
    int  (*get_voltage_mv)(void* user);                     /* optional, L+ diag */
    bool (*is_short_circuit)(void* user);                   /* optional, fault diag */
} iolink_phy_api_t;
```

Rules that matter for the master core:

- `send` must return the exact length or a negative/short result — never a partial
  "success" — so the core can enter error handling.
- `recv_byte` must be non-blocking and must surface UART framing errors as a
  negative return, not hide them.
- `set_cq_line` is required for DQ output; `get_voltage_mv` / `is_short_circuit`
  feed `iolink_master_get_diagnostics` when present.
- The PHY is retained **by pointer** and must outlive the port. Never pass a stack
  temporary to `iolink_master_init`.

### 2. The config adapter hooks

These live in `iolink_master_config_t` and are what
`iolink_master_validate_phy_contract()` requires for strict hardware use:

| Hook | When the core calls it | Your job |
|---|---|---|
| `set_mode_checked(mode)` | mode transitions | Switch the transceiver into SDCI / SIO / inactive and **return non-zero on failure** |
| `set_baudrate_checked(baud)` | fixed and auto-baud startup | Apply COM1/COM2/COM3 and report failure |
| `flush_rx()` | before startup and before each retry / baud change | Clear the UART/adapter RX FIFO so stale bytes cannot bleed across attempts |
| `prepare_tx()` | before each core-driven `send` | Switch the half-duplex driver to transmit |
| `prepare_rx()` | after each `send` | Switch back to receive; return non-zero if you cannot, so the core stops instead of listening in the wrong direction |
| `wake_up()` | startup, per `wake_retry_limit` | Generate the master wake-up request (WURQ) — see timing below |
| `read_cq_line_checked()` | DI mode | Read the C/Q line, report adapter failure |
| `read_cq_line()` | DI mode (permissive fallback) | Legacy reader for tests/partial fakes |

`iolink_master_init()` stays permissive (it accepts partial fake PHYs for unit
tests); real adapters should pass `iolink_master_validate_phy_contract()` before a
hardware run.

## Timing that lives in the adapter — not the core

The core supplies monotonic 100µs pacing (`iolink_master_tick_at`), but the physical
line timing is **entirely the adapter's responsibility** and is currently
unverified on hardware:

- **The 80µs WURQ wake pulse.** `wake_up()` must generate the master wake-up
  request (a defined wake pulse on C/Q). The core only decides *when* to call it and
  how many times (`wake_retry_limit`); it does not shape the pulse.
- **`t_WU`** — the wake-up recovery / device-ready window after the pulse before the
  first master message.
- **`t_REN`** — the driver-enable / receiver-enable settling around half-duplex
  direction changes, which is why `prepare_tx` / `prepare_rx` exist as explicit
  hooks.
- **`TDMT`** — the master's inter-frame idle time before it starts a new message.

If your transceiver or MCU UART cannot meet these windows, that is a hardware/timing
limitation the core cannot paper over. Validate them with a logic analyzer per
[`HARDWARE_VALIDATION.md`](HARDWARE_VALIDATION.md).

## Wiring it up

```c
static const iolink_phy_api_t my_phy = {
    .user = &my_board_ctx,
    .init = my_init, .set_mode = my_set_mode, .set_baudrate = my_set_baud,
    .send = my_send, .recv_byte = my_recv, .set_cq_line = my_set_cq,
    .get_voltage_mv = my_vmon, .is_short_circuit = my_fault,
};

static iolink_master_config_t cfg = {
    .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
    .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1,
    .baudrate = IOLINK_BAUDRATE_COM3,
    .min_cycle_time = 20U, .pd_in_len = 1U, .pd_out_len = 1U,
    .response_timeout_100us = 30U, .wake_retry_limit = 3U,
    .set_mode_checked = my_set_mode_checked,
    .set_baudrate_checked = my_set_baud_checked,
    .flush_rx = my_flush_rx,
    .prepare_tx = my_prepare_tx, .prepare_rx = my_prepare_rx,
    .wake_up = my_wake_up,
    .read_cq_line_checked = my_read_cq_checked,
};

iolink_master_port_t port;                 /* caller-owned, no heap */
if (iolink_master_validate_phy_contract(&my_phy, &cfg) != IOLINK_MASTER_STATUS_OK) { /* fix adapter */ }
if (iolink_master_init(&port, &my_phy, &cfg) != IOLINK_MASTER_STATUS_OK)            { /* handle */ }
```

Then run it from your timer loop: compute the next due time with
`iolink_master_get_next_tick_time`, and on each due tick call
`iolink_master_tick_at(&port, event, now_100us)` followed by
`iolink_master_poll_rx(&port)`. See [`API.md`](API.md) for the tick model.

## Adapter don'ts

- Do not include board headers from `src/master_*.c` — adapter code lives outside
  the core.
- Do not sleep inside core calls; schedule the next call with the next-due-time
  helpers.
- Do not mutate core state from a fault callback; surface faults through PHY
  callbacks / diagnostics only.
