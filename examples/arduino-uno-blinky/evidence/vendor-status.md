# Arduino Uno R3 / ATmega328P live status

- Checked live: 2026-11-18.
- Arduino's official [Uno Rev3 store page](https://store.arduino.cc/products/arduino-uno-rev3)
  lists SKU `A000066`, identifies the fitted ATmega328P, and showed an **Add to
  cart** option at EUR 20.50 (discounted from EUR 29.30).
- Microchip's ATmega328P status was checked with the real Chrome profile using
  `~/.venvs/mesh/bin/python ~/.hermes/scripts/mc-live-status.py ATmega328P`.
  The live result was **Not Recommended for new designs**.
- This round is maintenance of the existing Uno R3 model and smoke. It does not
  onboard ATmega328P as new hardware or change the chip descriptor.
