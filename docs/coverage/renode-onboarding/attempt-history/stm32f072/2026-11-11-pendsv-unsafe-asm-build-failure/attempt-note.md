# STM32F072 PendSV probe compiler error

The first target-only build of the PendSV clear/re-pend probe failed before simulation because the initial inline `asm!` block that reads PRIMASK was outside an `unsafe` block. The compiler reported E0133. No firmware ran. Wrapped the block as required before retrying.
