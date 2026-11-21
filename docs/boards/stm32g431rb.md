# STM32G431RB (Nucleo-G431RB)

Cortex-M4F, RM0440. 128 KiB flash, 22 KiB SRAM + 10 KiB CCM SRAM.

## Status at a glance

| Aspect      | Status                                                              |
|-------------|------------------------------------------------------------------------------|
| Chip yaml   | [`configs/chips/stm32g431rb.yaml`](../../configs/chips/stm32g431rb.yaml)     |
| Validation  | tier-1 fast-boot fixture (`tests/fixtures/tier1/stm32g431rb.elf`) via `examples/stm32g431rb/{tier1-smoke,io-smoke}.yaml` |
| Tier        | **structural** — tier-1 fixture only, no silicon diff                        |
| Stock Arduino core | Simulator-verified: stm32duino 2.12.0 `nucleo_g431rb` passes Arduino matrix L0–L8 (9/9: Serial on LPUART1, blink on PA5, INA219 on Wire, MAX31855 on SPI, ADC 512 at 1651 mV, 61 TIM2_CH1 edges on PA0, micros, FDCAN1 loopback); evidence in `validation/arduino-matrix/evidence/nucleo-g431rb/` |

## What is proven

- The tier-1 raw-register self-test boots at the G431 geometry and reports
  its clock, GPIO and UART classes PASS, then `TIER1 done`
  (`examples/stm32g431rb/io-smoke.yaml`, `examples/stm32g431rb/tier1-smoke.yaml`).
- Peripherals declared in the chip yaml reuse the stm32g474re RM0440 models:
  RCC (`stm32g4` profile), GPIOA-D (`stm32v2` profile), SysTick, USART1/2,
  LPUART1, FLASH (`stm32g4` profile: 4-bit ACR.LATENCY), PWR, CRC, TIM2, TIM1
  PWM (G4 pad map), FDCAN1, I2C1, SPI1, ADC1, DMA1, IWDG, RTC, NVIC
- RAM is the 32 KB contiguous block at 0x20000000: SRAM1 + SRAM2 + the CCM
  SRAM alias at 0x20005800. The 0x10000000 CCM address is not mapped.

## What is NOT proven

- No STM32G431 silicon diff — no bench part.
- No upstream Zephyr survival case yet.
- G4-specific analog (OPAMP, COMP, DAC), CORDIC/FMAC and HRTIM are not modelled.
- FDCAN1 is proven in internal loopback only; no CAN-FD bit-rate switching or
  external bus traffic is claimed.

## Sources

- `configs/chips/stm32g431rb.yaml`, cross-checked against ST DS12589 and RM0440.
