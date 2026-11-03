# Arduino Nano RP2040 Connect (RP2040)

This profile covers the exposed header TX pin, RP2040 GP0, through IO_BANK0
and SIO. The smoke firmware toggles GP0 and the local simulator asserts at
least eight output edges.

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
simulator smoke logs. The live board availability check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md); hosted verifier
results and their limits are recorded in
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).

## Scope

Only header TX / GP0 is modeled. Wi-Fi and Bluetooth, the onboard IMU and
microphone, RGB LED, USB, and QSPI storage are outside this profile; see
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
