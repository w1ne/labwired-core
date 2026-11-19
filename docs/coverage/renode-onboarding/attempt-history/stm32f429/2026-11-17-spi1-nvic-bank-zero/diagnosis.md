# STM32F429 SPI1 first-run diagnosis

The imported system booted and SPI1 returned the expected MAX31855 frame
`01901600`. The UART output reported `txeirq=0` and `rxneirq=0`; the smoke
read NVIC ISPR0/ICPR0 even though SPI1 is external IRQ 35 and therefore belongs
to NVIC bank 1. Updating the smoke to read ISPR1/ICPR1 fixed the assertions.
The failed JUnit, UART, result, snapshot, metrics, and stage logs are retained
beside this note. The passing rerun is in `docs/coverage/renode-onboarding/metrics/stm32f429/`.
