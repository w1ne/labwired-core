# Validation — Arduino Nano RP2040 Connect

The smoke builds bare-metal RP2040 firmware, checks GP0 output edges, and reads
the onboard LSM6DSOX through I2C0. It asserts the sensor ID, accelerometer
configuration, data-ready status, XYZ sample and the max-step stop. The
NINA-W102 and other omitted parts are outside scope; see
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).

Run the command in [`README.md`](README.md) from the repository root. The
build logs, local CLI smoke result, GPIO edge capture and IMU UART evidence are
committed in [`evidence/`](evidence/).

The hosted verifier could not verify this board-specific Rust image; its
fallback attempts and exact outcomes are recorded in
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).
