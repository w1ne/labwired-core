// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Chip-specific interrupt-fabric state, held off the shared [`SystemBus`]
//! field list.
//!
//! # Why this module exists
//!
//! A bus is shared by every chip family we model. An interrupt fabric is not:
//! the ESP32-C3 routes 128 matrix sources through the RISC-V `INTERRUPT_CORE0`
//! bank into a 32-bit line mask the core ORs into `mip`, while the ESP32-S3
//! routes the same 128 source IDs through a dual-core Xtensa matrix into a
//! per-core CPU-interrupt-slot bitmap plus an `INTR_STATUS` mirror that
//! esp-hal reads back. Those are different silicon, and their state used to
//! sit as eleven loose fields on `SystemBus` — three of them `pub` and two of
//! those named for a chip, so every reader of the shared bus type read two
//! SoC part numbers.
//!
//! This module is that seam. The fabrics keep their names, their registers and
//! their separate state — that difference is real and must not be flattened —
//! but they live behind ONE `SystemBus` field, and a new family's fabric is a
//! type here rather than another handful of fields on the shared bus.
//!
//! # What this is NOT
//!
//! It is not a trait with one `recompute()`. The two fabrics do not share an
//! output: the C3 produces a line mask, the S3 produces a per-core slot bitmap
//! plus a register mirror. What they DO share — polling the live level of every
//! scheduler-driven peripheral — is already one function on the bus
//! (`poll_scheduler_matrix_sources`); each fabric folds that identical bitmap
//! into its own routing. Only the genuinely fabric-independent questions
//! ("does a matrix own `pending_cpu_irqs`?", "does any fabric still need a
//! per-cycle aggregation pass?") are answered here as chip-neutral predicates,
//! and they are what shared bus code calls.
//!
//! # Independence of the two fabrics
//!
//! The two are held side by side rather than as an exclusive enum. No SoC has
//! both, but the two flags are set by DIFFERENT disciplines — the S3 one is
//! *derived* in `rebuild_peripheral_ranges` from the presence of the intmatrix
//! peripheral, the C3 one is *asserted* by the C3 ROM-boot path — and
//! `c3_and_s3_interrupt_routing_caches_are_separate` pins that setting one
//! never implies or clears the other. Collapsing them into one variant would
//! change what a bus carrying both is allowed to do, which is a claim about
//! behaviour, not a rename. So it is not made here.

/// Every chip-specific interrupt fabric this bus could be routing through.
///
/// One field on [`SystemBus`](crate::bus::SystemBus) replacing eleven. Both
/// members are always present and inert by default: an inactive fabric costs
/// the flag reads its predicates already made as loose bools.
#[derive(Debug, Clone, Default)]
pub struct InterruptFabric {
    /// ESP32-C3 (RISC-V `INTERRUPT_CORE0` matrix).
    pub esp32c3: Esp32c3Fabric,
    /// ESP32-S3 (dual-core Xtensa interrupt matrix).
    pub esp32s3: Esp32s3Fabric,
}

impl InterruptFabric {
    /// True when a chip interrupt MATRIX owns the routed CPU-interrupt state,
    /// so a second fabric must not also write it.
    ///
    /// The classic-ESP32 DPORT aggregation asks this before rebuilding
    /// [`SystemBus::pending_cpu_irqs`](crate::bus::SystemBus::pending_cpu_irqs):
    /// on an S3 the intmatrix owns that bitmap, and on a C3 the routed output
    /// is the RISC-V line mask instead, so in either case DPORT routing would
    /// be writing over a fabric that already answered.
    #[inline]
    pub fn matrix_owns_cpu_irqs(&self) -> bool {
        self.esp32c3.routing || self.esp32s3.routing
    }

