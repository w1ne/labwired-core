# NUCLEO-F446RE — stock stm32duino matrix row

Stock STM32duino (PlatformIO `ststm32@19.7.1`, board `nucleo_f446re`, the
core's own `variant_NUCLEO_F446RE.cpp` clock setup), the matrix sketches
unchanged except for an L6 pin choice, the L8 `#if` and the L3 exact-tier
`#if`, run by
`validation/arduino-matrix/run_matrix.py --boards stm32f446re` on
`configs/chips/stm32f446re.yaml` (system
`validation/arduino-matrix/systems/stm32f446re.yaml`).

Board: ST NUCLEO-F446RE (MB1136), STM32F446RET6. Both product pages read
**ACTIVE** live on 2026-11-09 (real Chrome):
https://www.st.com/en/microcontrollers-microprocessors/stm32f446re.html and
https://www.st.com/en/evaluation-tools/nucleo-f446re.html.

**9 pass / 0 skip / 0 fail.**

| cell | result | steps | what it proves |
|---|---|---|---|
| L0 serial boot | pass | 183,961 | `SystemClock_Config`: HSI PLL to 180 MHz, PWR over-drive (ODEN/ODRDY, ODSWEN/ODSWRDY), FLASH latency 5, PLLI2S lock; `Serial` = USART2 (ST-LINK VCP) |
| L1 serial loop | pass | 363,964 | `loop()` + `delay()` on SysTick |
| L2 blink | pass, 3 edges | 544,064 | `digitalWrite(LED_BUILTIN)` toggles LD2 = PA5 (`led_watch: gpioa:5`) |
| L3 I2C | pass, exact tier | 732,885 | `Wire` = I2C1 (PB8/PB9) reads INA219 config 0x399F / bus 0x19CA. The sketch keeps other F4 boards on the ACK-only tier; the F446 takes the read (`uart-full-run.log`: a run to 3M steps prints no `LW_L3_PARTIAL_NO_RX`) |
| L4 SPI | pass | 365,698 | `SPI` = SPI1 (PA5/PA6/PA7) reads the MAX31855 frame 0x01901600 exactly |
| L5 ADC | pass | 199,578 | `analogRead(A0)` converts on ADC1 (deterministic-pair oracle, the same weak tier as the other STM32 F4 rows) |
| L6 PWM | pass, pad edges | 5,584,089 | `analogWrite(PA0, 128)` = TIM2_CH1 on AF1, a waveform on the pad (`pwm_watch: gpioa:0`, ≥ 20 edges), 50.2 % duty at **1.000 kHz** (180,015-cycle period at 180 MHz: TIM2 counts at TIMCLK = 2 x PCLK1 = 90 MHz, derived from RCC_CFGR.PPRE1) |
| L7 timer | pass | 724,025 | `micros()` advances monotonically across `delay(2)` |
| L8 CAN | pass | 185,341 | bxCAN1 register-level loopback, ID 0x123 / data 0xA5 |

Each cell directory holds the matrix's `test.yaml`, `result.json`,
`uart.log` and `junit.xml` (the run stops at the first assertion pass, so
`uart.log` is cut there). Paths are repo-relative; the firmware is the matrix
build of the cell's sketch.

## Engine changes this row needed

* **PWR over-drive** (`config: { profile: stm32f4, over_drive: true }`).
  RM0390 §5.1.4/§5.4: CR.ODEN → CSR.ODRDY, CR.ODSWEN → CSR.ODSWRDY.
  `HAL_PWREx_EnableOverDrive` spins on both. F401/F407 keep the bits reserved.
* **RCC PLLI2S** on every F4 (CR.PLLI2SON 26 → PLLI2SRDY 27, PLLI2SCFGR at
  0x84), plus **PLLSAI** (28 → 29, PLLSAICFGR/DCKCFGR/DCKCFGR2) behind
  `pllsai: true`. Reset values from ST's STM32F401x/F446x SVDs.
* **F4 timer outputs on pads** come from main (#1734): a timer declared
  `input_capture: stm32f4` drives its channels onto the F4 AF table, and
  `bus/timer_kernel_clock.rs` derives TIMxCLK from the APB1 prescaler. The
  chip YAML only has to declare its timers that way.

## Negative controls (`controls/`)

Chip copies with one change each, same ELF, same system otherwise. Each
`system.yaml` loads the `chip.yaml` beside it, so a control reruns with
`labwired test --script controls/<name>/test.yaml` once the firmware path
points at the matrix build:

| control | change | result |
|---|---|---|
| `no_over_drive` | drop `over_drive: true` | **fail**: `HAL_PWREx_EnableOverDrive` never sees ODRDY, `Error_Handler` spins, empty `uart.log`, 20M steps |
| `tim2_no_f4_stage` | drop TIM2 `input_capture: stm32f4` | UART prints `LW_L6_OK`, but PA0 shows **0 edges**; the row's `pwm_watch` fails it. The marker alone proves nothing |
| `no_pllsai` | drop `pllsai: true` | **pass**: stock stm32duino never starts PLLSAI. That half is covered by the unit test only |

There is no chip-YAML control for PLLI2S because it is on for every F4 RCC.
`HAL_RCCEx_PeriphCLKConfig` waits on PLLI2SRDY with a timeout and returns
`HAL_TIMEOUT` → `Error_Handler` without it (stm32f4xx_hal_rcc_ex.c), so the
need comes from reading the HAL, not from a run.

## Known gaps (not claimed)

* **Toolchain.** `ststm32` 20.0.0 (Arduino core 3.0.0) makes `SPI.begin()` a
  no-op (`configSpi` skips `spi_init` when the requested settings equal the
  default-initialized member), so L4 spins on a null SPI instance on every
  STM32 row, on silicon too. The row pins 19.7.1 like CI does.
* Not declared, so firmware that touches them faults as unmapped: UART4/5,
  SPI4, FMPI2C1, TIM6-14, ADC2/3, DAC, bxCAN2, SAI1/2, SPDIFRX, HDMI-CEC,
  QUADSPI, FMC, DCMI, USB OTG FS/HS, SDIO, CRC. No zero-answering stubs.
