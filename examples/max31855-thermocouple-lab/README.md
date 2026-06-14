# MAX31855 Thermocouple Lab

This firmware reads a MAX31855 cold-junction-compensated K-type thermocouple
converter over LabWired's simulated SPI1 path on STM32F103. Each cycle clocks
out the 32-bit MAX31855 frame and decodes the 14-bit thermocouple temperature
(Q4 °C), the 12-bit internal/cold-junction temperature, and the fault bits.

Run from the repo root:

```bash
cargo build -p max31855-thermocouple-lab --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test --script examples/max31855-thermocouple-lab/io-smoke.yaml
```

Expected UART begins with:

```text
MAX31855 Thermocouple Lab
word=0x... TC_q4=... INT_q12=... FAULT=...
```

The thermocouple model attaches over SPI1 with chip-select on `PA4`
(`device_type: max31855` in `system.yaml`).
