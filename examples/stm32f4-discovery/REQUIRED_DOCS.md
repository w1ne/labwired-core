# Required Documents — STM32F4-Discovery

Authoritative sources used by this example (retrieved 2026-10-19):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| UM1472 | Discovery kit with STM32F407VG MCU user manual | Board facts: user LEDs LD4/LD3/LD5/LD6 on PD12..PD15, USER button PA0, ST-LINK/V2 wiring (the VCP is not wired to the MCU UART), extension headers and MB997 geometry |
| DS8626 | STM32F405xx/407xx datasheet | Memory map (1 MB flash @ 0x08000000, 128 KB SRAM @ 0x20000000 + 64 KB CCM), 168 MHz max, LQFP100 pinout, I/O electrical characteristics |
| RM0090 | STM32F4 reference manual | RCC AHB1ENR/APB1ENR clock gates, GPIO MODER/AFRL/BSR/ODR register map, USART2 SR/DR/BRR/CR1 register map and 16 MHz HSI baud math |
| Zephyr `stm32f4_disco` board page | Board support documentation | Cross-check of the default pin mapping (UART2 PA2/PA3, LEDs PD12..PD15, USER button PA0) and the serial-console note |

The system uses the exact `configs/chips/stm32f407vg.yaml` descriptor for the
board's STM32F407VGT6. The example package keeps a board-local system manifest
that mirrors `configs/systems/stm32f4-discovery.yaml`.

Open item: none for the modelled surface (CPU + RCC + GPIO + USART2). The
on-board accelerometer, audio DAC and microphone are out of scope and listed
in `EXTERNAL_COMPONENTS.md`.
