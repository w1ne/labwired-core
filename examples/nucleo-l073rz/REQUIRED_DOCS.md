# Required source documents — NUCLEO-L073RZ

Every register address, bit field, and pin assignment in this board's chip
yaml and firmware traces to one of the documents below. They are the
ground truth for any future fidelity work (the board has no silicon
validation yet, so the docs *are* the oracle).

| Doc | ID | Used for | Notes |
|-----|----|----------|-------|
| STM32L0x3 Reference Manual | **RM0367** | Memory map, RCC/GPIO/USART/I2C/SPI/timer register offsets, DBGMCU_IDCODE (§27.4), EXTI line count | Primary register source. ST DocID025942. |
| STM32L073x8/xB/xZ Datasheet | **DS10685** | Flash/RAM/EEPROM sizes, package, pin alternate-function table (USART2 = AF4 on PA2/PA3) | Memory boundaries + AF mapping. |
| STM32 Nucleo-64 User Manual | **UM1724** | LD2 (PA5), B1 (PC13), USART2→ST-LINK VCP wiring, SWD pins | Board-level wiring only. |
| Cortex-M0+ Technical Reference Manual | ARM **DDI 0484** | Core, SysTick, NVIC, no-JTAG/SWD-only debug, no bit-band region | Explains ARMv6-M caveats in VALIDATION.md. |
| ARMv6-M Architecture Reference Manual | ARM **DDI 0419** | Legal Thumb instruction subset (toolchain target `thumbv6m-none-eabi`) | Why the firmware must build for thumbv6m. |

## Probed against silicon (2026-06-03)

- **DBGMCU_IDCODE = `0x20086447`** read over SWD: DEV_ID `0x447`, **REV_ID
  `0x2008`** (the earlier datasheet-guessed `0x1000` was wrong and is now
  corrected in the chip yaml). Core: Cortex-M0+ r0p1. See `VALIDATION.md` §1.

## Open items

- Per-peripheral register fidelity (RCC clock tree, ADC sequencer, LPTIM)
  has not been diffed against silicon — see `VALIDATION.md` "Known fidelity
  limits".
