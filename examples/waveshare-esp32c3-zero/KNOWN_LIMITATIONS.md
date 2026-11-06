# Known limitations — Waveshare ESP32-C3-Zero

This narrow profile includes GPIO output, the GPIO10 IO_MUX register, and a
protocol-level model of the onboard WS2812 RGB LED.

- Firmware bit-bangs the LED data signal through GPIO10. The RMT controller is
  not included in this board profile and is not claimed by this smoke.
- The pixel model checks a 24-bit GRB waveform and decoded color; it does not
  model the exact LED die's voltage thresholds, current, brightness, or thermal
  behavior.
- Wi-Fi, Bluetooth LE, USB Serial/JTAG, UART, BOOT/RESET buttons, power,
  castellated-header electrical behavior, and other C3 peripherals are outside
  this profile.
- This is repository simulator evidence. No physical-board capture is included.
