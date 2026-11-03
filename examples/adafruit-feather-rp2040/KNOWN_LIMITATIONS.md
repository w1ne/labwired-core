# Known limitations

This onboarding profile claims only the D13 red LED path: the board guide maps
D13 to RP2040 GP13, IO_BANK0 selects the SIO function, and SIO drives the pin
high and low. The committed smoke asserts at least eight transitions on
`sio:13`.

The following Feather hardware is outside this profile and is not tested:

- The RGB NeoPixel, which uses PIO and its own power path.
- The STEMMA QT I2C connector and any attached sensor.
- The LiPo charger and charging-status LED.
- USB CDC, USB UF2 boot/flash, and the RP2040 boot ROM / second-stage boot path.
- The revision-dependent BOOTSEL input and optional SWD connector.
- UART, SPI, ADC, PWM, DMA, and other RP2040 peripherals not needed by the LED smoke.

The board-specific chip descriptor maps the actual 8 MiB flash window and
264 KiB SRAM but installs only IO_BANK0 and SIO models. The run starts the
firmware from the mapped XIP image; it is local LabWired engine evidence, not a
physical-board capture.
