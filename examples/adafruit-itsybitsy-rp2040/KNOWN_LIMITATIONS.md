# Known limitations

This profile covers only the red user LED on D13 / GP11. The board system maps
that pin to SIO, and the smoke configures the RP2040 IO_BANK0 function and
repeatedly toggles SIO output. Other board IO is outside the profile.

In particular, this profile does not model the onboard NeoPixel and its power
control, BOOTSEL/RESET buttons, USB/UF2, or the UART, SPI, I2C, ADC, PWM, DMA,
and flash-controller paths. The descriptor records 8 MiB external flash and
264 KiB SRAM, but the smoke does not execute the physical boot ROM or QSPI
boot path. There are no zero-answering stubs in this focused descriptor.

The simulator assumes an ideal VDD rail for this profile, so power-on and
brown-out reset behavior are not simulated. The result is local simulator
evidence, not a physical-board capture.
