# Known limitations — ESP32-C3-DevKitM-1-N4X

The board profile includes only the C3 GPIO register model, the GPIO8 IO_MUX
register window, and a one-pixel protocol-level NeoPixel model attached to the
built-in addressable RGB LED (schematic reference D6, SK68XXMINI-HS).

- The RMT controller is not modeled or used. Firmware bit-bangs GPIO8.
- The pixel model checks a 24-bit GRB waveform and decoded color. It does not
  model the exact LED die's voltage thresholds, current, brightness, or
  temperature behavior.
- The smoke ELF reserves a 4 KiB stack. The smoke does not measure worst-case
  application stack usage.
- IO_MUX values are stored, but pad-level function routing and electrical
  behavior are not simulated.
- Wi-Fi, Bluetooth LE, USB Serial/JTAG, USB-to-UART bridge, UART pins, buttons,
  power, reset, and other headers/peripherals are outside this profile.
- This is repository simulator evidence. No physical-board capture is included.
