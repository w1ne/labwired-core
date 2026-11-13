Negative control (2026-11-09), final firmware and engine: the sam4s16c
descriptor with the three `irq: 11/12/13` lines removed from pioA-C
(chip.yaml here). The PIO still latches PIO_ISR, but no NVIC line carries
it: after "bp2 pressed" the release never pends PIOA's line 11 and the
bounded wait runs past max_steps. FAIL 2/5.
