// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Flash-PC-keyed block cache with hot-counter promotion.
//!
//! ## Keying
//!
//! Blocks are keyed by their **flash entry PC** alone. The JIT only
//! compiles flash-resident code, and flash is immutable after config, so
//! the `(pc) -> compiled block` mapping is stable for the life of a run.
//!
//! ## Promotion
//!
//! A PC starts "cold". Each time the dispatcher lands on a cold PC it
//! bumps a hot counter and interprets one instruction. When the counter
//! crosses [`BlockCache::hot_threshold`] the PC is promoted: the frontend
//! is asked to translate it and, on success, the compiled artifact is
//! installed. Cold blocks cost one `HashMap` bump; only genuinely hot code
//! pays the translation cost.
//!
//! ## Invalidation
//!
//! On **any** flash write the entire cache is dropped
//! ([`BlockCache::invalidate_all`]). This is deliberately blunt: flash
//! writes are rare (firmware self-update / OTA), correctness is absolute,
//! and per-range invalidation would need a write-address→block index we do
//! not want to maintain. Compiling only flash-resident code is what makes
//! "invalidate everything, rarely" cheap and correct.

use std::collections::HashMap;

use super::Pc;

/// Default promotion threshold: land on a PC this many times before
/// compiling it. Tuned later against `invaders`; the value here is a
/// sane starting point that keeps one-shot init code on the interpreter.
pub const DEFAULT_HOT_THRESHOLD: u32 = 50;

/// State of one PC in the cache.
enum Slot<A> {
    /// Seen but not yet hot: just a hit counter.
    Cold { hits: u32 },
    /// Promoted and compiled: the executable artifact plus run stats.
    Hot { artifact: A, runs: u64 },
}

/// The block cache. `A` is the runtime-specific compiled artifact type
/// (e.g. a `wasmtime` instance, or a `js_sys::WebAssembly.Instance`
/// wrapper) — the cache is agnostic to it.
pub struct BlockCache<A> {
    slots: HashMap<Pc, Slot<A>>,
    hot_threshold: u32,
    /// Monotonic count of full invalidations (telemetry).
    generation: u64,
}

impl<A> Default for BlockCache<A> {
    fn default() -> Self {
        Self::new(DEFAULT_HOT_THRESHOLD)
    }
}

/// What [`BlockCache::observe`] tells the dispatcher to do with a PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    /// PC has a compiled artifact ready — run it.
    Ready,
    /// PC is cold (or newly seen); interpret one instruction. `true` means
    /// this hit crossed the threshold and the caller should now try to
    /// compile + [`BlockCache::install`] it.
    Interpret { promote: bool },
}

impl<A> BlockCache<A> {
    /// New cache with an explicit promotion threshold.
    pub fn new(hot_threshold: u32) -> Self {
        Self {
            slots: HashMap::new(),
            hot_threshold: hot_threshold.max(1),
            generation: 0,
        }
    }

    /// The promotion threshold in effect.
    pub fn hot_threshold(&self) -> u32 {
        self.hot_threshold
    }

    /// How many times the cache has been fully invalidated.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Number of currently-installed (hot, compiled) blocks.
    pub fn compiled_len(&self) -> usize {
        self.slots
            .values()
            .filter(|s| matches!(s, Slot::Hot { .. }))
            .count()
    }

    /// Whether `pc` currently has a compiled artifact installed.
    pub fn is_hot(&self, pc: Pc) -> bool {
        matches!(self.slots.get(&pc), Some(Slot::Hot { .. }))
    }

    /// Record that the dispatcher landed on `pc` and decide what to do.
    ///
    /// * If `pc` is compiled → [`Lookup::Ready`].
    /// * Otherwise bump the hot counter and return [`Lookup::Interpret`],
    ///   with `promote = true` exactly on the hit that crosses the
    ///   threshold (so the caller compiles it once).
    pub fn observe(&mut self, pc: Pc) -> Lookup {
        match self.slots.get_mut(&pc) {
            Some(Slot::Hot { .. }) => Lookup::Ready,
            Some(Slot::Cold { hits }) => {
                *hits += 1;
                Lookup::Interpret {
                    promote: *hits == self.hot_threshold,
                }
            }
            None => {
                self.slots.insert(pc, Slot::Cold { hits: 1 });
                // threshold of 1 => promote on first sight
                Lookup::Interpret {
                    promote: 1 == self.hot_threshold,
                }
            }
        }
    }

    /// Install a freshly compiled artifact for `pc`, promoting it to hot.
    /// Replaces any existing slot for `pc`.
    pub fn install(&mut self, pc: Pc, artifact: A) {
        self.slots.insert(pc, Slot::Hot { artifact, runs: 0 });
    }

