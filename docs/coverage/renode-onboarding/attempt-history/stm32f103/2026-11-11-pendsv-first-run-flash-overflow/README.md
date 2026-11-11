# stm32f103 — first PendSV deferral run (2026-11-11)

The exact-import target-only smoke failed before reaching the new PendSV assertion. Its release ELF had a 66,057-byte load image, which exceeded the imported STM32F103 C8 profile’s 64 KiB flash map by 521 bytes. The linker script allowed 128 KiB, so the simulator could not load the second PT_LOAD segment at 0x08000040. It executed 50,000,000 instructions but emitted only the first two SysTick lines; the smoke failed 71 assertions. The full metrics, logs, UART, JUnit, result, and snapshot from that attempt are preserved here.
