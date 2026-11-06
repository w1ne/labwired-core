# Adafruit ESP32 Feather V2

These examples boot Arduino firmware on the Adafruit ESP32 Feather V2 and
exercise two board-specific paths: the discrete GPIO13 user LED and a BME280
on the STEMMA QT I2C connector. The sensor smoke enables the connector's
switched 3.3 V rail with GPIO2 before reading I2C0 on SDA=GPIO22 / SCL=GPIO20.
The Arduino board variant normally enables that rail during initialization;
the smoke switches it off and on to verify both states.

## Scope

- The GPIO smoke requires at least 16 GPIO13 edges.
- The BME280 smoke checks for an address NACK with the rail off, then checks the
  sensor ID and raw temperature after firmware raises GPIO2.
- Both local LabWired CLI runs are acceptance evidence for the configured twin,
  not physical-board tests.
- PSRAM is a probe-only memory stub, so the boot log reports a PSRAM ID error
  and continues without external memory. PSRAM data behavior, Wi-Fi/Bluetooth,
  the USB-to-UART bridge, and NeoPixel signaling remain outside these profiles.
  The twin assumes an ideal 3.3 V rail; power-on and brown-out behavior are not
  tested.
- The hosted `labwired_verify` service currently returns `unsupported` for the
  classic ESP32 target after the GPIO oracle succeeds, faulting at unmapped
  address `0x3fc9b2b4`. That result is preserved in
  [`evidence/hosted-verifier-probe.json`](evidence/hosted-verifier-probe.json).
  No hosted model-verified claim is made.

## Reproduce

From the repository root, run the GPIO smoke:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-esp32-v2 \
  --script examples/adafruit-feather-esp32-v2/io-smoke.yaml \
  --system configs/systems/adafruit-feather-esp32-v2.yaml \
  --out-dir examples/adafruit-feather-esp32-v2/evidence \
  --firmware-build-script scripts/build-adafruit-feather-esp32-v2-smoke.sh
```

Run the BME280 smoke with:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-esp32-v2-bme280 \
  --script examples/adafruit-feather-esp32-v2/bme280-smoke.yaml \
  --system configs/systems/adafruit-feather-esp32-v2-bme280.yaml \
  --out-dir examples/adafruit-feather-esp32-v2/evidence/bme280 \
  --firmware-build-script scripts/build-adafruit-feather-esp32-v2-bme280.sh
```

The `RUSTC_WRAPPER=` prefix bypasses the missing `sccache` configured globally
in this worktree; omit it on hosts where the configured wrapper is installed.
Both PlatformIO environments remove Espressif's PSRAM cache workaround because
the classic ESP32 CPU model does not implement its instruction sequence. These
smokes do not access PSRAM.

## Pin map and evidence

The GPIO-only map is in
[`configs/systems/adafruit-feather-esp32-v2.yaml`](../../configs/systems/adafruit-feather-esp32-v2.yaml).
The sensor map is in
[`configs/systems/adafruit-feather-esp32-v2-bme280.yaml`](../../configs/systems/adafruit-feather-esp32-v2-bme280.yaml).
The firmware sources are [`firmware/src/main.cpp`](firmware/src/main.cpp) and
[`firmware/src/bme280_smoke.cpp`](firmware/src/bme280_smoke.cpp); the smoke
scripts are [`io-smoke.yaml`](io-smoke.yaml) and
[`bme280-smoke.yaml`](bme280-smoke.yaml).

Adafruit's [official pinout guide](https://learn.adafruit.com/adafruit-esp32-feather-v2/pinouts)
lists the red LED on GPIO13, STEMMA QT SDA/SCL on GPIO22/GPIO20, and the shared
NeoPixel/I2C power switch on GPIO2. The BME280 is modeled as an external
STEMMA QT device; NeoPixel signaling is not part of this example.

- [`evidence/vendor-status.md`](evidence/vendor-status.md) records current
  lifecycle and stock information.
- [`evidence/part-model.json`](evidence/part-model.json) records the board's
  hosted catalog identity and pin map.
- [`evidence/hosted-verifier-probe.json`](evidence/hosted-verifier-probe.json)
  preserves the hosted verifier's unsupported result.
- [`evidence/onboarding-metrics.json`](evidence/onboarding-metrics.json),
  [`evidence/logs/`](evidence/logs/), and
  [`evidence/simulation-output/`](evidence/simulation-output/) contain the
  GPIO smoke artifacts. The BME280 run writes to
  [`evidence/bme280/`](evidence/bme280/).
