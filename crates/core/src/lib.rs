#![allow(clippy::manual_is_multiple_of)]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod boot;
pub mod bus;
pub mod config;
pub mod cosim;
pub mod coverage;
pub mod cpu;
pub mod cycle_clock;
pub mod decoder;
pub mod fidelity;
pub mod inspect;
pub mod interrupt;
pub mod logic_capture;
pub mod machine;
pub mod memory;
pub mod metrics;
pub mod multi_core;
pub mod network;
pub mod pc_coverage;
pub mod peripherals;
pub mod physics;
pub mod runtime_snapshot;
pub mod sched;
pub mod signals;
pub mod sim_input;
pub mod snapshot;
pub mod system;
pub mod trace;
pub mod vfi;
pub mod world;

pub use config::SimulationConfig;
pub use cycle_clock::CycleClock;
pub use machine::{
    AdvanceLimits, AdvanceReport, AdvanceRequest, AdvanceStop, BatchPolicy, BreakpointPolicy,
    IdlePolicy,
};

use std::any::Any;
use std::sync::Arc;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Arch {
    Arm,
    RiscV,
    XtensaLx7,
    Unknown,
}

#[derive(Debug, thiserror::Error)]
pub enum SimulationError {
    #[error("Memory access violation at {0:#x}")]
    MemoryViolation(u64),
    #[error("Instruction decoding error at {0:#x}")]
    DecodeError(u64),
    #[error("Simulation halted")]
    Halt,
    #[error("Snapshot schema mismatch: expected v{expected}, got v{got}")]
    SnapshotSchemaMismatch { expected: u32, got: u32 },
    #[error("not implemented: {0}")]
    NotImplemented(String),
    #[error("Breakpoint hit at {0:#x}")]
    BreakpointHit(u32),
    #[error("Exception raised: cause={cause} at pc={pc:#x}")]
    ExceptionRaised { cause: u8, pc: u32 },
    #[error("Simulation error: {0}")]
    Other(String),
}

pub type SimResult<T> = Result<T, SimulationError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaDirection {
    Read,
    Write,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaRequest {
    pub src_addr: u64,
    pub addr: u64,
    pub val: u8,
    pub direction: DmaDirection,
    /// Unit-level reshape for engines that convert between source and
    /// destination data widths (STM32H5 GPDMA CTR1 PAM / SBX / DBX / DHX).
    /// `None` keeps the classic single-byte semantics.
    pub transform: Option<DmaUnitTransform>,
}

/// One source-data-unit -> destination-data-unit transform descriptor.
/// Semantics follow RM0481 §15 and are pinned by the DMA_DataHandling
/// HAL example's expected-result vectors (sim) and its on-board run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DmaUnitTransform {
    /// Source data width in bytes (1, 2 or 4).
    pub src_width: u8,
    /// Destination data width in bytes (1, 2 or 4).
    pub dst_width: u8,
    /// CTR1.PAM[0]: 0 = right-align zero-pad (narrow->wide) / left-truncate
    /// keep-LSBs (wide->narrow); 1 = sign-extend / right-truncate keep-MSBs.
    pub pam: u8,
    /// CTR1.SBX — exchange the two middle bytes of a word-width source.
    pub sbx: bool,
    /// CTR1.DBX — swap bytes within each destination half-word.
    pub dbx: bool,
    /// CTR1.DHX — swap the half-words of a word-width destination.
    pub dhx: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PeripheralTickResult {
    /// Raises the peripheral's configured NVIC IRQ position (set in
    /// `PeripheralEntry::irq`). The bus pends it on NVIC.ISPR so the
    /// CPU only takes it when ISER also has it enabled.
    pub irq: bool,
    pub cycles: u32,
    pub dma_requests: Option<Vec<DmaRequest>>,
    /// Extra NVIC IRQ positions beyond the peripheral's default.
    pub explicit_irqs: Option<Vec<u32>>,
    /// System exception number (0..15) to raise — for SysTick (15) and
    /// other core-tied exceptions that bypass NVIC. Pushed directly to
    /// the CPU's pending_exceptions bitmap; PRIMASK still gates it.
    pub system_exception: Option<u32>,
    pub dma_signals: Option<Vec<u32>>,
    pub ticks_until_next: Option<u64>,

    /// Cross-peripheral side-effect writes the peripheral wants the bus
    /// to apply on its behalf. Used by GPIOTE to drive GPIO OUTSET/OUTCLR
    /// without holding a bus handle, and as the sink for PPI-triggered
    /// task writes.
    pub mmio_writes: Vec<(u32, u32)>,

    /// Offsets (within this peripheral's window) of EVENTS_* registers
    /// that transitioned 0→1 during this tick. The bus globalises them
    /// to absolute addresses (peripheral.base + offset) and feeds them
    /// to the PPI router. Peripherals that don't fire events leave this
    /// empty; consumers ignore it.
    pub fired_events: Vec<u32>,
}

impl PeripheralTickResult {
    pub fn with_irq(irq: bool) -> Self {
        Self {
            irq,
            ..Default::default()
        }
    }
}

/// Trait for observing simulation events in a modular way.
pub trait SimulationObserver: std::fmt::Debug + Send + Sync {
    fn on_simulation_start(&self) {}
    fn on_simulation_stop(&self) {}
    fn on_trace_event(&self, _event: labwired_hw_trace::TraceEvent) {}
    fn on_step_start(&self, _pc: u32, _opcode: u32) {}
    fn on_step_end(&self, _cycles: u32, _registers: &[u32]) {}
    fn on_memory_write(&self, _addr: u64, _old: u8, _new: u8) {}
    fn on_peripheral_tick(&self, _name: &str, _cycles: u32) {}
}

pub fn emit_trace_event(
    observers: &[Arc<dyn SimulationObserver>],
    event: labwired_hw_trace::TraceEvent,
) {
    for observer in observers {
        observer.on_trace_event(event.clone());
    }
}

/// Trait representing a CPU architecture
/// Feature-agnostic JIT engine run counters, mirrored from a concrete CPU's
/// engine so generic (`C: Cpu`) callers can observe non-vacuity without pulling
/// in the JIT-only `EngineStats` type. All-zero / absent for interpreter-only
/// CPUs and for runs where the JIT engine was never created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuJitStats {
    /// Blocks that crossed the hot threshold and were compiled + installed.
    pub compiled: u64,
    /// Compiled-block invocations.
    pub block_runs: u64,
    /// Guest instructions retired inside compiled blocks.
    pub block_instrs: u64,
    /// Guest instructions retired on the interpreter fallback path.
    pub interpreted: u64,
}

pub trait Cpu: Send {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()>;
    /// JIT engine counters for this CPU, if it ran a JIT that was created.
    /// Default `None` (interpreter-only CPUs / non-JIT builds). Lets generic
    /// `C: Cpu` callers — e.g. the CLI oracle loop — assert the compiled path
    /// was non-vacuously exercised without downcasting to the concrete CPU.
    fn jit_engine_stats(&self) -> Option<CpuJitStats> {
        None
    }
    /// Downcast escape hatch for runtime fast-paths that need the
    /// concrete CPU type (e.g. the browser-side JIT prototype in
    /// `labwired-wasm` reaches into `XtensaLx7` for direct register
    /// access). Default returns `None`, matching the rest of the trait
    /// surface: only the CPUs that opt in are reachable this way.
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()>;
    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        // While push-mode logic capture is armed, the tap clock must advance
        // once per retired instruction so pad writes stamp with the cycle
        // boundary they become observable at (see `crate::logic_capture`).
        // One Arc clone + flag check per batch when idle; a relaxed atomic
        // increment per instruction while armed.
        let tap = bus.logic_tap().filter(|t| t.push_armed());
        for i in 0..max_count {
            if let Some(tap) = &tap {
                tap.bump_clock();
            }
            self.step(bus, observers, config)?;
            if config.idle_fast_forward_enabled && self.idle_fast_forward_budget(bus).is_some() {
                return Ok(i + 1);
            }
        }
        Ok(max_count)
    }
    fn set_pc(&mut self, val: u32);
    fn get_pc(&self) -> u32;
    fn set_sp(&mut self, val: u32);
    fn set_exception_pending(&mut self, exception_num: u32);

    // Debug Access
    fn get_register(&self, id: u8) -> u32;
    fn set_register(&mut self, id: u8, val: u32);
    fn snapshot(&self) -> snapshot::CpuSnapshot;
    fn apply_snapshot(&mut self, snapshot: &snapshot::CpuSnapshot);

    /// Full mid-flight CPU state for binary runtime snapshots. Returns the
    /// arch tag + an opaque blob the matching CPU type knows how to parse.
    /// Default panics — every concrete `Cpu` impl must override.
    fn runtime_snapshot(&self) -> (runtime_snapshot::CpuKind, Vec<u8>) {
        unimplemented!("runtime_snapshot not implemented for this Cpu")
    }

    /// Apply a previously-taken runtime snapshot. Default no-op so
    /// stub/test CPUs don't need to override.
    fn apply_runtime_snapshot(
        &mut self,
        _kind: runtime_snapshot::CpuKind,
        _bytes: &[u8],
    ) -> SimResult<()> {
        Ok(())
    }
    fn get_register_names(&self) -> Vec<String>;
    fn index_of_register(&self, name: &str) -> Option<u8>;

    // Security & Physical Extensions
    fn inject_fault(&mut self, _target: &str) -> SimResult<()> {
        Ok(())
    }
    fn get_energy_consumption(&self) -> f64 {
        0.0
    }

    /// Set bits in the pending-interrupt register. Default no-op; Xtensa
    /// implementations latch the bits into INTERRUPT (SR id 226) so a
    /// cross-core IPI bridge in the host runtime can synthesise edges
    /// without owning the SR file directly.
    fn raise_interrupt_bits(&mut self, _mask: u32) {}

    /// Halt this CPU: subsequent `step()` calls are no-ops until
    /// `unhalt()`. Models reset-hold for dual-core SoCs where one core
    /// is held in reset until the bootstrap CPU releases it. Default
    /// no-op so single-core CPUs need no implementation.
    fn halt(&mut self) {}
    /// Release a previously-halted CPU; pairs with [`Self::halt`].
    fn unhalt(&mut self) {}

    /// Current interrupt-mask level. Used by dual-core schedulers to
    /// serialize critical sections — when one CPU has intlevel > 0
    /// (typically because portENTER_CRITICAL raised it to 3 to hold a
    /// spinlock), the sim runs that CPU solo until it drops back to 0.
    /// Default returns 0 so single-core CPUs need no implementation.
    fn intlevel(&self) -> u8 {
        0
    }

    /// Return the maximum number of idle cycles this CPU may skip now, or
    /// `None` when it is not architecturally waiting or an interrupt is already
    /// observable. Machine-level guards decide whether the bus/peripheral side
    /// is also safe to skip.
    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        None
    }

    /// Advance CPU-local time/counters for cycles skipped while idle. The
    /// default is a no-op for CPUs that do not opt into idle fast-forwarding.
    fn fast_forward_idle_cycles(&mut self, _cycles: u64) {}

    /// True while this core is parked in an architectural wait (e.g. Xtensa
    /// `WAITI`) and will only retire work when an interrupt wakes it.
    ///
    /// Dual-core machines use this to batch the primary core while a secondary
    /// APP CPU sits in FreeRTOS idle: lockstep quantum-1 is only required when
    /// the secondary is actively executing or still held in reset (`halted`).
    fn is_parked_idle(&self) -> bool {
        false
    }

    /// Phase 3.2 JIT pilot (issue #124): total number of times any
    /// JIT-compiled block on this CPU has been invoked. Default 0 for
    /// CPUs/builds without JIT support so callers can unconditionally
    /// query it in reporting paths.
    fn jit_hit_count(&self) -> u64 {
        0
    }
}

