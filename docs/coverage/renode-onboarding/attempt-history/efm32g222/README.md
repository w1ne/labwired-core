# EFM32G222 smoke attempt history

## Attempt 01

The initial run built an ELF with entry point `0x0`, but the imported G222
profile places flash at `0x08000000`. Execution therefore stopped before the
first instruction with a memory access violation at `0xfffffffe`; it emitted no
UART output and passed neither smoke assertion.

Added the target-specific linker memory map at `0x08000000` and retained the
initial metrics, build/run logs, simulator result, snapshot, and JUnit output in
this directory. The corrected run is recorded in
`../../metrics/efm32g222/` and the scoreboard reflects that latest attempt.
