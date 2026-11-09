Negative control (2026-11-09), final firmware and engine: DMA2 SPI1 requests
routed on SxCR.CHSEL 2 instead of RM0385 Table 27's channel 3. FAIL 10/12:
dma2 spi1 unselected=0 (the smoke's deliberately-wrong CHSEL 2 pass now
moves data) and the right CHSEL 3 pass moves nothing (frame 0xFFFFFFFF).
Summary 0x747FF7FF (bit 11 missing).
