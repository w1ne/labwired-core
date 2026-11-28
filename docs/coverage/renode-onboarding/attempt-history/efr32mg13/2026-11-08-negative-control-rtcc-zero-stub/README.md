# Negative control: EFR32MG13 RTCC as a zero-answering stub

Same firmware (firmware-efr32mg13-smoke) and smoke script as the passing run,
but with `rtcc` typed `stub` instead of `efm32s1_rtcc`. Base, size and IRQ (31)
are unchanged. A stub is what the window would be without a model: it
answers every read with zero.

The Renode type name `efr32_rtcc` was tried first. The engine refuses it at
load time ("unknown peripheral type 'efr32_rtcc' ... Refusing to answer it with
a zero-filled stub"), so a stub has to be declared explicitly.

The smoke FAILS 3/5. Every check up to `wtim0` passes. The RTCC check never
completes: CNT stays at zero, so OF and CC1 never set, and each bounded wait
spins until the 200000-step budget is spent. Neither the `rtcc ... ok` line nor
`EFR32MG13 PASS` is printed, and the summary word at 0x2000FC00 is never
written (expected 0xEF3107FF). See `run_smoke.log` and
`simulation-output/uart.log`.
