# Attempt 1: F1 regular scan, second IRQ pending check

The exact imported `stm32f103` system booted 50,000,000 instructions to
`max_steps`. The scan converted both ranks to the expected values
`0x03FF,0x07FF`, the first EOC/IRQ and DR-read clear passed, but the second
NVIC pending check was false. The new target assertion failed; this attempt is
retained as evidence before the IRQ observation is narrowed/fixed.
