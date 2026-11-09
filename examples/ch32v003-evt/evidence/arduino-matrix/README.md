# CH32V003F4P6 — WCH's official Arduino core, matrix row

WCH's own Arduino core (openwch/arduino_core_ch32, via the community
`platform-ch32v` PlatformIO platform, board `genericCH32V003F4P6`), run by
`validation/arduino-matrix/run_matrix.py --boards ch32v003` on
`configs/chips/ch32v003f4p6-ch32fun-smoke.yaml` (system
`validation/arduino-matrix/systems/ch32v003.yaml`).

Pins: platform-ch32v `f26b06a375c5a8b4dd58af5265538e88aa2a68c7`,
framework-arduino-openwch-ch32 `3820604346d0a9f0ee9a72cab8b7583e488e6674`
(the same pins as the CH32V203, CH32V307 and CH32X035 rows). The ELFs are
built by the matrix and not committed: the upstream core has no
redistribution license.

Part status, read live 2026-11-09 in a real Chrome (new tab on :9223):

* WCH, <https://www.wch-ic.com/products/CH32V003.html>: lists "32-bit
  General-purpose RISC-V MCU CH32V003" as a current product (packages
  TSSOP20, QFN20, SOP16, SOP8; datasheet, RM and CH32V003EVT linked). No NRND
  or discontinued notice. WCH does not print a formal lifecycle label.
* LCSC C5187096 (WCH CH32V003F4P6, TSSOP-20): "24,305 In stock, ships now".

**7 pass / 2 skip / 0 fail.**

| cell | result | steps | what it proves |
|---|---|---|---|
| L0 serial boot | pass | 49,722 | stock core boot to `setup()`; `Serial` = USART1 TX on PD5 |
| L1 serial loop | pass | 97,689 | `loop()` + `delay()` on the QingKe V2 STK |
| L2 blink | pass, 3 edges | 145,846 | `digitalWrite(LED_BUILTIN)` toggles PD0 (`led_watch: gpiod:0`) |
| L3 I2C | pass | 202,803 | `Wire` = I2C1 (PC1 SDA, PC2 SCL) reads INA219 config 0x399F / bus 0x19CA |
| L4 SPI | pass | 99,585 | `SPI` = SPI1 (PC5/PC6/PC7) reads the MAX31855 frame 0x01901600 |
| L5 ADC | pass, exact | 50,835 | `analogRead(A0)` = PA2 = ADC channel 0, held at 1000 mV, reads **310 / 310** |
| L6 PWM | pass, 60 pad edges | 1,489,876 | `analogWrite(PD4, 128)` = TIM2_CH1 at reset mapping: period 48,000 ticks (1 kHz at 48 MHz), high 1,500 ticks |
| L7 timer | skip | — | upstream core bug (below), re-measured this round |
| L8 CAN | skip | — | no CAN controller on the CH32V003 |

Each cell directory holds the matrix's `test.yaml`, `result.json`, `uart.log`
and `junit.xml`. The run stops at the first assertion pass, so `uart.log` can
be cut mid-line (L5 prints `LW_L5_VAL 310` before the second value).

## L5 is exact, not "any stable pair"

Before this round the generic L5 oracle accepted the F1 unseeded path, where
DR counts up by one per conversion: on this part that read `2 3` and passed
without measuring anything. The system now holds A0 at 1000 mV and the sketch
requires 310 on `CH32V003`: 1000 / 3300 × 1024 = 310.3, and 310.0 on a 1023
scale, so the code is 310 under either transfer convention. On V00x the core
reads RDATAR raw (no `ADC_CTLR_ADCAL` calibration offset) at
`ADC_RESOLUTION 10`, so `analogRead` does no remapping.

Control (`controls/L5_adc_wrong_channel/`): the same 1000 mV moved to channel
1 makes the cell fail with `LW_L5_FAIL` / `LW_L5_VAL 2 3`.

## L6 duty is the core's, not 50 %

WCH's core sets `PWM_RESOLUTION 12` (pins_arduino.h), so `analogWrite`'s
default write resolution is 12 bits, unlike stock Arduino's 8. Then
`analogWrite(PD4, 128)` asks for 128 / 4096 = 3.125 % at `PWM_FREQUENCY`
1000 Hz. The pad shows exactly that: 1,500 of every 48,000 ticks high. The
same sketch on real silicon behaves the same way.

## L7 skip, re-measured

`controls/L7_timer_unskipped/`: with the skip lifted the cell prints
`LW_L7_FAIL t0=1991 t1=3999 t2=3778 t3=4999`, the same as on 2026-11-07.
`getCurrentMicros()` (ch32/clock.c) computes `CMP+1-CNT`, which assumes a
down-counter. The V2 STK counts up (WCH's own `Delay_Ms` sets CNT=0 and polls
CNTIF at CMP), so `micros()` runs backwards inside each millisecond. This is
the core's bug, not the model's.

## What is not claimed

* The base `configs/chips/ch32v003f4p6.yaml` still carries only main's slices
  (RCC, GPIO, USART1). The row runs on the full lane profile
  (`-ch32fun-smoke`), which models PFIC, STK, ADC1, I2C1, SPI1, TIM1/TIM2,
  DMA1, EXTI, AFIO, PWR and IWDG.
* The OPA, and `analogReadResolution` above 10 bits, are not exercised.
* `LED_BUILTIN=PD0` and `LW_PWM_PIN=PD4` are pin choices passed as build
  flags: the CH32V003F4 variant leaves `LED_BUILTIN` undefined.
