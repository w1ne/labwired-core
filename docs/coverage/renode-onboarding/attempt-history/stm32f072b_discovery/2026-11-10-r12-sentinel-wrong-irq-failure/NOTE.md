# Cortex-M0 r12 sentinel first attempt

The target booted for 4,000,000 instructions, but the smoke failed because the
new fixture accidentally changed the NVIC pending value from IRQ 5's `0x20` to
the r12 sentinel `0x55`. IRQ 5 therefore never entered its handler, and the
frame capture remained at its sentinel fill value. This is a firmware fixture
bug, not a simulator finding. The follow-up restores `r4 = 0x20` after copying
`0x55` to r12 and asserts the stacked r12 word.
