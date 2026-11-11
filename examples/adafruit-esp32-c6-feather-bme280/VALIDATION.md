# Validation record

## Demand and scope

ESP32 is demand rank **#1** (38% of respondents). This adds a tested profile
for an exact buyable Feather board and STEMMA QT wiring; the ESP32-C6FH4 and
BME280 models already exist.

## Reproduce

```sh
examples/adafruit-esp32-c6-feather-bme280/firmware/build-firmware.sh
scripts/onboarding_smoke.sh \
  --target-id adafruit-esp32-c6-feather-bme280 \
  --firmware-build-script examples/adafruit-esp32-c6-feather-bme280/firmware/build-firmware.sh \
  --script examples/adafruit-esp32-c6-feather-bme280/bme280-smoke.yaml \
  --system configs/systems/adafruit-esp32-c6-feather-bme280.yaml \
  --out-dir examples/adafruit-esp32-c6-feather-bme280/evidence
```

The smoke must observe BME280 ID `0x60`, raw temperature `0x082DC0`, GPIO20
power-enable activity, and GPIO15 LED edges. Build and run logs, structured
result, metrics and simulator snapshot are committed under `evidence/`.

## Model boundary

The test uses the ESP32-C6FH4 CPU, modeled GPIO/I²C0 peripherals, BME280 device
model and USB Serial/JTAG console. The GPIO and I²C0 paths are behavioral
models, not zero-answering stubs. The C6 LP core, radio, RF protocols, charging,
fuel gauge, on-board AHT20, USB electrical/enumeration behavior, NeoPixel and
power-switch wiring are outside this profile. GPIO20 is driven high by the
firmware before I²C access, but the system descriptor connects the BME280
directly and does not electrically gate it with GPIO20. The smoke is local
engine evidence, not hosted `labwired_verify` evidence, mask-ROM/flash boot, or
a physical-board test. The hosted catalog currently lists ESP32-C6-DevKitC-1
as `design_only`, with no hosted compile/run path.
