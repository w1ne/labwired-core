Negative control (2026-11-09), final firmware and engine: SYSCFG with
`profile: stm32f0` (the F07x CFGR1 mask) instead of `stm32f04x`. FAIL 4/6:
`syscfg cfgr1=0x7F3F7F00` — no PA11_PA12_RMP bit, F07x-only DMA remap bits
13:14 / 24:30 kept, I2C_FMP_PA9/PA10 (22:23) dropped. Summary 0x0423FFFF
(bit 18 missing).
