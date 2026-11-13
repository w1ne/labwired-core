# STM32L071 Round 500 second run

This retry corrected the UART hex expectation and moved the new RX check after
all existing peripheral checks. All five RX checks passed, but the pre-existing
SPI1 and SPI2 RXNE IRQ assertions still reported `pend=0`; their scoreboard
status therefore remains failed in this archived attempt. The complete run
artifacts are preserved here. The next retry delays RX injection until the
existing peripheral checks have finished.
