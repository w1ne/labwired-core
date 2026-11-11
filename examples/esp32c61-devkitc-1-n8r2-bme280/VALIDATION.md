# Validation: ESP32-C61-DevKitC-1-N8R2 + BME280

The committed smoke builds `firmware-esp32c61-bme280-smoke` for
`riscv32imc-unknown-none-elf`. It asserts a UART0 marker after firmware reads
BME280 ID `0x60` and the model's raw-temperature test vector `0x082DC0` over
I2C0. The sensor is connected at `0x77`, with SDA GPIO6 and SCL GPIO22. Evidence
is committed under [`evidence/`](evidence/).

The descriptor includes C61 I2C0 command-list behavior, the C61 PCR I2C clock
gate, the GPIO matrix I2C0 subset, and register storage for IO_MUX. IO_MUX pad
electrical behavior and pull-ups are not modeled. The model does not claim
Wi-Fi, Bluetooth, native USB, PSRAM, flash boot behavior, I2C timing at the
electrical level, or real BME280 sampling and compensation. This is simulator
evidence, not a physical-board run.
