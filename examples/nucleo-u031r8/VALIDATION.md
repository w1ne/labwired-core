# NUCLEO-U031R8 validation

The reproducible smoke builds `firmware-nucleo-u031r8-smoke`, boots it against
`configs/systems/nucleo-u031r8.yaml`, and requires at least 16 PA5 output edges
before the 10,000-step limit. The `evidence/` directory records the firmware
build, simulator result, assertion log, and onboarding metrics.

This profile exercises the modeled STM32U0 RCC GPIOA clock gate at IOPENR
offset 0x4C and GPIOA register path attached to LD4. It does not claim
USART2/VCP, USB, ST-LINK, buttons, timers, or remaining STM32U031 behavior.
This is local simulator evidence, not physical-board confirmation or a hosted
`labwired_verify` verdict. The supply rail is idealized; power-on and
brown-out behavior are not simulated.

ST's live lifecycle and orderability check is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
