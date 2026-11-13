First L072 run of the final firmware on the engine before this round's DAC
and RNG fixes (2026-11-09). FAIL: `dac dor1=0x00000000 direct=0 dual=0
held=0 swtrig=0` — the old model put DOR1/DOR2 at 0x28/0x2C (0x28 is
DHR8RD), never moved the 8-bit / left-aligned views and ignored SWTRIGR;
`rng draws=0` — every DR read returned the same 0xCAFEBABE. Summary
0x720FFFFF (bits 20 and 21 missing). Kept as the before-state of the
dac.rs / rng.rs fixes.
