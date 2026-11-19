# STM32F072B-DISCO PendSV probe build overflow

The 2026-11-11 attempt added PendSV clear/re-pend assertions to the existing board smoke. Firmware compilation reached the linker, but the 128 KiB FLASH region overflowed by 4,728 bytes in `.text` (and 6,056 bytes in `.rodata`/`.data`). The simulator did not run, so this is a build failure, not a platform boot result. The probe was moved to the smaller `stm32f072` imported smoke fixture, which uses the same Cortex-M0 and SCB profile.
