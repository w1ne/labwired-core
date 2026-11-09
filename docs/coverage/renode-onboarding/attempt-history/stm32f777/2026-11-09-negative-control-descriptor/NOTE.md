Negative control (2026-11-09), final firmware and engine: stm32f777
chip with the F746 RCC (APB1/APB2 masks), the F746 SYSCFG (profile
stm32f7) and SPI5 on NVIC 86 instead of 85. FAIL 10/12, exactly:
rcc mask=0; syscfg memrmp=0x00000C00 (no SWP_FB); spi5 pend=0 (its line
86 stays pending, so spi6's quiet-before check fails too).
Summary 0x767FFEEE (bits 0, 4, 8 missing).
