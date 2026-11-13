# BASEPRI smoke attempt 3

The imported system booted, and the full firmware smoke completed. The new
BASEPRI context passed (`0x3ff`), including IRQ 8 running under BASEPRI,
pending IRQs 7 and 9 dispatching in order after unmask, and MSP restoration.
The pipeline failed one stale memory assertion: the expected pass bitmap was
still `0x70001fff`; the observed value was `0x70003fff` after adding the new
smoke bit.

All per-target run artifacts are preserved here; the runner had recorded one
failed target before the final replay regenerated the current scoreboard.
