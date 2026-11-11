# NUCLEO-G491RE + BME280 validation

This local CLI smoke boots a bare-metal Cortex-M4 image on the NUCLEO-G491RE
profile, configures I2C1 on PB8/PB9, reads BME280 `CHIP_ID` (`0xD0`) and raw
temperature (`0xFA..0xFC`) from the attached model at address `0x77`, and pulses
LD2 on PA5 only after the expected values match.

## Run

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g491re-bme280 \
  --crate firmware-nucleo-g491re-bme280-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-g491re-bme280/bme280-smoke.yaml \
  --system configs/systems/nucleo-g491re-bme280.yaml \
  --out-dir examples/nucleo-g491re-bme280/evidence
```

The smoke checks address and register-pointer ACKs, sensor ID `0x60`, all three
raw-temperature bytes (`0x82`, `0xDC`, `0x00`, encoding the 20-bit value
`0x082DC0`), PB8/PB9 edge activity, LD2 GPIO
edges, and the configured max-step stop. The saved evidence currently reports
`PASS 14/14`; inspect `evidence/onboarding-summary.md` and
`evidence/simulation-output/result.json` for the recorded run. The nightly
onboarding workflow includes this target.

## Model boundary

This verifies the committed firmware against the local I2C1 and BME280 models.
The external device is attached directly to the I2C model; the run does not
validate physical pull-ups, voltage levels, cable continuity, or sensor
calibration against a real breakout. The base NUCLEO-G491RE GPIO smoke remains
separate. ST-LINK, USB, analog behavior, and other G491 peripherals are outside
these profiles. The hosted LabWired catalog has no G491 board entry, so there is
no hosted `labwired_verify` verdict for this target.
