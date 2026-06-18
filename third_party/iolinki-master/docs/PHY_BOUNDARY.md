# PHY Boundary

The master core is board-agnostic. Board support must live outside
`src/master_*.c` and enter the core only through `iolink_phy_api_t`,
`iolink_master_config_t`, and explicit time inputs such as
`iolink_master_tick_at()` or `iolink_master_controller_tick_at()`.

## Ownership

- The caller owns hardware timers and supplies monotonic 100us timestamps.
- The controller computes the next due timestamp with
  `iolink_master_controller_get_next_tick_time()`.
- The PHY adapter owns transceiver registers, UART/USART setup, C/Q line
  direction, fault pins, and board-specific interrupt wiring.
- The protocol core owns frame encoding/decoding, startup state, retry policy,
  service sequencing, process data, and diagnostics.

## Minimum PHY Operations

### IO-Link Mode

Required for strict hardware validation:

- `send`: transmit a complete encoded frame buffer.
- `recv_byte`: non-blocking byte receive from UART/USART.
- `set_mode_checked` in `iolink_master_config_t`: switch the transceiver into
  SDCI mode and report adapter failures.
- `set_baudrate_checked` in `iolink_master_config_t`: apply COM1, COM2, or
  COM3 during fixed or auto-baud startup and report adapter failures.
- `flush_rx` in `iolink_master_config_t`: clear the adapter/UART receive FIFO
  before startup begins and before startup retries or baudrate changes.
- `prepare_tx` and `prepare_rx` in `iolink_master_config_t`: switch the
  half-duplex adapter direction before and after each core-driven frame send.
- `wake_up` in `iolink_master_config_t`: generate the master wake-up pulse.

Recommended:

- `set_mode` and `set_baudrate`: legacy permissive fallbacks for unit tests or
  partial fakes. Real adapters should expose the checked config hooks above.
- `get_voltage_mv`: expose L+ diagnostics when the transceiver supports it.
- `is_short_circuit`: expose hard line faults when available.

### DI Mode

Required for strict hardware validation:

- `set_mode_checked`: switch the transceiver into SIO mode and report failures.
- `read_cq_line_checked` in `iolink_master_config_t`: read C/Q and report
  adapter failures.

Recommended:

- `read_cq_line`: legacy permissive fallback for existing tests/fakes.

Not required:

- UART receive/transmit callbacks.

### DQ Mode

Required:

- `set_cq_line`: drive C/Q high or low.
- `set_mode_checked`: switch the transceiver into SIO mode and report failures.

Not required:

- UART receive/transmit callbacks.

### Deactivated Mode

Required for strict hardware validation:

- `set_mode_checked`: switch the transceiver into inactive/high-impedance mode
  and report failures.

## Adapter Rules

- Do not include board headers from `src/master_*.c`.
- Do not sleep inside core calls. Schedule the next call using the public next
  due-time helpers.
- Do not hide UART framing errors. Return a negative value from `recv_byte`.
- Do not partially report successful sends. `send` must return the exact length
  or a negative/short result so the core can enter error handling.
- Real hardware adapters should pass `iolink_master_validate_phy_contract()`.
  `iolink_master_init()` remains permissive for unit tests and partial fake PHYs.
- Keep response timeout separate from cycle pacing when the adapter can support
  it. `response_timeout_100us` controls the deadline while `min_cycle_time`
  controls cycle spacing; a zero response timeout falls back to `min_cycle_time`.
- Flush stale adapter RX bytes explicitly. The core always clears its internal
  RX accumulator, and real adapters should implement `flush_rx` so stale UART
  bytes cannot bleed across startup attempts or baudrate changes.
- Keep half-duplex direction explicit. Core-driven frame sends call
  `prepare_tx`, then `send`, then `prepare_rx`; adapters that cannot switch
  direction must return a nonzero error so the core can stop instead of
  listening in the wrong state.
- Keep adapter fault policy explicit: line faults may be surfaced through PHY
  callbacks and public diagnostics, but must not mutate core state behind its
  back. `iolink_master_get_diagnostics()` samples `get_voltage_mv` and
  `is_short_circuit` when those hooks are present.