// Forwarding impl so `Machine<Box<dyn Cpu>>` is valid — used by the WASM
// runtime to hold an arch-dispatched CPU without making WasmSimulator
// generic over C: Cpu (wasm-bindgen can't expose a generic struct).
impl Cpu for Box<dyn Cpu> {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        (**self).reset(bus)
    }
    /// Forward the concrete-type escape hatch through the Box.
    /// Without this, `Machine<Box<dyn Cpu>>::cpu.as_any_mut()` would
    /// hit the default impl on the trait (which returns `None`),
    /// silently disabling every downcast — including the browser JIT
    /// dispatcher in `labwired-wasm`. Reported during #124 Phase 4.2
    /// bench validation.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        (**self).as_any_mut()
    }
    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        (**self).step(bus, observers, config)
    }
    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        (**self).step_batch(bus, observers, config, max_count)
    }
    fn set_pc(&mut self, val: u32) {
        (**self).set_pc(val)
    }
    fn get_pc(&self) -> u32 {
        (**self).get_pc()
    }
    fn set_sp(&mut self, val: u32) {
        (**self).set_sp(val)
    }
    fn set_exception_pending(&mut self, n: u32) {
        (**self).set_exception_pending(n)
    }
    fn get_register(&self, id: u8) -> u32 {
        (**self).get_register(id)
    }
    fn set_register(&mut self, id: u8, val: u32) {
        (**self).set_register(id, val)
    }
    fn snapshot(&self) -> snapshot::CpuSnapshot {
        (**self).snapshot()
    }
    fn apply_snapshot(&mut self, s: &snapshot::CpuSnapshot) {
        (**self).apply_snapshot(s)
    }
    fn runtime_snapshot(&self) -> (runtime_snapshot::CpuKind, Vec<u8>) {
        (**self).runtime_snapshot()
    }
    fn apply_runtime_snapshot(
        &mut self,
        kind: runtime_snapshot::CpuKind,
        bytes: &[u8],
    ) -> SimResult<()> {
        (**self).apply_runtime_snapshot(kind, bytes)
    }
    fn get_register_names(&self) -> Vec<String> {
        (**self).get_register_names()
    }
    fn index_of_register(&self, name: &str) -> Option<u8> {
        (**self).index_of_register(name)
    }
    fn inject_fault(&mut self, target: &str) -> SimResult<()> {
        (**self).inject_fault(target)
    }
    fn get_energy_consumption(&self) -> f64 {
        (**self).get_energy_consumption()
    }
    fn raise_interrupt_bits(&mut self, mask: u32) {
        (**self).raise_interrupt_bits(mask)
    }
    fn halt(&mut self) {
        (**self).halt()
    }
    fn unhalt(&mut self) {
        (**self).unhalt()
    }
    fn intlevel(&self) -> u8 {
        (**self).intlevel()
    }
    fn idle_fast_forward_budget(&self, bus: &dyn Bus) -> Option<u64> {
        (**self).idle_fast_forward_budget(bus)
    }
    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        (**self).fast_forward_idle_cycles(cycles)
    }
    fn is_parked_idle(&self) -> bool {
        (**self).is_parked_idle()
    }
    fn jit_hit_count(&self) -> u64 {
        (**self).jit_hit_count()
    }
}

/// Trait representing a memory-mapped peripheral
/// Host-side classification of one MMIO access for idle/coalesce policy.
///
/// **CPU-agnostic:** the bus never hardcodes chip register maps. Each
/// peripheral model opts in via [`Peripheral::mmio_access_class`]. Default is
/// [`SideEffecting`](MmioAccessClass::SideEffecting) so unknown devices never
/// get accelerated and one CPU's optim path cannot silently affect another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MmioAccessClass {
    /// May change peripheral or world state — disqualifies poll-coalesce.
    SideEffecting,
    /// Freerunning timer snapshot only (busy-poll loops). Coalesce-eligible
    /// under idle FF; device time still advances to the next event.
    FreerunningTimerPoll,
    /// Side-effect-free window (e.g. XIP code/rodata). Ignored for bookkeeping.
    SideEffectFree,
}

pub trait Peripheral: std::fmt::Debug + Send {
    fn read(&self, offset: u64) -> SimResult<u8>;
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()>;
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let b0 = self.read(offset)? as u16;
        let b1 = self.read(offset + 1)? as u16;
        Ok(b0 | (b1 << 8))
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let b0 = self.read(offset)? as u32;
        let b1 = self.read(offset + 1)? as u32;
        let b2 = self.read(offset + 2)? as u32;
        let b3 = self.read(offset + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write(offset + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write(offset + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }
    /// Plan 2: word-granular write path. The bus calls this after performing
    /// the four byte writes, giving peripherals a single coherent 32-bit
    /// view of the write. Default: no-op. Peripherals with 32-bit word
    /// triggers (e.g. declarative configs with WriteWord triggers) override.
    fn write_word_32(&mut self, _offset: u64, _value: u32) -> SimResult<()> {
        Ok(())
    }
    /// Side-effect-free value probe used for debug/observer bookkeeping.
    /// Implementations should return `None` when such probing is not supported.
    fn peek(&self, _offset: u64) -> Option<u8> {
        None
    }
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }
    /// Advance a peripheral by the number of CPU cycles elapsed since the last
    /// bus tick. The default preserves the legacy contract: one tick callback
    /// per bus tick, regardless of the configured interval. Timer peripherals
    /// that care about elapsed CPU cycles override this.
    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        self.tick()
    }
    /// Specialized compatibility hook for a bare-CPU hardware oracle that
    /// freezes the CPU and settles peripherals through their historical walk
    /// even when the production event scheduler owns them.
    ///
    /// The default preserves ordinary tick behavior. Scheduler-driven models
    /// whose regular `tick` deliberately no-ops may override this to expose
    /// their legacy one-tick transition to that oracle only.
    #[doc(hidden)]
    fn tick_elapsed_forced(&mut self, cycles: u64) -> PeripheralTickResult {
        self.tick_elapsed(cycles)
    }
    /// PPI hook: given absolute addresses of events that just fired
    /// across the bus, return absolute addresses of tasks to trigger.
    /// Only the PPI peripheral overrides this; everyone else returns
    /// the default empty vector and the bus skips them at near-zero
    /// cost.
    fn route_ppi_events(&mut self, _fired_global: &[u32]) -> Vec<u32> {
        Vec::new()
    }

    /// Cross-peripheral GPIO change hook: bus snapshots GPIO IN registers
    /// each tick and calls this with a list of `(port, pin, new_level)`
    /// transitions. GPIOTE overrides to drive EVENTS_IN[i] when a channel
    /// is configured to watch a matching (port, pin) with a matching
    /// polarity. Default no-op.
    fn observe_gpio_change(&mut self, _changes: &[(u8, u8, u8)]) {}

    /// GPIO capability: read the firmware-visible input level for `pin`.
    /// Non-GPIO peripherals return `None`.
    fn read_gpio_input(&self, _pin: u8) -> Option<bool> {
        None
    }

    /// GPIO capability: read the firmware-visible output latch for `pin`.
    /// Non-GPIO peripherals return `None`.
    fn read_gpio_output(&self, _pin: u8) -> Option<bool> {
        None
    }

