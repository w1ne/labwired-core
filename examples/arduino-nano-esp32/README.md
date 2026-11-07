# Arduino Nano ESP32 RGB LED smoke

This smoke targets the Arduino Nano ESP32 and toggles its three active-low
onboard RGB LED contacts on ESP32-S3 GPIO46, GPIO0, and GPIO45. Arduino's
current store listing is the headered SKU ABX00092; the profile follows the
documented Nano ESP32 GPIO mapping.
The system profile is
[`configs/systems/arduino-nano-esp32.yaml`](../../configs/systems/arduino-nano-esp32.yaml)
and names only the real ESP32-S3 GPIO model needed for this check.

## Scope

- The repository smoke runs `firmware/board_gpio_smoke.c`, a minimal bare-metal
  S3 image that configures GPIO0, GPIO45, and GPIO46 and repeatedly toggles all
  three active-low LED contacts. The test requires at least two transitions on
  each pad.
- PlatformIO also builds `src/main.ino`, an Arduino sketch that drives all
  three contacts low. The hosted verifier checks each final GPIO state and
  returns **model-verified**; neither result is a physical-board test.
- The local repository smoke uses the S3 fast-boot ELF path. Full Arduino
  framework startup through the mask ROM is outside this profile.
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
| D9 | 18 | D10 | 21 |
| D11 / COPI | 38 | D12 / CIPO | 47 |

The onboard active-low RGB LED uses GPIO46, GPIO0, and GPIO45:

| Arduino core alias | ESP32-S3 GPIO | Notes |
|---|---:|---|
| `LED_RED` | 46 | Red contact |
| `LED_GREEN` | 0 | Some early boards swap the green and blue contacts |
| `LED_BLUE` | 45 | Some early boards swap the green and blue contacts |

The RGB contacts are active-low. Arduino reports that some early boards have
the green and blue contacts swapped, so the profile and local smoke identify
GPIO0 and GPIO45 by pad instead of assigning them a guaranteed color. A hosted
probe using Arduino's `LED_BUILTIN` alias did not drive the documented
D13/GPIO48 high, so the smoke addresses the physical GPIO numbers directly;
the hosted Arduino board-alias mapping remains an open model gap.

## Evidence

- [`evidence/vendor-status.md`](evidence/vendor-status.md) records the live
  manufacturer lifecycle and orderability checks.
- [`evidence/part-model.json`](evidence/part-model.json) records the hosted
  board identity and the unresolved hosted-versus-vendor I²C pin map.
- [`evidence/alias-pin-probe.json`](evidence/alias-pin-probe.json) records the
  failed hosted `LED_BUILTIN` alias probe that led the smoke to address the
  physical GPIO numbers directly.
- [`evidence/verify-request.json`](evidence/verify-request.json) records the
  board, source, diagram, and three GPIO oracle clauses.
- [`evidence/model-verification.json`](evidence/model-verification.json) is the
  hosted LabWired verification result.
- [`firmware/board_gpio_smoke.c`](firmware/board_gpio_smoke.c) is the local
  fast-boot smoke firmware; [`src/main.ino`](src/main.ino) is the hosted
  Arduino firmware proof source.

## Reproduce the repository smoke

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id arduino-nano-esp32 \
  --script examples/arduino-nano-esp32/onboarding-smoke.yaml \
  --system configs/systems/arduino-nano-esp32.yaml \
  --firmware-build-script examples/arduino-nano-esp32/ci/build-firmware.sh \
  --out-dir examples/arduino-nano-esp32/evidence
```

The PlatformIO environment is pinned to Espressif32 8.0.0 and the
`arduino_nano_esp32` board definition. It builds the Arduino sketch and uses
the same pinned Xtensa toolchain for the small local smoke image. The committed
build and simulator logs are under [`evidence/logs/`](evidence/logs/).
