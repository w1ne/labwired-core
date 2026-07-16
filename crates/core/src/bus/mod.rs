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
pub mod bus_trace;
mod embedded_descriptors;
mod from_config;
mod routing;
mod tick;

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

    /// Clear batch-local MMIO activity counters (call before each CPU batch).
    #[inline]
    pub fn reset_mmio_activity_counters(&self) {
        self.freerunning_timer_poll_mmio.set(0);
        self.side_effecting_mmio.set(0);
    }

    /// True when the just-finished batch only performed freerunning-timer
    /// polls (no side-effecting MMIO). Consumes and clears the counters.
    /// Chip-specific which regs count as polls — decided by each peripheral.
    #[inline]
    pub fn take_timer_poll_coalesce_eligible(&self) -> bool {
        let timer = self.freerunning_timer_poll_mmio.replace(0);
        let side = self.side_effecting_mmio.replace(0);
        // At least two poll accesses (e.g. OP update + value read).
        timer >= 2 && side == 0
    }

    /// Bookkeep one peripheral MMIO via [`Peripheral::mmio_access_class`]
    /// only — no chip name or register map knowledge on the bus.
    #[inline]
    pub(crate) fn note_mmio_activity(&self, peri_idx: usize, offset: u64) {
        let Some(p) = self.peripherals.get(peri_idx) else {
            return;
        };
        match p.dev.mmio_access_class(offset) {
            crate::MmioAccessClass::FreerunningTimerPoll => {
                self.freerunning_timer_poll_mmio
                    .set(self.freerunning_timer_poll_mmio.get().saturating_add(1));
            }
            crate::MmioAccessClass::SideEffecting => {
                self.side_effecting_mmio
                    .set(self.side_effecting_mmio.get().saturating_add(1));
            }
            crate::MmioAccessClass::SideEffectFree => {}
        }
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

pub struct CanDiagnosticTester {
    pub id: String,
    pub connection: String,
    pub request_id: u32,
    pub request_data: Vec<u8>,
    pub sent: bool,
}

/// Stateful ISO-TP / UDS tester driving a *multi-frame* SecurityAccess exchange
/// against an emulated ECU's CAN controller running in **normal** mode (not
/// loopback). Unlike [`CanDiagnosticTester`] (a one-shot single-frame injector),
/// this is a real second CAN node: it injects a FirstFrame, waits for the ECU's
/// FlowControl, injects the ConsecutiveFrame, then waits for the ECU's
/// SecurityAccess positive response — exactly the handshake a physical UDS
/// tester would perform over ISO 15765-2.
///
/// The ECU side is driven entirely through the peripheral's *public* API: we
/// drain its `tx_frames` (frames it transmitted in normal mode) and inject our
/// frames via `deliver_rx` (bxCAN) / `receive_frame` (FDCAN). Injection is
/// filter-gated, so a `false` return (filter not yet configured, FIFO full)
/// leaves the FSM parked on the same send to retry next tick.
// -----
/// One step in a UDS tester script: a raw payload to send and the
/// expected response bytes (`None` = `..` wildcard, any byte matches).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdsStep {
    /// Raw bytes to send to the ECU (before ISO-TP framing).
    pub send: Vec<u8>,
    /// Expected response bytes; `None` entries match any byte.
    pub expect: Vec<Option<u8>>,
    /// Optional expected NRC byte (response 0x7F <sid> <nrc>).
    pub expect_nrc: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanUdsTesterState {
    /// Need to inject the FirstFrame.
    Start,
    /// FirstFrame sent; waiting for the ECU's FlowControl frame.
    AwaitFc,
    /// ConsecutiveFrame sent; waiting for the ECU's positive response.
    AwaitResp,
    /// Tester sent FlowControl; collecting ECU ConsecutiveFrames until the
    /// declared PDU length is reached (script-driven multi-frame response path).
    AwaitMultiResp,
    /// SecurityAccess positive response observed — handshake complete.
    Done,
    /// Timed out before completion (broken / silent ECU).
    Failed,
}

pub struct CanUdsTester {
    pub id: String,
    /// Name of the connected CAN peripheral (e.g. `bxcan1` / `fdcan1`).
    pub connection: String,
    /// Tester → ECU request id (ISO-TP single physical address). Default 0x111.
    pub request_id: u32,
    /// ECU → tester response id. Default 0x222.
    pub reply_id: u32,
    /// ISO-TP FirstFrame payload injected in state `Start`.
    pub first_frame: Vec<u8>,
    /// ISO-TP ConsecutiveFrame payload injected on FlowControl.
    pub consecutive_frame: Vec<u8>,
    /// Current FSM state. Exposed for tests.
    pub state: CanUdsTesterState,
    /// Ticks elapsed since the tester started; used for the give-up timeout.
    pub ticks: u64,
    /// Tick budget before declaring `Failed`.
    pub max_ticks: u64,
    /// Scripted exchange steps; empty when using legacy hardcoded payloads.
    pub script: Vec<UdsStep>,
    /// Index of the current step in `script`.
    pub step_idx: usize,
    /// Set when a step fails; describes what went wrong.
    pub failure: Option<String>,
    /// PDU accumulator for the script-driven multi-frame ECU response path.
    /// Cleared at the start of each step.
    resp_buf: Vec<u8>,
    /// Declared PDU length from the ECU's FF header (script path only).
    resp_expected_len: usize,
    /// Remaining ConsecutiveFrames to inject for a multi-frame tester request,
    /// populated after the request FF is accepted and the ECU FlowControl is
    /// received (script path only).
    pending_cfs: Vec<Vec<u8>>,
}

impl CanUdsTester {
    /// Default tester ↔ ECU ids and ISO-TP payloads for the SecurityAccess
    /// SeedRequest exchange the firmware contract expects.
    pub const DEFAULT_REQUEST_ID: u32 = 0x111;
    pub const DEFAULT_REPLY_ID: u32 = 0x222;
    pub const DEFAULT_FIRST_FRAME: [u8; 8] = [0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33];
    pub const DEFAULT_CONSECUTIVE_FRAME: [u8; 8] = [0x21, 0x44, 0x55, 0x66, 0x77, 0x88, 0x55, 0x55];
    const DEFAULT_MAX_TICKS: u64 = 200_000;

    pub fn new(id: String, connection: String) -> Self {
        Self {
            id,
            connection,
            request_id: Self::DEFAULT_REQUEST_ID,
            reply_id: Self::DEFAULT_REPLY_ID,
            first_frame: Self::DEFAULT_FIRST_FRAME.to_vec(),
            consecutive_frame: Self::DEFAULT_CONSECUTIVE_FRAME.to_vec(),
            state: CanUdsTesterState::Start,
            ticks: 0,
            max_ticks: Self::DEFAULT_MAX_TICKS,
            script: Vec::new(),
            step_idx: 0,
            failure: None,
            resp_buf: Vec::new(),
            resp_expected_len: 0,
            pending_cfs: Vec::new(),
        }
    }

    /// Build the ISO-TP request frame(s) for `script[step_idx]`.
    /// Single-frame when `send.len() <= 7`; otherwise a FirstFrame followed by
    /// ConsecutiveFrames. The caller sends the first frame and queues the rest
    /// in `pending_cfs` after FlowControl.
    fn build_request_frames(&self) -> Vec<Vec<u8>> {
        let Some(step) = self.script.get(self.step_idx) else {
            return Vec::new();
        };
        let data = &step.send;
        let len = data.len();
        if len <= 7 {
            // Single-frame: [len, payload...]
            let mut frame = Vec::with_capacity(len + 1);
            frame.push(len as u8);
            frame.extend_from_slice(data);
            return vec![frame];
        }
        // Multi-frame: FF then CFs.
        let mut frames = Vec::new();
        // FirstFrame: [0x10 | (len>>8), len & 0xFF, first 6 bytes]
        let mut ff = Vec::with_capacity(8);
        ff.push(0x10 | ((len >> 8) as u8));
        ff.push((len & 0xFF) as u8);
        ff.extend_from_slice(&data[..6.min(len)]);
        frames.push(ff);
        // ConsecutiveFrames
        let mut seq: u8 = 1;
        let mut offset = 6;
        while offset < len {
            let end = (offset + 7).min(len);
            let mut cf = Vec::with_capacity(8);
            cf.push(0x20 | (seq & 0x0F));
            cf.extend_from_slice(&data[offset..end]);
            frames.push(cf);
            seq = seq.wrapping_add(1);
            offset = end;
        }
        frames
    }

    /// Return `true` when `resp` satisfies the match criteria of `step`.
    /// If `step.expect_nrc` is `Some(nrc)`, matches `[0x7F, send[0], nrc]`.
    /// Otherwise compares against `step.expect` element-wise (`None` = any byte),
    /// allowing `resp` to be longer than the pattern (prefix match).
    fn matches(resp: &[u8], step: &UdsStep) -> bool {
        if let Some(nrc) = step.expect_nrc {
            return resp == [0x7F, step.send.first().copied().unwrap_or(0), nrc];
        }
        let pattern = &step.expect;
        if resp.len() < pattern.len() {
            return false;
        }
        pattern
            .iter()
            .zip(resp.iter())
            .all(|(p, b)| p.is_none_or(|expected| expected == *b))
    }

    /// Observe one ECU frame in the **script-driven** path. Returns the payload
    /// to inject next (FlowControl or first pending CF), or `None`. Sets
    /// `state = Done / Failed` when the exchange concludes.
    fn observe_ecu_frame_script(&mut self, id: u32, data: &[u8]) -> Option<Vec<u8>> {
        if id != self.reply_id {
            return None;
        }
        match self.state {
            CanUdsTesterState::AwaitFc => {
                if data.first().map(|b| b & 0xF0) == Some(0x30) {
                    // FlowControl received: signal the next CF to inject.
                    // Do NOT change state here — the injected block in
                    // service_can_uds_testers advances AwaitFc→AwaitResp only
                    // after the last CF has been successfully accepted, draining
                    // pending_cfs one entry per tick.
                    return self.pending_cfs.first().cloned();
                }
                None
            }
            CanUdsTesterState::AwaitResp => {
                let ptype = data.first().map(|b| b & 0xF0).unwrap_or(0xFF);
                if ptype == 0x00 {
                    // ECU SingleFrame response. Two ISO-TP SF encodings:
                    //   * classic: byte0 = 0x0L, length L (1..=7) in the low
                    //     nibble, payload from byte 1.
                    //   * CAN-FD escape: byte0 = 0x00, real length in byte 1,
                    //     payload from byte 2 (used for SF up to 62 bytes on FD
                    //     frames — the ECU runs ISO-TP in FD mode).
                    let b0 = data.first().copied().unwrap_or(0);
                    let (pdu_len, data_off) = if b0 == 0x00 {
                        // CAN-FD escape SF: a length byte must follow.
                        match data.get(1) {
                            Some(&len) => (len as usize, 2),
                            None => {
                                self.failure = Some(format!(
                                    "step {}: malformed FD escape SingleFrame (no length byte)",
                                    self.step_idx
                                ));
                                self.state = CanUdsTesterState::Failed;
                                return None;
                            }
                        }
                    } else {
                        ((b0 & 0x0F) as usize, 1)
                    };
                    // The frame must actually carry the declared payload bytes; a
                    // short/truncated SF is a protocol error, not an empty match.
                    if data.len() < data_off + pdu_len {
                        self.failure = Some(format!(
                            "step {}: truncated SingleFrame (declared {} payload bytes, frame carries {})",
                            self.step_idx,
                            pdu_len,
                            data.len().saturating_sub(data_off)
                        ));
                        self.state = CanUdsTesterState::Failed;
                        return None;
                    }
                    let payload: Vec<u8> = data[data_off..data_off + pdu_len].to_vec();
                    self.complete_response(payload);
                } else if ptype == 0x10 {
                    // ECU FirstFrame: start reassembly, send FlowControl.
                    let declared = if data.len() >= 2 {
                        (((data[0] & 0x0F) as usize) << 8) | (data[1] as usize)
                    } else {
                        0
                    };
                    self.resp_expected_len = declared;
                    self.resp_buf.clear();
                    if data.len() > 2 {
                        self.resp_buf.extend_from_slice(&data[2..]);
                    }
                    self.state = CanUdsTesterState::AwaitMultiResp;
                    // FlowControl: ContinueToSend, block size 0, ST 0.
                    return Some(vec![0x30, 0x00, 0x00]);
                }
                None
            }
            CanUdsTesterState::AwaitMultiResp => {
                if data.first().map(|b| b & 0xF0) == Some(0x20) {
                    self.resp_buf
                        .extend_from_slice(data.get(1..).unwrap_or(&[]));
                    if self.resp_buf.len() >= self.resp_expected_len {
                        let payload = self.resp_buf[..self.resp_expected_len].to_vec();
                        self.resp_buf.clear();
                        self.complete_response(payload);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Called when a complete PDU has been reassembled. Matches against the
    /// current step and either advances to the next step (or `Done`) or sets
    /// `Failed`.
    fn complete_response(&mut self, payload: Vec<u8>) {
        let Some(step) = self.script.get(self.step_idx) else {
            self.state = CanUdsTesterState::Done;
            return;
        };
        if Self::matches(&payload, step) {
            self.step_idx += 1;
            self.resp_buf.clear();
            self.resp_expected_len = 0;
            if self.step_idx >= self.script.len() {
                self.state = CanUdsTesterState::Done;
            } else {
                // More steps: the driver will send the next request next tick.
                self.state = CanUdsTesterState::Start;
            }
        } else {
            let msg = if let Some(nrc) = step.expect_nrc {
                format!(
                    "step {}: expected NRC 7F {:02X} {:02X}, got {:02X?}",
                    self.step_idx,
                    step.send.first().copied().unwrap_or(0),
                    nrc,
                    payload
                )
            } else {
                format!(
                    "step {}: expected {:02X?}, got {:02X?}",
                    self.step_idx, step.expect, payload
                )
            };
            self.failure = Some(msg);
            self.state = CanUdsTesterState::Failed;
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            CanUdsTesterState::Done | CanUdsTesterState::Failed
        )
    }

    /// Observe one frame the ECU transmitted. Legacy path (empty `script`):
    /// In `AwaitFc` an ISO-TP FlowControl (`(data[0] & 0xF0) == 0x30`) on
    /// `reply_id` returns the ConsecutiveFrame payload to inject; in `AwaitResp`
    /// a SecurityAccess single-frame positive response (`data[0] == 0x06 &&
    /// data[1] == 0x67`) completes the handshake. Returns the payload to inject
    /// next, else `None`.
    ///
    /// When `script` is non-empty, delegates to `observe_ecu_frame_script`
    /// instead so the script-driven logic handles framing and matching.
    fn observe_ecu_frame(&mut self, id: u32, data: &[u8]) -> Option<Vec<u8>> {
        if !self.script.is_empty() {
            return self.observe_ecu_frame_script(id, data);
        }
        if id != self.reply_id {
            return None;
        }
        match self.state {
            CanUdsTesterState::AwaitFc => {
                if data.first().map(|b| b & 0xF0) == Some(0x30) {
                    // FlowControl seen → time to send the ConsecutiveFrame.
                    return Some(self.consecutive_frame.clone());
                }
                None
            }
            CanUdsTesterState::AwaitResp => {
                if data.first() == Some(&0x06) && data.get(1) == Some(&0x67) {
                    self.state = CanUdsTesterState::Done;
                }
                None
            }
            _ => None,
        }
    }
}

/// Deterministic CAN log replay node: an external bus participant that
/// delivers pre-parsed frames into a bxCAN/FDCAN peripheral at scheduled
/// tick offsets. Vendor-neutral by design — candump input only; vendor log
/// formats convert outside core (2026-07-02 replay-showcase spec).
pub struct CanLogPlayer {
    pub id: String,
    /// Name of the connected CAN peripheral (e.g. `bxcan1` / `fdcan1`).
    pub connection: String,
    /// (due_tick, frame), ascending; first frame rebased to tick 0.
    pub frames: Vec<(u64, crate::network::CanFrame)>,
    pub next_idx: usize,
    pub ticks: u64,
    /// Frames accepted by the peripheral.
    pub delivered: u64,
    /// Frames refused (filters closed / FIFO full / CAN not initialized).
    pub dropped: u64,
}

impl CanLogPlayer {
    pub fn from_candump(
        id: String,
        connection: String,
        text: &str,
        ticks_per_second: u64,
    ) -> Result<Self, String> {
        let parsed = crate::network::candump::parse_candump(text)?;
        if parsed.is_empty() {
            return Err(format!("can-player '{id}': log contains no frames"));
        }
        let t0 = parsed[0].0;
        let mut frames: Vec<(u64, crate::network::CanFrame)> = parsed
            .into_iter()
            .map(|(t, f)| {
                (
                    ((t - t0).max(0.0) * ticks_per_second as f64).round() as u64,
                    f,
                )
            })
            .collect();
        frames.sort_by_key(|(t, _)| *t);
        Ok(Self {
            id,
            connection,
            frames,
            next_idx: 0,
            ticks: 0,
            delivered: 0,
            dropped: 0,
        })
    }

    pub fn is_done(&self) -> bool {
        self.next_idx >= self.frames.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PeripheralRange {
    start: u64,
    end: u64,
    index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeripheralTickCost {
    pub index: usize,
    pub cycles: u32,
}

impl Default for SystemBus {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemBus {
    /// Walk every attached device that accepts simulated input, in bus order,
    /// calling `f(owner_name, device)` for each. `owner_name` is the owning
    /// peripheral's bus name for transport-attached devices (I²C / SPI / UART
    /// stream) and the sensor `id` for devices that live directly on the bus
    /// (HC-SR04). Stops early when `f` returns `true`.
    ///
    /// This is the ONE walk behind `list_inputs` / `set_input`, so a new
    /// transport (or directly-attached input device) added here is picked up
    /// by discovery, dispatch, the manifest schema consumers, and every
    /// external surface (test-script stimuli, MCP, WASM) at once.
    fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&str, &mut dyn crate::sim_input::SimInput) -> bool,
    ) {
        for entry in self.peripherals.iter_mut() {
            let Some(any) = entry.dev.as_any_mut() else {
                continue;
            };
            if let Some(i2c) = any.downcast_ref::<crate::peripherals::i2c::I2c>() {
                for cell in i2c.attached_devices() {
                    let mut dev = cell.borrow_mut();
                    if let Some(si) = dev.as_sim_input_mut() {
                        if f(&entry.name, si) {
                            return;
                        }
                    }
                }
            } else if let Some(spi) = any.downcast_mut::<crate::peripherals::spi::Spi>() {
                for dev in spi.attached_devices.iter_mut() {
                    if let Some(si) = dev.as_sim_input_mut() {
                        if f(&entry.name, si) {
                            return;
                        }
                    }
                }
            } else if let Some(uart) = any.downcast_mut::<crate::peripherals::uart::Uart>() {
                for stream in uart.attached_streams.iter_mut() {
                    if let Some(si) = stream.as_sim_input_mut() {
                        if f(&entry.name, si) {
                            return;
                        }
                    }
                }
            }
        }
        for sensor in self.hcsr04.iter_mut() {
            let id = sensor.id.clone();
            if f(&id, sensor) {
                return;
            }
        }
    }

    /// Whether one walked device answers to `component`: its stamped
    /// system.yaml id first (the name an author writes), falling back to the
    /// owning peripheral's bus name.
    fn component_matches(
        component: Option<&str>,
        owner: &str,
        si: &dyn crate::sim_input::SimInput,
    ) -> bool {
        component.is_none_or(|c| si.component_id() == Some(c) || c == owner)
    }

    /// Discover every drivable input channel across attached devices, tagged
    /// with the owning component — the "what can an agent drive?" query
    /// behind `labwired_list_inputs` / the browser panel / the MCP stimulus
    /// surface. The owner is the device's system.yaml `external_devices` id
    /// when stamped (so discovery speaks the SAME vocabulary `set_input`'s
    /// `component` accepts), falling back to the peripheral's bus name.
    pub fn list_inputs(&mut self) -> Vec<(String, crate::sim_input::InputChannel)> {
        let mut out = Vec::new();
        self.for_each_sim_input(&mut |name, si| {
            let owner = si.component_id().unwrap_or(name).to_string();
            for ch in si.input_channels() {
                out.push((owner.clone(), *ch));
            }
            false
        });
        out
    }

    /// Drive `channel` to `value` (in the channel's engineering unit) on the
    /// unique attached input device that exposes it. Generic over device type
    /// via [`crate::sim_input::SimInput`] — no per-type dispatch.
    ///
    /// `component`, when given, narrows resolution to the named device — the
    /// disambiguator when two devices expose the same channel key (e.g. two
    /// accelerometers on one bus, or `distance` on both a VL53L1X and an
    /// HC-SR04). It matches the external-device id from system.yaml
    /// (`fxos8700`, stamped onto the model at attach) or the owning
    /// peripheral's bus name (`i2c1`). Errors if no device (or more than one,
    /// after narrowing) exposes the channel, or the value is out of range.
    pub fn set_input(
        &mut self,
        component: Option<&str>,
        channel: &str,
        value: f64,
    ) -> Result<(), crate::sim_input::SimInputError> {
        use crate::sim_input::SimInputError;
        // Count matches first so ambiguity is a typed error, not a silent
        // "first wins".
        let mut matches = 0usize;
        self.for_each_sim_input(&mut |name, si| {
            if Self::component_matches(component, name, si)
                && si.input_channels().iter().any(|c| c.key == channel)
            {
                matches += 1;
            }
            false
        });
        if matches == 0 {
            let missing = match component {
                Some(c) => format!("{c}/{channel}"),
                None => channel.to_string(),
            };
            return Err(SimInputError::NoDevice(missing));
        }
        if matches > 1 {
            return Err(SimInputError::Ambiguous {
                channel: channel.to_string(),
                matches,
            });
        }
        let mut result = Ok(());
        self.for_each_sim_input(&mut |name, si| {
            if Self::component_matches(component, name, si)
                && si.input_channels().iter().any(|c| c.key == channel)
            {
                result = si.set_input(channel, value);
                true
            } else {
                false
            }
        });
        result
    }

    /// Apply several input sets as ONE transaction: every set is resolved and
    /// range-checked first, and only if ALL are valid are any applied — so a
    /// multi-channel pose (an accelerometer's x/y/z, a GPS lat+lon) can never
    /// be half-applied, and the firmware can never observe a torn update
    /// (nothing steps between the applications). All-or-nothing: the first
    /// error aborts the whole batch with nothing written.
    pub fn set_inputs(
        &mut self,
        sets: &[(Option<&str>, &str, f64)],
    ) -> Result<(), crate::sim_input::SimInputError> {
        use crate::sim_input::SimInputError;
        // Validate pass: unique resolution + range for every set.
        for &(component, channel, value) in sets {
            let mut matches = 0usize;
            let mut range: Result<(), SimInputError> = Ok(());
            self.for_each_sim_input(&mut |name, si| {
                if Self::component_matches(component, name, si)
                    && si.input_channels().iter().any(|c| c.key == channel)
                {
                    matches += 1;
                    range = si.require_channel(channel, value).map(|_| ());
                }
                false
            });
            if matches == 0 {
                let missing = match component {
                    Some(c) => format!("{c}/{channel}"),
                    None => channel.to_string(),
                };
                return Err(SimInputError::NoDevice(missing));
            }
            if matches > 1 {
                return Err(SimInputError::Ambiguous {
                    channel: channel.to_string(),
                    matches,
                });
            }
            range?;
        }
        // Apply pass — validated above, so failures here can't strand a
        // partial batch short of a device error, which set_input surfaces.
        for &(component, channel, value) in sets {
            self.set_input(component, channel, value)?;
        }
        Ok(())
    }

    /// Snapshot of the universal bus-transaction trace (logic analyzer):
    /// every I²C/SPI byte recorded so far by peripherals wired to
    /// `self.bus_trace` (see `crate::bus::bus_trace`), oldest first.
    pub fn bus_trace_snapshot(&self) -> Vec<bus_trace::BusTraceEvent> {
        self.bus_trace.snapshot()
    }

    /// Attach an I²C slave without a physical route. This remains suitable for
    /// fixed-pin controllers and low-level test fixtures; ESP32-C3 rejects it
    /// because C3's GPIO matrix makes a controller-only binding ambiguous.
    pub fn attach_i2c_slave(
        &mut self,
        controller: &str,
        dev: Box<dyn crate::peripherals::i2c::I2cDevice>,
    ) -> anyhow::Result<()> {
        self.attach_i2c_slave_with_route(controller, dev, None)
    }

    /// The single funnel through which every manifest-backed I²C slave reaches
    /// a controller. `route` is a target-neutral signal map (`sda`/`scl` for
    /// I²C); ESP32-C3 lowers it to real GPIO-matrix pads and rejects missing,
    /// unsupported, or ambiguous routes instead of silently attaching by bus
    /// name alone. Other controller families preserve the generic shape for
    /// forward-compatible physical routing while retaining their fixed-pin
    /// behavior today.
    pub fn attach_i2c_slave_with_route(
        &mut self,
        controller: &str,
        dev: Box<dyn crate::peripherals::i2c::I2cDevice>,
        route: Option<&std::collections::BTreeMap<String, String>>,
    ) -> anyhow::Result<()> {
        let wrapped = bus_trace::wrap_i2c(controller, &self.bus_trace, dev);
        let idx = self
            .find_peripheral_index_by_name(controller)
            .ok_or_else(|| anyhow::anyhow!("attach_i2c_slave: no peripheral '{controller}'"))?;
        let any = self.peripherals[idx].dev.as_any_mut().ok_or_else(|| {
            anyhow::anyhow!("attach_i2c_slave: '{controller}' is not downcastable")
        })?;
        if let Some(c) = any.downcast_mut::<crate::peripherals::i2c::I2c>() {
            c.push_slave(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>() {
            let route = route.ok_or_else(|| {
                anyhow::anyhow!(
                    "ESP32-C3 I2C external device on '{controller}' requires both route.sda and route.scl"
                )
            })?;
            let route =
                crate::peripherals::esp32c3::i2c::C3I2cPadRoute::from_manifest_route(route)?;
            c.push_slave_with_route(wrapped, route);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32s3::i2c::Esp32s3I2c>() {
            c.push_slave(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32::i2c::Esp32I2c>() {
            c.push_slave(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::nrf52::twim::Nrf52Twim>() {
            c.push_slave(wrapped);
        } else {
            anyhow::bail!("attach_i2c_slave: '{controller}' is not an I2C controller");
        }
        Ok(())
    }

    /// Wire the ESP32-C3 I²C0 bit engine to C3 GPIO in both directions: GPIO
    /// reads the live SDA/SCL waveform, while I²C reads GPIO's live input/output
    /// matrix state before allowing a physically routed slave to acknowledge.
    /// No-op unless both C3 models are on the bus.
    pub(crate) fn wire_esp32c3_i2c_pads(&mut self) {
        use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
        use crate::peripherals::esp32c3::i2c::Esp32c3I2c;
        let i2c_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|a| a.is::<Esp32c3I2c>())
                .unwrap_or(false)
        });
        let gpio_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|a| a.is::<Esp32c3Gpio>())
                .unwrap_or(false)
        });
        let (Some(i2c_idx), Some(gpio_idx)) = (i2c_idx, gpio_idx) else {
            return;
        };
        let matrix_route = self.peripherals[gpio_idx]
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<Esp32c3Gpio>())
            .map(|g| g.i2c_matrix_route_state());
        let lines = self.peripherals[i2c_idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Esp32c3I2c>())
            .and_then(|c| {
                matrix_route.map(|route| {
                    c.set_matrix_route_state(route);
                    c.line_levels_arc()
                })
            });
        if let (Some(lines), Some(gpio)) = (
            lines,
            self.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Esp32c3Gpio>()),
        ) {
            gpio.set_i2c_lines(lines);
        }
    }

    /// Wire C3 IO_MUX per-pad controls into C3 GPIO after both models have
    /// been constructed. The IO_MUX owns the shared register bank; GPIO reads
    /// `FUN_WPU` from it to model Arduino `INPUT_PULLUP`. No-op on any bus
    /// without both C3 peripherals.
    pub(crate) fn wire_esp32c3_pad_controls(&mut self) {
        use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
        use crate::peripherals::esp32c3::io_mux::Esp32c3IoMux;

        let io_mux_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|any| any.is::<Esp32c3IoMux>())
                .unwrap_or(false)
        });
        let gpio_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|any| any.is::<Esp32c3Gpio>())
                .unwrap_or(false)
        });
        let (Some(io_mux_idx), Some(gpio_idx)) = (io_mux_idx, gpio_idx) else {
            return;
        };

        let controls = self.peripherals[io_mux_idx]
            .dev
            .as_any()
            .and_then(|any| any.downcast_ref::<Esp32c3IoMux>())
            .map(Esp32c3IoMux::pad_controls);
        if let (Some(controls), Some(gpio)) = (
            controls,
            self.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<Esp32c3Gpio>()),
        ) {
            gpio.set_pad_controls(controls);
        }
    }

    /// Bracket a C3 IO_MUX write with GPIO push-capture sampling. A `FUN_WPU`
    /// write changes an input pad electrically even though the GPIO register
    /// block itself is not written, so the usual GPIO-local write hooks would
    /// otherwise miss the edge. The returned GPIO index is passed to
    /// [`Self::finish_esp32c3_io_mux_write`] after the MMIO write succeeds.
    pub(crate) fn begin_esp32c3_io_mux_write(&mut self, io_mux_idx: usize) -> Option<usize> {
        use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
        use crate::peripherals::esp32c3::io_mux::Esp32c3IoMux;

        if !self.peripherals.get(io_mux_idx).is_some_and(|p| {
            p.dev
                .as_any()
                .map(|any| any.is::<Esp32c3IoMux>())
                .unwrap_or(false)
        }) {
            return None;
        }
        let gpio_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .map(|any| any.is::<Esp32c3Gpio>())
                .unwrap_or(false)
        })?;
        self.peripherals[gpio_idx]
            .dev
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<Esp32c3Gpio>())?
            .tap_snapshot();
        Some(gpio_idx)
    }

    /// Complete a successful C3 IO_MUX write started by
    /// [`Self::begin_esp32c3_io_mux_write`], pushing any changed pad level to
    /// the in-engine logic tap.
    pub(crate) fn finish_esp32c3_io_mux_write(&mut self, gpio_idx: Option<usize>) {
        let Some(gpio_idx) = gpio_idx else {
            return;
        };
        if let Some(gpio) = self.peripherals[gpio_idx]
            .dev
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<crate::peripherals::esp32c3::gpio::Esp32c3Gpio>())
        {
            gpio.tap_report();
        }
    }

    /// Wire the STM32 SPI bit engines' live SCK/MOSI/MISO levels into the
    /// STM32 GPIO ports, so pads whose MODER/AFR (V2) or CRL/CRH CNF (F1)
    /// route an SPI alternate function read the real waveform through
    /// `read_gpio_pad` (which is what the in-engine logic analyzer samples).
    /// The SPI counterpart of [`Self::wire_esp32c3_i2c_pads`]; no-op on buses
    /// without a classic/FIFO STM32 SPI.
    ///
    /// Signal mapping comes from static per-family AF tables sourced from the
    /// datasheet alternate-function maps:
    /// * L4 (FIFO SPI + V2 GPIO): STM32L476 datasheet DS10198 Table 17 —
    ///   SPI1/SPI2 on AF5, SPI3 on AF6.
    /// * F4 (classic SPI + V2 GPIO): STM32F407 datasheet DS8626 Table 9 —
    ///   SPI1/SPI2 on AF5.
    /// * F1 (classic SPI + F1 GPIO): RM0008 §9.3 default pinout, no AFIO
    ///   remap (remap is not modeled). F1 MISO pads are input-mode on real
    ///   silicon and are intentionally not routed (see `GpioPort` docs).
    pub(crate) fn wire_stm32_spi_pads(&mut self) {
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::peripherals::spi::{Spi, SpiSignal};
        use SpiSignal::{Miso, Mosi, Sck};

        // (spi, port, pin, AF, signal, func) — V2 ports, L4 parts (DS10198
        // Table 17: SPI1-3).
        const L4: &[(&str, char, u8, u8, SpiSignal, &str)] = &[
            ("spi1", 'a', 5, 5, Sck, "SPI1_SCK"),
            ("spi1", 'a', 6, 5, Miso, "SPI1_MISO"),
            ("spi1", 'a', 7, 5, Mosi, "SPI1_MOSI"),
            ("spi1", 'b', 3, 5, Sck, "SPI1_SCK"),
            ("spi1", 'b', 4, 5, Miso, "SPI1_MISO"),
            ("spi1", 'b', 5, 5, Mosi, "SPI1_MOSI"),
            ("spi1", 'e', 13, 5, Sck, "SPI1_SCK"),
            ("spi1", 'e', 14, 5, Miso, "SPI1_MISO"),
            ("spi1", 'e', 15, 5, Mosi, "SPI1_MOSI"),
            ("spi2", 'b', 10, 5, Sck, "SPI2_SCK"),
            ("spi2", 'b', 13, 5, Sck, "SPI2_SCK"),
            ("spi2", 'b', 14, 5, Miso, "SPI2_MISO"),
            ("spi2", 'b', 15, 5, Mosi, "SPI2_MOSI"),
            ("spi2", 'c', 2, 5, Miso, "SPI2_MISO"),
            ("spi2", 'c', 3, 5, Mosi, "SPI2_MOSI"),
            ("spi2", 'd', 1, 5, Sck, "SPI2_SCK"),
            ("spi2", 'd', 3, 5, Miso, "SPI2_MISO"),
            ("spi2", 'd', 4, 5, Mosi, "SPI2_MOSI"),
            ("spi3", 'b', 3, 6, Sck, "SPI3_SCK"),
            ("spi3", 'b', 4, 6, Miso, "SPI3_MISO"),
            ("spi3", 'b', 5, 6, Mosi, "SPI3_MOSI"),
            ("spi3", 'c', 10, 6, Sck, "SPI3_SCK"),
            ("spi3", 'c', 11, 6, Miso, "SPI3_MISO"),
            ("spi3", 'c', 12, 6, Mosi, "SPI3_MOSI"),
        ];
        // V2 ports, F4 parts (DS8626 Table 9: SPI1-2).
        const F4: &[(&str, char, u8, u8, SpiSignal, &str)] = &[
            ("spi1", 'a', 5, 5, Sck, "SPI1_SCK"),
            ("spi1", 'a', 6, 5, Miso, "SPI1_MISO"),
            ("spi1", 'a', 7, 5, Mosi, "SPI1_MOSI"),
            ("spi1", 'b', 3, 5, Sck, "SPI1_SCK"),
            ("spi1", 'b', 4, 5, Miso, "SPI1_MISO"),
            ("spi1", 'b', 5, 5, Mosi, "SPI1_MOSI"),
            ("spi2", 'b', 10, 5, Sck, "SPI2_SCK"),
            ("spi2", 'b', 13, 5, Sck, "SPI2_SCK"),
            ("spi2", 'b', 14, 5, Miso, "SPI2_MISO"),
            ("spi2", 'b', 15, 5, Mosi, "SPI2_MOSI"),
            ("spi2", 'c', 2, 5, Miso, "SPI2_MISO"),
            ("spi2", 'c', 3, 5, Mosi, "SPI2_MOSI"),
        ];
        // F1 ports (RM0008 §9.3 default mapping, SPI1-2, SCK/MOSI only).
        const F1: &[(&str, char, u8, SpiSignal, &str)] = &[
            ("spi1", 'a', 5, Sck, "SPI1_SCK"),
            ("spi1", 'a', 7, Mosi, "SPI1_MOSI"),
            ("spi2", 'b', 13, Sck, "SPI2_SCK"),
            ("spi2", 'b', 15, Mosi, "SPI2_MOSI"),
        ];

        for spi_name in ["spi1", "spi2", "spi3"] {
            let Some(spi_idx) = self.find_peripheral_index_by_name(spi_name) else {
                continue;
            };
            let Some((fifo, lines)) = self.peripherals[spi_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Spi>())
                .filter(|s| s.is_stm32_wire_layout())
                .map(|s| (s.is_fifo_layout(), s.line_levels_arc()))
            else {
                continue;
            };
            for port in ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'] {
                let Some(gpio_idx) = self.find_peripheral_index_by_name(&format!("gpio{port}"))
                else {
                    continue;
                };
                let Some(gpio) = self.peripherals[gpio_idx]
                    .dev
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<GpioPort>())
                else {
                    continue;
                };
                match gpio.register_layout() {
                    GpioRegisterLayout::Stm32V2 => {
                        let table = if fifo { L4 } else { F4 };
                        for &(spi, p, pin, af, sig, func) in table {
                            if spi == spi_name && p == port {
                                gpio.add_spi_pad_route(&lines, pin, Some(af), sig, func);
                            }
                        }
                    }
                    GpioRegisterLayout::Stm32F1 => {
                        for &(spi, p, pin, sig, func) in F1 {
                            if spi == spi_name && p == port {
                                gpio.add_spi_pad_route(&lines, pin, None, sig, func);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// The single funnel through which every SPI device reaches a controller —
    /// the SPI counterpart of [`Self::attach_i2c_slave`]. Wraps then dispatches.
    pub fn attach_spi_device(
        &mut self,
        controller: &str,
        dev: Box<dyn crate::peripherals::spi::SpiDevice>,
    ) -> anyhow::Result<()> {
        let wrapped = bus_trace::wrap_spi(controller, &self.bus_trace, dev);
        let idx = self
            .find_peripheral_index_by_name(controller)
            .ok_or_else(|| anyhow::anyhow!("attach_spi_device: no peripheral '{controller}'"))?;
        let any = self.peripherals[idx].dev.as_any_mut().ok_or_else(|| {
            anyhow::anyhow!("attach_spi_device: '{controller}' is not downcastable")
        })?;
        if let Some(c) = any.downcast_mut::<crate::peripherals::spi::Spi>() {
            c.push_device(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32c3::spi::Esp32c3Spi>() {
            c.push_device(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32::spi::Esp32Spi>() {
            c.push_device(wrapped);
        } else if let Some(c) = any.downcast_mut::<crate::peripherals::esp32s3::gpspi::Esp32s3Spi>()
        {
            c.push_device(wrapped);
        } else {
            anyhow::bail!("attach_spi_device: '{controller}' is not a SPI controller");
        }
        Ok(())
    }

    pub(crate) fn canonical_peripheral_type(raw_type: &str) -> String {
        let t = raw_type.to_ascii_lowercase();

        // 1. If the name is ALREADY a canonical model type, return it verbatim.
        //    This is the single source of truth (co-located with the factory in
        //    `generic_factory::MODEL_TYPES`) and replaces both the old core-type
        //    match and the per-name identity pre-emption blocks. It guarantees a
        //    real model type (e.g. `esp32c3_spi`, `nrf52840_twim`, `rp2040_timer`)
        //    is never coerced by the legacy fuzzy heuristics below.
        if crate::peripherals::generic_factory::is_canonical_model_type(&t) {
            return t;
        }

        // 2. Alias table: raw INPUT spellings whose canonical OUTPUT differs from
        //    the input. These are NOT identities (the verbatim case is handled by
        //    membership above), so they must not appear in `MODEL_TYPES`. Mostly
        //    nRF52 vendor synonyms (`nrf52840_i2c` → the TWIM master model, …)
        //    that must resolve before the fuzzy `contains(...)` chain, otherwise
        //    e.g. `nrf52840_saadc` (contains "adc") or `nrf52840_qspi`
        //    (contains "spi") would be coerced onto STM32 layouts. Iterated in
        //    order; first matching group wins.
        const ALIASES: &[(&[&str], &str)] = &[
            // SAADC: nRF52 SAR ADC (vendor "adc"/"saadc" spellings).
            (
                &["nrf52840_saadc", "nrf52_saadc", "nrf52840_adc"],
                "nrf52_saadc",
            ),
            // QSPI: nRF52 external-flash quad-SPI controller.
            (&["nrf52840_qspi", "nrf52_qspi"], "nrf52_qspi"),
            // SPIS / TWIS: SPI / I²C slave with EasyDMA.
            (&["nrf52840_spis", "nrf52_spis"], "nrf52840_spis"),
            (&["nrf52840_twis", "nrf52_twis"], "nrf52840_twis"),
            // TWIM / TWI master: nRF52 I²C master with EasyDMA.
            (
                &["nrf52840_i2c", "nrf52840_twim", "nrf52_twim", "nrf52_i2c"],
                "nrf52840_twim",
            ),
            // UARTE: nRF52 UART with EasyDMA (PSEL/BAUDRATE/CONFIG).
            (
                &["nrf52840_uart", "nrf52_uart", "nrf52_uarte"],
                "nrf52840_uart",
            ),
            // GPIOTE: Nordic GPIO task/event controller (shares "gpio" in name
            // but a totally different register surface).
            (
                &["nrf52840_gpiotasksevents", "nrf52_gpiote"],
                "nrf52_gpiote",
            ),
        ];
        for (inputs, canonical) in ALIASES {
            if inputs.contains(&t.as_str()) {
                return canonical.to_string();
            }
        }

        // Serial-instance mux (SPIM0/TWIM0 share one MMIO window) — must
        // precede the generic "contains(spi)" and "contains(i2c)" matchers.
        if t == "nrf52840_serial"
            || t == "nrf52_serial"
            || t == "nrf52_spim_twim"
            || t == "nrf52840_spim_twim"
        {
            return "nrf52_serial_instance".to_string();
        }

        // 3. Legacy generic SVD-name heuristics (fallback). Fuzzy `contains` /
        //    `starts_with` / `ends_with` matching for raw vendor names we have
        //    not given an explicit canonical type. Ordering matters: specific
        //    mappers come before broader ones so e.g.
        // "quadspi" doesn't get swallowed by the generic "contains(spi)" rule.
        if t.contains("quadspi") || t == "qspi" {
            return "quadspi".to_string();
        }
        if t.contains("lptim") || t == "low_power_timer" {
            return "lptim".to_string();
        }
        if t == "sai" || t.starts_with("sai_") || t.contains("audio") {
            return "sai".to_string();
        }
        if t.contains("otg") || t == "usb_fs" || t == "usb_otg_fs" {
            return "usb_otg".to_string();
        }
        if t == "bxcan" || t == "stm32_can" {
            return "bxcan".to_string();
        }
        if t == "sdmmc" || t == "sdio" || t.starts_with("sdmmc_") {
            return "sdmmc".to_string();
        }
        if t == "comp" || t == "comparator" || t.starts_with("comp_") {
            return "comp".to_string();
        }
        if t == "tsc" || t == "touchsense" {
            return "tsc".to_string();
        }
        if t == "fmc" || t == "fsmc" || t == "memorycontroller" {
            return "fmc".to_string();
        }

        if t.contains("uart") || t.contains("usart") || t == "leuart" || t.ends_with("_sci") {
            return "uart".to_string();
        }
        if t == "sam4s_pio" || (t.contains("gpio") && t != "pio") {
            return "gpio".to_string();
        }
        if t.contains("i2c") || t.contains("iic") || t.contains("smbus") || t.ends_with("_twi") {
            return "i2c".to_string();
        }
        if t.contains("spi") {
            return "spi".to_string();
        }
        if t == "udma" || t.contains("dma") {
            return "dma".to_string();
        }
        // Nordic CLOCK shares its name with the generic "rcc" bin in the
        // canonicalize, but its register layout is Nordic-specific and it
        // is unioned with the POWER peripheral at the same base address.
        // Route it to the dedicated nRF52 model.
        if t == "nrf_clock" || t == "nrf52_clock" || t == "nrf52840_clock" {
            return "nrf52_clock".to_string();
        }
        if t.contains("rcc") || t.contains("cmu") {
            return "rcc".to_string();
        }
        if t == "arm_generictimer" || t == "arm_globaltimer" || t == "arm_sp804_timer" {
            return "systick".to_string();
        }
        if t.contains("timer") || t.ends_with("_gpt") || t.ends_with("_agt") {
            return "timer".to_string();
        }
        if t.contains("adc") {
            return "adc".to_string();
        }

        t
    }

    pub(crate) fn profile_name(p_cfg: &PeripheralConfig) -> anyhow::Result<Option<&str>> {
        if let Some(value) = p_cfg.config.get("profile") {
            return value.as_str().map(Some).ok_or_else(|| {
                anyhow::anyhow!("Peripheral '{}' config.profile must be a string", p_cfg.id)
            });
        }
        if let Some(value) = p_cfg.config.get("register_layout") {
            return value.as_str().map(Some).ok_or_else(|| {
                anyhow::anyhow!(
                    "Peripheral '{}' config.register_layout must be a string",
                    p_cfg.id
                )
            });
        }
        Ok(None)
    }

    pub(crate) fn parse_profile_or_default<T>(
        p_cfg: &PeripheralConfig,
        peripheral_kind: &str,
    ) -> anyhow::Result<T>
    where
        T: FromStr<Err = String> + Default,
    {
        let Some(profile_name) = Self::profile_name(p_cfg)? else {
            return Ok(T::default());
        };
        T::from_str(profile_name).map_err(|e| {
            anyhow::anyhow!(
                "Peripheral '{}' has invalid {} profile '{}': {}",
                p_cfg.id,
                peripheral_kind,
                profile_name,
                e
            )
        })
    }

    /// Resolve the UART register layout for a peripheral **deterministically**
    /// from its declared type. The decision order is fixed and total, and there
    /// is no path that silently mismodels a strange UART:
    ///
    ///   1. An explicit `config.profile` always wins (the author's deliberate
    ///      choice), so any UART can be pinned to any modelled layout.
    ///   2. A type whose silicon register map we actually model routes to that
    ///      layout: `*lpuart*` → Kinetis LPUART; `stm32h5`/`stm32f7` → modern
    ///      STM32 USART; any other `stm32…` name and the bare generic `"uart"`
    ///      → the classic STM32 USART map (SR/DR/BRR/CR1…).
    ///   3. Anything else — every vendor UART we do not model yet (PL011, 16550,
    ///      Gaisler APBUART, EFM32/EFR32, Renesas SCI, LiteX, SiFive, SAM, …) —
    ///      ERRORS. It must name a layout via `config.profile` to run. A UART is
    ///      never silently mapped onto an STM32 register map by omission, the
    ///      way `nxp_lpuart` was before this gate existed.
    pub(crate) fn uart_layout_for(
        p_cfg: &PeripheralConfig,
    ) -> anyhow::Result<crate::peripherals::uart::UartRegisterLayout> {
        use crate::peripherals::uart::UartRegisterLayout::{self, Lpuart, Stm32F1, Stm32V2};

        // 1. Explicit author override wins, for any UART type.
        if let Some(name) = Self::profile_name(p_cfg)? {
            return UartRegisterLayout::from_str(name).map_err(|e| {
                anyhow::anyhow!(
                    "Peripheral '{}' has invalid UART profile '{}': {}",
                    p_cfg.id,
                    name,
                    e
                )
            });
        }

        // 2. Route the families we model faithfully, by declared type. Each
        //    family's register map lives in `UartRegisterLayout`; the offsets
        //    come from datasheets / vendor CMSIS headers / in-tree drivers.
        use UartRegisterLayout::*;
        let raw = p_cfg.r#type.to_ascii_lowercase();
        let has = |needle: &str| raw.contains(needle);
        let layout = if has("lpuart") {
            Lpuart
        } else if raw == "uart" {
            // The generic escape hatch: the classic STM32 USART map.
            Stm32F1
        } else if has("stm32") {
            if has("stm32h5") || has("stm32f7") {
                Stm32V2
            } else {
                Stm32F1
            }
        } else if has("pl011") {
            Pl011
        } else if has("16550") {
            Ns16550
        } else if has("da14") {
            // Dialog/Renesas DA1469x = Synopsys DW_apb_uart (16550, 4-byte stride).
            DwApbUart
        } else if has("cadence") {
            Cadence
        } else if has("efr32") {
            Efr32
        } else if has("efm32") {
            Efm32
        } else if raw == "leuart" {
            // Exact: "leuart" is a substring of unrelated names (e.g. "simpleuart").
            Leuart
        } else if has("sci") {
            // Renesas SCI (renesas_sci, renesasraXmY_sci).
            Sci
        } else if has("gaisler") || has("apbuart") {
            Gaisler
        } else if has("npcx") {
            Npcx
        } else if has("max32650") {
            Max32650
        } else if has("opentitan") {
            OpenTitan
        } else if has("sam_usart") || has("samusart") {
            Sam
        } else if has("samd5") || has("same5") || has("sercom") {
            Sercom
        } else if has("imx") {
            Imx
        } else if has("sifive") {
            Sifive
        } else if has("litex") {
            Litex
        } else if has("murax") {
            Murax
        } else if has("coreuart") || has("miv") {
            CoreUart
        } else if has("k6xf") {
            KinetisUart
        } else if has("pulp") || has("udma") {
            Pulp
        } else if has("ft9001") || has("ft900") {
            // Bridgetek FT9xx UART is 16550-compatible.
            Ns16550
        } else if has("cosimulated") {
            // Co-simulation stub with no fixed register map — default to 16550.
            Ns16550
        } else if has("mpc5567") || has("esci") {
            Esci
        } else if has("picosoc") || has("simpleuart") {
            PicoUart
        } else {
            // 3. Unmodelled UART — refuse to guess.
            anyhow::bail!(
                "UART type '{}' (peripheral '{}') has no register layout modelled yet \
                 and no `config.profile` set; it will NOT be silently mapped onto an \
                 STM32. Choose a layout explicitly with \
                 `config: {{ profile: <one of the supported layouts> }}`, or add a \
                 dedicated model for it.",
                p_cfg.r#type,
                p_cfg.id
            );
        };
        Ok(layout)
    }

    /// Resolve the GPIO register layout for a peripheral **deterministically**
    /// from its declared type, mirroring [`Self::uart_layout_for`]. There is NO
    /// path that silently mismodels a GPIO port onto STM32F1 by omission — a
    /// wrong GPIO layout moves the output-data-register offset, and anything
    /// that latches a pin level from that register (e.g. a SPI display's D/C
    /// line via [`Self::resolve_pin_odr`]) then samples the wrong address and
    /// silently misbehaves. The FRDM-KW41Z "cow" LCD blanked exactly this way:
    /// a `type: gpio` port with no `profile` fell back to Stm32F1 (ODR @0x0C),
    /// so the D/C line resolved to an address the Kinetis firmware (PDOR @0x00)
    /// never drives, D/C stayed low, and every pixel byte decoded as a command.
    ///
    ///   1. An explicit `config.profile` always wins (author's deliberate choice).
    ///   2. A type whose silicon layout the *name* pins down routes to it:
    ///      `*nrf*` → Nordic; `stm32f4`/`*h5*`/`*v2*` → modern STM32;
    ///      `stm32_gpioport`/`stm32f1`/`stm32f2` and the legacy placeholder
    ///      ports (`efmgpioport`/`npcx_gpio`/`imxrt_gpio`, historically run on
    ///      the F1 map) → classic STM32F1.
    ///   3. The bare vendor-neutral `"gpio"` type (or any other gpio-ish type we
    ///      do not model) with NO `profile` ERRORS. It is never silently mapped
    ///      onto STM32F1 by omission.
    pub(crate) fn gpio_layout_for(
        p_cfg: &PeripheralConfig,
    ) -> anyhow::Result<crate::peripherals::gpio::GpioRegisterLayout> {
        use crate::peripherals::gpio::GpioRegisterLayout;

        // 1. Explicit author override wins, for any GPIO type.
        if let Some(name) = Self::profile_name(p_cfg)? {
            return GpioRegisterLayout::from_str(name).map_err(|e| {
                anyhow::anyhow!(
                    "Peripheral '{}' has invalid GPIO profile '{}': {}",
                    p_cfg.id,
                    name,
                    e
                )
            });
        }

        // 2. Route the families whose declared type pins down the layout.
        let raw = p_cfg.r#type.to_ascii_lowercase();
        let has = |needle: &str| raw.contains(needle);
        let layout = if has("nrf") {
            GpioRegisterLayout::Nrf52
        } else if has("stm32f4") || has("h5") || has("stm32v2") {
            GpioRegisterLayout::Stm32V2
        } else if raw == "stm32_gpioport" || has("stm32f1") || has("stm32f2") {
            GpioRegisterLayout::Stm32F1
        } else if raw == "efmgpioport" || raw == "npcx_gpio" || raw == "imxrt_gpio" {
            // Not yet modelled with a dedicated register map; historically ran
            // on the STM32F1 layout. Kept explicit (by type) so the choice is
            // visible rather than an omission-driven silent default.
            GpioRegisterLayout::Stm32F1
        } else if raw == "gpio" {
            // 3a. The bare vendor-neutral "gpio" type with no profile is the
            //     dangerous case (real product chips: KW41Z / STM32 / nRF /
            //     ESP32) — the author meant a *specific* silicon layout and
            //     omitting it silently picked STM32F1, corrupting D/C
            //     resolution (the FRDM-KW41Z "cow" blank). REFUSE to guess.
            anyhow::bail!(
                "GPIO peripheral '{}' is declared with the vendor-neutral `type: gpio` but no \
                 `config.profile`; it will NOT be silently mapped onto STM32F1 (a wrong layout \
                 moves the output register and blanks a display's D/C line). Choose a layout \
                 explicitly with `config: {{ profile: <stm32f1|stm32v2|nrf52|kinetis> }}`.",
                p_cfg.id
            );
        } else {
            // 3b. A vendor-named gpio type we do not model yet (e.g.
            //     `gaisler_gpio`, `cc2538_gpio`, `mpfs_gpio`, `gpio_esp32`, …),
            //     used by the strict-onboarding chip ramp. These historically
            //     ran on the STM32F1 placeholder layout. Keep them loadable so
            //     onboarding isn't blocked, but WARN loudly instead of failing
            //     silently — the placeholder is a known-incomplete model, not a
            //     faithful one. Pin it with `config.profile` (or add a real
            //     model) to silence this and get correct register behaviour.
            tracing::warn!(
                "GPIO type '{}' (peripheral '{}') has no dedicated register model; falling back \
                 to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — \
                 set `config.profile` or add a real model for correct behaviour.",
                p_cfg.r#type,
                p_cfg.id
            );
            GpioRegisterLayout::Stm32F1
        };
        Ok(layout)
    }

    fn resolve_peripheral_path(manifest: &SystemManifest, descriptor_path: &str) -> PathBuf {
        let raw = PathBuf::from(descriptor_path);
        if raw.is_absolute() {
            return raw;
        }

        let chip_path = Path::new(&manifest.chip);
        let chip_dir = chip_path.parent().unwrap_or_else(|| Path::new("."));
        let chip_relative = chip_dir.join(descriptor_path);
        if chip_relative.exists() {
            chip_relative
        } else {
            raw
        }
    }

    /// True when the wired devices need cycle-accurate (non-batched) execution
    /// to behave correctly. Some external devices are driven from `tick_peripherals`
    /// and observed by cycle-tight firmware loops — e.g. the HC-SR04 holds ECHO
    /// high for a pulse the firmware times by polling GPIO IN in a busy loop.
    /// Batched execution advances many instructions before ticking peripherals,
    /// so the firmware polls a frozen ECHO and measures nothing. Runners should
    /// disable instruction batching when this returns true (correctness > speed).
    /// New per-tick GPIO-timing devices should extend this predicate.
    ///
    /// Also true when an H5 op-modeling FLASH is on the bus (`flash_models_ops`,
    /// cached in `rebuild_peripheral_ranges`): its erase/bank-swap ops are
    /// recorded as pending and drained+applied per instruction by the machine
    /// layer, an invariant that only holds at batch size 1. Without this the
    /// CLI/batch run path would record the op in the FLASH cell but never apply
    /// it (no 0xFF fill, no bank swap, no reset).
    pub fn requires_cycle_accurate(&self) -> bool {
        let hcsr04_needs_cycle_accurate = !self.hcsr04.is_empty() && !self.hcsr04_event_scheduled();
        hcsr04_needs_cycle_accurate || self.has_iolink_master() || self.flash_models_ops
    }

    /// The largest `peripheral_tick_interval` this bus can run at without
    /// losing fidelity: [`RECOMMENDED_TICK_INTERVAL`] when every peripheral is
    /// scheduler-driven (cycle-exact event deadlines, observation quantised by
    /// at most one interval), `1` when anything non-relaxable is present.
    ///
    /// Non-relaxable arms are checked directly rather than through
    /// [`Self::requires_cycle_accurate`]: that predicate treats HC-SR04 as
    /// cycle-accurate until the interval is ALREADY raised above 1
    /// (`hcsr04_event_scheduled` gates on it), so consulting it at interval 1
    /// would always answer "stay at 1". HC-SR04 itself is relaxable — its ECHO
    /// edges become scheduler events (batch-clamped to the exact edge) the
    /// moment the interval rises — except under the test-only
    /// `hcsr04_scheduling_disabled` override, which pins the legacy per-tick
    /// path. Callers (the wasm `recommended_tick_interval` getter) apply the
    /// result via `set_peripheral_tick_interval` at engine init.
    pub fn max_safe_tick_interval(&self) -> u32 {
        #[cfg(feature = "event-scheduler")]
        {
            let hcsr04_forced_legacy = !self.hcsr04.is_empty() && self.hcsr04_scheduling_disabled;
            if self.legacy_walk_disabled
                && !self.has_iolink_master()
                && !self.flash_models_ops
                && !hcsr04_forced_legacy
            {
                return RECOMMENDED_TICK_INTERVAL;
            }
        }
        1
    }

    /// True when the HC-SR04 echo waveform is driven by the event scheduler
    /// (rise/fall edges scheduled at their exact cycles and drained by
    /// `Machine::drain_scheduler_events`) rather than the per-cycle
    /// `service_hcsr04` pass. Active only under the `event-scheduler` feature on
    /// a walk-deleted bus (`legacy_walk_disabled`) — the same buses that already
    /// route every migrated peripheral through the scheduler. On the legacy-walk
    /// or feature-off path the sensor stays on the per-tick service path and
    /// `requires_cycle_accurate` keeps batches at one instruction. The
    /// `hcsr04_scheduling_disabled` override forces the legacy path (differential
    /// determinism test only).
    ///
    /// Gated on `peripheral_tick_interval > 1`: at interval 1 there is no
    /// instruction batching to unlock (batches are already one instruction), so
    /// the scheduled path would only add per-cycle drain overhead for no win —
    /// the proven per-tick service path is kept, byte-for-byte identical to the
    /// pre-migration build. The scheduled path activates exactly when the browser
    /// raises the interval to batch, which is when it pays off (see the throughput
    /// numbers in the migration notes).
    #[inline]
    pub(crate) fn hcsr04_event_scheduled(&self) -> bool {
        cfg!(feature = "event-scheduler")
            && self.legacy_walk_disabled
            && !self.hcsr04.is_empty()
            && !self.hcsr04_scheduling_disabled
            && self.config.peripheral_tick_interval > 1
    }

    /// True when the per-cycle tick (`tick_peripherals_fully`) has no orchestration
    /// work beyond the NVIC scan: the legacy peripheral walk is deleted, no
    /// bus-aware peripheral needs a pre-tick pass, no Nordic GPIO/GPIOTE service
    /// is wired, no CAN synthetic testers are attached, and every HC-SR04 (if any)
    /// is event-scheduled. On such a bus the tick early-outs to just the NVIC
    /// aggregation, avoiding the phase-1 orchestration and its allocations every
    /// cycle. Only meaningful under the `event-scheduler` feature (the walk is
    /// never deleted otherwise).
    ///
    /// ESP32-C3 IRQ routing no longer pins this to `false` when the cached
    /// aggregation is available: on a walk-deleted C3 bus there are no
    /// tick-produced peripheral sources (nothing walks), and the remaining
    /// routing inputs — INTC config + FROM_CPU IPI — are re-aggregated at
    /// their MMIO write choke (`sync_esp32c3_irq_cache_write`), so the
    /// per-cycle tick genuinely has nothing left to do. Without the cache
    /// (hand-built buses) the per-tick register-read fallback is the only
    /// aggregation point, so it keeps the walk-era behaviour.
    #[cfg(feature = "event-scheduler")]
    #[inline]
    fn per_cycle_tick_is_trivial(&self) -> bool {
        self.legacy_walk_disabled
            && self.bus_tick_indices.is_empty()
            && !self.nordic_gpio_service
            && (!self.esp32c3_irq_routing || self.esp32c3_irq_cache.is_some())
            && !self.esp32s3_irq_routing
            && self.can_diagnostic_testers.is_empty()
            && self.can_uds_testers.is_empty()
            && self.can_log_players.is_empty()
            && (self.hcsr04.is_empty() || self.hcsr04_event_scheduled())
    }

    /// True when an IO-Link master peer is attached to any UART. The master is
    /// paced one byte per UART tick and runs a deterministic, tick-counted
    /// startup schedule (wake-up → IDLE → OPERATE → cyclic) with a large
    /// inter-frame gap. Under instruction batching the UART would tick only once
    /// per ~10k-instruction batch, stretching the handshake to hundreds of
    /// millions of steps; ticking per instruction keeps it well within the
    /// runner's step budget. Cheap: called once at loop setup.
    fn has_iolink_master(&self) -> bool {
        use crate::peripherals::components::IolinkMaster;
        for p in &self.peripherals {
            let Some(any) = p.dev.as_any() else { continue };
            let Some(uart) = any.downcast_ref::<Uart>() else {
                continue;
            };
            for stream in &uart.attached_streams {
                if let Some(sa) = stream.as_any() {
                    if sa.downcast_ref::<IolinkMaster>().is_some() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Service all HC-SR04 sensors for one tick: compute each sensor's ECHO
    /// level from its (write-hook-armed) echo window and drive it onto the ECHO
    /// input register, touching the bus only on a level transition. TRIG is NOT
    /// polled here — `maybe_arm_hcsr04` arms the window on the GPIO write, which
    /// is cycle-exact (see `Machine::step`). No-op when no sensors are wired.
    pub(crate) fn service_hcsr04(&mut self) {
        if self.hcsr04.is_empty() {
            return;
        }
        for i in 0..self.hcsr04.len() {
            // TRIG is no longer polled here — `maybe_arm_hcsr04` arms the window
            // on the GPIO write (cycle-exact, see the note in `Machine::step`).
            // The per-cycle work is two integer comparisons plus, only on a
            // transition, one read-modify-write of the ECHO input bit.
            self.drive_hcsr04_echo(i);
        }
    }

    /// Drive sensor `i`'s ECHO input register to the level its armed window
    /// implies at `self.current_cycle`, touching the bus only on a transition.
    /// The single choke point shared by the per-cycle [`service_hcsr04`] pass and
    /// the event-scheduler edge handler ([`apply_hcsr04_event`]) — routing both
    /// through the same `write_u32` keeps logic-analyzer probe capture on the
    /// ECHO pad byte-identical across the two paths.
    ///
    /// [`service_hcsr04`]: Self::service_hcsr04
    /// [`apply_hcsr04_event`]: Self::apply_hcsr04_event
    fn drive_hcsr04_echo(&mut self, i: usize) {
        let now = self.current_cycle;
        let echo_high = self.hcsr04[i].echo_high_at(now);
        if echo_high == self.hcsr04[i].last_echo_high() {
            return;
        }
        let echo_addr = self.hcsr04[i].echo_idr_addr;
        let echo_bit = self.hcsr04[i].echo_bit;
        let idr = self.read_u32(echo_addr).unwrap_or(0);
        let new_idr = if echo_high {
            idr | (1 << echo_bit)
        } else {
            idr & !(1 << echo_bit)
        };
        if new_idr != idr {
            let _ = self.write_u32(echo_addr, new_idr);
        }
        self.hcsr04[i].set_last_echo_high(echo_high);
    }

    /// Event-scheduler edge handler: a scheduled ECHO rise/fall for sensor
    /// `sensor` came due, so drive its ECHO input register to the current
    /// window level. Recomputing from the window (rather than trusting a
    /// hard-coded rise/fall) makes the handler idempotent and self-correcting
    /// through the same choke point the per-cycle pass uses. Called by
    /// `Machine::drain_scheduler_events` for [`crate::sched::SUBSYSTEM_PERIPHERAL_IDX`]
    /// events. Out-of-range `sensor` (a sensor removed after scheduling) is a
    /// no-op.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn apply_hcsr04_event(&mut self, sensor: usize) {
        if sensor < self.hcsr04.len() {
            self.drive_hcsr04_echo(sensor);
        }
    }

    /// Event-scheduler path: the earliest cycle at which any event-scheduled
    /// HC-SR04 must next drive its ECHO pad, or `None` when no sensor has a
    /// pending edge. The run loop clamps its batch to end exactly here so a
    /// busy-polling firmware observes the edge on time. Scoped to HC-SR04 (not
    /// every scheduled peripheral): the SPI wire engine self-corrects against
    /// its anchor when its event fires a batch late, so clamping to it would only
    /// shrink batches during framebuffer pushes for no correctness gain — HC-SR04
    /// is the one device that is polled cycle-tight and must not be observed late.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn next_hcsr04_deadline_cycle(&self) -> Option<u64> {
        if !self.hcsr04_event_scheduled() {
            return None;
        }
        let interval = (self.config.peripheral_tick_interval as u64).max(1);
        let now = self.current_cycle;
        self.hcsr04
            .iter()
            .filter_map(|s| s.next_edge_deadline_cycle(now, interval))
            .min()
    }

    /// Event-scheduler path: harvest any sensor whose echo window was (re)armed
    /// since the last harvest, returning `(sensor_idx, rise_cycle, fall_cycle)`
    /// absolute cycle deadlines (quantised up to the tick grid — see
    /// `HcSr04::take_edge_schedule`) for `Machine::drain_scheduler_events`
    /// to enqueue. No allocation when nothing was armed.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn harvest_hcsr04_edges(&mut self, interval: u64, out: &mut Vec<(usize, u64, u64)>) {
        for i in 0..self.hcsr04.len() {
            if let Some((rise, fall)) = self.hcsr04[i].take_edge_schedule(interval) {
                out.push((i, rise, fall));
            }
        }
    }

    pub(crate) fn service_can_diagnostic_testers(&mut self) {
        if self.can_diagnostic_testers.is_empty() {
            return;
        }

        for i in 0..self.can_diagnostic_testers.len() {
            if self.can_diagnostic_testers[i].sent {
                continue;
            }

            let connection = self.can_diagnostic_testers[i].connection.clone();
            let Some(idx) = self.find_peripheral_index_by_name(&connection) else {
                continue;
            };
            let Some(fdcan) = self.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<crate::peripherals::fdcan::Fdcan>())
            else {
                continue;
            };

            let frame = crate::network::CanFrame {
                id: self.can_diagnostic_testers[i].request_id,
                data: self.can_diagnostic_testers[i].request_data.clone(),
                extended: false,
                fd: self.can_diagnostic_testers[i].request_data.len() > 8,
                bitrate_switch: self.can_diagnostic_testers[i].request_data.len() > 8,
                remote: false,
            };
            if fdcan.receive_frame(frame) {
                self.can_diagnostic_testers[i].sent = true;
            }
        }
    }

    /// Per-tick service for the stateful ISO-TP/UDS testers. For each tester:
    /// resolve its peripheral by name, drain the ECU's outbound `tx_frames`,
    /// advance the FSM, and inject the next ISO-TP frame (filter-gated) when due.
    ///
    /// Works against both bxCAN (`deliver_rx`) and FDCAN (`receive_frame`); the
    /// downcast picks whichever is wired. A filtered/dropped injection (return
    /// `false`) leaves the FSM parked on the same send so it retries next tick —
    /// important on the first ticks before the ECU has configured its filter.
    pub(crate) fn service_can_uds_testers(&mut self) {
        if self.can_uds_testers.is_empty() {
            return;
        }

        for i in 0..self.can_uds_testers.len() {
            if self.can_uds_testers[i].is_terminal() {
                continue;
            }

            // Timeout guard so a broken/silent ECU never hangs the sim.
            self.can_uds_testers[i].ticks += 1;
            if self.can_uds_testers[i].ticks > self.can_uds_testers[i].max_ticks {
                self.can_uds_testers[i].state = CanUdsTesterState::Failed;
                continue;
            }

            let connection = self.can_uds_testers[i].connection.clone();
            let Some(idx) = self.find_peripheral_index_by_name(&connection) else {
                continue;
            };

            // Drain the ECU's outbound frames and feed the FSM. `observe_ecu_frame`
            // may return a payload to inject (e.g. the CF unblocked by FlowControl);
            // the actual injection happens below so both peripheral kinds share one
            // filter-gated send path.
            let request_id = self.can_uds_testers[i].request_id;
            let mut pending_inject: Option<Vec<u8>> = None;

            // Resolve the peripheral once; reborrow per phase to satisfy the
            // borrow checker (drain, then inject).
            let drained: Vec<crate::network::CanFrame> = {
                let any = self.peripherals[idx].dev.as_any_mut();
                match any {
                    Some(a) => {
                        if let Some(bx) = a.downcast_mut::<crate::peripherals::bxcan::BxCan>() {
                            bx.tx_frames.drain(..).collect()
                        } else if let Some(fd) =
                            a.downcast_mut::<crate::peripherals::fdcan::Fdcan>()
                        {
                            fd.tx_frames.drain(..).collect()
                        } else {
                            continue;
                        }
                    }
                    None => continue,
                }
            };

            for frame in &drained {
                if let Some(payload) =
                    self.can_uds_testers[i].observe_ecu_frame(frame.id, &frame.data)
                {
                    pending_inject = Some(payload);
                }
            }

            // Decide what (if anything) to inject this tick.
            let has_script = !self.can_uds_testers[i].script.is_empty();
            let to_send: Option<Vec<u8>> = if has_script {
                match self.can_uds_testers[i].state {
                    CanUdsTesterState::Start => {
                        // Build request frames for the current script step.
                        let frames = self.can_uds_testers[i].build_request_frames();
                        if let Some((first, rest)) = frames.split_first() {
                            // Queue any CFs for later (after FlowControl).
                            self.can_uds_testers[i].pending_cfs = rest.to_vec();
                            Some(first.clone())
                        } else {
                            None
                        }
                    }
                    // Use the observe result when an FC arrived this tick, or
                    // the front of pending_cfs when additional CFs remain from
                    // a previous tick's FC (no new ECU frame → pending_inject
                    // is None but the queue is non-empty).
                    CanUdsTesterState::AwaitFc => pending_inject
                        .or_else(|| self.can_uds_testers[i].pending_cfs.first().cloned()),
                    // ECU sent a FirstFrame this tick; observe_ecu_frame_script
                    // already set state=AwaitMultiResp and returned the FlowControl
                    // in pending_inject. Forward it so the ECU can send its CFs.
                    CanUdsTesterState::AwaitMultiResp => pending_inject,
                    _ => None,
                }
            } else {
                match self.can_uds_testers[i].state {
                    CanUdsTesterState::Start => Some(self.can_uds_testers[i].first_frame.clone()),
                    CanUdsTesterState::AwaitFc => pending_inject,
                    _ => None,
                }
            };

            let Some(payload) = to_send else {
                continue;
            };

            let frame = crate::network::CanFrame::classic(request_id, payload);
            let injected = {
                let any = self.peripherals[idx].dev.as_any_mut();
                match any {
                    Some(a) => {
                        if let Some(bx) = a.downcast_mut::<crate::peripherals::bxcan::BxCan>() {
                            bx.deliver_rx(frame)
                        } else if let Some(fd) =
                            a.downcast_mut::<crate::peripherals::fdcan::Fdcan>()
                        {
                            fd.receive_frame(frame)
                        } else {
                            false
                        }
                    }
                    None => false,
                }
            };

            if injected {
                // Advance only on a successful (accepted) injection; otherwise
                // stay parked and retry next tick.
                let has_script = !self.can_uds_testers[i].script.is_empty();
                match self.can_uds_testers[i].state {
                    CanUdsTesterState::Start if has_script => {
                        // SF (no pending CFs) → go straight to AwaitResp.
                        // FF (pending CFs queued) → go to AwaitFc.
                        if self.can_uds_testers[i].pending_cfs.is_empty() {
                            self.can_uds_testers[i].state = CanUdsTesterState::AwaitResp;
                        } else {
                            self.can_uds_testers[i].state = CanUdsTesterState::AwaitFc;
                        }
                    }
                    CanUdsTesterState::Start => {
                        self.can_uds_testers[i].state = CanUdsTesterState::AwaitFc
                    }
                    CanUdsTesterState::AwaitFc if has_script => {
                        // Pop the CF that was just successfully injected.
                        if !self.can_uds_testers[i].pending_cfs.is_empty() {
                            self.can_uds_testers[i].pending_cfs.remove(0);
                        }
                        // Only advance to AwaitResp once all CFs have been sent.
                        if self.can_uds_testers[i].pending_cfs.is_empty() {
                            self.can_uds_testers[i].state = CanUdsTesterState::AwaitResp;
                        }
                    }
                    CanUdsTesterState::AwaitFc => {
                        self.can_uds_testers[i].state = CanUdsTesterState::AwaitResp
                    }
                    _ => {}
                }
            }
        }
    }

    /// Per-tick service for deterministic CAN log replay nodes. For each
    /// player, advance its tick counter and deliver every due frame
    /// (`due_tick < now`) into the connected peripheral, filter-gated the
    /// same way a real bus would drop unmatched frames.
    pub(crate) fn service_can_log_players(&mut self) {
        if self.can_log_players.is_empty() {
            return;
        }
        for i in 0..self.can_log_players.len() {
            self.can_log_players[i].ticks += 1;
            if self.can_log_players[i].is_done() {
                continue;
            }
            let connection = self.can_log_players[i].connection.clone();
            let Some(idx) = self.find_peripheral_index_by_name(&connection) else {
                continue;
            };
            let now = self.can_log_players[i].ticks;
            while !self.can_log_players[i].is_done()
                && self.can_log_players[i].frames[self.can_log_players[i].next_idx].0 < now
            {
                let j = self.can_log_players[i].next_idx;
                let frame = self.can_log_players[i].frames[j].1.clone();
                let accepted = {
                    let any = self.peripherals[idx].dev.as_any_mut();
                    match any {
                        Some(a) => {
                            if let Some(bx) = a.downcast_mut::<crate::peripherals::bxcan::BxCan>() {
                                bx.deliver_rx(frame)
                            } else if let Some(fd) =
                                a.downcast_mut::<crate::peripherals::fdcan::Fdcan>()
                            {
                                fd.receive_frame(frame)
                            } else {
                                false
                            }
                        }
                        None => false,
                    }
                };
                if accepted {
                    self.can_log_players[i].delivered += 1;
                } else {
                    self.can_log_players[i].dropped += 1;
                }
                self.can_log_players[i].next_idx += 1;
            }
        }
    }

    fn yaml_u32(value: Option<&serde_yaml::Value>, default: u32) -> u32 {
        match value {
            Some(serde_yaml::Value::Number(n)) => n.as_u64().map(|v| v as u32).unwrap_or(default),
            Some(serde_yaml::Value::String(s)) => {
                let s = s.trim();
                if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                    u32::from_str_radix(&hex.replace('_', ""), 16).unwrap_or(default)
                } else {
                    s.replace('_', "").parse::<u32>().unwrap_or(default)
                }
            }
            _ => default,
        }
    }

    fn yaml_bytes(value: Option<&serde_yaml::Value>, default: &[u8]) -> Vec<u8> {
        match value {
            Some(serde_yaml::Value::Sequence(seq)) => seq
                .iter()
                .map(|value| Self::yaml_u32(Some(value), 0) as u8)
                .collect(),
            Some(serde_yaml::Value::String(s)) => s
                .split(|c: char| c.is_ascii_whitespace() || c == ',' || c == ':')
                .filter(|part| !part.is_empty())
                .map(|part| {
                    let part = part.trim();
                    if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
                        match u8::from_str_radix(hex, 16) {
                            Ok(b) => b,
                            Err(_) => {
                                tracing::warn!(
                                    "[uds-tester] malformed send byte {:?}, treating as 0x00",
                                    part
                                );
                                0
                            }
                        }
                    } else {
                        u8::from_str_radix(part, 16)
                            .unwrap_or_else(|_| part.parse::<u8>().unwrap_or(0))
                    }
                })
                .collect(),
            _ => default.to_vec(),
        }
    }

    /// Parse an expect string such as `"51 01 .."` into a mask vector.
    /// `".."` becomes `None` (wildcard); any other token is parsed as a hex
    /// byte and becomes `Some(byte)`.
    fn parse_expect(s: &str) -> Vec<Option<u8>> {
        s.split_ascii_whitespace()
            .map(|tok| {
                if tok == ".." {
                    None
                } else {
                    let hex = tok.trim_start_matches("0x").trim_start_matches("0X");
                    match u8::from_str_radix(hex, 16) {
                        Ok(b) => Some(b),
                        Err(_) => {
                            tracing::warn!(
                                "[uds-tester] malformed expect token {:?}, treating as 0x00",
                                tok
                            );
                            Some(0)
                        }
                    }
                }
            })
            .collect()
    }

    /// Parse an optional YAML `script:` sequence into a `Vec<UdsStep>`.
    fn parse_script(value: Option<&serde_yaml::Value>) -> Vec<UdsStep> {
        let seq = match value {
            Some(serde_yaml::Value::Sequence(s)) => s,
            _ => return Vec::new(),
        };
        seq.iter()
            .map(|entry| {
                let send = Self::yaml_bytes(entry.get("send"), &[]);
                let expect_str = entry.get("expect").and_then(|v| v.as_str()).unwrap_or("");
                let expect = Self::parse_expect(expect_str);
                let expect_nrc = entry
                    .get("expect_nrc")
                    .map(|v| Self::yaml_u32(Some(v), 0) as u8);
                UdsStep {
                    send,
                    expect,
                    expect_nrc,
                }
            })
            .collect()
    }

    /// Write-hook mirror of [`maybe_latch_dc`](Self::maybe_latch_dc) for the
    /// HC-SR04: after an MMIO write to peripheral `idx`, if that peripheral is
    /// the GPIO hosting any sensor's TRIG line, re-read the TRIG ODR bit and run
    /// the sensor's rising-edge/arm logic at `now = self.current_cycle`.
    ///
    /// Because TRIG only changes via a GPIO write, edge detection on the write is
    /// exactly equivalent to the old per-cycle TRIG poll, and `current_cycle`
    /// here equals the value the immediately-following `service_hcsr04` tick sees
    /// (see `Machine::step`), so the arming is cycle-exact.
    fn maybe_arm_hcsr04(&mut self, idx: usize) {
        if self.hcsr04.is_empty() {
            return;
        }
        let now = self.current_cycle;
        for i in 0..self.hcsr04.len() {
            // Resolve & cache the TRIG GPIO's peripheral index on first use.
            let trig_idx = match self.hcsr04[i].trig_peripheral_idx() {
                Some(t) => t,
                None => {
                    let trig_addr = self.hcsr04[i].trig_odr_addr;
                    match self.find_peripheral_index(trig_addr) {
                        Some(t) => {
                            self.hcsr04[i].set_trig_peripheral_idx(t);
                            t
                        }
                        None => continue,
                    }
                }
            };
            if trig_idx != idx {
                continue;
            }
            let trig_addr = self.hcsr04[i].trig_odr_addr;
            let trig_bit = self.hcsr04[i].trig_bit;
            let trig_high = self
                .read_u32(trig_addr)
                .map(|v| (v >> trig_bit) & 1 != 0)
                .unwrap_or(false);
            self.hcsr04[i].observe_trig(trig_high, now);
        }
    }

    /// Write-hook sibling of [`maybe_arm_hcsr04`](Self::maybe_arm_hcsr04) for
    /// bit-banged TM1637 displays: after an MMIO write to peripheral `idx`, if
    /// that peripheral hosts a display's CLK or DIO line, re-read both output
    /// bits and feed the `(clk, dio)` levels to the display's protocol state
    /// machine. Both lines are MCU outputs while writing, so every edge the
    /// firmware bit-bangs arrives as one of these write-hook calls — no polling.
    fn maybe_clock_tm1637(&mut self, idx: usize) {
        if self.tm1637.is_empty() {
            return;
        }
        for i in 0..self.tm1637.len() {
            // Resolve & cache the CLK / DIO GPIO peripheral indices on first use.
            let clk_idx = match self.tm1637[i].clk_peripheral_idx() {
                Some(t) => t,
                None => {
                    let addr = self.tm1637[i].clk_odr_addr;
                    match self.find_peripheral_index(addr) {
                        Some(t) => {
                            self.tm1637[i].set_clk_peripheral_idx(t);
                            t
                        }
                        None => continue,
                    }
                }
            };
            let dio_idx = match self.tm1637[i].dio_peripheral_idx() {
                Some(t) => t,
                None => {
                    let addr = self.tm1637[i].dio_odr_addr;
                    match self.find_peripheral_index(addr) {
                        Some(t) => {
                            self.tm1637[i].set_dio_peripheral_idx(t);
                            t
                        }
                        None => continue,
                    }
                }
            };
            // Only react when this write actually touched the CLK or DIO port.
            if clk_idx != idx && dio_idx != idx {
                continue;
            }
            let clk_addr = self.tm1637[i].clk_odr_addr;
            let clk_bit = self.tm1637[i].clk_bit;
            let dio_addr = self.tm1637[i].dio_odr_addr;
            let dio_bit = self.tm1637[i].dio_bit;
            let clk = self
                .read_u32(clk_addr)
                .map(|v| (v >> clk_bit) & 1 != 0)
                .unwrap_or(true);
            let dio = self
                .read_u32(dio_addr)
                .map(|v| (v >> dio_bit) & 1 != 0)
                .unwrap_or(true);
            self.tm1637[i].observe_lines(clk, dio);
        }
    }

    /// Before an SPI transfer, refresh the D/C level of any attached
    /// display that observes a D/C GPIO line (e.g. the PCD8544 Nokia 5110)
    /// by reading the driving GPIO's output bit. No-op for non-SPI writes and
    /// for SPI peripherals with no D/C-observing device (one cheap downcast).
    fn maybe_latch_dc(&mut self, idx: usize) {
        use crate::peripherals::esp32::spi::Esp32Spi;
        use crate::peripherals::esp32c3::spi::Esp32c3Spi;
        use crate::peripherals::spi::{Spi, SpiDevice};

        // Borrow the attached-device list off whichever SPI peripheral kind
        // this is (generic `Spi` for STM32/Nordic, ESP32-family SPI variants).
        fn attached_ref(any: &dyn std::any::Any) -> Option<&Vec<Box<dyn SpiDevice>>> {
            if let Some(s) = any.downcast_ref::<Spi>() {
                return Some(&s.attached_devices);
            }
            if let Some(s) = any.downcast_ref::<Esp32Spi>() {
                return Some(&s.attached_devices);
            }
            if let Some(s) = any.downcast_ref::<Esp32c3Spi>() {
                return Some(&s.attached_devices);
            }
            None
        }
        fn attached_mut(any: &mut dyn std::any::Any) -> Option<&mut Vec<Box<dyn SpiDevice>>> {
            if any.is::<Spi>() {
                return any.downcast_mut::<Spi>().map(|s| &mut s.attached_devices);
            }
            if any.is::<Esp32Spi>() {
                return any
                    .downcast_mut::<Esp32Spi>()
                    .map(|s| &mut s.attached_devices);
            }
            if any.is::<Esp32c3Spi>() {
                return any
                    .downcast_mut::<Esp32c3Spi>()
                    .map(|s| &mut s.attached_devices);
            }
            None
        }

        // Phase 1: collect (attached_index, odr_addr, bit) — immutable borrow.
        let sources: Vec<(usize, u64, u8)> = {
            let Some(any) = self.peripherals[idx].dev.as_any() else {
                return;
            };
            let Some(devs) = attached_ref(any) else {
                return;
            };
            devs.iter()
                .enumerate()
                .filter_map(|(i, d)| d.dc_source().map(|(a, b)| (i, a, b)))
                .collect()
        };
        if sources.is_empty() {
            return;
        }
        // Phase 2: sample the GPIO output bits via the bus.
        let levels: Vec<(usize, bool)> = sources
            .iter()
            .map(|&(i, addr, bit)| {
                let lvl = crate::Bus::read_u32(self, addr)
                    .map(|v| (v >> bit) & 1 != 0)
                    .unwrap_or(false);
                (i, lvl)
            })
            .collect();
        // Phase 3: push the latched levels into the devices — mutable borrow.
        if let Some(any) = self.peripherals[idx].dev.as_any_mut() {
            if let Some(devs) = attached_mut(any) {
                for (i, lvl) in levels {
                    if let Some(d) = devs.get_mut(i) {
                        d.set_dc_level(lvl);
                    }
                }
            }
        }
    }

    /// Whether peripheral `idx` is currently clocked. `true` (always-on) for any
    /// peripheral without a declared clock-gate — the safe default that keeps
    /// every existing config/firmware working. For a gated peripheral, reads the
    /// RCC enable register the gate points at and returns whether the gate bit is
    /// set. If no RCC peripheral is registered, or its register read fails, the
    /// peripheral is treated as clocked (fail-open: never wedge a chip that has
    /// no modelled RCC). Cheap: one `Option` check, then on the rare gated path a
    /// single cached-index RCC register read.
    fn is_peripheral_clocked(&self, idx: usize) -> bool {
        // missing_clock fault: force the peripheral unclocked and count the
        // suppressed access as the runtime fired-observation. Checked before the
        // bypass so a fault is honoured even under measurement mode.
        if let Some(suppressed) = self.fault_unclocked.get(&idx) {
            suppressed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        if self.clock_gating_bypass {
            return true; // measurement mode: ignore gating (see set_clock_gating_bypass)
        }
        let Some(gate) = self
            .peripherals
            .get(idx)
            .and_then(|p| p.clock_gate.as_ref())
        else {
            return true; // ungated → always accessible
        };
        let Some(rcc_idx) = self.rcc_idx else {
            return true; // no RCC modelled → don't gate
        };
        match self.peripherals[rcc_idx].dev.read_u32(gate.reg_offset) {
            Ok(reg) => (reg >> gate.bit) & 1 != 0,
            Err(_) => true,
        }
    }

    /// Disable RCC clock-gating for measurement/diagnostic tooling: while set,
    /// `is_peripheral_clocked` returns `true` for every peripheral, so a gated
    /// peripheral's registers stay accessible regardless of the RCC enable bits.
    ///
    /// This is a measurement hook, NOT something the runtime ever calls. The
    /// runtime requires firmware to clock a peripheral before use, and a gated-
    /// but-unclocked peripheral must read 0 / ignore writes (silicon fidelity).
    /// But tooling that measures *register modeling* (the SVD coverage probe)
    /// needs to ask "is this register modelled" — a property of the device —
    /// independent of whether its clock happens to be on. A flag is used rather
    /// than pre-setting the RCC enable bits because the coverage probe itself
    /// writes 0 to every register, including the RCC enable registers, which
    /// would re-gate any peripheral probed after the RCC.
    pub fn set_clock_gating_bypass(&mut self, bypass: bool) {
        self.clock_gating_bypass = bypass;
    }

    /// Inject a `missing_clock` fault: force `peripheral` to behave as if its
    /// clock is never enabled, so every CPU access to it is suppressed (reads
    /// return 0, writes are dropped) exactly like an unclocked peripheral on
    /// silicon. Returns an error if the peripheral is absent. Whether the fault
    /// actually fired (an access was suppressed) is read back with
    /// [`Self::missing_clock_suppressed`] after the run.
    pub fn inject_missing_clock(&mut self, peripheral: &str) -> Result<(), String> {
        let idx = self
            .find_peripheral_index_by_name(peripheral)
            .ok_or_else(|| format!("fault target peripheral '{peripheral}' not found"))?;
        self.fault_unclocked
            .entry(idx)
            .or_insert_with(|| std::sync::atomic::AtomicU64::new(0));
        Ok(())
    }

    /// Number of accesses suppressed by a `missing_clock` fault on `peripheral`
    /// (0 if not faulted or never accessed). `> 0` means the fault fired.
    pub fn missing_clock_suppressed(&self, peripheral: &str) -> u64 {
        let Some(idx) = self.find_peripheral_index_by_name(peripheral) else {
            return 0;
        };
        self.fault_unclocked
            .get(&idx)
            .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Inject a `stuck_at_bit` fault: hold `bit` of `register` on the
    /// declarative peripheral `peripheral` at `level` (0/1) — the CPU always
    /// reads that level regardless of writes. Returns an error if the peripheral
    /// or register is absent, the bit is out of range, or the peripheral is not
    /// a declarative `GenericPeripheral`.
    pub fn inject_stuck_bit(
        &mut self,
        peripheral: &str,
        register: &str,
        bit: u8,
        level: u8,
    ) -> Result<(), String> {
        let idx = self
            .find_peripheral_index_by_name(peripheral)
            .ok_or_else(|| format!("fault target peripheral '{peripheral}' not found"))?;
        let any = self.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| format!("peripheral '{peripheral}' is not introspectable for faults"))?;
        let generic = any
            .downcast_mut::<crate::peripherals::declarative::GenericPeripheral>()
            .ok_or_else(|| format!("peripheral '{peripheral}' is not a declarative peripheral"))?;
        if !generic.force_stuck_bit(register, bit, level) {
            return Err(format!(
                "register '{register}' bit {bit} invalid on peripheral '{peripheral}'"
            ));
        }
        Ok(())
    }

    /// Inject a `wrong_reset_value` fault: force `register` on the declarative
    /// peripheral `peripheral` to `value`. Returns an error (never a silent
    /// no-op) if the peripheral or register is absent, or the peripheral is not
    /// a declarative `GenericPeripheral`.
    pub fn inject_wrong_reset_value(
        &mut self,
        peripheral: &str,
        register: &str,
        value: u32,
    ) -> Result<(), String> {
        let idx = self
            .find_peripheral_index_by_name(peripheral)
            .ok_or_else(|| format!("fault target peripheral '{peripheral}' not found"))?;
        let any = self.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| format!("peripheral '{peripheral}' is not introspectable for faults"))?;
        let generic = any
            .downcast_mut::<crate::peripherals::declarative::GenericPeripheral>()
            .ok_or_else(|| format!("peripheral '{peripheral}' is not a declarative peripheral"))?;
        if !generic.force_register_value(register, value) {
            return Err(format!(
                "register '{register}' not found on peripheral '{peripheral}'"
            ));
        }
        Ok(())
    }

    pub fn new() -> Self {
        // Default initialization for tests
        let mut bus = Self {
            flash_thunks: std::collections::HashMap::new(),
            flash: LinearMemory::new(1024 * 1024, 0x0),
            ram: LinearMemory::new(1024 * 1024, 0x2000_0000),
            extra_mem: Vec::new(),
            peripherals: vec![
                PeripheralEntry {
                    name: "uart1".to_string(),
                    base: 0x4000_C000,
                    size: 0x400,
                    irq: Some(37),
                    dev: Box::new(crate::peripherals::uart::Uart::new()),
                    ticks_remaining: 0,
                    generation: 0,
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "gpioa".to_string(),
                    base: 0x4001_0800,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::gpio::GpioPort::new()),
                    ticks_remaining: 0,
                    generation: 0,
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "rcc".to_string(),
                    base: 0x4002_1000,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::rcc::Rcc::new()),
                    ticks_remaining: 0,
                    generation: 0,
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "systick".to_string(),
                    base: 0xE000_E010,
                    size: 0x100,
                    irq: Some(15),
                    dev: Box::new(crate::peripherals::systick::Systick::new()),
                    ticks_remaining: 0,
                    generation: 0,
                    clock_gate: None,
                },
            ],
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            clock_gating_bypass: false,
            fault_unclocked: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            legacy_tick_indices: Vec::new(),
            bus_tick_indices: Vec::new(),
            scheduler_driver_indices: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_route: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            cycle_clock: crate::CycleClock::default(),
            pending_schedule: Vec::new(),
            freerunning_timer_poll_mmio: std::cell::Cell::new(0),
            side_effecting_mmio: std::cell::Cell::new(0),
            legacy_walk_disabled: false,
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            hcsr04: Vec::new(),
            tm1637: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            can_log_players: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
            esp32c3_system_idx: None,
            esp32c3_interrupt_core0_idx: None,
            esp32c3_irq_cache: None,
            esp32c3_asserted_sources: [0; 2],
            esp32c3_sched_asserted_sources: [0; 2],
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };
        bus.rebuild_peripheral_ranges();
        bus
    }

    /// Construct an empty bus with no flash, RAM, or peripherals.
    ///
    /// Useful for tests that want to register peripherals manually without
    /// inheriting the STM32 defaults from `new()`. The flash and ram backings
    /// are zero-sized so they never satisfy a read.
    pub fn empty() -> Self {
        let mut bus = Self {
            flash_thunks: std::collections::HashMap::new(),
            flash: LinearMemory::new(0, 0),
            ram: LinearMemory::new(0, 0),
            extra_mem: Vec::new(),
            peripherals: Vec::new(),
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: false,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            clock_gating_bypass: false,
            fault_unclocked: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            legacy_tick_indices: Vec::new(),
            bus_tick_indices: Vec::new(),
            scheduler_driver_indices: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_route: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            cycle_clock: crate::CycleClock::default(),
            pending_schedule: Vec::new(),
            freerunning_timer_poll_mmio: std::cell::Cell::new(0),
            side_effecting_mmio: std::cell::Cell::new(0),
            legacy_walk_disabled: false,
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            hcsr04: Vec::new(),
            tm1637: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            can_log_players: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
            esp32c3_system_idx: None,
            esp32c3_interrupt_core0_idx: None,
            esp32c3_irq_cache: None,
            esp32c3_asserted_sources: [0; 2],
            esp32c3_sched_asserted_sources: [0; 2],
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };
        bus.rebuild_peripheral_ranges();
        bus
    }

    /// Mirror `Machine::total_cycles` into the bus AND publish it on the
    /// shared [`crate::CycleClock`] in one step, so the clock `&self`
    /// peripheral reads sync against can never skew from `current_cycle`.
    /// All engine refresh points (batch start/end, per-step, idle
    /// fast-forward) go through here.
    #[inline]
    pub fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
        self.cycle_clock.publish(cycle);
    }

    /// Append a peripheral to the bus at runtime. Useful for tests and
    /// dynamic configuration that bypasses `from_config`.
    ///
    /// **No overlap check is performed.** If two peripherals claim overlapping
    /// address ranges, reads and writes are routed to the **first** matching
    /// peripheral in registration order (i.e. the earlier-registered peripheral
    /// wins). Callers are responsible for ensuring non-overlapping ranges.
    pub fn add_peripheral(
        &mut self,
        name: &str,
        base: u64,
        size: u64,
        irq: Option<u32>,
        mut dev: Box<dyn Peripheral>,
    ) {
        // Attach choke point (walk-free plan Part 1): hand the peripheral the
        // bus's shared cycle clock before it is registered, so read-side lazy
        // sync is available from the first instruction.
        dev.attach_cycle_clock(self.cycle_clock.clone());
        self.peripherals.push(PeripheralEntry {
            name: name.to_string(),
            base,
            size,
            irq,
            dev,
            ticks_remaining: 0,
            generation: 0,
            clock_gate: None,
        });
        self.rebuild_peripheral_ranges();
    }

    /// Phase 2B.1 (issue #192): snapshot of every peripheral's lazy-cancel
    /// generation, indexed by `peripheral_idx`. Threaded into
    /// `EventScheduler::drain_due` / `next_event_deadline` so stale events
    /// (scheduled before a peripheral reset) are dropped.
    pub fn peripheral_generations(&self) -> Vec<u32> {
        self.peripherals.iter().map(|p| p.generation).collect()
    }

    /// Look up a registered ROM thunk by absolute PC.
    ///
    /// Iterates the registered peripherals; if any is a `RomThunkBank` whose
    /// address range contains `pc`, asks it for a thunk at `pc`.  Returns
    /// `None` if no bank covers the PC or no thunk is registered.
    ///
    /// Used by the CPU's `BREAK 1, 14` dispatch in `xtensa_lx7.rs`.
    pub fn get_rom_thunk(
        &self,
        pc: u32,
    ) -> Option<crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkFn> {
        // First check the Bus-level flash thunk table (for thunks installed
        // outside any RomThunkBank's range — typically firmware functions
        // resident in flash that we want to intercept).
        if let Some(&thunk) = self.flash_thunks.get(&pc) {
            return Some(thunk);
        }
        for p in &self.peripherals {
            let base = p.base as u32;
            let end = base.wrapping_add(p.size as u32);
            if pc >= base && pc < end {
                if let Some(any) = p.dev.as_any() {
                    if let Some(bank) =
                        any.downcast_ref::<crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkBank>()
                    {
                        return bank.get(pc);
                    }
                }
            }
        }
        None
    }

    /// Install a thunk for `pc` outside any `RomThunkBank`. Writes
    /// `BREAK 1,14` at `pc` so instruction fetch dispatches to the
    /// CPU's break-handler path, where `get_rom_thunk(pc)` returns the
    /// supplied closure. Used to intercept firmware functions resident
    /// in flash (e.g. ESP-IDF's `multi_heap_register`).
    pub fn install_flash_thunk(
        &mut self,
        pc: u32,
        thunk: crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkFn,
    ) -> SimResult<()> {
        let bytes = crate::peripherals::esp_xtensa_common::rom_thunks::ROM_THUNK_BREAK_BYTES;
        for (i, b) in bytes.iter().enumerate() {
            self.write_u8(pc as u64 + i as u64, *b)?;
        }
        self.flash_thunks.insert(pc, thunk);
        Ok(())
    }

    /// Plan 3: look up the cpu0 IRQ slot the registered intmatrix has bound
    /// to peripheral source `source_id`. Returns None if no intmatrix is
    /// registered or no binding exists for the source.
    pub fn route_irq_source_to_cpu_irq(&self, source_id: u32) -> Option<u8> {
        self.route_irq_source_to_cpu_irq_core(source_id, 0)
    }

    /// Plan 3 (SMP): look up the IRQ slot `source_id` is bound to on
    /// `core_id` (0 = PRO_CPU, 1 = APP_CPU) via the registered interrupt
    /// matrix's per-core map table. None if unregistered or unbound.
    pub fn route_irq_source_to_cpu_irq_core(&self, source_id: u32, core_id: u8) -> Option<u8> {
        let idx = self.esp32s3_intmatrix_idx?;
        self.peripherals
            .get(idx)?
            .dev
            .as_any()
            .and_then(|a| {
                a.downcast_ref::<crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix>()
            })
            .and_then(|matrix| matrix.route_for_core(source_id, core_id))
    }

    /// Cross-core `FROM_CPU` IPI slots currently asserted for `core_id`,
    /// read live from the ESP32-classic DPORT interrupt matrix. Replaces the
    /// old test-harness IPI bridge that polled the same registers from
    /// outside the core. Returns 0 when no DPORT is mapped (non-ESP32 buses).
    fn dport_cross_core_pending(&self, core_id: u8) -> u32 {
        // O(1) via the index cached in `rebuild_peripheral_ranges`. No DPORT
        // (every ESP32-S3 bus) → no scan, just return 0.
        let Some(idx) = self.dport_idx else { return 0 };
        self.peripherals
            .get(idx)
            .and_then(|p| p.dev.as_any())
            .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::dport::Dport>())
            .map(|dport| dport.cross_core_pending(core_id))
            .unwrap_or(0)
    }

    /// Attach a UART TX capture sink to any UART peripherals on this bus.
    ///
    /// When `echo_stdout` is false, UART writes will no longer be printed to stdout.
    pub fn attach_uart_tx_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>, echo_stdout: bool) {
        use crate::peripherals::components::IolinkMaster;
        use crate::peripherals::esp32::uart::Esp32Uart;
        use crate::peripherals::esp32s3::uart::Esp32s3Uart;
        use crate::peripherals::nrf52::uarte::Nrf52Uarte;
        for p in &mut self.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            // STM32-layout generic UART.
            if let Some(uart) = any.downcast_mut::<Uart>() {
                // UARTs carrying an IO-Link master are the binary IO-Link C/Q
                // wire, not a text console: their raw bytes must neither be
                // echoed to stdout nor captured into the assertion buffer (they
                // would pollute the console log and could collide with assertion
                // substrings). A freshly built `Uart` defaults to
                // `echo_stdout = true`, so we cannot simply skip it — we must
                // explicitly clear the sink AND disable the echo. The master's
                // own decoded records reach the capture sink via
                // `attach_iolink_master_log_sink`.
                let is_iolink_wire = uart
                    .attached_streams
                    .iter()
                    .any(|s| s.as_any().map(|a| a.is::<IolinkMaster>()).unwrap_or(false));
                if is_iolink_wire {
                    uart.set_sink(None, false);
                } else {
                    uart.set_sink(Some(sink.clone()), echo_stdout);
                }
                continue;
            }
            // Real ESP32-classic UART (echo is fixed at construction time).
            if let Some(uart) = any.downcast_mut::<Esp32Uart>() {
                uart.set_sink(Some(sink.clone()));
                continue;
            }
            // nRF52 UARTE console (EasyDMA): captured/echoed the same way.
            if let Some(uarte) = any.downcast_mut::<Nrf52Uarte>() {
                uarte.set_sink(Some(sink.clone()), echo_stdout);
                continue;
            }
            // ESP32-S3 UART0 — the faithful ROM-boot console. The real mask ROM
            // and 2nd-stage bootloader print their banner/progress here, and
            // esp-hal's default `esp_println` targets UART0 too. Without this the
            // faithful S3 boot produces no captured serial (uart.log stays empty).
            // Its `echo_stdout` is fixed at construction (uart0 defaults to true;
            // the run service passes --no-uart-stdout, but only the CAPTURE sink
            // matters there), so `set_sink` only wires the capture buffer.
            if let Some(uart) = any.downcast_mut::<Esp32s3Uart>() {
                uart.set_sink(Some(sink.clone()));
                continue;
            }
            // RP2040 USB CDC: an Arduino Mbed-OS sketch's default `Serial` is
            // USB CDC, so the console text arrives on the USB bulk-IN endpoint,
            // not UART0. Route it into the same capture sink.
            if let Some(usb) = any.downcast_mut::<crate::peripherals::rp2040::usb::Rp2040Usb>() {
                usb.set_sink(Some(sink.clone()));
            }
        }
    }

    /// Wire a capture sink into any attached IO-Link master so it records what
    /// it received over IO-Link (`MASTER PD=`, `MASTER VERDICT`, `MASTER EVENT`)
    /// into the given buffer. Pass the same `Arc<Mutex<Vec<u8>>>` used for the
    /// UART-TX capture sink so `uart_contains` assertions can observe the
    /// MASTER side (not just the device console). No-op when no IO-Link master
    /// is attached.
    pub fn attach_iolink_master_log_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>) {
        use crate::peripherals::components::IolinkMaster;
        for p in &mut self.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            let Some(uart) = any.downcast_mut::<Uart>() else {
                continue;
            };
            for stream in &mut uart.attached_streams {
                if let Some(sa) = stream.as_any_mut() {
                    if let Some(master) = sa.downcast_mut::<IolinkMaster>() {
                        master.set_log_sink(sink.clone());
                    }
                }
            }
        }
    }

    /// Attach a UART TX capture sink to one named UART peripheral.
    /// Returns false when no matching UART peripheral exists.
    pub fn attach_uart_tx_sink_named(
        &mut self,
        name: &str,
        sink: Arc<Mutex<Vec<u8>>>,
        echo_stdout: bool,
    ) -> bool {
        for p in &mut self.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            if let Some(uart) = any.downcast_mut::<Uart>() {
                uart.set_sink(None, false);
            }
        }

        for p in &mut self.peripherals {
            if p.name != name {
                continue;
            }
            let Some(any) = p.dev.as_any_mut() else {
                return false;
            };
            let Some(uart) = any.downcast_mut::<Uart>() else {
                return false;
            };
            uart.set_sink(Some(sink), echo_stdout);
            return true;
        }
        false
    }

    /// Collect shared RX buffer handles from all UART peripherals on this bus.
    /// The caller can push bytes into these buffers to inject serial input.
    pub fn attach_uart_rx_source(&self) -> Vec<Arc<Mutex<VecDeque<u8>>>> {
        let mut sources = Vec::new();
        for p in &self.peripherals {
            let Some(any) = p.dev.as_any() else {
                continue;
            };
            let Some(uart) = any.downcast_ref::<Uart>() else {
                continue;
            };
            sources.push(uart.rx_buffer());
        }
        sources
    }

    /// Whether this chip's core implements the Cortex-M bit-band feature.
    ///
    /// Bit-band aliasing is an optional feature of the Cortex-M3 and
    /// Cortex-M4 cores only — M0/M0+/M23/M33/M7 do not implement it.
    /// Chips without it may map real peripherals inside the would-be alias
    /// ranges (e.g. STM32H5/WBA M33 parts put GPIO at 0x4202_xxxx), so
    /// translating there would shadow those peripherals.
    ///
    /// A chip yaml without a `core` field keeps the historical default
    /// (enabled on Arm) so pre-existing third-party configs that rely on
    /// bit-band keep working; all in-tree Arm chip configs declare `core`.
    fn chip_has_bit_band(chip: &ChipDescriptor) -> bool {
        match chip.core.as_deref() {
            Some(core) => {
                let c = core.trim().to_ascii_lowercase();
                let c = c.strip_prefix("cortex-").unwrap_or(&c);
                matches!(c, "m3" | "m4" | "m4f")
            }
            None => matches!(chip.arch, labwired_config::Arch::Arm),
        }
    }

    /// Decode an RP2040 atomic register-alias access. Returns the aligned base
    /// register address and the atomic op when `addr` lands on a `+0x1000`
    /// (XOR), `+0x2000` (SET) or `+0x3000` (CLR) alias of a peripheral register
    /// in the APB/AHB-Lite peripheral window; `None` for a normal (`+0x0000`)
    /// access or any address outside the window. Only consulted when
    /// `atomic_register_aliases` is set, so it is a no-op for other parts.
    #[inline]
    pub fn atomic_alias_redirect(&self, addr: u64) -> Option<(u64, AtomicAliasOp)> {
        const APB_AHB: std::ops::Range<u64> = 0x4000_0000..0x5040_0000;
        if !APB_AHB.contains(&addr) {
            return None;
        }
        let op = match (addr >> 12) & 0x3 {
            0 => return None,
            1 => AtomicAliasOp::Xor,
            2 => AtomicAliasOp::Set,
            _ => AtomicAliasOp::Clr,
        };
        Some((addr & !0x3000, op))
    }

    /// Whether the 8 bytes at `addr` form a plausible Cortex-M reset vector:
    /// word[0] (the initial SP) points into RAM and word[1] (the initial PC)
    /// points into flash. Used by `Machine::load_firmware` to decide whether a
    /// candidate vector table (flash base vs. post-stage-2 offset) is the real
    /// one, so a second-stage bootloader (RP2040 boot2) can be skipped.
    pub fn vector_pair_valid(&self, addr: u64) -> bool {
        let (Some(sp), Some(pc)) = (
            self.read_u32(addr).ok(),
            self.read_u32(addr.wrapping_add(4)).ok(),
        ) else {
            return false;
        };
        let pc = pc & !1; // strip the Thumb bit
                          // The initial SP is the top of the full-descending stack, conventionally
                          // one past the last RAM byte (ram.base + ram.size), so the upper bound
                          // is inclusive.
        let in_ram = (sp as u64) >= self.ram.base_addr
            && (sp as u64) <= self.ram.base_addr + self.ram.data.len() as u64;
        let in_flash = (pc as u64) >= self.flash.base_addr
            && (pc as u64) < self.flash.base_addr + self.flash.data.len() as u64;
        in_ram && in_flash
    }

    /// Place a built peripheral on the bus using the descriptor's window size
    /// (default 4KB) and IRQ. Shared by the per-family factory dispatch and the
    /// generic-match path in [`Self::from_config`] so both stay in lockstep.
    fn push_peripheral(
        &mut self,
        p_cfg: &labwired_config::PeripheralConfig,
        mut dev: Box<dyn Peripheral>,
    ) -> anyhow::Result<()> {
        let size = match &p_cfg.size {
            Some(size) => parse_size(size)?,
            None => 0x1000,
        };
        // Attach choke point (walk-free plan Part 1): hand the peripheral the
        // bus's shared cycle clock before it is registered — the `from_config`
        // twin of the same attach in `add_peripheral`, so descriptor-built
        // models (SysTick et al.) get read-side lazy sync too. No-op for the
        // vast majority of models (the default `attach_cycle_clock` discards).
        dev.attach_cycle_clock(self.cycle_clock.clone());
        self.peripherals.push(PeripheralEntry {
            name: p_cfg.id.clone(),
            base: p_cfg.base_address,
            size,
            irq: p_cfg.irq,
            dev,
            ticks_remaining: 0,
            generation: 0,
            // Resolved in a post-pass once every peripheral (incl. the RCC) is
            // on the bus — see `resolve_clock_gates`.
            clock_gate: None,
        });
        Ok(())
    }

    /// Resolve every peripheral's optional `clock: { reg, bit }` declaration into
    /// a concrete [`ResolvedClockGate`] (RCC register offset + bit). Run as a
    /// post-pass by `from_config` after all peripherals — crucially the RCC —
    /// are on the bus, so the symbolic `reg` name can be mapped to the active
    /// chip family's RCC offset via [`Rcc::enable_reg_offset`] regardless of the
    /// order peripherals appear in the config.
    ///
    /// A peripheral with no `clock` field is left ungated. A declared gate whose
    /// `reg` name the family doesn't recognise is a hard config error (a silent
    /// "never gate" would mask a typo that lets unclocked firmware falsely pass).
    fn resolve_clock_gates(
        &mut self,
        peripherals: &[labwired_config::PeripheralConfig],
    ) -> anyhow::Result<()> {
        // Find the RCC model once (clock-gating requires one).
        let rcc_off = |bus: &SystemBus, reg: &str| -> Option<u64> {
            let idx = bus.rcc_idx?;
            bus.peripherals[idx]
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::rcc::Rcc>())
                .and_then(|rcc| rcc.enable_reg_offset(reg))
        };
        for p_cfg in peripherals {
            let Some(gate) = &p_cfg.clock else { continue };
            let Some(idx) = self.find_peripheral_index_by_name(&p_cfg.id) else {
                continue;
            };
            let Some(reg_offset) = rcc_off(self, &gate.reg) else {
                return Err(anyhow::anyhow!(
                    "peripheral '{}' declares clock gate reg '{}' which the chip's \
                     RCC model does not expose (no such enable register, or no RCC \
                     peripheral is registered)",
                    p_cfg.id,
                    gate.reg
                ));
            };
            self.peripherals[idx].clock_gate = Some(ResolvedClockGate {
                reg_offset,
                bit: gate.bit,
            });
        }
        Ok(())
    }

    pub fn signal_nvic_irq(&self, irq: u32) {
        if let Some(nvic) = &self.nvic {
            if irq >= 16 {
                let idx = (irq / 32) as usize;
                let bit = irq % 32;
                if idx < 8 {
                    nvic.ispr[idx].fetch_or(1 << bit, Ordering::SeqCst);
                }
            } else {
                // Core exceptions are handled differently if needed,
                // but signal_nvic_irq is mostly for external IRQs.
                tracing::warn!("signal_nvic_irq called for core exception {}", irq);
            }
        }
    }

    pub fn read_u32(&self, addr: u64) -> SimResult<u32> {
        if self.config.optimized_bus_access {
            if let Some(val) = self.ram.read_u32(addr) {
                return Ok(val);
            }
            if let Some(val) = self.flash.read_u32(addr) {
                return Ok(val);
            }
            for mem in &self.extra_mem {
                if let Some(val) = mem.read_u32(addr) {
                    return Ok(val);
                }
            }
            // Boot alias handle
            if self.flash.base_addr != 0 {
                let alias_end = self.flash.data.len() as u64;
                if addr + 3 < alias_end {
                    if let Some(val) = self.flash.read_u32(self.flash.base_addr + addr) {
                        return Ok(val);
                    }
                }
            }
        }

        if let Some(idx) = self.find_peripheral_index(addr) {
            if !self.is_peripheral_clocked(idx) {
                return Ok(0); // unclocked peripheral reads 0 (silicon gating)
            }
            let p = &self.peripherals[idx];
            return p.dev.read_u32(addr - p.base);
        }

        let b0 = self.read_u8(addr)? as u32;
        let b1 = self.read_u8(addr + 1)? as u32;
        let b2 = self.read_u8(addr + 2)? as u32;
        let b3 = self.read_u8(addr + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    /// Side-effect-free instruction-fetch of up to `max_len` **contiguous**
    /// guest code bytes starting at virtual address `vaddr`, materialised the
    /// SAME way the interpreter fetches instructions (see [`Self::read_u32`] /
    /// [`Self::read_u16`] and `RiscV::step`'s `bus.read_u32(pc)` fetch): linear
    /// RAM / flash / extra memories, and the MMU-translating flash-XIP windows
    /// (0x4200_0000 / 0x3C00_0000). Iterating one address at a time keeps the
    /// buffer contiguous in **virtual** space — exactly what the walker's
    /// [`CodeView`](crate::cpu::jit_framework::CodeView) indexes — even when the
    /// XIP MMU maps consecutive virtual pages to discontiguous flash pages.
    ///
    /// Stops at the first address that is not fetchable code memory (an
    /// unmapped XIP page, or an MMIO peripheral — which is deliberately **never
    /// read here**, so no FIFO / clear-on-read side effect can fire). The
    /// returned buffer is therefore a byte-exact prefix of what the CPU would
    /// fetch from `vaddr` onward.
    ///
    /// Used by the RISC-V JIT to compile hot blocks through the same XIP/MMU
    /// mapping the interpreter fetches through: reading `bus.flash.data`
    /// directly bypasses the ESP32-C3 XIP MMU and yields the wrong bytes
    /// (typically zeros → a spurious 1024-instruction runaway block).
    pub fn read_code_slice(&self, vaddr: u64, max_len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(max_len);
        for i in 0..max_len as u64 {
            match self.read_code_byte(vaddr.wrapping_add(i)) {
                Some(b) => out.push(b),
                None => break,
            }
        }
        out
    }

    /// One code byte at `addr`, or `None` if `addr` is not side-effect-free
    /// code memory. See [`Self::read_code_slice`].
    fn read_code_byte(&self, addr: u64) -> Option<u8> {
        // Linear code/data memories are side-effect free; check them in the
        // same precedence `read_u32`'s optimized path uses (ranges are
        // disjoint, so precedence only picks the one backing store).
        if let Some(b) = self.ram.read_u8(addr) {
            return Some(b);
        }
        if let Some(b) = self.flash.read_u8(addr) {
            return Some(b);
        }
        for mem in &self.extra_mem {
            if let Some(b) = mem.read_u8(addr) {
                return Some(b);
            }
        }
        // Flash-XIP window: read-only, MMU-translated exactly as the CPU's
        // instruction fetch routes it. Restricting to the XIP peripheral keeps
        // this fetch side-effect free — no MMIO is ever touched.
        if let Some(idx) = self.find_peripheral_index(addr) {
            let p = &self.peripherals[idx];
            if p.dev
                .as_any()
                .and_then(|a| {
                    a.downcast_ref::<crate::peripherals::esp32s3::flash_xip::FlashXipPeripheral>()
                })
                .is_some()
            {
                return p.dev.read(addr - p.base).ok();
            }
        }
        None
    }

    pub fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        if self.config.optimized_bus_access && self.ram.write_u32(addr, value) {
            return Ok(());
        }
        if self.config.optimized_bus_access {
            for mem in &mut self.extra_mem {
                if mem.write_u32(addr, value) {
                    return Ok(());
                }
            }
        }
        // Flash is read-only via bus writes usually, but let's stick to the behavior of write_u8
        // which would likely fail or do nothing if it's flash.
        // Actually write_u8 checks flash_alias_old etc.

        if let Some(idx) = self.find_peripheral_index(addr) {
            if !self.is_peripheral_clocked(idx) {
                return Ok(()); // unclocked peripheral: write dropped (gating)
            }
            #[cfg(feature = "event-scheduler")]
            self.sync_scheduler_peripheral(idx);
            self.maybe_latch_dc(idx);
            let c3_io_mux_capture = self.begin_esp32c3_io_mux_write(idx);
            let (base, r) = {
                let p = &mut self.peripherals[idx];
                p.ticks_remaining = 0;
                let base = p.base;
                let r = p.dev.write_u32(addr - base, value);
                (base, r)
            };
            if r.is_ok() {
                self.finish_esp32c3_io_mux_write(c3_io_mux_capture);
            }
            self.maybe_arm_hcsr04(idx);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(idx);
            if r.is_ok() {
                // Keep the C3 IRQ routing cache coherent on this inherent
                // write path too (the Bus-trait accessors already do this) —
                // host/tooling writes to INTC or FROM_CPU must re-aggregate
                // exactly like CPU stores.
                self.sync_esp32c3_irq_cache_write(idx, addr - base);
                self.refresh_legacy_tick_index(idx);
                self.refresh_bus_tick_index(idx);
            }
            return r;
        }

        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write_u8(addr + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write_u8(addr + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    pub fn read_u16(&self, addr: u64) -> SimResult<u16> {
        if self.config.optimized_bus_access {
            if let Some(val) = self.ram.read_u16(addr) {
                return Ok(val);
            }
            if let Some(val) = self.flash.read_u16(addr) {
                return Ok(val);
            }
            for mem in &self.extra_mem {
                if let Some(val) = mem.read_u16(addr) {
                    return Ok(val);
                }
            }
            // Boot alias handle
            if self.flash.base_addr != 0 {
                let alias_end = self.flash.data.len() as u64;
                if addr + 1 < alias_end {
                    if let Some(val) = self.flash.read_u16(self.flash.base_addr + addr) {
                        return Ok(val);
                    }
                }
            }
        }

        if let Some(idx) = self.find_peripheral_index(addr) {
            if !self.is_peripheral_clocked(idx) {
                return Ok(0); // unclocked peripheral reads 0 (silicon gating)
            }
            let p = &self.peripherals[idx];
            return p.dev.read_u16(addr - p.base);
        }

        let b0 = self.read_u8(addr)? as u16;
        let b1 = self.read_u8(addr + 1)? as u16;
        Ok(b0 | (b1 << 8))
    }

    pub fn write_u16(&mut self, addr: u64, value: u16) -> SimResult<()> {
        if self.config.optimized_bus_access && self.ram.write_u16(addr, value) {
            return Ok(());
        }
        if let Some(idx) = self.find_peripheral_index(addr) {
            if !self.is_peripheral_clocked(idx) {
                return Ok(()); // unclocked peripheral: write dropped (gating)
            }
            #[cfg(feature = "event-scheduler")]
            self.sync_scheduler_peripheral(idx);
            self.maybe_latch_dc(idx);
            let c3_io_mux_capture = self.begin_esp32c3_io_mux_write(idx);
            let (base, r) = {
                let p = &mut self.peripherals[idx];
                p.ticks_remaining = 0;
                let base = p.base;
                let r = p.dev.write_u16(addr - base, value);
                (base, r)
            };
            if r.is_ok() {
                self.finish_esp32c3_io_mux_write(c3_io_mux_capture);
            }
            self.maybe_arm_hcsr04(idx);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(idx);
            if r.is_ok() {
                // Cache coherence — see the write_u32 note above.
                self.sync_esp32c3_irq_cache_write(idx, addr - base);
                self.refresh_legacy_tick_index(idx);
                self.refresh_bus_tick_index(idx);
            }
            return r;
        }

        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{
        Access, ChipDescriptor, PeripheralDescriptor, RegisterDescriptor, SystemManifest,
        TimingAction, TimingDescriptor, TimingTrigger,
    };
    use std::path::PathBuf;

    #[test]
    fn timer_poll_coalesce_uses_peripheral_access_class_not_chip_names() {
        let mut bus = SystemBus::new();
        // Systimer model owns ESP register map; bus only sees MmioAccessClass.
        bus.add_peripheral(
            "systimer",
            0x6002_3000,
            0x1000,
            None,
            Box::new(crate::peripherals::esp32s3::systimer::Systimer::new(
                160_000_000,
            )),
        );
        // Default StubPeripheral is SideEffecting (CPU-agnostic default).
        bus.add_peripheral(
            "gpio",
            0x6000_4000,
            0x1000,
            None,
            Box::new(crate::peripherals::stub::StubPeripheral::new(0x1000)),
        );
        let sys_idx = bus.find_peripheral_index_by_name("systimer").unwrap();
        let gpio_idx = bus.find_peripheral_index_by_name("gpio").unwrap();

        bus.reset_mmio_activity_counters();
        bus.note_mmio_activity(sys_idx, 0x04); // poll class (model decides)
        bus.note_mmio_activity(sys_idx, 0x44); // poll class
        assert!(
            bus.take_timer_poll_coalesce_eligible(),
            "pure freerunning-timer polls should coalesce"
        );

        bus.reset_mmio_activity_counters();
        bus.note_mmio_activity(sys_idx, 0x04);
        bus.note_mmio_activity(sys_idx, 0x44);
        bus.note_mmio_activity(gpio_idx, 0x00); // side-effecting
        assert!(!bus.take_timer_poll_coalesce_eligible());
    }

    /// `max_safe_tick_interval`: 1 while the legacy walk is live (the default
    /// bus), the batching recommendation once the walk is deleted, and back to
    /// 1 when a non-relaxable device (test-only HC-SR04 legacy pin) is present.
    #[test]
    fn max_safe_tick_interval_relaxes_only_walk_deleted_buses() {
        let mut bus = SystemBus::new();
        assert_eq!(bus.max_safe_tick_interval(), 1, "legacy walk → stay exact");

        bus.legacy_walk_disabled = true;
        let relaxed = bus.max_safe_tick_interval();
        if cfg!(feature = "event-scheduler") {
            assert_eq!(relaxed, RECOMMENDED_TICK_INTERVAL);
        } else {
            assert_eq!(relaxed, 1, "feature-off builds never batch");
        }

        bus.hcsr04.push(crate::peripherals::hc_sr04::HcSr04::new(
            "dist".into(),
            0x4800_0014,
            0,
            0x4800_0010,
            1,
            1_000_000,
            100.0,
        ));
        assert_eq!(
            bus.max_safe_tick_interval(),
            relaxed,
            "HC-SR04 is relaxable (its edges become scheduler events)"
        );
        bus.hcsr04_scheduling_disabled = true;
        assert_eq!(
            bus.max_safe_tick_interval(),
            1,
            "the test-only legacy-pin override must force interval 1"
        );
    }

    /// `derive_walk_deletable`: an all-scheduler / inert bus derives deletion;
    /// a single walk-dependent peripheral (native Timer, or an unknown model
    /// keeping the conservative default) pins the walk on.
    #[test]
    fn derive_walk_deletable_is_conservative() {
        use crate::peripherals::spi::{Spi, SpiRegisterLayout};
        use crate::peripherals::stub::StubPeripheral;
        use crate::peripherals::timer::Timer;

        // Start from a truly empty peripheral set (`new()` pre-populates a few).
        let mut bus = SystemBus::new();
        bus.peripherals.clear();
        assert!(bus.derive_walk_deletable(), "empty bus derives deletion");

        // Scheduler-driven (SPI) + inert stub: still deletable.
        bus.add_peripheral(
            "spi1",
            0x4001_3000,
            0x400,
            None,
            Box::new(Spi::new_with_layout(SpiRegisterLayout::Stm32)),
        );
        bus.add_peripheral(
            "syscfg",
            0x4001_0000,
            0x400,
            None,
            Box::new(StubPeripheral::new(0)),
        );
        assert!(
            bus.derive_walk_deletable(),
            "scheduler + inert-stub bus is walk-independent"
        );

        // Add a native Timer pinned to legacy mode (its `tick()` counts once
        // CEN is set — walk work reachable via MMIO). The bus must NOT derive
        // deletion. (`add_peripheral` attaches the cycle clock, which under
        // `event-scheduler` migrates the timer to the scheduler — detach it
        // so this test keeps pinning the conservative legacy-walker default.)
        bus.add_peripheral("tim2", 0x4000_0000, 0x400, None, Box::new(Timer::new()));
        let tim_idx = bus.find_peripheral_index_by_name("tim2").unwrap();
        bus.peripherals[tim_idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Timer>()
            .unwrap()
            .force_legacy_walk();
        assert!(
            !bus.derive_walk_deletable(),
            "a legacy-walk native timer is walk-dependent — walk must stay on"
        );
    }

    /// The default `needs_legacy_walk() == true` makes an unknown/native model
    /// (here the fixed-value `TagPeripheral`, which does not override it) pin the
    /// walk on — the conservative default that prevents silently starving a
    /// peripheral of ticks.
    #[test]
    fn derive_walk_deletable_defaults_conservative_for_unknown_models() {
        let mut bus = SystemBus::new();
        bus.peripherals.clear();
        bus.add_peripheral(
            "tag",
            0x4002_0000,
            0x400,
            None,
            Box::new(TagPeripheral(0xAB)),
        );
        assert!(
            !bus.derive_walk_deletable(),
            "a model that doesn't prove walk-independence keeps the walk"
        );
    }

    /// Minimal fixed-value peripheral for routing tests: reads return a
    /// constant tag byte, writes are ignored.
    #[derive(Debug)]
    struct TagPeripheral(u8);
    impl crate::Peripheral for TagPeripheral {
        fn read(&self, _offset: u64) -> crate::SimResult<u8> {
            Ok(self.0)
        }
        fn write(&mut self, _offset: u64, _value: u8) -> crate::SimResult<()> {
            Ok(())
        }
    }

    fn declarative_descriptor(timing: Option<Vec<TimingDescriptor>>) -> PeripheralDescriptor {
        PeripheralDescriptor {
            peripheral: "test".to_string(),
            version: "1.0".to_string(),
            registers: vec![
                RegisterDescriptor {
                    id: "CTRL".to_string(),
                    address_offset: 0x00,
                    size: 32,
                    access: Access::ReadWrite,
                    reset_value: 0,
                    fields: vec![],
                    side_effects: None,
                },
                RegisterDescriptor {
                    id: "STATUS".to_string(),
                    address_offset: 0x04,
                    size: 32,
                    access: Access::ReadWrite,
                    reset_value: 0,
                    fields: vec![],
                    side_effects: None,
                },
            ],
            interrupts: None,
            timing,
        }
    }

    #[test]
    fn declarative_peripherals_enter_legacy_tick_set_only_while_events_are_pending() {
        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "idle_declarative",
            0x1000,
            0x100,
            None,
            Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                declarative_descriptor(None),
            )),
        );
        assert!(
            bus.legacy_tick_indices.is_empty(),
            "declarative peripherals with no timing events should not be in the hot tick set"
        );

        bus.add_peripheral(
            "delayed_declarative",
            0x2000,
            0x100,
            None,
            Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                declarative_descriptor(Some(vec![TimingDescriptor {
                    id: "set-status".to_string(),
                    trigger: TimingTrigger::Write {
                        register: "CTRL".to_string(),
                        value: Some(1),
                        mask: None,
                    },
                    delay_cycles: 0,
                    action: TimingAction::SetBits {
                        register: "STATUS".to_string(),
                        bits: 1,
                    },
                    interrupt: None,
                }])),
            )),
        );
        assert!(
            bus.legacy_tick_indices.is_empty(),
            "write-triggered declarative timing is inactive until firmware writes the trigger"
        );

        bus.write_u32(0x2000, 1).unwrap();
        assert!(
            bus.peripherals[1].dev.legacy_tick_active(),
            "the declarative write should schedule a pending timing event"
        );
        assert_eq!(
            bus.legacy_tick_indices,
            vec![1],
            "write-triggered timing should activate only the touched C3/S3 peripheral entry"
        );

        bus.tick_peripherals_fully();
        assert_eq!(bus.read_u32(0x2004).unwrap(), 1);
        assert!(
            bus.legacy_tick_indices.is_empty(),
            "one-shot declarative timing should leave the hot tick set after it drains"
        );
    }

    #[cfg(feature = "event-scheduler")]
    #[test]
    fn scheduler_peripherals_do_not_enter_legacy_tick_index() {
        #[derive(Debug)]
        struct SchedulerPeripheral;
        impl crate::Peripheral for SchedulerPeripheral {
            fn read(&self, _offset: u64) -> SimResult<u8> {
                Ok(0)
            }

            fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
                Ok(())
            }

            fn legacy_tick_active(&self) -> bool {
                true
            }

            fn uses_scheduler(&self) -> bool {
                true
            }
        }

        let mut bus = SystemBus::empty();
        bus.add_peripheral("sched", 0x1000, 0x100, None, Box::new(SchedulerPeripheral));

        assert!(
            bus.legacy_tick_indices.is_empty(),
            "scheduler-owned peripherals are advanced by the event scheduler, not the legacy tick walk"
        );
        assert!(!bus.refresh_legacy_tick_index(0));
        assert!(
            bus.legacy_tick_indices.is_empty(),
            "refresh must not reinsert scheduler-owned peripherals into the legacy tick walk"
        );
    }

    #[test]
    fn c3_and_s3_interrupt_routing_caches_are_separate() {
        let mut bus = SystemBus::empty();
        assert!(!bus.esp32c3_irq_routing);
        assert!(!bus.esp32s3_irq_routing);

        bus.esp32c3_irq_routing = true;
        bus.refresh_peripheral_index();
        assert!(bus.esp32c3_irq_routing);
        assert_eq!(bus.esp32c3_system_idx, None);
        assert_eq!(bus.esp32c3_interrupt_core0_idx, None);
        assert!(
            !bus.esp32s3_irq_routing,
            "enabling C3 RISC-V routing must not imply an S3 intmatrix model"
        );

        bus.add_peripheral(
            "system",
            0x600C_0000,
            0x1000,
            None,
            Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                declarative_descriptor(None),
            )),
        );
        bus.add_peripheral(
            "interrupt_core0",
            0x600C_2000,
            0x1000,
            None,
            Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                declarative_descriptor(None),
            )),
        );
        assert_eq!(bus.esp32c3_system_idx, Some(0));
        assert_eq!(bus.esp32c3_interrupt_core0_idx, Some(1));
        assert!(
            !bus.esp32s3_irq_routing,
            "adding C3 interrupt banks must not imply an S3 intmatrix model"
        );

        bus.add_peripheral(
            "intmatrix",
            0x600C_2000,
            0x1000,
            None,
            Box::new(crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix::new()),
        );
        assert!(
            bus.esp32s3_irq_routing,
            "S3 routing should be cached only when the S3 intmatrix peripheral is present"
        );
        assert!(
            bus.esp32c3_irq_routing,
            "adding S3 routing must not clear the independent C3 routing flag"
        );
    }

    #[test]
    fn missing_clock_fault_suppresses_access_and_counts() {
        let mut bus = SystemBus::new();
        let base = 0x4000_0000u64;
        bus.add_peripheral("usart1", base, 0x400, None, Box::new(TagPeripheral(0xAB)));

        // Normally clocked: reads the peripheral's tag bytes.
        assert_eq!(bus.read_u32(base).unwrap(), 0xABAB_ABAB);

        bus.inject_missing_clock("usart1").unwrap();
        assert_eq!(bus.missing_clock_suppressed("usart1"), 0);

        // Now the access is suppressed: reads 0, and the fault is recorded fired.
        assert_eq!(bus.read_u32(base).unwrap(), 0);
        assert!(bus.missing_clock_suppressed("usart1") > 0);

        // An unknown peripheral is an error, not a silent no-op.
        assert!(bus.inject_missing_clock("nope").is_err());
    }

    /// Routing must be a pure function of the address — never of access
    /// history. A broad catch-all window with a narrower twin layered inside
    /// it (the ESP32-S3 low-MMIO + per-peripheral twin pattern) must route
    /// the twin's addresses to the twin even when the immediately preceding
    /// access touched a broad-window-only address (which seeds the hint
    /// cache with the broad entry — containment alone must not let it
    /// short-circuit the canonical last-start-wins search).
    #[test]
    fn pin_labels_parse_for_both_vendor_forms() {
        // STM32 letter ports.
        assert_eq!(
            SystemBus::parse_stm32_pin("PC7"),
            Some(("gpioc".to_string(), 7))
        );
        assert_eq!(SystemBus::parse_stm32_pin("PA16"), None); // STM32 ports stop at 15
                                                              // Nordic numbered ports: nRF52840 P0.00-P0.31, P1.00-P1.15.
        assert_eq!(
            SystemBus::parse_stm32_pin("P0.04"),
            Some(("gpio0".to_string(), 4))
        );
        assert_eq!(
            SystemBus::parse_stm32_pin("P1.15"),
            Some(("gpio1".to_string(), 15))
        );
        assert_eq!(SystemBus::parse_stm32_pin("P0.32"), None);
        assert_eq!(SystemBus::parse_stm32_pin("P0."), None);
    }

    #[test]
    fn overlapping_windows_route_history_independently() {
        let mut bus = SystemBus::new();
        // Broad catch-all: 0x7000_0000..0x7000_8000, reads 0xBB.
        bus.add_peripheral(
            "broad",
            0x7000_0000,
            0x8000,
            None,
            Box::new(TagPeripheral(0xBB)),
        );
        // Narrow twin layered inside: 0x7000_4000..0x7000_5000, reads 0xAA.
        bus.add_peripheral(
            "narrow",
            0x7000_4000,
            0x1000,
            None,
            Box::new(TagPeripheral(0xAA)),
        );

        // Cold route: twin wins its window.
        assert_eq!(
            bus.read_u8(0x7000_4000).unwrap(),
            0xAA,
            "cold: twin owns it"
        );

        // Poison the hint with the broad entry, then re-route a twin address.
        assert_eq!(
            bus.read_u8(0x7000_0008).unwrap(),
            0xBB,
            "broad-only address"
        );
        assert_eq!(
            bus.read_u8(0x7000_4FFC).unwrap(),
            0xAA,
            "hint poisoned by broad entry must not hijack the twin's window"
        );

        // resolve_window must agree with dispatch, in both hint states.
        assert_eq!(bus.read_u8(0x7000_0008).unwrap(), 0xBB); // re-poison
        assert_eq!(
            bus.resolve_window(0x7000_4000),
            Some((0x7000_4000, 0x1000)),
            "resolve_window must return the twin, not the hinted broad entry"
        );

        // Addresses in the broad window above the twin still go broad —
        // including right after a twin access (reverse poisoning), and the
        // fallback must pick the GREATEST containing start, not the
        // first-registered entry.
        assert_eq!(bus.read_u8(0x7000_4000).unwrap(), 0xAA);
        assert_eq!(
            bus.read_u8(0x7000_5000).unwrap(),
            0xBB,
            "past the twin's end the broad window resumes"
        );

        // next_window_start: the twin's start bounds the broad window's
        // uniform service region (used by the coverage probe's baseline).
        assert_eq!(bus.next_window_start(0x7000_0000), Some(0x7000_4000));
        assert_eq!(
            bus.next_window_start(0x7000_4000),
            Some(0xE000_E010),
            "above the twin the next start is the default bus's systick"
        );
    }

    #[test]
    fn test_system_bus_from_config_declarative() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip_path = root.join("tests/fixtures/test_chip_declarative.yaml");
        let manifest_path = root.join("tests/fixtures/test_system_declarative.yaml");

        let chip = ChipDescriptor::from_file(&chip_path).unwrap();
        let manifest = SystemManifest::from_file(&manifest_path).unwrap();

        let bus =
            SystemBus::from_config(&chip, &manifest).expect("Failed to create bus from config");

        // Verify TIMER1 is present at 0x40001000
        let found = bus
            .peripherals
            .iter()
            .find(|p| p.name == "TIMER1")
            .expect("TIMER1 not found");
        assert_eq!(found.base, 0x40001000);
        assert_eq!(found.size, 1024);

        // Verify we can read/write to it through the bus
        // Address 0x40001000 + 0x00 = CTRL register (reset value 0)
        let ctrl_val = bus.read_u32(0x40001000).unwrap();
        assert_eq!(ctrl_val, 0);

        // Address 0x40001000 + 0x04 = COUNT register
        let mut bus = bus;
        bus.write_u32(0x40001004, 0x12345678).unwrap();
        let count_val = bus.read_u32(0x40001004).unwrap();
        assert_eq!(count_val, 0x12345678);
    }

    #[test]
    fn test_system_bus_resolves_descriptor_path_relative_to_chip_file() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip_path = root.join("tests/fixtures/test_chip_declarative.yaml");
        let manifest_path = root.join("tests/fixtures/test_system_declarative.yaml");

        let mut chip = ChipDescriptor::from_file(&chip_path).unwrap();
        let mut manifest = SystemManifest::from_file(&manifest_path).unwrap();

        // Simulate a descriptor path that is relative to chip.yaml location.
        if let Some(path) = chip.peripherals[0].config.get_mut("path") {
            *path = serde_yaml::Value::String("test_timer_descriptor.yaml".to_string());
        }
        manifest.chip = chip_path.to_string_lossy().into_owned();

        let bus =
            SystemBus::from_config(&chip, &manifest).expect("Failed to create bus from config");

        let found = bus
            .peripherals
            .iter()
            .find(|p| p.name == "TIMER1")
            .expect("TIMER1 not found");
        assert_eq!(found.base, 0x40001000);
    }

    #[test]
    fn test_from_config_attaches_adxl345_external_device_to_i2c() {
        use labwired_config::{
            Arch, ChipDescriptor, ExternalDevice, MemoryRange, PeripheralConfig, SystemManifest,
        };
        use std::collections::HashMap;

        let chip = ChipDescriptor {
            schema_version: "1.0".to_string(),
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            memory_regions: Vec::new(),
            name: "stm32f103-test".to_string(),
            arch: Arch::Arm,
            core: None,
            flash: MemoryRange {
                base: 0x0800_0000,
                size: "64KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![PeripheralConfig {
                id: "i2c1".to_string(),
                r#type: "i2c".to_string(),
                base_address: 0x4000_5400,
                size: Some("1KB".to_string()),
                irq: Some(31),
                clock: None,
                config: HashMap::new(),
            }],
            pins: Default::default(),
        };

        let mut config = HashMap::new();
        config.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(0x53.into()),
        );
        let manifest = SystemManifest {
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "adxl345-test".to_string(),
            chip: "../chips/stm32f103.yaml".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: vec![ExternalDevice {
                id: "adxl345".to_string(),
                r#type: "adxl345".to_string(),
                connection: "i2c1".to_string(),
                route: Default::default(),
                config,
            }],
            board_io: Vec::new(),
            debug_uart: None,
            peripherals: Vec::new(),
        };

        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let i2c_idx = bus.find_peripheral_index_by_name("i2c1").unwrap();
        let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
        let i2c = any.downcast_mut::<crate::peripherals::i2c::I2c>().unwrap();
        assert_eq!(i2c.attached_devices().len(), 1);
    }

    #[test]
    fn test_esp32c3_i2c_device_requires_declared_physical_route() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("read ESP32-C3 chip descriptor");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "c3-i2c-route-required"
