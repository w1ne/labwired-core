# SiFive FU540 attempt 01

The release firmware built for `riscv64imac-unknown-none-elf`, but the
simulation stopped before executing firmware instructions. The RV64 core tried
to read an eight-byte literal from the XIP image at `0x20000010`; the system bus
only allowed eight-byte reads from RAM, so it reported `MMIO64 is unsupported`.
The original metrics and run log are preserved in this directory. The follow-up
fix allows eight-byte reads from mapped memory while retaining the MMIO limit.
