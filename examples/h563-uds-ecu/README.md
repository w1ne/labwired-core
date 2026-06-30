# STM32H563 FDCAN UDS ECU

Proves the STM32H563 **FDCAN** model (CAN-FD) with a small UDS ECU firmware.
The firmware links UDSLib from `UDSLIB_DIR` and shares its UDS application logic
with the F103 example via `examples/common/uds_ecu_app.c`. On boot it prints
`H563-UDS-ECU` and `ECU_READY`, enables FDCAN1, and processes incoming ISO-TP
frames through the full UDSLib stack.

This is a **CLI regression example**, not a bundled web playground demo.

## UDSLib version pin (v2.0.0)

The firmware compiles UDSLib **from source** out of `UDSLIB_DIR` and is pinned to
**UDSLib v2.0.0**. The build asserts `UDS_VERSION_STR "2.0.0"`; a mismatched
checkout fails `check-udslib` loudly. Fetch a matching tree, or point
`UDSLIB_DIR` at an existing v2.0.0 checkout:

```bash
make -C examples/h563-uds-ecu/firmware fetch-udslib       # clones tag v2.0.0
```

This example exercises the v2.0.0 hardening:

- **>512-byte response** — DID `0xF1A0` returns a 600-byte block (603-byte
  response) reassembled over multi-frame ISO-TP in CAN-FD mode.
- **ECUReset (0x11) post-TX deferral** — `fn_tx_complete` polls FDCAN `TXBRP`
  bit 0 and `reset_tx_wait_ms` bounds the wait, so the SCB `SYSRESETREQ` reset
  cannot fire until `51 01` has arbitrated onto the wire (udslib #88).
- **Configurable S3** — `uds_config_t.s3_ms` set explicitly (4000 ms).

Concurrency/mutex hardening is **not applicable**: this is a single-context
polled super-loop, so there is no cross-context lock to exercise.

Build (needs an `arm-none-eabi` toolchain and UDSLib v2.0.0 checked out):

```bash
make -C examples/h563-uds-ecu/firmware UDSLIB_DIR=/path/to/udslib-v2.0.0
```

## Scenarios

- `uds-session-smoke.yaml` — the full everyday diagnostic session driven by the
  scripted tester over FDCAN1 (request_id 0x7E0, reply_id 0x7E8):
  ReadDataByIdentifier (0x22) incl. the >512-byte `0xF1A0` multi-frame block,
  session switch (0x10) and TesterPresent (0x3E),
  WriteDataByIdentifier (0x2E, extended-session gated), ReadDTCInformation /
  ClearDTC (0x19/0x14), RoutineControl (0x31, extended-gated),
  InputOutputControl (0x2F), CommunicationControl (0x28), and ECUReset (0x11).
  The default-session write is asserted to return `7F 2E 31`, then succeeds after
  `10 03`. `ECU_READY` appears twice because the 0x11 reset triggers a real
  AIRCR reboot.

- `uds-smoke.yaml` — minimal sanity check: banner + ECU_READY + tester result
  done (VIN read only via the same session system.yaml).

These smokes drive a locally-built ELF and are **not** clean-checkout CI gates.
Build first:

    make -C firmware UDSLIB_DIR=$HOME/projects/udslib

Run the full-session smoke:

    cargo run -p labwired-cli --bin labwired -- test --script examples/h563-uds-ecu/uds-session-smoke.yaml

The always-on regression for the scriptable tester's framing (single/multi-frame
requests and responses, wildcard and NRC matching) and the FDCAN model lives in
the `uds_tester_*` tests in `crates/core/src/bus/mod.rs`, which run in CI.
