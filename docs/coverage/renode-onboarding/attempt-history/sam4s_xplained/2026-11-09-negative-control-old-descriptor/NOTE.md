Negative control (2026-11-09), final firmware and engine: the previous
10-block sam4s_xplained descriptor (PMC, UART/USART, PIOA-C, TC, no
cpu_hz, chip.yaml here). No EEFC/RSTC/WDT window: EEFC_FMR reads 0
("eefc fmr=00000000 FAIL") and the GETD wait on EEFC_FSR.FRDY never
completes. FAIL 1/5.
