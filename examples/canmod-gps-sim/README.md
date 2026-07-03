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

## Fidelity gate
`canmod-gps.dbc` is CSS Electronics' own published database (from their
[api-examples](https://github.com/CSS-Electronics/api-examples) repository).
`decode_check.py` feeds the sim's on-wire frames back through **cantools** — the
same decoder a real CANmod.gps user runs — and asserts every signal resolves to
the intended physical value against that DBC:
```
python3 decode_check.py out/uart.log
```
CI runs this on every push, so "bit-accurate to the published DBC" is proven by a
scorer, not asserted. (CSS Electronics is credited as the DBC's author; this
example implies no affiliation.)

## Determinism
Two runs produce a byte-identical `uart.log` (the simulator is fully
deterministic). This is what makes the module usable as a reliable CI gate.
