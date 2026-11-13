# STM32F412: stale profile against current shared F429 firmware

Date: 2026-11-13. Exact-target rerun (`--only stm32f412`) before any change.

The committed pass used an older `firmware-stm32f429-smoke` binary. Commit
`0b4df27b2` added a SysTick-to-PendSV PSP tail-chain check to the shared
firmware and declared SysTick only on `stm32f429`. On `stm32f412`, which has no
SysTick declared, the UART captured only the banner and the `gpioPortD:12`
`gpio_edges` assertion failed (800,000 steps to max_steps).
