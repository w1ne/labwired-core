The committed stm32f746 profile before this round (2026-11-09): USART1, GPIOI
and RCC only (3 claimed windows), 2 MiB / 384 KiB (Renode's repl geometry,
not the F746xG's 1 MiB / 320 KiB). Run with this round's smoke firmware: the
first access outside the three windows (GPIOD_ODR, 0x40020C14) BusFaults
(PRECISERR, BFAR=0x40020C14) before any output; none of the fixture devices
exist. Kept as the before-state of the Renode stm32f746.repl surface work.
