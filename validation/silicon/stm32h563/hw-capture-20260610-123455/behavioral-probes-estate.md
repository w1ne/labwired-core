# Behavioral write/read probes, round 2 (peripheral estate) — NUCLEO-H563ZI, 2026-06-10

CPU reset-halted, SWD, bus clocks enabled first, state restored after each
probe. Backs the estate onboarding (TIM/I2C/UART/WDT/CRC/RNG/LPTIM wiring).

| # | Probe | Silicon result |
|---|-------|----------------|
| P1 | TIM2 PSC=7, ARR=0x10000, EGR.UG, SR clear, CEN | All round-trip; UG→UIF=1; write-0 clears; CNT advanced 0x57F7→0xA800 while CPU halted (no DBGMCU freeze by default) |
| P2 | I2C1 OAR1=0x80000052, TIMINGR, CR1.PE | OAR1 reads 0x52 (bit 31 dropped — not a field); TIMINGR/PE round-trip |
| P3 | IWDG KR=0x5555, PR=2, RLR=0xABC (LSI OFF) | PR reads 0, RLR reads 0xFFF, SR.PVU stuck 1 — **writes never commit without LSI**; KR=0xCCCC never written |
| P4 | WWDG CFR=0x60 | Round-trips; CR read 0x1A — counter decrements live with WDGA clear |
| P5 | CRC reset + DR=0x12345678 | DR = **0xDF8A8A2B** (matches poly 0x04C11DB7 / init 0xFFFFFFFF reference and the sim model — pinned in `crc_compute_matches_silicon`) |
| P6 | RNG CR reset readback; RNGEN with no kernel clock | CR reset 0x00800D00 confirmed; SR = 0x22 (CECS|CEIS clock error), no DRDY |
| P7 | USART1 BRR=0x116, CR1=UE\|TE | Round-trips |
| P8 | LPTIM1 CFGR=0x20 | Round-trips |

Model deltas taken: IWDG LSI-sync and RNG clock-error divergences documented
in the chip yaml; tier1 `wdt` check rewritten silicon-true (LSI bring-up via
RCC_BDCR LSION→LSIRDY + SR.PVU/RVU polling); CRC compute pinned as a
behavioral conformance assertion.
