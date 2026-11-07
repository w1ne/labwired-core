# NXP Vybrid imported profile

## Measured scope

Renode's [Vybrid platform description](https://github.com/renode/renode/blob/master/platforms/cpus/vybrid.repl)
defines a Cortex-M4 CPU and 512 KiB of OCRAM at `0x3F000000`. The LabWired
profile includes that CPU and OCRAM. UART0-2 and the
GPIO0-4 data blocks are also mapped. The Cortex-A5, OCRAM aliases, boot ROM,
DDR, other peripherals, and reset-vector boot are outside the measured profile.

## Smoke evidence

The smoke firmware (`examples/renode-onboarding/colibri-vf61-cortex-m4-smoke.S`,
built by `scripts/build_colibri_vf61_cortex_m4_smoke.sh`) is loaded directly
into OCRAM. It sets UART0 C2 to TE|RE and reads it back. It checks the S1 idle
flags (TDRE|TC). It drives GPIO1 PDOR through PSOR, PCOR and PTOR
(`0xF0 -> 0xC0 -> 0x41`) and transmits the results on UART0, polling
S1.TDRE. The exact imported system ran 100,000 instructions and passed 4/4
assertions. These are the UART0 lines `VF61 M4 SMOKE`, `uart0 c2=0c`,
`gpio1 pdor=00000041` and `pass=1f`, plus the result word `0x5F61001F` at
`0x3F07FFF0` and the `max_steps` stop. Negative control: the same firmware on
the previous CPU-only chip profile fails 0/4 with a BusFault at UART0 C2
(`0x40027003`).

UART0-2 use the Kinetis 8-bit UART model, which has the Vybrid UART register
layout. GPIO0-4 use the Kinetis GPIO data-register model. The model also has a
PDDR register at `0x14` that Vybrid lacks (direction lives in IOMUXC).
Interrupts are not claimed: the MSCM IRQ router that feeds the M4 NVIC is not
modelled. This is local LabWired CLI evidence. It does not cover reset-vector
boot, the A5, baud timing, FIFOs, DMA, Renode execution or hardware.

Evidence is under `docs/coverage/renode-onboarding/metrics/vybrid/`, including
the target metric, JUnit result, simulator result, snapshot, and run log.