    /// True when no active fabric needs the per-cycle walk to aggregate
    /// interrupt levels — i.e. the walk-free per-cycle tick can skip it.
    ///
    /// `walk_deleted` is the caller's `SystemBus::legacy_walk_disabled`. Both
    /// arms depend on it and neither can read it: a fabric holds interrupt
    /// state, not the bus's walk policy. It is a PARAMETER rather than an
    /// unstated precondition because the two escape hatches below are only
    /// sound on a walk-DELETED bus — on a walk-ON bus the per-tick aggregation
    /// still owns level derivation (`irq_fabric.*.walk_sources` is rebuilt from
    /// the walk's emitted source IDs every tick) and skipping it would drop
    /// every walk-emitted source. The sole caller
    /// (`SystemBus::per_cycle_tick_is_trivial`) already required this; passing
    /// it makes the predicate answerable on its own terms instead of true-by-
    /// convention.
    ///
    /// An active C3 fabric qualifies once its declarative INTC cache is
    /// populated, because levels are then re-derived at the MMIO write choke
    /// (`sync_esp32c3_irq_cache_write`) and the event path
    /// (`deliver_scheduled_irq_levels`) instead of at the tick. Without the
    /// cache `aggregate_esp32c3_irqs` reads the routing registers directly and
    /// is the ONLY aggregation point, so the tick has to keep running.
    ///
    /// An active S3 fabric qualifies with no extra condition, and the asymmetry
    /// is real rather than an oversight. The C3's `intc` cache exists because
    /// its `INTERRUPT_CORE0` bank is a DECLARATIVE peripheral: routing has to
    /// decode `MAP`/`CPU_INT_ENABLE`/`CPU_INT_PRI_n`/`CPU_INT_THRESH` out of a
    /// register file, and `intc.is_some()` is exactly "that decode exists".
    /// The S3 intmatrix is a NATIVE model
    /// ([`Esp32s3IntMatrix`](crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix))
    /// that already holds its per-core routing as decoded `[Option<u8>; 99]`
    /// tables maintained at its own `write`, and `route_irq_source_to_cpu_irq_core`
    /// is an array index into them. There is nothing left to cache: a mirror on
    /// this fabric would be a SECOND copy of the MAP tables with its own
    /// invalidation to get wrong, buying an array read that is already O(1) and
    /// that the walk-free path performs only at a write choke or a scheduler
    /// event, never per cycle. So the S3 arm gates on the same thing the C3 arm
    /// really gates on — "the routed state is derived at the choke, not at the
    /// tick" — which on the S3 is true as soon as the walk is deleted.
    #[inline]
    pub fn per_cycle_aggregation_free(&self, walk_deleted: bool) -> bool {
        if !walk_deleted {
            return !self.matrix_owns_cpu_irqs();
        }
        !self.esp32c3.routing || self.esp32c3.intc.is_some()
    }
}

