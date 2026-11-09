Negative control (2026-11-09), final firmware and engine: DMA1 `requests`
as bare channel numbers (`spi1_rx: 2`, ...) instead of `[channel, CSELR
selector]`, i.e. hard-wired like an F0/F1. FAIL: `dma1 spi1/spi2
unselected=0` — the SPI frame moves while the channels' CSELR nibbles hold
the other SPI's selector, which an L0 cannot do. Summary 0x710F9FFF
(bits 13 and 14 missing).
