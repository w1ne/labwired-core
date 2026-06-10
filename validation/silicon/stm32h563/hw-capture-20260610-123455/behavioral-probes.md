# Behavioral write/read probes — NUCLEO-H563ZI, 2026-06-10

CPU reset-halted, SWD (same OpenOCD recipe as registers.txt). Each probe
restores the touched state. Drove the silicon-probed write semantics of the
`stm32h5` RCC model (labwired-core PR #229).

| # | Probe | Silicon result |
|---|-------|----------------|
| T1 | CFGR1 SW=001 while CSI OFF | CFGR1=0x00000001 — SWS did NOT mirror (switch gated on source ready) |
| T2 | CR CSION → SW=001 | CR=0x32B (CSIRDY latched); CFGR1=0x00000009 — switch completes |
| T3 | SW=000 back to HSI | CFGR1=0x00000000 |
| T4 | HSICFGR trim 0x40→0x55 | 0x0055050C — CAL tracks TRIM (+0x15: 0x4F7→0x50C); restore → 0x004004F7 |
| T5 | CSICFGR trim 0x20→0x15 | 0x0015007C — same law (−0xB: 0x87→0x7C); restore → 0x00200087 |
| T6 | HSEON alone; +HSEBYP | CR=0x0003002B both — HSERDY latches even w/o BYP (board feeds HSE from STLINK MCO); restore clears RDY |
| T7 | AHB2ENR = 0xC000007F | reads back exactly; restored |
| T8 | APB1HENR 0x20 / APB3ENR 0x840 | round-trip OK; restored |
| T9 | GPIOA MODER PA0 analog→output | 0xABFFFFFD reads back; restored |
| T10 | SysTick CALIB write 0x12345678 | 0x001003E8 — read-only confirmed |
| T11 | RSR RMVF bit 16 / bit 23 | bit 16 ignored (0x0C000000 unchanged); bit 23 clears to 0 |

Note: T11 cleared the sticky RSR flags. PINRST re-latches on the next NRST;
BORRST only returns after a power cycle — re-captures of RSR show 0x04000000
until the board is power-cycled. The committed reset capture (0x0C000000) is
the power-on truth.
