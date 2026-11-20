# Renesas EK-RA2E1 imported board profile

## Measured profile

LabWired's onboarding profile for the imported Renode `renesas-ek_ra2e1`
board is narrowed to what the engine executes and the smoke exercises. It
follows Renode's
[`renesas-ek_ra2e1.repl`](https://github.com/renode/renode/blob/master/platforms/boards/renesas-ek_ra2e1.repl),
which includes
[`renesas-r7fa2e1a9.repl`](https://github.com/renode/renode/blob/master/platforms/cpus/renesas-r7fa2e1a9.repl):

- **CPU:** Cortex-M23 (ARMv8-M Baseline).
- **Memory:** 128 KiB code flash at `0x00000000` and 16 KiB SRAM at
  `0x20004000`. The 4 KiB data flash at `0x40100000` is omitted.
- **SCI9** at `0x40070120` on the classic Renesas SCI UART model (Renode
  `RenesasRA6M5_SCI`, FIFO disabled on this channel). Polled TX only: no IRQ
  is declared, ICU event routing is not mapped, and baud timing is not
  claimed.
- **PORT0** at `0x40040000` and **PORT9** at `0x40040120` on the RA port
  model, with the PFS block at `0x40040800`.
- **Board wiring** (system `board_io`), as in the Renode board file: LED1
  blue P915, LED2 green P914, LED3 red P913; buttons S1 P004 and S2 P015.
  The buttons are active low here. The EK-RA2E1 switches pull the pin to
  ground against a pull-up. Renode's `Button` model drives the pin high when
  pressed instead.

Not modelled and omitted from the descriptor: the ICU (so no button
interrupts), GPT, AGT0/1, IIC0, SCI0/1/2, ports 1..5, the
`RenesasRA_GPIOMisc` block and Renode's `sysc_oscsf` Python script.

The earlier report on this page described a 2026-10-18 run of a Thumb-2
Cortex-M3 fixture (`uart-ok-thumbv7m.elf`) against the unnarrowed import. That
run hit a memory violation at step 2, and its coverage table listed
zero-answering stubs and LED/button pin numbers as base addresses. It never
exercised an RA2E1 peripheral, so it is withdrawn.

## Smoke evidence

The firmware crate `crates/firmware-renesas-ek-ra2e1-smoke`
(`thumbv8m.base-none-eabi`) does the following:

1. Stores a MULS result.
2. Sets `SCR.TE` on SCI9 and stores the SCR readback.
3. Polls `SSR.TDRE` before each byte of `EK-RA2E1 SCI9 SMOKE`, then waits
   for `SSR.TEND`.
4. Makes P913..P915 outputs through PCNTR1, turns LED1 and LED3 on and LED2
   off through POSR/PORR, and stores the PODR readback.
5. Stores the idle S1 level (PCNTR2 PIDR bit 4).
6. Loops: toggles LED1 as a heartbeat, mirrors S1 onto LED2, and counts S1
   presses (falling edges) in SRAM.

The firmware does not program the PFS pin mux, MSTPCRB or the clock tree.
Firmware for real silicon must program all three.

The smoke script presses S1 twice (stimuli at 100k/160k and 220k/280k
cycles). The run took **400,000 instructions** and passed 10/10 checks:

- the MULS word `0x143`
- the SCR readback `0x20`
- the PODR readback `0xA000`
- the idle S1 level `0x10`
- a press count of `2`
- the UART banner
- at least 4 edges on `port9:15` (LED1)
- at least 4 edges on `port9:14` (LED2, following S1)
- `max_steps`

Negative control: the same firmware with the stimuli removed passes 8/10. The
press count stays 0 and LED2 has no edges
(`docs/coverage/renode-onboarding/attempt-history/renesas-ek_ra2e1/2026-11-08-no-press-negative-control/`).

The exact system manifest is `configs/systems/onboarding/renesas-ek_ra2e1.yaml`
and the smoke script is `examples/renode-onboarding/renesas-ek_ra2e1-smoke.yaml`.
Per-target metrics, JUnit, UART output, simulator result, snapshot and logs are
under `docs/coverage/renode-onboarding/metrics/renesas-ek_ra2e1/`.

This is local CLI simulation evidence for the smoke assertions. It does not
show whole-SoC coverage, Renode equivalence or hardware behaviour. See the
[generated scoreboard](../coverage/renode-onboarding-scoreboard.md) for the
current global totals.
