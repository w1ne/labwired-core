# canmod-gps-sim — simulated CSS Electronics CANmod.gps

A GNSS/IMU-to-CAN module modelled in LabWired. The firmware synthesizes a
deterministic GNSS track and broadcasts CSS Electronics' real **canmod-gps.dbc**
message set (9 messages, classical CAN IDs 0x1–0x9) on the STM32F103 bxCAN
peripheral in internal loopback.

**MCU note:** the STM32F103 is a *compute stand-in* — CANmod.gps's real MCU is
undisclosed. The CAN frames on the wire are bit-accurate to the published DBC
regardless of the core that produces them.

## Run
```
cd firmware && make
cd .. && cargo run -q -p labwired-cli -- test --script canmod-smoke.yaml \
  --output-dir out --no-uart-stdout
```
`out/uart.log` shows decoded GNSS values plus every raw frame as hex.

## Determinism
Two runs produce a byte-identical `uart.log` (the simulator is fully
deterministic). This is what makes the module usable as a reliable CI gate.
