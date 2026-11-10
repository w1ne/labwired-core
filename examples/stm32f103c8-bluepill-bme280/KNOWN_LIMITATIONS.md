# Known limitations

- The chip profile declares only RCC, GPIOA/B/C, USART1, I2C1, and SysTick.
  Other F103 peripherals are outside this profile even where the broader
  `configs/chips/stm32f103.yaml` has real models. The broad descriptor also
  contains explicit zero-answer `stub` windows for USB and backup registers;
  this profile includes neither stub window.
- The attached BME280 answers on the I2C1 peripheral model. The test does not
  simulate physical SDA/SCL voltage levels, pull-ups, power sequencing, or
  signal integrity.
- USART1 output is captured by the simulator. A physical Blue Pill needs an
  external 3.3 V USB-to-UART adapter on PA9 to observe the same marker.
- This is a simulator smoke, not a measurement on a physical Blue Pill or
  sensor.
