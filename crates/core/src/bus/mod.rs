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
mod attached_devices;
pub mod bus_trace;
mod can_devices;
mod construct;
mod declarative_device;
mod device_hooks;
pub(crate) mod embedded_descriptors;
pub(crate) mod external_devices;
mod faults;
mod from_config;
pub mod known_stubs;
mod mmio_activity;
mod mmio_words;
mod motors;
pub(crate) mod part_pack;
mod pms;
mod policy;
mod profiles;
mod resident_device;
mod routing;
pub mod sim_inputs;
mod tick;

pub use can_devices::*;
pub use resident_device::BusResidentDevice;

pub use bus_trace::{new_log, BusPayload, BusTraceEvent, BusTraceLog, I2cSym};
pub use motors::MotorSnapshot;

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

/// One RCC bit a peripheral's clock depends on, resolved to a concrete register
/// offset at bus-build time (the symbolic `reg` name from the yaml is mapped to
/// the active chip family's offset via [`Rcc::rcc_reg_offset`]).
#[derive(Debug, Clone, Copy)]
pub struct RccClockBit {
    /// Byte offset of the RCC register within the rcc peripheral.
    pub reg_offset: u64,
    /// Bit position within that register that must be set.
    pub bit: u8,
}

/// A peripheral's RCC clock-gate: every bit in [`Self::requires`] must be set in
/// the *live* RCC register map for a CPU access to the owning peripheral to take
/// effect — modelling silicon clock-gating.
///
/// This is the ONE place the engine expresses "this model may only answer while
/// the RCC says it is clocked", and [`SystemBus::is_peripheral_clocked`] is the
/// ONE place it is evaluated. A peripheral model must never grow its own clock
/// check: a bus-enable bit and a kernel-clock-source ready bit are both just
/// entries in this list, so a new gating reason is a config line, not a second
/// mechanism scattered into `peripherals/`.
#[derive(Debug, Clone, Default)]
pub struct ResolvedClockGate {
    /// The bits that must ALL be set. Never empty when the gate is `Some`.
    pub requires: Vec<RccClockBit>,
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
    /// Optional RCC clock-gate (silicon clock-gating model). `None` (the common
    /// case) → the peripheral is never gated and accesses always pass through.
    /// `Some` → accesses are dropped (writes ignored, reads return 0) while ANY
    /// required bit is clear in the RCC, exactly like an unclocked peripheral on
    /// real silicon. Resolved from `PeripheralConfig::clock` in `from_config`.
    pub clock_gate: Option<ResolvedClockGate>,
}

/// Atomic register-alias operation (see [`SystemBus::atomic_alias_redirect`]).
/// Which alias index means which op is a per-family fact and lives with the
/// descriptor, in [`labwired_config::AtomicAliasFlavour`] — re-exported here
/// because the bus is where it is applied.
pub use labwired_config::{AtomicAliasFlavour, AtomicAliasOp};

// True while SystemBus::write_u32 is applying an RP2040 CLR-alias (+0x3000)
// as an absolute final value. Write-clear status registers (USB SIE_STATUS /
// BUFF_STATUS) use this to distinguish CLR-alias (pico-sdk hw_clear) from
// direct W1C (ArduinoCore-mbed USBPhyHw).
thread_local! {
    static CLR_ALIAS_WRITE: Cell<bool> = const { Cell::new(false) };
}

/// See [`CLR_ALIAS_WRITE`].
#[inline]
pub fn is_clr_alias_write() -> bool {
    CLR_ALIAS_WRITE.with(|c| c.get())
}