/// ESP32-C3 interrupt-fabric state: the RISC-V `INTERRUPT_CORE0` matrix.
///
/// Sources (0..127) are mapped to CPU lines (1..31) by `MAP` registers at
/// `0x600C_2000 + source*4`, gated by `CPU_INT_ENABLE` (0x104) and per-line
/// priority (`CPU_INT_PRI_n`, 0x114 + n*4) against `CPU_INT_THRESH` (0x194).
/// The `SYSTEM` bank's `FROM_CPU_INTR_n` doorbells (`0x600C_0028..0x34`) enter
/// the same matrix as sources 50..53 — the FreeRTOS yield mechanism.
#[derive(Debug, Clone, Default)]
pub struct Esp32c3Fabric {
    /// When true, each tick the bus routes asserted peripheral sources and the
    /// SYSTEM `FROM_CPU` IPI registers through the `INTERRUPT_CORE0` matrix
    /// `MAP` registers into [`Self::irq_lines`]. Set by the C3 rom-boot setup;
    /// false everywhere else, so no other architecture's bus is affected.
    ///
    /// Deliberately NOT derived from the presence of the C3 interrupt banks
    /// (unlike the S3 flag next door): a bus can carry a declarative
    /// `interrupt_core0` peripheral without the ROM-boot wiring that makes
    /// matrix routing correct, and deriving it would silently switch such a
    /// bus onto the matrix path.
    pub routing: bool,
    /// Level-sensitive bitmask of asserted CPU interrupt lines (1..31),
    /// recomputed by `recompute_esp32c3_irq_lines`. Read by the RISC-V core
    /// via `Bus::external_irq_lines`. 0 while [`Self::routing`] is false.
    pub irq_lines: u32,
    /// Peripheral index of the C3 `SYSTEM` bank (`0x600C_0000`), cached by
    /// `rebuild_peripheral_ranges`. `None` on every non-C3 bus.
    pub(crate) system_idx: Option<usize>,
    /// Peripheral index of the C3 `INTERRUPT_CORE0` bank (`0x600C_2000`),
    /// cached by `rebuild_peripheral_ranges`. `None` on every non-C3 bus.
    pub(crate) interrupt_core0_idx: Option<usize>,
    /// Decoded mirror of the `INTERRUPT_CORE0` register file, maintained at the
    /// MMIO write choke so routing never re-reads the register bank per tick.
    /// `None` on a hand-built bus with no declarative INTC, where
    /// `aggregate_esp32c3_irqs` falls back to reading the registers directly.
    pub(crate) intc: Option<Esp32c3IntcCache>,
    /// Bitmap (128 sources) of the matrix source IDs asserted by the most
    /// recent peripheral WALK tick (`explicit_irqs` — e.g. the SYSTIMER alarm
    /// on source 37). Stored so the write-choke re-aggregation can recombine
    /// them with the `FROM_CPU`/INTC state without waiting for the next tick.
    /// Level semantics: rebuilt from scratch each tick, so a source that stops
    /// asserting drops out at the next tick boundary (≤ one
    /// `peripheral_tick_interval` — the same bound as the write path).
    pub(crate) walk_sources: [u64; 2],
    /// Matrix sources asserted by SCHEDULER-driven peripherals (the SYSTIMER
    /// alarm once migrated off the walk). The per-cycle walk rebuilds
    /// [`Self::walk_sources`] from scratch each tick and skips scheduler-driven
    /// peripherals, so their level would drop every tick; this bitmap is
    /// re-derived from `Peripheral::matrix_irq_sources` at the event path and
    /// the walk-tick aggregation, and OR-ed with [`Self::walk_sources`] in
    /// `recompute_esp32c3_irq_lines`. Same level semantics, so delivery matches
    /// the legacy walk cycle-for-cycle at a given tick interval.
    pub(crate) sched_sources: [u64; 2],
}

/// ESP32-S3 interrupt-fabric state: the dual-core Xtensa interrupt matrix.
///
/// The same 128 source IDs the C3 uses, but each source is bound per CORE to a
/// CPU interrupt slot by that core's `MAP` table, and the routed result is a
/// two-element slot bitmap plus a `PRO_INTR_STATUS_REG_n` mirror esp-hal's
/// `__level_*_interrupt` reads back to discover which source fired.
#[derive(Debug, Clone, Default)]
pub struct Esp32s3Fabric {
    /// True exactly while the S3 intmatrix peripheral is registered. DERIVED in
    /// `rebuild_peripheral_ranges` (see [`Esp32c3Fabric::routing`], which is
    /// not) — an S3 bus is an S3 bus the moment it carries the matrix model.
    pub routing: bool,
    /// Peripheral index of the S3 intmatrix, cached alongside
    /// [`Self::routing`]. `None` on every other bus.
    pub(crate) intmatrix_idx: Option<usize>,
    /// Bitmap (128 sources) of the intmatrix source IDs asserted by the most
    /// recent peripheral WALK tick — the mirror of
    /// [`Esp32c3Fabric::walk_sources`], with the same level semantics.
    pub(crate) walk_sources: [u64; 2],
    /// Intmatrix sources asserted by SCHEDULER-driven peripherals — the mirror
    /// of [`Esp32c3Fabric::sched_sources`], UNIONED with
    /// [`Self::walk_sources`] in `recompute_esp32s3_irq_lines`.
    pub(crate) sched_sources: [u64; 2],
}

/// Decoded `INTERRUPT_CORE0` register state (see [`Esp32c3Fabric::intc`]).
///
/// Register offsets verified against `interrupt_core0.yaml`: `CPU_INT_ENABLE`
/// 0x104, `CPU_INT_PRI_n` 0x114 + n*4, `CPU_INT_THRESH` 0x194, and the
/// per-source `MAP` registers at source*4.
#[derive(Clone, Debug)]
pub struct Esp32c3IntcCache {
    pub int_enable: u32,
    pub int_thresh: u8,
    pub source_line: [u8; 128],
    pub line_pri: [u8; 32],
    pub from_cpu_pending: u8,
}

