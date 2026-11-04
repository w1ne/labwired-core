# Adafruit ESP32 Feather V2 GPIO smoke

This smoke boots Arduino firmware on the Adafruit ESP32 Feather V2 (PID 5400)
and toggles its discrete user LED. The board profile maps that active-high LED
to ESP32 GPIO13. The LED pin, rather than serial output, is the acceptance
signal because the classic ESP32 hosted profile does not capture UART output.

## Scope

- The committed PlatformIO firmware is built for `adafruit_feather_esp32_v2`
  and drives GPIO13 through Espressif's GPIO API.
- The local LabWired CLI test requires at least 16 GPIO13 edges before its step
  budget ends. This proves the board's modeled GPIO-to-LED path; it is not a
  physical-board test.
- The board's 2 MiB PSRAM is represented only by the classic ESP32 model's
  probe-only memory stub. The boot log therefore reports a PSRAM ID read error;
  the firmware continues without using external memory. PSRAM data behavior,
  Wi-Fi/Bluetooth, the USB-to-UART bridge, STEMMA QT power gating, I2C sensors,
  and the NeoPixel are outside this smoke. The local result also records that
  the twin assumes an ideal 3.3 V rail; power-on and brown-out behavior are not
  tested.
- The hosted `labwired_verify` service currently returns `unsupported` for this
  classic ESP32 target: after its GPIO oracle becomes true, the run faults on
  unmapped address `0x3fc9b2b4` (an S3-range address, not in the classic ESP32
  memory map). That result is preserved in
  [`evidence/hosted-verifier-probe.json`](evidence/hosted-verifier-probe.json).
  The repository's local CLI smoke is the acceptance evidence for this board
  profile; no hosted model-verified claim is made.

## Reproduce

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-esp32-v2 \
  --script examples/adafruit-feather-esp32-v2/io-smoke.yaml \
  --system configs/systems/adafruit-feather-esp32-v2.yaml \
  --out-dir examples/adafruit-feather-esp32-v2/evidence \
  --firmware-build-script scripts/build-adafruit-feather-esp32-v2-smoke.sh
```

The `RUSTC_WRAPPER=` prefix bypasses the missing `sccache` configured globally
in the worktree used to produce the committed log; omit it on hosts where the
configured wrapper is installed.

The build removes Espressif's `-mfix-esp32-psram-cache-issue` code-generation
workaround because the classic ESP32 CPU model does not implement that
workaround's instruction sequence. The firmware does not access PSRAM.

## Pin map and evidence

The board-specific map is in
[`configs/systems/adafruit-feather-esp32-v2.yaml`](../../configs/systems/adafruit-feather-esp32-v2.yaml):
GPIO13 is the active-high red user LED; GPIO2 is the STEMMA QT/NeoPixel power
switch and is not modeled as an LED. The firmware source is
[`firmware/src/main.cpp`](firmware/src/main.cpp), and the test script is
[`io-smoke.yaml`](io-smoke.yaml).
Adafruit's [official pinout guide](https://learn.adafruit.com/adafruit-esp32-feather-v2/pinouts)
lists the red LED on pin 13, STEMMA QT SDA/SCL on GPIO22/GPIO20, and the
NeoPixel/I2C power switch on pin 2.

- [`evidence/vendor-status.md`](evidence/vendor-status.md) records current
  vendor lifecycle and stock information.
- [`evidence/part-model.json`](evidence/part-model.json) records the hosted
  catalog identity, chip, and board-specific pins used in this profile.
- [`evidence/hosted-verifier-probe.json`](evidence/hosted-verifier-probe.json)
  preserves the separate hosted verifier's unsupported result.
- [`evidence/onboarding-metrics.json`](evidence/onboarding-metrics.json),
  [`evidence/logs/`](evidence/logs/), and
  [`evidence/simulation-output/`](evidence/simulation-output/) hold the local
  build and engine-test artifacts.
