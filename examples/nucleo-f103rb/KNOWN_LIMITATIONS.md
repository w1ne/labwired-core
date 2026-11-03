# Known limitations — NUCLEO-F103RB

The GPIO/UART smoke is recorded in `evidence/` as deterministic simulator
evidence. No physical-board comparison is recorded. Treat behavior beyond the
documented smoke path and chip geometry as unvalidated.

## Declared board surface

- STM32F103RBT6 identity, 128 KB flash, 20 KB SRAM, and LQFP64 package geometry.
- The narrow board profile models RCC, GPIOA, GPIOC, and USART2 only. USB,
  backup registers, and all other F103 peripherals are omitted from this
  profile; the broader `stm32f103rb.yaml` descriptor is not the board smoke
  system.
- LD2 on PA5 (active high) and B1 on PC13 (active low).
- USART2 on PA2/PA3, routed to the ST-LINK/V2-1 virtual COM port through the
  default SB13/SB14 solder-bridge configuration.
- The smoke firmware only configures GPIOA/PA5 and USART2, emits `OK`, and
  toggles LD2. It does not read B1.

## Not established

- Physical UART-byte or GPIO-edge parity against an NUCLEO-F103RB.
- Behavior of timers, ADC, DMA, SPI, I2C, USB, CAN, RTC, watchdogs, clock-tree
  switching, or any peripheral outside the smoke.
- Physical reset and peripheral parity. The smoke profile reuses F1 register
  models, and its exact geometry does not establish STM32F103RB-specific
  silicon behavior.

See [`VALIDATION.md`](VALIDATION.md) for the firmware hash and smoke results.
The physical-board comparison remains open.
