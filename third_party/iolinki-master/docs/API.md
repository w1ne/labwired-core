# iolinki-master API Reference

A guided tour of the public API in
[`include/iolinki_master/master.h`](../include/iolinki_master/master.h). Every
function returns a named result constant — `IOLINK_MASTER_STATUS_OK` (0),
`IOLINK_MASTER_STATUS_PENDING` (1), or a negative `IOLINK_MASTER_ERR_*` /
`..._ISDU_ERR_*` / `..._SIO_ERR_*` / `..._PARAM_ERR_*` code. Check every return.

All state is caller-owned: you allocate `iolink_master_port_t` (a fixed-size opaque
union, 1280 B) or `iolink_master_controller_t` (32 B). There is no heap.

## 1. Configuration and init

Fill an `iolink_master_config_t`, then init a port against a PHY:

```c
int iolink_master_init(iolink_master_port_t* port,
                       const iolink_phy_api_t* phy,
                       const iolink_master_config_t* config);
```

Key config fields (`iolink_master_config_t`): `port_mode`
(`IOLINK_MASTER_PORT_MODE_IOLINK` / `_DI` / `_DQ` / `_DEACTIVATED`), `m_seq_type`,
`baudrate`, `min_cycle_time`, `pd_in_len` / `pd_out_len`, `auto_baudrate`,
`response_timeout_100us`, `wake_retry_limit`, the identity fields
(`validate_device_info`, `inspection_level`, `expected_vendor_id`,
`expected_device_id`), the event callbacks (§6), and the PHY adapter hooks
(`set_mode_checked`, `set_baudrate_checked`, `flush_rx`, `prepare_tx`,
`prepare_rx`, `read_cq_line` / `read_cq_line_checked`, `wake_up` — see
[`PORTING.md`](PORTING.md)).

> **Lifetime:** the config is copied into the port, but the PHY is retained **by
> pointer**. The `iolink_phy_api_t` must outlive the port — never pass a stack
> temporary.

Related: `iolink_master_validate_phy_contract(phy, config)` checks the PHY/config
pair is complete for real hardware; `iolink_master_restart(port)` restarts startup;
`iolink_master_get_state(port)` returns the current `iolink_master_state_t`.

## 2. The tick / scheduler model

The core owns no clock. You drive it and supply time.

- `iolink_master_process(port)` — send one pending startup/preoperate/operate action.
- `iolink_master_poll_rx(port)` — decode available RX bytes; returns the decoded
  frame count, `OK` when no byte is available, or `INVALID_ARG`/`ERR_FRAME`/`ERR_CHECKSUM`.
- `iolink_master_tick(port, response_timeout)` — bool-flag tick.
- `iolink_master_tick_event(port, event)` — explicit event tick
  (`IOLINK_MASTER_TICK_NONE` / `_CYCLE_DUE` / `_RESPONSE_TIMEOUT`).
- `iolink_master_tick_at(port, event, now_100us)` — as above, applying monotonic
  100µs `min_cycle_time` pacing.
- `iolink_master_get_next_tick_time(port, now_100us, &out_next_100us)` — when the
  port is next due, for your hardware timer.
- `iolink_master_on_timeout(port)` — advance the retry policy on a response timeout;
  returns `OK`, `PENDING` while retrying, or `ERR_RETRY_LIMIT`.
- `iolink_master_get_timing(port, &timing)` — read-only scheduler snapshot.

`response_timeout_100us` controls the response deadline; `min_cycle_time` controls
cycle spacing (a zero response timeout falls back to `min_cycle_time`).

## 3. Process data

```c
int iolink_master_set_pd_out(iolink_master_port_t* port, const uint8_t* data, uint8_t len);
int iolink_master_get_pd_in(const iolink_master_port_t* port,
                            uint8_t* buffer, uint8_t buffer_len, uint8_t* out_len);
int iolink_master_get_od_status(const iolink_master_port_t* port, uint8_t* status);
```

