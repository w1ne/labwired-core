# Walk-free STM32: making the per-cycle peripheral walk structurally unnecessary

> **Goal.** Any user-compiled firmware on any board runs fast, not just curated
> demos carrying a hand `walk_deleted: true`. Concretely: on a bare STM32 bus,
> `SystemBus::derive_walk_deletable()` (`crates/core/src/bus/tick.rs`) flips
> `true`, `max_safe_tick_interval()` returns `RECOMMENDED_TICK_INTERVAL` (64)
> universally, and interval batching engages. Today ~49 of the invaders bus's 58
> peripherals force it `false` because they return the conservative default
> `needs_legacy_walk() == true`.
>
> This plan is code-pinned. Every count and file reference below was read from
> `origin/main` at `a147fe0b`. It drives the implementation agents; keep it in
> the repo.

## 0. Where the gate stands today

`derive_walk_deletable` (bus/tick.rs) is `true` iff **every** peripheral on the
bus satisfies `uses_scheduler() || !needs_legacy_walk()`. Both predicates
default toward the walk:

- `needs_legacy_walk()` defaults to `true` (lib.rs ~574). **No STM32 model
  overrides it** — grep across `crates/core/src/peripherals/` finds zero
  `fn needs_legacy_walk` overrides on any STM32 peripheral.
- `uses_scheduler()` is overridden `true` only by `uart.rs` (line 1077) and
  `spi.rs` (line 1501).

So on the L476 invaders bus (58 peripheral instances), the 9 `uart`×6 + `spi`×3
pass; the other **49 force the walk on**. That is the "#513 = 49" number, and it
is why `walk_deleted: true` must be hand-written per lab. The universal fix is
to make the remaining 49 assert walk-independence *honestly* — trivially for the
inert ones, and by migrating the rest onto the event scheduler.

The one structural blocker to migrating the "real work" timers is a read-path
constraint, resolved in Part 1.

---

## Part 1 — The architectural decision: read-side freshness

### The constraint

Scheduler-migrated peripherals advance lazily: the bus calls
`sync_scheduler_peripheral(idx)` → `dev.sync_to(current_cycle)` **before an MMIO
write** (bus/tick.rs ~137, called from the write path in bus/accessors.rs
`write_u8`/`write_u16`/`write_u32`). But firmware polls free-running counters
**by reading them** — `TIM->CNT`, SysTick `SYST_CVR` (VAL), `DWT->CYCCNT`, RTC
sub-second — and the bus read path is `&self`:

```
fn read_u8(&self, addr) -> ...   // Bus trait, accessors.rs:52
    ... p.dev.read(addr - p.base) // Peripheral::read(&self, offset)  lib.rs:399
```

A `&self` read cannot call `sync_to(&mut self, …)`. This is exactly why
`esp32c3/rtc_timer.rs` (lines 118–124) keeps `uses_scheduler() == false` with the
comment *"The bus read API is intentionally `&self`; until it can sync
scheduler-driven peripherals before reads, this read-driven RTC must stay on the
legacy tick path or firmware delay loops observe stale time and spin forever."*
It is also why ESP32 TIMG (`esp32/timg.rs`) gets away with **write-side-only**
sync: TIMG hardware latches the counter into a readable register on an
`UPDATE`-register *write*, so the write-path `sync_to` refreshes it. **STM32
timers have no such latch** — firmware reads `CNT` directly with no preceding
write.

### The three options, evaluated

#### (a) `&mut` read choke point — REJECTED (blast radius)

Change `Peripheral::read*` to `&mut self` (or add `read_synced(&mut self)` the
bus prefers). Mechanical blast radius, measured on the tree:

| Surface | Count | Why it hurts |
|---|---|---|
| `impl Peripheral for …` blocks | **134** | every one's `read`/`read_u16`/`read_u32` signature changes |
| `Bus::read_u8` call sites in `core/src` | **200** | `read_u8` is `&self` today; making the peripheral read `&mut` forces `Bus::read*` to `&mut` too |
| `Bus::read_u16`/`read_u32` call sites in `core/src` | **1318** | same cascade |
| read calls inside `crates/core/src/cpu/` | **105** | the CPU holds a **shared** `&self` bus borrow during instruction fetch |
| read calls outside `core/src` (hw-oracle, wasm, dap…) | **219** | every differential/host caller |

