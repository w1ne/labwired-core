# Hosted LabWired verification result

Checked 2026-11-04 UTC with catalog target `stm32h735`, the MCU part in a bare
board diagram, and UART plus GPIO clauses for the board smoke behavior.

- Result: **unsupported**, not model-verified.
- Both GPIO clauses observed PC2 and PC3 low, which is the active state for the
  board's user LEDs.
- The UART marker was not observed before the hosted run faulted.
- The hosted H735 run reported unmapped writes at `0xFFFFFFE0` and
  `0xFFFFFFFC`, then a precise BusFault escalated to HardFault. It also reports
  that its bare diagram assumes an ideal 3.3 V rail.

The repository's local board-specific smoke passes its UART and GPIO
assertions, but this hosted result does not support a model-verification claim.
The narrow hosted follow-up is to fix the H735 runner's exception/stack access
at those addresses and rerun the same oracle.
