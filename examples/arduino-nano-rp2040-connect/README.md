# Arduino Nano RP2040 Connect (RP2040)

This profile covers two real paths on the ABX00052: the exposed header TX pin
(RP2040 GP0), and the onboard ST LSM6DSOX accelerometer. The firmware toggles
GP0 through SIO, then reads the IMU's identity, configures its accelerometer,
waits for data-ready and checks XYZ samples over I2C0 on GP12/GP13 (Arduino
A4/A5).

## Run

```bash
scripts/onboarding_smoke.sh \
  --target-id arduino-nano-rp2040-connect \
  --crate firmware-nano-rp2040-connect-smoke \
  --target thumbv6m-none-eabi \
  --script examples/arduino-nano-rp2040-connect/io-smoke.yaml \
  --system configs/systems/arduino-nano-rp2040-connect.yaml \
  --out-dir examples/arduino-nano-rp2040-connect/evidence
```

The committed `evidence/` directory contains the firmware build and local
simulator smoke logs. Live board and sensor status are in
[`evidence/vendor-status.md`](evidence/vendor-status.md); hosted verifier
results and their limits are recorded in
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).

## Scope

The LSM6DSOX accelerometer slice and GP0 SIO/UART path are modeled. Wi-Fi and
Bluetooth, microphone, crypto device, RGB LED, USB, and QSPI storage are outside
this profile; see
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
