# Validation — Arduino Nano 33 BLE

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the entry point (PC init) | smoke-tested | run below: `Initial PC: 0x0` (flash base) |
| nRF52840 Cortex-M4F executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| Flash (0x0, 1 MiB) + RAM (0x20000000, 256 KiB) load + execute | smoke-tested | firmware links into flash and runs with the stack in RAM |
| UARTE0 EasyDMA console (PSEL/BADRATE/TXD.PTR/MAXCNT) | smoke-tested | `ARDUINO NANO 33 BLE SMOKE` + heartbeat |
| Active-low RGB LED mapping (P0.24/P0.16/P0.06) | smoke-tested | `rgb=red out=00012040`, `rgb=green out=01002040`, `rgb=blue out=01012000` (GPIO0 OUT readback) |
| Built-in D13 LED (P0.13, active high) | smoke-tested | bit 13 set in every `out=` readback |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical UART streams (`diff` clean) |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| LSM9DS1 IMU | **not modelled** | the TWI0 controller is modelled; no sensor model is declared |
| USB CDC (`Serial`) | **not modelled** | the console is UARTE0 (`Serial1`) |
| Silicon capture / executing-fidelity differential | **none** | SIM-DERIVED |

## Commands and evidence (verified 2026-10-23)

```bash
# from core/
cargo build -p firmware-arduino-nano-33-ble-smoke --release --target thumbv7em-none-eabihf
#   Finished `release` profile — warning-free

./target/release/labwired test \
  --script examples/arduino-nano-33-ble/io-smoke.yaml \
  --output-dir out/arduino-nano-33-ble-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 2.05s

./target/release/labwired \
  --firmware target/thumbv7em-none-eabihf/release/firmware-arduino-nano-33-ble-smoke \
  --system configs/systems/arduino-nano-33-ble.yaml --max-steps 400000
#   Initial PC: 0x0
#   ARDUINO NANO 33 BLE SMOKE / cpu=nrf52840 uart=uarte0
#   rgb=red out=00012040 / rgb=green out=01002040 / rgb=blue out=01012000
#   hb=0 out=01012000 …

# Determinism: two runs, identical streams
… | grep -E '^(ARDUINO|cpu=|rgb=|hb=)' > run1
… > run2 ; diff run1 run2   # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabihf/release/firmware-arduino-nano-33-ble-smoke \
  --system configs/systems/arduino-nano-33-ble.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/arduino-nano-33-ble
#   simulator exit 0 · Instructions executed: 200000 · Unsupported: 0

cargo test -p labwired-core --test firmware_survival test_arduino_nano_33_ble_smoke_survival
```

## Notes

- The `out=` readbacks are the GPIO0 OUT register, not assumed values: the
  red channel's bit is cleared while green/blue stay set (active low), which
  is exactly the board's wiring.
- UARTE0 is the board's `Serial1` on D0/D1; the USB CDC `Serial` is not
  modelled and is not used by this firmware.
- The board's LSM9DS1 IMU is not modelled; the smoke never touches TWI0.
