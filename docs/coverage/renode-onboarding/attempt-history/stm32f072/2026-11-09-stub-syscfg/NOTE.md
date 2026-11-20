First full run of the extended smoke (2026-11-09) with the final firmware,
before the engine had an STM32F0 SYSCFG model:

- `exti pads ... pf1=0 pc13=0 FAIL`: a `syscfg` entry with no profile is
  built as a read-zero stub, so EXTICR1..4 did not exist. Both buttons
  toggled their pads (`presses=1`), but no edge reached EXTI lines 1/13 or
  NVIC 5/7. Fixed by `Stm32F0Syscfg` (`profile: stm32f0`, A..F = 0..5).

Everything else already passed: summary 0x072FDFFF (bit 13 missing).
Earlier probe runs (not kept) found two firmware mistakes, not engine
bugs: AHBENR enabled IOPA..IOPE only (the GPIOF clock gate correctly
ignored PF writes), and the second TMP102 read expected 0x1900 although
configs/devices/tmp102.yaml documents +0.5 °C self-drift per read.
