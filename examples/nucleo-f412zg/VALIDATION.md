# NUCLEO-F412ZG validation

The committed smoke builds `firmware-nucleo-f412zg-smoke` for
`thumbv7em-none-eabi`, boots it against `configs/systems/nucleo-f412zg.yaml`,
and requires at least eight GPIO edges on `gpiob:7` before the simulator's
256-step limit. The trace demonstrates that firmware enabled the GPIOB clock,
configured PB7 as an output, and exercised both output levels.

This is a deterministic simulator smoke, not a physical-board test. The
descriptor is intentionally limited to the RCC, GPIOB, and SysTick models used
by that path. The board's ST-LINK, VCP UART, USB, Ethernet PHY, button, and
other MCU peripherals remain outside the profile.
