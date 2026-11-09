# Known limitations — NUCLEO-H533RE

- The chip profile exposes RCC, GPIOA, GPIOB, SysTick, H5 I2C1, and SRAM1 for
  the LD2 and companion BME280 smokes. SRAM2, SRAM3, backup SRAM, and other
  H533 memory/peripheral blocks are not modeled here.
- The STM32H533RET6 TrustZone security attribution and secure boot option
  configuration are not modeled or verified.
- The board's button, STLINK-V3EC virtual COM port, USB device path, and other
  board connections are omitted.
- The simulator reports `unpowered_rail_assumed`; supply sequencing and
  brown-out reset behavior are not covered.
- Evidence is from the local LabWired engine only, not a hardware capture.
