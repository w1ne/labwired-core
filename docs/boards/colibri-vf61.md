# Toradex Colibri VF61 imported profile

## Measured scope

Toradex identifies the Colibri VF61 as a Vybrid VF6xx system-on-module with a
Cortex-A5 and a secondary Cortex-M4. Renode's [Vybrid platform description](https://github.com/renode/renode/blob/master/platforms/cpus/vybrid.repl)
maps `cpuM4` as a Cortex-M4 and on-chip OCRAM at `0x3F000000` with a 512 KiB
window. The LabWired profile includes one Cortex-M4 execution context and this
OCRAM window. The A5, OCRAM aliases, boot ROM, DDR, peripherals, and reset-vector
boot are outside the measured profile. See Toradex's [Colibri VF61 page](https://developer.toradex.com/hardware/colibri-som-family/modules/colibri-vf61/)
for the module overview.

## Smoke evidence

The committed Cortex-M4 Thumb integer-loop ELF was loaded directly into OCRAM.
The exact imported system executed 100,000 instructions, reached the
`max_steps` limit, and passed its 1/1 smoke assertion with zero peripheral
accesses. The result records unmapped startup reads at `0x0` and `0x4`, plus an
ideal-rail assumption. Reset-vector boot remains unverified. This is local CLI
evidence for the exercised integer subset; it does not validate the M4 reset
path, the A5, other M4 instructions, peripherals, Renode, or physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/colibri-vf61/` contains the
per-target metric, JUnit result, simulator result, snapshot, and run log.
