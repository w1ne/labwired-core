# Arduino Nano ESP32 GPIO smoke

This smoke runs on the Arduino Nano ESP32 (ABX00083) profile and drives the
onboard RGB LED's red channel through ESP32-S3 GPIO46. GPIO46 is active-low.
The system profile is
[`configs/systems/arduino-nano-esp32.yaml`](../../configs/systems/arduino-nano-esp32.yaml)
and names only the real ESP32-S3 GPIO model needed for this check.

## Scope

- The committed firmware uses ESP-IDF's GPIO API to drive physical GPIO46 low.
- The required oracle checks that GPIO46 ends low. The hosted result is
  **model-verified** for that GPIO state; it is not a physical-board test.
- The hosted twin assumes an ideal 3.3 V rail, so power-on and brown-out reset
  behavior are outside this result.
- The Nano's Wi-Fi/Bluetooth, PSRAM, native USB, ADC, timer, I²C and SPI paths
  are outside this smoke. In particular, the hosted catalog reports I²C on
  GPIO8/GPIO9, while Arduino's published Nano pinout maps A4/A5 I²C to
  GPIO11/GPIO12. The local profile makes no I²C claim pending resolution of
  that mapping discrepancy.

## Pin map

The header map below follows Arduino's
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
| D12 | 47 |  |  |

The RGB channels are active-low. A hosted probe using Arduino's `LED_BUILTIN`
alias did not drive the documented D13/GPIO48 high, so the smoke addresses the
physical GPIO46 directly. The accepted oracle is tied to that documented pad;
the hosted Arduino board-alias mapping remains an open model gap.

## Evidence

- [`evidence/vendor-status.md`](evidence/vendor-status.md) records the live
  manufacturer lifecycle and orderability checks.
- [`evidence/part-model.json`](evidence/part-model.json) records the hosted
  board identity and the unresolved hosted-versus-vendor I²C pin map.
- [`evidence/alias-pin-probe.json`](evidence/alias-pin-probe.json) records the
  failed hosted `LED_BUILTIN` alias probe that narrowed the smoke to GPIO46.
- [`evidence/verify-request.json`](evidence/verify-request.json) records the
  board, source, diagram, and GPIO oracle.
- [`evidence/model-verification.json`](evidence/model-verification.json) is the
  hosted LabWired verification result.
- [`src/main.ino`](src/main.ino) is the tested firmware source.
