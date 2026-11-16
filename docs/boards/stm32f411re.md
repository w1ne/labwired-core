# STM32F411RE / NUCLEO-F411RE

**Support:** raw-register firmware smoke (PR-gated) plus the stock STM32duino
Arduino matrix row L0-L7 (no CAN on this part); simulated, not
hardware-validated.

- 512 KiB flash, 128 KiB SRAM, Cortex-M4F, maximum 100 MHz, LQFP64.
- Model: [`configs/chips/stm32f411re.yaml`](../../configs/chips/stm32f411re.yaml);
  board: [`configs/systems/nucleo-f411re.yaml`](../../configs/systems/nucleo-f411re.yaml).
- Same die and register set as the Black Pill
  [`stm32f411ceu6`](../../configs/chips/stm32f411ceu6.yaml); the LQFP64
  package adds the PC0..PC5 ADC pads (ADC1_IN10..IN15).
- PR-gated smoke: Cortex-M reset/stack, RCC enables, USART2 output on PA2,
  GPIO PA5 (LD2) edges.
- Stock STM32duino `NUCLEO_F411RE` (HSE bypass 8 MHz, PLL 100 MHz): Serial,
  `delay`, LD2 blink, Wire INA219 exact register reads, SPI MAX31855 exact
  frame, `analogRead(A0)` at a board-held 1650 mV = 511 (10-bit), and
  `analogWrite` on LD2 (TIM2_CH1, 1 kHz), `micros()`. Evidence:
  `examples/nucleo-f411re/evidence/arduino-matrix/`.
- Stubbed (read zero): DMA1/DMA2, USB OTG FS, SDIO, CRC, WWDG,
  I2S2EXT/I2S3EXT, DBGMCU (no IDCODE).
- Not proven: no silicon register diff, no reset-value capture.

[Build / run and validation receipt](../../examples/nucleo-f411re/VALIDATION.md).
[ST product page](https://www.st.com/en/evaluation-tools/nucleo-f411re.html).
