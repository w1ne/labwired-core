# Required source documents

- [Raspberry Pi Pico product page](https://www.raspberrypi.com/products/raspberry-pi-pico/): current Pico 1 product, RP2040 and PWM specifications, live reseller selector, and production-through date.
- [Raspberry Pi Pico-series documentation](https://www.raspberrypi.com/documentation/microcontrollers/pico-series.html): Pico 1 variants and RP2040 board capabilities.
- [Raspberry Pi Pico pinout](https://pip-assets.raspberrypi.com/categories/610-raspberry-pi-pico/documents/RP-008309-DS-1-Pico-R3-A4-Pinout.pdf?disposition=inline): GP16 is exposed as physical header pin 21.
- [RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf): PWM slices, channel outputs and divider/wrap behavior.
- `tests/fixtures/real_world/rp2040.svd`: PWM base address and register offsets used by the chip descriptor.
