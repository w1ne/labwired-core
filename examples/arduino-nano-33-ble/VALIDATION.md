# Arduino Nano 33 BLE Validation

Validation date: 2026-08-11

## Scope

This board reuses the **silicon-verified nRF52840** chip model. Carrier work is
the Nano form-factor pinout + LED `board_io`. No separate SWD capture of this
PCB is claimed; silicon evidence rides the nRF52840 / Seeed XIAO capture.

## Simulator Evidence

```bash
# Manifest + GPIO LED pins + SPIM0 EasyDMA (offline)
cargo test -p labwired-core nano33ble -- --nocapture

# Shared nRF52840 smoke firmware against the product system
cargo build -p firmware-nrf52840-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-33-ble/uart-gpio-spi-smoke.yaml \
  --output-dir out/arduino-nano-33-ble/uart-gpio-spi-smoke \
  --no-uart-stdout
```

Expected offline tests:

| Test | What it proves |
|------|----------------|
| `nano33ble_manifest_builds_with_uart_gpio_and_board_leds` | System YAML loads; UART/GPIO/I2C/SPI present; all five LED board_io ids |
| `nano33ble_gpio_task_registers_drive_led_pins` | OUTSET/OUTCLR on P0.24, P0.13, and P1.09 (gpio1 remap window) |
| `nano33ble_spim0_start_sets_end_event_and_amount` | SPIM0 EasyDMA end events on the Nano product system |

UART smoke artifact must contain `NRF52840_SMOKE_OK` when the demo firmware is built.

## Hardware

No dedicated Nano 33 BLE bench capture is on file. USB VID:PID for the Arduino
board is `0x2341:0x005a` / `0x805a` (PlatformIO `nano33ble.json`). A future
SWD capture would re-confirm the same nRF52840 register set already covered by
the 2026-06-17 XIAO sweep.

## Coverage

| Area | Evidence |
|------|----------|
| Board manifest | `nano33ble_manifest_*` |
| GPIO LEDs | `nano33ble_gpio_task_registers_drive_led_pins` |
| SPIM0 | `nano33ble_spim0_*` |
| Chip silicon | Rides `nrf52840` full register sweep (see `docs/boards/nrf52840.md`) |
| Product catalog | `packages/board-config` BOARDS + manufacturer ABX00030 |