    /// GPIO capability: the pad level a logic probe clipped to `pin` would
    /// see — output-driven pins report the output latch, input pins report
    /// the input level, pins routed to a peripheral (alternate function)
    /// report `None` when the model can't know the wire state. GPIO models
    /// with a direction register override this; the default prefers the
    /// output latch.
    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        self.read_gpio_output(pin)
            .or_else(|| self.read_gpio_input(pin))
    }

    /// GPIO capability: the routing of `pin` — its direction/`mode` and, when
    /// resolvable, the peripheral signal `func` it is wired to — derived from the
    /// SAME register truth [`read_gpio_pad`](Self::read_gpio_pad) reads (no
    /// fabrication). This is the honest source the UI logic analyzer uses instead
    /// of guessing signal roles from pin NAMES. `None` for non-GPIO peripherals
    /// or out-of-range pins; a returned routing may still carry
    /// `mode = Unknown` / `func = None` where a family cannot say.
    fn gpio_routing(&self, _pin: u8) -> Option<crate::peripherals::gpio::GpioRouting> {
        None
    }

    /// GPIO capability: drive an externally controlled input level for `pin`
    /// (e.g. browser button press). Returns `false` if unsupported.
    fn set_gpio_input(&mut self, _pin: u8, _level: bool) -> bool {
        false
    }

    /// Logic-capture capability: install (or clear) a push-mode logic tap.
    ///
    /// `watched` is this peripheral's slice of the machine's watch set as
    /// `(pin, channel)` pairs (empty ⇒ remove any installed tap). A
    /// push-instrumented peripheral stores the tap and, from then on, reports
    /// every watched pad-level change from its own write sites via
    /// [`logic_capture::LogicTap::push`] — the same direction-aware level
    /// truth [`read_gpio_pad`](Self::read_gpio_pad) reads.
    ///
    /// The return value DECLARES push capability: `true` means "I am
    /// instrumented; my watched pads need no per-cycle polling" (returned for
    /// empty `watched` too), `false` (the default) keeps the machine's
    /// per-cycle poll fallback for channels resolved to this peripheral. This
    /// is the single source of truth — the machine never keeps a hardcoded
    /// list of push-capable peripherals.
    fn install_logic_tap(
        &mut self,
        _tap: &logic_capture::LogicTap,
        _watched: &[(u8, u32)],
    ) -> bool {
        false
    }

    /// Bus-aware tick hook for peripherals that need to read or write the
    /// bus themselves (e.g. Easy DMA on RADIO). Default no-op.
    fn tick_with_bus(&mut self, _bus: &mut dyn Bus) {}

    /// True if this peripheral wants the bus to call `tick_with_bus`.
    /// Default false so the bus skips the swap dance for everyone else.
    fn needs_bus_tick(&self) -> bool {
        false
    }
    /// True if this peripheral needs the legacy per-tick `tick()` walk.
    ///
    /// The conservative default is true: hand-written behavioral peripherals
    /// keep their existing timing unless they explicitly opt out. Declarative
    /// register banks override this dynamically so inert descriptors do not
    /// consume a virtual call on every simulated cycle.
    fn legacy_tick_active(&self) -> bool {
        true
    }
    /// True if `legacy_tick_active` can change after this peripheral's own
    /// `tick()` call. Stable peripherals stay in the cached tick set without
    /// a per-cycle refresh.
    fn legacy_tick_dynamic(&self) -> bool {
        false
    }
    /// True if this peripheral's participation in the legacy per-cycle walk is
    /// behaviorally significant for SOME reachable firmware state — i.e.
    /// deleting the walk (`SystemBus::derive_walk_deletable`) could change
    /// observable output. This is a static, firmware-independent property of
    /// the model, distinct from [`Self::legacy_tick_active`] (a per-instant
    /// state query the walk uses to skip inert entries cheaply).
    ///
    /// The conservative default is `true`: unless a model *proves* its
    /// `tick()`/`tick_elapsed()` can never mutate observable state, it keeps the
    /// walk on. Override to `false` ONLY when BOTH hold for every reachable
    /// state:
    ///
    /// - the peripheral's `tick()`/`tick_elapsed()` is a genuine no-op
    ///   (a pure register bank / stub, or a purely lazy model that advances its
    ///   state on MMIO access rather than in `tick`), AND
    /// - it never emits an IRQ, DMA request, mmio-write, or fired event from
    ///   the walk.
    ///
    /// Scheduler-driven peripherals need NOT override this (they are handled by
    /// `uses_scheduler()` in the derivation); overriding is only for models that
    /// stay on the legacy path but do nothing there. Getting this wrong (a
    /// `false` on a model that actually does walk work) silently starves the
    /// peripheral of ticks once the bus derives walk-deletion — so the honest
    /// direction under any doubt is to leave it `true`.
    fn needs_legacy_walk(&self) -> bool {
        true
    }
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }

    /// Stimulus reachability: call `f` once for every device attached to this
    /// controller that accepts simulated input, in attach order.
    ///
    /// This is the seam behind [`crate::bus::SystemBus::list_inputs`] and
    /// `set_input`. The bus walks peripherals and asks each one this question;
    /// it does NOT know which concrete controller types can host devices. That
    /// is the whole point — the previous implementation was a downcast chain
    /// over three hardcoded types, so every controller added afterwards
    /// (ESP32-C3 I²C/SPI, ESP32-S3 I²C) silently hosted devices that no agent,
    /// test script, MCP call, or UI panel could drive. The component unit tests
    /// still passed; the devices were simply unreachable.
    ///
    /// **A controller that can host attachable devices MUST override this.**
    /// If it does not, its devices are undrivable: they answer no
    /// `list_inputs` query and `set_input` fails with `NoDevice`. There is no
    /// diagnostic for this — the default below is indistinguishable from an
    /// honest "I host nothing", which is exactly how the bug survived. The
    /// rule of thumb: if a type appears in
    /// [`crate::bus::SystemBus::attach_i2c_slave`] or `attach_spi_device`, it
    /// owes an implementation here.
    ///
    /// Early stop: `f` returns `true` to request that the walk stop. An
    /// implementation MUST stop calling `f` at that point and propagate
    /// `true` as its own return value; return `false` when the walk ran to
    /// completion. The bus relies on this to make `set_input` apply to exactly
    /// one device.
    fn for_each_attached_sim_input(
        &mut self,
        _f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        false
    }

    fn dma_request(&mut self, _request_id: u32) {}
    fn snapshot(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn restore(&mut self, _state: serde_json::Value) -> SimResult<()> {
        Ok(())
    }

    /// Optional source register descriptor for debugger clients that need the
    /// config-level layout (including reset values and descriptions), rather
    /// than the display-oriented [`Self::describe_registers`] schema.
    ///
    /// Declarative peripherals return their original descriptor. Behavioral
    /// peripherals that replace a declarative register surface may do the same
    /// to keep debugger integrations faithful to the configured chip.
    fn peripheral_descriptor(&self) -> Option<labwired_config::PeripheralDescriptor> {
        None
    }

    /// Optional register-layout schema for the universal inspect interface.
    /// Declarative peripherals ([`crate::peripherals::declarative::GenericPeripheral`])
    /// return their descriptor's registers, so every declarative peripheral
    /// decodes named registers + bitfields for free. Native peripherals may
    /// return a static map or `None` (then `inspect` yields no schema-decoded
    /// registers). See [`crate::inspect`].
    fn describe_registers(&self) -> Option<Vec<crate::inspect::RegisterSchema>> {
        None
    }

    /// Uniform, snapshot-semantics inspection. The default decodes
    /// [`Self::describe_registers`] against live bytes via [`Self::peek`]
    /// (side-effect-free), so most peripherals need no override. Peripherals
    /// with non-register artifacts (framebuffers, traces) override this,
    /// typically by calling [`crate::inspect::default_inspect`] and pushing
    /// artifacts onto the result. `base`/`name` are supplied by the bus, which
    /// owns the peripheral's placement.
    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        crate::inspect::default_inspect(self, base, name, opts)
    }

    /// Binary mid-flight runtime snapshot — captures whatever state this
    /// peripheral needs for resume. Default is an empty blob: stateless
    /// peripherals can ignore it. Override for RAM regions, framebuffer-
    /// holding panels, sparse register banks, etc.
    ///
    /// Distinct from [`Self::snapshot`], which produces a JSON value for
    /// the determinism gates and human-inspectable replay tools — those
    /// can't carry the megabyte of RAM contents we need for instant
    /// resume.
    fn runtime_snapshot(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Restore from a previously-taken `runtime_snapshot()`. Default is
    /// a no-op: peripherals that don't override `runtime_snapshot`
    /// don't need to override this either.
    fn restore_runtime_snapshot(&mut self, _bytes: &[u8]) -> SimResult<()> {
        Ok(())
    }

    /// Phase 2B.1 (issue #192): event-driven peripheral scheduler hook.
    /// Default returns an empty result; peripherals that opt in
    /// (`uses_scheduler() == true`) override to interpret `event_token`
    /// via their own internal token enum and produce side-effects via
    /// the shared `EventResult` channel.
    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut sched::EventScheduler,
        _bus: &mut dyn Bus,
    ) -> sched::EventResult {
        sched::EventResult::default()
    }

    /// Phase 2B.1: synchronously notified when a subscribed clock domain
    /// changes rate. Implementations typically cancel in-flight events and
    /// reschedule at the new cadence. Default no-op.
    fn on_clock_change(
        &mut self,
        _domain: sched::ClockDomain,
        _new_hz: u64,
        _sched: &mut sched::EventScheduler,
    ) {
    }

    /// Phase 2B.1: when `true`, the authoritative [`Machine::advance`]
    /// lifecycle skips this peripheral's legacy `tick()` walk and relies on
    /// the scheduler to drive it. Default `false` preserves existing per-cycle
    /// tick behaviour.
    fn uses_scheduler(&self) -> bool {
        false
    }

    /// Phase 2B.2 (issue #192): advance a scheduler-driven peripheral's lazy
    /// state to CPU cycle `now_cycle` (`SystemBus::current_cycle` — the same
    /// cycle count the legacy walk advances by via `tick_elapsed(interval)`).
    /// Called by the bus immediately before an MMIO write observes the
    /// peripheral, so a frozen-then-strobed counter reads the up-to-date
    /// value. During a CPU batch `current_cycle` holds the batch-start cycle,
    /// so the sync point trails the true write cycle by less than one
    /// `peripheral_tick_interval` (exact at interval 1). Default no-op; only
    /// peripherals that opt into the scheduler implement it.
    fn sync_to(&mut self, _now_cycle: u64) {}

    /// Phase 2B.3a (issue #192): hand the bus any events this peripheral wants
    /// scheduled as a result of the MMIO write that just completed (e.g. a
    /// UART arming its TX interrupt). Each entry is `(delay_cycles, token)` —
    /// a delay in CPU cycles from the peripheral's just-synced state (the
    /// `sync_to` cycle) and an opaque token the peripheral interprets in its
    /// own `on_event`. The bus converts the delay to an absolute cycle
    /// deadline (see `SystemBus::collect_scheduled_events`). The buffer is
    /// drained (cleared) by this call. A peripheral can't reach the scheduler
    /// from `write`, so this is the bootstrap path; `on_event` reschedules
    /// itself thereafter. Default empty — only write-scheduling peripherals
    /// override.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        Vec::new()
    }

    /// Hand this peripheral the bus's shared [`CycleClock`] so `&self` reads
    /// can lazily sync `Cell`-held counter state to the published "now"
    /// (batch-boundary freshness — exact at batch boundaries, < one
    /// `peripheral_tick_interval` stale mid-batch, the same bound as the
    /// write-path [`Self::sync_to`]).
    ///
    /// Called once by `SystemBus::add_peripheral` at attach time. Default
    /// no-op: peripherals that don't opt in never see it. A read-polled
    /// counter model overrides this to store the clock; the conservative
    /// contract is that WITHOUT an attached clock the model must stay on
    /// its legacy walk path (`uses_scheduler() == false`), so hand-built
    /// buses that bypass `add_peripheral` keep the old exact semantics.
    fn attach_cycle_clock(&mut self, _clock: CycleClock) {}

    /// Tell the peripheral which NVIC/matrix line it was registered with (the
    /// `irq` of its `PeripheralEntry`), or `None` when the descriptor wired
    /// none. Called once at the same attach choke points as
    /// [`Peripheral::attach_cycle_clock`] (`add_peripheral` / `push_peripheral`).
    ///
    /// Exists because [`crate::sched::EventResult::raise_own_irq`] is DROPPED by
    /// the machine when the entry has no `irq` (`lib.rs`, `apply_event_result`):
    /// a model whose only reason to wake per cycle is holding a level-triggered
    /// own-IRQ is doing provably unobservable work on such a bus, and can stop
    /// scheduling itself. Only the shared `Uart` opts in today.
    ///
    /// Default no-op, and the conservative contract matches
    /// `attach_cycle_clock`: a model that never receives this must assume its
    /// IRQ *is* wired and keep its legacy wakeup cadence, so hand-built buses
    /// that bypass the choke points keep the old exact semantics.
    fn attach_irq_line(&mut self, _irq: Option<u32>) {}

    /// Classify an MMIO access at `offset` for host idle/coalesce policy.
    /// Default [`MmioAccessClass::SideEffecting`] — only models that are
    /// proven poll-safe (or proven side-effect-free) override this.
    fn mmio_access_class(&self, _offset: u64) -> MmioAccessClass {
        MmioAccessClass::SideEffecting
    }

    /// Walk-free plan (ESP32-C3): the interrupt-matrix source IDs this
    /// peripheral is asserting RIGHT NOW (level-sensitive). The per-cycle walk
    /// normally re-emits a level source every tick via `PeripheralTickResult::
    /// explicit_irqs`, and the bus rebuilds the C3 asserted-source bitmap from
    /// that each tick. A scheduler-driven peripheral is skipped by the walk, so
    /// the bus re-derives its live level from this method instead
    /// (`SystemBus::refresh_esp32c3_sched_sources`, called from the event path
    /// and the walk-tick aggregation). Default empty — only scheduler-driven
    /// peripherals that raise C3 matrix IRQs override it.
    ///
    /// Push-based twin of [`Self::matrix_irq_sources`]: the bus polls this on
    /// the per-batch IRQ-level re-derivation path (`poll_scheduler_matrix_sources`)
    /// with a RETAINED scratch buffer, so a scheduler-driven peripheral no longer
    /// allocates a fresh `Vec` per poll. Override THIS (not the returning form) in
    /// new models; the returning `matrix_irq_sources` defaults to a thin wrapper.
    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        let _ = out;
    }

    /// Convenience returning form (tests, one-shot callers). The hot per-batch
    /// poll uses [`Self::matrix_irq_sources_into`] with retained scratch instead.
    fn matrix_irq_sources(&self) -> Vec<u32> {
        let mut out = Vec::new();
        self.matrix_irq_sources_into(&mut out);
        out
    }
}

