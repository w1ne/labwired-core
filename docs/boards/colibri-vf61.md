# Toradex Colibri VF61 imported profile

## Measured scope

Toradex identifies the Colibri VF61 as a Vybrid VF6xx system-on-module with a
Cortex-A5 and a secondary Cortex-M4. Renode's [Vybrid platform description](https://github.com/renode/renode/blob/master/platforms/cpus/vybrid.repl)
maps `cpuM4` as a Cortex-M4 and on-chip OCRAM at `0x3F000000` with a 512 KiB
window. The LabWired profile includes one Cortex-M4 execution context and this
OCRAM window. UART0-2 and the GPIO0-4 data blocks are also mapped. The A5, OCRAM aliases,
boot ROM, DDR, other peripherals, and reset-vector boot are outside the
measured profile. See Toradex's [Colibri VF61 page](https://developer.toradex.com/hardware/colibri-som-family/modules/colibri-vf61/)
for the module overview.

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

Evidence: `docs/coverage/renode-onboarding/metrics/colibri-vf61/` contains the
per-target metric, JUnit result, simulator result, snapshot, and run log.