The fatal problem is not the count — it is **aliasing**. `fetch_slice(&self)`
(accessors.rs:509) hands the CPU a `&[u8]` borrowed from a peripheral's backing
buffer; observers iterate `&self.observers` during reads; bit-band and
atomic-alias reads *recurse* through `read_u32(&self)`. Making reads `&mut`
collapses the "many concurrent shared reads during a batch" model the
interpreter is built on. This is a multi-week refactor touching every crate, to
serve a handful of counter peripherals. **Rejected.**

#### (b) Interior mutability + a bus-published clock — the MECHANISM

The `Peripheral` trait is `Send` (lib.rs:398), **not `Sync`**, and the engine is
single-threaded per machine, so interior mutability is sound. It is already in
use: **24 files** under `peripherals/` hold state in `Cell`/`RefCell`, and
`esp32c3/rtc_timer.rs` already stores its `counter`/`anchor_tick` in `Cell` with
a working `sync_to`. The only missing piece is that `read(&self, offset)`
receives *only the offset* — it cannot today reach `bus.current_cycle`.

Give each scheduler-migrated counter peripheral a shared clock handle,
`Arc<AtomicU64>` (Send — `Rc<Cell<u64>>` is **not** Send and would break the
trait bound), into which the bus publishes `current_cycle` once per batch (the
same value the write path already syncs to). `read(&self)` then advances its
`Cell` counter to that clock and returns a fresh value — **zero changes to the
`&self` bus read path or the 134 impls.** This is the mechanism; it delivers the
freshness bound analysed in (c).

#### (c) Batch-boundary freshness — the CONTRACT (RECOMMENDED)

`bus.current_cycle` is the **batch-start** cycle during a CPU batch (lib.rs:2029:
*"intra-batch staleness is < one tick"*). So a read synced to it trails the true
cycle by **strictly less than one `peripheral_tick_interval`** — *exactly the
"< one tick" bound the write path already ships and documents* (lib.rs ~2001 and
the `sync_to` doc-comment, lib.rs:669). The quantization has direct precedent:
HC-SR04 echo edges are already ceil'd to the tick grid and differential-gated,
and the run loop already clamps a batch to the next scheduled edge
(`next_hcsr04_deadline_cycle`, lib.rs:2086).

**Recommendation: adopt (c) as the determinism contract, implemented via (b)'s
interior-mutability read-sync. Reject (a).**

### The determinism argument (the differential gate)

The oracle's observable outputs are the firmware's **externally visible
effects**: GPIO/pad transitions, bytes on the SPI/I²C/UART wire, and IRQ-driven
control flow. Split by how the scheduler reproduces each:

1. **Event-derived observables** (a compare-match IRQ, an overflow, a DMA
   transfer-complete, an ADC EOC, a timeout-driven GPIO toggle). Migrated
   peripherals emit these as scheduler events pinned to an **absolute cycle
   deadline** (`current_cycle + 1 + delay`, bus/tick.rs:167), and the run loop
   clamps the batch to the nearest armed deadline (generalizing today's HC-SR04
   clamp). These land at their **exact** cycle — byte-identical to a walk-on
   interval-1 run. This is the real oracle claim, and it holds at interval 64.

2. **Raw counter reads** (`TIM->CNT`, `SYST_CVR`, `CYCCNT`, RTC sub-second) polled
   mid-batch. These are quantized to the batch-start grid, ≤ (interval − 1)
   cycles behind. This is **not** oracle-observable *unless firmware exports the
   raw value* (e.g. sends `CNT` over UART). For that narrow class, interval-1
   byte-equivalence is impossible under *any* batching; the honest promise is the
   documented ≤ one-interval quantization, with full same-run determinism.

