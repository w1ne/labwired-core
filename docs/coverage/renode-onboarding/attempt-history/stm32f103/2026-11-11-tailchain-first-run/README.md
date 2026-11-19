# First tail-chain smoke attempt

The exact imported STM32F103 run booted, but the new tail-chain UART assertion failed. The record showed IRQ 55 entered while IRQ 54 was still active (`EXC_RETURN=FFFFFFF1`) and the R0 marker was absent. The fixture had the two priorities reversed: IRQ 55 was higher priority than IRQ 54, so it correctly preempted instead of waiting to tail-chain. The final fixture swaps the priority bytes and reruns the same target.
