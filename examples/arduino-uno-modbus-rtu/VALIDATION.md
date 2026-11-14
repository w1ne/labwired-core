# Uno + MAX485 + Modbus RTU: validation runbook

Run from `core/`.

## What this proves

- The ModbusMaster library runs on the AVR core and polls two XY-MD02 slaves through a
  MAX485 whose DE and /RE are driven by a GPIO pin.
- USART0 RX works: UDR0, RXC0 and the RX-complete interrupt, with the bytes
  coming from peers on `usart0`.
- The frames on the bus decode with pymodbus, CRC included.
- Every frame example in the XY-MD02 manual is answered byte for byte (`manual_examples` in `rs485_modbus.rs`).
- A wrong CRC, a frame split by more than 3.5 character times and a request for
  another address get no answer.

## What it cannot prove

Line-level behaviour (termination, bias, reflections), parity, a second master,
and timing against a real transceiver. No bench capture exists.

## Smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-uno-modbus-rtu/io-smoke.yaml --no-uart-stdout
```

Pass criteria: exit 0, `PASS 10/10 checks`.

## Tests

```bash
cargo test --release -p labwired-core --test misc rs485_modbus::
python -m pip install pymodbus pytest
python -m pytest scripts/ci/test_modbus_pymodbus.py
```

## Rebuild the fixture

See the README.
