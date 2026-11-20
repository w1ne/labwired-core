# STM32L432KC / NUCLEO-L432KC

**Support:** raw-register firmware smoke (PR-gated) plus the stock STM32duino
Arduino matrix row L0-L8; simulated, not hardware-validated.

- 256 KiB flash, 64 KiB SRAM, Cortex-M4F, maximum 80 MHz, UFQFPN32.
- Model: [`configs/chips/stm32l432kc.yaml`](../../configs/chips/stm32l432kc.yaml);
  board: [`configs/systems/nucleo-l432kc.yaml`](../../configs/systems/nucleo-l432kc.yaml).
- Instance set from ST's `stm32l432xx.h` (RM0394 L43x), not the L476: USART1,
  USART2, LPUART1, SPI1, SPI3, I2C1, I2C3, one ADC, TIM1/2/6/7/15/16, LPTIM1/2,
  bxCAN1, DAC, DMA1/2, GPIOA/B/C/H. Register behaviour reuses the STM32L4
  family models.
- PR-gated smoke: Cortex-M reset/stack, RCC enables, USART2 output on PA2,
  GPIO PB3 (LD3) edges.
- Stock STM32duino `NUCLEO_L432KC` (MSI PLL 80 MHz + LSE): Serial, `delay`,
  LD3 blink, Wire INA219 exact register reads, SPI MAX31855 exact frame,
  `analogRead(A0)` at a board-held 1650 mV, `analogWrite` on LD3 (TIM2_CH2,
  1 kHz), `micros()`, bxCAN1 loopback. Evidence:
  `examples/nucleo-l432kc/evidence/arduino-matrix/`.
- Absent: USB FS device, CRS, OPAMP, SWPMI and FIREWALL are not declared, so
  an access faults instead of reading zeros. SYSCFG is a read-zero stub.
- Not proven: no silicon register diff, no reset-value capture, REV_ID is a
  placeholder 0.

[Build / run and validation receipt](../../examples/nucleo-l432kc/VALIDATION.md).
[ST product page](https://www.st.com/en/evaluation-tools/nucleo-l432kc.html).