chip: "../chips/esp32c3.yaml"
external_devices:
  - id: "bmp280"
    type: "bmp280"
    connection: "i2c0"
    config:
      i2c_address: 0x76
"#,
        )
        .expect("parse route-less C3 manifest");

        let err = match SystemBus::from_config(&chip, &manifest) {
            Ok(_) => panic!("ESP32-C3 I2C must reject a route-less external device"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("route.sda") && err.to_string().contains("route.scl"),
            "error must tell the author exactly which physical signals are required: {err:#}"
        );
    }

    #[test]
    fn curated_esp32c3_i2c_manifests_declare_physical_routes() {
        #[derive(serde::Deserialize)]
        struct DeviceInventory {
            #[serde(default)]
            external_devices: Vec<labwired_config::ExternalDevice>,
        }

        // Upgrade inventory: every curated C3 system that attaches an I²C
        // device must declare the physical pair. Keep this explicit list next
        // to the runtime gate so adding a route-less demo cannot silently
        // restore controller-only behavior.
        const MANIFESTS: &[&str] = &[
            "configs/systems/esp32c3-oled-demo.yaml",
            "configs/systems/esp32c3-oled-128x32-workshop.yaml",
            "configs/systems/esp32c3-mlx90640-thermal.yaml",
            "examples/esp32c3-mlx90640-thermal/system.yaml",
            "examples/esp32c3-mlx90640-thermal/system-fault.yaml",
            "examples/esp32c3-mlx90640-thermal/system-iolink.yaml",
            "examples/esp32c3-mlx90640-thermal/system-iolink-fault.yaml",
            "examples/esp32c3-leo-airquality/system.yaml",
            "examples/esp32c3-leo-airquality/system-fresh.yaml",
            "examples/esp32c3-leo-airquality/system-stuffy.yaml",
        ];

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut routed_devices = 0usize;
        for rel in MANIFESTS {
            let source = std::fs::read_to_string(root.join(rel))
                .unwrap_or_else(|err| panic!("read curated C3 manifest {rel}: {err}"));
            let inventory: DeviceInventory = serde_yaml::from_str(&source)
                .unwrap_or_else(|err| panic!("parse curated C3 manifest {rel}: {err}"));
            for device in inventory
                .external_devices
                .iter()
                .filter(|device| device.connection == "i2c0")
            {
                crate::peripherals::esp32c3::i2c::C3I2cPadRoute::from_manifest_route(
                    &device.route,
                )
                .unwrap_or_else(|err| {
                    panic!(
                        "curated C3 manifest {rel} external device '{}' has no usable physical I2C route: {err:#}",
                        device.id
                    )
                });
                routed_devices += 1;
            }
        }
        assert!(
            routed_devices > 0,
            "the C3 route upgrade inventory must exercise at least one I2C device"
        );
    }

    #[test]
    fn test_esp32c3_i2c_gpio_matrix_distinguishes_gpio45_from_gpio67() {
        use labwired_config::ExternalDevice;
        use std::collections::{BTreeMap, HashMap};

        const GPIO_BASE: u64 = 0x6000_4000;
        const GPIO_ENABLE_W1TS: u64 = 0x24;
        const GPIO_FUNC_IN_SEL: u64 = 0x154;
        const GPIO_FUNC_OUT_SEL: u64 = 0x554;
        const MATRIX_INPUT_SELECT: u32 = 1 << 6;
        const I2C_SCL_SIGNAL: u32 = 53;
        const I2C_SDA_SIGNAL: u32 = 54;
        const I2C_BASE: u64 = 0x6001_3000;
        const I2C_INT_RAW: u64 = 0x20;
        const I2C_REG_CTR: u64 = 0x04;
        const I2C_REG_DATA: u64 = 0x1C;
        const I2C_REG_CMD0: u64 = 0x58;
        const I2C_INT_NACK: u32 = 1 << 10;
        const I2C_INT_TRANS_COMPLETE: u32 = 1 << 7;

        fn route(sda: u8, scl: u8) -> BTreeMap<String, String> {
            BTreeMap::from([
                ("sda".to_string(), format!("GPIO{sda}")),
                ("scl".to_string(), format!("GPIO{scl}")),
            ])
        }

        fn build_bus(route: BTreeMap<String, String>) -> SystemBus {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
                .expect("read ESP32-C3 chip descriptor");
            let mut config = HashMap::new();
            config.insert(
                "i2c_address".to_string(),
                serde_yaml::Value::Number(0x3C.into()),
            );
            let manifest = SystemManifest {
                walk_deleted: Some(false),
                schema_version: "1.0".to_string(),
                name: "c3-physical-i2c-route".to_string(),
                chip: "../chips/esp32c3.yaml".to_string(),
                memory_overrides: HashMap::new(),
                external_devices: vec![ExternalDevice {
                    id: "oled".to_string(),
                    r#type: "oled-ssd1306-128x32".to_string(),
                    connection: "i2c0".to_string(),
                    route,
                    config,
                }],
                board_io: Vec::new(),
                debug_uart: None,
                peripherals: Vec::new(),
            };
            let mut bus = SystemBus::from_config(&chip, &manifest).expect("construct C3 bus");
            let i2c_idx = bus
                .find_peripheral_index_by_name("i2c0")
                .expect("C3 I2C0 must be present");
            bus.peripherals[i2c_idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>())
                .expect("C3 behavioral I2C0")
                .force_legacy_walk();
            bus
        }

        fn configure_wire_begin_equivalent(bus: &mut SystemBus, sda: u8, scl: u8) {
            bus.write_u32(GPIO_BASE + GPIO_ENABLE_W1TS, (1 << sda) | (1 << scl))
                .expect("enable I2C pads");
            bus.write_u32(
                GPIO_BASE + GPIO_FUNC_OUT_SEL + u64::from(sda) * 4,
                I2C_SDA_SIGNAL,
            )
            .expect("route SDA output");
            bus.write_u32(
                GPIO_BASE + GPIO_FUNC_OUT_SEL + u64::from(scl) * 4,
                I2C_SCL_SIGNAL,
            )
            .expect("route SCL output");
            bus.write_u32(
                GPIO_BASE + GPIO_FUNC_IN_SEL + u64::from(I2C_SDA_SIGNAL) * 4,
                MATRIX_INPUT_SELECT | u32::from(sda),
            )
            .expect("route SDA input");
            bus.write_u32(
                GPIO_BASE + GPIO_FUNC_IN_SEL + u64::from(I2C_SCL_SIGNAL) * 4,
                MATRIX_INPUT_SELECT | u32::from(scl),
            )
            .expect("route SCL input");
        }

        fn probe_oled_address(bus: &mut SystemBus) -> u32 {
            let cmd = |opcode: u32, byte_num: u32| (opcode << 11) | byte_num;
            bus.write_u32(I2C_BASE + I2C_REG_CMD0, cmd(6, 0))
                .expect("RSTART");
            bus.write_u32(I2C_BASE + I2C_REG_CMD0 + 4, cmd(1, 1))
                .expect("WRITE address");
            bus.write_u32(I2C_BASE + I2C_REG_CMD0 + 8, cmd(2, 0))
                .expect("STOP");
            bus.write_u32(I2C_BASE + I2C_REG_DATA, 0x78)
                .expect("OLED address byte");
            bus.write_u32(I2C_BASE + I2C_REG_CTR, 1 << 5)
                .expect("start I2C transaction");
            for _ in 0..1_000_000 {
                let flags = bus.read_u32(I2C_BASE + I2C_INT_RAW).unwrap();
                if flags & I2C_INT_TRANS_COMPLETE != 0 {
                    return flags;
                }
                bus.tick_peripherals_fully();
            }
            panic!("C3 I2C address probe did not complete");
        }

        // A physical OLED on GPIO4/5 must not be reached by a firmware route
        // to GPIO6/7; the exact same controller/address starts ACKing only
        // after the `Wire.begin(4, 5)`-equivalent GPIO-matrix writes.
        let mut physical_45_wrong_67 = build_bus(route(4, 5));
        configure_wire_begin_equivalent(&mut physical_45_wrong_67, 6, 7);
        assert_ne!(
            probe_oled_address(&mut physical_45_wrong_67) & I2C_INT_NACK,
            0,
            "GPIO6/7 must NACK an OLED physically wired to GPIO4/5"
        );
        let mut physical_45_right_45 = build_bus(route(4, 5));
        configure_wire_begin_equivalent(&mut physical_45_right_45, 4, 5);
        assert_eq!(
            probe_oled_address(&mut physical_45_right_45) & I2C_INT_NACK,
            0,
            "GPIO4/5 must ACK an OLED physically wired to GPIO4/5"
        );

        // Reverse the physical circuit as well: this proves the pair is not
        // metadata and that GPIO6/7 has its own observable electrical path.
        let mut physical_67_wrong_45 = build_bus(route(6, 7));
        configure_wire_begin_equivalent(&mut physical_67_wrong_45, 4, 5);
        assert_ne!(
            probe_oled_address(&mut physical_67_wrong_45) & I2C_INT_NACK,
            0,
            "GPIO4/5 must NACK an OLED physically wired to GPIO6/7"
        );
        let mut physical_67_right_67 = build_bus(route(6, 7));
        configure_wire_begin_equivalent(&mut physical_67_right_67, 6, 7);
        assert_eq!(
            probe_oled_address(&mut physical_67_right_67) & I2C_INT_NACK,
            0,
            "GPIO6/7 must ACK an OLED physically wired to GPIO6/7"
        );
    }

    /// Wiring guard for the ESP32-C3 behavioral I²C: a chip yaml declaring
    /// `i2c0` as `esp32c3_i2c` plus a system manifest declaring a BMP280 on
    /// `connection: "i2c0"` must attach that slave to the behavioral controller
    /// AND let a register-driven write-then-read transaction reach it. This is
    /// the path the MLX90640 will use (different device type, same wiring).
    #[test]
    fn test_from_config_attaches_bmp280_to_esp32c3_i2c0() {
        use labwired_config::{
            Arch, ChipDescriptor, ExternalDevice, MemoryRange, PeripheralConfig, SystemManifest,
        };
        use std::collections::{BTreeMap, HashMap};

        let chip = ChipDescriptor {
            schema_version: "1.0".to_string(),
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            memory_regions: Vec::new(),
            name: "esp32c3-i2c-test".to_string(),
            arch: Arch::RiscV,
            core: None,
            flash: MemoryRange {
                base: 0x4200_0000,
                size: "4MB".to_string(),
            },
            ram: MemoryRange {
                base: 0x3FC8_0000,
                size: "400KB".to_string(),
            },
            peripherals: vec![
                PeripheralConfig {
                    id: "i2c0".to_string(),
                    r#type: "esp32c3_i2c".to_string(),
                    base_address: 0x6001_3000,
                    size: Some("4KB".to_string()),
                    irq: None,
                    config: HashMap::new(),
                    clock: None,
                },
                PeripheralConfig {
                    id: "gpio".to_string(),
                    r#type: "esp32c3_gpio".to_string(),
                    base_address: 0x6000_4000,
                    size: Some("4KB".to_string()),
                    irq: None,
                    config: HashMap::new(),
                    clock: None,
                },
            ],
            pins: Default::default(),
        };

        let mut config = HashMap::new();
        config.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(0x76.into()),
        );
        let manifest = SystemManifest {
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "esp32c3-bmp280-test".to_string(),
            chip: "../chips/esp32c3.yaml".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: vec![ExternalDevice {
                id: "bmp280".to_string(),
                r#type: "bmp280".to_string(),
                connection: "i2c0".to_string(),
                route: BTreeMap::from([
                    ("sda".to_string(), "GPIO4".to_string()),
                    ("scl".to_string(), "GPIO5".to_string()),
                ]),
                config,
            }],
            board_io: Vec::new(),
            debug_uart: None,
            peripherals: Vec::new(),
        };

        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        // `Wire.begin(4, 5)`-equivalent GPIO-matrix setup: both output and
        // input paths must select the manifest's physical pads before the
        // attached BMP280 can answer.
        bus.write_u32(0x6000_4000 + 0x24, (1 << 4) | (1 << 5))
            .unwrap();
        bus.write_u32(0x6000_4000 + 0x554 + 4 * 4, 54).unwrap();
        bus.write_u32(0x6000_4000 + 0x554 + 5 * 4, 53).unwrap();
        bus.write_u32(0x6000_4000 + 0x154 + 54 * 4, (1 << 6) | 4)
            .unwrap();
        bus.write_u32(0x6000_4000 + 0x154 + 53 * 4, (1 << 6) | 5)
            .unwrap();
        let i2c_idx = bus
            .find_peripheral_index_by_name("i2c0")
            .expect("i2c0 must be registered");
        let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
        let i2c = any
            .downcast_mut::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>()
            .expect("i2c0 must be the behavioral Esp32c3I2c controller");
        // This test drives the bit engine directly via `tick_elapsed` (the
        // legacy walk path) with no Machine event loop; pin it off the scheduler
        // so the direct drive advances the engine (byte-identical to the
        // scheduler path, which a Machine drives via `drain_scheduler_events`).
        i2c.force_legacy_walk();

        // Drive the canonical register-pointer read of the BMP280 chip-id
        // (0xD0 → 0x58), exactly as C3 firmware would, through the controller's
        // registers: RSTART; WRITE 2 (addr+W, ptr); RSTART; WRITE 1 (addr+R);
        // READ 1; STOP. Opcodes: 6=RSTART, 1=WRITE, 3=READ, 2=STOP.
        i2c.write_u32(0x58, 6 << 11).unwrap(); // CMD0 RSTART
        i2c.write_u32(0x5C, (1 << 11) | 2).unwrap(); // CMD1 WRITE 2
        i2c.write_u32(0x60, 6 << 11).unwrap(); // CMD2 RSTART
        i2c.write_u32(0x64, (1 << 11) | 1).unwrap(); // CMD3 WRITE 1
        i2c.write_u32(0x68, (3 << 11) | 1).unwrap(); // CMD4 READ 1
        i2c.write_u32(0x6C, 2 << 11).unwrap(); // CMD5 STOP
        i2c.write_u32(0x1C, 0xEC).unwrap(); // addr+W (0x76<<1)
        i2c.write_u32(0x1C, 0xD0).unwrap(); // pointer = chip-id
        i2c.write_u32(0x1C, 0xED).unwrap(); // addr+R
        i2c.write_u32(0x04, 1 << 5).unwrap(); // TRANS_START

        // The C3 controller now clocks the command list bit-by-bit over
        // simulated cycles; run the engine to completion.
        for _ in 0..1_000_000 {
            if !i2c.engine_active() {
                break;
            }
            i2c.tick_elapsed(64);
        }
        assert!(!i2c.engine_active(), "C3 I2C bit engine must complete");

        // Address must have matched (no NACK at bit 10) and the chip-id byte
        // must round-trip out of the RX FIFO.
        let int_raw = i2c.read_u32(0x20).unwrap();
        assert_eq!(
            int_raw & (1 << 10),
            0,
            "BMP280 must ACK; INT_RAW=0x{int_raw:08x}"
        );
        assert_eq!(
            i2c.read_u32(0x1C).unwrap(),
            0x58,
            "BMP280 CHIP_ID must round-trip through the bus-attached controller"
        );
    }

    /// Wiring + reachability guard for the MLX90640 thermal camera on the
    /// ESP32-C3 behavioral I²C0: a system manifest declaring an `mlx90640` on
    /// `connection: "i2c0"` must attach it at 0x33 AND let a register-driven
    /// 16-bit-addressed read reach an EEPROM word. We read the gainEE word at
    /// EEPROM address 0x2430 (== 0x2400 + 48), which the linearized calibration
    /// fixes to 6000, exercising the 16-bit register-address protocol over the
    /// real bus-attached controller.
    #[test]
    fn test_from_config_attaches_mlx90640_to_esp32c3_i2c0_and_reads_eeprom() {
        use labwired_config::{
            Arch, ChipDescriptor, ExternalDevice, MemoryRange, PeripheralConfig, SystemManifest,
        };
        use std::collections::{BTreeMap, HashMap};

        let chip = ChipDescriptor {
            schema_version: "1.0".to_string(),
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            memory_regions: Vec::new(),
            name: "esp32c3-mlx-test".to_string(),
            arch: Arch::RiscV,
            core: None,
            flash: MemoryRange {
                base: 0x4200_0000,
                size: "4MB".to_string(),
            },
            ram: MemoryRange {
                base: 0x3FC8_0000,
                size: "400KB".to_string(),
            },
            peripherals: vec![
                PeripheralConfig {
                    id: "i2c0".to_string(),
                    r#type: "esp32c3_i2c".to_string(),
                    base_address: 0x6001_3000,
                    size: Some("4KB".to_string()),
                    irq: None,
                    config: HashMap::new(),
                    clock: None,
                },
                PeripheralConfig {
                    id: "gpio".to_string(),
                    r#type: "esp32c3_gpio".to_string(),
                    base_address: 0x6000_4000,
                    size: Some("4KB".to_string()),
                    irq: None,
                    config: HashMap::new(),
                    clock: None,
                },
            ],
            pins: Default::default(),
        };

        let mut config = HashMap::new();
        config.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(0x33.into()),
        );
        config.insert(
            "ambient_c".to_string(),
            serde_yaml::Value::Number(25.0.into()),
        );
        let manifest = SystemManifest {
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "esp32c3-mlx90640-test".to_string(),
            chip: "../chips/esp32c3.yaml".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: vec![ExternalDevice {
                id: "thermal_cam".to_string(),
                r#type: "mlx90640".to_string(),
                connection: "i2c0".to_string(),
                route: BTreeMap::from([
                    ("sda".to_string(), "GPIO4".to_string()),
                    ("scl".to_string(), "GPIO5".to_string()),
                ]),
                config,
            }],
            board_io: Vec::new(),
            debug_uart: None,
            peripherals: Vec::new(),
        };

        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        bus.write_u32(0x6000_4000 + 0x24, (1 << 4) | (1 << 5))
            .unwrap();
        bus.write_u32(0x6000_4000 + 0x554 + 4 * 4, 54).unwrap();
        bus.write_u32(0x6000_4000 + 0x554 + 5 * 4, 53).unwrap();
        bus.write_u32(0x6000_4000 + 0x154 + 54 * 4, (1 << 6) | 4)
            .unwrap();
        bus.write_u32(0x6000_4000 + 0x154 + 53 * 4, (1 << 6) | 5)
            .unwrap();
        let i2c_idx = bus
            .find_peripheral_index_by_name("i2c0")
            .expect("i2c0 must be registered");
        let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
        let i2c = any
            .downcast_mut::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>()
            .expect("i2c0 must be the behavioral Esp32c3I2c controller");
        // Direct `tick_elapsed` drive (legacy walk path), no Machine event loop:
        // pin off the scheduler so the direct drive advances the engine.
        i2c.force_legacy_walk();

        // 16-bit-addressed read of EEPROM word 0x2430: write the 2-byte big-
        // endian register address (0x24, 0x30), repeated-start, read 2 bytes
        // (MSB first). Opcodes: 6=RSTART, 1=WRITE, 3=READ, 2=STOP.
        i2c.write_u32(0x58, 6 << 11).unwrap(); // CMD0 RSTART
        i2c.write_u32(0x5C, (1 << 11) | 3).unwrap(); // CMD1 WRITE 3 (addr+W, addr_hi, addr_lo)
        i2c.write_u32(0x60, 6 << 11).unwrap(); // CMD2 RSTART
        i2c.write_u32(0x64, (1 << 11) | 1).unwrap(); // CMD3 WRITE 1 (addr+R)
        i2c.write_u32(0x68, (3 << 11) | 2).unwrap(); // CMD4 READ 2 (one 16-bit word)
        i2c.write_u32(0x6C, 2 << 11).unwrap(); // CMD5 STOP
        i2c.write_u32(0x1C, 0x66).unwrap(); // addr+W (0x33<<1)
        i2c.write_u32(0x1C, 0x24).unwrap(); // reg addr high byte
        i2c.write_u32(0x1C, 0x30).unwrap(); // reg addr low byte
        i2c.write_u32(0x1C, 0x67).unwrap(); // addr+R (0x33<<1 | 1)
        i2c.write_u32(0x04, 1 << 5).unwrap(); // TRANS_START

        // The C3 controller now clocks the command list bit-by-bit over
        // simulated cycles; run the engine to completion.
        for _ in 0..1_000_000 {
            if !i2c.engine_active() {
                break;
            }
            i2c.tick_elapsed(64);
        }
        assert!(!i2c.engine_active(), "C3 I2C bit engine must complete");

        let int_raw = i2c.read_u32(0x20).unwrap();
        assert_eq!(
            int_raw & (1 << 10),
            0,
            "MLX90640 at 0x33 must ACK; INT_RAW=0x{int_raw:08x}"
        );
        let hi = i2c.read_u32(0x1C).unwrap();
        let lo = i2c.read_u32(0x1C).unwrap();
        let word = (hi << 8) | lo;
        assert_eq!(
            word, 6000,
            "MLX90640 gainEE EEPROM word (0x2430) must round-trip the 16-bit \
             register protocol through the bus-attached C3 controller"
        );
    }

    #[test]
    fn test_from_config_can_diagnostic_tester_injects_frame_into_fdcan() {
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: "h563-test"
arch: "arm"
core: "cortex-m33"
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "64KB"
peripherals:
  - id: "fdcan1"
    type: "fdcan"
    base_address: 0x4000A400
    size: "4KB"
"#,
        )
        .unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "uds-tester"
