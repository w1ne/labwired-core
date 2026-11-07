# ARV-SOM Renode onboarding smoke

## Measured profile

The machine import identified the ARV-SOM as a PolarFire SoC RISC-V target, but
its generated chip profile was mislabeled ARM and contained unrelated
zero-answering reset windows. The profile now exposes only the engine's RV64IMAC
CPU subset and an 8 MiB executable window at the imported RAM address
`0x80000000`. The import's original 8 GiB range is not treated as evidence of
physical module capacity. This profile does not claim the RV64GC
floating-point extensions, supervisor mode, Linux boot, FPGA fabric, or
peripheral behavior.

## Smoke evidence

The exact imported system ran direct-ELF RV64IMAC firmware for 100,000
instructions and stopped at `max_steps`. It executes `mul` on two 64-bit
operands with nonzero high halves, then stores the result `0x0000000500000003`
at `0x80700000`; the smoke asserts both 32-bit halves and the stop reason.
The run recorded zero peripheral accesses. The result also records an
ideal-rail assumption; power-on and brown-out behavior were not tested. This
establishes execution of the asserted integer instruction on the bounded
CPU-and-memory profile; it is not validation of the full SoC, board
peripherals, Linux boot, Renode, or physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/arvsom/` contains the
per-target metrics, build and run logs, simulator result, snapshot, UART capture,
and JUnit assertions.
