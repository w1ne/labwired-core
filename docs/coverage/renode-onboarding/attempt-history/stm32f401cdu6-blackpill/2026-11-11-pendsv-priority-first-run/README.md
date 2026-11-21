# F401 PendSV priority probe: first run

The target reached the new test but stopped emitting UART after the periodic
SysTick check. The result snapshot had SP at `0x20017f18`, inside the
`0x20017f0c..0x20017f1c` scratch words used by the new handlers. The smoke's
result and handler scratch area was moved to `0x20001000` before the next run.

This attempt is preserved as a failed local CLI run. It is not part of the
current scoreboard measurement.
