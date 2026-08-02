# NTC Thermistor Lab

This firmware samples an NTC thermistor on ADC1 channel 0 through LabWired's
simulated ADC path on STM32F103, printing the raw 12-bit conversion result each
iteration. It is the minimal "analog sensor" example — no I²C/SPI bus, just a
single ADC read against a modeled thermistor. The thermistor starts at 25 degC
and is driven at runtime through the standard stimulus API - `set_input` on the
`temperature` channel (degC), from a test script's `stimuli:`, the MCP, or the
WASM bridge.

Run from the repo root:

```bash
cargo build -p ntc-thermistor-lab --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test --script examples/ntc-thermistor-lab/io-smoke.yaml
```

Expected UART begins with:

```text
NTC Thermistor Lab
ADC1 ch0 -> 12-bit count (0..4095)
[NTC] iter=... adc=.../4095
```

The thermistor model attaches as an `adc_input` on ADC1 channel 0
(`device_type: ntc-thermistor` in `system.yaml`).
