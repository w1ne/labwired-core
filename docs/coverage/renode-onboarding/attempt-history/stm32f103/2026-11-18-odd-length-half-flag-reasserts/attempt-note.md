# Odd-length DMA half-transfer flag reassertion attempt

The exact-import `stm32f103` target booted with all 88/89 smoke checks passing.
The new 13-item DMA assertion observed HTIF at `CNDTR=6`, observed DMA1
channel 1 pending with TCIE disabled, cleared HTIF and NVIC pending, and then
found the DMA IRQ pending again after TC. UART reported `TC_IRQ_OFF=0`.

This exposed that the DMA model can reassert HTIF on every later item after the
halfway threshold if software clears the flag before transfer complete. This
run's build/run logs, UART, JUnit, result, snapshot, and metrics are preserved
here. The next model revision makes the half-transfer event one-shot per
transfer and adds a regression for mid-transfer flag clearing.
