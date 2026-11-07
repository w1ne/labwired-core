# Known limitations

- The descriptor covers the ESP32-C6 high-performance core only. The low-power
  core and its peripherals are outside the profile.
- Firmware is loaded as a bare-metal ELF; mask-ROM and flash-image boot are
  not modeled by this smoke.
- UART0 output is captured directly by the simulator. The USB-to-UART bridge's
  electrical propagation through GPIO16/GPIO17 is not modeled.
- RMT TX channel 0, GPIO8 output, and the WS2812B pixel are exercised. RMT RX,
  other channels, radio protocols, USB, BOOT/RESET buttons, ADC, I2C, SPI, and
  other board functions are outside this profile.
- IO_MUX register writes are recorded, but analog levels, power, and physical
  pad electrical behavior are not modeled.
- The smoke proves the modeled path in the local repo runner; it does not
  claim silicon parity or hardware confirmation.