`set_pd_out` returns `ERR_BUFFER_TOO_SMALL` if `len` does not match the configured
PD-out size. `get_pd_in` returns `PENDING` until valid PD has arrived.

## 4. Device identity and Direct Parameter Page 1

- `iolink_master_parse_direct_parameter_page1(page, len, &info)` — decode a raw
  page-1 buffer into `iolink_master_device_info_t`.
- `iolink_master_apply_direct_parameter_page1(port, page, len)` — parse and store.
- `iolink_master_get_device_info(port, &info)` / `iolink_master_read_device_info(port)`.
- `iolink_master_validate_device_info(port)` — VendorID/DeviceID vs configured
  expectations at the selected `inspection_level` (`NO_CHECK` / `TYPE_COMP` /
  `IDENTICAL`; the SerialNumber leg of `IDENTICAL` is not yet wired).
- `iolink_master_select_config_from_device_info(&info, &config)` and
  `iolink_master_validate_config_against_device_info(&info, &config)`.
- `iolink_master_decode_min_cycle_time_100us(octet)` — MasterCycleTime octet →100µs.

## 5. ISDU and services

All service calls are non-blocking state machines: they return `OK` when complete,
`PENDING` while active, `INVALID_ARG`, or a domain error.

```c
int iolink_master_read_isdu (port, index, subindex, data, &len);   /* len: in=cap, out=actual */
int iolink_master_write_isdu(port, index, subindex, data, len);
int iolink_master_verify_isdu(port, index, subindex, expected, len);
```

Data Storage: `iolink_master_read_data_storage`, `..._write_data_storage`,
`..._restore_data_storage`, `..._verify_data_storage`. Block parameterization:
`iolink_master_begin_parameter_download` / `..._end_parameter_download` /
`..._begin_parameter_upload` / `..._end_parameter_upload` /
`..._store_parameter_download` / `..._write_parameter_block`. Status:
`iolink_master_read_detailed_device_status`. ISDU errors use
`IOLINK_MASTER_ISDU_ERR_*` (`BUFFER_TOO_SMALL`, `BUSY`, `DEVICE`, `INVALID_STATE`,
`VERIFY_FAILED`).

## 6. Events

Poll model: read `iolink_master_diagnostics_t.event_pending`, then:

```c
int iolink_master_read_event_code(port, &event_code);
int iolink_master_ack_event(port, &event_code);                 /* read == explicit ack */
int iolink_master_read_event_details(port, events, max_events, &out_count);
```

`read_event_details` writes at most `max_events` `iolink_master_event_t`
(`{qualifier, type, code}`) and returns `BUFFER_TOO_SMALL` rather than overrunning.

Dispatch model (optional): set `event_pending_handler` and/or `event_handler` in the
config (with `event_user` passed through). `event_pending_handler` fires on the
rising edge of the OD Event flag during a cyclic response; `event_handler` fires
once per decoded event. Both NULL keeps poll-only behavior.

## 7. Diagnostics

```c
int iolink_master_get_diagnostics(const iolink_master_port_t* port,
                                  iolink_master_diagnostics_t* diagnostics);
uint8_t iolink_master_get_device_status(const iolink_master_port_t* port);
```

`iolink_master_diagnostics_t` carries `od_status`, `event_pending`,
`rx_retry_count`, `checksum_errors`, `send_errors`, `response_timeouts`,
`cycle_slips`, last/max cycle jitter (100µs), `supply_voltage_mv`, `short_circuit`
(sampled from PHY hooks when present), `link_quality_percent`, `last_service_result`,
`last_event_count`/`last_event_code`, and `last_isdu_error`.

## 8. SIO DI/DQ

```c
int iolink_master_set_dq(iolink_master_port_t* port, bool level);       /* DQ mode */
int iolink_master_get_di(const iolink_master_port_t* port, bool* level); /* DI mode */
int iolink_master_set_port_mode(iolink_master_port_t* port, iolink_master_port_mode_t mode);
```

