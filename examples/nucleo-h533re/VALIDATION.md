# NUCLEO-H533RE validation scope

The deterministic simulator smoke boots a Cortex-M33 image, transmits
`NUCLEO_H533RE_OK` through USART2, and toggles LD2 on PA5. The board descriptor
only maps RCC, GPIOA, USART2, and physical pins PA2/PA3/PA5. GPIO ports B-F,
other USARTs, timers, ADC, SPI, I2C, USB, radio, ST-LINK debugger behavior, and
the rest of the MCU are not modeled by this profile.

The H5 RCC/GPIO/USART models are reused from the existing H563 profile. This
H533RE board profile is **SIM-DERIVED**; no H533RE silicon capture or physical
board run is available. The local CLI smoke is the committed evidence and is
not a hosted `model_verified` result.
