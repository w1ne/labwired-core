# Hosted LabWired verification — BME280 model

On 2026-11-10, hosted LabWired returned `status: model_verified`, with the
oracle and run both passing for an STM32F103 I2C transaction to a BME280 at
address `0x77`. The test read register `0xD0` and observed ID `0x60`.

The hosted diagram used the `stm32f103-blinky` board (`stm32-dev` MCU model),
PB6/PB7 as I2C1 SCL/SDA, 4.7 kΩ pull-ups to VCC, and SDO tied to VCC to select
`0x77`. The hosted firmware was an Arduino `Wire` sketch. This validates the
hosted BME280 device model and address path. It did not run the Rust firmware
or the exact narrow Blue Pill profile in this repository; the repository
firmware's evidence is the local smoke run in `result.json` and `uart.log`.

The hosted result reported two approximations: peripheral time derives from
the configured 72 MHz CPU frequency and does not track PLL reconfiguration;
the MCU had no routed power net, so its 3.3 V supply was idealized and power-on
or brown-out reset was not simulated.
