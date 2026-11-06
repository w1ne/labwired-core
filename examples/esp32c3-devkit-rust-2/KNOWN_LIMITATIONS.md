# Known limitations — ESP32-C3-DevKit-RUST-2

The board profile includes only the ESP32-C3 GPIO register model, GPIO2's
IO_MUX register window, and a one-pixel protocol-level NeoPixel model attached
to the onboard WS2812B RGB LED.

- ICM-42670-P IMU and SHTC3 sensor are not modeled or connected.
- The plain user LED, button, Wi-Fi, Bluetooth LE, USB, UART bridge, battery,
  reset circuit, and other board pins are outside this profile.
- Firmware bit-bangs GPIO2. The ESP32-C3 RMT controller is not modeled here.
- GPIO2 is an ESP32-C3 strapping pin; this smoke does not test reset-time
  strap behavior.
- The pixel model checks a 24-bit GRB waveform and decoded color. It does not
  model LED voltage thresholds, current, brightness, or temperature behavior.
- IO_MUX values are stored, but pad-level routing and electrical behavior are
  not simulated.
- The smoke ELF reserves a 4 KiB stack; the smoke does not measure application
  stack requirements.
- This is repository simulator evidence. No physical-board capture is
  included.
