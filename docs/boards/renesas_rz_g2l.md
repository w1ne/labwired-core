# Renesas RZ/G2L imported profile

The onboarding profile is narrowed to direct integer execution on the Cortex-M33
and its 64 KiB non-secure code SRAM at `0x00010000` plus 64 KiB data SRAM at
`0x20010000`. The upstream Renode platform defines both windows for `cpu_m33`;
it separately models the Cortex-A55 cluster, another SRAM bank, aliases, and
RZ/G2L peripherals. Those other parts are outside this profile and this result. See the
[upstream Renode platform description](https://github.com/renode/renode/blob/master/platforms/cpus/renesas_rz_g2l.repl#L14-L80).

The profile also maps SCIF0-4 at the non-secure M33 aliases
`0x4004B800`..`0x4004C800`. Renode models them as `SCI.RenesasRZG_SCIFA`;
LabWired uses the `scifa` uart profile (16-bit SMR/BRR/SCR/FTDR/FSR/FRDR/FCR/
FDR/SPTR/LSR/SEMR/FTCR, TX dropped while SCR.TE is clear, BRR locked while
TE|RE are set). They are polled only: no IRQ is declared, so the NVIC lines and
the IM33 controller are not claimed. GPT, GTM, watchdog, MHU, RSPI, GPIO, RIIC,
DMAC, SYC and CPG are not mapped, because LabWired has no model for them.

The focused firmware (`examples/renode-onboarding/rzg2l-cm33-cpu-smoke.S`)
sets MSP into the M33 data SRAM. It checks the SCIF2 FSR reset value (TDFE|TEND),
BRR readback, and the BRR lock under TE|RE. It also checks SCR readback, the
write-1 FCR resets, the FTCR reset value, and that SCIF0 keeps its own BRR. It
writes a `#` before setting SCR.TE, then prints a banner and the results on
SCIF2 (the Zephyr `rzg2l_smarc` cm33 console). Finally it stores `0x2620003F` at
`0x2001FFF0`. The smoke asserts an anchored regex (so the pre-TE byte must have
been dropped), the ordered UART lines, the result word and `max_steps`. The CLI's
initial SP is outside the M33 RAM window, so the firmware sets MSP before using
the stack. This direct-ELF run does not test reset-vector boot. This is local
LabWired CLI smoke evidence for the M33 + SCIF subset. It is not a hosted
`labwired_verify` verdict, an A55 run, interrupt validation, boot flow, or
hardware confirmation.

The earlier generated import validation used the broad fallback memory map and
faulted on a stack write at `0x2001ffe0`. Its complete report is preserved under
[`attempt-history`](../coverage/renode-onboarding/attempt-history/renesas_rz_g2l/2026-10-18-generated-import-validation/onboarding-summary.md).
It is not counted as a current onboarding run.

Current evidence is under
[`metrics/renesas_rz_g2l`](../coverage/renode-onboarding/metrics/renesas_rz_g2l/):
build/run logs, simulator result, snapshot, and smoke assertion output.
