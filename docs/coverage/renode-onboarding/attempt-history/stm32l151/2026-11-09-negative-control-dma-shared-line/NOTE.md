Negative control (2026-11-09): DMA1 declared without `channel_irqs`, so the
classic controller falls back to its single shared line (none declared).
The SPI1 transfer still completes, but DMA1_Channel2_IRQn (12) never pends:
`dma1 spi1 frame=0x01901600 data=1 flags=1 pend=0 ... FAIL`, smoke FAIL 5/7.
