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
