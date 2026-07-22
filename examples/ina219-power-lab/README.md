# INA219 Power Lab

STM32F103 + INA219 high-side current / bus voltage monitor over I²C1.

```bash
cargo build -p ina219-power-lab --release --target thumbv7m-none-eabi
cp target/thumbv7m-none-eabi/release/ina219-power-lab \
  ../packages/playground/public/wasm/demo-ina219-power-lab.elf
```
