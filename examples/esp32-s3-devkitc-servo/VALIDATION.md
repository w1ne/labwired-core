Run the board smoke from the repository root:

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-s3-devkitc-1-n16r8-v10-servo \
  --script examples/esp32-s3-devkitc-servo/onboarding-smoke.yaml \
  --system configs/systems/esp32-s3-devkitc-1-n16r8-v10-servo.yaml \
  --firmware-build-script examples/esp32-s3-devkitc-servo/ci/build-firmware.sh \
  --out-dir examples/esp32-s3-devkitc-servo/evidence
```

Acceptance covers UART0, a decoded lit pixel on the modeled v1.0 GPIO48 WS2812
LED, LEDC channel 3 register commit, and at least two transitions on the GPIO4 output pad. The
evidence folder contains the firmware build log, simulator log, result JSON,
JUnit, summary, and onboarding metrics. The servo model is a command/angle
digital twin; this profile does not represent its electrical load or mechanics.
