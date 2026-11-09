# NUCLEO-H723ZG Arduino matrix evidence

Demand placement: **other STM32, rank 4 at 13% measured MCU usage**; Nucleo/Discovery kits are 23%. This is H7 depth work on the already onboarded H723 board, not a new-family demand pick.

Generated 2026-11-09 with stock STM32duino (`ststm32@19.7.1`, `nucleo_h723zg`). The matrix result is **8 pass / 1 skipped / 0 fail**. L0–L7 all compile and run through the LabWired model. The smoke paths include USART3/VCP serial, LD1 GPIO, INA219 on I2C1, MAX31855 on SPI1, ADC1 channel 15/A0, and TIM2_CH1 PWM on PA0.

| Sketch | Result | Evidence |
|---|---:|---|
| L0 serial boot | pass | UART marker; 555,245 steps |
| L1 serial loop | pass | UART marker; 1,105,240 steps |
| L2 blink | pass | PB0/LD1: 3 pad edges; 1,655,357 steps |
| L3 I2C sensor | pass | INA219 read; 8,900,653 steps |
| L4 SPI sensor | pass | MAX31855 read; 1,108,729 steps |
| L5 ADC | pass | A0/PA3 returned 512 from 1,651 mV input; 620,520 steps |
| L6 PWM | pass | PA0/TIM2_CH1: 120 pad edges; 17,055,391 steps |
| L7 timer | pass | timer sketch marker; 2,205,301 steps |
| L8 CAN | skipped | H7 FDCAN/M_CAN model not implemented; omitted from profile |

`scoreboard.md` and `run_matrix.log` are the matrix-level report. Each sketch directory includes its simulation result, UART, JUnit and snapshot output. `result.json` records the firmware hash.

Reproduce from the repository root after building LabWired and installing the Arduino PlatformIO toolchain dependencies:

```bash
python3 validation/arduino-matrix/run_matrix.py \
  --boards stm32h723zg \
  --out examples/nucleo-h723zg/evidence/arduino-matrix
```

## Model boundary

The profile includes the peripherals exercised above. It omits H7 FDCAN, DMA/BDMA/DMAMUX, RTC, OCTOSPI, Ethernet and ADC3. H7 SYSCFG models the EXTI source mux and compensation-cell enable/READY handshake; analog calibration values and the other H7 SYSCFG register groups are not simulated. The board's USB connector is not modeled as USB CDC here: the tested console path is USART3 through the ST-LINK virtual COM port. Model evidence is deterministic simulation, not a capture from physical silicon.
