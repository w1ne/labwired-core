# ut32m0r500 — negative control: wrong-layout-stm32f1 (2026-11-09)

Run: this round's descriptor with the three GPIO banks on the `stm32f1` layout instead of `cmsdk` (same addresses and sizes). Result: FAIL 4/7 assertions; summary `PFFPFFFFFP` (0x5E320209). Only check 1 (reset zeros), 4 (pull-down = 0 by default) and 10 (SRAM) pass, so the CMSDK checks discriminate the register map.
