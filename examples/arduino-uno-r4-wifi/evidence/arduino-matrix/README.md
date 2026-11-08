# Arduino UNO R4 WiFi — stock Arduino matrix row

Stock ArduinoCore-renesas (`renesas-ra@1.9.0`, board `uno_r4_wifi`), the
matrix sketches unchanged except for the L5 oracle branch, run by
`validation/arduino-matrix/run_matrix.py --boards arduino-uno-r4-wifi` on
`configs/chips/r7fa4m1ab-uno-r4-wifi-smoke.yaml`.

| cell | result | what it proves |
|---|---|---|
| L0 serial boot | pass | FSP SystemInit (VBATT switch, HOCO 48 MHz, flash cache, SPMON), the core's `init()` (ADC140 open, AGT time base), `Serial` = SCI9 |
| L1 serial loop | pass | `loop()` + `delay()`/`millis()` on the AGT |
| L2 blink | pass | `digitalWrite(LED_BUILTIN)` toggles P102 (2 edges, `led_watch: port1:2`) |
| L3 I2C | pass, exact tier | `Wire` = IIC1 reads INA219 config 0x399F and bus 0x19CA; no `LW_L3_PARTIAL_NO_RX` (`L3_i2c_sensor/full-uart.log`) |
| L4 SPI | pass, exact frame | `SPI` = SPI0 (`ra_spi`) reads the MAX31855 frame 0x01901600; 100 °C control reads 0x06401600 (`L4_spi_sensor/`) |
| L5 ADC | pass, exact | `analogRead(A0)` = 675 with A0 on 3V3 (see below) |
| L6 PWM | pass, 28 edges | `analogWrite(13, 128)` = GTIOC2B on P102 (see below) |
| L7 timer | pass | `micros()` advances monotonically across `delay(2)` |
| L8 CAN | skip | RA4M1 CAN0 is not modelled |

Each cell directory holds the matrix's `test.yaml`, `result.json`,
`uart.log` and `junit.xml` as `run_matrix.py` ran them (they stop at the
first assertion pass, so later lines are in `full-uart.log` where shown).

## Boot path

The sketch is linked at 0x4000 behind the 16 KB bootloader
(`variants/UNOWIFIR4/memory_regions.ld`, `FLASH_IMAGE_START`); the chip's
`reset_vector_offset: 0x4000` enters it there. The variant builds with
`-DNO_USB`, so `Serial` is UART1 = SCI9 on P109/P110 (the ESP32-S3 bridge),
not USB CDC. Before this round the boot stopped at, in order: SystemInit's
wait on VBTSR.VBTRVLD (new `ra4_sysc`), the flash cache (FCACHE stub), the
stack-pointer monitor (SPMON stub), ADC140 and the core's phantom ADC unit 1
at 0x4005C200 (reserved on the RA4M1; stub), and the AGT time base.
A fault inside `init()` is silent on this core: its HardFault backtrace
prints through `UART::write_raw` on a NULL SCI, so the CPU spins on
address 3.

## L5

The matrix system ties A0 (P014 = AN009) to 3300 mV. ArduinoCore-renesas
always opens ADC140 at 14 bits (`ADCER.ADPRC` = 3) and maps the result to
the requested 10 bits. 3300 mV of the 5 V AVCC0 is 10,812 (14-bit), which
maps to **675** (silicon nominal: 10,813 → 675). `full-uart.log` prints
`LW_L5_VAL 675 675`.

`controls/adc14-off-*`: same ELF, same CLI, `adc14: false`. The model then
treats ADPRC = 3 as 12-bit (the RA6/RA8 meaning): 2,702 → 168, and the
sketch prints `LW_L5_FAIL` / `LW_L5_VAL 168 168`. Before this round's sketch
branch, L5 on this board would have passed on the generic "two equal
samples" rule, which a channel reading 0 also meets.

## L6

`result.json` `logic_edges` holds 28 transitions on P102. Every period is
97,956 cycles at 48 MHz (**490.0 Hz**, the core's default PWM rate) and
every high phase is 49,168 cycles (**50.2 %** = 128/255).

`controls/no-gtioc-pad-*`: same ELF, the chip with `gtioc_pads` removed. The
sketch still prints `LW_L6_OK`, but P102 shows **0 edges**. The marker alone
does not prove PWM; `pwm_watch` does.

## Not shown

USB (the R4 WiFi's native USB is not on `Serial` here), the ESP32-S3 module
and Wi-Fi, the 12x8 LED matrix, SPI, CAN, the DAC, and PWM on any header pin
other than D13 (only GTIOC2B → P102 is bound). Controls were run from
/dev/shm copies of the chip and system yaml with absolute paths.
