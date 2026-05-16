# ADXL345 Sensor Lab

This firmware reads the ADXL345 device ID and X/Y/Z acceleration registers through LabWired's simulated I2C1 path on STM32F103.

Run from `core/`:

```bash
cargo build -p adxl345-sensor-lab --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7m-none-eabi/release/adxl345-sensor-lab \
  --system examples/adxl345-sensor-lab/system.yaml \
  --max-steps 200000
```

Expected UART begins with:

```text
ADXL345 Sensor Lab
DEVID=0xE5
X=
```
