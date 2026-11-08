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
| L2 blink | **pass** | WS2812 waveform on the GPIO18 pad via RMT ch0 (see below) |
| L3 I2C INA219 | skip | no S2 I2C model: `LW_L3_FAIL nack` |
| L4 SPI MAX31855 | skip | no S2 GP-SPI model (fault @ 0x3F424030) |
| L5 ADC | skip | no S2 SAR ADC model |
| L6 PWM | skip | prints `LW_L6_OK` but no LEDC model drives a pad, so not claimed |
| L7 micros | **pass** | 80 MHz SYSTIMER (`SYSTIMER_LL_TICKS_PER_US` = 80) |
| L8 CAN | skip | no S2 TWAI model: `LW_L8_FAIL no_can` |

Gate test: `cargo test -p labwired-core --test esp32s2_arduino_boot`, with
a negative control (erased partition table → no marker).

## L2 — RMT (added later the same day)

`LED_BUILTIN` on this board is the WS2812 on GPIO18, so `digitalWrite`
becomes `rgbLedWrite`: ESP-IDF's RMT TX driver writes 24 symbols into
RMTMEM, starts channel 0 and waits for the TX_END interrupt. The S2 RMT is
the classic ESP32 design with 4 channels and moved offsets
(`RmtLayout::ESP32S2`, type `esp32s2_rmt`); its channel outputs are a wire
the GPIO matrix carries to whichever pad `FUNCn_OUT_SEL` selects (signal 87).

Re-run of the row with `--watch-gpio gpio:18`
([`arduino-matrix-esp32s2-l2-rmt-2026-11-08.log`](arduino-matrix-esp32s2-l2-rmt-2026-11-08.log)):
L0, L1, L2, L7 pass. The L2 oracle needs `LW_L2_OK`, ≥ 48 pad edges on
GPIO18 and ≥ 1 RMT TX_START. The run had 96 edges and 2 TX_STARTs
([`result.json`](arduino-matrix-esp32s2-l2-result-2026-11-08.json.gz), gzip).
Decoded ([`…-ws2812-decode-2026-11-08.txt`](arduino-matrix-esp32s2-l2-ws2812-decode-2026-11-08.txt)):

| frame | GRB | meaning | T0H / T1H / bit period |
|---|---|---|---|
| 0 | `0x404040` | `digitalWrite(HIGH)` = `rgbLedWrite(64, 64, 64)` (`RGB_BRIGHTNESS`) | 400 / 800 / 1200 ns |
| 1 | `0x000000` | `digitalWrite(LOW)` = off | 400 / — / 1200 ns |

Gate test: `cargo test -p labwired-core --test esp32s2_arduino_rmt` decodes
the same two frames from the logic analyzer on the pad. Without the bus pass
that binds the RMT wire to the GPIO matrix, it sees 0 edges and fails
(mutation-checked). Before this change the cell faulted at `0x3F416080`
(RMT_APB_CONF). RMT RX, carrier, continuous and loop modes are not modelled.

## Vendor status (live, 2026-11-08)

- DigiKey ESP32-S2-MINI-2-N4 (16688710): **Part Status: Active**, in stock.
- Espressif longevity commitment lists ESP32-S2 (12 years from 2020-01-01);
  see [`vendor-status.md`](vendor-status.md) (checked 2026-11-05).
