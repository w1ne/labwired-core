# Retained original F746 smoke failure

This run used the original F746 firmware and smoke script, without the new
PendSV check. It booted the imported system for 8,000,000 instructions, then
failed the same SPI1/2/3 `pend=1` assertions after the RX FIFO was drained.
This confirmed the mismatch was in the old assertion timing, independent of the
PendSV probe. The scoreboard was regenerated after the corrected in-flight
RXNE observation passed.
