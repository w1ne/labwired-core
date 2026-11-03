# Arduino Nano ESP32

The Arduino Nano ESP32 (ABX00083) places a u-blox NORA-W106 module with an
ESP32-S3 in the Nano header footprint. The [Arduino store listing](https://store.arduino.cc/collections/most-popular/products/nano-esp32-with-headers)
showed Add to cart on 2026-11-03; Espressif's [product selector](https://products.espressif.com/product-selector)
lists ESP32-S3R8 as Mass Production.

## Current modeled scope

The narrow system profile at
[`configs/systems/arduino-nano-esp32.yaml`](../../configs/systems/arduino-nano-esp32.yaml)
declares the ESP32-S3 GPIO model and the active-low red RGB LED on GPIO46.
The smoke drives GPIO46 low and the hosted oracle observes that pad state.
This is **model evidence**, not a physical-board test. The hosted twin assumes
an ideal 3.3 V rail, so power-on and brown-out reset behavior are not covered.

The smoke does not claim Wi-Fi/Bluetooth, PSRAM, native USB, ADC, timer, I²C,
or SPI support for this board profile. The hosted catalog's I²C map conflicts
with Arduino's published A4/A5 mapping; those pins are intentionally not
exposed as a verified board path here.

## Header pin map

Mappings follow Arduino's
[Nano ESP32 cheat sheet](https://docs.arduino.cc/tutorials/nano-esp32/cheat-sheet/)
and [official pinout](https://docs.arduino.cc/resources/pinouts/ABX00083-full-pinout.pdf).

| Arduino pin | ESP32-S3 GPIO | Arduino pin | ESP32-S3 GPIO |
|---|---:|---|---:|
| D0 | 44 | A0 | 1 |
| D1 | 43 | A1 | 2 |
| D2 | 5 | A2 | 3 |
| D3 | 6 | A3 | 4 |
| D4 | 7 | A4 / SDA | 11 |
| D5 | 8 | A5 / SCL | 12 |
| D6 | 9 | A6 | 13 |
| D7 | 10 | A7 | 14 |
| D8 | 17 | D13 / SCK LED | 48 |
| D9 | 18 | RGB red / LED_RED (14) | 46 |
| D10 | 21 | RGB green / LED_GREEN (15) | 45 |
| D11 / COPI | 38 | RGB blue / LED_BLUE (16) | 0 |
| D12 / CIPO | 47 |  |  |

The RGB channels are active-low. A hosted probe of Arduino's `LED_BUILTIN`
alias did not drive the documented D13/GPIO48 high; the accepted smoke uses
physical GPIO46 directly. The hosted Arduino alias mapping remains unverified.

The hosted catalog currently reports I²C on
GPIO8/GPIO9, but Arduino's official pinout identifies A4/SDA as GPIO11 and
A5/SCL as GPIO12. No I²C claim is made until that discrepancy is resolved.

## Evidence

See [`examples/arduino-nano-esp32/`](../../examples/arduino-nano-esp32/) for
the firmware, live vendor status, hosted model record, and verification output.
