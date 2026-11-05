# Known limitations — ESP32-C61-DevKitC-1 v2.0

The board slice covers the HP core and memory windows, UART0/PCR for the USB
UART console, the C61 direct GPIO8 output registers and selector, the GPIO8
IO_MUX word, and one protocol-level WS2812B pixel attached to GPIO8.

- GPIO is limited to GPIO8 direct output. The model honors GPIO8's OUT latch,
  output-enable W1TS, selector value/OE_SEL, and IO_MUX function select. The
  GPIO matrix, other GPIO pads, input behavior, interrupt/status registers,
  and GPIO peripherals such as RMT are not modeled by this slice.
- The pixel model validates the 24-bit GRB waveform and decoded color. It does
  not model the LED's analog thresholds, brightness, current, or temperature.
- PSRAM, ROM boot, Wi-Fi, Bluetooth LE, USB Serial/JTAG, the LP subsystem,
  buttons, power, and reset behavior are outside this board profile.
- The smoke ELF reserves a 4 KiB stack; this is not a general application
  stack-size recommendation.
- Evidence is from the repository's local deterministic simulator. There is no
  physical-board capture or hosted `model_verified` verdict.
