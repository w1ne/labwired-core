# stm32w108 negative control: SC1 removed (2026-11-08)

Same firmware and smoke script, with `sc1` removed from the chip (the
`stm32w108.yaml` copy here, loaded through a one-line system manifest
`chip: stm32w108.yaml` with `labwired test --system`). FAIL 1/4: no UART bytes,
and neither the result word nor the UARTSTAT readback is written. Only the
max_steps stop reason still holds. The first SC1 access (the SC1_MODE write at
0x4000C854, PC 0x08000486) is unmapped and escalates to a HardFault
(CFSR 0x8200 PRECISERR, BFAR 0x4000C854). The UART checks therefore depend on
the modelled SC1. The timer checks never run because they come after the UART.
