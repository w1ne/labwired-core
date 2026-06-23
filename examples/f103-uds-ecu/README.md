# STM32F103 bxCAN UDS ECU

Proves the STM32F103 **bxCAN** model (classical CAN) with a small UDS ECU
firmware that links UDSLib from `UDSLIB_DIR`. The firmware enables bxCAN
internal loopback and, on that single looped-back node, plays both the tester
and the ECU: it injects the exact multi-frame CAN sequence captured on a real
PCAN bus (udslib issue #29, "FF first frame receive error") and checks that the
real UDSLib ISO-TP stack reassembles it and answers.

```
tester -> ECU (0x111): 10 0B 27 01 5A 11 22 33   FirstFrame  (FF_DL = 11)
ECU -> tester (0x222): 30 08 00 ...              FlowControl CTS
tester -> ECU (0x111): 21 44 55 66 77 88 ..      ConsecutiveFrame SN=1
ECU -> tester (0x222): 06 67 01 DE AD BE EF      SecurityAccess seed response
```

This is a **CLI regression example**, not a bundled web playground demo.

Build (needs an `arm-none-eabi` toolchain):

```bash
UDSLIB_DIR=/path/to/udslib make -C examples/f103-uds-ecu/firmware
```

Smoke test:

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/f103-uds-ecu/uds-smoke.yaml \
  --output-dir out/f103-uds-ecu
```

## Scenarios

- `uds-smoke.yaml` — multi-frame SecurityAccess (0x27) seed handshake.
- `uds-reset-smoke.yaml` — ECUReset (0x11): `51 01` lands before the real reboot.
- `uds-session-smoke.yaml` — the full everyday diagnostic session driven by the
  scripted tester: ReadDataByIdentifier (0x22), session switch (0x10) and
  TesterPresent (0x3E), WriteDataByIdentifier (0x2E, extended-session gated),
  ReadDTCInformation/ClearDTC (0x19/0x14), RoutineControl (0x31, extended-gated),
  InputOutputControl (0x2F), CommunicationControl (0x28), and ECUReset (0x11).
  The default-session write is asserted to return `7F 2E 31`, then succeeds after
  `10 03`.

These smokes drive a locally-built ELF and are **not** clean-checkout CI gates.
Build first:

    make -C firmware UDSLIB_DIR=$HOME/projects/udslib

Run:

    cargo run -p labwired-cli --bin labwired -- test --script examples/f103-uds-ecu/uds-session-smoke.yaml

The always-on regression for the scriptable tester's framing (single/multi-frame
requests and responses, wildcard and NRC matching) lives in the
`uds_tester_*` tests in `crates/core/src/bus/mod.rs`, which run in CI.
