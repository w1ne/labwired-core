# NUCLEO-F411RE + MCP9808 limitations

The smoke profile models only the paths exercised by this integration:

- RCC, flash control, and PWR used during STM32duino startup
- GPIOA/GPIOB, SysTick, and USART2 for the board and firmware console
- I2C1 with the MCP9808 register model

The MCP9808 model does not simulate its alert output or conversion-time
gating, and it clamps the temperature input to non-negative values until the
signed encoding path is verified. Other F411RE peripherals are omitted and
fault as unsupported. The simulator does not model the physical ST-LINK/USB
bridge, board power sequencing, or physical jumper wiring; the smoke has not
been bench-confirmed.