chip: "unused"
external_devices:
  - id: "uds_tester"
    type: "can-diagnostic-tester"
    connection: "fdcan1"
    config:
      request_id: "0x7E0"
      request_data: "03 22 F1 90"
board_io: []
"#,
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(bus.can_diagnostic_testers.len(), 1);

        // Still in INIT: tester retries but cannot inject into a stopped FDCAN.
        bus.tick_peripherals_fully();
        {
            let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
            let fdcan = bus.peripherals[idx]
                .dev
                .as_any()
                .unwrap()
                .downcast_ref::<crate::peripherals::fdcan::Fdcan>()
                .unwrap();
            assert!(fdcan.trace_snapshot("fdcan1").is_empty());
        }

        // Leave INIT; next bus tick lets the reusable tester drive the CAN frame.
        bus.write_u32(0x4000_A400 + 0x018, 0).unwrap();
        bus.tick_peripherals_fully();
        let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
        let fdcan = bus.peripherals[idx]
            .dev
            .as_any()
            .unwrap()
            .downcast_ref::<crate::peripherals::fdcan::Fdcan>()
            .unwrap();
        let trace = fdcan.trace_snapshot("fdcan1");
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].direction, "rx");
        assert_eq!(trace[0].id, 0x7E0);
        assert_eq!(trace[0].data, vec![0x03, 0x22, 0xF1, 0x90]);
        assert!(bus.can_diagnostic_testers[0].sent);
    }

    /// Pure FSM walk: FirstFrame → (ECU FlowControl) → ConsecutiveFrame →
    /// (ECU positive response) → Done, driving the tester's state machine by
    /// feeding ECU frames manually (no peripheral, no bus tick). This exercises
    /// the exact observe/advance logic `service_can_uds_testers` reuses.
    #[test]
    fn uds_tester_fsm_drives_ff_fc_cf_response() {
        let mut t = CanUdsTester::new("t".into(), "bxcan1".into());
        assert_eq!(t.state, CanUdsTesterState::Start);
        assert_eq!(t.request_id, 0x111);
        assert_eq!(t.reply_id, 0x222);

        // Start: the next frame to inject is the FirstFrame; on a (simulated)
        // accepted inject the FSM advances to AwaitFc.
        assert_eq!(t.first_frame, CanUdsTester::DEFAULT_FIRST_FRAME.to_vec());
        t.state = CanUdsTesterState::AwaitFc;

        // A non-FlowControl frame, or one on the wrong id, does not unblock.
        assert!(t.observe_ecu_frame(0x999, &[0x30, 0x00, 0x00]).is_none());
        assert!(t.observe_ecu_frame(0x222, &[0x06, 0x67]).is_none());
        assert_eq!(t.state, CanUdsTesterState::AwaitFc);

        // ECU FlowControl (0x30..) on reply_id → returns the ConsecutiveFrame.
        let cf = t
            .observe_ecu_frame(0x222, &[0x30, 0x00, 0x00, 0, 0, 0, 0, 0])
            .expect("FlowControl unblocks the ConsecutiveFrame");
        assert_eq!(cf, CanUdsTester::DEFAULT_CONSECUTIVE_FRAME.to_vec());

        // Simulate the accepted CF inject.
        t.state = CanUdsTesterState::AwaitResp;

        // A wrong response (negative / different service) does not complete.
        assert!(t.observe_ecu_frame(0x222, &[0x03, 0x7F, 0x27]).is_none());
        assert_eq!(t.state, CanUdsTesterState::AwaitResp);

        // SecurityAccess positive single-frame response → Done.
        assert!(t
            .observe_ecu_frame(0x222, &[0x06, 0x67, 0x01, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE])
            .is_none());
        assert_eq!(t.state, CanUdsTesterState::Done);
        assert!(t.is_terminal());
    }

    /// Script-driven AwaitResp must decode the CAN-FD *escape* SingleFrame
    /// (`0x00 LL <payload>`, length in byte 1, payload from byte 2), not just
    /// the classic SF (`0x0L`). The H563 ECU runs ISO-TP in FD mode and answers
    /// a 20-byte ReadDataByIdentifier as one 0x00-escape SF; the old parser read
    /// the low nibble of 0x00 as length 0 and completed with an empty payload
    /// ("got []"). Regression for the h563-uds-ecu smoke.
    #[test]
    fn scripted_tester_decodes_fd_escape_single_frame() {
        let mut t = CanUdsTester::new("t".into(), "fdcan1".into());
        t.reply_id = 0x7E8;
        t.script = vec![UdsStep {
            send: vec![0x22, 0xF1, 0x90],
            expect: vec![Some(0x62), Some(0xF1), Some(0x90)],
            expect_nrc: None,
        }];
        t.step_idx = 0;
        t.state = CanUdsTesterState::AwaitResp;

        // ECU FD escape SF: byte0 = 0x00, real length = 0x14 (20) in byte1,
        // payload 62 F1 90 + 17-byte VIN string.
        let mut resp = vec![0x00, 0x14, 0x62, 0xF1, 0x90];
        resp.extend_from_slice(b"LABWIRED-H563-UDS");
        assert!(t.observe_ecu_frame(0x7E8, &resp).is_none());
        assert_eq!(
            t.state,
            CanUdsTesterState::Done,
            "FD escape SF must decode the full payload and match step 0"
        );
        assert!(t.is_terminal());
    }

    /// A malformed SingleFrame (FD escape with no length byte, or a declared
    /// length the frame does not actually carry) must fail with a clear
    /// "malformed"/"truncated" reason — not be silently decoded as a short or
    /// empty payload that then reads as an ordinary response mismatch.
    #[test]
    fn scripted_tester_rejects_malformed_single_frame() {
        let mk = || {
            let mut t = CanUdsTester::new("t".into(), "fdcan1".into());
            t.reply_id = 0x7E8;
            t.script = vec![UdsStep {
                send: vec![0x22, 0xF1, 0x90],
                expect: vec![Some(0x62), Some(0xF1), Some(0x90)],
                expect_nrc: None,
            }];
            t.state = CanUdsTesterState::AwaitResp;
            t
        };

        // FD escape SF (byte0 = 0x00) with no length byte.
        let mut t = mk();
        assert!(t.observe_ecu_frame(0x7E8, &[0x00]).is_none());
        assert_eq!(t.state, CanUdsTesterState::Failed);
        assert!(
            t.failure.as_deref().unwrap_or("").contains("malformed"),
            "expected a malformed-frame reason, got {:?}",
            t.failure
        );

        // SF that declares 20 payload bytes but carries only one.
        let mut t = mk();
        assert!(t.observe_ecu_frame(0x7E8, &[0x00, 0x14, 0x62]).is_none());
        assert_eq!(t.state, CanUdsTesterState::Failed);
        assert!(
            t.failure.as_deref().unwrap_or("").contains("truncated"),
            "expected a truncated-frame reason, got {:?}",
            t.failure
        );
    }

    /// End-to-end against a real `BxCan` registered on the bus and configured
    /// (valid BTR + accept-0x111 filter, NORMAL mode — no loopback) so
    /// `deliver_rx` accepts the tester's frames. We drive the full bus tick:
    /// FF → (ECU emits FlowControl) → CF → (ECU emits positive response) → Done.
    /// The ECU's "transmit" side is modeled by pushing frames into the bxCAN's
    /// public `tx_frames`, which the tester drains exactly as it would for a
    /// firmware-driven controller in normal mode.
    #[test]
    fn uds_tester_completes_against_real_bxcan() {
        use crate::peripherals::bxcan::BxCan;

        // bxCAN register offsets (RM0008 §24.9) addressed via the bus.
        const MCR: u64 = 0x000;
        const BTR: u64 = 0x01C;
        const FMR: u64 = 0x200;
        const FM1R: u64 = 0x204;
        const FS1R: u64 = 0x20C;
        const FFA1R: u64 = 0x214;
        const FA1R: u64 = 0x21C;
        const FBANK: u64 = 0x240;
        const VALID_BTR: u32 = 0x00DC_0009; // valid TS1/TS2, no loopback bit.

        let base: u64 = 0x4000_6400;
        let mut bus = SystemBus::empty();
        bus.add_peripheral("bxcan1", base, 0x400, None, Box::new(BxCan::new()));

        // Bring the controller up in NORMAL mode and install a bank-0 mask
        // filter accepting exactly 0x111 into FIFO0.
        bus.write_u32(base + MCR, 1).unwrap(); // INRQ: request init
        bus.write_u32(base + BTR, VALID_BTR).unwrap(); // valid timing, NOT loopback
        bus.write_u32(base + FMR, 1).unwrap(); // FINIT: filter init
        bus.write_u32(base + FS1R, 0x1).unwrap(); // bank0 32-bit
        bus.write_u32(base + FM1R, 0x0).unwrap(); // bank0 mask mode
        bus.write_u32(base + FFA1R, 0x0).unwrap(); // bank0 -> FIFO0
        bus.write_u32(base + FBANK, (0x111u32) << 21).unwrap(); // F0R1
        bus.write_u32(base + FBANK + 4, (0x111u32) << 21).unwrap(); // F0R2 mask
        bus.write_u32(base + FA1R, 0x1).unwrap(); // bank0 active
        bus.write_u32(base + FMR, 0x0).unwrap(); // clear FINIT: filters live
        bus.write_u32(base + MCR, 0).unwrap(); // leave init -> running (normal)

        bus.can_uds_testers
            .push(CanUdsTester::new("uds".into(), "bxcan1".into()));

        // Tick 1: tester injects the FirstFrame (filter accepts) → AwaitFc.
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitFc);

        // The injected FF landed in the ECU's RX FIFO0 (filter-accepted).
        {
            let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
            let bx = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<BxCan>()
                .unwrap();
            // ECU "transmits" a FlowControl frame in normal mode (id = reply_id).
            bx.tx_frames.push_back(crate::network::CanFrame::classic(
                0x222,
                vec![0x30, 0x00, 0x00, 0, 0, 0, 0, 0],
            ));
        }

        // Tick 2: tester drains the FlowControl and injects the CF → AwaitResp.
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitResp);

        // ECU "transmits" the SecurityAccess positive single-frame response.
        {
            let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
            let bx = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<BxCan>()
                .unwrap();
            bx.tx_frames.push_back(crate::network::CanFrame::classic(
                0x222,
                vec![0x06, 0x67, 0x01, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE],
            ));
        }

        // Tick 3: tester observes the positive response → Done.
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    /// End-to-end against a real `Fdcan` registered on the bus and brought to
    /// normal mode (CCCR.INIT cleared, TEST.LBCK = 0) so `receive_frame`
    /// accepts the tester's frames. We drive a script-driven exchange where the
    /// ECU responds with a multi-frame FirstFrame + CF, exercising the
    /// FlowControl-delivery path on FDCAN — the same gap class that #343
    /// identified on the analogous bxCAN path.
    ///
    /// Exchange (script: `send = "22 F1 90"`, `expect = "62 F1 90"`):
    /// 1. Tick 1 (Start): tester sends SF ReadDataById request → AwaitResp.
    /// 2. ECU replies with a FirstFrame (13-byte response) via `tx_frames`.
    /// 3. Tick 2 (AwaitResp → AwaitMultiResp): tester sees the FF, transitions,
    ///    and MUST inject a FlowControl ([0x30, 0x00, 0x00]) via `receive_frame`.
    /// 4. ECU replies with the ConsecutiveFrame.
    /// 5. Tick 3 (AwaitMultiResp → Done): PDU reassembled and matched.
    ///
    /// The discriminating assertion is the presence of a FlowControl entry
    /// (`first_byte & 0xF0 == 0x30`) in the FDCAN "rx" trace after tick 2,
    /// proving `receive_frame` was called — not merely that Done was reached.
    #[test]
    fn uds_tester_completes_against_real_fdcan() {
        use crate::peripherals::fdcan::Fdcan;

        // FDCAN1 on H563: RM0481 base 0x4000_A400.
        const FDCAN_BASE: u64 = 0x4000_A400;
        const REG_CCCR: u64 = 0x018; // CCCR offset within the peripheral window

        let mut bus = SystemBus::empty();
        bus.add_peripheral("fdcan1", FDCAN_BASE, 0x1000, None, Box::new(Fdcan::new()));

        // Bring FDCAN to normal mode (mirrors fdcan_start in h563-uds-ecu/main.c):
        //   Step 1: assert INIT + CCE (config unlock).
        bus.write_u32(FDCAN_BASE + REG_CCCR, 0x3).unwrap();
        //   Step 2: clear INIT — CCE clears with it (capture13: 0xA2→0xA0).
        bus.write_u32(FDCAN_BASE + REG_CCCR, 0x0).unwrap();
        // CCCR now reads 0x0: bus_active = true, receive_frame will accept frames.

        // Script step: ReadDataByIdentifier 0xF190 (3 bytes), expect prefix 62 F1 90.
        // The response is multi-frame (13 bytes), so the tester must send a
        // FlowControl when the ECU sends its FirstFrame.
        let mut tester = CanUdsTester::new("uds".into(), "fdcan1".into());
        tester.request_id = 0x7E0;
        tester.reply_id = 0x7E8;
        tester.script = vec![UdsStep {
            send: vec![0x22, 0xF1, 0x90],
            expect: SystemBus::parse_expect("62 F1 90"),
            expect_nrc: None,
        }];
        bus.can_uds_testers.push(tester);

        // Tick 1: script-driven Start → tester sends SF request (3 bytes fit in SF)
        // → AwaitResp (no pending CFs for a SF request).
        bus.service_can_uds_testers();
        assert_eq!(
            bus.can_uds_testers[0].state,
            CanUdsTesterState::AwaitResp,
            "state must be AwaitResp after tester sends its SF request"
        );

        // Record trace length after tick 1 so the FC check ignores the SF request.
        let trace_len_before = {
            let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
            let fd = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<Fdcan>()
                .unwrap();
            fd.trace_snapshot("fdcan1").len()
        };

        // ECU "transmits" a FirstFrame: 13-byte (0x0D) response, 6 payload bytes
        // in the FF (62 F1 90 = RDBI positive response prefix + 3 VIN chars).
        {
            let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
            let fd = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<Fdcan>()
                .unwrap();
            fd.tx_frames.push_back(crate::network::CanFrame::classic(
                0x7E8,
                vec![0x10, 0x0D, 0x62, 0xF1, 0x90, 0x31, 0x32, 0x33],
            ));
        }

        // Tick 2: tester drains the ECU FirstFrame → observe_ecu_frame_script sees
        // a 0x10 frame in AwaitResp, sets state = AwaitMultiResp, and returns the
        // FlowControl payload [0x30, 0x00, 0x00].
        // service_can_uds_testers picks that up in the AwaitMultiResp branch and
        // MUST call receive_frame to inject it onto the FDCAN.
        bus.service_can_uds_testers();
        assert_eq!(
            bus.can_uds_testers[0].state,
            CanUdsTesterState::AwaitMultiResp,
            "state must be AwaitMultiResp after receiving ECU FirstFrame"
        );

        // Discriminating assertion: a FlowControl frame (first byte & 0xF0 == 0x30)
        // with the tester's request_id (0x7E0) must appear as an "rx" entry in the
        // FDCAN trace after tick 1.  An absent FC means the tester silently dropped
        // the CTS signal — the FDCAN analogue of the bxCAN #343 bug.
        {
            let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
            let fd = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<Fdcan>()
                .unwrap();
            let trace = fd.trace_snapshot("fdcan1");
            let new_frames = &trace[trace_len_before..];
            assert!(
                new_frames.iter().any(|f| {
                    f.direction == "rx"
                        && f.id == 0x7E0
                        && f.data.first().map(|b| b & 0xF0 == 0x30).unwrap_or(false)
                }),
                "FlowControl (0x30 nibble) must appear in FDCAN rx trace after ECU FirstFrame; \
                 new frames after tick 1: {:?}",
                new_frames
                    .iter()
                    .map(|f| (f.direction.as_str(), f.id, f.data.clone()))
                    .collect::<Vec<_>>()
            );
        }

        // ECU "transmits" the ConsecutiveFrame carrying the remaining 7 bytes.
        // 13 - 6 (from FF) = 7 bytes in the CF.
        {
            let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
            let fd = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<Fdcan>()
                .unwrap();
            fd.tx_frames.push_back(crate::network::CanFrame::classic(
                0x7E8,
                vec![0x21, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x30],
            ));
        }

        // Tick 3: tester drains the CF, PDU buf reaches the declared 13 bytes,
        // complete_response matches the expect prefix → Done.
        bus.service_can_uds_testers();
        assert_eq!(
            bus.can_uds_testers[0].state,
            CanUdsTesterState::Done,
            "state must be Done after CF received and PDU matched"
        );
    }

    /// Config parsing: a `uds-tester` external device populates a
    /// `CanUdsTester` with the configured ids and payloads.
    #[test]
    fn uds_tester_parsed_from_config() {
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: "f103"
arch: "arm"
core: "cortex-m3"
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "20KB"
peripherals:
  - id: "bxcan1"
    type: "bxcan"
    base_address: 0x40006400
    size: "1KB"
"#,
        )
        .unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "uds-multiframe"