/// Trait representing the system bus
pub trait Bus {
    fn read_u8(&self, addr: u64) -> SimResult<u8>;
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()>;
    fn tick_peripherals(&mut self) -> Vec<u32>; // Returns list of pending exception numbers
    fn execute_dma(&mut self, requests: &[DmaRequest]) -> SimResult<()>;
    fn config(&self) -> &SimulationConfig;
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }

    /// Clear a pending NVIC exception (called by CPU when taking an exception).
    /// For NVIC exceptions (number >= 16), this clears the ISPR bit to prevent
    /// immediate re-entry of the same exception.
    fn clear_nvic_pending(&mut self, _exception_num: u32) {}

    /// Check whether a NVIC exception is still hardware-pending (ISPR bit set).
    ///
    /// Called by the CPU before dispatching an exception that is present in
    /// `pending_exceptions` but may have been software-cleared via NVIC ICPR
    /// (e.g. `NRFY_IRQ_PENDING_CLEAR`) while the ISR was still executing.
    /// Returns `true` for non-NVIC exceptions (num < 16) or when no NVIC is
    /// present — the conservative/backward-compatible default.
    fn is_nvic_irq_pending(&self, _exception_num: u32) -> bool {
        true
    }

    /// Plan 3: route a peripheral source ID to a cpu0 IRQ slot via a
    /// registered ESP32-S3 interrupt matrix peripheral. Default returns
    /// None for buses that don't model this.
    fn route_irq_source_to_cpu_irq(&self, _source_id: u32) -> Option<u8> {
        None
    }

    /// Bitmask of pending CPU IRQ slots the bus has aggregated for the given
    /// core (`core_id` 0 = PRO_CPU, 1 = APP_CPU). On ESP32-S3 the per-core
    /// interrupt matrix routes peripheral source IDs (and the crosscore_ipi
    /// FROM_CPU sources) to each core; the classic-ESP32 dual-core path
    /// delivers FROM_CPU IPIs via the DPORT interrupt matrix instead.
    /// Default returns 0; non-ESP32 buses don't model this.
    fn pending_cpu_irqs(&self, _core_id: u8) -> u32 {
        0
    }

    /// Plan 3: clear the pending bit for cpu IRQ `slot` on `core_id`.
    fn clear_cpu_irq_pending(&mut self, _core_id: u8, _slot: u8) {}

    /// ESP32-C3 (RISC-V) external-interrupt delivery: the level-sensitive
    /// bitmask of CPU interrupt lines (1..31) currently asserted, after the bus
    /// has routed asserted peripheral sources (and the SYSTEM FROM_CPU IPI
    /// registers) through the INTERRUPT_CORE0 matrix MAP registers. The RISC-V
    /// core ORs this into `mip` when deciding whether to trap, so a source that
    /// de-asserts (e.g. the FROM_CPU yield register cleared by the ISR) drops
    /// its line on the next tick — no latching. Default 0 for buses that don't
    /// model it (the line stays low and nothing changes).
    fn external_irq_lines(&self) -> u32 {
        0
    }

    /// Event-scheduler Gap-#1 hook: `true` iff an MMIO write executed since the
    /// last `drain_scheduler_events` armed a peripheral event that has not yet
    /// been moved into the scheduler heap (it is sitting in `pending_schedule`).
    /// A CPU batch loop polls this after each interpreted instruction while the
    /// tick interval is widened (> 1) so it can END the batch on the arming
    /// write — the just-armed event is then enqueued by the post-batch drain and
    /// the NEXT batch's `next_event_deadline` clamp delivers it at its exact
    /// cycle, instead of the batch overrunning the deadline. O(1); default false
    /// for buses that don't model the scheduler.
    #[cfg(feature = "event-scheduler")]
    fn has_pending_schedule(&self) -> bool {
        false
    }

    /// Earliest absolute cycle among events sitting in `pending_schedule`
    /// (not yet on the scheduler heap). Used by the CPU batch loop to **clamp**
    /// the remaining batch to that deadline instead of ending on the first arm
    /// (far-future timers would otherwise force ~tens-of-instruction batches).
    /// Fidelity: we still never retire past the deadline without a drain.
    /// Default `None`.
    #[cfg(feature = "event-scheduler")]
    fn earliest_pending_deadline(&self) -> Option<u64> {
        None
    }

    /// The bus's mirrored "current cycle" (what lazy peripherals read through the
    /// shared `CycleClock`). A CPU batch loop reads this once at batch entry to
    /// learn the batch-start cycle, then republishes the EXACT cycle before each
    /// interpreted instruction via [`Self::publish_cycle`] so a mid-batch MMIO
    /// read of a lazily-derived counter sees `batch_start + retired` — the same
    /// value interval-1 would show — instead of the stale batch-start value.
    /// Default 0 for buses that don't model a cycle clock.
    #[cfg(feature = "event-scheduler")]
    fn current_cycle(&self) -> u64 {
        0
    }

    /// Republish the shared `CycleClock` to `cycle` (see [`Self::current_cycle`]).
    /// Called per interpreted instruction while the tick interval is widened, so
    /// the cost is a single relaxed atomic store on the hot path. Default no-op.
    #[cfg(feature = "event-scheduler")]
    fn publish_cycle(&mut self, _cycle: u64) {}

    /// Plan 2: deliver a coherent 32-bit value to peripherals after the
    /// four byte writes that compose a write_u32 have been dispatched.
    /// Default: no-op for buses that don't route to peripherals.
    fn notify_word_write(&mut self, _addr: u64, _value: u32) -> SimResult<()> {
        Ok(())
    }

    /// The bus's shared push-mode logic tap, when it carries one (a cheap
    /// `Arc` clone). CPU batch loops use it to advance the tap clock once per
    /// retired instruction while push capture is armed; `None` (the default)
    /// means no tap and no per-instruction work.
    fn logic_tap(&self) -> Option<logic_capture::LogicTap> {
        None
    }

    /// Plan 3: look up a registered ROM thunk by absolute PC. Used by the
    /// Xtensa LX7 `BREAK 1, 14` dispatch to redirect calls into the simulated
    /// ESP32-S3 mask ROM. Default returns None for buses that don't model
    /// ROM thunks.
    fn get_rom_thunk(
        &self,
        _pc: u32,
    ) -> Option<crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkFn> {
        None
    }

    fn read_u16(&self, addr: u64) -> SimResult<u16> {
        let b0 = self.read_u8(addr)? as u16;
        let b1 = self.read_u8(addr + 1)? as u16;
        // Little Endian
        Ok(b0 | (b1 << 8))
    }

    fn read_u32(&self, addr: u64) -> SimResult<u32> {
        let b0 = self.read_u8(addr)? as u32;
        let b1 = self.read_u8(addr + 1)? as u32;
        let b2 = self.read_u8(addr + 2)? as u32;
        let b3 = self.read_u8(addr + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write_u8(addr + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write_u8(addr + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    fn write_u16(&mut self, addr: u64, value: u16) -> SimResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }

    /// Optional fast-path for instruction fetch: return a contiguous
    /// `&[u8]` covering `pc`, plus the absolute `[range_start, range_end)`
    /// it serves. The CPU caches this slice on the side and reads
    /// instructions directly out of it without round-tripping the bus
    /// dispatcher, peripheral lookup, or `RefCell` borrow.
    ///
    /// Default returns `None`; buses that can serve linear memory
    /// (e.g. `SystemBus` when `pc` lands in a `RamPeripheral`) should
    /// return `Some`. Callers MUST treat the slice as read-only and
    /// invalidate their cached pointer on any bus write that may touch
    /// the same range and on snapshot restore. See labwired-core#119
    /// (JIT roadmap Phase 1.2).
    fn fetch_slice(&self, _pc: u64) -> Option<(u64, u64, &[u8])> {
        None
    }
}

use std::collections::HashSet;

/// Trait for controlling the machine in debug mode
pub trait DebugControl {
    fn add_breakpoint(&mut self, addr: u32);
    fn remove_breakpoint(&mut self, addr: u32);
    fn clear_breakpoints(&mut self);

    /// Run until breakpoint or steps limit
    fn run(&mut self, max_steps: Option<u32>) -> SimResult<StopReason>;

    /// Step a single instruction
    fn step_single(&mut self) -> SimResult<StopReason>;

    fn read_core_reg(&self, id: u8) -> u32;
    fn write_core_reg(&mut self, id: u8, val: u32);

    fn read_memory(&self, addr: u32, len: usize) -> SimResult<Vec<u8>>;
    fn write_memory(&mut self, addr: u32, data: &[u8]) -> SimResult<()>;

    fn get_pc(&self) -> u32;
    fn set_pc(&mut self, addr: u32);
    fn get_register_names(&self) -> Vec<String>;
    fn get_cycle_count(&self) -> u64;
    fn get_peripherals(&self) -> Vec<(String, u64, u64)>;
    fn get_peripheral_descriptor(
        &self,
        name: &str,
    ) -> Option<labwired_config::PeripheralDescriptor>;
    fn reset(&mut self) -> SimResult<()>;

    // State Management
    fn snapshot(&self) -> snapshot::MachineSnapshot;
    fn restore(&mut self, snapshot: &snapshot::MachineSnapshot) -> SimResult<()>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    Breakpoint(u32),
    StepDone,
    MaxStepsReached,
    ManualStop,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// Counters collected during one measured machine run.
///
/// These are execution-path counters, not wall-clock or workload markers:
/// `cpu_instructions` counts retired instructions, `cpu_batches` counts CPU
/// dispatch batches, and the remaining fields describe peripheral/bus work
/// driven by that same pass. Workload-specific observables (for example an
/// OLED first-paint cycle or a serial completion marker) belong to the
/// workload harness and must not be added here.
pub struct StepProfile {
    pub cpu_instructions: u64,
    pub cpu_batches: u64,
    pub peripheral_ticks: u64,
    pub peripheral_ticked_entries: u64,
    pub bus_tick_entries: u64,
    pub legacy_tick_entries: u64,
}

pub struct Machine<C: Cpu> {
    pub cpu: C,
    /// Secondary CPU instance — for dual-core SoCs (ESP32, ESP32-S3).
    /// When `Some`, `step()` interleaves CPU 0 and CPU 1 instructions
    /// over a shared bus. `None` for everything else (Cortex-M, RP2040,
    /// pre-existing single-core Xtensa configs that don't opt in).
    /// Snapshot/restore APIs currently track only the primary CPU —
    /// callers that need full dual-core snapshots must extend the format.
    pub cpu_secondary: Option<C>,
    pub bus: bus::SystemBus,
    pub observers: Vec<Arc<dyn SimulationObserver>>,

    // Debug state
    pub breakpoints: std::collections::HashSet<u32>,
    pub last_breakpoint: Option<u32>,
    pub total_cycles: u64,
    /// Cumulative CPU cycles advanced by idle fast-forward (WFI skip), not
    /// interpreted. Lets the browser `?perf=1` HUD prove FF is firing; 0 means
    /// either FF is off or firmware never parks in a skippable idle.
    pub idle_fast_forward_cycles_skipped: u64,
    pub config: SimulationConfig,
    step_profile: StepProfile,

    /// Phase 2B.1 (issue #192): event-driven peripheral scheduler. Active
    /// behaviour is gated behind the `event-scheduler` feature; the field
    /// is always present so other crates can borrow it without cfg gates.
    pub sched: sched::EventScheduler,
    /// Phase 2B.1: clock domain registry + observer fan-out. Wiring to
    /// real ESP32 register writes lands with the DPORT / RTC_CNTL
    /// migration PRs (design §12a).
    pub clocks: sched::ClockGraph,
    /// Cached bus index of the (single) RTC_CNTL peripheral, resolved at
    /// construction. `step()` drains the SW_SYS_RST latch every cycle; the
    /// pre-cache code walked the full ~40-peripheral list and downcast each
    /// to `RtcCntl` per cycle. With the index cached this collapses to one
    /// indexed access + one downcast. `None` for configs that don't register
    /// an RTC_CNTL peripheral (every non-ESP32-classic target).
    rtc_cntl_index: Option<usize>,
    /// Cached bus index of the FLASH peripheral (H5 layout). Resolved once at
    /// construction; `step()` drains pending FLASH ops (sector erase,
    /// bank-swap+reset) at clean instruction boundaries without walking the
    /// full peripheral list every cycle. `None` for configs with no FLASH
    /// peripheral on the bus (e.g. bare-bus unit tests).
    flash_index: Option<usize>,
    /// Cached bus index of the SCB peripheral (Cortex-M). Resolved once at
    /// construction; `step()` drains a pending SYSRESETREQ latch every cycle
    /// and, when set, reboots the CPU through the vector table via the
    /// existing `Machine::reset`. `None` for non-Cortex-M targets (no SCB on
    /// the bus), so the per-cycle drain short-circuits without a peripheral
    /// walk or downcast.
    scb_index: Option<usize>,
    /// Phase 2B.3b (issue #192): whether the one-time scheduler bootstrap has
    /// run. On the first `drain_scheduler_events`, peripherals with setup-time
    /// work (e.g. a UART with an RX stream attached before any MMIO write) get
    /// one chance to schedule their initial events. Always present; only read
    /// under the `event-scheduler` feature.
    #[allow(dead_code)]
    scheduler_bootstrapped: bool,
    /// Reusable scratch for `drain_scheduler_events`'s HC-SR04 edge harvest, so
    /// the per-drain (per-cycle at tick interval 1) harvest allocates nothing on
    /// the steady-state path. Holds `(sensor_idx, rise_tick, fall_tick)` and is
    /// drained each call. Always present; only used under the `event-scheduler`
    /// feature.
    #[allow(dead_code)]
    hcsr04_edge_scratch: Vec<(usize, u64, u64)>,
    /// Reusable scratch for the per-tick `tick_peripherals_fully` interrupt
    /// harvest, so the steady-state peripheral tick pushes pending NVIC IRQs
    /// into a retained buffer instead of allocating a fresh `Vec` every tick
    /// (the ~731k `RawVec::grow_one` the callgrind profile blamed on the C3
    /// SYSTIMER tick). Cleared, not reallocated, each tick. Same pattern as
    /// [`Self::hcsr04_edge_scratch`].
    tick_irq_scratch: Vec<u32>,
    /// Reusable scratch for the per-tick peripheral-cost list, paired with
    /// [`Self::tick_irq_scratch`]. Empty on the walk-free fast path.
    tick_cost_scratch: Vec<bus::PeripheralTickCost>,
    /// Reusable scratch for `drain_scheduler_events`'s pending-schedule harvest.
    /// Swapped with `bus.pending_schedule` and drained in place so the drain
    /// reuses the buffer's capacity instead of `mem::take` freeing and later
    /// reallocating a fresh `Vec` every time write-context events were buffered.
    /// Semantics are identical (same entries, same order). Always present; only
    /// used under the `event-scheduler` feature.
    #[allow(dead_code)]
    pending_schedule_scratch: Vec<(usize, u64, u32)>,
    /// Reusable scratch for the due-events batch drained out of the scheduler
    /// each `drain_scheduler_events`. Taken out, filled by
    /// `EventScheduler::drain_due_into`, iterated, then restored — so the
    /// steady-state SYSTIMER tick (which drains an event nearly every batch)
    /// reuses the buffer's capacity instead of allocating a fresh `Vec` per
    /// drain. Always present; only used under the `event-scheduler` feature.
    #[allow(dead_code)]
    due_events_scratch: Vec<sched::ScheduledEvent>,
    /// Reusable no-op stand-in for the peripheral swap-out dance in
    /// `drain_scheduler_events` (a peripheral's `on_event` needs `&mut self.bus`,
    /// so the peripheral is temporarily replaced by a stub). Held here and
    /// swapped in/out so the hot scheduler event path (the ESP32-C3 SYSTIMER
    /// alarm fires one per drain) does not `Box::new` + free a fresh stub every
    /// event. Always `Some` between events. Only used under `event-scheduler`.
    #[allow(dead_code)]
    event_placeholder: Option<Box<dyn Peripheral>>,

    /// In-engine logic-analyzer edge capture (see [`crate::logic_capture`]).
    /// Empty/inactive by default — the step loop pays a single `is_active`
    /// check per step and nothing more until `logic_watch` installs a watch
    /// set. Not part of snapshot/restore: capture is a UI observation stream,
    /// re-armed by the frontend after a resume.
    logic_capture: logic_capture::LogicCapture,
    /// Test-only forcing knob (see [`Machine::logic_force_poll_capture`]):
    /// when `true`, `logic_watch` keeps every channel on the per-cycle poll
    /// path even for push-instrumented peripherals. This is what the
    /// differential oracle tests use to compare the two capture modes; it is
    /// NOT user-facing configuration.
    logic_force_poll: bool,
}

impl<C: Cpu> Machine<C> {
    /// Whether any logic-analyzer / signal probe is armed (poll or push mode).
    /// The `jit_framework` [`SafetyGate`](crate::cpu::jit_framework::fallback::SafetyGate)
    /// reads this to force the interpreter while a probe needs per-cycle pad
    /// visibility. `logic_capture` is module-private, so this crate-internal
    /// accessor is how the RISC-V JIT host reaches it.
    #[cfg(any(feature = "jit", feature = "jit-framework"))]
    pub(crate) fn logic_probes_active(&self) -> bool {
        self.logic_capture.poll_active() || self.logic_capture.push_active()
    }

    /// Discover the drivable input channels on this machine (delegates to
    /// [`bus::SystemBus::list_inputs`]). See [`crate::sim_input`].
    pub fn list_inputs(&mut self) -> Vec<(String, crate::sim_input::InputChannel)> {
        self.bus.list_inputs()
    }

    /// Drive a simulated input channel to `value` (delegates to
    /// [`bus::SystemBus::set_input`]). The generic entry point an agent, the
    /// WASM bridge, or a test-script stimulus calls to steer an input device
    /// mid-run without knowing its concrete type.
    pub fn set_input(
        &mut self,
        channel: &str,
        value: f64,
    ) -> Result<(), crate::sim_input::SimInputError> {
        self.bus.set_input(None, channel, value)
    }

    /// [`Machine::set_input`] narrowed to the device named `component` — the
    /// external-device id from system.yaml (stamped onto the model at attach)
    /// or the owning peripheral's bus name; the disambiguator a test-script
    /// stimulus `target.component` resolves through when two devices expose
    /// the same channel key.
    pub fn set_input_on(
        &mut self,
        component: &str,
        channel: &str,
        value: f64,
    ) -> Result<(), crate::sim_input::SimInputError> {
        self.bus.set_input(Some(component), channel, value)
    }

    /// Apply several input sets as one atomic transaction (delegates to
    /// [`bus::SystemBus::set_inputs`]): every set is validated first and
    /// either all apply or none do, with no execution in between — the way to
    /// drive a multi-channel pose (an IMU's x/y/z, a GPS lat+lon) without the
    /// firmware ever observing a torn update.
    pub fn set_inputs(
        &mut self,
        sets: &[(Option<&str>, &str, f64)],
    ) -> Result<(), crate::sim_input::SimInputError> {
        self.bus.set_inputs(sets)
    }

    /// Install a logic-analyzer watch set, resetting the capture buffer and
    /// cursor. `resolved[i]` is `Some((peripheral_index, pin))` for a
    /// resolvable GPIO ref or `None` for an unresolvable one (never sampled).
    /// Returns each channel's initial pad level (`None` = unknown), same order
    /// as `resolved`, so the caller can seed the waveform before the first
    /// edge. Passing an empty slice disarms capture.
    ///
    /// Each resolvable channel is armed in one of two modes: push
    /// (event-driven — the owning peripheral accepted
    /// [`Peripheral::install_logic_tap`] and reports pad writes itself) or the
    /// per-cycle poll fallback. See [`crate::logic_capture`].
    pub fn logic_watch(&mut self, resolved: &[Option<(usize, u8)>]) -> Vec<Option<bool>> {
        // Group the watch set per owning peripheral as (pin, channel) pairs.
        let mut per_peripheral: std::collections::HashMap<usize, Vec<(u8, u32)>> =
            std::collections::HashMap::new();
        if !self.logic_force_poll {
            for (ch, r) in resolved.iter().enumerate() {
                if let Some((idx, pin)) = *r {
                    per_peripheral
                        .entry(idx)
                        .or_default()
                        .push((pin, ch as u32));
                }
            }
        }

        // Offer every peripheral its slice of the watch set (empty ⇒ clears a
        // previously installed tap). Whether a peripheral ACCEPTS is its own
        // declaration of push capability — no hardcoded list here.
        let tap = self.bus.logic_tap.clone();
        let mut push = vec![false; resolved.len()];
        static EMPTY: [(u8, u32); 0] = [];
        for (idx, p) in self.bus.peripherals.iter_mut().enumerate() {
            let watched: &[(u8, u32)] = per_peripheral.get(&idx).map_or(&EMPTY, |v| v.as_slice());
            let accepted = p.dev.install_logic_tap(&tap, watched);
            if accepted {
                for &(_, ch) in watched {
                    push[ch as usize] = true;
                }
            }
        }

        let bus = &self.bus;
        let initial: Vec<Option<bool>> = resolved
            .iter()
            .map(|r| {
                r.and_then(|(idx, pin)| {
                    bus.peripherals
                        .get(idx)
                        .and_then(|p| p.dev.read_gpio_pad(pin))
                })
            })
            .collect();
        self.logic_capture.install(resolved, &initial, &push);

        // Arm the tap clock at "the next observation boundary" so pushes that
        // happen before any stepping (e.g. a paused-machine input change)
        // stamp where the first post-watch sample would observe them.
        tap.clear_events();
        tap.set_clock(self.total_cycles + 1);
        tap.set_armed(self.logic_capture.push_active());
        initial
    }

    /// Test-only forcing knob for the differential capture oracle: when
    /// `true`, subsequent [`logic_watch`](Self::logic_watch) calls keep every
    /// channel on the per-cycle poll path even where push instrumentation
    /// exists (the batch clamp and idle-fast-forward disable apply again).
    /// Takes effect at the next `logic_watch`. Not user-facing configuration —
    /// it exists so tests can assert push and poll produce byte-identical
    /// edge streams.
    #[doc(hidden)]
    pub fn logic_force_poll_capture(&mut self, force: bool) {
        self.logic_force_poll = force;
    }

    /// Read logic edges newer than `cursor`, acknowledging retained edges
    /// before it (see [`logic_capture::LogicCapture::read_edges`]).
    pub fn logic_read_edges(&mut self, cursor: u64) -> logic_capture::LogicEdgeBatch {
        self.logic_capture.read_edges(cursor)
    }

    /// Current engine cycle — the `nowCycle` reported alongside a logic-edge
    /// read so the UI can extend flat traces to "now".
    pub fn logic_now_cycle(&self) -> u64 {
        self.total_cycles
    }

    /// `true` while at least one watched channel is on the per-cycle poll
    /// fallback. Frontends inspect this when choosing an outer request batch
    /// limit; [`Machine::advance`] independently clamps each internal batch to
    /// one so polled pads are sampled at every cycle boundary. Push-only watch
    /// sets keep the full batch width because their peripherals report edges
    /// from the write sites.
    #[inline]
    pub fn logic_poll_active(&self) -> bool {
        self.logic_capture.poll_active()
    }

    /// Observe the watched channels at the current cycle boundary: drain the
    /// push tap (event-driven channels) and sample the polled channels.
    /// Hooked into the step loop; the leading `is_active` guard is the entire
    /// cost when nothing is watched.
    ///
    /// `boundary` is the engine cycle at the end of the just-executed
    /// instruction batch, BEFORE any peripheral tick-cost cycles were charged
    /// — pushes stamped at it are finalised to `total_cycles` ("now"), which
    /// is where a per-cycle poll would have seen them.
    #[inline]
    fn logic_observe(&mut self, boundary: u64) {
        if !self.logic_capture.is_active() {
            return;
        }
        let now = self.total_cycles;
        if self.logic_capture.push_active() {
            let events = self.bus.logic_tap.take_events();
            if !events.is_empty() {
                self.logic_capture.ingest_push(&events, boundary, now);
            }
            // Re-arm the provisional stamp at the NEXT boundary so pad writes
            // arriving while the machine is paused (sim-input, button pushes)
            // stamp where the first post-resume observation would see them.
            self.bus.logic_tap.set_clock(now + 1);
        }
        if self.logic_capture.poll_active() {
            let bus = &self.bus;
            self.logic_capture.sample(now, |idx, pin| {
                bus.peripherals
                    .get(idx)
                    .and_then(|p| p.dev.read_gpio_pad(pin))
            });
        }
    }
}

impl<C: Cpu> Machine<C> {
    pub fn new(cpu: C, bus: bus::SystemBus) -> Self {
        let rtc_cntl_index = bus.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::rtc_cntl::RtcCntl>())
                .is_some()
        });
        let flash_index = bus.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::flash::Flash>())
                .is_some()
        });
        let scb_index = bus.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::scb::Scb>())
                .is_some()
        });
        Self {
            cpu,
            cpu_secondary: None,
            bus,
            observers: Vec::new(),
            breakpoints: HashSet::new(),
            last_breakpoint: None,
            total_cycles: 0,
            idle_fast_forward_cycles_skipped: 0,
            config: SimulationConfig::default(),
            step_profile: StepProfile::default(),
            sched: sched::EventScheduler::new(),
            clocks: sched::ClockGraph::new(),
            rtc_cntl_index,
            flash_index,
            scb_index,
            scheduler_bootstrapped: false,
            hcsr04_edge_scratch: Vec::new(),
            tick_irq_scratch: Vec::new(),
            tick_cost_scratch: Vec::new(),
            pending_schedule_scratch: Vec::new(),
            due_events_scratch: Vec::new(),
            event_placeholder: Some(Box::new(crate::peripherals::stub::StubPeripheral::new(0))),
            logic_capture: logic_capture::LogicCapture::new(),
            logic_force_poll: false,
        }
    }

    /// Enable dual-core mode by attaching a secondary CPU instance.
    /// The secondary CPU shares the same bus and steps in lockstep with
    /// the primary (one instruction each per `Machine::step()`). Used by
    /// ESP32 configs to model APP_CPU alongside PRO_CPU.
    pub fn with_secondary_cpu(mut self, cpu1: C) -> Self {
        self.cpu_secondary = Some(cpu1);
        self
    }

    pub fn reset_step_profile(&mut self) {
        self.step_profile = StepProfile::default();
    }

    pub fn step_profile(&self) -> StepProfile {
        self.step_profile
    }

    fn record_peripheral_tick_profile(&mut self, cost_entries: usize) {
        let (bus_tick_entries, legacy_tick_entries) = self.bus.tick_profile_entry_counts();
        self.step_profile.peripheral_ticks += 1;
        self.step_profile.peripheral_ticked_entries += cost_entries as u64;
        self.step_profile.bus_tick_entries += bus_tick_entries as u64;
        self.step_profile.legacy_tick_entries += legacy_tick_entries as u64;
    }

    fn try_idle_fast_forward(
        &mut self,
        _max_steps: Option<u64>,
        _steps: u64,
        breakpoints_block_idle: bool,
    ) -> u64 {
        // POLLED logic capture disables the skip: a scheduler event inside the
        // skipped window could toggle a watched pad, and the per-cycle poll
        // guarantee must hold even under this opt-in acceleration. Push-only
        // watch sets keep the skip — a pad write inside the window happens in
        // instrumented peripheral code, which pushes its own edge with the
        // post-skip tap clock (seeded below before the scheduler drain).
        if !self.config.idle_fast_forward_enabled
            || breakpoints_block_idle
            || self.bus.requires_cycle_accurate()
            || self.logic_capture.poll_active()
        {
            return 0;
        }

        #[cfg(not(feature = "event-scheduler"))]
        {
            0
        }

        #[cfg(feature = "event-scheduler")]
        {
            if !self.bus.idle_fast_forward_legacy_safe() {
                return 0;
            }

            // Drain first: a due scheduler event may assert an interrupt, in
            // which case the CPU must resume normally instead of skipping.
            self.drain_scheduler_events();

            // Two coalesce sources under the same idle-FF flag:
            // 1) Architectural WFI / wait-for-interrupt (existing).
            // 2) Pure freerunning-timer poll batches (peripherals opt in via
            //    MmioAccessClass::FreerunningTimerPoll — e.g. ESP SYSTIMER
            //    snapshot regs for Arduino millis). Device time still advances
            //    to the next scheduled event; we only skip empty spin. Bus
            //    stays CPU-agnostic: chip register maps live in peripheral
            //    models, not SystemBus.
            let wfi_budget = self.cpu.idle_fast_forward_budget(&self.bus);
            let timer_poll = self.bus.take_timer_poll_coalesce_eligible();
            let mut budget = match wfi_budget {
                Some(budget) if budget > 0 => budget,
                _ if timer_poll => u64::MAX,
                _ => return 0,
            };
            let remaining = _max_steps
                .map(|limit| limit.saturating_sub(_steps))
                .unwrap_or(1_000_000);
            if remaining == 0 {
                return 0;
            }
            budget = budget.min(remaining);

            if let Some(deadline_cycle) = self.sched.next_event_deadline() {
                if deadline_cycle <= self.total_cycles {
                    return 0;
                }
                budget = budget.min(deadline_cycle - self.total_cycles);
            } else if timer_poll && wfi_budget.is_none() {
                // No scheduled event and not WFI: cap pure millis spins so a
                // single batch cannot leap the whole step budget when the heap
                // is empty (still advances real device time, just bounded).
                budget = budget.min(1_000_000);
            }

            // Tiny windows are not worth the skip bookkeeping on the timer-poll
            // path (WFI may still skip short waits).
            if budget == 0 || (timer_poll && wfi_budget.is_none() && budget < 1024) {
                return 0;
            }
            let skipped = budget.min(u32::MAX as u64) as u32;
            self.cpu.fast_forward_idle_cycles(skipped as u64);
            self.total_cycles += skipped as u64;
            self.idle_fast_forward_cycles_skipped += skipped as u64;
            self.bus.set_current_cycle(self.total_cycles);
            self.bus.bus_trace.set_cycle(self.total_cycles);
            // Push-mode logic capture: stamp any pad writes made by the
            // scheduler events due at the end of the skipped window with the
            // cycle we skipped to (the events' own deadline — the budget was
            // clamped to it above), keeping edges deterministic and correctly
            // placed inside what would otherwise be a silent window.
            if self.logic_capture.push_active() {
                self.bus.logic_tap.set_clock(self.total_cycles);
            }
            self.sched.advance_to(self.total_cycles);
            self.drain_scheduler_events();
            u64::from(skipped)
        }
    }

    /// Take a binary mid-flight runtime snapshot of the entire machine —
    /// CPU + every peripheral that overrides `runtime_snapshot()`. The
    /// returned blob feeds straight into [`Self::apply_runtime_snapshot`]
    /// on a freshly-constructed `Machine` with the same firmware loaded
    /// and the same bus topology.
    ///
    /// Distinct from [`Self::snapshot`] (which produces a JSON value for
    /// the determinism gates) — this one is binary, captures everything
    /// needed for resume (full SR file, shadow stacks, RAM regions, etc.)
    /// and is what the playground uses to ship pre-warmed boot snapshots
    /// alongside firmware ELFs.
    pub fn take_runtime_snapshot(&self) -> runtime_snapshot::MachineRuntimeSnapshot {
        let (cpu_kind, cpu_data) = self.cpu.runtime_snapshot();
        let peripherals: Vec<(String, Vec<u8>)> = self
            .bus
            .peripherals
            .iter()
            .filter(|entry| {
                !runtime_snapshot::RUNTIME_SNAPSHOT_SKIPPED_PERIPHERALS
                    .iter()
                    .any(|skip| *skip == entry.name)
            })
            .map(|entry| (entry.name.clone(), entry.dev.runtime_snapshot()))
            .collect();
        let mut snap =
            runtime_snapshot::MachineRuntimeSnapshot::new(cpu_kind, cpu_data, peripherals);
        // RISC-V (ESP32-C3 rom-boot) keeps its live program state — `.data`,
        // `.bss`, stacks — in the flat SRAM/IRAM linear windows (`bus.ram` +
        // `bus.extra_mem`), NOT in RamPeripherals like the Xtensa configs. So a
        // faithful resume must carry those windows. Xtensa/Arm snapshots leave
        // `memories` empty: their RAM is peripheral-backed (already captured
        // above) and their linear windows are firmware mirrors re-derived on
        // load. Untouched (all-zero) extra_mem windows — e.g. the flash-mapped
        // DROM mirror — are skipped so the blob stays compact.
        if cpu_kind == runtime_snapshot::CpuKind::RiscV {
            let mut memories = vec![(self.bus.ram.base_addr, self.bus.ram.data.clone())];
            for mem in &self.bus.extra_mem {
                if mem.data.iter().any(|&b| b != 0) {
                    memories.push((mem.base_addr, mem.data.clone()));
                }
            }
            snap.memories = memories;
        }
        snap
    }

    /// Restore from a previously-taken runtime snapshot. Bus topology
    /// must match (same peripherals registered under the same names) —
    /// peripherals not present in the snapshot keep their current state,
    /// peripherals named in the snapshot but missing on the bus return
    /// `MissingPeripheral` so the caller can fail loudly instead of
    /// silently dropping state.
    pub fn apply_runtime_snapshot(
        &mut self,
        snap: &runtime_snapshot::MachineRuntimeSnapshot,
    ) -> SimResult<()> {
        self.cpu
            .apply_runtime_snapshot(snap.cpu_kind, &snap.cpu_data)?;
        for (name, blob) in &snap.peripherals {
            let entry = self
                .bus
                .peripherals
                .iter_mut()
                .find(|e| &e.name == name)
                .ok_or_else(|| {
                    SimulationError::NotImplemented(format!(
                        "apply_runtime_snapshot: peripheral '{name}' not on bus"
                    ))
                })?;
            entry.dev.restore_runtime_snapshot(blob)?;
        }
        // Restore flat linear windows (RISC-V SRAM/IRAM; empty for other
        // arches). Match each captured window to its live backing by base
        // address and fail loudly on a topology/size mismatch — a resume must
        // be applied on the same machine layout it was captured on.
        for (base, bytes) in &snap.memories {
            let target = if *base == self.bus.ram.base_addr {
                Some(&mut self.bus.ram)
            } else {
                self.bus.extra_mem.iter_mut().find(|m| m.base_addr == *base)
            };
            let mem = target.ok_or_else(|| {
                SimulationError::NotImplemented(format!(
                    "apply_runtime_snapshot: no memory window @ {base:#010x} on bus"
                ))
            })?;
            if mem.data.len() != bytes.len() {
                return Err(SimulationError::NotImplemented(format!(
                    "apply_runtime_snapshot: memory @ {base:#010x} size {} != snapshot {}",
                    mem.data.len(),
                    bytes.len()
                )));
            }
            mem.data.copy_from_slice(bytes);
        }
        self.bus.refresh_peripheral_index();
        Ok(())
    }
}

