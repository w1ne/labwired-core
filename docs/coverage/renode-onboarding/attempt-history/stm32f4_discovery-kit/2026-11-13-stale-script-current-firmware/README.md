# STM32F4DISCOVERY kit: stale smoke script against current firmware

Date: 2026-11-13. Exact-target rerun (`--only stm32f4_discovery-kit`) before any
change in this round.

The committed pass (started 2026-11-09T16:43:31Z) used an older
`firmware-f4-discovery-smoke` binary. On 2026-11-11 the shared firmware gained
USART2 RX-gate, SPI1 MAX31855 and SysTick/PendSV checks for the
`stm32f4_discovery` row. The kit row kept the old script and chip profile.

Result: the CLI ran 4,000,000 steps to `max_steps`, the UART captured only
`F4-DISCO SMOKE`, and the `uart_ordered` heartbeat assertion failed. The
firmware waited in the USART2 RX-gate loop because the kit script injects no
bytes, and the kit chip declares no SPI1 or SysTick.
