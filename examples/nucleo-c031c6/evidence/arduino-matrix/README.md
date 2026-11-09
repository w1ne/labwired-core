# NUCLEO-C031C6 Arduino matrix evidence (lane)

Lane run, 2026-11-09, stock STM32duino (`ststm32@19.7.1`, `nucleo_c031c6`) on
`configs/chips/stm32c031c6.yaml`: **6 pass / 3 skip / 0 fail**.

L0 (USART2 VCP after the HSE switch), L1, L2 (LD4/PA5 edges), L3 (INA219
exact), L4 (MAX31855) and L7 pass. Three cells are skipped:

- L5: there's no C0 ADC model, and `analogRead` returns `0 0`.
- L6: the lane predates main #1734 (STM32 timer AF pad routing). Here PA5
  shows 0 edges, and the earlier marker-only pass was vacuous.
- L8: the C031 has no CAN.

The delivery is PR #1756 to main, which adds the `stm32c0` timer pad table.
On main L6 is a 1.000 kHz / 50.2 % TIM1_CH1 waveform on PA5, and the row is
7 pass / 2 skip. Full evidence ships in that PR.
