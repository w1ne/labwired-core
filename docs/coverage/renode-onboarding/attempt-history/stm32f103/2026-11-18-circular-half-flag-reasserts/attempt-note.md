# Circular DMA half-transfer flag reassertion attempt

The exact-import `stm32f103` target booted, and the new odd-length HTIE-only
check passed. One pre-existing four-item circular-buffer assertion then failed:
its second two-rank scan expected HTIF and TCIF together after firmware had
cleared HTIF at the midpoint. With one-shot-per-transfer HT tracking, TCIF is
set at completion and HTIF remains clear; the next circular lap raises HTIF
again at its midpoint as expected.

This run's build/run logs, UART, JUnit, result, snapshot, and metrics are
preserved here. The smoke assertion should distinguish flag clearing within a
transfer from a new circular lap after CNDTR reload.
