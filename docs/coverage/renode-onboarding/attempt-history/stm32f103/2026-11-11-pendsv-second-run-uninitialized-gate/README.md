# stm32f103 — second PendSV deferral run (2026-11-11)

After reducing the ELF to 21,973 bytes and enforcing 64 KiB flash, the target booted and passed its first two SysTick checks. It then stopped in the existing periodic SysTick smoke. The new request flag lived in uninitialized SRAM and was read by the shared SysTick handler before the new test set it. Initialize the gate at firmware entry before running any SysTick checks. Full metrics and simulation artifacts are preserved here.
