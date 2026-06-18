# Hardware Validation Matrix

Local CTests verify protocol behavior only. A hardware-tested master needs the
matrix below before claiming real-device support.

## Required Setup

- One IO-Link master PHY adapter wired through `iolink_phy_api_t`.
- `iolink_master_validate_phy_contract()` passing for the selected
  PHY/config pair before the run starts.
- Adapter config hooks for fallible hardware operations:
  `set_mode_checked`, `set_baudrate_checked`, `flush_rx`, `prepare_tx`,
  `prepare_rx`, `wake_up`, and `read_cq_line_checked` for DI-mode validation.
- One known sensor with cyclic PD input.
- One known actuator or output module with cyclic PD output.
- Capture path for UART frames or a logic analyzer on the PHY side.
- Monotonic timer source feeding `iolink_master_controller_tick_at()`.

## Matrix

| Area | Sensor | Actuator | Evidence |
| --- | --- | --- | --- |
| Adapter contract | Checked mode changes, baudrate changes, RX flush, wake pulse, UART direction | Checked mode changes, baudrate changes, RX flush, wake pulse, UART direction | Adapter logs plus captured pins/register state |
| Startup | Wake-up, baudrate, PREOPERATE, OPERATE | Wake-up, baudrate, PREOPERATE, OPERATE | Captured startup frames and final state |
| Cycle timing | Min cycle and response timeout respected for 10k cycles | Min cycle and response timeout respected for 10k cycles | Timing log with max jitter, slips, and timeout deadlines |
| PD input | PD valid and stable under nominal operation | Status input if available | Captured PD bytes and API readback |
| PD output | Not applicable unless sensor accepts output | Output command reflected by device | Captured master frame and device behavior |
| ISDU read | Vendor ID, Device ID, status objects | Vendor ID, Device ID, status objects | API result and captured ISDU frames |
| ISDU write | Application tag or safe writable object | Application tag or safe writable object | Write result and readback |
| Events | Trigger or simulate one event | Trigger or simulate one event | Event code/details and ack behavior |
| Data Storage | Backup/read object where supported | Backup/read/restore where supported | Readback and restore evidence |
| SIO | DI C/Q readback through checked hook | DQ C/Q output drive | Logic trace and public API result |
| Faults | Disconnect/CRC/no-response handling | Disconnect/CRC/no-response handling | Diagnostics counters, voltage/short flags, and recovery/error state |
| RX hygiene | RX flush before startup retries and baudrate changes | RX flush before startup retries and baudrate changes | Captured UART stream and adapter log |
| Soak | At least 8 hours cyclic read | At least 8 hours cyclic read/write | Error counters, protocol-local link quality, timing stats |

## Pass Criteria

- No unexpected error state during nominal startup and cyclic operation.
- Captured frames match the configured M-sequence type, PD sizes, OD size, and
  checksum expectations.
- Public diagnostics show bounded jitter, no unexplained checksum growth,
  hardware fault fields matching adapter evidence, and protocol-local link
  quality consistent with captured faults.
- Service APIs return `OK`, `PENDING`, or documented negative result codes only.
- Any hardware-specific behavior is isolated in the adapter, not in
  `src/master_*.c`.

## What This Does Not Prove

This matrix is not official IO-Link master conformance. Official conformance
testing remains a separate external validation step.
