# NUCLEO-U083RC validation

The reproducible smoke builds `firmware-nucleo-u083rc-smoke`, boots it against
`configs/systems/nucleo-u083rc.yaml`, and requires at least 16 output edges on
PA5 before the configured instruction limit. The `evidence/` directory records
the build, simulator result, assertion log, and onboarding metrics.

This profile exercises the modeled STM32U0 RCC GPIOA clock gate at IOPENR
offset 0x4C and the STM32 GPIO register path attached to LD4. It does not claim
UART/VCP, USB, ST-LINK, button, timer, or other STM32U083 behavior. The result
is local simulator evidence, not physical-board confirmation or a hosted
`labwired_verify` verdict. The run also records that the supply rail is
idealized, so power-on and brown-out reset behavior are not simulated.

ST's live lifecycle and availability check is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
