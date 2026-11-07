# Known limitations

- The firmware is bare-metal RISC-V loaded directly at its mapped flash
  address. The ESP32-C3 ROM, second-stage bootloader, partition table, Arduino
  runtime and flash-image startup are not exercised.
- The smoke covers GPIO-matrix routing of I2C0 on D4/GPIO6 and D5/GPIO7 plus
  BME280 register reads. It does not cover other XIAO pins or C3 peripherals.
- UART0 bytes are captured at the modeled peripheral. Physical UART0 pad
  propagation through IO_MUX to D6/D7 is not part of this smoke.
- The BME280 model supplies deterministic register values. The smoke does not
  model power, pull-up voltages, conversion timing, oversampling, filtering,
  calibration compensation or physical measurements.
- Wi-Fi, Bluetooth, USB, SPI, ADC, timers, sleep, battery charging and reset
  behavior remain outside the descriptor.
