# External Components — STM32U575I-EV

No external simulated component is required. The smoke drives on-chip blocks
only: RCC clock gates, the `stm32v2` GPIOB/GPIOH output path, the U5 memory map
and USART1 TX captured by the engine's UART sink.

The board carries more hardware than this example models:

| Board component | Wiring | State |
| :--- | :--- | :--- |
| ST-LINK-V3E virtual COM port | USART1 PA9/PA10 | modelled as the console sink (no host bridge) |
| LED5 / LED6 | PB7 / PH7, active high | modelled GPIO outputs with `board_io` bindings |
| LED7 / LED8 | MFX I/O expander | **not modelled** (not direct GPIO) |
| USER button | PC13 | modelled input with a `board_io` binding; not pressed by the smoke |
| LCD, camera, Octo-SPI, microSD, audio, EEPROM, SRAM | board peripherals | not modelled / not attached |
| FDCAN transceiver | FDCAN1 pads | the transceiver is not modelled; loopback is internal |

When any of these becomes a milestone, add it here with its fidelity tier and
descriptor path, and grow the system manifest's `external_devices` entry.
