# Renesas RZ/G2L imported profile

The onboarding profile is narrowed to direct integer execution on the Cortex-M33
and its 64 KiB non-secure code SRAM at `0x00010000` plus 64 KiB data SRAM at
`0x20010000`. The upstream Renode platform defines both windows for `cpu_m33`;
it separately models the Cortex-A55 cluster, another SRAM bank, aliases, and
RZ/G2L peripherals. Those other parts are outside this profile and this result. See the
[upstream Renode platform description](https://github.com/renode/renode/blob/master/platforms/cpus/renesas_rz_g2l.repl#L14-L80).

The focused firmware sets MSP into the M33 data SRAM, exercises stack push/pop,
then runs a small Thumb integer loop from the code SRAM for 100,000 instructions.
It checks the bounded `max_steps` stop condition and does not access MMIO. The
CLI's initial SP is outside the M33 RAM window, so the firmware sets MSP before
using the stack. Startup also records unmapped vector reads at `0x0` and `0x4`;
this direct-ELF run does not test reset-vector boot. This is local LabWired CLI
smoke evidence for the M33 execution subset, not a hosted `labwired_verify`
verdict, an A55 run, peripheral validation, boot flow, or hardware confirmation.

The earlier generated import validation used the broad fallback memory map and
faulted on a stack write at `0x2001ffe0`. Its complete report is preserved under
[`attempt-history`](../coverage/renode-onboarding/attempt-history/renesas_rz_g2l/2026-10-18-generated-import-validation/onboarding-summary.md).
It is not counted as a current onboarding run.

Current evidence is under
[`metrics/renesas_rz_g2l`](../coverage/renode-onboarding/metrics/renesas_rz_g2l/):
build/run logs, simulator result, snapshot, and smoke assertion output.
