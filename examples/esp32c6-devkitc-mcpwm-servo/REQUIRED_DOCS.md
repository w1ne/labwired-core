# Required source documents

- [ESP32-C6-DevKitC-1 v1.2 user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/user_guide.html): current N8 board, header access, retail availability, and GPIO8 onboard RGB LED.
- [ESP32-C6 GPIO signal map](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c6/include/soc/gpio_sig_map.h): `PWM0_OUT0A_IDX = 87` through `PWM0_OUT2B_IDX = 92`.
- Vendored `tests/fixtures/real_world/esp32c6.svd`: MCPWM0 base, register offsets/fields, and interrupt source 61.
- ESP32-C6 Technical Reference Manual, MCPWM chapter: timer and operator behavior used by the narrow PWM profile.
- TowerPro SG90 calibration reference used by the existing servo model: 0.5–2.4 ms pulse range.