chip: "f103"
external_devices:
  - id: "uds_node"
    type: "uds-tester"
    connection: "bxcan1"
    config:
      request_id: "0x111"
      reply_id: "0x222"
      first_frame: "10 0B 27 01 5A 11 22 33"
      consecutive_frame: "21 44 55 66 77 88 55 55"
board_io: []
"#,
        )
        .unwrap();
        let bus = SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(bus.can_uds_testers.len(), 1);
        let t = &bus.can_uds_testers[0];
        assert_eq!(t.request_id, 0x111);
        assert_eq!(t.reply_id, 0x222);
        assert_eq!(t.first_frame, CanUdsTester::DEFAULT_FIRST_FRAME.to_vec());
        assert_eq!(
            t.consecutive_frame,
            CanUdsTester::DEFAULT_CONSECUTIVE_FRAME.to_vec()
        );
        assert_eq!(t.state, CanUdsTesterState::Start);
    }

    /// Minimal F103 chip yaml reused across UDS script tests.
    const MIN_F103_CHIP: &str = r#"
name: "f103"
arch: "arm"
core: "cortex-m3"
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "20KB"
peripherals:
  - id: "bxcan1"
    type: "bxcan"
    base_address: 0x40006400
    size: "1KB"
"#;

    #[test]
    fn uds_script_parses_send_expect_and_wildcards() {
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "uds-script"
chip: "f103"
external_devices:
  - id: "uds-tester"
    type: "uds-tester"
    connection: "bxcan1"
    config:
      request_id: "0x111"
      reply_id: "0x222"
      script:
        - send: "11 01"
          expect: "51 01"
        - send: "27 01"
          expect: "67 01 .."
board_io: []
"#,
        )
        .unwrap();
        let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
        let bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let t = &bus.can_uds_testers[0];
        assert_eq!(t.script.len(), 2);
        assert_eq!(t.script[0].send, vec![0x11, 0x01]);
        assert_eq!(t.script[0].expect, vec![Some(0x51), Some(0x01)]);
        assert_eq!(t.script[1].expect, vec![Some(0x67), Some(0x01), None]); // .. = wildcard
    }

    #[test]
    fn uds_script_parses_optional_expect_nrc() {
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "uds-script-opts"
chip: "f103"
external_devices:
  - id: "uds-tester"
    type: "uds-tester"
    connection: "bxcan1"
    config:
      request_id: "0x111"
      reply_id: "0x222"
      script:
        - send: "28 03"
          expect: "68 03"
          expect_nrc: "0x22"
board_io: []
"#,
        )
        .unwrap();
        let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
        let bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let step = &bus.can_uds_testers[0].script[0];
        assert_eq!(step.expect_nrc, Some(0x22));
    }

    #[test]
    fn uds_legacy_config_becomes_one_step_script() {
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "uds-legacy"
chip: "f103"
external_devices:
  - id: "uds_node"
    type: "uds-tester"
    connection: "bxcan1"
    config:
      request_id: "0x111"
      reply_id: "0x222"
      first_frame: "10 0B 27 01 5A 11 22 33"
      consecutive_frame: "21 44 55 66 77 88 55 55"
board_io: []
"#,
        )
        .unwrap();
        let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
        let bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let t = &bus.can_uds_testers[0];
        assert_eq!(t.script.len(), 1);
        assert_eq!(t.script[0].expect, vec![Some(0x06), Some(0x67)]);
        assert_eq!(
            t.script[0].send,
            vec![0x27, 0x01, 0x5A, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]
        );
        assert_eq!(t.script[0].expect_nrc, None);
    }

    /// Config parsing: a `can-player` external device with inline `data:`
    /// attaches a `CanLogPlayer` to the bus with the parsed frames.
    #[test]
    fn can_player_from_config_attaches_replayer() {
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "can-player-attach"
chip: "f103"
external_devices:
  - id: "p"
    type: "can-player"
    connection: "bxcan1"
    config:
      data: "(1.0) can0 123#11\n"
board_io: []
"#,
        )
        .unwrap();
        let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
        let bus = SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(bus.can_log_players.len(), 1);
        assert_eq!(bus.can_log_players[0].frames.len(), 1);
    }

    /// Config parsing: a `can-player` device whose `connection` doesn't name
    /// a real peripheral on the bus fails with an error naming the device.
    #[test]
    fn can_player_from_config_errors_on_missing_connection() {
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "can-player-bad-conn"
chip: "f103"
external_devices:
  - id: "p"
    type: "can-player"
    connection: "nope"
    config:
      data: "(1.0) can0 123#11\n"
board_io: []
"#,
        )
        .unwrap();
        let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
        let err = expect_from_config_error(&chip, &manifest);
        let msg = err.to_string();
        assert!(msg.contains("can-player 'p'"), "unexpected error: {msg}");
    }

    /// Config parsing: a `can-player` device with neither `path` nor `data`
    /// (post config-crate path-inlining, only `data` ever reaches core)
    /// fails with an error naming both keys.
    #[test]
    fn can_player_from_config_errors_when_neither_path_nor_data_present() {
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "can-player-no-data"
chip: "f103"
external_devices:
  - id: "p"
    type: "can-player"
    connection: "bxcan1"
    config: {}
board_io: []
"#,
        )
        .unwrap();
        let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
        let err = expect_from_config_error(&chip, &manifest);
        let msg = err.to_string();
        assert!(msg.contains("path"), "unexpected error: {msg}");
        assert!(msg.contains("data"), "unexpected error: {msg}");
    }

    /// Config parsing: an explicit `ticks_per_second:` on a `can-player`
    /// device actually reaches the attached `CanLogPlayer` — two frames 1.0s
    /// apart at 2 ticks/sec rebase to ticks 0 and 2.
    #[test]
    fn can_player_from_config_honors_ticks_per_second_override() {
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "can-player-tps"
chip: "f103"
external_devices:
  - id: "p"
    type: "can-player"
    connection: "bxcan1"
    config:
      ticks_per_second: 2
      data: "(1.0) can0 123#11\n(2.0) can0 123#22\n"
board_io: []
"#,
        )
        .unwrap();
        let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
        let bus = SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(bus.can_log_players[0].frames.len(), 2);
        assert_eq!(bus.can_log_players[0].frames[0].0, 0);
        assert_eq!(bus.can_log_players[0].frames[1].0, 2);
    }

    /// Config parsing: omitting `ticks_per_second:` defaults to
    /// 1_000_000 ticks/sec — two frames 100µs apart rebase to tick 100.
    #[test]
    fn can_player_from_config_defaults_ticks_per_second() {
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "can-player-tps-default"
chip: "f103"
external_devices:
  - id: "p"
    type: "can-player"
    connection: "bxcan1"
    config:
      data: "(10.000000) can0 123#11\n(10.000100) can0 123#22\n"
board_io: []
"#,
        )
        .unwrap();
        let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
        let bus = SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(bus.can_log_players[0].frames.len(), 2);
        assert_eq!(bus.can_log_players[0].frames[0].0, 0);
        assert_eq!(bus.can_log_players[0].frames[1].0, 100);
    }

    /// Parse a minimal chip yaml with the given header lines (name/arch/core).
    fn bit_band_test_chip(header: &str, gpio_base: &str, gpio_profile: &str) -> ChipDescriptor {
        let yaml = format!(
            r#"
{header}
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "64KB"
peripherals:
  - id: "gpiox"
    type: "gpio"
    base_address: {gpio_base}
    size: "1KB"
    config:
      profile: "{gpio_profile}"
"#
        );
        serde_yaml::from_str(&yaml).expect("test chip yaml must parse")
    }

    fn empty_manifest() -> SystemManifest {
        SystemManifest {
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "bit-band-test".to_string(),
            chip: "unused".to_string(),
            memory_overrides: std::collections::HashMap::new(),
            external_devices: Vec::new(),
            board_io: Vec::new(),
            debug_uart: None,
            peripherals: Vec::new(),
        }
    }

    /// Cortex-M33 parts (STM32H5/WBA) have no bit-band feature and map real
    /// peripherals inside 0x4200_0000-0x43FF_FFFF. Word accesses there must
    /// reach the peripheral model, never be alias-translated.
    #[test]
    fn from_config_m33_gpio_in_alias_range_receives_word_accesses() {
        let chip = bit_band_test_chip(
            "name: \"m33-test\"\narch: \"arm\"\ncore: \"cortex-m33\"",
            "0x42020400",
            "stm32v2",
        );
        let mut bus = SystemBus::from_config(&chip, &empty_manifest()).unwrap();

        // Go through the `crate::Bus` trait — the CPU's access path, where
        // bit-band translation lives (the inherent methods skip it).
        // BSRR (V2 offset 0x18): set pin 0.
        crate::Bus::write_u32(&mut bus, 0x4202_0418, 0x0000_0001)
            .expect("BSRR word write must reach the GPIO model, not bit-band");
        // ODR (V2 offset 0x14) must show the pin high.
        let odr = crate::Bus::read_u32(&bus, 0x4202_0414)
            .expect("ODR word read must reach the GPIO model, not bit-band");
        assert_eq!(odr & 1, 1, "GPIO BSRR write was shadowed by bit-band alias");
    }

    /// Cortex-M3 parts (STM32F1) DO have the bit-band feature: word accesses
    /// to the 0x4200_0000 alias region must keep translating to single-bit
    /// operations on the underlying 0x4000_0000 peripheral registers.
    #[test]
    fn from_config_m3_bit_band_alias_still_translates() {
        let chip = bit_band_test_chip(
            "name: \"m3-test\"\narch: \"arm\"\ncore: \"cortex-m3\"",
            "0x40011000",
            "stm32f1",
        );
        let mut bus = SystemBus::from_config(&chip, &empty_manifest()).unwrap();

        // Alias word for GPIOC_ODR (0x4001100C) bit 0:
        // 0x42000000 + (0x1100C * 32) + (0 * 4) = 0x42220180.
        // Trait path (`crate::Bus`) — the CPU's access path with bit-band.
        crate::Bus::write_u32(&mut bus, 0x4222_0180, 1)
            .expect("bit-band alias write must translate on M3");
        let odr = crate::Bus::read_u32(&bus, 0x4001_100C).unwrap();
        assert_eq!(odr & 1, 1, "bit-band alias write must set ODR bit 0");
        assert_eq!(
            crate::Bus::read_u32(&bus, 0x4222_0180).unwrap(),
            1,
            "bit-band alias read must return the physical bit"
        );
    }

    /// Bit-band gating matrix: only M3/M4 cores have the feature. Absent
    /// core info on an Arm chip preserves the historical default (enabled)
    /// for configs that predate the `core` field.
    #[test]
    fn from_config_bit_band_gated_on_core() {
        let manifest = empty_manifest();
        let cases: &[(&str, bool)] = &[
            ("core: \"cortex-m3\"", true),
            ("core: \"cortex-m4\"", true),
            ("core: \"cortex-m0+\"", false),
            ("core: \"cortex-m7\"", false),
            ("core: \"cortex-m23\"", false),
            ("core: \"cortex-m33\"", false),
            ("", true), // absent core on Arm: historical default
        ];
        for (core_line, expected) in cases {
            let header = format!("name: \"gate-test\"\narch: \"arm\"\n{core_line}");
            let chip = bit_band_test_chip(&header, "0x40011000", "stm32f1");
            let bus = SystemBus::from_config(&chip, &manifest).unwrap();
            assert_eq!(
                bus.bit_band_enabled, *expected,
                "bit_band_enabled mismatch for chip header {header:?}"
            );
        }
    }

    fn chip_with_i2c_and_uart() -> labwired_config::ChipDescriptor {
        use labwired_config::{Arch, MemoryRange, PeripheralConfig};
        use std::collections::HashMap;

        labwired_config::ChipDescriptor {
            schema_version: "1.0".to_string(),
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            memory_regions: Vec::new(),
            name: "stm32f103-test".to_string(),
            arch: Arch::Arm,
            core: None,
            flash: MemoryRange {
                base: 0x0800_0000,
                size: "64KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![
                PeripheralConfig {
                    id: "i2c1".to_string(),
                    r#type: "i2c".to_string(),
                    base_address: 0x4000_5400,
                    size: Some("1KB".to_string()),
                    irq: Some(31),
                    clock: None,
                    config: HashMap::new(),
                },
                PeripheralConfig {
                    id: "uart1".to_string(),
                    r#type: "uart".to_string(),
                    base_address: 0x4000_3800,
                    size: Some("1KB".to_string()),
                    irq: Some(37),
                    clock: None,
                    config: HashMap::new(),
                },
            ],
            pins: Default::default(),
        }
    }

    fn manifest_with_external_device(
        r#type: &str,
        connection: &str,
        config: std::collections::HashMap<String, serde_yaml::Value>,
    ) -> labwired_config::SystemManifest {
        labwired_config::SystemManifest {
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "adxl345-test".to_string(),
            chip: "../chips/stm32f103.yaml".to_string(),
            memory_overrides: std::collections::HashMap::new(),
            external_devices: vec![labwired_config::ExternalDevice {
                id: "sensor1".to_string(),
                r#type: r#type.to_string(),
                connection: connection.to_string(),
                route: Default::default(),
                config,
            }],
            board_io: Vec::new(),
            debug_uart: None,
            peripherals: Vec::new(),
        }
    }

    fn assert_external_device_error_contains_context(
        err: anyhow::Error,
        ext_type: &str,
        connection: &str,
    ) {
        let message = err.to_string();
        assert!(
            message.contains("sensor1"),
            "error missing external device id: {message}"
        );
        assert!(
            message.contains(ext_type),
            "error missing external device type: {message}"
        );
        assert!(
            message.contains(connection),
            "error missing external device connection: {message}"
        );
    }

    fn expect_from_config_error(
        chip: &labwired_config::ChipDescriptor,
        manifest: &labwired_config::SystemManifest,
    ) -> anyhow::Error {
        match SystemBus::from_config(chip, manifest) {
            Ok(_) => panic!("expected SystemBus::from_config to reject manifest"),
            Err(err) => err,
        }
    }

    #[test]
    fn test_from_config_errors_for_missing_external_device_connection() {
        let chip = chip_with_i2c_and_uart();
        let manifest = manifest_with_external_device(
            "adxl345",
            "missing-i2c",
            std::collections::HashMap::new(),
        );

        let err = expect_from_config_error(&chip, &manifest);

        assert_external_device_error_contains_context(err, "adxl345", "missing-i2c");
    }

    #[test]
    fn test_from_config_errors_for_external_device_on_non_i2c_connection() {
        let chip = chip_with_i2c_and_uart();
        let manifest =
            manifest_with_external_device("adxl345", "uart1", std::collections::HashMap::new());

        let err = expect_from_config_error(&chip, &manifest);

        assert_external_device_error_contains_context(err, "adxl345", "uart1");
    }

    #[test]
    fn test_from_config_skips_unsupported_external_device_type() {
        let chip = chip_with_i2c_and_uart();
        let mut config = std::collections::HashMap::new();
        config.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(0x48.into()),
        );
        // Use a clearly-fictional device type — tmp102/adxl345/etc. are all
        // real components now, so we need something the factory will refuse.
        let manifest = manifest_with_external_device("definitely_not_a_device", "i2c1", config);

        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let i2c_idx = bus.find_peripheral_index_by_name("i2c1").unwrap();
        let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
        let i2c = any.downcast_mut::<crate::peripherals::i2c::I2c>().unwrap();

        assert_eq!(i2c.attached_devices().len(), 0);
    }

    #[test]
    fn test_from_config_errors_for_invalid_external_device_i2c_address() {
        for value in [
            serde_yaml::Value::String("0x53".to_string()),
            serde_yaml::Value::Number(0x80.into()),
        ] {
            let chip = chip_with_i2c_and_uart();
            let mut config = std::collections::HashMap::new();
            config.insert("i2c_address".to_string(), value);
            let manifest = manifest_with_external_device("adxl345", "i2c1", config);

            let err = expect_from_config_error(&chip, &manifest);

            assert_external_device_error_contains_context(err, "adxl345", "i2c1");
        }
    }

    #[test]
    fn test_system_bus_memory_observer() {
        use std::sync::Arc;
        use std::sync::Mutex;

        #[derive(Debug)]
        struct MockObserver {
            writes: Arc<Mutex<Vec<(u64, u8, u8)>>>,
        }

        impl crate::SimulationObserver for MockObserver {
            fn on_step_end(&self, _cycles: u32, _registers: &[u32]) {}
            fn on_memory_write(&self, addr: u64, old: u8, new: u8) {
                self.writes.lock().unwrap().push((addr, old, new));
            }
        }

        let writes = Arc::new(Mutex::new(Vec::new()));
        let mut bus = SystemBus::new();
        bus.observers.push(Arc::new(MockObserver {
            writes: writes.clone(),
        }));

        // Write to RAM (e.g., 0x20000000)
        bus.write_u8(0x20000000, 0xAA).unwrap();
        {
            let w = writes.lock().unwrap();
            assert_eq!(w.len(), 1);
            assert_eq!(w[0], (0x20000000, 0, 0xAA));
        }

        // Write to Peripheral (e.g., UART at 0x4000C000)
        bus.write_u8(0x4000C000, 0xBB).unwrap();
        {
            let w = writes.lock().unwrap();
            assert_eq!(w.len(), 2);
            assert_eq!(w[1], (0x4000C000, 0xC0, 0xBB));
        }
    }

    #[test]
    fn test_flash_boot_alias_read_and_write() {
        let mut bus = SystemBus {
            flash: LinearMemory::new(256, 0x0800_0000),
            ram: LinearMemory::new(256, 0x2000_0000),
            extra_mem: Vec::new(),
            peripherals: Vec::new(),
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            clock_gating_bypass: false,
            fault_unclocked: std::collections::HashMap::new(),
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            legacy_tick_indices: Vec::new(),
            bus_tick_indices: Vec::new(),
            scheduler_driver_indices: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_route: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            cycle_clock: crate::CycleClock::default(),
            pending_schedule: Vec::new(),
            freerunning_timer_poll_mmio: std::cell::Cell::new(0),
            side_effecting_mmio: std::cell::Cell::new(0),
            legacy_walk_disabled: false,
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            hcsr04: Vec::new(),
            tm1637: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            can_log_players: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
            esp32c3_system_idx: None,
            esp32c3_interrupt_core0_idx: None,
            esp32c3_irq_cache: None,
            esp32c3_asserted_sources: [0; 2],
            esp32c3_sched_asserted_sources: [0; 2],
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };

        bus.flash.write_u8(0x0800_0000, 0x12);
        bus.flash.write_u8(0x0800_0001, 0x34);

        // Read through aliased 0x0000_0000 boot window.
        assert_eq!(bus.read_u8(0x0000_0000).unwrap(), 0x12);
        assert_eq!(bus.read_u8(0x0000_0001).unwrap(), 0x34);

        // Write through alias and verify backing flash changed.
        bus.write_u8(0x0000_0001, 0xAB).unwrap();
        assert_eq!(bus.flash.read_u8(0x0800_0001), Some(0xAB));
    }

    /// Build a bus with a 1 KiB flash region (erased to 0xFF, like real silicon
    /// after erase) and an H5 FLASH register peripheral at 0x4002_2000, with the
    /// opt-in program-error gate set to `gate`.
    fn h5_flash_bus(gate: bool) -> SystemBus {
        let mut flash = LinearMemory::new(0x400, 0x0800_0000);
        // Erased state is all-ones; the gate's not-erased check keys off this.
        flash.data.iter_mut().for_each(|b| *b = 0xFF);
        let mut bus = SystemBus {
            flash,
            ram: LinearMemory::new(256, 0x2000_0000),
            extra_mem: Vec::new(),
            peripherals: vec![PeripheralEntry {
                name: "flash".to_string(),
                base: 0x4002_2000,
                size: 0x400,
                irq: None,
                dev: Box::new(
                    crate::peripherals::flash::Flash::new_with_layout(
                        crate::peripherals::flash::FlashRegisterLayout::Stm32H5,
                    )
                    .with_error_flags(gate),
                ),
                ticks_remaining: 0,
                generation: 0,
                clock_gate: None,
            }],
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: false,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            clock_gating_bypass: false,
            fault_unclocked: std::collections::HashMap::new(),
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            legacy_tick_indices: Vec::new(),
            bus_tick_indices: Vec::new(),
            scheduler_driver_indices: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_route: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            cycle_clock: crate::CycleClock::default(),
            pending_schedule: Vec::new(),
            freerunning_timer_poll_mmio: std::cell::Cell::new(0),
            side_effecting_mmio: std::cell::Cell::new(0),
            legacy_walk_disabled: false,
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            hcsr04: Vec::new(),
            tm1637: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            can_log_players: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
            esp32c3_system_idx: None,
            esp32c3_interrupt_core0_idx: None,
            esp32c3_irq_cache: None,
            esp32c3_asserted_sources: [0; 2],
            esp32c3_sched_asserted_sources: [0; 2],
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };
        bus.rebuild_peripheral_ranges();
        bus
    }

    fn read_nssr(bus: &SystemBus) -> u32 {
        use crate::peripherals::flash::h5::NSSR_OFF;
        bus.read_u32(0x4002_2000 + NSSR_OFF).unwrap()
    }

    /// Enable NSCR.PG on the H5 FLASH peripheral so the write-buffer machine
    /// programs (silicon requires PG for a flash-region write to land).
    fn h5_set_pg(bus: &mut SystemBus) {
        use crate::peripherals::flash::h5;
        bus.write_u32(0x4002_2000 + h5::NSCR_OFF, h5::NSCR_PG)
            .unwrap();
    }

    #[test]
    fn h5_gate_on_full_quadword_commits_as_and() {
        use crate::peripherals::flash::h5;
        let mut bus = h5_flash_bus(true);
        assert!(bus.flash_error_flags_idx.is_some(), "gate index cached");
        h5_set_pg(&mut bus);
        // Pre-load the quad-word at 0x08000020 with 0xAA in the first lane so the
        // commit must AND with it (flash only flips 1→0). Write the lower 15
        // lanes via the buffer first... but to exercise the AND we re-program a
        // committed quad-word below; here verify a clean commit from erased.
        for i in 0..16u64 {
            bus.write_u8(0x0800_0020 + i, 0x33).unwrap();
        }
        // 0xFF (erased) & 0x33 = 0x33 — full quad-word committed.
        for i in 0..16u64 {
            assert_eq!(bus.flash.read_u8(0x0800_0020 + i), Some(0x33));
        }
        assert_ne!(read_nssr(&bus) & h5::NSSR_EOP, 0, "EOP set on commit");
        assert_eq!(read_nssr(&bus) & h5::NSSR_WBNE, 0, "WBNE clear on commit");
    }

    #[test]
    fn h5_gate_on_partial_quadword_buffers_no_commit() {
        use crate::peripherals::flash::h5;
        let mut bus = h5_flash_bus(true);
        h5_set_pg(&mut bus);
        // Only 4 of 16 bytes: still buffering, flash unchanged, WBNE set.
        for i in 0..4u64 {
            bus.write_u8(0x0800_0020 + i, 0x55).unwrap();
            assert_eq!(bus.flash.read_u8(0x0800_0020 + i), Some(0xFF), "not yet");
        }
        assert_ne!(read_nssr(&bus) & h5::NSSR_WBNE, 0, "WBNE set");
        assert_eq!(read_nssr(&bus) & h5::NSSR_EOP, 0, "no EOP");
    }

    #[test]
    fn h5_gate_on_reprogram_committed_quadword_ands_no_pgserr() {
        use crate::peripherals::flash::h5;
        let mut bus = h5_flash_bus(true);
        h5_set_pg(&mut bus);
        // First program: 0xFF & 0xF0 = 0xF0.
        for i in 0..16u64 {
            bus.write_u8(0x0800_0040 + i, 0xF0).unwrap();
        }
        assert_eq!(bus.flash.read_u8(0x0800_0040), Some(0xF0));
        // Clear EOP via NSCCR, then re-program the SAME (now-not-erased) word.
        bus.write_u32(0x4002_2000 + h5::NSCCR_OFF, h5::NSSR_EOP)
            .unwrap();
        for i in 0..16u64 {
            bus.write_u8(0x0800_0040 + i, 0x0F).unwrap();
        }
        // Re-program ALLOWED, result is the AND: 0xF0 & 0x0F = 0x00. No PGSERR.
        assert_eq!(bus.flash.read_u8(0x0800_0040), Some(0x00), "AND of old&new");
        assert_eq!(read_nssr(&bus) & h5::NSSR_PGSERR, 0, "no PGSERR over-write");
        assert_ne!(read_nssr(&bus) & h5::NSSR_EOP, 0, "EOP set (success)");
    }

    #[test]
    fn h5_gate_on_misaligned_run_sets_incerr_alone_no_commit() {
        use crate::peripherals::flash::h5;
        let mut bus = h5_flash_bus(true);
        h5_set_pg(&mut bus);
        // Start at base+4 (quad-word 0x20), then jump into the next quad-word
        // (0x30) before completing — an inconsistent program run.
        bus.write_u8(0x0800_0024, 0x11).unwrap();
        assert_ne!(read_nssr(&bus) & h5::NSSR_WBNE, 0, "WBNE while partial");
        bus.write_u8(0x0800_0030, 0x22).unwrap();
        // INCERR alone, nothing committed (both targets stay erased).
        assert_eq!(bus.flash.read_u8(0x0800_0024), Some(0xFF), "no commit");
        assert_eq!(bus.flash.read_u8(0x0800_0030), Some(0xFF), "no commit");
        let nssr = read_nssr(&bus);
        assert_ne!(nssr & h5::NSSR_INCERR, 0, "INCERR set");
        assert_eq!(nssr & h5::NSSR_PGSERR, 0, "INCERR alone (no PGSERR)");
    }

    #[test]
    fn h5_gate_off_commits_every_program_with_no_flag() {
        use crate::peripherals::flash::h5;
        let mut bus = h5_flash_bus(false);
        assert!(bus.flash_error_flags_idx.is_none(), "gate off ⇒ no index");
        // No buffering, no flags: every byte commits straight through, even
        // misaligned and over-not-erased (old byte-identical behaviour).
        bus.write_u8(0x0800_0003, 0x42).unwrap();
        assert_eq!(bus.flash.read_u8(0x0800_0003), Some(0x42));
        bus.write_u8(0x0800_0003, 0x99).unwrap();
        assert_eq!(bus.flash.read_u8(0x0800_0003), Some(0x99));
        assert_eq!(read_nssr(&bus) & h5::NSSR_W1C_MASK, 0, "no flag ever");
        assert_eq!(read_nssr(&bus) & h5::NSSR_WBNE, 0, "no WBNE ever");
    }

    // ── H5 read-while-write fidelity gate (opt-in, default off) ─────────────

    use crate::Cpu as _RwwCpuTrait;

    /// Minimal CPU stub with a settable PC for the RWW Machine-level tests.
    /// `step` is a no-op (the tests drive `apply_pending_flash_op` directly via
    /// a manually recorded erase, so the CPU never needs to execute).
    #[derive(Default)]
    struct PcCpu {
        pc: u32,
    }

    impl crate::Cpu for PcCpu {
        fn reset(&mut self, _bus: &mut dyn crate::Bus) -> crate::SimResult<()> {
            Ok(())
        }
        fn step(
            &mut self,
            _bus: &mut dyn crate::Bus,
            _observers: &[std::sync::Arc<dyn crate::SimulationObserver>],
            _config: &crate::SimulationConfig,
        ) -> crate::SimResult<()> {
            Ok(())
        }
        fn set_pc(&mut self, val: u32) {
            self.pc = val;
        }
        fn get_pc(&self) -> u32 {
            self.pc
        }
        fn set_sp(&mut self, _val: u32) {}
        fn set_exception_pending(&mut self, _n: u32) {}
        fn get_register(&self, _id: u8) -> u32 {
            0
        }
        fn set_register(&mut self, _id: u8, _val: u32) {}
        fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
            crate::snapshot::CpuSnapshot::Arm(crate::snapshot::ArmCpuSnapshot {
                registers: vec![0; 16],
                pc: self.pc,
                xpsr: 0,
                primask: false,
                pending_exceptions: 0,
                pending_exceptions_hi: Vec::new(),
                vtor: 0,
            })
        }
        fn apply_snapshot(&mut self, _snapshot: &crate::snapshot::CpuSnapshot) {}
        fn get_register_names(&self) -> Vec<String> {
            vec![]
        }
        fn index_of_register(&self, _name: &str) -> Option<u8> {
            None
        }
    }

    /// Build a bus with a 2 MiB flash region (two 1 MiB banks, as on the H563)
    /// and an H5 FLASH register peripheral, with the opt-in read-while-write gate
    /// set to `gate`. The flash is unlocked so a NSCR.SER|STRT write records an
    /// erase op straight away.
    fn h5_rww_bus(gate: bool) -> SystemBus {
        use crate::peripherals::flash::h5;
        let mut flash = LinearMemory::new((2 * h5::BANK_SIZE) as usize, h5::FLASH_BASE);
        flash.data.iter_mut().for_each(|b| *b = 0xFF);
        let mut bus = SystemBus {
            flash,
            ram: LinearMemory::new(0x1000, 0x2000_0000),
            extra_mem: Vec::new(),
            peripherals: vec![PeripheralEntry {
                name: "flash".to_string(),
                base: 0x4002_2000,
                size: 0x400,
                irq: None,
                dev: Box::new(
                    crate::peripherals::flash::Flash::new_with_layout(
                        crate::peripherals::flash::FlashRegisterLayout::Stm32H5,
                    )
                    .with_read_while_write(gate),
                ),
                ticks_remaining: 0,
                generation: 0,
                clock_gate: None,
            }],
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: false,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            clock_gating_bypass: false,
            fault_unclocked: std::collections::HashMap::new(),
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            legacy_tick_indices: Vec::new(),
            bus_tick_indices: Vec::new(),
            scheduler_driver_indices: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_route: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            cycle_clock: crate::CycleClock::default(),
            pending_schedule: Vec::new(),
            freerunning_timer_poll_mmio: std::cell::Cell::new(0),
            side_effecting_mmio: std::cell::Cell::new(0),
            legacy_walk_disabled: false,
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            hcsr04: Vec::new(),
            tm1637: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            can_log_players: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
            esp32c3_system_idx: None,
            esp32c3_interrupt_core0_idx: None,
            esp32c3_irq_cache: None,
            esp32c3_asserted_sources: [0; 2],
            esp32c3_sched_asserted_sources: [0; 2],
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };
        bus.rebuild_peripheral_ranges();
        bus
    }

    /// Unlock NSKEYR then record a sector erase of `bank` (BKSEL logical) on the
    /// bus, so a subsequent `apply_pending_flash_op` drains it.
    fn h5_record_erase(bus: &mut SystemBus, bank: u8, sector: u32) {
        use crate::peripherals::flash::h5;
        bus.write_u32(0x4002_2000 + h5::NSKEYR_OFF, 0x4567_0123)
            .unwrap();
        bus.write_u32(0x4002_2000 + h5::NSKEYR_OFF, 0xCDEF_89AB)
            .unwrap();
        let mut nscr = h5::NSCR_SER | (sector << h5::NSCR_SNB_SHIFT) | h5::NSCR_STRT;
        if bank == 1 {
            nscr |= h5::NSCR_BKSEL;
        }
        bus.write_u32(0x4002_2000 + h5::NSCR_OFF, nscr).unwrap();
    }

    #[test]
    fn rww_gate_on_same_bank_erase_faults() {
        use crate::peripherals::flash::h5;
        let mut cpu = PcCpu::default();
        // PC executing from bank 1 (boot view at 0x08000000), sector 11.
        cpu.set_pc(0x0801_6000);
        let mut bus = h5_rww_bus(true);
        h5_record_erase(&mut bus, 0, 11); // erase bank 1 (BKSEL=0), sector 11
        let mut machine = crate::Machine::new(cpu, bus);
        let err = machine
            .apply_pending_flash_op()
            .expect_err("same-bank erase under the RWW gate must fault");
        match err {
            crate::SimulationError::Other(msg) => {
                assert!(msg.contains("RWW"), "reason names the RWW violation: {msg}");
                assert!(
                    msg.contains("SRAM"),
                    "reason tells firmware to use SRAM: {msg}"
                );
            }
            other => panic!("expected SimulationError::Other, got {other:?}"),
        }
        // Faulted before the fill: the erased sector is NOT cleared to 0xFF by us
        // (it was already 0xFF), but more importantly the op did not silently
        // "succeed" — the error propagated.
        let _ = h5::BANK_SIZE;
    }

    #[test]
    fn rww_gate_on_other_bank_erase_proceeds() {
        use crate::peripherals::flash::h5;
        let mut cpu = PcCpu::default();
        // PC in bank 1; erase targets bank 2 — the normal cross-bank OTA case.
        cpu.set_pc(0x0801_6000);
        let mut bus = h5_rww_bus(true);
        // Dirty the bank-2 boot-state sector so we can see the erase land.
        let off = h5::BANK_SIZE + 11 * h5::SECTOR_SIZE;
        bus.flash.write_u8(h5::FLASH_BASE + off, 0x00);
        h5_record_erase(&mut bus, 1, 11); // erase bank 2 (BKSEL=1)
        let mut machine = crate::Machine::new(cpu, bus);
        machine
            .apply_pending_flash_op()
            .expect("cross-bank erase must proceed");
        assert_eq!(
            machine.bus.flash.read_u8(h5::FLASH_BASE + off),
            Some(0xFF),
            "bank-2 sector erased to 0xFF"
        );
    }

    #[test]
    fn rww_gate_on_pc_in_sram_never_faults() {
        // The intended production layout: the flash routine runs from SRAM, so
        // PC is not in any flash bank — even a same-(logical-)bank erase is fine.
        use crate::peripherals::flash::h5;
        let mut cpu = PcCpu::default();
        cpu.set_pc(0x2000_0100); // SRAM
        let mut bus = h5_rww_bus(true);
        h5_record_erase(&mut bus, 0, 11);
        let mut machine = crate::Machine::new(cpu, bus);
        machine
            .apply_pending_flash_op()
            .expect("erase from a SRAM-resident routine must proceed");
        let _ = h5::FLASH_BASE;
    }

    #[test]
    fn rww_gate_on_respects_swap_bank_mapping() {
        // After a SWAP_BANK, the physical second bank answers at 0x08000000.
        // PC at 0x08000000 is then in physical bank 2; an erase that lands in
        // that physical bank (BKSEL=0, which now maps to physical bank 2) must
        // fault, while BKSEL=1 (physical bank 1, the inactive one) proceeds.
        use crate::peripherals::flash::h5;

        // Same-physical-bank under swap → fault.
        {
            let mut cpu = PcCpu::default();
            cpu.set_pc(0x0800_4000); // bank presented at 0x08000000
            let mut bus = h5_rww_bus(true);
            // Toggle the FLASH's swap state directly to model an applied swap.
            let idx = bus.find_peripheral_index_by_name("flash").unwrap();
            bus.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<crate::peripherals::flash::Flash>())
                .unwrap()
                .mark_swapped();
            h5_record_erase(&mut bus, 0, 2); // BKSEL=0 → physical bank 2 under swap
            let mut machine = crate::Machine::new(cpu, bus);
            let err = machine
                .apply_pending_flash_op()
                .expect_err("swapped: BKSEL=0 erase hits PC's physical bank");
            assert!(matches!(err, crate::SimulationError::Other(_)));
        }

        // Cross-physical-bank under swap → proceeds.
        {
            let mut cpu = PcCpu::default();
            cpu.set_pc(0x0800_4000);
            let mut bus = h5_rww_bus(true);
            let idx = bus.find_peripheral_index_by_name("flash").unwrap();
            bus.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<crate::peripherals::flash::Flash>())
                .unwrap()
                .mark_swapped();
            // BKSEL=1 → physical bank 1, which sits at buffer offset 0..1 MiB?
            // No: under swap, logical bank 1 maps to physical bank 0, the bank
            // NOT presented at 0x08000000 — the cross-bank case.
            let off = h5::BANK_SIZE + 2 * h5::SECTOR_SIZE;
            bus.flash.write_u8(h5::FLASH_BASE + off, 0x00);
            h5_record_erase(&mut bus, 1, 2);
            let mut machine = crate::Machine::new(cpu, bus);
            machine
                .apply_pending_flash_op()
                .expect("swapped: cross-physical-bank erase proceeds");
        }
    }

    #[test]
    fn rww_gate_off_same_bank_erase_succeeds_silently() {
        // Default behaviour (gate off): a same-bank erase succeeds, byte-
        // identical to before this gate existed.
        use crate::peripherals::flash::h5;
        let mut cpu = PcCpu::default();
        cpu.set_pc(0x0801_6000);
        let mut bus = h5_rww_bus(false);
        let off = 11 * h5::SECTOR_SIZE;
        bus.flash.write_u8(h5::FLASH_BASE + off, 0x00);
        h5_record_erase(&mut bus, 0, 11);
        let mut machine = crate::Machine::new(cpu, bus);
        machine
            .apply_pending_flash_op()
            .expect("gate off: same-bank erase succeeds");
        assert_eq!(
            machine.bus.flash.read_u8(h5::FLASH_BASE + off),
            Some(0xFF),
            "gate off: sector erased to 0xFF as before"
        );
    }

    #[test]
    fn test_peripheral_range_index_lookup() {
        let mut bus = SystemBus {
            flash: LinearMemory::new(256, 0x0800_0000),
            ram: LinearMemory::new(256, 0x2000_0000),
            extra_mem: Vec::new(),
            peripherals: vec![
                PeripheralEntry {
                    name: "high".to_string(),
                    base: 0x5000_0000,
                    size: 0x1000,
                    irq: None,
                    dev: Box::new(crate::peripherals::uart::Uart::new()),
                    ticks_remaining: 0,
                    generation: 0,
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "low".to_string(),
                    base: 0x4000_0000,
                    size: 0x1000,
                    irq: None,
                    dev: Box::new(crate::peripherals::uart::Uart::new()),
                    ticks_remaining: 0,
                    generation: 0,
                    clock_gate: None,
                },
            ],
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            clock_gating_bypass: false,
            fault_unclocked: std::collections::HashMap::new(),
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            legacy_tick_indices: Vec::new(),
            bus_tick_indices: Vec::new(),
            scheduler_driver_indices: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_route: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            cycle_clock: crate::CycleClock::default(),
            pending_schedule: Vec::new(),
            freerunning_timer_poll_mmio: std::cell::Cell::new(0),
            side_effecting_mmio: std::cell::Cell::new(0),
            legacy_walk_disabled: false,
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            hcsr04: Vec::new(),
            tm1637: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            can_log_players: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
            esp32c3_system_idx: None,
            esp32c3_interrupt_core0_idx: None,
            esp32c3_irq_cache: None,
            esp32c3_asserted_sources: [0; 2],
            esp32c3_sched_asserted_sources: [0; 2],
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };

        bus.rebuild_peripheral_ranges();
        let low_idx = bus.find_peripheral_index(0x4000_0004);
        let high_idx = bus.find_peripheral_index(0x5000_0004);

        assert_eq!(low_idx, Some(1));
        assert_eq!(high_idx, Some(0));
    }

    #[test]
    fn test_execute_dma_copy_request() {
        let mut bus = SystemBus::new();
        bus.write_u8(0x2000_0010, 0xAB).unwrap();
        bus.write_u8(0x2000_0020, 0x00).unwrap();

        let req = crate::DmaRequest {
            src_addr: 0x2000_0010,
            addr: 0x2000_0020,
            val: 0,
            direction: crate::DmaDirection::Copy,
            transform: None,
        };
        bus.execute_dma(&[req]).unwrap();

        assert_eq!(bus.read_u8(0x2000_0020).unwrap(), 0xAB);
    }

    #[test]
    fn test_dma_tick_executes_copy_and_raises_irq() {
        let mut bus = SystemBus {
            flash: LinearMemory::new(256, 0x0800_0000),
            ram: LinearMemory::new(256, 0x2000_0000),
            extra_mem: Vec::new(),
            peripherals: vec![PeripheralEntry {
                name: "dma1".to_string(),
                base: 0x4002_0000,
                size: 0x400,
                irq: Some(16),
                dev: Box::new(crate::peripherals::dma::Dma1::new()),
                ticks_remaining: 0,
                generation: 0,
                clock_gate: None,
            }],
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            clock_gating_bypass: false,
            fault_unclocked: std::collections::HashMap::new(),
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            legacy_tick_indices: Vec::new(),
            bus_tick_indices: Vec::new(),
            scheduler_driver_indices: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_route: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            cycle_clock: crate::CycleClock::default(),
            pending_schedule: Vec::new(),
            freerunning_timer_poll_mmio: std::cell::Cell::new(0),
            side_effecting_mmio: std::cell::Cell::new(0),
            legacy_walk_disabled: false,
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            hcsr04: Vec::new(),
            tm1637: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            can_log_players: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
            esp32c3_system_idx: None,
            esp32c3_interrupt_core0_idx: None,
            esp32c3_irq_cache: None,
            esp32c3_asserted_sources: [0; 2],
            esp32c3_sched_asserted_sources: [0; 2],
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };
        bus.rebuild_peripheral_ranges();

        // Per STM32 RM mem-to-mem semantics: data flows CMAR -> CPAR
        // (CMAR is the source, CPAR is the destination). Set up source
        // at SRC_ADDR via CMAR; expect destination at DST_ADDR (CPAR).
        const SRC_ADDR: u64 = 0x2000_0010;
        const DST_ADDR: u64 = 0x2000_0020;
        bus.write_u8(SRC_ADDR, 0x5A).unwrap();
        bus.write_u8(DST_ADDR, 0x00).unwrap();

        // Program DMA1 Channel1:
        //   CMAR (source) = SRC_ADDR
        //   CPAR (destination) = DST_ADDR
        //   CNDTR = 1, CCR = EN | TCIE | PINC | MINC | DIR | MEM2MEM
        bus.write_u32(0x4002_0014, SRC_ADDR as u32).unwrap(); // CMAR1
        bus.write_u32(0x4002_0010, DST_ADDR as u32).unwrap(); // CPAR1
        bus.write_u32(0x4002_000C, 1).unwrap(); // CNDTR1
        bus.write_u32(
            0x4002_0008,
            (1 << 0) | (1 << 1) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 14),
        )
        .unwrap(); // CCR1 (EN | TCIE | DIR | PINC | MINC | MEM2MEM)

        let (interrupts, _costs) = bus.tick_peripherals_fully();
        assert_eq!(
            bus.read_u8(DST_ADDR).unwrap(),
            0x5A,
            "DST should hold the SRC byte after mem-to-mem copy"
        );
        assert!(interrupts.contains(&16), "TCIE should pend NVIC IRQ 16");
    }

    /// RCC clock-gating (silicon fidelity): a peripheral with a declared
    /// `clock:` gate is inert until its RCC enable bit is set — writes are
    /// dropped and reads return 0 — and behaves normally once clocked. The
    /// reg-name → offset mapping is family-aware (F1 apb2enr @ 0x18).
    #[test]
    fn gated_peripheral_is_inert_until_rcc_bit_set() {
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: "f1-clockgate-test"
arch: "arm"
core: "cortex-m3"
flash:
  base: 0x08000000
  size: "64KB"
ram:
  base: 0x20000000
  size: "20KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    clock: { reg: "apb2enr", bit: 14 }
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
"#,
        )
        .unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "clockgate"
