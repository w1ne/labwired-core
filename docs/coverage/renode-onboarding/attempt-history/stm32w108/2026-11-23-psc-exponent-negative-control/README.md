# stm32w108 negative control: linear prescaler (2026-11-23)

The STM32W108 TIMx_PSC field is a 4-bit exponent: CK_CNT = PCLK / 2^PSC[3:0].
Source: Contiki `cpu/stm32w108/rtimer-arch.h` (PSC=12 is documented as
"PCLK/4096"; PSC=11 as "PCLK/2048"). Renode's `STM32_Timer` writes
`Divider = PSC + 1` for this part.

The smoke firmware now loads TIM2 PSC=3 and requires TIM1/TIM2 counts in the
6x..10x window (2^3 = 8). A linear PSC+1 divider gives 4x.

Same firmware and smoke script, with the chip descriptor from before this
change (the `stm32w108.yaml` copy here, no `prescaler: power_of_two`), loaded
through `name: stm32w108` / `chip: stm32w108.yaml` with `labwired test`.
FAIL 3/4: the result word reads `0x571001EF` instead of `0x571001FF`. Only
bit 4 (the ratio window) is clear. UART banner, UARTSTAT readback and the stop
reason still pass. With the new descriptor the same firmware passes 4/4
(`docs/coverage/renode-onboarding/metrics/stm32w108/`).
