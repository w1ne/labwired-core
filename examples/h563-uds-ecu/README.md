# STM32H563 FDCAN UDS ECU

This example proves the STM32H563 FDCAN model with a small UDS ECU firmware.
The firmware links UDSLib from `UDSLIB_DIR`, enables FDCAN, waits for the
`can-diagnostic-tester` external device to send ReadDataByIdentifier DID
`0xF190`, and validates the CAN-FD ISO-TP positive response.

Build:

```bash
UDSLIB_DIR=/tmp/udslib-labwired-scope make -C examples/h563-uds-ecu/firmware
```

Smoke test:

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/h563-uds-ecu/uds-smoke.yaml \
  --output-dir out/h563-uds-ecu \
  --no-uart-stdout
```

## Two-node headless gate

`twonode-smoke.yaml` proves the multi-node path end-to-end: two virtual STM32H563
ECU nodes are wired to a shared virtual CAN bus and each runs the same pre-built
`h563_uds_ecu.elf` firmware. Both nodes independently receive the UDS
`ReadDataByIdentifier 0xF190` request injected by their `can-diagnostic-tester`,
respond over FDCAN, and write the result marker `0x62F190A5` to `0x20010000`.
The `labwired test` runner asserts both markers after 200 000 simulation steps.

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/h563-uds-ecu/twonode-smoke.yaml \
  --output-dir out/twonode \
  --no-uart-stdout
```

Exit 0 means both `ecu_a` and `ecu_b` reached the marker — the CI-runnable
artifact for Sub-project B.
