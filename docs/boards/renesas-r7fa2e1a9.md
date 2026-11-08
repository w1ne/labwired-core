# Renesas R7FA2E1A9 (RA2E1) imported profile

## Measured profile

LabWired's onboarding profile for the imported Renode `renesas-r7fa2e1a9`
target is narrowed to what the engine executes and the smoke exercises. It
follows Renode's
[`renesas-r7fa2e1a9.repl`](https://github.com/renode/renode/blob/master/platforms/cpus/renesas-r7fa2e1a9.repl):

- **CPU:** Cortex-M23 (ARMv8-M Baseline).
- **Memory:** 128 KiB code flash at `0x00000000` and 16 KiB SRAM at
  `0x20004000`. The 4 KiB data flash at `0x40100000` is omitted.
- **SCI9** at `0x40070120` (Renode `RenesasRA6M5_SCI`, FIFO disabled on this
  channel). This uses the classic Renesas SCI UART model: SCR readback,
  TDR at `+0x03`, SSR at `+0x04` reading TDRE|TEND, RDR at `+0x05`. Polled TX
  only: no IRQ is declared, ICU event routing is not mapped, and baud timing
  is not claimed.
- **PORT9** at `0x40040120` with the PFS block at `0x40040800`: PCNTR1
  (PODR/PDR) and PCNTR3 (POSR/PORR) on the RA port model; the smoke does
  not exercise PCNTR2/PCNTR4.
  P915 is LED1 on the EK-RA2E1 (FSP `ra/board/ra2e1_ek/board_leds.c`).

Not modelled and omitted from the descriptor: the ICU, GPT, AGT0/1, IIC0,
SCI0/1/2, ports 0..5, the `RenesasRA_GPIOMisc` block and Renode's
`sysc_oscsf` Python script.

The earlier report on this page described a 2026-10-18 run of a Thumb-2
Cortex-M3 fixture (`uart-ok-thumbv7m.elf`) against the unnarrowed import. That
run hit a memory violation at step 2 and listed zero-answering stubs as
hardware coverage. It never exercised an RA2E1 peripheral and is withdrawn.

## Smoke evidence

The firmware crate `crates/firmware-renesas-r7fa2e1a9-smoke`
(`thumbv8m.base-none-eabi`) stores a MULS result, sets `SCR.TE` on SCI9 and
stores the SCR readback, then polls `SSR.TDRE` before each byte of
`RA2E1 SCI9 SMOKE` and waits for `SSR.TEND`. It makes P915 an output through
PCNTR1, sets it through POSR, and stores the PODR readback. Then it toggles
P915 through POSR/PORR as a heartbeat. The firmware does not program the PFS
pin mux, MSTPCRB or the clock tree. Firmware for real silicon must program all
three.

It ran **200,000 instructions** and passed 6/6 checks: the MULS word `0x143`
at `0x20004000`, the SCR readback `0x20` at `0x20004004`, the PODR readback
`0x8000` at `0x20004008`, the UART banner, at least 4 edges on `port9:15`,
and `max_steps`.

With SCI9 removed from the descriptor, the same firmware takes a precise
BusFault (BFAR `0x40070122`) on its first SCI9 access and passes 2/6
(`docs/coverage/renode-onboarding/attempt-history/renesas-r7fa2e1a9/2026-11-08-sci9-negative-control/`).

The exact system manifest is `configs/systems/onboarding/renesas-r7fa2e1a9.yaml`
and the smoke script is `examples/renode-onboarding/renesas-r7fa2e1a9-smoke.yaml`.
Per-target metrics, JUnit, UART output, simulator result, snapshot and logs are
under `docs/coverage/renode-onboarding/metrics/renesas-r7fa2e1a9/`.

This is local CLI simulation evidence for the smoke assertions. It does not
show whole-SoC coverage, Renode equivalence or hardware behaviour. See the
[generated scoreboard](../coverage/renode-onboarding-scoreboard.md) for the
current global totals.
