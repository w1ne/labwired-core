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

## UDSLib version pin (v2.0.0)

The firmware compiles UDSLib **from source** out of `UDSLIB_DIR` and is pinned to
**UDSLib v2.0.0**. The build asserts `UDS_VERSION_STR "2.0.0"` in
`uds_version.h`; a stale or mismatched checkout fails the `check-udslib` target
loudly instead of compiling against the wrong API (v2.0.0 renamed
`ctx->active_session` to `ctx->session.active`, which silently broke the old
unpinned build). Fetch a matching tree, or point `UDSLIB_DIR` at an existing
v2.0.0 checkout:

```bash
make -C examples/f103-uds-ecu/firmware fetch-udslib      # clones tag v2.0.0
# or: UDSLIB_DIR=/path/to/udslib-v2.0.0 make -C examples/f103-uds-ecu/firmware
```

This example exercises the v2.0.0 hardening:

- **>512-byte response** — DID `0xF1A0` returns a 600-byte calibration block
  (`62 F1 A0` + 600 = 603 bytes). The `tx_buffer` and ISO-TP SDU buffer are
  sized > 512 so the response is built then streamed as a multi-frame ISO-TP
  reply (the snapshot-vs-send-under-lock TX split).
- **ECUReset (0x11) post-TX deferral** — `fn_tx_complete` polls the bxCAN
  `TSR.TME0` bit and `reset_tx_wait_ms` bounds the wait, so `NVIC_SystemReset`
  cannot fire until `51 01` has left the mailbox (udslib #88).
- **Configurable S3** — `uds_config_t.s3_ms` is set explicitly (4000 ms) instead
  of the 5000 ms default.

Concurrency/mutex hardening is **not applicable** here: this is a single-context
polled super-loop (one thread drives both `uds_input_sdu` via the RX poll and
`uds_process`), so there is no cross-context lock to exercise.

Build (needs an `arm-none-eabi` toolchain):

```bash
UDSLIB_DIR=/path/to/udslib-v2.0.0 make -C examples/f103-uds-ecu/firmware
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
  scripted tester: ReadDataByIdentifier (0x22) incl. the >512-byte `0xF1A0`
  multi-frame block, session switch (0x10) and
  TesterPresent (0x3E), WriteDataByIdentifier (0x2E, extended-session gated),
  ReadDTCInformation/ClearDTC (0x19/0x14), RoutineControl (0x31, extended-gated),
  InputOutputControl (0x2F), CommunicationControl (0x28), and ECUReset (0x11).
  The default-session write is asserted to return `7F 2E 31`, then succeeds after
  `10 03`.

These smokes drive a locally-built ELF and are **not** clean-checkout CI gates.
Build first:

    make -C firmware UDSLIB_DIR=$HOME/projects/udslib   # must be UDSLib v2.0.0

Run:

    cargo run -p labwired-cli --bin labwired -- test --script examples/f103-uds-ecu/uds-session-smoke.yaml

The always-on regression for the scriptable tester's framing (single/multi-frame
requests and responses, wildcard and NRC matching) lives in the
`uds_tester_*` tests in `crates/core/src/bus/mod.rs`, which run in CI.
