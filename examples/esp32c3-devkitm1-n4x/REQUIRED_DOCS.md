# Source documents — ESP32-C3-DevKitM-1-N4X

- Espressif's [live ESP32-C3-DevKitM-1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c3/esp32-c3-devkitm-1/user_guide.html)
  lists the exact N4X variant, its 4 MB flash, and the addressable GPIO8 RGB LED.
- Espressif's [board schematic](https://dl.espressif.com/dl/schematics/SCH_ESP32-C3-DEVKITM-1_V1_20200915A.pdf)
  labels the RGB LED `SK68XXMINI-HS` (D6) and connects its data input to GPIO8.
- OPSCO's [SK6812MINI-HS datasheet](https://42keebs.eu/wp-content/uploads/2023/12/SK6812-MINI-HS.pdf)
  documents the compatible 24-bit GRB single-wire protocol: 800 kHz, 0.3/0.9 µs
  zero timing, 0.6/0.6 µs one timing, and an 80 µs reset-low interval. The
  smoke checks that protocol with the generic NeoPixel model; it does not claim
  the exact LED revision or its electrical behavior.
