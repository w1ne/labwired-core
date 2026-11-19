# ARV-SOM Renode onboarding smoke

## Measured profile

The machine import described the ARV-SOM as a PolarFire SoC module, but
Renode's `platforms/boards/arvsom.repl` does not model PolarFire: it is
`using "platforms/cpus/starfive-jh7100.repl"` plus a `MappedMemory` DDR at
`0x80000000`. The engine profile follows that Renode platform: one U74 hart in
the engine's RV64IMAC subset, an 8 MiB window at the start of that DDR, and the
JH7100 UART0 at `0x11870000` (NS16550 with 32-bit wide registers, matched by the
engine's DesignWare 16550 profile). The 8 GiB DDR range is not treated as
evidence of physical module capacity. This profile does not claim PolarFire
silicon, RV64GC floating point, supervisor mode, Linux boot, the second hart,
interrupts, UART RX, or other SoC/board peripherals.

## Smoke evidence

The exact imported system ran direct-ELF RV64IMAC firmware for 100,000
instructions and stopped at `max_steps`. It executes `mul` on two 64-bit
operands with nonzero high halves and stores `0x0000000500000003` at
`0x80700000`, then polls UART0 LSR.THRE and prints
`ARV-SOM JH7100 UART0 ok; mul=0000000500000003`. The smoke asserts both result
halves, the UART text and the stop reason (4/4).

Negative control: with UART0 removed from the profile the same firmware stops
with a memory violation at `0x11870014` (UART0 LSR) and fails 2/4 checks
(`attempt-history/arvsom/2026-11-08-uart0-negative-control/`). The earlier
CPU-only measurement is kept in
`attempt-history/arvsom/2026-11-08-before-uart0-smoke/`.

This is evidence for the asserted instruction, DDR window and UART TX path on
the bounded profile; it is not validation of the full SoC, Linux boot, Renode,
or physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/arvsom/` contains the
per-target metrics, build and run logs, simulator result, snapshot, UART capture,
and JUnit assertions.
