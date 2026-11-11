# Source notes

- Raspberry Pi, [Pico microcontroller boards](https://www.raspberrypi.com/documentation/microcontrollers/pico-series.html): Pico W uses RP2040, has the Pico W board layout, and connects its onboard LED through `WL_GPIO0`.
- Raspberry Pi, [Pico W product brief](https://pip-assets.raspberrypi.com/categories/686-raspberry-pi-pico-w/documents/RP-008313-DS-1-pico-w-product-brief): Pico W production lifetime through at least January 2036.
- Raspberry Pi, [Pico W datasheet](https://datasheets.raspberrypi.com/picow/pico-w-datasheet.pdf): CYW43439-connected wireless board signals and board details.
- DigiKey, [Raspberry Pi SC0918 listing](https://www.digikey.com/en/products/detail/raspberry-pi/SC0918/16627943): active buyable board status; stock is recorded separately in [vendor-status.md](vendor-status.md) because it changes.
- Existing LabWired model evidence: `configs/systems/rp2040-pico-bme280.yaml` and `examples/rp2040-pico-bme280/` exercise the RP2040 I2C0 BME280 path at address `0x77` on GP4/GP5.

The Pico W profile reuses the existing RP2040 model. It does not model the CYW43439 radio or Pico W's radio-driven LED.
