# BlackPill F401CC Board Support Example

## Board Specs
- MCU: STM32F401CC (Cortex-M4, 84MHz)
- Flash: 256 KB
- RAM: 64 KB
- LED: PC13 (active low)
- Button: PA0

## Running the Smoke Test
```bash
cargo build -p firmware-f401-demo --release --target thumbv7em-none-eabi
cargo run -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-f401-demo \
  --system examples/blackpill-f401cc/system.yaml \
  --max-steps 100
```
