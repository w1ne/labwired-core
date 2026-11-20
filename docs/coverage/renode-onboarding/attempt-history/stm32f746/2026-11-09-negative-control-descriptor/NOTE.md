Negative control (2026-11-09), final firmware and engine, four descriptor
mutations at once, each owning one check: RCC without rcc_ahb1enr_reset,
SYSCFG profile stm32f4 instead of stm32f7, DMA2 without dma2: true, CRC
without idr_width: 8. FAIL 10/12, exactly those four lines:
rcc reset=0 (AHB1ENR reads 0, not DTCMRAMEN); syscfg memrmp=0x00000003
pmc=0x00810000 (F4 fields); dma m2m dma2_copied=0 (memory-to-memory
refused on a controller that is not DMA2); crc idr=0 (32-bit IDR).
Summary 0x7477DFF6 (bits 0, 3, 13, 19 missing).
