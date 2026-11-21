Negative control (2026-11-09), final firmware and engine: sam4s16c with
the previous 2-window descriptor (PMC + UART0 only). No EEFC/RSTC/WDT
window: EEFC_FMR reads 0 ("eefc fmr=00000000 FAIL") and the GETD wait on
EEFC_FSR.FRDY never completes. FAIL 1/3, 45 UART bytes.
