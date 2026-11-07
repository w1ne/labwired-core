# NUCLEO-U3C5ZI-Q smoke validation

- Target: ST NUCLEO-U3C5ZI-Q / STM32U3C5ZIT6Q
- Vendor status and availability: [live status receipt](evidence/vendor-status.md)
- Smoke: reset into flash, enable GPIOA, configure PA5, and require at least 16
  LD1 GPIO edges
- Scope: local LabWired CLI simulator; LD1 GPIO only
- Known gaps: VCP/UART, USB, FDCAN, I3C, timers, HSP, buttons, and remaining
  MCU peripherals are not modeled by this focused profile
- Limitation: ideal VDD is assumed; power-on and brown-out behavior are not
  simulated

Build log, smoke log, and simulator output are committed under [`evidence/`](evidence/).
