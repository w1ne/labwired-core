# Known limitations — ESP32-C3-DevKitM-1-N4X

The board profile includes C3 GPIO, the GPIO8 IO_MUX register window, RMT TX0
symbol playback, and a one-pixel protocol-level NeoPixel model attached to the
built-in addressable RGB LED (schematic reference D6, SK68XXMINI-HS).

- RMT models TX0/TX1 symbol playback and TX_END on the fixed 80 MHz APB source
  with integer divider settings. This smoke exercises TX0 with one 24-bit
  frame; it does not cover RMT RX sampling, carrier modulation, DMA, fractional
  clock division, or arbitrary multi-block allocation.
- The pixel model checks a 24-bit GRB waveform and decoded color. It does not
  model the exact LED die's voltage thresholds, current, brightness, or
  temperature behavior.
- The smoke ELF reserves a 4 KiB stack. The smoke does not measure worst-case
  application stack usage.
- The repository runner starts this bare-metal ELF at its linked entry point;
  it does not exercise the C3 mask-ROM reset and flash-boot sequence.
- The smoke selects GPIO function 1 and the C3 RMT TX0 matrix signal on GPIO8.
  Pad-level electrical behavior, strapping effects, and boot behavior are not
  simulated.
- Wi-Fi, Bluetooth LE, USB Serial/JTAG, USB-to-UART bridge, UART pins, buttons,
  power, reset, and other headers/peripherals are outside this profile.
- This is repository simulator evidence. No physical-board capture is included.
