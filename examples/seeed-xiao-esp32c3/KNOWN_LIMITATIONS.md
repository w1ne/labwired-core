# Known limitations

- The system profile models only the ESP32-C3 GPIO block and the XIAO D0-D10
  pins as GPIO. Alternate pin functions are not part of this profile.
- The D10 smoke observes GPIO transitions. The external LED and resistor used
  for physical observation are not simulated components.
- The smoke loads a bare-metal ELF directly; ESP32-C3 mask-ROM and Arduino
  startup are not exercised.
- Wi-Fi, BLE, USB, UART, I2C, SPI, ADC, timers, power management, and other
  board/chip functions are outside the verified scope.
- No physical XIAO ESP32-C3 was used for this smoke.
