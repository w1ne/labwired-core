# nRF52840 SysTick tail-chain attempt

The first M4 tail-chain smoke attempt booted and executed 4,000,000
instructions, then failed. The imported nRF52840 chip descriptor did not map
the architected SysTick block at `0xE000E010`; the firmware's initial CSR write
raised a precise BusFault and escalated to HardFault. UART stopped after the
PPI check, so the new tail-chain assertion and final memory check did not run.

This attempt is retained as a failure record. The next attempt adds the existing
SysTick peripheral model to the exact nRF52840 chip descriptor, then reruns only
this target.
