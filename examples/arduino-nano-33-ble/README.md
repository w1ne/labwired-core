# Arduino Nano 33 BLE — smoke example

Runs a bare-metal Cortex-M4F firmware on the Arduino Nano 33 BLE (nRF52840):
the firmware boots from flash at 0x0, prints over UARTE0, walks the RGB LED
through red/green/blue and reads the GPIO0 OUT register back to prove the
active-low channel mapping.

## Build

```bash
cd core
rustup target add thumbv7em-none-eabihf   # once
cargo build -p firmware-arduino-nano-33-ble-smoke --release --target thumbv7em-none-eabihf
```

## Run

```bash
cd core
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabihf/release/firmware-arduino-nano-33-ble-smoke \
  --system examples/arduino-nano-33-ble/system.yaml \
  --max-steps 400000
```

Expected console output (prefix):

```
ARDUINO NANO 33 BLE SMOKE
cpu=nrf52840 uart=uarte0
rgb=red out=00012040
rgb=green out=01002040
rgb=blue out=01012000
hb=0 out=01012000
...
```

The `out=` field is the GPIO0 OUT register read from the model. The RGB
channels are **active low**, so `rgb=red out=00012040` means bit 24 (red) is
cleared while bits 16 (green) and 6 (blue) stay set; bit 13 is the built-in
D13 LED (active high).

## Smoke test

```bash
cd core
./target/release/labwired test \
  --script examples/arduino-nano-33-ble/io-smoke.yaml \
  --output-dir out/arduino-nano-33-ble-smoke --no-uart-stdout
```

## What the board models

| Surface | State |
| :--- | :--- |
| nRF52840 CPU (`cortex-m4`) | modelled (shipped chip) |
| Flash (0x0, 1 MiB) / RAM (0x20000000, 256 KiB) | modelled |
| UARTE0 (0x40002000, EasyDMA) | modelled — the board's `Serial1` (D0/D1) |
| RGB LED (P0.24/P0.16/P0.06) | modelled GPIO — active low |
| Built-in D13 LED (P0.13) | modelled GPIO — active high |
| LSM9DS1 IMU (TWI0 0x6B/0x1E) | **not modelled** (the TWI0 controller is) |
| USB CDC (`Serial`) | **not modelled** |
| Silicon capture / executing-fidelity differential | **none** — SIM-DERIVED |

## Documents

- `REQUIRED_DOCS.md` — the sources this package is grounded on
- `EXTERNAL_COMPONENTS.md` — what the board carries beyond the modelled surface
- `VALIDATION.md` — claims, evidence, and explicit non-claims
