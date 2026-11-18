# NUCLEO-H533RE validation scope

The deterministic simulator smoke boots a Cortex-M33 image, reads BME280
`CHIP_ID` (`0x60`) over I2C1 on PB6/PB7, transmits the success marker through
USART2, and toggles LD2 on PA5. The board descriptor models RCC, GPIOA, GPIOB,
USART2, I2C1, and physical pins PA2/PA3/PA5/PB6/PB7. GPIO ports C-F, other
USARTs, timers, ADC, SPI, USB, radio, ST-LINK debugger behavior, and the rest of
the MCU are not modeled by this profile.

The BME280 model provides deterministic register responses. This smoke does not
verify electrical levels, physical pin muxing, the attached breakout, the
sensor's environmental conversion timing, or live temperature/pressure/humidity
measurements.
The firmware's `TIMINGR` value is used to exercise the H5 transaction model;
the local result does not establish an SCL rate on physical H533RE hardware.

The H5 RCC/GPIO/USART/I2C models are reused from the existing H563 profile. This
H533RE board profile is **SIM-DERIVED**; no H533RE silicon capture or physical
board run is available. The local CLI smoke is the committed evidence and is
not a hosted `model_verified` result.