impl Default for Esp32c3IntcCache {
    fn default() -> Self {
        Self {
            int_enable: 0,
            int_thresh: 0,
            source_line: [0; 128],
            line_pri: [0; 32],
            from_cpu_pending: 0,
        }
    }
}

/// Differential audit of the ESP32-S3 walk-free interrupt path.
///
/// # Why this exists
///
/// Opening [`InterruptFabric::per_cycle_aggregation_free`] for the S3 is a
/// claim: that the routed state left behind by the write choke and the event
/// path is, at EVERY bus boundary, bit-identical to what re-polling every
/// scheduler-driven peripheral would have produced. That claim is exactly the
/// kind that is easy to assert and easy to get wrong in a way no functional
/// test notices — a level that de-asserts one boundary late still renders the
/// same pixels, right up to the run where it does not.
///
/// So it is measured instead of asserted. With an audit installed
/// (`SystemBus::install_esp32s3_irq_audit`) every walk-free boundary computes
/// the answer BOTH ways: the CACHED one already sitting in `pending_cpu_irqs`
/// and the intmatrix `INTR_STATUS` mirror, against the POLLED one from a fresh
/// `poll_scheduler_matrix_sources` and recompute. Any disagreement is recorded
/// with the cycle it happened on. `esp32s3_irq_cache_differential` runs real S3
/// firmware with the audit on and fails if this is non-empty.
///
/// The audit LEAVES the polled answer in place, so an audited run behaves as
/// the pre-optimisation build did. It is a measurement harness, not a
/// fallback: divergence is reported, never repaired.
#[derive(Debug, Default, Clone)]
pub struct Esp32s3IrqAudit {
    /// Walk-free bus boundaries audited. Zero means the gate never ran —
    /// a fully-skipped audit reads exactly like a passing one, so tests assert
    /// on this too.
    pub boundaries: u64,
    /// Boundaries at which the routed per-core slot bitmap was NON-ZERO, i.e.
    /// an interrupt was actually pending at the cores. Anti-vacuity: a run that
    /// never routes an interrupt agrees with a re-poll trivially, so a gate
    /// that only checked `divergences.is_empty()` would pass on a workload
    /// whose interrupt path never came up.
    pub boundaries_with_routed_irq: u64,
    /// Boundaries at which at least one scheduler-driven matrix SOURCE was
    /// asserting. Reported so a workload that turns out never to raise an
    /// interrupt says so out loud instead of passing quietly.
    pub boundaries_with_sched_sources: u64,
    /// Union over the run of every scheduler-driven matrix source seen
    /// asserting — a bitmap of source IDs, so a failure names the peripherals
    /// that were actually live.
    pub sched_source_union: [u64; 2],
    /// Every boundary at which the cached and polled answers disagreed, capped
    /// at [`Self::MAX_RECORDED`] so a systematically broken build reports the
    /// first failures instead of exhausting memory.
    pub divergences: Vec<Esp32s3IrqDivergence>,
    /// Total disagreements, including those past [`Self::MAX_RECORDED`].
    pub divergence_count: u64,
}

impl Esp32s3IrqAudit {
    /// How many divergences are retained in full. Past this only the count
    /// grows.
    pub const MAX_RECORDED: usize = 16;
}

/// One boundary at which the cached and polled S3 routed state disagreed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Esp32s3IrqDivergence {
    /// `SystemBus::current_cycle` at the audited boundary.
    pub cycle: u64,
    /// Per-core CPU-interrupt slot bitmap the walk-free path had left behind.
    pub cached_routed: [u32; 2],
    /// The same bitmap re-derived by polling every scheduler-driven peripheral.
    pub polled_routed: [u32; 2],
    /// `INTR_STATUS` mirror the walk-free path had left behind.
    pub cached_intr_status: [u32; 4],
    /// The same mirror re-derived by polling.
    pub polled_intr_status: [u32; 4],
    /// Scheduler-driven matrix source bitmap the walk-free path had cached.
    pub cached_sched_sources: [u64; 2],
    /// The same bitmap re-derived by polling — the ROOT input, so a divergence
    /// here names a peripheral whose level moved with no write and no event.
    pub polled_sched_sources: [u64; 2],
}
