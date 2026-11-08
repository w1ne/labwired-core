# NUCLEO-F411RE + BME280 limitations

The smoke profile models only the paths used by this integration:

- RCC clock gates for GPIOA, GPIOB, USART2, and I2C1
- STM32F4 flash control and PWR register models used during HAL clock setup
- Cortex-M SysTick configured by HAL_Init
- GPIOA/GPIOB register banks
- USART2 capture for the ST-LINK VCP firmware log
- I2C1 plus the attached BME280 register model

All other F411RE peripherals are omitted from this profile and fault as
unsupported. The model does not include the physical ST-LINK/USB bridge, board
power sequencing, sensor conversion dynamics, environmental variation, or
hardware timing tolerances. The D14/D15 (PB9/PB8) I2C route is exercised in the twin;
physical jumper wiring has not been bench-confirmed.