**Therefore the differential gate is: walk-on @ interval-1 (golden reference)
vs scheduler @ interval-64, asserting byte-identity of the event-observable
trace** — GPIO transitions, bus bytes, and IRQ cycle stamps. The reference runs
at interval 1 (the most accurate); the scheduler matches it for every
event-derived observable because deadlines are absolute and batch-clamped. The
raw-counter-read class is explicitly carved out (bounded, documented) — the same
carve-out the write side already lives with.

> **Optional exactness upgrade (deferred, not required for walk-freedom).**
> Publishing a *per-retired-instruction* clock would erase even the raw-read
> quantization. The CPU already does exactly this for logic-capture push mode —
> `tap.bump_clock()` once per retired instruction (cortex_m.rs:682/716/746). A
> future "always-on sim clock" bumped per instruction would let raw counter reads
> match interval-1 byte-for-byte, at the cost of one integer increment per
> instruction. Keep it in reserve; the batch-start contract is sufficient to
> delete the walk.

### Spike result (committed, marked `SPIKE`)

`crates/core/tests/walk_free_read_sync_spike.rs` implements the (b)+(c) pattern
against the **real** `Peripheral` trait: a `Cell`-based free-running counter with
an `Arc<AtomicU64>` shared clock, `read(&self)` syncing to the published clock.
Two tests pass (`cargo test -p labwired-core --test walk_free_read_sync_spike
--features event-scheduler`, 26.7s build, both green):

- `read_sync_is_exact_at_batch_boundaries` — at every batch boundary the
  read-synced counter equals the interval-1 walk **exactly**.
- `mid_batch_read_staleness_is_bounded_by_interval` — a mid-batch read never runs
  ahead of the walk and trails it by **strictly < interval**, landing exactly on
  the batch-start counter value (clean grid quantization, no drift).

This proves the recommendation compiles (the `Send` bound holds with
`Arc<AtomicU64>`), needs no bus-signature change, and honours the contract. The
spike is a design artifact, not a shipped model — delete it or fold it into the
SysTick migration test when Batch 1 lands.

---

## Part 2 — Walker inventory and migration order

### Inventory (L476 invaders bus, 58 instances)

Migrated already: `uart`×6, `spi`×3 (`uses_scheduler()==true`). The remaining
**49 walkers** split into two work classes.

**Class A — inert / stub tick: assert `needs_legacy_walk()==false` (one line, S).**
These models have a *default no-op* `tick()` (no `fn tick`/`fn tick_elapsed` body
in the file) or a pure register bank; they force the walk only because nobody
asserted otherwise. 28 instances:

| Model (`type`) | Inst. | `tick()` today | Note |
|---|---|---|---|
| `gpio` | 6 | none (no-op) | pure register + pad model; edges handled by the GPIO-diff pass, not `tick` |
| `lptim` | 2 | none | register bank |
| `rcc`, `pwr`, `flash`, `rng`, `crc`, `dac`, `dbgmcu`, `fmc`, `quadspi`, `usb_otg`, `sdmmc`, `comp`, `tsc`, `nvic`, `stub` | 1 each (15) | none | register banks / controllers with no time-driven state |
| `sai` | 2 | none | register bank |
| `iwdg`, `wwdg` | 1 each (2) | none | **stubs today** — they do not actually count down. `false` is honest for the current model; a real countdown (future fidelity work) would move them to Class B, not back to the walk |
| `rtc` | 1 | none (`rtc.rs` → `Rtc::new`) | On L476 `type: "rtc"` binds `rtc.rs` (register bank, no-op tick). The counter-timer RTC (`rtc_v3.rs`, per-second alarm tick) appears on H5-class chips — there it is Class B |

**Class B — real `tick()` work: migrate to the scheduler (sync_to + on_event +
uses_scheduler).** 21 instances, 8 model types:

