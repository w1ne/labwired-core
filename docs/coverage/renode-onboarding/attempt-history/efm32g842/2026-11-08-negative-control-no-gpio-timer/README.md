# Negative control — EFM32G842 GPIO/TIMER smoke on the USART0-only descriptor

Round 384 firmware (`firmware-efm32g842-smoke`) run against the pre-round
descriptor (`chip-usart0-only.yaml`: USART0 only, no GPIO ports or TIMERs).
The banner prints, then the first GPIO access faults:
BusFault @0x4000604C (GPIO port C MODEL), 2/5 checks. The pass on the
round-384 descriptor therefore depends on the `efm32s0_gpioport` /
`efm32s0_timer` models, not on zero-answering windows.
