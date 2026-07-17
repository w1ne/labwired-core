// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::memory::LinearMemory;
use crate::peripherals::nvic::NvicState;
use crate::peripherals::uart::Uart;
use crate::{Bus, Peripheral, SimResult};
use labwired_config::{parse_size, ChipDescriptor, PeripheralConfig, SystemManifest};
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;

mod accessors;
mod attach;
pub mod bus_trace;
mod can_devices;
mod construct;
mod device_hooks;
mod embedded_descriptors;
mod faults;
mod from_config;
mod mmio_activity;
mod mmio_words;
mod policy;
mod profiles;
mod routing;
mod sim_inputs;
mod tick;

pub use can_devices::*;

pub use bus_trace::{new_log, BusPayload, BusTraceEvent, BusTraceLog, I2cSym};

impl SystemBus {
    /// Describe the currently active legacy per-step entries.
    ///
    /// This is intentionally a diagnostic view of the assembled bus, not a
    /// second execution path.  Consumers use it to tie profile entries back
    /// to the concrete device window before attempting a scheduler migration.
    pub fn legacy_tick_entry_descriptors(&self) -> Vec<(String, u64, u64)> {
        self.legacy_tick_indices
            .iter()
            .filter_map(|&idx| {
                self.peripherals
                    .get(idx)
                    .map(|p| (p.name.clone(), p.base, p.size))
            })
            .collect()
    }

    #[inline]
    fn legacy_tick_index_active(p: &PeripheralEntry) -> bool {
        if cfg!(feature = "event-scheduler") && p.dev.uses_scheduler() {
            return false;
        }
        p.dev.legacy_tick_active()
    }

    /// True when CPU idle fast-forward can skip the legacy peripheral walk for
    /// the skipped window without dropping observable work. Scheduler-driven
    /// peripherals are safe because the machine clamps to their next deadline;
    /// inert or currently-inactive legacy peripherals have no tick output to
    /// lose. Active non-scheduler legacy work blocks fast-forward until the
    /// normal tick path drains it.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn idle_fast_forward_legacy_safe(&self) -> bool {
        self.legacy_walk_disabled
            || self
                .peripherals
                .iter()
                .all(|p| p.dev.uses_scheduler() || !p.dev.legacy_tick_active())
    }
}

/// A peripheral's RCC clock-gate, resolved to a concrete RCC register offset +
/// bit at bus-build time (the symbolic `reg` name from the yaml is mapped to the
/// active chip family's offset via [`Rcc::enable_reg_offset`]). When present, a
/// CPU access to the owning peripheral only takes effect while `bit` is set in
/// the RCC enable register at `reg_offset` — modelling silicon clock-gating.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedClockGate {
    /// Byte offset of the RCC enable register within the rcc peripheral.
    pub reg_offset: u64,
    /// Enable-bit position within that register.
    pub bit: u8,
}

/// The `peripheral_tick_interval` recommended for a fully scheduler-driven
/// (walk-deleted) bus — see [`SystemBus::max_safe_tick_interval`]. Native
/// C3 OLED throughput keeps climbing through a few hundred (host drain tax
/// falls as `avg_batch` tracks the interval) and plateaus near 512–1k.
/// SSD1306 framebuffer stays byte-identical to interval 1 at 512 (see
/// `oled_lab_framebuffer_is_byte_identical_at_tick_512`). Event delivery is
/// still exact via the scheduler deadline clamp; 512 only reduces how often
/// the host runs the empty walk-deleted tick.
pub const RECOMMENDED_TICK_INTERVAL: u32 = 512;

pub struct PeripheralEntry {
    pub name: String,
    pub base: u64,
    pub size: u64,
    pub irq: Option<u32>,
    pub dev: Box<dyn Peripheral>,
    pub ticks_remaining: u64,
    /// Phase 2B.1 (issue #192): lazy cancel token for the event scheduler.
    /// Bumped when the peripheral resets; `EventScheduler::drain_due` drops
    /// entries whose generation no longer matches the snapshot.
    pub generation: u32,
    /// Optional RCC clock-gate (silicon clock-gating model). `None` (the common
    /// case) → the peripheral is never gated and accesses always pass through.
    /// `Some` → accesses are dropped (writes ignored, reads return 0) while the
    /// gate bit is clear in the RCC, exactly like an unclocked peripheral on
    /// real silicon. Resolved from `PeripheralConfig::clock` in `from_config`.
    pub clock_gate: Option<ResolvedClockGate>,
}

