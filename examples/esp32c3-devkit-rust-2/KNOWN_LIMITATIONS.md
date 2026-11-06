# Known limitations — ESP32-C3-DevKit-RUST-2

The board profile includes UART0, GPIO, IO_MUX, the cycle-driven I2C0
controller, a one-pixel protocol-level NeoPixel model attached to the onboard
WS2812B RGB LED, and a declarative SHTC3 I2C model. The smoke reads the
temperature/humidity frame and checks both Sensirion CRC bytes.

- The ICM-42670-P IMU, plain user LED, button, Wi-Fi, Bluetooth LE, USB, UART
  bridge electrical wiring, battery, reset circuit, and other board pins are
  outside this profile.
- The SHTC3 model does not emulate its internal sleep state, reset timing,
  clock stretching, or measurement noise. It enforces conversion availability
  using the datasheet's maximum time for each modeled measurement mode.
- Firmware bit-bangs GPIO2. The ESP32-C3 RMT controller is not modeled here.
- GPIO2 is an ESP32-C3 strapping pin; this smoke does not test reset-time
  strap behavior.
- The pixel model checks a 24-bit GRB waveform and decoded color. It does not
  model LED voltage thresholds, current, brightness, or temperature behavior.
- GPIO matrix routes are checked for I2C0 transactions; analog pad behavior,
  pull-ups, line capacitance, and physical signal integrity are not simulated.
- The smoke ELF reserves a 4 KiB stack; the smoke does not measure application
  stack requirements.
- This is repository simulator evidence. No physical-board capture is
  included.
