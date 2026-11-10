# Arduino Nano Every + MCP9808

This copyable stock Arduino example reads the MCP9808 manufacturer ID,
device ID, and ambient temperature over `Wire`. The sensor is attached to
TWI0 at its default address (`0x18`); the temperature input defaults to 25 °C.
It prints the pass marker only when the IDs and measured temperature are in
range.

## Demand position

Arduino kits ranked third among kit types in the 2025 survey (37%). ATmega4809
was not a separate MCU response; the measured #2 MCU response was ATmega328
(30%). This is a focused Nano Every extension within the Arduino kit demand
signal, not a new ATmega328 pick. ATmega4809 and MCP9808 are both currently
In Production.

## Wiring

| MCP9808 breakout | Arduino Nano Every |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | D20 / SDA (PA2) |
| SCL | D21 / SCL (PA3) |

The firmware uses the stock Arduino `Wire` API. The system attaches the sensor
to the modeled TWI0 controller; pad muxing and SDA/SCL waveforms are outside
this profile. The Nano Every's USART3 console is captured directly and does
not model the SAMD11 USB bridge.

## Run

```sh
./examples/arduino-nano-every-mcp9808/firmware/build.sh
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-every-mcp9808/onboarding-smoke.yaml \
  --no-uart-stdout
```

The smoke's build logs and structured simulation result are committed under
[`evidence/`](evidence/). The live lifecycle and availability check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The MCP9808 model now follows Microchip's register pointers: TA `0x05`,
manufacturer ID `0x06`, device ID `0x07`, and resolution `0x08`. The smoke
does not exercise alert output or conversion-time gating. Temperature input is
positive-only. The MCU rail is idealized, and this local digital-twin run is
not a physical-board confirmation. The hosted verification catalog does not
list the Nano Every / ATmega4809, so this is local CLI smoke evidence rather
than a hosted `labwired_verify` verdict.
