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