impl<C: Cpu> Machine<C> {
    pub fn load_firmware(&mut self, image: &memory::ProgramImage) -> SimResult<()> {
        for segment in &image.segments {
            // 1. Try Cortex-M / ARM flash backing.
            if self.bus.flash.load_from_segment(segment) {
                continue;
            }
            // 2. Try Cortex-M / ARM ram backing.
            if self.bus.ram.load_from_segment(segment) {
                continue;
            }
            // 2b. Try extra CPU-visible memory windows (ESP32 IRAM/DROM, etc).
            if self
                .bus
                .extra_mem
                .iter_mut()
                .any(|m| m.load_from_segment(segment))
            {
                continue;
            }
            // 3. Fall back to peripheral-backed memory (ESP32 IRAM /
            //    flash XIP, RP2040 SRAM, etc — anything registered as a
            //    Peripheral rather than living in bus.flash/bus.ram).
            //    Walk the segment byte-by-byte through the bus dispatcher.
            //    Slow path, but only runs once at load time.
            let mut all_written = true;
            for (i, byte) in segment.data.iter().enumerate() {
                let addr = segment.start_addr + i as u64;
                if self.bus.write_u8(addr, *byte).is_err() {
                    all_written = false;
                    break;
                }
            }
            if !all_written {
                tracing::warn!(
                    "Failed to load segment at {:#x} - outside of memory map",
                    segment.start_addr
                );
            }
        }

        for observer in &self.observers {
            observer.on_simulation_start();
        }
        self.reset()?;

        // Resolve the reset vector. Reset reads the initial (SP, PC) from the
        // vector table at VTOR, which defaults to 0 and aliases to the flash
        // base — correct for the common case (STM32/nRF/etc.). Some SoCs
        // prepend a second-stage bootloader: the RP2040 bootrom runs a 256-byte
        // stage-2 (boot2) blob from flash and only then enters the application
        // vector table at `flash_base + reset_vector_offset`. We don't execute
        // boot2 (flash is directly mapped), so when the flash-base vectors are
        // not valid, relocate to the declared post-stage-2 table — emulating
        // boot2's only observable effect.
        let flash_base = self.bus.flash.base_addr;
        if !self.bus.vector_pair_valid(flash_base) {
            let offset = self.bus.reset_vector_offset;
            let relocated = if offset != 0 {
                let table = flash_base + offset;
                if self.bus.vector_pair_valid(table) {
                    let sp = self.bus.read_u32(table)?;
                    let pc = self.bus.read_u32(table + 4)? & !1;
                    // Point VTOR at the relocated table so early exceptions
                    // (before firmware sets VTOR itself) vector correctly.
                    let _ = self.bus.write_u32(0xE000_ED08, table as u32);
                    self.cpu.set_sp(sp);
                    self.cpu.set_pc(pc);
                    true
                } else {
                    false
                }
            } else {
                false
            };

            // Last-resort fallback if the vector table is missing/zero.
            if !relocated && self.cpu.get_pc() == 0 {
                self.cpu.set_pc(image.entry_point as u32);
            }
        }

        Ok(())
    }

