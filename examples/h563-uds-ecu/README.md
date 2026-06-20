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

`twonode-coexistence-smoke.yaml` exercises the **multi-node headless test runner
and per-node `memory_value` assertions**. It proves both ECUs **boot and respond
to their own injector concurrently on a shared virtual CAN bus (co-existence)**.

Both nodes run the same `h563_uds_ecu.elf` firmware. Each node receives a UDS
`ReadDataByIdentifier 0xF190` request from its own `can-diagnostic-tester`,
responds over FDCAN, and writes the result marker `0x62F190A5` to `0x20010000`.
The runner asserts both markers after 200 000 simulation steps.

**This does NOT prove cross-node A→B delivery.** Each node answers its own local
injector; neither node depends on the other, so the assertions would pass even if
the `can_bus` interconnect were removed. For the authoritative cross-node delivery
proof see `crates/core/tests/fdcan_firmware_crossing.rs`, which uses a bare,
non-transmitting observer FDCAN that can only see `0x7E8` if the frame crossed the
bus from the ECU.

A true tester↔ECU cross-node gate (a real udslib tester firmware driving requests
over the bus and asserting the ECU's response) is Sub-project B, not yet
implemented here.

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/h563-uds-ecu/twonode-coexistence-smoke.yaml \
  --output-dir out/twonode \
  --no-uart-stdout
```

Exit 0 means both `ecu_a` and `ecu_b` reached the marker — confirming the
multi-node runner and co-existence on a shared bus.
