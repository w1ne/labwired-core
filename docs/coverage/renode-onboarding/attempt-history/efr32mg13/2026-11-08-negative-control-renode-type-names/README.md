# Negative control: EFR32MG13 with Renode's type names

Same firmware (firmware-efr32mg13-smoke) and smoke script as the passing run,
but with the chip descriptor's GPIO ports typed `efr32_gpioport` and the
TIMERs typed `efr32_timer` (Renode's `EFR32_GPIOPort` / `EFR32_Timer`), which
is what a naive import would write. In this engine those names route to the
STM32F1 placeholder GPIO layout and the Series-2 TIMER model.

Without `config: { counter_bits: 16 }` the Series-2 timer refuses the config.
With it, the run executes and the smoke FAILS 2/5: every GPIO line prints
FAIL, the firmware does not get past the TIMER checks within 200000 steps, so
the summary word is never
written (0x2000FC00 = 0x0). See `run_smoke.log` and `simulation-output/uart.log`.

The passing descriptor uses `efm32s1_gpioport` / `efm32s1_timer`.
