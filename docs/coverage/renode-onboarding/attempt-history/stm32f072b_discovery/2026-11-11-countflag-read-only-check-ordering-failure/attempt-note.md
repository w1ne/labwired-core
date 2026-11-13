# Failed COUNTFLAG read-only assertion attempt

The exact imported `stm32f072b_discovery` target booted and executed all 4,000,000 configured instructions. Its existing smokes passed, and the new byte-read check observed `COUNTFLAG` remain set after a byte read. The new assertion then reported `write_ignored=0` because the firmware performed a word CSR read to capture the control bits; that read itself correctly cleared `COUNTFLAG` before the code checked whether the CSR write had preserved it.

This was an assertion-ordering bug in the smoke firmware, not evidence of a SysTick model failure. The complete failed metrics and simulator output are preserved alongside this note. The check is corrected by capturing control bits and `COUNTFLAG` from the same read-clear word read.
