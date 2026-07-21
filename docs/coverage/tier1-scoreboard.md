# Tier-1 Validation Matrix

Each cell shows its committed status (gated by the Tier-1 ratchet on every
change; the full sim suite runs nightly) and links the CI run that recorded
it when one is available.

**Confidence tier:** ✅ means *sim-consistent* — the check passed against
the simulator's peripheral models on real firmware. Silicon-anchored
verification (hardware-in-the-loop capture replay) is a separate tier
that arrives with the HIL workstream; no cell currently claims it.

**Legend:** ✅ passed · 🟡 partial · ⛔ modeled but failing · 🚧 not
modeled yet · · no check written. Every column here is a subsystem that
exists in silicon on all listed parts, so 🚧 is an honest model gap, not
an "N/A".

| chip | clock | gpio | uart | timer | dma | irq | i2c | spi | adc | pwm | wdt | rtc |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| ESP32 (Xtensa LX6) | ✅ | ✅ | ✅ | ✅ | 🚧 | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 | 🚧 |
| ESP32-C3 (RISC-V) | · | ✅ | ✅ | ✅ | · | ✅ | ⛔ | ✅ | ✅ | ⛔ | 🚧 | 🚧 |
| ESP32-S3 (Xtensa LX7) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 | 🚧 | ✅ | 🚧 | 🚧 |
| nRF52832 | ✅ | ✅ | ✅ | ✅ | 🚧 | 🚧 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| nRF52840 | ✅ | ✅ | ✅ | ✅ | 🚧 | 🚧 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| RP2040 | ✅ | ✅ | ✅ | ✅ | · | 🚧 | ✅ | ✅ | 🚧 | 🚧 | 🚧 | 🚧 |
| STM32F103C8 | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 | ✅ | ✅ | ✅ | 🚧 | ✅ | ✅ |
| STM32F401RE | ✅ | ✅ | ✅ | ✅ | 🚧 | 🚧 | ✅ | ✅ | ✅ | 🚧 | ✅ | ✅ |
| STM32F407VG | ✅ | ✅ | ✅ | ✅ | · | 🚧 | ✅ | ✅ | ✅ | 🚧 | ✅ | ✅ |
| STM32G474RE | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| STM32H563 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| stm32h735 | ✅ | ✅ | ✅ | ✅ | 🚧 | ✅ | ✅ | ✅ | 🚧 | ✅ | ✅ | 🚧 |
| STM32L073RZ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 | ✅ | ✅ |
| STM32L476RG | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 | ✅ | ✅ |
| STM32WB55 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| STM32WBA52 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🚧 | ✅ | ✅ | ✅ |
