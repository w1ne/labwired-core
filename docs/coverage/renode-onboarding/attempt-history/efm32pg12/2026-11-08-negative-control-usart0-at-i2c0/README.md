# Negative control: EFM32PG12 with USART0 at 0x4000C000 (discriminating)

The round-387 firmware (`firmware-efm32pg12-smoke`, USART0 at 0x40010000,
linked at 0x00000000) ran against a copy of the pre-fix
`configs/chips/onboarding/efm32pg12.yaml` (`efm32pg12-negctl-chip.yaml`). In
that copy USART0 sits at 0x4000C000, which is I2C0 on EFM32PG12 silicon and in
Renode's efm32xg1x/efm32pg12.repl. The system file differed from
`configs/systems/onboarding/efm32pg12.yaml` only in its `chip:` path.

Result: FAIL 1/2. The first USART0 write BusFaults at 0x4001000C, so
`uart_contains` gets 0 bytes. The fixed descriptor passes 2/2. The old
descriptor could only pass because the old firmware made the same wrong
assumption.

The flash-base change (0x08000000 → 0x0) in the same round can't be told
apart this way. `flash_boot_alias` mirrors any base at 0. See
attempt-history/efm32zg222/2026-11-08-negative-control-flash-0x08000000.