    pub fn reset(&mut self) -> SimResult<()> {
        self.cpu.reset(&mut self.bus)?;
        if let Some(cpu1) = self.cpu_secondary.as_mut() {
            cpu1.reset(&mut self.bus)?;
        }
        Ok(())
    }

    /// Advances one primary-CPU boundary through the authoritative lifecycle.
    ///
    /// This compatibility adapter delegates to [`Machine::advance`]. Frontends
    /// that need bounded runs or stop reports should call `advance` directly;
    /// they must not reproduce the lifecycle with direct [`Cpu::step`] calls.
    pub fn step(&mut self) -> SimResult<()> {
        self.advance(AdvanceRequest::single()).map(|_| ())
    }

    /// Drain and apply the single pending H5 FLASH hardware operation, if any.
    ///
    /// The FLASH peripheral records at most one op per instruction (in a `Cell`);
    /// this helper must therefore run once per instruction so no op is lost. It
    /// is called from the authoritative `Machine::advance` batch loop. The
    /// run loop clamps its batch to 1 when `requires_cycle_accurate()` is true
    /// (which an H5 op-modeling FLASH forces), preserving the one-op-per-
    /// instruction invariant and the correct erase-before-program ordering.
    pub(crate) fn apply_pending_flash_op(&mut self) -> SimResult<()> {
        if let Some(op) = self.drain_flash_op() {
            use crate::peripherals::flash::h5;
            use crate::peripherals::flash::FlashOp;
            match op {
                FlashOp::EraseSector { bank, sector } => {
                    // OPT-IN read-while-write fidelity gate (H5 only, default
                    // off). On real STM32H563 silicon a bank cannot be fetched
                    // from while it is being erased — code running from that bank
                    // stalls and cannot make progress, so production flash
                    // routines run from SRAM. When the gate is on, an erase of
                    // the SAME physical bank the CPU is currently executing from
                    // is an unrecoverable access fault rather than a silent
                    // success. The bank comparison is SWAP_BANK-aware: both the
                    // BKSEL logical erase bank and PC's bank are mapped to a
                    // physical bank through the active swap state (see
                    // `Flash::rww_erase_violates`). Gate off ⇒ this branch is
                    // skipped entirely and the erase proceeds as before.
                    let pc = self.cpu.get_pc() as u64;
                    let in_flash =
                        (h5::FLASH_BASE..h5::FLASH_BASE + 2 * h5::BANK_SIZE).contains(&pc);
                    if let Some(flash) = self.flash_peripheral() {
                        if flash.h5_rww_enabled()
                            && in_flash
                            && flash.rww_erase_violates(bank, pc - h5::FLASH_BASE)
                        {
                            let phys = flash.physical_bank_of_offset(pc - h5::FLASH_BASE);
                            return Err(SimulationError::Other(format!(
                                "flash RWW violation: erase of bank {phys} while executing \
                                 from bank {phys} (PC={pc:#010x}) — run the flash routine \
                                 from SRAM"
                            )));
                        }
                    }
                    let offset = (bank as u64) * h5::BANK_SIZE + (sector as u64) * h5::SECTOR_SIZE;
                    self.bus.flash.fill(offset, h5::SECTOR_SIZE, 0xFF);
                    tracing::debug!(
                        "FLASH EraseSector bank={bank} sector={sector} offset={offset:#010x}"
                    );
                }
                FlashOp::SwapAndReset => {
                    // Swap the two architectural 1 MiB (0x100000) banks. The
                    // H563 flash buffer is sized to exactly 2 * BANK_SIZE by the
                    // chip yaml (`size: "2MiB"`), so the same BANK_SIZE used by
                    // EraseSector above also bounds the swap — keeping erase and
                    // swap on one consistent bank-size notion (real silicon:
                    // bank 2 @ 0x08100000). swap_banks returns false if the
                    // buffer is not exactly two banks; debug-assert that here so
                    // a mis-sized chip yaml fails loudly in tests.
                    let swapped = self.bus.flash.swap_banks(h5::BANK_SIZE);
                    debug_assert!(
                        swapped,
                        "SWAP_BANK: flash buffer ({} bytes) is not 2 * BANK_SIZE ({}); \
                         check the chip yaml flash size uses binary (MiB) units",
                        self.bus.flash.data.len(),
                        2 * h5::BANK_SIZE
                    );
                    // Record the swap so the RWW bank mapping reflects the new
                    // active view (the physical second bank now answers at
                    // 0x08000000). No-op for the gate-off path beyond a bool.
                    if let Some(flash) = self.flash_peripheral_mut() {
                        flash.mark_swapped();
                    }
                    tracing::debug!("FLASH SwapAndReset: banks swapped, resetting CPU");
                    self.reset()?;
                }
            }
        }
        Ok(())
    }

