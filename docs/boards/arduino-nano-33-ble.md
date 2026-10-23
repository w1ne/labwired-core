# Arduino Nano 33 BLE

The **Arduino Nano 33 BLE** (ABX00030): an **nRF52840** (Cortex-M4F, 64 MHz), 1 MiB of flash at `0x0`, 256 KiB of RAM at `0x20000000`, a **4-channel RGB LED** (active low) on `P0.24`/`P0.16`/`P0.06`, the built-in D13 LED on `P0.13`, and `Serial1` on **UARTE0** (`D0/TX = P1.03`, `D1/RX = P1.10`). The board also carries an **LSM9DS1** IMU on TWI0 (`0x6B`/`0x1E`) and the nRF52840's USB CDC (`Serial`).

This is an **L1 (smoke)** target: boot + the UARTE console and the active-low RGB GPIO mapping are proven by a committed smoke fixture and an `io-smoke.yaml`. The nRF52840 chip is the shipped model (same silicon as the nRF52840-DK); the board adds the board-level pin map. The LSM9DS1 IMU and the USB CDC are **not modelled** — the smoke touches neither, and no silicon capture exists: **SIM-DERIVED**, no executing-fidelity differential of its own.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/nrf52840.yaml`](../../configs/chips/nrf52840.yaml) (shipped chip, shared with the nRF52840-DK) |
| Example system | [`configs/systems/arduino-nano-33-ble.yaml`](../../configs/systems/arduino-nano-33-ble.yaml) |
| Reference firmware | [`crates/firmware-arduino-nano-33-ble-smoke/`](../../crates/firmware-arduino-nano-33-ble-smoke/) |
| Example | [`examples/arduino-nano-33-ble/`](../../examples/arduino-nano-33-ble/) |
| Committed artifact | `tests/fixtures/arduino-nano-33-ble-smoke.elf` (`ARDUINO NANO 33 BLE SMOKE`) |
| Build target | `thumbv7em-none-eabihf` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + UARTE + GPIO); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Pin / base | State |
|--------|------|-------|
| Flash | `0x0000_0000`, 1 MiB | modelled — the smoke links and runs here |
| RAM | `0x2000_0000`, 256 KiB | modelled — data/stack |
| UARTE0 | `0x4000_2000` | modelled — EasyDMA TX/RX; `PSEL.TXD = P1.03`, `PSEL.RXD = P1.10` |
| RGB LED red / green / blue | `P0.24` / `P0.16` / `P0.06` | modelled GPIO; **active low** |
| Built-in LED (D13) | `P0.13` | modelled GPIO; active high |
| LSM9DS1 IMU | TWI0, `0x6B` / `0x1E` | **not modelled** (TWI0 controller is modelled; the sensor is not) |
| USB CDC (`Serial`) | nRF52840 USBD | **not modelled** — the console is UARTE0 (`Serial1`) |

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point
(the flash base, `0x0`) with SP seeded at the top of RAM. The nRF52840 boot
flow (bootloader/DFU) is not modelled.

## Validation

```bash
cd core
cargo build -p firmware-arduino-nano-33-ble-smoke --release --target thumbv7em-none-eabihf
./target/release/labwired test --script examples/arduino-nano-33-ble/io-smoke.yaml --no-uart-stdout
```

See [`examples/arduino-nano-33-ble/VALIDATION.md`](../../examples/arduino-nano-33-ble/VALIDATION.md) for the claims, runtime evidence and explicit non-claims.

## Sources

The Arduino ABX00030/ABX00069 schematics and the ArduinoCore-mbed
`variants/ARDUINO_NANO33BLE` pin map for the board wiring, and the nRF52840
Product Specification for the UARTE/GPIO register map — detailed in
[`examples/arduino-nano-33-ble/REQUIRED_DOCS.md`](../../examples/arduino-nano-33-ble/REQUIRED_DOCS.md).
