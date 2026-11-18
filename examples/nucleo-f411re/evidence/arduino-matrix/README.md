# NUCLEO-F411RE — stock STM32duino matrix row

Stock STM32duino (PlatformIO `ststm32@19.7.1`, board `nucleo_f411re`, the
core's own `variant_NUCLEO_F411RE.cpp` clock setup), run by
`validation/arduino-matrix/run_matrix.py --boards stm32f411re` on
`configs/chips/stm32f411re.yaml` (system
`validation/arduino-matrix/systems/stm32f411re.yaml`). The sketches are
unchanged except for two `#if` lines: L3 puts `STM32F411xE` on the exact tier,
and L5 has an exact oracle for `ARDUINO_NUCLEO_F411RE`.

Board: ST NUCLEO-F411RE, STM32F411RET6. Both product pages read **ACTIVE**
live on 2026-11-15 (real Chrome); see `../vendor-status.md`.

**8 pass / 1 skip / 0 fail.** L8 is skipped because the STM32F411 has no CAN
peripheral.

| cell | result | steps | what it proves |
|---|---|---|---|
| L0 serial boot | pass | 103,571 | `SystemClock_Config`: HSE bypass 8 MHz (ST-LINK MCO), PLL M=4 N=100 P=2 → 100 MHz; `Serial` = USART2 (PA2/PA3, ST-LINK VCP) |
| L1 serial loop | pass | 203,574 | `loop()` + `delay()` on SysTick |
| L2 blink | pass, 3 edges | 303,654 | `digitalWrite(LED_BUILTIN)` toggles LD2 = PA5 (`led_watch: gpioa:5`) |
| L3 I2C | pass, exact tier | 413,370 | `Wire` = I2C1 (PB8/PB9) reads INA219 config 0x399F / bus 0x19CA (`uart-full-run.log`: a run to 12M steps prints no `LW_L3_PARTIAL_NO_RX`) |
| L4 SPI | pass | 205,217 | `SPI` = SPI1 (PA5/PA6/PA7) reads the MAX31855 frame 0x01901600 exactly |
| L5 ADC | pass, exact | 114,135 | `analogRead(A0)` = ADC1_IN0 on PA0 held at 1650 mV reads **511 511** (`uart-full-run.log`); the sketch accepts only 511 |
| L6 PWM | pass, pad edges | 3,103,671 | `analogWrite(LED_BUILTIN, 128)` = TIM2_CH1 on PA5 (AF1), 60 transitions on the pad, period 100,000 cycles = **1.000 kHz** at 100 MHz, high 50,196 cycles = duty **50.20 %** (128/255) |
| L7 timer | pass | 403,635 | `micros()` advances monotonically across `delay(2)` |
| L8 CAN | skip | — | no CAN on STM32F411 |

Each cell directory holds the matrix's `test.yaml`, `result.json`,
`uart.log` and `junit.xml` (the run stops at the first assertion pass, so
`uart.log` is cut there). Paths are repo-relative; the firmware is the matrix
build of the cell's sketch.

## Engine change this row needed

* **F2/F4 `ADC_CR1.RES[25:24]` sets the conversion width**
  (`crates/core/src/peripherals/adc.rs`, RM0383 §11.12.2: 00 = 12, 01 = 10,
  10 = 8, 11 = 6 bits). STM32duino 2.12 programs `ADC_RESOLUTION_10B` in
  hardware and does not re-map the result. The model ignored RES and returned
  the 12-bit code, so the row read `2047 2047` instead of `511 511`. The F1
  layout has no RES field (CR1 bit 24 is DUALMOD there) and is unchanged.
* **`input_mv` on the generic `adc` factory**: board-level static channel
  voltages. This is the same hunk as open PR #1935 (NUCLEO-L432KC), so the
  two PRs merge in either order.

## Controls

* `L5_adc` on the matrix system with an empty `adc1` config (no `input_mv`):
  the same ELF reads `2 3` (the model's incrementing fallback) and prints
  `LW_L5_FAIL` (`L5_adc/uart-control-no-input-mv.log`).
* Before the RES fix, the same L5 ELF with `input_mv` read `2047 2047`.
* L3 and L5 on the Black Pill row (`stm32f411ceu6`), and on the F401, F407
  and F446RE rows, still pass after the sketch and ADC changes. The Black
  Pill now also takes the exact L3 tier.