    /// Phase 2B.1/2B.3a (issue #192): advance the scheduler and fire every due
    /// peripheral event. The authoritative [`Machine::advance`] lifecycle
    /// calls this after each committed CPU batch so no scheduler-driven
    /// peripheral is silently stranded.
    ///
    /// The scheduler runs in absolute CPU cycles (`total_cycles`) — the same
    /// quantum the legacy walk advances by (`tick_elapsed(interval)`) — so
    /// cycle-denominated peripheral delays (an SPI half-period, a systimer
    /// alarm) keep their exact meaning at any `peripheral_tick_interval`. An
    /// event lands at the first drain at or after its exact cycle; drains run
    /// at least once per CPU batch, so the observation error is bounded by one
    /// tick interval (and is zero at interval 1, where drains run per cycle).
    /// Write-context schedule requests the bus buffered during the committed
    /// batch's MMIO writes (`pending_schedule`) are enqueued first: a
    /// peripheral can't reach the scheduler from `write`, so the bus buffers
    /// an absolute cycle deadline (see `collect_scheduled_events`) which is
    /// clamped to `now` here — a deadline that expired mid-batch fires on this
    /// drain.
    #[cfg(feature = "event-scheduler")]
    fn drain_scheduler_events(&mut self) {
        // One-time bootstrap: give every scheduler-driven peripheral a chance
        // to schedule events that arise from *setup* rather than an MMIO write
        // (e.g. a UART with an RX stream attached before firmware advances, or
        // a SYSTIMER whose alarm was configured before `Machine::advance`). The
        // absolute deadline is `total_cycles + delay`, which is only exact if
        // the returned delay is measured from `total_cycles` — but a peripheral
        // is anchored at ATTACH (cycle 0) and the first drain can run after
        // `Machine::advance` has committed its first batch and advanced
        // `total_cycles`. Sync each
        // peripheral up to `total_cycles` FIRST so its delay is genuinely
        // relative to now; without this the first scheduled event lands one (or
        // up to one tick-interval) cycle late versus the legacy per-cycle walk —
        // the exact off-by-one the ESP32-S3 `intmatrix_alarm`/walk-differential
        // gate caught on a pre-advance alarm config. `sync_to` is idempotent at
        // cycle 0 (no-op) and the same call the write path already makes, so
        // steady-state behaviour is unchanged.
        if !self.scheduler_bootstrapped {
            self.scheduler_bootstrapped = true;
            let now = self.total_cycles;
            for idx in 0..self.bus.peripherals.len() {
                if self.bus.peripherals[idx].dev.uses_scheduler() {
                    self.bus.peripherals[idx].dev.sync_to(now);
                    for (delay, token) in self.bus.peripherals[idx].dev.take_scheduled_events() {
                        self.bus.pending_schedule.push((idx, now + delay, token));
                    }
                }
            }
        }
        let interval = (self.config.peripheral_tick_interval as u64).max(1);
        self.sched.advance_to(self.total_cycles);
        let now = self.sched.now();
        // Swap the buffered schedule out into retained scratch (instead of
        // `mem::take`, which frees the buffer's capacity each drain) and drain
        // it in place — same entries, same order, but the capacity is reused.
        std::mem::swap(
            &mut self.bus.pending_schedule,
            &mut self.pending_schedule_scratch,
        );
        for (idx, deadline, token) in self.pending_schedule_scratch.drain(..) {
            self.sched.schedule(deadline.max(now), idx as u32, token);
        }
        // HC-SR04: enqueue the ECHO rise/fall edges of any freshly-armed window
        // as events under the reserved subsystem idx, at their exact cycles
        // quantised up to the tick grid (per-tick reference semantics — see
        // `take_edge_schedule`). The sensor is not a `peripherals[]` entry, so
        // it can't ride the `pending_schedule` (peripheral-idx) path; it is
        // dispatched below by idx match instead.
        self.bus
            .harvest_hcsr04_edges(interval, &mut self.hcsr04_edge_scratch);
        for (sensor, rise_cycle, fall_cycle) in self.hcsr04_edge_scratch.drain(..) {
            self.sched.schedule(
                rise_cycle.max(now),
                sched::SUBSYSTEM_PERIPHERAL_IDX,
                sensor as u32,
            );
            self.sched.schedule(
                fall_cycle.max(now),
                sched::SUBSYSTEM_PERIPHERAL_IDX,
                sensor as u32,
            );
        }
        // Nothing queued (steady state between an SPI frame / HC-SR04 pulse):
        // skip the heap drain entirely — no allocation.
        if self.sched.is_empty() {
            return;
        }
        // Fill the retained scratch (taken out so `on_event` below can borrow
        // `&mut self.sched` / `&mut self.bus`) instead of allocating a fresh
        // `Vec` per drain; restored at the end with its capacity intact.
        let mut due = std::mem::take(&mut self.due_events_scratch);
        self.sched.drain_due_into(&mut due);
        for ev in due.drain(..) {
            // Bus-subsystem pseudo-peripheral (HC-SR04): no `peripherals[]`
            // entry — dispatch straight to the shared ECHO choke point.
            if ev.peripheral_idx == sched::SUBSYSTEM_PERIPHERAL_IDX {
                self.bus.apply_hcsr04_event(ev.event_token as usize);
                continue;
            }
            let idx = ev.peripheral_idx as usize;
            if idx >= self.bus.peripherals.len() {
                continue;
            }
            // Swap the peripheral out so we can pass `&mut self.bus` into
            // `on_event` without holding two simultaneous mutable borrows.
            // Same dance the bus uses for `tick_with_bus`, but the stub stand-in
            // is reused from `event_placeholder` instead of allocated per event.
            let stub = self
                .event_placeholder
                .take()
                .expect("event_placeholder present between events");
            let mut dev = std::mem::replace(&mut self.bus.peripherals[idx].dev, stub);
            let result = dev.on_event(ev.event_token, &mut self.sched, &mut self.bus);
            // Put the real peripheral back and reclaim the stub for reuse.
            let stub_back = std::mem::replace(&mut self.bus.peripherals[idx].dev, dev);
            self.event_placeholder = Some(stub_back);
            // Phase 2B.3b: a level-triggered peripheral re-arms its own event
            // (same token) while it has active work. We own the idx the
            // scheduler needs, so we do it here.
            if let Some(delay) = result.reschedule_delay {
                let deadline = self.sched.now() + delay;
                self.sched.schedule(deadline, idx as u32, ev.event_token);
            }
            self.apply_event_result(idx, result);
        }
        // `drain(..)` above emptied it but kept its capacity; restore for reuse.
        self.due_events_scratch = due;
    }

