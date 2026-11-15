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
| L4 SPI MAX31855 | **pass** | `SPI.transfer` reads the MAX31855 frame 0x01901600 via GP-SPI2 (see below) |
| L5 ADC | **pass** | `analogRead(A0)` converts on SENS: exact 2048 at 12 bits (see below) |
| L6 PWM | **pass** | 1 kHz, 50 % PWM on the GPIO2 pad via LEDC LS ch6 (see below) |
| L7 micros | **pass** | 80 MHz SYSTIMER (`SYSTIMER_LL_TICKS_PER_US` = 80) |
| L8 CAN | **pass** | TWAI self-reception returns ID 0x123 / 0xA5 (register-level, no bus; see below) |

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

## L4 — GP-SPI2 (added later the same day)

The S2's GP-SPI matches neither model the engine already had. Its own
`soc/esp32s2/register/soc/spi_reg.h` launches with `SPI_CMD.USR` at bit 24,
like the S3. But it keeps separate `MOSI_DLEN` (0x24) and `MISO_DLEN` (0x28)
and reports completion in `SPI_SLAVE.TRANS_DONE` (0x30 bit 4), like the
classic part. `USER` is at 0x18 and `W0` at 0x98. The classic `Esp32Spi`
reads CMD bit 24 as `FLASH_SE`, and the S3 model has a single `MS_DLEN` at
0x1C. So the new type `esp32s2_spi` is a small model of its own: a register
file seeded with the header's reset values, and a synchronous data phase.
Each W-buffer byte goes through the attached devices, and with `USR_MISO`
the reply goes back into the same lane. `USR` self-clears and `TRANS_DONE`
latches. `CTRL.WR/RD_BIT_ORDER` select LSB-first. The profile maps SPI2
(FSPI, Arduino's default `SPI`) at 0x3F42_4000 and SPI3 at 0x3F42_5000.

Re-run of the row
([`arduino-matrix-esp32s2-l4-spi-2026-11-08.log`](arduino-matrix-esp32s2-l4-spi-2026-11-08.log)):
L0, L1, L2, L3, L4, L6, L7 pass, L5 and L8 skip. L4 prints `LW_L4_OK` at
852k steps, so the four `SPI.transfer(0)` bytes equal the MAX31855 model's
default frame 0x01901600 (25.0 C hot junction, 22.0 C internal, no fault).
With the MAX31855 removed from the system, the same ELF prints
`LW_L4_FAIL frame=0xffffffff` (MISO idles high).

Gate test: `cargo test -p labwired-core --test esp32s2_arduino_spi` runs the
pass and the no-device negative control. The unit tests in
`peripherals::esp32::spi_s2` cover reset values, bit order, and the bit-24
launch (a classic bit-18 write does not start a transaction).

Not modelled: DMA, slave mode, the command/address/dummy phases (only the
data phase reaches a device), a waveform on the pads, and the interrupt.
`ETS_SPI2_INTR_SOURCE` (33) is never raised, so ESP-IDF `spi_master` in
interrupt mode would not complete. CS is not routed: the MAX31855 kit is
broadcast, so the CS pin in the system file (GPIO5, the sketch's) is not
checked.

## L8 — TWAI self-reception (added later the same day)

The S2 TWAI is the S3's controller. `soc/esp32s2/register/soc/twai_struct.h`
matches `soc/esp32s3/register/soc/twai_struct.h` field for field. The one
difference is `BUS_TIMING_0.brp`: 14 bits on the S2, 13 on the S3. The model
stores all 16 low bits, so both round-trip. No new model was needed: the
profile maps the existing `esp32s3_twai` type at 0x3F42_B000
(`esp32s2.peripherals.ld`: `TWAI = 0x3f42B000`) with `irq: 47`
(`ETS_TWAI_INTR_SOURCE` in `soc/esp32s2/include/soc/interrupts.h`). The L8
sketch gets an `ARDUINO_ESP32S2_DEV` branch with the same register sequence
and checks as the S3 branch. It leaves reset mode, loads the standard frame
(ID 0x123, DLC 1, data 0xA5), and issues `CMD.self_rx_req`. Then it checks
`STATUS.rx_buf_st`, `RX_MSG_CNT == 1` and the frame bytes.

Re-run of the row
([`arduino-matrix-esp32s2-l8-twai-2026-11-08.log`](arduino-matrix-esp32s2-l8-twai-2026-11-08.log)):
L0, L1, L2, L3, L4, L6, L7 and L8 pass. L5 skips. UART0 prints `LW_L8_BOOT`,
then `LW_L8_OK`. Mutation check: with the TWAI window moved off 0x3F42_B000,
the same ELF stops after `LW_L8_` and the run fails.

Gate test: `cargo test -p labwired-core --test esp32s2_arduino_twai` reaches
`LW_L8_OK`. It then reads the controller back over the bus: operating mode,
`rx_buf_st` set, `RX_MSG_CNT` 1, and the RX buffer bytes `01 24 60 A5`.

Not modelled: a CAN bus. There are no peers, no arbitration, no ACK, and no
bit timing. `tx_req` completes at once, and only self-reception produces an
RX frame. The sketch is register-level, not the ESP-IDF `twai` driver: the
driver's alert/ISR path through source 47 is not exercised.

## L5 — SENS oneshot ADC (added later the same day)

Before this change, linking `analogRead()` stopped the S2 before `setup()`.
IDF's boot-time `adc_hw_calibration` constructor busy-polls
`SENS_SAR_MEAS1_CTRL2.MEAS1_DONE_SAR` (`read_cal_channel`, pc 0x40097287),
and SENS was part of a zero-answering stub page. On the S2 the oneshot driver
converts on this RTC-domain controller, not on APB_SARADC.

- **SENS (`esp32s2_sens`, 0x3F40_8800).** The S2 block shares 0x000..0x058
  with the S3 field for field: START bit 17, DONE bit 16, DATA [15:0],
  `SAR1_EN_PAD` [30:19], and `SLAVE_ADDR1.MEAS_STATUS` [29:22]. So it is the
  existing `Esp32s3Sens` with the S2's own register map. Reset values and
  write masks come from the ESP32-S2 SVD shipped with arduino-esp32. The
  touch/DAC/COCPU tail differs, and `SARDATE` (0x01906140) is at 0x148. The
  S2's RTC SAR is 13-bit (`SOC_ADC_RTC_MAX_BITWIDTH`), so its codes are too:
  mid-scale 0x1000 with nothing injected, and `mV * 8191 / 3300` on an
  injected channel. The generic `SystemBus::set_input` analog path reaches
  it unchanged.
- **APB_SARADC (`esp32s2_apb_saradc`, 0x3F44_0000).** Once SENS answered,
  the next fault was `adc_hal_arbiter_config` writing `ARB_CTRL` (0x38). The
  S2 map diverges from the S3's after 0x3C, so this is a register file of
  its own, seeded from the SVD. The oneshot path only configures it.
- The RTC page is now split: RTC_CNTL+RTCIO stub (2 KiB), SENS (1 KiB),
  RTC_I2C stub (1 KiB).

**The sketch was reading the wrong pin.** arduino-esp32's `A0` is a
`static const uint8_t`, not a macro, so the sketch's `#if defined(A0)` was
false and it read GPIO0. GPIO0 is not an ADC pin on the S2:
`adc_oneshot_io_to_channel` fails and `analogRead` returns 0 without
converting (traced). `0 == 0` satisfied the old "two equal samples" check.
So with only the model change, the cell "passed" with `LW_L5_VAL 0 0`. The
first gate test caught this. The sketch's `ARDUINO_ESP32S2_DEV` branch now
reads `A0` (GPIO1 = ADC1_CH0) and calls `analogReadResolution(12)` (the
core's `mapResolution` shifts 13 → 12). It accepts only the exact twin code
2048. The other ESP32 rows probably have the same `defined(A0)` miss. GPIO0
is ADC1_CH0 on the C3, but not on the S3. Not checked or changed here.

Re-run of the row
([`arduino-matrix-esp32s2-l5-adc-2026-11-08.log`](arduino-matrix-esp32s2-l5-adc-2026-11-08.log)):
all nine cells pass (9 pass / 0 skip / 0 fail). UART0 prints `LW_L5_BOOT`,
then `LW_L5_OK`. The matrix run stops at the marker, so the
`LW_L5_VAL 2048 2048` line that follows is checked by the gate test below.

Gate test: `cargo test -p labwired-core --test esp32s2_arduino_adc`. With
nothing injected it expects `LW_L5_OK` and `LW_L5_VAL 2048 2048`. With 1000 mV
injected on ADC1_CH0 it expects `LW_L5_VAL 1241 1241` (2482 at 13 bits, then
shifted). That is not the 1240 a 12-bit model would give, and the exact-2048
oracle then fails, as it should.

Not modelled: the analog cells (attenuation, the calibration ground short
via regi2c, noise), ADC2 injection (fixed mid-scale), digital-controller
conversions and DMA (continuous mode), touch, the temperature sensor, the
ULP, and the APB_ADC interrupt. eFuse holds no ADC calibration, so
`analogReadMilliVolts` uses IDF's no-eFuse fallback and was not checked.

## Vendor status (live, 2026-11-08)

- DigiKey ESP32-S2-MINI-2-N4 (16688710): **Part Status: Active**, in stock.
- Espressif longevity commitment lists ESP32-S2 (12 years from 2020-01-01);
  see [`vendor-status.md`](vendor-status.md) (checked 2026-11-05).
