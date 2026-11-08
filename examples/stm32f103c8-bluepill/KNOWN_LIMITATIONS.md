# Known limitations

- The system profile models only RCC, GPIOC, and the PC13 onboard LED.
- Blue Pill is a generic board design with manufacturer variants. The smoke
  represents the cited schematic's active-low PC13 LED path.
- USB, ST-LINK/SWD behavior, UART, ADC, timers, SPI, I2C, CAN, button, and the
  board's power circuitry are not modeled or claimed by this profile.
- The simulator assumes an ideal 3.3 V supply; it does not model the board's
  regulator or power-on behavior.
- The smoke is simulator evidence; no physical board capture is included.