| Model | Inst. | `tick()` does (when armed) | Read-polled? | IRQ/event deadlines | Migration class | Effort |
|---|---|---|---|---|---|---|
| `systick` | 1 | down-count `cvr`; at 0 → `system_exception 15`, reload `rvr` (systick.rs) | **yes** (`SYST_CVR`) | reload/underflow every `rvr+1` cycles | counter-timer w/ deadline | **M** |
| `timer` (TIM1–17) | 11 | prescaled up-count; UIF on `arr` wrap + CCxIF compare match; holds IRQ level while `sr&dier` latched (timer.rs) | **yes** (`CNT`) | next update + up to 4 compare deadlines; **level re-assert** | counter-timer w/ compare deadlines | **L** |
| `dma` (DMA1/2) | 2 | per active channel w/ `cndtr>0`: emit DMA req(s) + TC IRQ (dma.rs) | flags (`ISR`) | transfer pacing + TC | DMA transfer engine | **L** |
| `i2c` (I2C1–3) | 3 | byte-transfer state machine (4 `tick` bodies, i2c.rs) | flags (`SR1/SR2/ISR`) | per-byte pacing + event/error IRQ | wire engine / byte-stream pacing | **L** |
| `adc` | 1 | one-shot conversion latency countdown; EOC + optional continuous restart; EOCIE IRQ (adc.rs) | flags (`SR.EOC`) | conversion-complete one-shot | one-shot latency → event | **M** |
| `exti` | 1 | route `pr & imr` (+ bank2 wakeup lines) → IRQ (exti.rs) | no | **no time** — re-eval on write/edge | edge/level detector (needs no time) | **M** |
| `scb` | 1 | drain software-pended NMI/SysTick/PendSV from ICSR writes → `system_exception` (scb.rs) | no | delay-0 on the ICSR write | write-latch → immediate event | **S/M** |
| `bxcan` | 1 | drain RX interconnect channel into receiver (bxcan.rs) | flags | on external frame arrival | wire engine (external mailbox) | **M** |

`dwt.rs` (CYCCNT, free-running, read-polled, no IRQ) is a Cortex-M core
peripheral not listed in the L476 descriptor; when a bus carries it, it is the
**purest lazy-read** case — the (b)+(c) mechanism handles it with a bare
`Arc<AtomicU64>` counter, no events.

Class exemplars to copy:
- counter-timer w/ deadlines → `esp32/timg.rs` (`sync_to`) + `esp32s3/systimer.rs`
  (`on_event` reschedule via `next_alarm_delay_cycles`, plus **level-IRQ
  re-assert** — the pattern TIM/EXTI need so a line held by a latched flag
  re-pends after firmware clears NVIC pending mid-flag).
- byte-stream pacing → `uart.rs` events.
- wire engine → `spi.rs`.
- lazy read-only counter → `esp32c3/rtc_timer.rs` (Cell + `sync_to`), unblocked to
  flip `uses_scheduler()==true` the moment Part 1's read-sync lands.

### Migration order (by dynamic importance for real firmware)

0. **Class A sweep** — mechanical, independent, low-risk. Add
   `needs_legacy_walk()==false` to all 28 inert instances. Does **not** flip
   `derive_walk_deletable` alone (needs all 49) but removes 28/49 and is the
   cheapest bulk progress.
1. **SysTick** — every HAL `delay`/tick loop and the FreeRTOS tick. #1 by far.
2. **SCB** — PendSV/SVCall drive FreeRTOS context switches; tiny (delay-0
   write-latch). Land with SysTick so the RTOS path is coherent.
3. **TIM2–5** (general-purpose), then TIM1/8 (advanced), TIM6/7 (basic),
   TIM15/16/17. Largest count; the counter-timer template proven here is reused
   across all 11.
4. **DMA1/2** — memcpy + peripheral streaming.
5. **ADC** — one-shot conversion latency.
6. **EXTI** — button/edge IRQs (re-eval on write + GPIO-edge injection).
7. **I2C1–3** — wire engine.
8. **bxCAN** — external mailbox drain.
9. **RTC** (`rtc_v3` on H5-class buses) — second-granularity alarms.

### PR batches (each with its differential gate)

Every gate arms the peripheral and runs **walk-on @ interval-1 vs scheduler @
interval-64**, asserting byte-identity of the event-observable trace (Part 1).

