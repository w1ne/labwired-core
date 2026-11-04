# NUCLEO-C542RC smoke validation

- Target: ST NUCLEO-C542RC / STM32C542RCT6
- Vendor status and buyability: [live status receipt](evidence/vendor-status.md)
- Smoke: reset into flash, enable GPIOA, configure PA5, and produce at least 16
  LD1 GPIO edges
- Scope: local LabWired CLI simulator; LED GPIO only
- Known gaps: SRAM2, buttons, VCP/UART, USB, FDCAN, I3C, and all other
  peripherals are not modeled by this focused board profile
- Limitation: simulator evidence assumes ideal VDD and does not cover reset
  sequencing or physical hardware

Run log and simulator output are committed under [`evidence/`](evidence/).
