# NUCLEO-F756ZG validation

The committed smoke uses `scripts/onboarding_smoke.sh` to build
`firmware-nucleo-f756zg-smoke` and boot it against
`configs/systems/nucleo-f756zg.yaml`. Passing requires the USART3 marker, at
least 16 GPIO transitions on `gpiob:7`, and the expected simulator step limit.

The board profile is intentionally limited to the RCC, GPIOB/GPIOD, USART3,
and SysTick models used by the smoke. Ethernet, USB OTG, crypto, and the other
MCU peripherals are outside the profile. The simulation assumes ideal board
power rails; it does not model power sequencing or brown-out reset.
