# Negative control: EFR32MG13 WTIMER0 as a 16-bit TIMER

Same firmware (firmware-efr32mg13-smoke) and smoke script as the passing run,
but with `wtimer0` typed `efm32s1_timer` (16-bit counter) instead of
`efm32s1_wtimer`. Base, size and IRQ (37) are unchanged.

The smoke FAILS 3/5. The WTIMER line prints
`wtim0 32bit top=0 cc0=1 no_early_of=0 cnt_hi=0 of=1 ... FAIL`: TOP
0x11000 reads back as 0x1000, the counter wraps at 0xFFFF and raises OF
before CC0 matches, and CNT never reaches 0x10000. The summary word is
0xEF3101FF instead of 0xEF3103FF. See `run_smoke.log` and
`simulation-output/uart.log`.
