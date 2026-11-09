# Known limitations — ESP32-C61-DevKitC-1 v2.0

The board slice covers the HP core and memory windows, UART0/PCR for the USB
UART console, I2C0 and a BME280 on GPIO4/5, LEDC PWM on GPIO6, and one
protocol-level WS2812B pixel on GPIO8.

- The GPIO matrix covers only I2C0 input/output signals 46/47 and LEDC output
  signals 0–5. The direct GPIO output path is exercised on GPIO8. Other GPIO
  pads, input behavior, and interrupt/status registers are outside this slice.
- The LEDC model uses the C61 timer and channel register fields, with a fixed
  reset-selected 40 MHz XTAL source. Changing the PCR LEDC source is not
  modeled. The smoke exercises timer 2/channel 0; LEDC fade and other channel
  use are not asserted by this board smoke.
- The SG90 model receives the committed LEDC duty and routed GPIO waveform. It
  does not model current draw, torque, gear backlash, or the external supply;
  servo-angle telemetry is not a smoke assertion.
- The pixel model validates the 24-bit GRB waveform and decoded color. It does
  not model the LED's analog thresholds, brightness, current, or temperature.
- PSRAM, ROM boot, Wi-Fi, Bluetooth LE, USB Serial/JTAG, the LP subsystem,
  other buttons/pads, power, and reset behavior are outside this board profile.
- The smoke ELF reserves a 4 KiB stack; this is not a general application
  stack-size recommendation.
- Evidence is from the repository's local deterministic simulator. There is no
  physical-board capture or hosted `model_verified` verdict.
