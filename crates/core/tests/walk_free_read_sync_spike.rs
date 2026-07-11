// SPIKE (walk-free STM32 plan, Part 1): prove the recommended read-side
// freshness mechanism — interior-mutability sync-on-read against a shared "now"
// the bus publishes — COMPILES against the real `Peripheral` trait and is
// byte-exact vs the legacy walk at batch boundaries, stale by < one interval
// in between.
//
// This is a design spike, NOT a shipped model. It implements the pattern a
// scheduler-migrated STM32 timer / SysTick / DWT would use:
//   * counter state lives in `Cell` (the `esp32c3/rtc_timer.rs` precedent),
//   * a shared `Arc<AtomicU64>` clock (Send — the trait bound; `Rc<Cell>` is
//     NOT Send) carries `SystemBus::current_cycle` down to a `&self` read,
//   * `read(&self)` advances the counter through the Cell to the published
//     clock and returns a FRESH value — no `&mut`, so the existing `&self`
//     bus read path is untouched.
//
// The assertions pin the determinism contract in Part 1(c): reads AT a batch
// boundary equal the interval-1 walk exactly; reads mid-batch trail by strictly
// less than the tick interval (the same "< one tick" bound the WRITE path
// already documents).

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use labwired_core::{Peripheral, PeripheralTickResult, SimResult};

/// A free-running up-counter (models DWT CYCCNT / a running TIM->CNT): +1 per
/// CPU cycle while enabled. `cnt`/`anchor` are `Cell` so the sync runs through
/// a shared `&self` reference.
#[derive(Debug)]
struct SchedCounter {
    cnt: Cell<u64>,
    anchor: Cell<u64>,
    enabled: bool,
    /// The bus-published "now". In the real bus this is fed from
    /// `SystemBus::current_cycle` (batch-start cycle) once per batch.
    clock: Arc<AtomicU64>,
}

impl SchedCounter {
    fn new(clock: Arc<AtomicU64>) -> Self {
        Self {
            cnt: Cell::new(0),
            anchor: Cell::new(0),
            enabled: true,
            clock,
        }
    }

    /// Lazily advance the counter to the published clock. Callable from a
    /// `&self` read because all mutated state is in `Cell`. This is the whole
    /// mechanism: a read that is fresh-to-batch-start without a `&mut` bus.
    fn sync_from_clock(&self) {
        let now = self.clock.load(Ordering::Relaxed);
        let anchor = self.anchor.get();
        if now <= anchor {
            return;
        }
        if self.enabled {
            self.cnt.set(self.cnt.get().wrapping_add(now - anchor));
        }
        self.anchor.set(now);
    }
}

impl Peripheral for SchedCounter {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // Read-side freshness: sync THEN return. The value is the counter as of
        // the last published clock (batch-start).
        self.sync_from_clock();
        Ok((self.cnt.get() & 0xFF) as u8)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    /// The scheduler-migrated read returns the full counter (bypasses the
    /// byte accessor for the test's exactness checks).
    fn read_u32(&self, _offset: u64) -> SimResult<u32> {
        self.sync_from_clock();
        Ok(self.cnt.get() as u32)
    }

    /// Legacy walk body — the reference behaviour. Advancing by `cycles` per
    /// tick is exactly what `sync_from_clock` reproduces in O(1).
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.enabled {
            let v = self.cnt.get().wrapping_add(cycles);
            self.cnt.set(v);
        }
        PeripheralTickResult::default()
    }

    // The two assertions the walk-free derivation reads:
    fn uses_scheduler(&self) -> bool {
        true
    }
    fn needs_legacy_walk(&self) -> bool {
        false
    }
}

/// Interval-1 legacy walk: the golden reference. Tick every cycle; sample CNT
/// at the query cycles.
fn walk_reference(query_cycles: &[u64], up_to: u64) -> Vec<u64> {
    let clock = Arc::new(AtomicU64::new(0));
    let mut dev = SchedCounter::new(clock);
    let mut out = Vec::new();
    let mut q = 0usize;
    for c in 1..=up_to {
        dev.tick_elapsed(1);
        while q < query_cycles.len() && query_cycles[q] == c {
            out.push(dev.read_u32(0).unwrap() as u64);
            q += 1;
        }
    }
    out
}

/// Scheduler path at `interval`: the bus publishes the batch-START cycle once
/// per batch; a read lands mid-batch and sees that batch-start clock.
fn scheduler_reads(query_cycles: &[u64], interval: u64) -> Vec<u64> {
    let clock = Arc::new(AtomicU64::new(0));
    let dev = SchedCounter::new(clock.clone());
    query_cycles
        .iter()
        .map(|&c| {
            // Batch-start cycle for the batch containing cycle `c`.
            let batch_start = (c / interval) * interval;
            clock.store(batch_start, Ordering::Relaxed);
            dev.read_u32(0).unwrap() as u64
        })
        .collect()
}

#[test]
fn read_sync_is_exact_at_batch_boundaries() {
    // Query cycles that are all multiples of 64 (batch boundaries at interval 64).
    let queries = [64u64, 128, 192, 256, 640];
    let golden = walk_reference(&queries, 640);
    let sched = scheduler_reads(&queries, 64);
    // Free-running counter from 0 ⇒ CNT == cycle at every boundary.
    assert_eq!(golden, vec![64, 128, 192, 256, 640]);
    assert_eq!(
        sched, golden,
        "at batch boundaries the read-synced counter is byte-exact vs the interval-1 walk"
    );
}

#[test]
fn mid_batch_read_staleness_is_bounded_by_interval() {
    let interval = 64u64;
    let queries = [50u64, 65, 100, 130, 199, 200];
    let golden = walk_reference(&queries, 200);
    let sched = scheduler_reads(&queries, interval);
    for (i, (&c, (&g, &s))) in queries.iter().zip(golden.iter().zip(sched.iter())).enumerate() {
        // The scheduler read never runs ahead of the walk, and trails it by
        // strictly less than one interval — the documented "< one tick" bound.
        assert!(s <= g, "query[{i}] c={c}: scheduler read {s} ran ahead of walk {g}");
        assert!(
            g - s < interval,
            "query[{i}] c={c}: staleness {} exceeds interval {interval}",
            g - s
        );
        // And the stale value equals the walk value AT the batch-start cycle —
        // i.e. exact quantization to the tick grid, no drift.
        let batch_start = (c / interval) * interval;
        assert_eq!(s, batch_start, "query[{i}] c={c}: read must equal batch-start counter");
    }
}
