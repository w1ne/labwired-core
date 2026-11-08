# NUCLEO-F767ZI + ST LSM6DSOX

This example adds ST's LSM6DSOX accelerometer to the already-supported
NUCLEO-F767ZI as an externally wired customer design. The descriptor maps
I2C1 to PB8/PB9 (Nucleo-144 D15/D14); the sensor uses address `0x6A` with
SDO/SA0 tied low. The firmware brings up I2C1, reads `WHO_AM_I`, enables the
416 Hz / ±2 g accelerometer setting, waits for XLDA, burst-reads all three
signed axes, prints a pass marker on USART3, and turns on LD1 only on a match.

The model is intentionally a narrow accelerometer slice: identity, CTRL1_XL,
a fixed 416 Hz data-ready interval, and scaled XYZ sample inputs. Gyroscope,
temperature, FIFO, interrupts, embedded functions, SPI, I3C, electrical
behavior, and the other ODR cadences are not modeled. The descriptor therefore
does not claim those paths.

For physical wiring, use a carrier for the LGA-14 sensor: connect VDD and
VDDIO to a suitable 3.3 V rail, common the grounds, pull CS high for I2C,
strap SDO/SA0 low, and connect SDA/SCL to PB9/PB8 respectively. Add external
I2C pull-ups appropriate to the bus capacitance; the model does not simulate
electrical pull-ups. The [ST datasheet](https://www.st.com/resource/en/datasheet/lsm6dsox.pdf)
defines the package and power requirements.

The local CLI smoke passed and its build/run evidence is committed in
[`evidence/`](evidence/). The hosted verifier catalog does not include the
STM32F767ZI target, so this is not labeled as a hosted `labwired_verify`
verdict or a physical bench test. Current lifecycle and distributor
availability are recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

Reproduce from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f767zi-lsm6dsox \
  --crate nucleo-f767zi-lsm6dsox-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f767zi-lsm6dsox/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f767zi-lsm6dsox.yaml \
  --out-dir examples/nucleo-f767zi-lsm6dsox/evidence
```
