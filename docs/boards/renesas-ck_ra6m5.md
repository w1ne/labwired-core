# Renesas CK-RA6M5 imported board profile

## Measured profile

LabWired's onboarding profile for the imported Renode `renesas-ck_ra6m5`
board is narrowed to what the engine executes and the smoke exercises. It
follows Renode's
[`renesas-ck_ra6m5.repl`](https://github.com/renode/renode/blob/master/platforms/boards/renesas-ck_ra6m5.repl),
which includes
[`renesas-r7fa6m5b.repl`](https://github.com/renode/renode/blob/master/platforms/cpus/renesas-r7fa6m5b.repl):

- **CPU:** Cortex-M33 (ARMv8-M Mainline).
- **Memory:** 2 MiB code flash at `0x00000000` and 512 KiB SRAM at
  `0x20000000`. The 8 KiB data flash at `0x08000000` is omitted.
- **SCI0** at `0x40118000` on the classic Renesas SCI UART model (Renode
  `RenesasRA6M5_SCI` with `enableFIFO: false`; Renode's RA6M5 tests use sci0
  as the console). Polled TX only: no IRQ is declared, ICU event routing is
  not mapped, and baud timing is not claimed.
- **PORT6** at `0x400800C0` and **PORT8** at `0x40080100` on the RA port
  model, with the PFS block at `0x40080800`.
- **R_MSTP** at `0x40084000`: SVD-derived register readback only. Clearing a
  module-stop bit does not gate anything.
- **Board wiring** (system `board_io`), as in the Renode board file: LEDs red
  P610, green P609, blue P601; the button on P804. The button polarity follows
  Renode's `Button` model, which drives the pin high while pressed. The
  CK-RA6M5 schematic was not checked.

Not modelled and omitted from the descriptor: the ICU (so no button
interrupts), GPT, AGT0..5, IIC0 and the board's I2C sensors (HS3001,
ZMOD4410/4510, ICP-101xx, ICM-20948), SCI1..9, the other ports, the
`RenesasRA_GPIOMisc` block and Renode's `sysc_oscsf` Python script. The
promoted RA6M5 part with ICU, AGT, GPT and FIFO SCI is [`r7fa6m5bh`](r7fa6m5bh.md)
(board: [EK-RA6M5](ek-ra6m5.md)).

The earlier report on this page described a 2026-10-18 run of a Thumb-2
Cortex-M3 fixture (`uart-ok-thumbv7m.elf`) against the unnarrowed import,
whose code flash sat at the data-flash address `0x08000000`. It never
exercised an RA6M5 peripheral, so it is withdrawn.

## Smoke evidence

The firmware crate `crates/firmware-renesas-ck-ra6m5-smoke`
(`thumbv8m.main-none-eabi`) does the following:

1. Stores a MULS result.
2. Sets `SCR.TE` on SCI0 and stores the SCR readback.
3. Polls `SSR.TDRE` before each byte of `CK-RA6M5 SCI0 SMOKE`, then waits
   for `SSR.TEND`.
4. Makes P601, P609 and P610 outputs through PCNTR1, turns red and blue on
   and green off through POSR/PORR, and stores the PODR readback.
5. Stores the idle button level (PORT8 PCNTR2 PIDR bit 4).
6. Loops: toggles the red LED as a heartbeat, mirrors the button onto the
   green LED, and counts presses (rising edges) in SRAM.

The firmware does not program the PFS pin mux, MSTPCRB or the clock tree.
Firmware for real silicon must program all three.

The smoke script presses the button twice (stimuli at 100k/160k and
220k/280k cycles). The run took **400,000 instructions** and passed 11/11
checks:

- the MULS word `0x143`
- the SCR readback `0x20`
- the PODR readback `0x0402`
- the idle button level `0`
- a press count of `2`
- MSTPCRA and MSTPCRB reset values
- the UART banner
- at least 4 edges on `port6:10` (red LED)
- at least 4 edges on `port6:9` (green LED, following the button)
- `max_steps`

Negative control: the same firmware with the stimuli removed passes 9/11. The
press count stays 0 and the green LED has no edges
(`docs/coverage/renode-onboarding/attempt-history/renesas-ck_ra6m5/2026-11-08-no-press-negative-control/`).

The exact system manifest is `configs/systems/onboarding/renesas-ck_ra6m5.yaml`
and the smoke script is `examples/renode-onboarding/renesas-ck_ra6m5-smoke.yaml`.
Per-target metrics, JUnit, UART output, simulator result, snapshot and logs are
under `docs/coverage/renode-onboarding/metrics/renesas-ck_ra6m5/`.

This is local CLI simulation evidence for the smoke assertions. It does not
show whole-SoC coverage, Renode equivalence or hardware behaviour. See the
[generated scoreboard](../coverage/renode-onboarding-scoreboard.md) for the
current global totals.
