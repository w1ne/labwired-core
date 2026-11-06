# NXP Vybrid imported profile

## Measured scope

Renode's [Vybrid platform description](https://github.com/renode/renode/blob/master/platforms/cpus/vybrid.repl)
defines a Cortex-M4 CPU and 512 KiB of OCRAM at `0x3F000000`. The LabWired
profile includes that CPU and OCRAM. The Cortex-A5, OCRAM aliases, boot ROM,
DDR, peripherals, and reset-vector boot are outside the measured profile.

## Smoke evidence

The shared Cortex-M4 Thumb integer-loop ELF was loaded directly into the
imported Vybrid system's OCRAM. The run executed 100,000 instructions, reached
`max_steps`, and passed its 1/1 smoke assertion with zero peripheral accesses.
The result records two unmapped reads at `0x0` and `0x4`, and assumes an ideal
power rail. This is local LabWired CLI evidence for the exercised integer
subset; it is not reset-vector boot, peripheral behavior, Renode execution, or
hardware confirmation.

Evidence is under `docs/coverage/renode-onboarding/metrics/vybrid/`, including
the target metric, JUnit result, simulator result, snapshot, and run log.
