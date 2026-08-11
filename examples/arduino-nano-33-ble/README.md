# Arduino Nano 33 BLE

Arduino Nano form-factor board with Nordic **nRF52840** (Cortex-M4F, BLE, USB).
LabWired reuses the generic `nrf52840` chip descriptor and adds board-level
wiring for the onboard LEDs plus Playground Arduino pin aliases.

| | |
|--|--|
| Product id | `arduino-nano-33-ble` |
| MPN | ABX00030 |
| Chip yaml | `configs/chips/nrf52840.yaml` |
| System yaml | `configs/systems/arduino-nano-33-ble.yaml` |
| PlatformIO | `nordicnrf52` / `nano33ble` / `arduino` |
| Firmware demo | `crates/firmware-nrf52840-demo/` |

## Modeled board I/O

| Signal | nRF pin | LabWired binding | Notes |
|--------|---------|------------------|-------|
| LED_BUILTIN (D13) | P0.13 | `led_builtin` | Active-high yellow user LED |
| LEDR | P0.24 | `led_red` | Active-low |
| LEDG | P0.16 | `led_green` | Active-low |
| LEDB | P0.06 | `led_blue` | Active-low |
| LED_PWR | P1.09 | `led_pwr` | Active-high power LED |

Sources: Arduino mbed `pins_arduino.h` / `variant.cpp` for `ARDUINO_NANO33BLE`.

## Stock bus pins (Arduino defaults)

| Bus | Role | Arduino pin | nRF pin |
|-----|------|-------------|---------|
| Wire | SDA | A4 | P0.31 |
| Wire | SCL | A5 | P0.02 |
| SPI | MOSI | D11 | P1.01 |
| SPI | MISO | D12 | P1.08 |
| SPI | SCK | D13 | P0.13 |
| SPI | SS | D10 | P1.02 |
| Serial1 | TX | D0 | P1.03 |
| Serial1 | RX | D1 | P1.10 |

Playground also accepts Arduino labels `D0`…`D13`, `A0`…`A7`, `LEDR`/`LEDG`/`LEDB`
as aliases of the Nordic names.

## Build / run

```bash
# Offline gates
cargo test -p labwired-core nano33ble -- --nocapture

# Shared nRF52840 smoke firmware against this system
cargo build -p firmware-nrf52840-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-33-ble/uart-gpio-spi-smoke.yaml \
  --output-dir out/arduino-nano-33-ble/uart-gpio-spi-smoke \
  --no-uart-stdout
```

Hosted compile uses PlatformIO `nano33ble`. Any nRF52840 ELF can also run via:

```bash
labwired run \
  --system configs/systems/arduino-nano-33-ble.yaml \
  --firmware <path-or-sha256-ref>
```

## Fidelity scope

- **Chip**: full nRF52840 peripheral estate (silicon-verified on Seeed XIAO; see
  `docs/boards/nrf52840.md`).
- **Carrier**: LEDs in `board_io` + Arduino pin aliases in board-config.
- **Not modeled here**: BLE air interface end-to-end, USB device stack detail,
  **Sense** onboard IMU/mic/APDS (separate product).

See [VALIDATION.md](VALIDATION.md) and [EXTERNAL_COMPONENTS.md](EXTERNAL_COMPONENTS.md).
