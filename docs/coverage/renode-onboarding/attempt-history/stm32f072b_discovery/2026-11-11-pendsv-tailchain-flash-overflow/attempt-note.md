# STM32F072B-DISCO SysTick-to-PendSV tail-chain attempt

The target-only onboarding run stopped during firmware linking, before a
simulated boot. The added Cortex-M0 test exceeded the imported part's 128 KiB
flash region by 500 bytes in `.text`; the linker also reported `.rodata` and
related sections outside the region. The linker log and generated failure
metrics are preserved here. The smoke firmware now uses the workspace's
size-optimized release profile for this package; a later target-only run will
determine whether it fits and exercises the new behavior.
