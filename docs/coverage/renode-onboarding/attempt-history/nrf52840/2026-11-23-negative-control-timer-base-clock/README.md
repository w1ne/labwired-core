# nrf52840 negative control: TIMER counts CPU cycles (2026-11-23)

The nRF52840 TIMER base clock is 16 MHz: fTIMER = 16 MHz / 2^PRESCALER
(nRF52840 PS v1.7 §6.30; Renode `NRF52840_Timer.cs` `InitialFrequency =
16000000`, `Divider = 1 << PRESCALER`). The Cortex-M4F runs at 64 MHz.
Before this change the model advanced one base tick per CPU cycle, so TIMER
ran 4x fast against the core.

The smoke firmware now runs TIMER0 at PRESCALER=0 and SysTick (CLKSOURCE=1,
core clock) over one busy loop and requires SysTick to advance 3x..5x TIMER0.

Same firmware and smoke script, with the chip descriptor from before this
change (the `nrf52840.yaml` copy here: no `cpu_hz`, no `peripheral_hz`),
run with `target/debug/labwired test`. FAIL 5/7:
`timer0 clk sys=001775 tim=001774 FAIL`; the result word reads `0x5281FFEF`
instead of `0x5281FFFF` (only bit 4, the TIMER0 block, is clear).
With the new descriptor the same firmware passes:
`timer0 clk sys=001775 tim=0005dd ok` (ratio 4.00), see
`docs/coverage/renode-onboarding/metrics/nrf52840/` and
`metrics/nrf52840dk_nrf52840/` (`sys=001777 tim=0005dd`).
