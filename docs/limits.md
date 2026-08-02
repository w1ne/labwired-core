# Run Limits

Every `labwired test` run is bounded by a `limits` block. `max_steps` is
required; everything else is optional and defaults to unbounded. Unknown keys
are rejected, so the list below is exhaustive.

```yaml
schema_version: "1.0"
inputs:
  firmware: "firmware.elf"
limits:
  max_steps: 1000000
  wall_time_ms: 10000
  no_progress_steps: 1000
```

## Keys

| Key | Type | Stops the run when |
|-----|------|--------------------|
| `max_steps` | required | the step count is reached |
| `max_cycles` | optional | the CPU cycle count is reached (includes peripheral wait states) |
| `max_uart_bytes` | optional | captured UART output reaches this size |
| `wall_time_ms` | optional | this much real time has elapsed, regardless of progress |
| `no_progress_steps` | optional | the PC has not changed for this many steps |
| `max_vcd_bytes` | optional | the VCD trace reaches this size (interactive mode only) |

Cycles are not steps: a step that hits a wait-stated peripheral costs more than
one cycle, so `max_cycles` has to be set from a cycle budget, not scaled off
`max_steps`.

## Early stop on assertions

A run can finish as soon as its assertions hold, rather than burning the full
step budget:

| Key | Default | Effect |
|-----|---------|--------|
| `stop_when_assertions_pass` | `false` | stop once all runtime assertions pass |
| `stop_when_assertions_pass_settle_steps` | `100000` | keep executing this many steps past the first passing moment before accepting the result |
| `stop_when_assertions_pass_min_steps` | `0` | never early-stop before this many steps have run |

The settling window closes the print-then-crash hole: firmware that emits its
acceptance token and then faults breaks with the fault reason instead of
certifying as passed. Lower it only if you know the firmware cannot fault after
the token.

## Choosing values

- **CI**: always set `wall_time_ms`. A hung build costs more than a false
  timeout, and it is the only limit that bounds pathological cases you did not
  anticipate.
- **Infinite loops**: `no_progress_steps` catches a PC that stops moving.
  Raise it above the default if the firmware intentionally spins — a WFI idle
  loop or a busy-wait on a peripheral flag will otherwise trip it.
- **Verbose firmware**: `max_uart_bytes` bounds the capture buffer. Logging in
  a hot loop is the usual way a run exhausts memory.
- **VCD**: traces grow with instruction count and can reach hundreds of
  megabytes on a million-step run. Enable VCD per debugging session, and pair
  it with `max_vcd_bytes` or a reduced `max_steps`.

Limits are independent — the run stops at whichever trips first, so setting
both a step and a time bound is normal.