| Batch | Scope | Differential gate (test firmware) | Unlocks |
|---|---|---|---|
| **B0** | Class A sweep: `needs_legacy_walk()==false` ×28 | per-model "tick is a genuine no-op" unit assert (tick on an armed instance returns `default()` and mutates nothing) | 28/49; no bus flips yet |
| **B1** | SysTick + SCB → scheduler; land the shared-clock read-sync from the spike | SysTick-driven blinky + a FreeRTOS-tick / PendSV context-switch fixture; assert GPIO-transition + IRQ-cycle byte-identity | RTOS + delay loops |
| **B2** | TIM2–5 (counter-timer template) | timer-IRQ blinky (UIF) + a PWM compare-match (CCxIF) fixture; GPIO edges byte-exact | general-purpose timers |
| **B3** | TIM1/8/6/7/15/16/17 (reuse B2 template) | advanced-timer + basic-timer IRQ fixtures | all 11 timers |
| **B4** | DMA1/2 | mem-to-mem memcpy + peripheral→mem stream; assert destination bytes + TC IRQ cycle | DMA |
| **B5** | ADC | ADC poll-loop (EOC) + EOCIE IRQ fixture | ADC |
| **B6** | EXTI | button-press → EXTI IRQ (SWIER + GPIO-edge paths); IRQ cycle byte-exact | GPIO interrupts |
| **B7** | I2C1–3 | I²C temp-sensor read (`i2c_temp_sensor.rs`); wire bytes + event IRQ byte-exact | I²C |
| **B8** | bxCAN | CAN loopback frame RX; delivered frame + FIFO IRQ | CAN |

When **B0–B8** all land, the L476 invaders bus (and any bus built from the same
model set) derives `derive_walk_deletable()==true` with **no** hand
`walk_deleted` flag, `max_safe_tick_interval()==64`, and batching engages for
user-compiled firmware.

**Per-board partial unlock.** `derive_walk_deletable` is per-bus, so a board
flips as soon as *its* peripherals are all migrated — it does not wait for every
model in the tree. Boards carrying a thin descriptor cross the line earlier:
check each bus's instantiated set (nRF52 / Kinetis KW41Z buses carry a different,
generally smaller walker list — e.g. no bxCAN/QUADSPI/SAI — so B0 + the SysTick
and timer batches may already flip them). Track the flip per board in the
differential CI matrix rather than assuming the L476 order applies everywhere.

---

## Part 3 — End state and the `walk_deleted` deprecation

Once B0–B8 land for a model set, `derive_walk_deletable()` returns the honest
answer with no manual input, so the hand `walk_deleted: true` in a lab manifest
becomes **redundant** for those buses. It is not immediately removable, for one
reason spelled out in the derivation doc-comment (bus/tick.rs ~72): the hand flag
can assert *firmware-specific* byte-identity ("this firmware never touches the 11
timers the chip instantiates") that no config-time predicate can prove. But after
migration that assertion is no longer *needed* — a fully-migrated bus is
walk-free for **all** firmware, not just the one that avoids the timers.

**Deprecation path:**

1. Land B0–B8; the differential CI matrix shows each target bus deriving
   walk-deletion with the manifest flag removed.
2. For every lab whose bus now auto-derives, delete `walk_deleted: true` from its
   `system.yaml` (invaders first — its comment already claims byte-identity).
   `from_config.rs:730` already prefers `manifest.walk_deleted` when present and
   falls back to `derive_walk_deletable()` when absent, so removal is safe and
   the derivation takes over.
3. When no in-tree manifest sets `walk_deleted`, mark the field deprecated in
   `labwired-config` (keep parsing it as an escape hatch for out-of-tree /
   fidelity-incomplete models, but warn), and drop the per-lab `walk-identity`
   checks in favour of the per-model differential gates from Part 2.

The end state: **the walk is dead code for migrated buses** — `legacy_tick_indices`
is empty, `per_cycle_tick_is_trivial()` is true, and the per-cycle hot loop is
just the NVIC scan (bus/tick.rs:704). No lab author writes a flag; correctness is
a property each model proves once, in its own differential test.
