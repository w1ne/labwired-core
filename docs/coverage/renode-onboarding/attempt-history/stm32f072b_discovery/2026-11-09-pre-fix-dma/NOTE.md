This round's disco smoke on the engine before the two legacy STM32 DMA
fixes (dma.rs reverted, everything else as committed): FAIL 3/5,
`dma1 m2m tc=1 data=0 pend=1 alone=1 released=0`. MEM2MEM ignored DIR and
copied CMAR -> CPAR, so the HAL-style DIR=0 transfer (source in CPAR) wrote
the zeroed destination over the source; and IFCR.CGIF1 cleared only GIF1,
leaving TCIF1 set. Summary 0x0DC05FFF (bit 13 missing).
