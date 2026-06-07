# Tier-1 Validation Matrix

Every cell links the CI run that produced it; no link → `·` unrecorded.

**Confidence tier:** ✅ means *sim-consistent* — the check passed against
the simulator's peripheral models on real firmware. Silicon-anchored
verification (hardware-in-the-loop capture replay) is a separate tier
that arrives with the HIL workstream; no cell currently claims it.

| chip | clock | gpio | uart | timer | dma | irq | adc | i2c | pwm | rmt | rtc | spi | wdt |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| esp32 | · | · | · | · | — | · | — | — | — | · | — | — | — |
| esp32c3 | — | · | · | · | — | · | — | — | — | · | — | — | — |
| esp32s3 | · | · | · | · | · | · | — | · | · | · | — | — | — |
| nrf52832 | — | — | · | — | — | — | — | — | — | · | — | — | — |
| nrf52840 | — | · | · | — | — | — | — | — | — | · | — | · | — |
| rp2040 | — | — | · | — | — | — | — | — | — | · | — | — | — |
| stm32f103 | · | · | · | · | · | — | · | · | — | · | · | · | · |
| stm32f401 | · | · | · | — | — | — | — | · | — | · | — | — | — |
| stm32f407 | · | · | · | — | — | — | — | · | — | · | — | — | — |
| stm32g474re | · | · | · | — | — | — | — | — | — | · | — | — | — |
| stm32h563 | · | · | · | — | · | — | — | — | — | · | — | — | — |
| stm32l073 | · | · | · | · | · | · | · | · | — | · | · | · | · |
| stm32l476 | · | · | · | · | · | · | · | · | — | · | · | · | · |
| stm32wb55 | · | · | · | — | — | — | — | — | — | · | — | — | — |
| stm32wba52 | · | · | · | — | — | — | — | — | — | · | — | — | — |
