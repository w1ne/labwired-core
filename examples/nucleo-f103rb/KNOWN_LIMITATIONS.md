# Known limitations — NUCLEO-F103RB

The GPIO/UART smoke passed in the simulator on 2026-10-27. No
physical-board comparison is recorded. Treat the behavior beyond the
documented smoke path and chip geometry as unvalidated.

## Declared board surface

- STM32F103RBT6 identity, 128 KB flash, 20 KB SRAM, and LQFP64 package geometry.
- LD2 on PA5 (active high) and B1 on PC13 (active low).
- USART2 on PA2/PA3, routed to the ST-LINK/V2-1 virtual COM port through the
  default SB13/SB14 solder-bridge configuration.
- The smoke firmware only configures GPIOA/PA5 and USART2, emits `OK`, and
  toggles LD2. It does not read B1.

## Not established

- Physical UART-byte or GPIO-edge parity against an NUCLEO-F103RB.
- Behavior of timers, ADC, DMA, SPI, I2C, USB, CAN, RTC, watchdogs, clock-tree
  switching, or any peripheral outside the smoke.
- Full part-specific silicon behavior. The descriptor uses the shared F103
  medium-density peripheral map; exact geometry does not make that shared
  behavior an STM32F103RB-specific silicon capture.

See [`VALIDATION.md`](VALIDATION.md) for the firmware hash, smoke results, and
unsupported-instruction audit. The physical-board comparison remains open.
