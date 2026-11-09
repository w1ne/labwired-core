# NUCLEO-H533RE validation

The reproducible local smoke builds `firmware-nucleo-h533re-smoke`, boots it
against the exact `configs/systems/nucleo-h533re.yaml` descriptor, and requires
at least 16 GPIO transitions on PA5 before the configured step limit. The
committed `evidence/` directory contains the build and run logs, JUnit
assertions, simulator result, and onboarding metrics.

The tested LED profile uses the real H5 RCC and GPIO models plus SysTick; it
does not use a zero-answering stub. Its shared chip descriptor also exposes
GPIOB and I2C1 for the BME280 profile documented separately. It maps SRAM1
only. SRAM2, SRAM3, backup SRAM, TrustZone attribution, VCP, the button, USB,
and other H533 peripherals are not modeled by these profiles. This local
simulator evidence is not a physical board capture or a hosted
`model_verified` verdict. The result also records
`unpowered_rail_assumed`, so power-on sequencing and brown-out reset are not
simulated.

Reproduce from the repository root with the command in [README.md](README.md).
ST's live lifecycle and availability evidence is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
