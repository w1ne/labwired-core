# Arduino Nano 33 BLE

Arduino Nano form-factor **nRF52840** board (ABX00030) with USB, RGB + user LEDs,
and classic Nano headers.

## Status

| Aspect | Status |
|--------|--------|
| Product id | `arduino-nano-33-ble` |
| Chip yaml | `configs/chips/nrf52840.yaml` |
| System yaml | `configs/systems/arduino-nano-33-ble.yaml` |
| Example | `examples/arduino-nano-33-ble/` |
| Firmware | `crates/firmware-nrf52840-demo/` (shared die smoke) |
| PlatformIO | `platform: nordicnrf52`, `board: nano33ble`, `framework: arduino` |
| Form factor | Arduino Nano (45 × 18 mm) |
| Tier | sim-validated (carrier + offline gates; silicon rides nRF52840) |

## Modeled board I/O

| Signal | nRF pin | board_io id | Polarity |
|--------|---------|-------------|----------|
| LED_BUILTIN | P0.13 | `led_builtin` | active-high |
| LEDR | P0.24 | `led_red` | active-low |
| LEDG | P0.16 | `led_green` | active-low |
| LEDB | P0.06 | `led_blue` | active-low |
| LED_PWR | P1.09 | `led_pwr` | active-high |

## Stock buses (Arduino defaults)

| Bus | Pins |
|-----|------|
| Wire | SDA `P0.31` (A4), SCL `P0.02` (A5) |
| SPI | SCK `P0.13` (D13), MISO `P1.08` (D12), MOSI `P1.01` (D11), CS `P1.02` (D10) |
| Serial1 | TX `P1.03` (D0), RX `P1.10` (D1) |

## Offline tests

```bash
cargo test -p labwired-core nano33ble -- --nocapture
```

| Test | Coverage |
|------|----------|
| `nano33ble_manifest_builds_with_uart_gpio_and_board_leds` | Manifest + board_io |
| `nano33ble_gpio_task_registers_drive_led_pins` | GPIO0/1 LED pins |
| `nano33ble_spim0_start_sets_end_event_and_amount` | SPIM0 EasyDMA |

## Silicon

Same die as [nRF52840](nrf52840.md) / [XIAO nRF52840 Sense](seeed-xiao-nrf52840-sense.md).
Full register sweep was captured on XIAO (2026-06-17). This board entry does
not claim a separate Nano PCB capture.

## Playground

Board id `arduino-nano-33-ble` is in the picker. Pin bank accepts Nordic
`P0.xx`/`P1.xx` and Arduino `D*`/`A*`/`LEDR`/`LEDG`/`LEDB` aliases.

## USB + BLE (chip-level)

| Block | Chip id | Status |
|-------|---------|--------|
| USBD | `usbd` | Modelled on nRF52840 (edge cases may differ) |
| RADIO | `radio` | Modelled; SoftDevice/blobs are firmware responsibility |
| Antenna / USB path | manufacturer electrical | `usb_serial`, `antenna`, `crystal` on ABX00030 |

## KiCad

`Module:Arduino_Nano` — 30 dual-row pads in stock KiCad pad order (D1…D12 left,
VIN…D13 right). Export nets use Arduino silkscreen names.

## Related

- [Arduino Nano 33 BLE Sense](arduino-nano-33-ble-sense.md) — onboard Wire1 suite
- [nRF52840](nrf52840.md) — die fidelity
