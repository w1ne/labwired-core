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
| L3 I2C INA219 | **pass** | Wire reads INA219 CONFIG 0x399F and BUS 0x19CA via I2C0 (see below) |
| L4 SPI MAX31855 | skip | no S2 GP-SPI model (fault @ 0x3F424030) |
| L5 ADC | skip | no S2 SAR ADC model |
| L6 PWM | **pass** | 1 kHz, 50 % PWM on the GPIO2 pad via LEDC LS ch6 (see below) |
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

## L6 — LEDC (added later the same day)

The S2 LEDC is the S3's block: `soc/esp32s2/register/soc/ledc_reg.h` has the
same offsets, fields and `LEDC_DATE` reset (0x19040200). What differs is the
interrupt source (45), the matrix signals (`LEDC_LS_SIG_OUT0..7` = 79..86)
and RC_FAST (≈ 8.5 MHz). So type `esp32s2_ledc` is the S3 model with those
three values; its channel wire is published through `matrix_output_lines`,
the same bus pass the S2 RMT uses, and the S3's 73..80 binding declines it.

The sketch had to change for this board. `LED_BUILTIN` on the Saola-1 is the
WS2812's virtual pin 64 (`SOC_GPIO_PIN_COUNT + 18`), and stock `analogWrite`
returns without doing anything for pins ≥ `SOC_GPIO_PIN_COUNT`
(`esp32-hal-ledc.c`). The old cell printed `LW_L6_OK` without touching LEDC.
For `ARDUINO_ESP32S2_DEV` the sketch now uses GPIO2.

Re-run with `pwm_watch: gpio:2`, `pwm_min_edges: 4`
([`arduino-matrix-esp32s2-l6-ledc-2026-11-08.log`](arduino-matrix-esp32s2-l6-ledc-2026-11-08.log)):
L0, L1, L2, L6, L7 pass. Logic analyzer on the pad
([`result.json`](arduino-matrix-esp32s2-l6-result-2026-11-08.json.gz), gzip;
[`measurement`](arduino-matrix-esp32s2-l6-pwm-measure-2026-11-08.txt)):
58 edges, func `LEDC_LS_CH6` (Arduino allocates from the top channel), every
high phase 500.000 µs and every low phase 500.000 µs. That is 1 kHz at duty
0.5, which is `analogWrite(2, 128)` at the stock 1 kHz / 8-bit.

Gate test: `cargo test -p labwired-core --test esp32s2_arduino_ledc` checks
the same widths from the logic analyzer on the pad. Without the
`matrix_output_lines` publication it sees 0 edges and fails
(mutation-checked). Fades, the LEDC timer counter (`VALUE` reads 0) and
the overflow interrupts are not modelled.

## L3 — I2C0 (added later the same day)

The S2 I2C is the classic ESP32 design, not the S3's: `i2c_reg.h` puts 16
COMD slots at 0x58..0x94 (the S3 has 8) and `hal/esp32s2/.../i2c_ll.h` uses
the original opcodes (RSTART 0, WRITE 1, READ 2, STOP 3, END 4). The
I2CEXT0 matrix signals are the classic 29/30 too. So type `esp32s2_i2c` is
the classic `Esp32I2c` with the two values that differ: the CTR reset
(0xA0B: REF_ALWAYS_ON, ARBITRATION_EN, RX_FULL_ACK_LEVEL, SCL/SDA_FORCE_OUT)
and `ETS_I2C_EXT0_INTR_SOURCE` = 52. IDF writes and reads the FIFO through
PeriBus2 (0x6001_301C), which the profile's `peribus2` alias already maps.

The sketch had to change for this board. Its generic ESP32 branch calls
`Wire.begin(21, 22)`, and the S2 has no GPIO22. For `CONFIG_IDF_TARGET_ESP32S2`
it now uses the S2 variant's SDA 8 / SCL 9 (`variants/esp32s2/pins_arduino.h`).
The matrix system puts an INA219 at 0x40 on those pads.

Re-run of the row
([`arduino-matrix-esp32s2-l3-i2c-2026-11-08.log`](arduino-matrix-esp32s2-l3-i2c-2026-11-08.log)):
L0, L1, L2, L3, L6, L7 pass. L3 prints `LW_L3_OK` with no
`LW_L3_PARTIAL_NO_RX`, at 1.33M steps. That means the full tier passed:
pointer write, then `requestFrom` of CONFIG and BUS, and both values matched.
With the INA219 removed from the system, the same ELF prints
`LW_L3_FAIL nack`.

Gate test: `cargo test -p labwired-core --test esp32s2_arduino_i2c` runs both
the pass and the no-device negative control. Not modelled: I2C1 (same type
with `irq: 53`, but not in this profile), slave mode, clock stretching, and
bus timing. The controller does not check that the matrix routes I2CEXT0 to
GPIO8/9, so the pad assignment is not verified.

## Vendor status (live, 2026-11-08)

- DigiKey ESP32-S2-MINI-2-N4 (16688710): **Part Status: Active**, in stock.
- Espressif longevity commitment lists ESP32-S2 (12 years from 2020-01-01);
  see [`vendor-status.md`](vendor-status.md) (checked 2026-11-05).