#[inline]
pub(crate) fn with_clr_alias_write<R>(f: impl FnOnce() -> R) -> R {
    CLR_ALIAS_WRITE.with(|c| {
        let prev = c.replace(true);
        let r = f();
        c.set(prev);
        r
    })
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
    /// Debugger-only register schemas for NATIVE peripherals, keyed by
    /// peripheral name. Populated from a chip YAML's optional
    /// `config.debug_schema` path.
    ///
    /// Native peripherals model behaviour in hand-written Rust and advertise no
    /// `describe_registers()`, so they inspect as `registers: []` — which reads
    /// in a debugger as "this peripheral has no registers" when the truth is
    /// "nobody told the debugger their names". On nRF52840 that was all 52.
    ///
    /// This is a side map rather than a `PeripheralEntry` field on purpose: it
    /// is debugger metadata, not part of a peripheral's identity on the bus, and
    /// keeping it out of the entry keeps it structurally impossible for it to
    /// influence dispatch.
    ///
    /// It confers NO fidelity. Nothing here changes what the bus does, and the
    /// `register_coverage` gate measures live bus behaviour, not schema.
    /// See [`crate::inspect::inspect_with_schema`].
    pub debug_schemas: std::collections::HashMap<String, Vec<crate::inspect::RegisterSchema>>,
    pub nvic: Option<Arc<NvicState>>,
    pub observers: Vec<Arc<dyn crate::SimulationObserver>>,
    pub config: crate::SimulationConfig,
    /// The clock this system's core runs at, in Hz: the manifest's `cpu_hz:`
    /// when it declares one, otherwise the chip descriptor's.
    ///
    /// Every device that times its own waveform reads it from here. Before
    /// this field the number was a literal at each attach site — `80_000_000`
    /// in the declarative-device arms, `160_000_000` in the WS2812 kit — and a
    /// board's declared clock reached none of them.
    ///
    /// `0` when neither the manifest nor the chip declares one; each attach
    /// site keeps its former literal as the fallback for that case, so a chip
    /// YAML predating [`labwired_config::ChipDescriptor::cpu_hz`] behaves
    /// exactly as it did.
    pub cpu_hz: u64,
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
    /// Which family's atomic register aliases this chip implements (see
    /// `ChipDescriptor::atomic_register_aliases`). When enabled, word accesses
    /// in the peripheral window whose address has bits [13:12] set decode as a
    /// read-modify-write on the aligned base register; the flavour decides
    /// which of the three aliases is SET, which CLR and which XOR/TGL.
    pub atomic_register_aliases: AtomicAliasFlavour,
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
    /// Retained scratch for `poll_scheduler_matrix_sources`: each scheduler-driven
    /// peripheral fills this buffer via `Peripheral::matrix_irq_sources_into`
    /// instead of returning a freshly-allocated `Vec` per poll. Cleared before
    /// each peripheral, reused across the whole poll and across batches — the
    /// per-batch IRQ-level re-derivation no longer allocates.
    matrix_source_scratch: Vec<u32>,
    peripheral_hint: Cell<Option<usize>>,
    /// Last **winning** peripheral window from [`find_peripheral_index`]:
    /// `(range_ord, start, end, peri_index)` where `range_ord` is the index
    /// into `peripheral_ranges`. Same-window sequential accesses are O(1) when
    /// the next sorted range starts past `addr` (no narrower window can win).
    /// Cleared on range rebuild. Fidelity: greatest-start-wins, history-independent
    /// (see `overlapping_windows_route_history_independently`).
    last_route: Cell<Option<(usize, u64, u64, usize)>>,
    /// Negative route cache: a `[start, end)` address gap proven to contain
    /// NO peripheral window. Instruction fetch (XIP/flash) and plain RAM
    /// traffic miss the peripheral map on every access; without this they
    /// re-scan `peripheral_ranges` each time and cache nothing (the miss path
    /// clears `last_route`). Same staleness contract as `last_route`: cleared
    /// on range rebuild.
    last_gap: Cell<Option<(u64, u64)>>,
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
    /// edge-detection pass that drives GPIOTE EVENTS_IN.
    ///
    /// `None` until the first edge-detection pass, which ADOPTS the live IN
    /// registers as the baseline and reports no changes. An edge is a
    /// transition, and before the first observation there is nothing to have
    /// transitioned from: a level the outside world already holds — a
    /// `board_io` button settling its released level at attach, a sensor
    /// driving a status line before the first cycle — was never a press. A
    /// `[0; 2]` seed made every such pin present itself as a rising edge on the
    /// very first tick, which both latched a GPIOTE EVENTS_IN nothing caused
    /// and, through the per-edge scheduler harvest, perturbed unrelated
    /// scheduler-driven models. `Option` rather than a companion flag so a
    /// construction site cannot silently spell "not yet sampled" as "sampled
    /// zero".
    last_gpio_in: Option<[u32; 4]>,
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
    /// Run-lifetime successful RAM / flash / extra_mem reads (any width).
    /// Always-on, cheap; not MMIO (see [`Self::peripheral_accesses`]).
    memory_reads: std::cell::Cell<u64>,
    /// Run-lifetime successful RAM / flash / extra_mem writes (any width).
    memory_writes: std::cell::Cell<u64>,
    /// Run-lifetime peripheral MMIO accesses (read or write), counted at
    /// [`Self::note_mmio_activity`]. Never double-counts memory.
    peripheral_accesses: std::cell::Cell<u64>,
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
    /// Tick-driven GPIO-stimulus devices that live directly on the bus and
    /// drive input-register pins the MCU samples — the DHT22 one-wire sensor,
    /// the incremental rotary encoder, and the 4×4 matrix keypad. Each is a
    /// [`BusResidentDevice`]; a single per-tick pass ([`service_gpio_devices`])
    /// drives them all in registration order, touching the bus only on a
    /// transition. Empty by default → zero cost. (The HC-SR04 keeps its own
    /// field above because it also rides the event-scheduler edge-deadline
    /// path.)
    ///
    /// [`service_gpio_devices`]: Self::service_gpio_devices
    pub gpio_devices: Vec<Box<dyn BusResidentDevice>>,
    /// WS2812 / NeoPixel strips. Each is installed as a GPIO observer on its data
    /// pin (ESP32-S3 only today — the RMT drives the pad), so decode is fully
    /// edge-driven with no per-tick pass. Held here as `Arc` clones purely so the
    /// UI/oracle can read the decoded pixels back. Empty by default → zero cost.
    pub ws2812: Vec<std::sync::Arc<crate::peripherals::components::ws2812::Ws2812>>,
    /// Hobby PWM servos (SG90 / MG996R-class). Driven by GPIO edges and/or LEDC
    /// duty observers; held as `Arc` clones so the UI can poll shaft angle via
    /// `get_actuator_states`. Empty by default → zero cost.
    pub servos: Vec<std::sync::Arc<crate::peripherals::components::servo::Servo>>,
    /// STEP/DIR steppers (A4988/DRV8825/TMC2209). GPIO-observer driven.
    pub step_dir_motors:
        Vec<std::sync::Arc<crate::peripherals::components::step_dir_motor::StepDirMotor>>,
    /// H-bridge channels (L298N/TB6612). GPIO-observer driven.
    pub h_bridge_motors:
        Vec<std::sync::Arc<crate::peripherals::components::h_bridge_motor::HBridgeMotor>>,
    /// Deterministic typed motor plants, resolved from the system manifest.
    motors: Vec<motors::MotorRuntime>,
    /// Last simulator-cycle boundary applied to `motors`.
    motor_cycle_anchor: u64,
    /// ILI9341 16-bit 8080 parallel panels (GPIO bit-bang). Observer-driven on
    /// ESP32 / ESP32-S3; held as `Arc` clones so inspect can read the RGB565
    /// framebuffer. Empty by default → zero cost. Distinct from SPI `ili9341`.
    pub ili9341_parallel:
        Vec<std::sync::Arc<crate::peripherals::components::ili9341_parallel::Ili9341Parallel>>,
    /// 4-phase unipolar steppers (28BYJ-48 + ULN2003). GPIO-observer driven.
    pub unipolar_steppers:
        Vec<std::sync::Arc<crate::peripherals::components::unipolar_stepper::UnipolarStepper>>,
    /// TM1637 4-digit 7-segment displays bit-banged over two GPIO lines. Each is
    /// driven by the CLK/DIO GPIO write-hook (`maybe_clock_tm1637`), which feeds
    /// line transitions to the display's protocol state machine. Purely
    /// write-driven (no per-tick pass). Empty by default → zero cost.
    pub tm1637: Vec<crate::peripherals::components::tm1637_7seg::Tm1637>,
    /// HX711 load-cell amps bit-banged over SCK/DT. Write-hook clocks data out;
    /// DT level is driven onto the MCU input register. Empty → zero cost.
    pub hx711: Vec<crate::peripherals::components::hx711::Hx711>,
    /// Direct-drive single-digit 7-segment displays: eight segment GPIOs plus a
    /// common pin, no driver chip. Sampled by the GPIO write-hook
    /// (`maybe_sample_seven_segment`), which recomputes the lit segments
    /// combinationally — no protocol state, no per-tick pass. Empty by default
    /// → zero cost.
    pub seven_segment: Vec<crate::peripherals::components::seven_segment::SevenSegment>,
    /// Analog stimulus sources (potentiometer, NTC thermistor). Unlike a bus
    /// slave these do not sit on I2C/SPI/UART - they drive one ADC channel's
    /// injected millivolt level. They are held here so the generic stimulus
    /// walk ([`Self::for_each_sim_input`]) can reach them: previously the kit
    /// computed a level at attach and dropped the model, which made these
    /// parts un-drivable at runtime. Empty by default -> zero cost.
    pub analog_inputs: Vec<sim_inputs::AnalogInputSource>,
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
    /// Index of the ESP32-C3 `SENSITIVE` peripheral (0x600C_1000), which owns
    /// the permission-control (PMS) register file. `None` on every other bus.
    esp32c3_sensitive_idx: Option<usize>,
    /// ESP32-C3 permission-control unit. A *derived cache* of the `SENSITIVE`
    /// register file (rebuilt by `sync_esp32c3_pms_write` on every write into
    /// the PMS register span) plus the latched violation status. `None` unless
    /// the bus carries a C3 `SENSITIVE` block.
    esp32c3_pms: Option<Box<crate::peripherals::esp32c3::pms::Esp32C3Pms>>,
    /// Measurement hook (never set by the runtime): while true, the C3 PMS
    /// accepts every register write, including ones a lock bit or a
    /// hardware-owned status register would otherwise reject. See
    /// [`SystemBus::set_pms_write_bypass`].
    pms_write_bypass: bool,
    /// Hot-path gate: `true` only while the PMS could actually block something
    /// (some area narrowed AND its monitor enabled). Every store and every
    /// instruction-fetch window refill reads this one bool, so firmware that
    /// never enables memory protection pays a single predictable branch and
    /// behaves byte-identically to before the PMS model existed.
    esp32c3_pms_armed: bool,
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
    /// Index of an nRF52 NVMC peripheral, if this chip has one. Cached in
    /// `rebuild_peripheral_ranges` (same contract as `flash_error_flags_idx`).
    /// When `Some(idx)`, the flash-region write path consults it on every
    /// store: dropped unless CONFIG.Wen is set, committed as `existing & new`
    /// (bits only flip 1→0) when it is. `None` on every non-nRF52 bus, so
    /// that path is unchanged everywhere else.
    nrf52_nvmc_idx: Option<usize>,
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
    /// What the system manifest DECLARED under `external_devices:`, verbatim.
    ///
    /// Purely identity metadata for [`crate::Machine::inspect`], which joins it
    /// onto the live models it finds by walking controllers. Nothing on the
    /// simulation path reads it and nothing here can influence dispatch — it is
    /// the same "side map, not part of a peripheral's identity" arrangement as
    /// [`Self::debug_schemas`], for the same reason.
    ///
    /// It is a record of what was WRITTEN, not of what was built. A declaration
    /// that no live model matches is therefore never reported as a device —
    /// see [`crate::inspect::DeviceInspect::declared`].
    pub external_device_decls: Vec<ExternalDeviceDecl>,
}

