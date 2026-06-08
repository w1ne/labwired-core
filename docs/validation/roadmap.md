# Silicon-validation roadmap

An honest map of how far the simulator-vs-silicon validation goes, and the
prioritized path to "you could stake firmware development on sim == silicon."
Companion to [`pending-silicon-verification.md`](./pending-silicon-verification.md)
(the per-fix ledger) and [`../../hil/README.md`](../../hil/README.md) (the HIL
runner).

## Where we are

The **peripheral-execution oracle** (`crates/hw-oracle/tests/stm32f1_exec_oracle.rs`)
runs real ARM code on the full chip bus in sim *and* on a bench STM32F103 over
SWD, then diffs register read-backs. 12 oracles, all `_hw`/`_diff` byte-exact,
10 real model bugs fixed across four classes:

| Class | Examples |
|-------|----------|
| byte-decomposition (need `write_u32`) | GPIO BSRR atomicity, EXTI SWIER/PR |
| reset / flag values | TIM2 UG compare-match flags, RCC_AHBENR=0x14, EXTI SWIER↔PR |
| reserved / width masking | AFIO MAPR & EXTICR, CRC_IDR 8-bit on F1/L0 |
| key / write-protection | IWDG PR/RLR until KR=0x5555 |

Plus validations where the model was already right (DMA mem-to-mem, GPIO CRL/CRH,
DBGMCU_CR, NVIC ISER/ICER) and the Cortex-M block (NVIC/SCB/DWT) is now wired
into the exec bus.

## Honest gaps (the self-roast)

1. **Coverage is a rounding error.** ~25 registers on **one** chip. No
   `%-silicon-validated` metric (it would be embarrassingly low).
2. **It's all static.** Every oracle is write-register → read-register. We
   deliberately avoid anything timed — timers counting, UART bit timing,
   interrupt latency, PLL lock, flash wait states. `settle_ticks` only checks a
   DMA *end state*, never the trajectory.
3. **No interrupt *delivery*.** We validate the NVIC enable *registers* but never
   take an interrupt — no exception entry/exit, priority, nesting, or faults.
4. **Halt-mode, not free-running.** Run-to-breakpoint over SWD; peripherals can
   behave differently halted (DBGMCU freeze bits literally change this).
5. **The "silicon-verified" stamp decays.** `_hw`/`_diff` are `#[ignore]` and do
   not run in CI — only the `_sim` half does, against expectations transcribed
   from silicon. Nothing catches a *new* divergence after the bench is unplugged.
   → **HIL-in-CI is the fix** (scaffolding landed, inert until a runner is up).
6. **Selection bias.** Hand-picked deterministic, safe targets. The 10 bugs are
   the easy bookkeeping class; zero evidence on clock trees, analog, JIT.
7. **One board, one die.** A single (clone-prone) Blue Pill.
8. **One core.** STM32F103 only — nothing on Xtensa / RISC-V / nRF / RP2040,
   all of which have *open* HW-pending ledger entries.

Verdict: a proven *method* that cleaned up real bugs — but ~2–3% of the way to
trustworthy for F103, ~0% elsewhere.

## Prioritized path

### P0 — HIL in CI (stop the decay) — *scaffolding done, deploy pending*
`hil/` + `core-hil.yml` are in place and proven on the bench F103. Deploy a
self-hosted runner on the Mac server (`hil/README.md`), set the board `active`,
enable the `schedule:` trigger, and make `hil` a required check **only** once the
runner is reliably online. Every supported chip gets a `boards.json` entry.

### P1 — Interrupt delivery oracles — *first oracle landed (ledger #23)*
`exti0_interrupt_delivery` validates the full path — VTOR relocation, NVIC
enable, vectoring, register stacking/unstacking, exception return — byte-exact on
the bench F103. The harness gained `Thumb::Data`, `bx`/`cpsie_i`, per-case
`entry_offset`, and opt-in `live_peripherals`. Next within P1: priority/nesting
(two IRQs), fault handlers, SysTick interrupt. The original design notes (now
implemented):
- **CPU built via `configure_cortex_m`** in `run_capture` (opt-in) so the CPU
  shares the bus NVIC/VTOR (currently `run_capture` builds a bare `CortexM`).
