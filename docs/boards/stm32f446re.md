# STM32F446RE / NUCLEO-F446RE

**Support:** minimal firmware smoke (L1); simulated, not hardware-validated.

- 512 KiB flash, 128 KiB RAM, Cortex-M4F, maximum 180 MHz.
- Model: [`configs/chips/stm32f446re.yaml`](../../configs/chips/stm32f446re.yaml);
  board: [`configs/systems/nucleo-f446re.yaml`](../../configs/systems/nucleo-f446re.yaml).
- Tested: Cortex-M reset/stack, RCC enables, USART2 output on PA2/PA3, GPIO PA5 edges.
- The broader model also declares GPIO, timers, I2C, SPI, ADC, CAN and DMA;
  qualification varies by peripheral. The earlier stock STM32duino matrix
  evidence is recorded in `examples/nucleo-f446re/evidence/arduino-matrix/`.
- Absent: USB, FMC, QuadSPI, DCMI, and other F446-specific blocks.
  Any access to an absent peripheral must produce a fault rather than a success claim.
- Other functions, PLL rate modeling and register-reset parity need further evidence.

[Build / run and validation receipt](../../examples/nucleo-f446re/VALIDATION.md).
[ST product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f446re.html).