chip: "unused"
external_devices: []
board_io: []
"#,
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();

        // USART1_CR1 @ 0x4001_380C. Clock is OFF out of reset → the write is
        // dropped and the register reads back 0 (an unclocked peripheral).
        const CR1: u64 = 0x4001_380C;
        const CR1_UE_TE: u32 = (1 << 13) | (1 << 3);
        bus.write_u32(CR1, CR1_UE_TE).unwrap();
        assert_eq!(
            bus.read_u32(CR1).unwrap(),
            0,
            "unclocked USART1 must drop writes and read 0"
        );

        // The ungated uart2 (no clock declared) is unaffected — accessible now.
        const UART2_CR1: u64 = 0x4000_440C;
        bus.write_u32(UART2_CR1, CR1_UE_TE).unwrap();
        assert_eq!(
            bus.read_u32(UART2_CR1).unwrap() & CR1_UE_TE,
            CR1_UE_TE,
            "ungated uart2 must work regardless of RCC"
        );

        // Enable RCC_APB2ENR.USART1EN (bit 14). RCC itself is never gated.
        const RCC_APB2ENR: u64 = 0x4002_1018;
        bus.write_u32(RCC_APB2ENR, 1 << 14).unwrap();
        assert_eq!(bus.read_u32(RCC_APB2ENR).unwrap() & (1 << 14), 1 << 14);

        // Now USART1 is clocked: the same write takes effect and reads back.
        bus.write_u32(CR1, CR1_UE_TE).unwrap();
        assert_eq!(
            bus.read_u32(CR1).unwrap() & CR1_UE_TE,
            CR1_UE_TE,
            "clocked USART1 must accept writes"
        );

        // Drop the clock again → the peripheral goes inert (reads 0).
        bus.write_u32(RCC_APB2ENR, 0).unwrap();
        assert_eq!(
            bus.read_u32(CR1).unwrap(),
            0,
            "USART1 must go inert again when its clock is removed"
        );
    }

    #[test]
    fn gated_peripheral_resolves_l4_rcc_offsets() {
        // The SAME symbolic reg names that map to F1 offsets above must resolve
        // to the L4 family's offsets via Rcc::enable_reg_offset: apb1enr1 @ 0x58
        // (not F1's 0x1C) and ahb2enr @ 0x4C. Mirrors the iolink-dido (USART2 on
        // apb1enr1) and nokia5110 (GPIOA on ahb2enr) gates on the L476.
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: "l4-clockgate-test"
arch: "arm"
core: "cortex-m4"
flash:
  base: 0x08000000
  size: "1MB"
ram:
  base: 0x20000000
  size: "96KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
    config:
      profile: "stm32l4"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x48000000
    size: "1KB"
    config:
      profile: "stm32v2"
    clock: { reg: "ahb2enr", bit: 0 }
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    config:
      profile: "stm32v2"
    clock: { reg: "apb1enr1", bit: 17 }
"#,
        )
        .unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "clockgate-l4"
