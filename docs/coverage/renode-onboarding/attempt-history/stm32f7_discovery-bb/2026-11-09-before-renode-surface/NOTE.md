The stm32f7_discovery-bb chip descriptor before this round (2026-11-09): a
board-only profile with USART1, GPIOI (`gpioPortI`) and RCC (3 claimed
windows), run with this round's board smoke firmware and script. ERROR 1/5:
the firmware's RCC bring-up writes GPIOD_ODR (0x40020C14) to prove the clock
gate, which the old profile does not map, so the first access BusFaults
(HardFault, BFAR 0x40020C14) before USART1 prints; the B1 stimuli could not
be applied because the old profile has no `gpioi`. The previous smoke only
asserted "OK" on USART1 and one PI1 edge. Kept as the before-state of the
Renode stm32f746.repl surface work.
