Negative control (2026-11-09), final firmware and engine: DMA1 without
`channels: 5` (the 7-channel controller, `channel_irqs` padded to 7 lines)
instead of the F04x's five channels. FAIL 4/6: `dma1 channels reserved=0
idle=0` — CCR6/CCR7 hold their writes and the memory-to-memory transfer
programmed on channel 6 runs, which an F042 cannot do. Summary 0x0427BFFF
(bit 14 missing).
