# ADS1115 ADC Lab

STM32F103 + ADS1115 16-bit ADC over I²C1 (AIN0 single-ended).

```bash
cargo build -p ads1115-adc-lab --release --target thumbv7m-none-eabi
cp target/thumbv7m-none-eabi/release/ads1115-adc-lab \
  ../packages/playground/public/wasm/demo-ads1115-adc-lab.elf
```
