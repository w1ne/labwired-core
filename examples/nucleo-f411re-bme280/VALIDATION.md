# Validation record

The onboarding smoke builds the pinned PlatformIO `nucleo_f411re` Arduino
target and runs the resulting firmware against
`configs/systems/nucleo-f411re-bme280.yaml`.

The acceptance checks require both `BME280_OK` and the deterministic
temperature, pressure, and humidity line. This means the sketch completed its
sensor probe and read the modeled measurement registers over I2C1. The build,
local smoke transcript, result JSON, snapshot, and scoreboard are committed
under [`evidence/`](evidence/).

This is passing repository smoke evidence, not a measurement from physical
hardware. The hosted `labwired_verify` catalog currently has no NUCLEO-F411RE
target, so this round does not carry a separate `model_verified` verdict.