- **Live-peripheral dispatch** (opt-in `live_peripherals`): after each
  `cpu.step()`, call `bus.tick_peripherals_fully()` and
  `cpu.set_exception_pending(irq)` for each returned IRQ — mirroring
  `Machine::step`. Default off, so the 12 static oracles are unaffected.
- **Vector table support**: a `Thumb::Data(u32)` raw-word variant + `bx`/`cpsie_i`
  encoders. Lay out `[vector table][isr ending in BX LR][main]`; the program
  sets VTOR via `STR` to `0xE000ED08` (SCB shares the VTOR `Arc`) and enables the
  IRQ via NVIC ISER. The table sits at `PROG_BASE_HW` (128-byte aligned, VTOR
  rule); the **entry PC is `PROG_BASE_HW + table + isr`** (start of `main`), so
  the harness needs a per-case **entry offset** (sim and HW). `main` ends at the
  auto-appended `B .` terminator (the breakpoint), so the ISR's `BX LR` returns
  into `main` which then settles — no mid-program terminator problem.
- First oracle: EXTI0 → ISR sets a RAM marker + clears `EXTI_PR`; assert the
  marker (proves the ISR ran) and the unstacked register state. Silicon-
  verifiable on the connected F103.

### P2 — Timed oracles — *DWT mechanism oracle landed (ledger #24)*
`dwt_cyccnt_advances` validates the DWT enable→count→read path with a self-relative boolean (the absolute count diverges — the sim is not cycle-accurate). True cycle-fidelity is a known sim limitation, not closeable by an oracle.
DWT is now mapped. Enable `DWT_CYCCNT`, run a known instruction/delay sequence,
compare cycle counts within a tolerance band (sim cycle models aren't
cycle-exact). Then timer counts over a fixed delay, and `systick` reload→COUNTFLAG.

### P3 — `%-silicon-validated` metric
Extend the `register_coverage` machinery: per peripheral, registers an oracle
asserts on ÷ SVD registers. Commit as a tracked report next to the
register-modeling ratchet, so the shallow-coverage critique becomes a number we
watch climb.

### P4 — Free-running (non-halt) validation
Generalize the ESP32-S3 JTAG-Unity self-reporting pattern: firmware computes and
reports results over UART/RTT while running, no debugger halt. Closes the
halt-mode epistemic gap.

### P5 — Firmware-level differential — *v1 landed*
`firmware-f103-conformance` is one bare-metal firmware that drives every
peripheral through a realistic sequence and writes an observable-state **digest**
to a fixed RAM block; `crates/hw-oracle/tests/f103_conformance.rs` runs the same
ELF on the full-chip `Machine` and on silicon (openocd flash + run) and diffs the
digests, reporting a conformance % + a per-field gap report, baseline-gated.

First run measured **10/11 (91%)** and already paid off — it found gaps the
register-poke oracles structurally can't, because real firmware drives registers
back-to-back and hits hardware access latencies the sim doesn't model:
- **CRC reset latency** — feeding DR immediately after CR.RESET lost the first
  word on silicon; a settle() between them fixed it.
- **TIM2 status-latch latency** — reading SR one instruction after EGR.UG read 0
  on silicon; the compare-match flags need a cycle to latch.
- residual: exti_pr (firmware-context re-pending of EXTI line 0 from the
  GPIO-driven PA0 — the isolated EXTI oracle confirms the sim value is correct,
  so this is a hardware artifact, not a modeling gap).

Meta-finding: **the sim models no peripheral access/latch latency**, so tightly-
coded firmware that works in sim can fail on silicon — a modeling-quality
dimension invisible to hand-spaced micro-oracles. Next: widen the firmware to
SPI/I2C/USART/ADC/RTC/PWR, and grow into the v2 checkpoint register-file diff.

### P6 — Breadth
Other cores (extend the existing Xtensa/RISC-V/nRF oracle banks under HIL),
multiple silicon specimens/revs, and fuzzed register sequences to kill the
selection bias.
