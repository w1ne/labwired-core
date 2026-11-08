# Stock Arduino-ESP32 on the ESP32-S2 — matrix row, 2026-11-08

Profile: `configs/chips/esp32s2-arduino.yaml` (genuine ESP32-S2 mask ROM,
ESP-IDF startup, FreeRTOS, UART0 console). PlatformIO `espressif32` 8.0.0,
board `esp32-s2-saola-1` (generic ESP32-S2 variant; Serial = UART0, the same
console the ESP32-S2-DevKitM-1's USB-UART bridge carries).

`python3 validation/arduino-matrix/run_matrix.py --boards esp32s2`
(log: [`arduino-matrix-esp32s2-2026-11-08.log`](arduino-matrix-esp32s2-2026-11-08.log);
that run predates the narrowed descriptor, re-measured cell by cell after it):

| cell | result | evidence |
|---|---|---|
| L0 serial boot | **pass** | `LW_L0_OK` at 606k steps |
| L1 serial loop | **pass** | |
| L2 blink | skip | LED_BUILTIN = RGB pixel on GPIO18 via RMT; no S2 RMT model (fault @ 0x3F416080) |
| L3 I2C INA219 | skip | no S2 I2C model: `LW_L3_FAIL nack` |
| L4 SPI MAX31855 | skip | no S2 GP-SPI model (fault @ 0x3F424030) |
| L5 ADC | skip | no S2 SAR ADC model |
| L6 PWM | skip | prints `LW_L6_OK` but no LEDC model drives a pad, so not claimed |
| L7 micros | **pass** | 80 MHz SYSTIMER (`SYSTIMER_LL_TICKS_PER_US` = 80) |
| L8 CAN | skip | no S2 TWAI model: `LW_L8_FAIL no_can` |

Gate test: `cargo test -p labwired-core --test esp32s2_arduino_boot`, with
a negative control (erased partition table → no marker).

## Vendor status (live, 2026-11-08)

- DigiKey ESP32-S2-MINI-2-N4 (16688710): **Part Status: Active**, in stock.
- Espressif longevity commitment lists ESP32-S2 (12 years from 2020-01-01);
  see [`vendor-status.md`](vendor-status.md) (checked 2026-11-05).
