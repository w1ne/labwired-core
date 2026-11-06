# Renode onboarding scope: egis_et171

The imported board descriptor remains `verified: false`. Renode defines an
Andes D25F core with `rv32gc_xandes`, SRAM at `0x00000000` (384 KiB), a 64 KiB
ROM at `0x70000000`, and flash at `0x80000000` (1 MiB) in its
[ET171 platform file](https://github.com/renode/renode/blob/master/platforms/cpus/egis_et171.repl).

The LabWired onboarding profile is intentionally narrower: it selects the
generic `rv32im` CPU path, 384 KiB of SRAM, and the sourced 1 MiB flash window.
The direct-ELF smoke checks an RV32IM multiply result written to SRAM and the
100,000-instruction stop condition. It does not test Renode's reset-vector/ROM
flow, the D25F's Xandes, compressed, atomic, or floating-point extensions,
interrupts, or any ET171 peripheral. The imported UART, timer, GPIO, watchdog,
SPI, RTC, system-control, and crypto declarations are omitted because this
engine has not demonstrated those models on the ET171 profile.

On 2026-11-06, the direct-ELF smoke executed 100,000 RV32IM instructions
(400,000 cycles), wrote `0x143` to SRAM at `0x1000`, passed both assertions,
and recorded zero peripheral accesses. The run assumes an ideal supply rail;
power-on and brown-out behavior were not simulated.

## Measurement

The current result is recorded in
[`docs/coverage/renode-onboarding/metrics/egis_et171/`](../coverage/renode-onboarding/metrics/egis_et171/)
and in the generated
[Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md).
This CLI smoke is scoped evidence for generic RV32IM execution and SRAM only;
it does not establish full-board behavior or hardware confirmation.
