# LabWired verification — NUCLEO-U545RE-Q + BME280

The repository onboarding smoke compiled and ran the committed Rust firmware
against `configs/systems/nucleo-u545re-bme280.yaml`. It passed both assertions:

- USART1 contained `NUCLEO_U545RE_BME280_OK=0x60` after the I2C1 chip-ID read.
- LD2 on PA5 produced at least two GPIO edges.

The hosted LabWired oracle independently returned `status: model_verified` for
the same board, BME280 at `0x77`, and PB6/PB7 I2C1 wiring. That hosted check used
an Arduino `Wire` sketch because the hosted NUCLEO-U545RE toolchain currently
supports Arduino, not the committed Rust binary. Its serial oracle matched
`NUCLEO_U545RE_BME280_OK=0x60` and stopped with `assertions_passed`.
The checked sketch is [`arduino-wire-oracle.ino`](arduino-wire-oracle.ino).

The hosted result also reported two non-blocking model approximations: device
time is derived from the configured 160 MHz `cpu_hz` because PLL changes are
not tracked, and the MCU supply is treated as an ideal rail because no power
net is routed. These do not affect the serial/I2C verdict; power-on, brown-out,
and runtime clock-tree behavior are outside this smoke's coverage.

This is simulator evidence, not hardware confirmation.
