# DS3231 RTC Lab

STM32F103 + DS3231 real-time clock over I²C1.

```bash
cargo build -p ds3231-rtc-lab --release --target thumbv7m-none-eabi
cp target/thumbv7m-none-eabi/release/ds3231-rtc-lab \
  ../packages/playground/public/wasm/demo-ds3231-rtc-lab.elf
```
