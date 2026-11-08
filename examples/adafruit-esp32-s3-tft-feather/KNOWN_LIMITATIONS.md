# Known limits

- The profile covers only the built-in ST7789, panel-control GPIOs, SPI2, and
  UART0 TX. ESP-IDF boot, flash, PSRAM, Wi-Fi/BLE, USB, buttons, and I²C
  accessories are outside the profile.
- The ESP32-S3 SYSTEM clock/reset window remains an explicit placeholder. The
  smoke leaves those registers at their reset values and does not claim dynamic
  peripheral clock or reset-gate behavior.
- The smoke paints controller frame memory and checks the declared glass crop.
  It does not model analog color, viewing angle, backlight brightness, or a
  physical panel capture.
- The system uses the S3 GP-SPI2 W-buffer path with up to 64 bytes per
  transaction. SPI DMA and higher-level display libraries are not covered.
- GPIO21 is modelled as an active-high panel/STEMMA rail gate. Other loads on
  that shared rail are not attached in this board profile.
- The current raw firmware smoke shortens panel startup waits for simulation;
  it verifies command ordering and the model response, not a measured cold
  start interval on hardware.
