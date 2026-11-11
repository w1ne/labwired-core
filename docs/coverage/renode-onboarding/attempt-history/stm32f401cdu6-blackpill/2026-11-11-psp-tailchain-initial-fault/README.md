# Initial F401 PSP tail-chain attempt

This directory preserves the first target-only run while adding the PSP
SysTick-to-PendSV case. It stopped before the first new UART assertion with a
memory violation at `0xA5A5A5A4`; the result reported zero executed firmware
instructions. The CPU snapshot showed the poisoned PSP sentinel selected after
the helper returned.

The PSP setup helper had pended SysTick and returned without waiting for
PendSV, restoring the caller's PSP and CONTROL, or popping the saved MSP
registers. The corrected helper waits for the PendSV handler marker, restores
the original task-stack state, and returns through the saved MSP frame. The
subsequent exact-import run passed and is recorded in the current metrics
directory.
