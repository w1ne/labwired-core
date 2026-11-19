# Renesas R7FA8M1A (RA8M1) imported profile

## Measured profile

LabWired's onboarding profile for the imported Renode `renesas-r7fa8m1a`
target is narrowed to what the engine executes and the smoke exercises. It
follows Renode's
[`renesas-r7fa8m1a.repl`](https://github.com/renode/renode/blob/master/platforms/cpus/renesas-r7fa8m1a.repl)
and the model-backed RA8M1 part in `configs/chips/r7fa8m1ah.yaml`:

- **CPU:** Cortex-M85, scalar Armv8-M Mainline only. Helium/MVE and
  TrustZone are not claimed.
- **Memory:** 2 MiB code flash at `0x02000000`, 896 KiB SRAM at
  `0x22000000`, ITCM/DTCM 64 KiB at `0x00000000`/`0x20000000`, 12 KiB data
  flash at `0x27000000`. The non-secure aliases, factory flash, standby SRAM
  and option-setting memory are omitted.
- **SYSTEM** at `0x4001E000` (`ra8_sysc`): SCICKDIVCR/SCICKCR with the
  CKSREQ/CKSRDY switch handshake. This is what Renode's `SYSC_SCICKCR`
  Python peripheral provides.
- **MSTP** at `0x40203000`: the MSTPCRA-E register file from the SVD (reset
  values, readback). No module is gated by it.
- **SCI0** at `0x40358000` (Renode `RenesasRA8M1_SCI`, 8 MHz) on the
  `sci_b` UART model: CCR0-3, CESR TE/RE mirror, CSR TDRE/TEND, TDR. Polled
  TX only: no ICU event is declared, so no RXI/TXI/TEI is raised.
- **PORT6** at `0x404000C0` with PFS at `0x40400800`: PCNTR1 (PDR/PODR) and
  PCNTR3 (POSR/PORR) on the RA port model. P600 is LED1 (blue) on the
  EK-RA8M1.

Not modelled and omitted from the descriptor: the ICU, GPT/GPT_NS, AGT0/1,
SCI1-4 and SCI9, IIC0/1, ports 0-5 and 7-B, and `RenesasRA_GPIOMisc`
beyond the PWPR/PWPRS bytes inside the PFS window. The EK-RA8M1 board
(SCI9 console, three LEDs, two switches) is the separate
`renesas-ek_ra8m1` target.

The earlier report on this page described a 2026-10-18 run of a Thumb-2
Cortex-M3 fixture (`uart-ok-thumbv7m.elf`) against the unnarrowed import. That
run hit a memory violation at step 2 and listed zero-answering stubs as
hardware coverage. It never exercised an RA8M1 peripheral and is withdrawn.

## Smoke evidence

The firmware crate `crates/firmware-renesas-r7fa8m1a-smoke`
(`thumbv8m.main-none-eabi`) stores a MUL result, switches SCICLK to MOCO
with the FSP CKSREQ/CKSRDY handshake under PRCR.PRC0, and stores the SCICKCR
readback. It clears MSTPCRB.MSTPB31 (SCI0) and opens SCI0 in the
`r_sci_b_uart.c` order (CCR0 off, wait CESR clear, CFCLR, CCR1-3, TE|RE,
wait CESR.TIST/RIST). It stores the CCR3 readback, polls CSR.TDRE before
each byte of `RA8M1 SCI0 SMOKE` and waits for CSR.TEND. It then makes P600
an output, sets it through POSR, stores the PODR readback, and toggles P600
through POSR/PORR. The firmware does not route the SCI0 pads through PmnPFS.
Firmware for real silicon must.

It ran **400,000 instructions** and passed 9/9 checks: the MUL word `0x143`,
SCICKCR `0x01`, CCR3 `0x1200`, PODR6 `0x0001` (at `0x22000000..0x2200000C`),
MSTPCRB `0x7FFFFFFF`, the UART banner, at least 4 edges on `port6:0`, and
`max_steps`.

With SCI0 removed from the descriptor, the same firmware takes a precise
BusFault (BFAR `0x40358008`) on its first SCI0 access and passes 4/8
(`docs/coverage/renode-onboarding/attempt-history/renesas-r7fa8m1a/2026-11-08-sci0-negative-control/`).

The exact system manifest is `configs/systems/onboarding/renesas-r7fa8m1a.yaml`
and the smoke script is `examples/renode-onboarding/renesas-r7fa8m1a-smoke.yaml`.
Per-target metrics, JUnit, UART output, simulator result, snapshot and logs are
under `docs/coverage/renode-onboarding/metrics/renesas-r7fa8m1a/`.

This is local CLI simulation evidence for the smoke assertions. It does not
show whole-SoC coverage, Renode equivalence or hardware behaviour. See the
[generated scoreboard](../coverage/renode-onboarding-scoreboard.md) for the
current global totals.