    /// Phase 2B.1 (issue #192): fan out the side-effects produced by a
    /// `Peripheral::on_event` handler. Mirrors the post-`tick()` fan-out
    /// in `tick_peripherals_phase1`: IRQ pend, system exception, mmio
    /// writes, PPI fired_events globalisation, DMA execute. No peripheral
    /// opts into the scheduler in 2B.1, so this code only runs when the
    /// `event-scheduler` feature is on AND a peripheral overrides
    /// `uses_scheduler()` in a later migration PR.
    #[cfg(feature = "event-scheduler")]
    fn apply_event_result(&mut self, peripheral_idx: usize, result: sched::EventResult) {
        let base = self.bus.peripherals[peripheral_idx].base as u32;
        let mut fallthrough: Vec<u32> = Vec::new();
        if let Some(irq) = result.raise_irq {
            self.bus.pend_irq_for_event(irq, &mut fallthrough);
        }
        // Phase 2B.3b: pend the peripheral's *own* configured NVIC line — the
        // event-path equivalent of the legacy `tick()` returning `irq: true`.
        if result.raise_own_irq {
            if let Some(irq) = self.bus.peripherals[peripheral_idx].irq {
                self.bus.pend_irq_for_event(irq, &mut fallthrough);
            }
        }
        // Scheduler-driven interrupt delivery — ONE per-fabric choke. A
        // scheduler-driven peripheral (e.g. the SYSTIMER alarm) is skipped by
        // the per-cycle walk, so the event path owns delivery of its
        // LEVEL-sensitive source at the exact firing cycle. Every MCU family
        // follows the same shape behind `deliver_scheduled_irq_levels`:
        //   * ESP32-C3 (RISC-V matrix)  → re-derive `matrix_irq_sources` into
        //     `riscv_irq_lines`;
        //   * ESP32-S3 (Xtensa intmatrix) → re-derive into `pending_cpu_irqs` +
        //     the intmatrix INTR_STATUS mirror.
        // A matrix source ID must NEVER be pended as a Cortex-M NVIC exception
        // (`pend_irq_for_event` would mis-route it), so the NVIC fallthrough is
        // taken only when no matrix fabric claimed delivery (the Cortex-M /
        // nRF SysTick + own-line path).
        if !self.bus.deliver_scheduled_irq_levels() {
            for irq in &result.explicit_irqs {
                self.bus.pend_irq_for_event(*irq, &mut fallthrough);
            }
        }
        // Phase 2B.3b: route DMA signals exactly as the legacy tick path does.
        if !result.dma_signals.is_empty() {
            let source_name = self.bus.peripherals[peripheral_idx].name.clone();
            for sig in &result.dma_signals {
                self.bus.route_dma_signal(&source_name, *sig);
            }
        }
        if let Some(exc) = result.system_exception {
            self.cpu.set_exception_pending(exc);
        }
        for irq in fallthrough {
            self.cpu.set_exception_pending(irq);
        }
        for (addr, val) in result.mmio_writes {
            if let Err(e) = self.bus.write_u32(addr as u64, val) {
                tracing::warn!("on_event mmio_write 0x{addr:08X} = 0x{val:08X} failed: {e:?}");
            }
        }
        // PPI fan-out: globalise event offsets to absolute bus addresses and
        // route through any peripheral that overrides `route_ppi_events`.
        if !result.fired_events.is_empty() {
            let fired_global: Vec<u32> = result
                .fired_events
                .iter()
                .map(|off| base.wrapping_add(*off))
                .collect();
            let mut pending_tasks: Vec<u32> = Vec::new();
            for p in self.bus.peripherals.iter_mut() {
                pending_tasks.extend(p.dev.route_ppi_events(&fired_global));
            }
            for task_addr in pending_tasks {
                if let Err(e) = self.bus.write_u32(task_addr as u64, 1) {
                    tracing::warn!("on_event PPI task 0x{task_addr:08X} failed: {e:?}");
                }
            }
        }
        if !result.dma_requests.is_empty() {
            if let Err(e) = self.bus.execute_dma(&result.dma_requests) {
                tracing::warn!("on_event execute_dma failed: {e:?}");
            }
        }
    }

    /// Returns true (and clears the latch) if the registered RTC_CNTL
    /// peripheral has a pending software-system-reset request. Used by
    /// `step()` to honor OPTIONS0 bit 31 writes at a clean instruction
    /// boundary. Uses the cached `rtc_cntl_index` resolved at construction
    /// — non-ESP32-classic configs (no RTC_CNTL on the bus) short-circuit
    /// to `false` without touching the peripheral vector at all.
    fn drain_rtc_cntl_reset_request(&self) -> bool {
        let Some(idx) = self.rtc_cntl_index else {
            return false;
        };
        let Some(p) = self.bus.peripherals.get(idx) else {
            return false;
        };
        p.dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::rtc_cntl::RtcCntl>())
            .map(|rtc| rtc.drain_reset_request())
            .unwrap_or(false)
    }

    /// Returns true (and clears the latch) if the registered SCB peripheral
    /// has a pending SYSRESETREQ — an AIRCR write with the correct VECTKEY and
    /// the SYSRESETREQ bit set (`Scb::write_reg`, offset 0x0C). The
    /// authoritative [`Machine::advance`] lifecycle drains this latch at a
    /// clean committed instruction boundary. Uses the cached `scb_index`
    /// resolved at construction — non-Cortex-M configs (no SCB on the bus)
    /// short-circuit to `false` without touching the peripheral vector at all.
    pub(crate) fn drain_scb_reset_request(&self) -> bool {
        let Some(idx) = self.scb_index else {
            return false;
        };
        let Some(p) = self.bus.peripherals.get(idx) else {
            return false;
        };
        p.dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::scb::Scb>())
            .map(|scb| scb.drain_reset_request())
            .unwrap_or(false)
    }

    /// Drain a pending FLASH hardware operation recorded by the H5 FLASH
    /// peripheral (sector erase or bank-swap+reset). Returns `None` for
    /// configs that have no FLASH peripheral on the bus. The caller is
    /// responsible for applying the returned op to `bus.flash` and issuing
    /// a CPU reset when required.
    fn drain_flash_op(&self) -> Option<crate::peripherals::flash::FlashOp> {
        let idx = self.flash_index?;
        let p = self.bus.peripherals.get(idx)?;
        p.dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::flash::Flash>())
            .and_then(|f| f.drain_pending_op())
    }

    /// Borrow the H5 FLASH peripheral, if one is on the bus. Used by the
    /// read-while-write gate to query the swap state / bank mapping.
    fn flash_peripheral(&self) -> Option<&crate::peripherals::flash::Flash> {
        let idx = self.flash_index?;
        self.bus
            .peripherals
            .get(idx)?
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::flash::Flash>())
    }

    /// Mutably borrow the FLASH peripheral, if one is on the bus. Used to record
    /// the applied SWAP_BANK so the RWW bank mapping tracks the active view.
    fn flash_peripheral_mut(&mut self) -> Option<&mut crate::peripherals::flash::Flash> {
        let idx = self.flash_index?;
        self.bus
            .peripherals
            .get_mut(idx)?
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<crate::peripherals::flash::Flash>())
    }

    pub fn snapshot(&self) -> snapshot::MachineSnapshot {
        snapshot::MachineSnapshot {
            schema_version: snapshot::SCHEMA_VERSION,
            cpu: self.cpu.snapshot(),
            peripherals: self
                .bus
                .peripherals
                .iter()
                .map(|p| (p.name.clone(), p.dev.snapshot()))
                .collect(),
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: snapshot::MachineSnapshot) -> SimResult<()> {
        if snapshot.schema_version != snapshot::SCHEMA_VERSION {
            return Err(SimulationError::SnapshotSchemaMismatch {
                expected: snapshot::SCHEMA_VERSION,
                got: snapshot.schema_version,
            });
        }
        self.cpu.apply_snapshot(&snapshot.cpu);
        for p in &mut self.bus.peripherals {
            if let Some(state) = snapshot.peripherals.get(&p.name) {
                p.dev.restore(state.clone())?;
            }
        }
        self.bus.refresh_peripheral_index();
        Ok(())
    }

    pub fn peek_peripheral(&self, name: &str) -> Option<serde_json::Value> {
        self.bus
            .peripherals
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.dev.snapshot())
    }

    /// Universal inspect: enumerate + decode peripheral state (snapshot
    /// semantics — reads the current paused/post-run machine). `name = Some`
    /// (or `opts.peripheral`) restricts the walk to one peripheral; `None`
    /// inspects all. Decode is side-effect-free (uses `peek`, never `read`).
    /// See [`crate::inspect`].
    pub fn inspect(
        &self,
        name: Option<&str>,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::MachineInspect {
        let filter = name.or(opts.peripheral.as_deref());
        let peripherals = self
            .bus
            .peripherals
            .iter()
            .filter(|entry| filter.is_none_or(|f| entry.name == f))
            .map(|entry| entry.dev.inspect(entry.base, &entry.name, opts))
            .collect();
        crate::inspect::MachineInspect { peripherals }
    }

    /// Raw escape hatch: read `len` bytes at absolute `addr`, side-effect-free.
    /// Clamps to mapped regions — bytes outside any memory region or peripheral
    /// window come back as [`crate::inspect::PeekByte::Unmapped`] rather than a
    /// silent zero, so unmodeled space is never mistaken for real data.
    pub fn peek(&self, addr: u64, len: usize) -> crate::inspect::PeekResult {
        let mut bytes = Vec::with_capacity(len);
        for i in 0..len as u64 {
            bytes.push(match self.bus.peek_byte(addr + i) {
                Some(v) => crate::inspect::PeekByte::Mapped(v),
                None => crate::inspect::PeekByte::Unmapped,
            });
        }
        crate::inspect::PeekResult { addr, bytes }
    }
}

impl<C: Cpu> DebugControl for Machine<C> {
    fn add_breakpoint(&mut self, addr: u32) {
        self.breakpoints.insert(addr);
    }

    fn remove_breakpoint(&mut self, addr: u32) {
        self.breakpoints.remove(&addr);
    }

    fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    fn run(&mut self, max_steps: Option<u32>) -> SimResult<StopReason> {
        let report = self.advance(AdvanceRequest::run(max_steps.map(u64::from)))?;
        Ok(match report.stop {
            AdvanceStop::Breakpoint(pc) => StopReason::Breakpoint(pc),
            AdvanceStop::FuelLimit => StopReason::MaxStepsReached,
            AdvanceStop::CycleLimit | AdvanceStop::NoProgress => StopReason::StepDone,
        })
    }
    fn step_single(&mut self) -> SimResult<StopReason> {
        self.step()?;
        Ok(StopReason::StepDone)
    }

    fn read_core_reg(&self, id: u8) -> u32 {
        self.cpu.get_register(id)
    }

    fn write_core_reg(&mut self, id: u8, val: u32) {
        self.cpu.set_register(id, val);
    }

    fn read_memory(&self, addr: u32, len: usize) -> SimResult<Vec<u8>> {
        let mut data = Vec::with_capacity(len);
        for i in 0..len {
            let byte = self.bus.read_u8((addr as u64) + (i as u64))?;
            data.push(byte);
        }
        Ok(data)
    }

    fn write_memory(&mut self, addr: u32, data: &[u8]) -> SimResult<()> {
        for (i, byte) in data.iter().enumerate() {
            self.bus.write_u8((addr as u64) + (i as u64), *byte)?;
        }
        Ok(())
    }

    fn get_pc(&self) -> u32 {
        self.cpu.get_pc()
    }

    fn set_pc(&mut self, addr: u32) {
        self.cpu.set_pc(addr);
    }

    fn get_register_names(&self) -> Vec<String> {
        self.cpu.get_register_names()
    }

    fn get_cycle_count(&self) -> u64 {
        self.total_cycles
    }

    fn get_peripherals(&self) -> Vec<(String, u64, u64)> {
        self.bus
            .peripherals
            .iter()
            .map(|p| (p.name.clone(), p.base, p.size))
            .collect()
    }

    fn get_peripheral_descriptor(
        &self,
        name: &str,
    ) -> Option<labwired_config::PeripheralDescriptor> {
        let entry = self.bus.peripherals.iter().find(|p| p.name == name)?;
        entry.dev.peripheral_descriptor()
    }

    fn reset(&mut self) -> SimResult<()> {
        Machine::reset(self)
    }

    fn snapshot(&self) -> snapshot::MachineSnapshot {
        self.snapshot()
    }

    fn restore(&mut self, snapshot: &snapshot::MachineSnapshot) -> SimResult<()> {
        self.apply_snapshot(snapshot.clone())
    }
}
