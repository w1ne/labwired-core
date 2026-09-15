# External Components (STM32F7 Discovery)

No required external simulated components for minimal deterministic smoke.

The onboarding path uses on-chip peripherals only:
1. RCC (`stm32f4`)
2. GPIOA–GPIOI (`stm32v2`)
3. USART1 (`stm32v2` — ST-LINK VCP)
4. SysTick
5. Stub windows: LTDC, ETH, DMA2D, USB OTG FS, QUADSPI

## On-board devices intentionally omitted

STM32F7 Discovery carriers include SDRAM (FMC), LCD-TFT (LTDC + DMA2D), Ethernet PHY, USB OTG, and Quad-SPI flash. Those are **out of scope** here — smoke uses USART1 + PI1 only and treats the listed blocks as stub MMIO windows.

## Adding an external device (I²C / SPI sensor, EEPROM, etc.)

See [`examples/demo-blinky/`](../demo-blinky/README.md) — that example is
the canonical reference for the `external_devices` attach pattern. Before
copying, check that the bus you need is actually modeled for this chip.
At time of writing the F746 model covers RCC / GPIO / USART1 / SysTick /
stubs only.
