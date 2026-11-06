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

The exact imported system ran a direct-ELF integer firmware loop for 100,000
instructions and stopped at `max_steps`. The smoke passed its stop-reason
assertion and recorded zero peripheral accesses. The result also records two
unmapped reads at `0x0` and `0x4`, plus an ideal-rail assumption; neither was a
smoke blocker. This establishes execution on the bounded CPU-and-memory profile;
it is not validation of the full SoC, board peripherals, Linux boot, Renode, or
physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/arvsom/` contains the
per-target metrics, build and run logs, simulator result, snapshot, UART capture,
and JUnit assertions.