    /// Borrow the compiled artifact for `pc` (if hot) **without** counting a
    /// run. Used to peek an installed block's metadata (e.g. its instruction
    /// count) before deciding whether it may run within the caller's budget.
    pub fn peek(&self, pc: Pc) -> Option<&A> {
        match self.slots.get(&pc) {
            Some(Slot::Hot { artifact, .. }) => Some(artifact),
            _ => None,
        }
    }

    /// Borrow the compiled artifact for `pc` (if hot) and count a run.
    pub fn run_artifact(&mut self, pc: Pc) -> Option<&mut A> {
        match self.slots.get_mut(&pc) {
            Some(Slot::Hot { artifact, runs }) => {
                *runs += 1;
                Some(artifact)
            }
            _ => None,
        }
    }

    /// Total number of compiled-block invocations across the cache.
    pub fn total_runs(&self) -> u64 {
        self.slots
            .values()
            .map(|s| match s {
                Slot::Hot { runs, .. } => *runs,
                Slot::Cold { .. } => 0,
            })
            .sum()
    }

    /// Drop **everything** — the invalidate-all-on-flash-write policy.
    /// Cold hit counters are dropped too, so post-write code re-warms from
    /// scratch (correct, and cheap because flash writes are rare).
    pub fn invalidate_all(&mut self) {
        self.slots.clear();
        self.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trivial artifact stand-in for cache tests.
    #[derive(Debug, PartialEq, Eq)]
    struct FakeArtifact(u32);

    #[test]
    fn cold_pc_promotes_exactly_at_threshold() {
        let mut cache: BlockCache<FakeArtifact> = BlockCache::new(3);
        // hit 1, 2 -> interpret, no promote
        assert_eq!(cache.observe(0x1000), Lookup::Interpret { promote: false });
        assert_eq!(cache.observe(0x1000), Lookup::Interpret { promote: false });
        // hit 3 -> crosses threshold, promote signalled once
        assert_eq!(cache.observe(0x1000), Lookup::Interpret { promote: true });
        // still not installed until the caller installs
        assert!(!cache.is_hot(0x1000));
        // subsequent cold hits do NOT re-signal promote
        assert_eq!(cache.observe(0x1000), Lookup::Interpret { promote: false });
    }

    #[test]
    fn threshold_of_one_promotes_on_first_sight() {
        let mut cache: BlockCache<FakeArtifact> = BlockCache::new(1);
        assert_eq!(cache.observe(0x2000), Lookup::Interpret { promote: true });
    }

    #[test]
    fn install_makes_pc_ready_and_counts_runs() {
        let mut cache: BlockCache<FakeArtifact> = BlockCache::new(2);
        cache.observe(0x3000);
        cache.install(0x3000, FakeArtifact(7));
        assert!(cache.is_hot(0x3000));
        assert_eq!(cache.observe(0x3000), Lookup::Ready);
        assert_eq!(cache.compiled_len(), 1);

        assert_eq!(cache.run_artifact(0x3000), Some(&mut FakeArtifact(7)));
        cache.run_artifact(0x3000);
        assert_eq!(cache.total_runs(), 2);
        // unknown PC has no artifact
        assert!(cache.run_artifact(0x9999).is_none());
    }

    #[test]
    fn invalidate_all_drops_compiled_and_cold_and_bumps_generation() {
        let mut cache: BlockCache<FakeArtifact> = BlockCache::new(1);
        cache.observe(0x4000);
        cache.install(0x4000, FakeArtifact(1));
        cache.observe(0x5000); // cold counter present
        assert!(cache.is_hot(0x4000));
        assert_eq!(cache.compiled_len(), 1);
        assert_eq!(cache.generation(), 0);

        cache.invalidate_all();

        assert!(!cache.is_hot(0x4000));
        assert_eq!(cache.compiled_len(), 0);
        assert_eq!(cache.total_runs(), 0);
        assert_eq!(cache.generation(), 1);
        // a previously-warm cold PC starts over from hit 1
        assert_eq!(cache.observe(0x5000), Lookup::Interpret { promote: true });
    }

    #[test]
    fn distinct_pcs_have_independent_counters() {
        let mut cache: BlockCache<FakeArtifact> = BlockCache::new(2);
        assert_eq!(cache.observe(0xA), Lookup::Interpret { promote: false });
        // 0xB's first hit must not inherit 0xA's counter
        assert_eq!(cache.observe(0xB), Lookup::Interpret { promote: false });
        assert_eq!(cache.observe(0xA), Lookup::Interpret { promote: true });
        assert_eq!(cache.observe(0xB), Lookup::Interpret { promote: true });
    }
}