chip: "unused"
external_devices: []
board_io: []
"#,
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();

        // USART2_CR1 @ 0x4000_4400 (stm32v2 layout: CR1 at offset 0x00).
        // Clock OFF out of reset.
        const U2_CR1: u64 = 0x4000_4400;
        const CR1_UE_TE: u32 = (1 << 0) | (1 << 3);
        bus.write_u32(U2_CR1, CR1_UE_TE).unwrap();
        assert_eq!(
            bus.read_u32(U2_CR1).unwrap(),
            0,
            "unclocked USART2 must drop writes and read 0"
        );

        // RCC_APB1ENR1 @ 0x58 (L4 offset, NOT the F1 0x1C). USART2EN = bit 17.
        const RCC_APB1ENR1: u64 = 0x4002_1058;
        bus.write_u32(RCC_APB1ENR1, 1 << 17).unwrap();
        bus.write_u32(U2_CR1, CR1_UE_TE).unwrap();
        assert_eq!(
            bus.read_u32(U2_CR1).unwrap() & CR1_UE_TE,
            CR1_UE_TE,
            "clocked USART2 must accept writes once apb1enr1.17 is set"
        );

        // GPIOA_MODER @ 0x4800_0000, gated on RCC_AHB2ENR @ 0x4C bit 0.
        const GPIOA_MODER: u64 = 0x4800_0000;
        bus.write_u32(GPIOA_MODER, 0x55).unwrap();
        assert_eq!(
            bus.read_u32(GPIOA_MODER).unwrap(),
            0,
            "unclocked GPIOA must drop writes and read 0"
        );
        const RCC_AHB2ENR: u64 = 0x4002_104C;
        bus.write_u32(RCC_AHB2ENR, 1 << 0).unwrap();
        bus.write_u32(GPIOA_MODER, 0x55).unwrap();
        assert_eq!(
            bus.read_u32(GPIOA_MODER).unwrap() & 0x55,
            0x55,
            "clocked GPIOA must accept writes once ahb2enr.0 is set"
        );
    }

    // -----------------------------------------------------------------------
    // Script-driven FSM tests
    // -----------------------------------------------------------------------

    /// Core helper: build a bus with a bxCAN in normal mode (filter accepts
    /// 0x111) and attach a UDS tester loaded with the given steps. Returns
    /// the bus after the first service tick so the tester has already sent its
    /// initial SF/FF and is in `AwaitResp` (or `AwaitFc` for a multi-frame
    /// request).
    fn bus_with_steps(script: Vec<UdsStep>) -> SystemBus {
        use crate::peripherals::bxcan::BxCan;
        const MCR: u64 = 0x000;
        const BTR: u64 = 0x01C;
        const FMR: u64 = 0x200;
        const FM1R: u64 = 0x204;
        const FS1R: u64 = 0x20C;
        const FFA1R: u64 = 0x214;
        const FA1R: u64 = 0x21C;
        const FBANK: u64 = 0x240;
        const VALID_BTR: u32 = 0x00DC_0009;
        const BASE: u64 = 0x4000_6400;

        let mut bus = SystemBus::empty();
        bus.add_peripheral("bxcan1", BASE, 0x400, None, Box::new(BxCan::new()));

        bus.write_u32(BASE + MCR, 1).unwrap();
        bus.write_u32(BASE + BTR, VALID_BTR).unwrap();
        bus.write_u32(BASE + FMR, 1).unwrap();
        bus.write_u32(BASE + FS1R, 0x1).unwrap();
        bus.write_u32(BASE + FM1R, 0x0).unwrap();
        bus.write_u32(BASE + FFA1R, 0x0).unwrap();
        bus.write_u32(BASE + FBANK, (0x111u32) << 21).unwrap();
        bus.write_u32(BASE + FBANK + 4, (0x111u32) << 21).unwrap();
        bus.write_u32(BASE + FA1R, 0x1).unwrap();
        bus.write_u32(BASE + FMR, 0x0).unwrap();
        bus.write_u32(BASE + MCR, 0).unwrap();

        let mut tester = CanUdsTester::new("uds".into(), "bxcan1".into());
        tester.script = script;
        bus.can_uds_testers.push(tester);
        bus.service_can_uds_testers();
        bus
    }

    /// Convenience wrapper: build a bus from `(send_hex, expect_hex)` tuples.
    /// Each step is parsed the same way as the YAML config.
    fn bus_with_script(steps: &[(&str, &str)]) -> SystemBus {
        let script: Vec<UdsStep> = steps
            .iter()
            .map(|(send_str, expect_str)| UdsStep {
                send: SystemBus::yaml_bytes(
                    Some(&serde_yaml::Value::String(send_str.to_string())),
                    &[],
                ),
                expect: SystemBus::parse_expect(expect_str),
                expect_nrc: None,
            })
            .collect();
        bus_with_steps(script)
    }

    /// Push a simulated ECU frame into the connected bxCAN's `tx_frames` so
    /// the next `service_can_uds_testers` call drains and processes it.
    fn inject_ecu_reply(bus: &mut SystemBus, id: u32, data: &[u8]) {
        use crate::peripherals::bxcan::BxCan;
        let idx = bus
            .find_peripheral_index_by_name("bxcan1")
            .expect("bxcan1 must be registered");
        let bx = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<BxCan>()
            .expect("bxcan1 must be BxCan");
        bx.tx_frames
            .push_back(crate::network::CanFrame::classic(id, data.to_vec()));
    }

    /// Same construction idiom as `bus_with_steps`: a bare `SystemBus` with a
    /// single `bxcan1` `BxCan`, taken out of INIT with a filter configured —
    /// here a wide (32-bit) mask filter with id=0/mask=0, which accepts every
    /// frame (standard or extended), so `CanLogPlayer` replay isn't gated by
    /// filter setup unrelated to this test.
    fn bus_with_open_bxcan() -> SystemBus {
        use crate::peripherals::bxcan::BxCan;
        const MCR: u64 = 0x000;
        const BTR: u64 = 0x01C;
        const FMR: u64 = 0x200;
        const FM1R: u64 = 0x204;
        const FS1R: u64 = 0x20C;
        const FFA1R: u64 = 0x214;
        const FA1R: u64 = 0x21C;
        const FBANK: u64 = 0x240;
        const VALID_BTR: u32 = 0x00DC_0009;
        const BASE: u64 = 0x4000_6400;

        let mut bus = SystemBus::empty();
        bus.add_peripheral("bxcan1", BASE, 0x400, None, Box::new(BxCan::new()));

        bus.write_u32(BASE + MCR, 1).unwrap();
        bus.write_u32(BASE + BTR, VALID_BTR).unwrap();
        bus.write_u32(BASE + FMR, 1).unwrap();
        bus.write_u32(BASE + FS1R, 0x1).unwrap(); // wide (32-bit) filter
        bus.write_u32(BASE + FM1R, 0x0).unwrap(); // mask mode (not list)
        bus.write_u32(BASE + FFA1R, 0x0).unwrap();
        bus.write_u32(BASE + FBANK, 0).unwrap(); // id = 0
        bus.write_u32(BASE + FBANK + 4, 0).unwrap(); // mask = 0 -> accept all
        bus.write_u32(BASE + FA1R, 0x1).unwrap();
        bus.write_u32(BASE + FMR, 0x0).unwrap();
        bus.write_u32(BASE + MCR, 0).unwrap();
        bus
    }

    #[test]
    fn can_log_player_delivers_frames_at_scheduled_ticks() {
        // Two frames 100µs apart at 1M ticks/sec => ticks 0 and 100.
        let log = "(10.000000) can0 0CF00300#DD0000FFFFFF5CFF\n\
                   (10.000100) can0 18FEF100#0102030405060708\n";
        let mut bus = bus_with_open_bxcan();
        let player =
            CanLogPlayer::from_candump("p".into(), "bxcan1".into(), log, 1_000_000).unwrap();
        bus.can_log_players.push(player);

        // Tick 1: first frame (tick 0 is due immediately).
        bus.service_can_log_players();
        assert_eq!(bus.can_log_players[0].delivered, 1);
        // Ticks 2..=99: nothing new.
        for _ in 0..98 {
            bus.service_can_log_players();
        }
        assert_eq!(bus.can_log_players[0].delivered, 1);
        assert!(!bus.can_log_players[0].is_done());
        // Tick 100+: second frame.
        for _ in 0..3 {
            bus.service_can_log_players();
        }
        assert_eq!(bus.can_log_players[0].delivered, 2);
        assert!(bus.can_log_players[0].is_done());

        // And the bxCAN RX FIFO actually holds data: read via the same register
        // asserts the uds-tester tests use (RF0R pending count != 0).
        const RF0R: u64 = 0x00C;
        const BASE: u64 = 0x4000_6400;
        let rf0r = bus.read_u32(BASE + RF0R).unwrap();
        assert_ne!(rf0r & 0x3, 0, "RF0R FMP0 must show pending frames");
    }

    #[test]
    fn can_log_player_counts_dropped_when_filters_never_opened() {
        // Same construction idiom as `bus_with_open_bxcan`, minus the filter
        // banks — the bxCAN is taken out of INIT (so it's "running") but no
        // filter bank is ever activated (FA1R stays 0), so every delivered
        // frame is refused by acceptance filtering and must count as
        // `dropped`, never `delivered`.
        use crate::peripherals::bxcan::BxCan;
        const MCR: u64 = 0x000;
        const BTR: u64 = 0x01C;
        const VALID_BTR: u32 = 0x00DC_0009;
        const BASE: u64 = 0x4000_6400;

        let mut bus = SystemBus::empty();
        bus.add_peripheral("bxcan1", BASE, 0x400, None, Box::new(BxCan::new()));
        bus.write_u32(BASE + MCR, 1).unwrap();
        bus.write_u32(BASE + BTR, VALID_BTR).unwrap();
        bus.write_u32(BASE + MCR, 0).unwrap(); // leave INIT; filters left unconfigured

        let log = "(10.000000) can0 0CF00300#DD0000FFFFFF5CFF\n\
                   (10.000100) can0 18FEF100#0102030405060708\n";
        let player =
            CanLogPlayer::from_candump("p".into(), "bxcan1".into(), log, 1_000_000).unwrap();
        bus.can_log_players.push(player);

        for _ in 0..101 {
            bus.service_can_log_players();
        }
        assert!(bus.can_log_players[0].is_done());
        assert_eq!(bus.can_log_players[0].delivered, 0);
        assert!(bus.can_log_players[0].dropped > 0);
    }

    #[test]
    fn can_log_player_rebases_first_frame_to_tick_zero() {
        let log = "(1578925462.000450) can0 123#11\n";
        let p = CanLogPlayer::from_candump("p".into(), "bxcan1".into(), log, 1_000_000).unwrap();
        assert_eq!(p.frames[0].0, 0);
    }

    #[test]
    fn uds_tester_single_step_sf_request_matches_reply() {
        let mut bus = bus_with_script(&[("11 01", "51 01")]);
        inject_ecu_reply(&mut bus, 0x222, &[0x02, 0x51, 0x01]);
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    #[test]
    fn uds_tester_wildcard_and_multistep() {
        let mut bus = bus_with_script(&[("10 03", "50 03"), ("27 01", "67 01 ..")]);
        // bus_with_script already sent step 0 request; inject step 0 reply.
        inject_ecu_reply(&mut bus, 0x222, &[0x02, 0x50, 0x03]);
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].step_idx, 1);
        // After step 0 completes, state returns to Start. The next service call
        // sends step 1 request.
        bus.service_can_uds_testers();
        inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x67, 0x01, 0xAB]);
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    #[test]
    fn uds_tester_nrc_mismatch_fails_with_reason() {
        let mut bus = bus_with_script(&[("11 01", "51 01")]);
        // NRC response (0x7F 0x11 0x22) — does not match expected "51 01".
        inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x7F, 0x11, 0x22]);
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Failed);
        assert!(bus.can_uds_testers[0]
            .failure
            .as_ref()
            .unwrap()
            .contains("step 0"));
    }

    /// Script-path FF+1CF request: send.len() == 8 (one CF required).
    /// Verifies that the ConsecutiveFrame is injected onto the bus after the
    /// ECU's FlowControl arrives, and that the step reaches Done.
    ///
    /// This test exercises the bug fixed in this commit: before the fix,
    /// observe_ecu_frame_script set state=AwaitResp before service_can_uds_testers
    /// evaluated to_send, so the CF payload was silently discarded.
    #[test]
    fn uds_tester_script_ff_plus_one_cf_request_completes() {
        use crate::peripherals::bxcan::BxCan;

        // 8-byte payload: FF carries bytes 0..5, CF carries bytes 6..7.
        // Expected response for 0x27 service: 0x67 0x02 (single-frame).
        let mut bus = bus_with_script(&[("27 01 02 03 04 05 06 07", "67 02")]);

        // bus_with_script already ran tick 1: FF sent, state=AwaitFc.
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitFc);
        assert_eq!(
            bus.can_uds_testers[0].pending_cfs.len(),
            1,
            "one CF must be queued after FF"
        );

        // ECU responds with FlowControl (ContinueToSend).
        inject_ecu_reply(
            &mut bus,
            0x222,
            &[0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        );

        // Tick 2: tester drains the FC and injects the CF.
        bus.service_can_uds_testers();
        assert_eq!(
            bus.can_uds_testers[0].state,
            CanUdsTesterState::AwaitResp,
            "CF must be injected and state must advance to AwaitResp"
        );
        assert!(
            bus.can_uds_testers[0].pending_cfs.is_empty(),
            "pending_cfs must be drained after the only CF is sent"
        );

        // Confirm the CF actually landed in the bxCAN RX buffer (direction=rx
        // means the tester delivered it into the ECU-side FIFO).
        {
            let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
            let bx = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<BxCan>()
                .unwrap();
            let trace = bx.trace_snapshot("bxcan1");
            // trace contains all rx frames: FF (tick 1) + CF (tick 2).
            assert!(
                trace
                    .iter()
                    .any(|f| f.direction == "rx" && f.id == 0x111 && f.data.first() == Some(&0x21)),
                "CF (SN=0x21) must appear as an rx frame in the bxCAN trace"
            );
        }

        // ECU sends the positive response (single-frame: len=3, 0x67 0x02 0xAB).
        inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x67, 0x02, 0xAB]);

        // Tick 3: tester matches the response → Done.
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    /// Script-path FF+2CF request: send.len() == 14 (two CFs required).
    /// Verifies that both ConsecutiveFrames are injected on successive ticks
    /// and the step reaches Done.
    #[test]
    fn uds_tester_script_ff_plus_two_cf_request_completes() {
        use crate::peripherals::bxcan::BxCan;

        // 14-byte payload: FF carries bytes 0..5, CF1 carries 6..12, CF2 carries 13.
        // Expected response: 0x76 0x01.
        let mut bus = bus_with_script(&[("36 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D", "76 01")]);

        // Tick 1 already ran: FF sent, two CFs queued.
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitFc);
        assert_eq!(
            bus.can_uds_testers[0].pending_cfs.len(),
            2,
            "two CFs must be queued after FF"
        );

        // ECU replies with FlowControl.
        inject_ecu_reply(
            &mut bus,
            0x222,
            &[0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        );

        // Tick 2: CF1 injected; one CF still pending, state stays AwaitFc.
        bus.service_can_uds_testers();
        assert_eq!(
            bus.can_uds_testers[0].state,
            CanUdsTesterState::AwaitFc,
            "state must stay AwaitFc while CFs remain"
        );
        assert_eq!(
            bus.can_uds_testers[0].pending_cfs.len(),
            1,
            "one CF must remain after CF1 is sent"
        );

        // Tick 3: no new ECU frame; CF2 taken from pending_cfs → AwaitResp.
        bus.service_can_uds_testers();
        assert_eq!(
            bus.can_uds_testers[0].state,
            CanUdsTesterState::AwaitResp,
            "state must advance to AwaitResp after last CF is sent"
        );
        assert!(bus.can_uds_testers[0].pending_cfs.is_empty());

        // Verify both CFs appear in the trace (SN 0x21 and 0x22).
        {
            let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
            let bx = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<BxCan>()
                .unwrap();
            let trace = bx.trace_snapshot("bxcan1");
            assert!(
                trace
                    .iter()
                    .any(|f| f.direction == "rx" && f.id == 0x111 && f.data.first() == Some(&0x21)),
                "CF1 (SN=0x21) must appear as an rx frame"
            );
            assert!(
                trace
                    .iter()
                    .any(|f| f.direction == "rx" && f.id == 0x111 && f.data.first() == Some(&0x22)),
                "CF2 (SN=0x22) must appear as an rx frame"
            );
        }

        // ECU single-frame positive response.
        inject_ecu_reply(&mut bus, 0x222, &[0x02, 0x76, 0x01]);

        // Tick 4: match → Done.
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    /// 0x2E WriteDataByIdentifier: single-frame multi-byte request (7 bytes) →
    /// positive 6E echo. Covers DID-write framing the existing tests lack.
    #[test]
    fn uds_tester_did_write_sf_completes() {
        let mut bus = bus_with_script(&[("2E 01 23 DE AD BE EF", "6E 01 23")]);
        // SF header 0x03 = three payload bytes (6E 01 23); the prior 0x04 was a
        // malformed fixture (declared 4, carried 3) the lenient decoder masked.
        inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x6E, 0x01, 0x23]);
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    /// 0x31 RoutineControl: reply carries an output byte after the echo; the
    /// prefix match must accept the longer response.
    #[test]
    fn uds_tester_routine_reply_with_output_byte() {
        let mut bus = bus_with_script(&[("31 01 02 03", "71 01 02 03")]);
        inject_ecu_reply(&mut bus, 0x222, &[0x05, 0x71, 0x01, 0x02, 0x03, 0x00]);
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    /// 0x2F IOControl: shortTermAdjustment request, reply echoes DID + state.
    #[test]
    fn uds_tester_io_control_reply_completes() {
        let mut bus = bus_with_script(&[("2F A0 01 03 01", "6F A0 01")]);
        inject_ecu_reply(&mut bus, 0x222, &[0x05, 0x6F, 0xA0, 0x01, 0x03, 0x01]);
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    /// 0x19 ReadDTCInformation: a multi-frame ECU reply (FF + 1 CF) must be
    /// reassembled (AwaitResp → AwaitMultiResp → Done) and prefix-matched.
    #[test]
    fn uds_tester_dtc_read_multiframe_reply_completes() {
        let mut bus = bus_with_script(&[("19 02 09", "59 02")]);
        // FF declares 10-byte response, carries first 6 bytes (59 02 09 01 23 45).
        inject_ecu_reply(
            &mut bus,
            0x222,
            &[0x10, 0x0A, 0x59, 0x02, 0x09, 0x01, 0x23, 0x45],
        );
        bus.service_can_uds_testers(); // tester replies FlowControl, enters AwaitMultiResp
        assert_eq!(
            bus.can_uds_testers[0].state,
            CanUdsTesterState::AwaitMultiResp
        );
        // CF carries the remaining bytes; total >= 10 → complete.
        inject_ecu_reply(&mut bus, 0x222, &[0x21, 0x67, 0xAA, 0xBB, 0xCC, 0xDD]);
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    /// Multi-frame ECU response: the tester must inject a FlowControl frame onto
    /// the bxCAN bus so the ECU can send its ConsecutiveFrames.
    ///
    /// Guards the bug where the `AwaitMultiResp` arm was missing from the
    /// `to_send` match in `service_can_uds_testers`, causing the FlowControl
    /// returned by `observe_ecu_frame_script` to be silently dropped (the
    /// `_ => None` arm swallowed it).  Without the fix the ECU never receives
    /// CTS and the exchange deadlocks.
    ///
    /// The discriminating assertion is NOT the final `Done` state (the
    /// `inject_ecu_reply` shortcut bypasses that gate) but the presence of a
    /// FlowControl frame (`first_byte & 0xF0 == 0x30`) in the bxCAN RX trace
    /// after the tick that processes the ECU FirstFrame.
    #[test]
    fn uds_tester_multiframe_ecu_response_injects_flowcontrol() {
        use crate::peripherals::bxcan::BxCan;

        // Step 0: ReadDataByIdentifier 0xF190 (VIN), expect prefix 62 F1 90.
        let mut bus = bus_with_script(&[("22 F1 90", "62 F1 90")]);

        // Tick 1 already ran: the SF request (first byte 0x03) was delivered to
        // the bxCAN via deliver_rx.  Record the trace length now so we can
        // distinguish that pre-existing frame from the FlowControl we expect next.
        let trace_len_before = {
            let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
            let bx = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<BxCan>()
                .unwrap();
            bx.trace_snapshot("bxcan1").len()
        };

        // ECU replies with a FirstFrame declaring a 13-byte (0x0D) response
        // and carrying the first 6 payload bytes (62 F1 90 + 3 VIN chars).
        // 13 bytes = 6 in FF + 7 in one CF.
        inject_ecu_reply(
            &mut bus,
            0x222,
            &[0x10, 0x0D, 0x62, 0xF1, 0x90, 0x31, 0x32, 0x33],
        );

        // Tick 2: tester sees the FF, sets state=AwaitMultiResp, and MUST
        // inject a FlowControl ([0x30, 0x00, 0x00]) onto the bxCAN bus.
        bus.service_can_uds_testers();

        assert_eq!(
            bus.can_uds_testers[0].state,
            CanUdsTesterState::AwaitMultiResp,
            "state must be AwaitMultiResp after receiving ECU FirstFrame"
        );

        // Verify the FlowControl was actually delivered to the bus.
        // Only frames appended AFTER tick 1 (index >= trace_len_before) are
        // candidates; the earlier SF request frame starts with 0x03, not 0x3x.
        {
            let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
            let bx = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<BxCan>()
                .unwrap();
            let trace = bx.trace_snapshot("bxcan1");
            let new_frames = &trace[trace_len_before..];
            assert!(
                new_frames.iter().any(|f| {
                    f.direction == "rx"
                        && f.id == 0x111
                        && f.data.first().map(|b| b & 0xF0 == 0x30).unwrap_or(false)
                }),
                "FlowControl (0x30 nibble) must appear in bxCAN rx trace after ECU FirstFrame; \
                 new frames after tick 1: {:?}",
                new_frames
                    .iter()
                    .map(|f| (f.direction.as_str(), f.id, f.data.clone()))
                    .collect::<Vec<_>>()
            );
        }

        // Complete the exchange: one CF carries the remaining 7 bytes to reach
        // the declared 13.  After this the tester must reach Done.
        inject_ecu_reply(
            &mut bus,
            0x222,
            &[0x21, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x30],
        );
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    /// Session-gated write rejected in the default session: the tester must
    /// accept a negative response when the step declares `expect_nrc`.
    #[test]
    fn uds_tester_expect_nrc_negative_response_completes() {
        let steps = vec![UdsStep {
            send: SystemBus::yaml_bytes(
                Some(&serde_yaml::Value::String(
                    "2E 01 23 DE AD BE EF".to_string(),
                )),
                &[],
            ),
            expect: Vec::new(),
            expect_nrc: Some(0x31),
        }];
        let mut bus = bus_with_steps(steps);
        inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x7F, 0x2E, 0x31]);
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }
}

#[cfg(test)]
mod pin_map_tests {
    use super::*;
    use labwired_config::{ChipDescriptor, SystemManifest};

    fn mkw41z4_bus() -> SystemBus {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../configs/chips/mkw41z4.yaml");
        let chip = ChipDescriptor::from_file(&path).expect("load mkw41z4");
        let manifest = SystemManifest {
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "pinmap-test".to_string(),
            chip: path.to_string_lossy().to_string(),
            external_devices: vec![],
            board_io: vec![],
            debug_uart: None,
            peripherals: vec![],
            memory_overrides: Default::default(),
        };
        SystemBus::from_config(&chip, &manifest).expect("assemble bus")
    }

    #[test]
    fn pin_map_populated_from_chip_pins() {
        let bus = mkw41z4_bus();
        assert_eq!(bus.pin_map.get("PC0"), Some(&("gpioc".to_string(), 0u8)));
        // KW41Z remap: "PB6" labels a gpioc pin, not gpiob.
        assert_eq!(bus.pin_map.get("PB6"), Some(&("gpioc".to_string(), 2u8)));
        assert_eq!(bus.pin_map.len(), 8);
    }
}
