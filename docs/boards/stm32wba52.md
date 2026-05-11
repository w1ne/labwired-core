# STM32WBA52 (NUCLEO-WBA52CG)

The STM32WBA52CG (Cortex-M33 with TrustZone, 1 MB flash, 128 KB SRAM,
2.4 GHz radio) is LabWired's reference Armv8-M target. The current model
covers the minimal smoke surface — RCC, GPIO, LPUART1, SysTick — and
**does not model the radio, crypto, or any of the timers / SPI / I²C
blocks**.

For build/run instructions, see
[`examples/nucleo_wba52cg/README.md`](../../examples/nucleo_wba52cg/README.md).

## Status at a glance

| Aspect              | Status                                                                          |
|---------------------|---------------------------------------------------------------------------------|
| Chip yaml           | [`configs/chips/stm32wba52.yaml`](../../configs/chips/stm32wba52.yaml)          |
| System yaml         | [`configs/systems/nucleo_wba52cg.yaml`](../../configs/systems/nucleo_wba52cg.yaml) |
| Reference firmware  | `examples/nucleo_wba52cg/board_firmware/`                                       |
| Validation          | LPUART1 smoke pass — see [`examples/nucleo_wba52cg/VALIDATION.md`](../../examples/nucleo_wba52cg/VALIDATION.md) |
| Tier                | smoke-validated (LPUART1 only)                                                  |
| Hardware parity     | Not byte-parity hardware-validated                                              |

## Peripherals (from chip yaml)

| Peripheral | Base       | Status       | Notes                                                   |
|------------|------------|--------------|---------------------------------------------------------|
| Cortex-M33 | —          | ✅ modeled   | Thumb-2 + FPU; TrustZone present on silicon, not modeled |
| SysTick    | 0xE000E010 | ✅ modeled   | System-exception (15) path                              |
| RCC        | 0x46020C00 | ✅ modeled   | `stm32v2` profile                                       |
| GPIOA      | 0x42020000 | ✅ modeled   | `stm32v2` MODER/AFR                                     |
| GPIOB      | 0x42020400 | ✅ modeled   |                                                         |
| GPIOC      | 0x42020800 | ✅ modeled   |                                                         |
| GPIOH      | 0x42021C00 | ✅ modeled   |                                                         |
| LPUART1    | 0x46002400 | ✅ modeled   | `stm32v2` profile, IRQ 45 — used as virtual COM         |

## Not yet modeled (commonly expected on WBA52)

The chip yaml does not declare: **USART1/2**, **SPI1/3**, **I²C1/3**,
**TIM1–TIM3/16/17**, **LPTIM1/2**, **ADC4**, **RTC**, **IWDG/WWDG**,
**FLASH** controller, **PWR**, **DMA / GPDMA**, **RNG / HASH / SAES /
PKA**, **AES**, **2.4 GHz radio**, **TrustZone (SAU)**.

Firmware that touches any of these registers will hit
`MemoryAccessViolation` or stall in a polling loop. See
[`docs/getting_started_firmware.md`](../getting_started_firmware.md).