Wrong-mode / unsupported-PHY calls return `IOLINK_MASTER_SIO_ERR_WRONG_MODE` /
`..._UNSUPPORTED_PHY`. Switching to IO-Link mode restarts startup on the port.

## 9. Master Command helpers

`iolink_master_encode_master_command(read, channel, address)` composes a Master
Command octet; `iolink_master_mc_is_read` / `..._mc_channel` / `..._mc_address`
decode one. Channels: `IOLINK_MASTER_MC_CHANNEL_PROCESS` / `_PAGE` / `_DIAGNOSIS` /
`_ISDU`.

## 10. Multi-port controller

```c
int iolink_master_controller_init(controller, ports, port_count, phys, configs);
int iolink_master_controller_tick(controller, response_timeouts);      /* bool[] */
int iolink_master_controller_tick_events(controller, events);          /* tick_event[] */
int iolink_master_controller_tick_at(controller, now_100us);
int iolink_master_controller_get_port_count(controller, &out_count);
int iolink_master_controller_get_port(controller, index, &out_port);
int iolink_master_controller_get_next_tick_time(controller, now_100us, &out_next_100us);
```

Same lifetime contract as `iolink_master_init`: the `phys` array (and the PHYs it
points to) and the `ports` array must outlive the controller. A failing port
returns the first negative result without corrupting its siblings.

## Minimal example

Drives one port from startup into OPERATE and reads back process data. Compiles
against the real API (mirrors `examples/master_loopback_demo.c`; the PHY here is a
trivial stub — a real PHY talks to a transceiver, see [`PORTING.md`](PORTING.md)).

```c
#include <stdio.h>
#include "iolinki/phy.h"
#include "iolinki_master/master.h"

/* A real PHY drives a transceiver; these stubs just satisfy the contract. */
static int   phy_init(void* u)                           { (void)u; return 0; }
static void  phy_set_mode(void* u, iolink_phy_mode_t m)  { (void)u; (void)m; }
static void  phy_set_baud(void* u, iolink_baudrate_t b)  { (void)u; (void)b; }
static int   phy_send(void* u, const uint8_t* d, size_t n){ (void)u; (void)d; return (int)n; }
static int   phy_recv(void* u, uint8_t* b)               { (void)u; (void)b; return 0; }

static const iolink_phy_api_t phy = {
    .init = phy_init, .set_mode = phy_set_mode,
    .set_baudrate = phy_set_baud, .send = phy_send, .recv_byte = phy_recv,
};

int main(void)
{
    iolink_master_port_t port;              /* caller-owned, no heap */
    iolink_master_config_t config = {
        .port_mode  = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1,
        .baudrate   = IOLINK_BAUDRATE_COM3,
        .min_cycle_time = 20U,
        .pd_in_len  = 1U,
        .pd_out_len = 1U,
        .auto_baudrate = false,
    };
    uint8_t pd_out[1] = { 0x11U };
    uint8_t pd_in[1]  = { 0U };
    uint8_t pd_in_len = sizeof(pd_in);

    if (iolink_master_init(&port, &phy, &config) != IOLINK_MASTER_STATUS_OK) {
        return 1;
    }
    if (iolink_master_set_pd_out(&port, pd_out, sizeof(pd_out)) != IOLINK_MASTER_STATUS_OK) {
        return 1;
    }

    /* Drive the port: process() sends, poll_rx() decodes responses. A real
     * integration paces these from a 100us timer via iolink_master_tick_at(). */
    iolink_master_process(&port);
    (void)iolink_master_poll_rx(&port);

    if (iolink_master_get_pd_in(&port, pd_in, sizeof(pd_in), &pd_in_len)
            == IOLINK_MASTER_STATUS_OK) {
        printf("PD in: 0x%02X\n", pd_in[0]);
    }
    return 0;
}
```

Build it against the master library (which links the sibling `iolinki` frame/CRC
helpers) — see [`CONTRIBUTING.md`](CONTRIBUTING.md) and the runnable
`examples/master_loopback_demo.c` / `examples/master_4port_controller_demo.c`.
