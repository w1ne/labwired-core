# ezr32hg320 — negative control: old window set, flash moved to 0 (2026-11-09)

Run: firmware-ezr32hg320-smoke (this round) against the descriptor at
8a91eaa22 with only `flash.base` changed to 0 (`chip.yaml`: USART0 only).
This isolates the peripheral claim from the flash-base fix.

Result: FAIL 2/5 (banner + stop reason). Only the banner prints (25 bytes),
then the first GPIO access (0x400060B8, port F MODEL) (0x400060B8, port F MODEL) is a precise BusFault escalated to
HardFault: no GPIO port is mapped in the old descriptor. Summary word 0x0.
