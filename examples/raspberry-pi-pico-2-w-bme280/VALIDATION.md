# Validation — Raspberry Pi Pico 2 W + BME280

The committed `evidence/` directory records the local build and simulator
smoke. The assertions check the BME280 chip ID, the model's raw-temperature
sample, and the firmware completion marker. This is a local engine smoke; it
is not a physical-board capture or a hosted `labwired_verify` verdict.

Run the command from the example's [README](README.md) at the repository root.