/// RP2040 atomic register-alias operation (see
/// [`SystemBus::atomic_alias_redirect`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicAliasOp {
    /// `+0x1000`: write XORs the bits, read returns the base register.
    Xor,
    /// `+0x2000`: write sets (ORs) the bits.
    Set,
    /// `+0x3000`: write clears (AND-NOT) the bits.
    Clr,
}

#[derive(Clone, Debug)]
pub(crate) struct Esp32c3IrqCache {
    pub int_enable: u32,
    pub int_thresh: u8,
    pub source_line: [u8; 128],
    pub line_pri: [u8; 32],
    pub from_cpu_pending: u8,
}

impl Default for Esp32c3IrqCache {
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

pub struct SystemBus {
    pub flash: LinearMemory,
    pub ram: LinearMemory,
    /// Extra CPU-visible RAM/ROM windows beyond `flash`/`ram` (e.g. ESP32 IRAM
    /// `0x4037C000` and flash-DROM `0x3C000000`), from the chip's
    /// `memory_regions`. Checked after `ram`/`flash`, before peripherals.
    pub extra_mem: Vec<LinearMemory>,
    pub peripherals: Vec<PeripheralEntry>,
    pub nvic: Option<Arc<NvicState>>,
    pub observers: Vec<Arc<dyn crate::SimulationObserver>>,
    pub config: crate::SimulationConfig,
    /// Enable Cortex-M peripheral/SRAM bit-band alias translation.
    /// False for architectures (e.g. RISC-V) whose memory maps collide with
    /// the bit-band alias ranges 0x42000000–0x44000000 / 0x22000000–0x24000000.
    pub bit_band_enabled: bool,
    /// Offset (bytes) from the flash base to the application vector table when
    /// a second-stage bootloader precedes it (RP2040 boot2 = `0x100`). `0`
    /// means the vector table sits at the flash base. Carried from the chip
    /// descriptor so `Machine::load_firmware` can relocate the reset vector
    /// past the stage-2 blob. See `ChipDescriptor::reset_vector_offset`.
    pub reset_vector_offset: u64,
    /// RP2040 atomic register aliases enabled (see
    /// `ChipDescriptor::atomic_register_aliases`). When set, word accesses in
    /// the APB peripheral window whose offset has bits [13:12] set decode as
    /// XOR/SET/CLR atomic ops on the aligned base register.
    pub atomic_register_aliases: bool,
    /// Plan 3: per-core bitmask of pending cpu IRQ slots (32 bits each;
    /// index 0 = PRO_CPU, 1 = APP_CPU). Aggregated by
    /// `tick_peripherals_with_costs` from peripheral `explicit_irqs` source
    /// IDs routed through the registered interrupt matrix's per-core map
    /// tables. Cleared per slot via `clear_cpu_irq_pending`.
    pub pending_cpu_irqs: [u32; 2],
    /// Bus-level thunk table for addresses outside any `RomThunkBank`.
    /// Used to intercept calls to firmware functions resident in flash
    /// (e.g. ESP-IDF's `multi_heap_register` at 0x40194954) so we can
    /// substitute a sim-side Rust implementation. To install: write
    /// BREAK 1,14 bytes (`ROM_THUNK_BREAK_BYTES`) at `pc` in flash AND
    /// `bus.flash_thunks.insert(pc, thunk)`. The CPU's BREAK 1,14
    /// dispatcher (xtensa_lx7.rs) calls `bus.get_rom_thunk(pc)` which
    /// checks both this table and any `RomThunkBank` peripherals.
    pub flash_thunks: std::collections::HashMap<
        u32,
        crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkFn,
    >,
    peripheral_ranges: Vec<PeripheralRange>,
    legacy_tick_indices: Vec<usize>,
    bus_tick_indices: Vec<usize>,
    /// Indices of peripherals with `uses_scheduler() == true`. Filled in
    /// `rebuild_peripheral_ranges` so IRQ-level re-derivation on MMIO write
    /// can poll only those models (not the full bus).
    scheduler_driver_indices: Vec<usize>,
    peripheral_hint: Cell<Option<usize>>,
    /// Last **winning** peripheral window from [`find_peripheral_index`]:
    /// `(range_ord, start, end, peri_index)` where `range_ord` is the index
    /// into `peripheral_ranges`. Same-window sequential accesses are O(1) when
    /// the next sorted range starts past `addr` (no narrower window can win).
    /// Cleared on range rebuild. Fidelity: greatest-start-wins, history-independent
    /// (see `overlapping_windows_route_history_independently`).
    last_route: Cell<Option<(usize, u64, u64, usize)>>,
    /// Cached index of the classic-ESP32 DPORT peripheral, if one is
    /// registered (`None` otherwise — the common case, incl. every ESP32-S3
    /// bus). Recomputed in `rebuild_peripheral_ranges` on each peripheral
    /// add/refresh, same staleness contract as `peripheral_ranges`. Lets
    /// `dport_cross_core_pending` — called on the per-step interrupt path —
    /// skip an O(peripherals) scan that would otherwise return 0 every step
    /// on buses with no DPORT.
    dport_idx: Option<usize>,
    /// Cached index of the "rcc" peripheral, if one is registered. Recomputed in
    /// `rebuild_peripheral_ranges` (same staleness contract as `dport_idx`). Lets
    /// the clock-gate check on the hot read/write path resolve the RCC peripheral
    /// in O(1) instead of scanning by name. `None` on buses with no RCC (e.g.
    /// most non-STM32 chips), in which case no peripheral is ever gated.
    rcc_idx: Option<usize>,
    /// Measurement-only escape hatch: when `true`, [`is_peripheral_clocked`]
    /// short-circuits to `true` so RCC clock-gating never suppresses an access.
    /// Off by default (the runtime always gates); only diagnostic tooling such
    /// as the SVD register-coverage probe flips it on via
    /// [`set_clock_gating_bypass`].
    clock_gating_bypass: bool,
    /// `missing_clock` fault injection: peripheral indices forced unclocked,
    /// mapped to a count of accesses suppressed because of the fault (the
    /// runtime fired-observation). Empty in the common case.
    fault_unclocked: std::collections::HashMap<usize, std::sync::atomic::AtomicU64>,
    /// Last-known IN value of GPIO ports 0 and 1, used by the per-tick
    /// edge-detection pass that drives GPIOTE EVENTS_IN. Both default to
    /// 0 at construction; the first tick after a GPIO write will produce
    /// edge events for any non-zero bits, which matches Nordic
    /// hardware's "reset to zero, edge on first set" behavior.
    last_gpio_in: [u32; 2],
    /// Phase 2B.2 (issue #192): the current CPU cycle count, mirrored from
    /// `Machine::total_cycles` once per step. Read by the MMIO write path to
    /// lazily sync scheduler-driven peripherals (`uses_scheduler() == true`)
    /// to "now" before a register write observes their state. Only consulted
    /// under the `event-scheduler` feature; harmlessly 0 otherwise.
    ///
    /// Prefer [`Self::set_current_cycle`] over assigning this field directly:
    /// the setter also publishes the value into [`Self::cycle_clock`], the
    /// shared clock `&self` peripheral reads sync against.
    pub current_cycle: u64,
    /// Walk-free plan Part 1: the shared cycle clock (`Arc<AtomicU64>`)
    /// published in lock-step with `current_cycle` (via
    /// [`Self::set_current_cycle`]) and handed to every peripheral at
    /// [`Self::add_peripheral`] time via `Peripheral::attach_cycle_clock`.
    /// Lets a `&self` MMIO read lazily sync `Cell`-held counter state to the
    /// batch-start cycle — the read-side complement of the write-path
    /// `sync_to`, with the identical "< one tick interval" freshness bound.
    pub cycle_clock: crate::CycleClock,
    /// Phase 2B.3a (issue #192): write-context schedule requests buffered
    /// during MMIO writes. A scheduler-driven peripheral can't reach the
    /// scheduler from `write`, so after the write the bus collects its
    /// `take_scheduled_events()` here as `(peripheral_idx, deadline_cycle,
    /// token)` — an ABSOLUTE CPU-cycle deadline, converted from the
    /// peripheral's relative delay at collect time (see
    /// `collect_scheduled_events`); `Machine::drain_scheduler_events` enqueues
    /// (clamped to its `now`) and clears them. Only populated under the
    /// `event-scheduler` feature.
    pub pending_schedule: Vec<(usize, u64, u32)>,
    /// Batch-local count of [`MmioAccessClass::FreerunningTimerPoll`] accesses.
    /// Classification is **peripheral-owned** (CPU-agnostic bus); see
    /// [`Peripheral::mmio_access_class`].
    freerunning_timer_poll_mmio: std::cell::Cell<u32>,
    /// Batch-local count of [`MmioAccessClass::SideEffecting`] accesses.
    /// Any non-zero value disqualifies timer-poll coalesce for that batch.
    side_effecting_mmio: std::cell::Cell<u32>,
    /// Phase 2B.3c (issue #192): when true, `tick_peripherals_phase1` skips the
    /// entire per-cycle peripheral walk — the actual ~2.4x win. Set ONLY for a
    /// config whose every peripheral is migrated (`uses_scheduler`) or inert
    /// (no real `tick()` work), e.g. ESP32-classic via `configure_xtensa_esp32`.
    /// Read only under the `event-scheduler` feature; flag-off the walk always
    /// runs, so the shipped build is unchanged.
    pub legacy_walk_disabled: bool,
    /// HC-SR04 ultrasonic sensors wired to GPIO TRIG/ECHO pins. The echo window
    /// is armed by the TRIG GPIO write-hook (`maybe_arm_hcsr04`); a cheap
    /// per-tick pass (`service_hcsr04`) drives the computed ECHO input level,
    /// touching the bus only on a transition. Empty by default → zero cost.
    pub hcsr04: Vec<crate::peripherals::hc_sr04::HcSr04>,
    /// TM1637 4-digit 7-segment displays bit-banged over two GPIO lines. Each is
    /// driven by the CLK/DIO GPIO write-hook (`maybe_clock_tm1637`), which feeds
    /// line transitions to the display's protocol state machine. Purely
    /// write-driven (no per-tick pass). Empty by default → zero cost.
    pub tm1637: Vec<crate::peripherals::components::tm1637_7seg::Tm1637>,
    /// Reusable CAN diagnostic clients declared as external devices. They
    /// inject configured CAN frames into a named FDCAN peripheral once it is
    /// running, so ECU examples can be driven by a virtual off-board tester
    /// instead of self-loopback firmware.
    pub can_diagnostic_testers: Vec<CanDiagnosticTester>,
    /// Stateful ISO-TP/UDS testers declared as external devices. Each is a real
    /// second CAN node driving a multi-frame SecurityAccess exchange against a
    /// named CAN peripheral (bxCAN or FDCAN) running in normal mode. Empty by
    /// default → zero per-tick cost.
    pub can_uds_testers: Vec<CanUdsTester>,
    /// Deterministic CAN log replay nodes (candump-sourced). Each delivers
    /// pre-parsed frames into a named bxCAN/FDCAN peripheral at scheduled
    /// tick offsets. Empty by default → zero per-tick cost.
    pub can_log_players: Vec<CanLogPlayer>,
    /// ESP32-C3 (RISC-V) interrupt routing: when true, each tick the bus routes
    /// asserted peripheral sources and the SYSTEM FROM_CPU IPI registers
    /// (0x600C0028..0x34) through the INTERRUPT_CORE0 matrix MAP registers into
    /// `riscv_irq_lines`. Set by the C3 rom-boot setup; false everywhere else
    /// so no other architecture's bus is affected.
    pub esp32c3_irq_routing: bool,
    /// ESP32-C3 level-sensitive bitmask of asserted CPU interrupt lines (1..31),
    /// recomputed every tick by `aggregate_esp32c3_irqs`. Read by the RISC-V
    /// core via `Bus::external_irq_lines`. 0 when `esp32c3_irq_routing` is false.
    pub riscv_irq_lines: u32,
    /// ESP32-C3 declarative interrupt banks. Cached separately from S3's
    /// intmatrix so each chip keeps its own interrupt-controller abstraction.
    esp32c3_system_idx: Option<usize>,
    esp32c3_interrupt_core0_idx: Option<usize>,
    esp32c3_irq_cache: Option<Esp32c3IrqCache>,
    /// Bitmap (128 sources) of the interrupt-matrix source IDs asserted by the
    /// most recent peripheral tick (`explicit_irqs` from the walk — e.g. the
    /// SYSTIMER alarm on source 37). Stored so the write-choke re-aggregation
    /// (`sync_esp32c3_irq_cache_write` → `recompute_esp32c3_irq_lines`) can
    /// recombine them with the FROM_CPU/INTC state without waiting for the
    /// next tick. Level semantics: rebuilt from scratch each tick, so a source
    /// that stops asserting drops out at the next tick boundary (≤ one
    /// `peripheral_tick_interval` — the same bound as the write path).
    esp32c3_asserted_sources: [u64; 2],
    /// C3 matrix sources asserted by SCHEDULER-driven peripherals (currently
    /// the SYSTIMER alarm once migrated off the walk). The per-cycle walk
    /// rebuilds `esp32c3_asserted_sources` from scratch each tick and skips
    /// scheduler-driven peripherals, so their level would drop every tick;
    /// this bitmap is re-derived from `Peripheral::matrix_irq_sources` at the
    /// event path (`apply_event_result`) and the walk-tick aggregation, and
    /// OR-ed with `esp32c3_asserted_sources` in `recompute_esp32c3_irq_lines`.
    /// Same level semantics (a source that stops asserting drops out at the
    /// next re-derivation), so delivery matches the legacy walk cycle-for-cycle
    /// at a given tick interval.
    esp32c3_sched_asserted_sources: [u64; 2],
    /// ESP32-S3 interrupt routing is present only when the S3 interrupt matrix
    /// peripheral is registered. Cached separately from C3's RISC-V routing so
    /// each chip model owns its own interrupt abstraction.
    pub esp32s3_irq_routing: bool,
    esp32s3_intmatrix_idx: Option<usize>,
    /// Bitmap (128 sources) of the intmatrix source IDs asserted by the most
    /// recent peripheral WALK tick (`explicit_irqs`, e.g. a not-yet-migrated
    /// timer_group source). Persisted — mirror of C3's `esp32c3_asserted_sources`
    /// — so the event path (`recompute_esp32s3_irq_lines`) can re-derive the
    /// routed `pending_cpu_irqs` + intmatrix INTR_STATUS mirror from the union of
    /// walk + scheduler levels without dropping a concurrent walk source. Level
    /// semantics: rebuilt from scratch each walk tick, so a source that stops
    /// asserting drops out at the next tick boundary.
    esp32s3_asserted_sources: [u64; 2],
    /// S3 intmatrix sources asserted by SCHEDULER-driven peripherals (the
    /// SYSTIMER alarm once migrated off the walk). The per-cycle walk skips
    /// scheduler-driven peripherals, so their level would never reach the
    /// intmatrix; this bitmap is re-derived from `Peripheral::matrix_irq_sources`
    /// at the event path (`apply_event_result` → `deliver_scheduled_irq_levels`)
    /// and the walk-tick aggregation, and UNIONED with `esp32s3_asserted_sources`
    /// in `recompute_esp32s3_irq_lines`. Same level semantics as the C3 field, so
    /// delivery matches the legacy walk cycle-for-cycle at a given tick interval.
    esp32s3_sched_asserted_sources: [u64; 2],
    /// True when a FLASH peripheral on this bus models hardware operations
    /// (H5 sector erase / bank swap) as pending ops that the machine layer must
    /// drain and apply per instruction. Cached in `rebuild_peripheral_ranges`
    /// (same staleness contract as `dport_idx`/`rcc_idx`) so
    /// `requires_cycle_accurate` — called per run-loop iteration — never scans
    /// peripherals. `false` on every bus without an H5 op-modeling FLASH.
    flash_models_ops: bool,
    /// True when an IO-Link master peer is attached to any UART on this bus
    /// (see [`SystemBus::has_iolink_master`]). Cached because the underlying
    /// probe is a NESTED scan — every peripheral, `as_any` + downcast to
    /// `Uart`, then every UART's `attached_streams` downcast to `IolinkMaster`
    /// — while `requires_cycle_accurate` consults it per batch plan
    /// (`machine/plan.rs`), per step (`cpu/riscv.rs`) and in the idle
    /// fast-forward check (`lib.rs`). Uncached it cost ~55% of wall on the
    /// shipped ESP32-C3 lab purely to answer "no".
    ///
    /// Staleness contract: recomputed by `rebuild_peripheral_ranges` (which
    /// every peripheral-set mutation funnels through, incl. `add_peripheral`)
    /// AND by `attach_uart_stream_by_id`, the only post-build seam that appends
    /// to a UART's `attached_streams`. Code that mutates the `pub peripherals`
    /// vector or a UART's streams by hand must call `refresh_peripheral_index`
    /// to re-derive it — the same contract already carried by `flash_models_ops`
    /// and `dport_idx`/`rcc_idx`. `bus/tests_main.rs` pins every path.
    iolink_master_attached: bool,
    /// Cached in `rebuild_peripheral_ranges`: true when a Nordic `gpio0`/`gpio1`
    /// port is present, so the per-cycle tick runs the GPIO-edge/GPIOTE service
    /// pass. Lets `tick_peripherals_fully` decide in O(1) whether the walk-free
    /// per-cycle tick has any work at all (see `per_cycle_tick_is_trivial`)
    /// instead of scanning peripherals by name every cycle.
    nordic_gpio_service: bool,
    /// Test/diagnostic override: force the legacy per-cycle HC-SR04 service path
    /// even under the `event-scheduler` feature (disables the scheduled-edge
    /// path). Set only by the differential determinism test; `false` in every
    /// real config so the scheduled path is used whenever it is available.
    pub hcsr04_scheduling_disabled: bool,
    /// Index of the FLASH register peripheral whose opt-in H5 program-error
    /// fidelity gate is enabled, if any. Cached in `rebuild_peripheral_ranges`
    /// (same staleness contract as `rcc_idx`). `None` on every bus where the
    /// gate is off — the common case — so the flash-region write path stays
    /// byte-identical to prior behaviour. When `Some(idx)`, a program (a write
    /// into the flash region) is validated against H5 silicon programming rules
    /// before committing, and `peripherals[idx]` (the `Flash`) records the
    /// resulting NSSR error flags.
    flash_error_flags_idx: Option<usize>,
    /// Universal bus-transaction trace (logic analyzer): a shared, ring-
    /// buffered log that `I2c`/`Spi` peripherals record into once wrapped via
    /// `set_bus_trace` + `attach` (see `crate::bus::bus_trace`). Always
    /// present (never `None`) — empty until at least one peripheral is wired
    /// to it in `from_config`.
    pub bus_trace: bus_trace::BusTraceLog,
    /// Push-mode logic-capture tap (see [`crate::logic_capture`]): the shared
    /// handle instrumented peripherals report pad writes into, and whose
    /// provisional cycle clock the CPU batch loops advance per retired
    /// instruction while push capture is armed. Always present (cheap when
    /// disarmed); wired to peripherals by `Machine::logic_watch`.
    pub logic_tap: crate::logic_capture::LogicTap,
    /// Authoritative pin → (gpio peripheral, bit) map, built from the chip
    /// config's `pins:`. Empty when the chip declares none (→ label parse).
    pub(crate) pin_map: std::collections::HashMap<String, (String, u8)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PeripheralRange {
    start: u64,
    end: u64,
    index: usize,
}

pub struct PeripheralTickCost {
    pub index: usize,
    pub cycles: u32,
}

impl Default for SystemBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "tests_main.rs"]
mod tests;

#[cfg(test)]
#[path = "pin_map_tests.rs"]
mod pin_map_tests;
