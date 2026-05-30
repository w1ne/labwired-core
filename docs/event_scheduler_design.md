# Event-driven peripheral scheduler (issue #192, Phase 2B)

> **Provenance.** The original `/tmp/event_scheduler_design.md` (signed off, "S12
> decisions") was lost in a host reboot on 2026-05-30 — `/tmp` does not survive
> reboots. This file reconstructs the design from the committed Phase 2B.1
> skeleton (`crates/core/src/sched/`, the `Peripheral` trait extensions in
> `lib.rs`, and `tests/event_scheduler.rs`). Everything in §1–§9 is pinned by
> code or tests. §10 (the 2B.2 TIMG migration mechanism) is **OPEN** — it was in
> the lost doc and is not yet pinned by any committed code; it needs a decision
> before TIMG opts in. Keep this doc in the repo, not `/tmp`.

## 1. Goal

Cut per-cycle orchestration cost out of the simulator's hot loop. Profiling the
ereader bench showed the CPU interpreter was only ~4.5% of wall time; the rest
was orchestration — chiefly `Machine::step` walking ~40 peripherals every CPU
cycle and ticking each. The perf micro-opts (#142) shaved the cheap parts
(cached RTC_CNTL index, dense DPORT storage, word-granular register access).
The structural win is to stop ticking peripherals every cycle at all: let each
peripheral schedule a wakeup at the *next cycle that matters* and otherwise be
idle. This directly serves faster browser/WASM simulation.

## 2. Quantum and ordering

- `SimCycle = u64`, CPU-CCOUNT-equivalent. Floor-truncate at clock-domain
  conversion. Peripherals model sub-cycle phase internally and schedule at the
  CPU-cycle boundary that matters.
- Event ordering is strictly `(deadline asc, event_id asc)`. `peripheral_idx`
  and `event_token` never participate — reordering peripherals on the bus can
  never change event order. (`ScheduledEvent::cmp`, test
  `same_deadline_deterministic_by_event_id`.)

## 3. Data structure

`EventScheduler` is an O(log P) min-heap (`BinaryHeap<Reverse<ScheduledEvent>>`)
of upcoming wakeups, plus a monotonic `now`, a monotonic `next_event_id`, and
`SchedulerStats`.

- `schedule(deadline, peripheral_idx, event_token, generation) -> event_id`
  pushes one wakeup. `debug_assert!(deadline >= now)`; release builds clamp a
  past deadline to `now` and bump `stats.past_schedule_clamps`.
- `advance_to(target)` moves `now` forward only (monotonic; rewind is ignored —
  test `advance_to_is_monotonic`).
- `drain_due(generations) -> Vec<ScheduledEvent>` pops every event with
  `deadline <= now` and live generation, in order; stale entries are popped and
  discarded.
- `next_event_deadline(generations) -> Option<SimCycle>` scans for the earliest
  *live* deadline without mutating the heap. O(P); called at most once per step
  (the hot path is `drain_due`).

## 4. The `event_token` contract

The scheduler has zero knowledge of token semantics. `event_token: u32` is
opaque; the owning peripheral interprets it via its own internal token enum.
This keeps the scheduler peripheral-agnostic.

## 5. Reentrancy

An `on_event` handler may call `schedule()` mid-drain. The new event gets a
higher `event_id`; if its deadline equals `now` it lands at the end of the
current logical order and is returned by the *next* `drain_due`, not the
in-flight batch (test `reentrant_schedule_during_drain_semantics`). This is the
documented model for "a timer reloads and re-arms itself."

## 6. Cancellation — lazy via generation

No heap removal. Each `PeripheralEntry` carries a `generation: u32`. A
peripheral reset bumps its generation; `drain_due` / `next_event_deadline` drop
any event whose snapshot generation no longer matches
(`is_stale`). Out-of-range `peripheral_idx` (peripheral removed) is also stale.
`SystemBus::peripheral_generations()` produces the snapshot threaded into the
scheduler each step. (Tests `lazy_cancel_via_generation_bump`,
`lazy_cancel_partial`, `stale_idx_out_of_range_is_dropped`.)

## 7. Clock changes are NOT scheduled

Clock-rate change is a *synchronous* call, not a heap event — avoids a circular
dependency and keeps the heap focused on per-peripheral wakeups.
`ClockGraph::set_rate(domain, hz, notify)` updates the rate and synchronously
invokes `notify(idx, domain, hz)` for each subscribed observer, which the
Machine routes to `Peripheral::on_clock_change`. A peripheral typically cancels
its in-flight events (generation bump) and reschedules at the new cadence.
`subscribe` is idempotent/deduplicated. `rate()` returns 0 for an unconfigured
domain — peripherals seed their domain at construction. (Tests
`clock_graph_set_rate_notifies_observers`, `clock_graph_subscribe_dedupes`.)

§12a (original doc): wiring `ClockGraph` to real ESP32 register writes
(`DPORT_CPU_PER_CONF`, `RTC_CNTL_CLK_CONF`) lands with the DPORT / RTC_CNTL
migration PRs, not 2B.1.

## 8. Side-effects — reuse the existing fan-out

`on_event` returns an `EventResult` — a subset of `PeripheralTickResult`
(`raise_irq`, `explicit_irqs`, `system_exception`, `mmio_writes`,
`fired_events`, `dma_requests`). `Machine::apply_event_result` fans these out
through the *same* machinery as the post-`tick()` path: NVIC pend (via
`SystemBus::pend_irq_for_event`), CPU exception pend, MMIO writes, PPI
`fired_events` globalisation through `route_ppi_events`, and DMA execute. No new
side-channels.

## 9. Feature flag and migration staging

- `event-scheduler` cargo feature (default **OFF**). The `sched` types and the
  `Peripheral` trait extensions (`on_event`, `on_clock_change`,
  `uses_scheduler`) compile in unconditionally; the flag only toggles the
  runtime drain block in `Machine::step`.
- With the flag OFF, behaviour is byte-for-byte pre-2B `main`.
- With the flag ON but no peripheral overriding `uses_scheduler()` (→ `false`),
  the drain is a no-op and the legacy per-cycle `tick()` walk still drives every
  peripheral. (Test `machine_step_parity_with_no_scheduler_users`.)
- Each migration PR (2B.2 … 2B.N) flips one peripheral to `uses_scheduler() ==
  true`, at which point `Machine::step` skips that peripheral's legacy tick and
  drives it purely by events. After the last peripheral migrates, the flag goes
  unconditional and the legacy walk is deleted.

`Machine::step` drain (gated): `advance_to(total_cycles)` →
`drain_due(peripheral_generations())` → for each event, swap the peripheral out
(placeholder `StubPeripheral`, same dance as `tick_with_bus`) so `&mut self.bus`
can be passed into `on_event`, then `apply_event_result`.

## 10. Phase 2B.2: TIMG migration mechanism — RESOLVED & IMPLEMENTED

> Decision (2026-05-30): **Machine-side anchor sync** (candidate A below). TIMG
> is the first peripheral to opt in (`uses_scheduler() == true`). Implemented;
> validated flag-OFF byte-identical and flag-ON behaviour-neutral (the ESP32
> epaper e2e produces the identical output, pre-existing one-pixel bug and all).

TIMG today (`peripherals/esp32/timg.rs`) is a free-running counter: `tick()`
increments `counter_t0`/`counter_t1` by 1 every CPU cycle when enabled, and
**never fires an alarm IRQ** (alarm logic is stubbed — see the "future actually
fire the alarm IRQ" note in the source). The only firmware-observable effect is
the counter value latched into `T0_LO/HI` on a `T0_UPDATE` write.

The problem: once TIMG opts into the scheduler, `Machine::step` stops ticking
it, so `counter_t0` stops advancing. Firmware that polls the timer for delays
would read a frozen counter. The counter must instead be derived lazily from
elapsed cycles — but the `Peripheral::read` / `write` signatures don't carry
`now`, so TIMG can't compute `anchor_val + (now - anchor_cycle) * step` on its
own at latch time.

Candidate mechanisms (decision needed):

- **A — read-time sync.** Thread `now` into the bus access path for
  scheduler-driven peripherals so TIMG computes the live counter on
  `T0_UPDATE`. Fully removes per-cycle TIMG work; best perf. Cost: a read/write
  path change touching the `Peripheral`/`Bus` interface.
- **B — Machine-side anchor sync.** Machine syncs a scheduler peripheral to
  `now` (a new `sync_to(now)` hook) just before dispatching any MMIO access to
  it. Localises the change to dispatch; no trait-signature churn on `read`.
- **C — periodic sync event.** TIMG schedules a coarse periodic wakeup to
  advance its anchor. Simplest, but re-introduces periodic work and bounds the
  perf win.

Alarms: when alarm support is added, TIMG schedules an `on_event` at the alarm
deadline (`counter == alarm` ⇒ `deadline = anchor_cycle + (alarm - anchor_val)`
in CPU cycles), raises its IRQ in the returned `EventResult`, and (for
auto-reload) re-arms via the reentrant `schedule(now, …)` path (§5). TIMG
subscribes to the APB clock domain and reschedules on `on_clock_change` (§7).

### Implemented (2B.2)

- `Peripheral::sync_to(&mut self, tick_now: u64)` — default no-op trait hook.
  `tick_now` is the peripheral-tick index = `total_cycles / peripheral_tick_interval`,
  the exact quantum the legacy walk advanced one-per-`tick()`, so the lazy model
  is behaviour-identical for any tick interval.
- `Machine::step` mirrors `total_cycles` into `SystemBus::current_cycle` once per
  step — O(1), does not reintroduce the per-peripheral walk.
- The bus calls `sync_to` on a scheduler-driven peripheral immediately *before*
  an MMIO **write** observes it (`sync_scheduler_peripheral`). Reads are `&self`
  and can't sync; this is fine because LO/HI reads require a prior `T0_UPDATE`
  *write* strobe (real-silicon requirement, honoured by esp-idf), and the strobe
  syncs first then latches the up-to-date counter.
- `tick_peripherals_phase1` skips `uses_scheduler()` peripherals — the actual
  orchestration saving.
- TIMG keeps its legacy per-cycle `tick()` increment for flag-OFF builds and
  gains the anchor-based `sync_to` for flag-ON. `anchor_tick` is monotonic; a
  span spent disabled is excluded.

All of the above is gated behind `event-scheduler`, so flag-OFF stays
byte-identical. Perf payoff is marginal at 2B.2 (only TIMG is lazy; the walk
still runs for the other ~39 peripherals) and scales as each subsequent
peripheral migrates and the walk is eventually deleted (§9).
