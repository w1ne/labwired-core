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

## Mixed-signal test

```bash
cd core
cargo test -p labwired-core --test analog cosim_nrf52840_family_analog
```

The test loads the same `analog-ntc-diode-system.yaml` deck against this board,
the nRF52840-DK and the Seeed XIAO nRF52840 Sense. It compares each solved node
with an independent closed form, then drives the SAADC register sequence and
checks the EasyDMA buffer against the 600 mV internal reference at gain 1/6.

## Analog deck

`analog-ntc-diode-system.yaml` declares two generic analog models. Neither
names this board, the SAADC or a channel: each publishes to
`board.analog.p0.04_volts` or `board.analog.p0.05_volts`, and the chip
descriptor's `analog_pins:` map selects AIN2 and AIN3. The same file runs on
any nRF52840 profile with the same chip descriptor.

## What the board models

| Surface | State |
| :--- | :--- |
| nRF52840 CPU (`cortex-m4`) | modelled (shipped chip) |
| Flash (0x0, 1 MiB) / RAM (0x20000000, 256 KiB) | modelled |
| UARTE0 (0x40002000, EasyDMA) | modelled — the board's `Serial1` (D0/D1) |
| RGB LED (P0.24/P0.16/P0.06) | modelled GPIO — active low |
| Built-in D13 LED (P0.13) | modelled GPIO — active high |
| SAADC AIN0..AIN7 (P0.02/P0.03/P0.04/P0.05/P0.28/P0.29/P0.30/P0.31) | modelled — `analog-ntc-diode-system.yaml` solves a Beta NTC divider on A0 (P0.04/AIN2) and a Shockley diode load line on A1 (P0.05/AIN3) into the EasyDMA result buffer |
| LSM9DS1 IMU (TWI0 0x6B/0x1E) | **not modelled** (the TWI0 controller is) |
| USB CDC (`Serial`) | not used by this smoke; modelled, see `examples/arduino-nano-33-ble-bme280/` |
| Silicon capture / executing-fidelity differential | **none** — SIM-DERIVED |

## Documents

- `REQUIRED_DOCS.md` — the sources this package is grounded on
- `EXTERNAL_COMPONENTS.md` — what the board carries beyond the modelled surface
- `VALIDATION.md` — claims, evidence, and explicit non-claims
