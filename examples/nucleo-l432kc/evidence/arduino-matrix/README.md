# NUCLEO-L432KC — stock STM32duino matrix row

Stock STM32duino (PlatformIO `ststm32@19.7.1`, board `nucleo_l432kc`, the
core's own `variant_NUCLEO_L432KC.cpp` clock setup), the matrix sketches
unchanged except for the L8 `#if` (same bxCAN1 as the L476) and an exact L5
oracle, run by `validation/arduino-matrix/run_matrix.py --boards stm32l432kc`
on `configs/chips/stm32l432kc.yaml` (system
`validation/arduino-matrix/systems/stm32l432kc.yaml`).

Board: ST NUCLEO-L432KC, STM32L432KCU6. Both product pages read **ACTIVE**
live on 2026-11-15 (real Chrome); see `../vendor-status.md`.

**9 pass / 0 skip / 0 fail.**

| cell | result | steps | what it proves |
|---|---|---|---|
| L0 serial boot | pass | 84,306 | `SystemClock_Config`: LSE on, MSI 4 MHz → PLL 80 MHz, PLLSAI1 48 MHz, FLASH latency 4, voltage scale 1, MSI PLL mode; `Serial` = USART2 (PA2/PA15, ST-LINK VCP) |
| L1 serial loop | pass | 164,303 | `loop()` + `delay()` on SysTick |
| L2 blink | pass, 3 edges | 244,365 | `digitalWrite(LED_BUILTIN)` toggles LD3 = PB3 (`led_watch: gpiob:3`) |
| L3 I2C | pass, exact tier | 7,735,956 | `Wire` = I2C1 (PB6/PB7) reads INA219 config 0x399F / bus 0x19CA (`uart-full-run.log`: a run to 12M steps prints no `LW_L3_PARTIAL_NO_RX`) |
| L4 SPI | pass | 165,916 | `SPI` = SPI1 (PB3/PB4/PB5) reads the MAX31855 frame 0x01901600 exactly |
| L5 ADC | pass, exact | 107,281 | `analogRead(A0)` = ADC1_IN5 on PA0 held at 1650 mV reads **511 511** (`uart-full-run.log`); the sketch accepts only 511 |
| L6 PWM | pass, pad edges | 2,484,393 | `analogWrite(LED_BUILTIN, 128)` = TIM2_CH2 on PB3 (AF1), 60 transitions on the pad, period 80,000 cycles = **1.000 kHz** at 80 MHz, duty **50.20 %** (128/255) |
| L7 timer | pass | 324,364 | `micros()` advances monotonically across `delay(2)` |
| L8 CAN | pass | 85,920 | bxCAN1 register-level loopback, ID 0x123 / data 0xA5 |

Each cell directory holds the matrix's `test.yaml`, `result.json`,
`uart.log` and `junit.xml` (the run stops at the first assertion pass, so
`uart.log` is cut there). Paths are repo-relative; the firmware is the matrix
build of the cell's sketch.

## Engine changes this row needed

* **STM32L4 ADC_CR command bits are "rs"** (`crates/core/src/peripherals/adc.rs`).
  ADCAL, JADSTP, ADSTP, JADSTART, ADSTART, ADDIS and ADEN: software can set
  them, writing 0 has no effect. The L4 HAL starts a conversion with
  `MODIFY_REG(CR, ADC_CR_BITS_PROPERTY_RS, ADSTART)`, which writes ADEN = 0.
  The model stored that write raw, the ADC disabled itself, no conversion
  started, and stock `analogRead()` returned 0 on every L4-register-file row.
  ADSTP/JADSTP and ADDIS now complete at once and clear their bits.
* **ADCAL completes on the synchronous clock.** `ADC_CCR.CKMODE` != 00 clocks
  the ADC from HCLK without a `CCIPR.ADCSEL` source, so calibration runs.
  STM32duino writes CKMODE = 11. The NUCLEO-L476RG capture (ADCAL latched with
  CKMODE = 00 and no CCIPR source) is unchanged and still passes.
* **`input_mv` on the generic `adc` factory**: board-level static channel
  voltages, the same key the SAMD21, RA and i.MX RT ADCs already take.

## Controls

* `L5_adc` with the matrix system minus its `input_mv` line: the same ELF
  reads `930 930` (the model's fixed 3.0 V internal source) and prints
  `LW_L5_FAIL`. The exact oracle separates "converted the board voltage on
  the selected channel" from "converted something".
* Before the ADC change, the same L5 ELF read `0 0` on both NUCLEO-L432KC and
  NUCLEO-L476RG (the generic oracle accepted it because the two samples were
  equal). End-of-run registers: `ADC1_CR = 0x1000_0004` (ADSTART set, ADEN
  clear), `ADC_CCR = 0x0003_0000`.