/// One `external_devices:` entry, reduced to the fields inspect joins on.
///
/// Kept as its own type rather than holding
/// [`labwired_config::ExternalDevice`] so the bus does not carry the whole
/// on-disk config shape (route maps, free-form YAML config) around for the sake
/// of four fields.
#[derive(Debug, Clone)]
pub struct ExternalDeviceDecl {
    pub id: String,
    pub device_type: String,
    /// A controller peripheral id (`"i2c0"`), or another declaration's `id`
    /// when this device sits behind an I²C bus switch.
    pub connection: String,
    /// Bus-switch channel, when `connection` names a switch.
    pub channel: Option<u8>,
    /// `config.i2c_address`, when declared.
    pub address: Option<u8>,
    /// `config.cs_pin`, when declared.
    pub cs_pin: Option<String>,
}

impl ExternalDeviceDecl {
    /// Reduce a manifest entry. `i2c_address` is read as an integer; a manifest
    /// that omits it (leaving the device model's own default to stand) yields
    /// `None`, and the inspect join falls back to positional matching.
    pub fn from_manifest(ext: &labwired_config::ExternalDevice) -> Self {
        let addr = ext
            .config
            .get("i2c_address")
            .and_then(|v| v.as_u64())
            .and_then(|v| u8::try_from(v).ok());
        let cs = ext
            .config
            .get("cs_pin")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        Self {
            id: ext.id.clone(),
            device_type: ext.r#type.clone(),
            connection: ext.connection.clone(),
            channel: ext.channel,
            address: addr,
            cs_pin: cs,
        }
    }
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
