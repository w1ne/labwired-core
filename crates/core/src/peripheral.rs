// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The memory-mapped peripheral contract.
//!
//! Kept out of the crate root so the trait, its DMA request/response types and
//! its tick result live in one place. The crate root re-exports every public
//! item, so `labwired_core::Peripheral` and the existing `crate::` paths
//! continue to resolve unchanged.

use std::any::Any;
use std::sync::Arc;

use crate::{logic_capture, peripherals, pins, sched, Bus, CycleClock, SimResult, SimulationError};

// ── DMA request vocabulary ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaDirection {
    Read,
    Write,
    Copy,
    /// A DMA FIFO filling from its source port: read `transform.src_width`
    /// bytes at `src_addr` (one access when aligned) and hand the value,
    /// little-endian, back to the DMA that issued the item
    /// ([`Peripheral::dma_load_data`]). `addr` is unused.
    Load,
    /// A DMA FIFO draining to its destination port: write the low
    /// `transform.dst_width` bytes of the value at `addr` (one access when
    /// aligned). `src_addr` is unused.
    Store(u32),
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
    /// Peripheral-paced STM32 DMA only: the items the channel/stream still
    /// has to move AFTER this one (`NDTR` after the decrement). The bus hands
    /// it to the peripheral on the request's peripheral side before the access
    /// ([`Peripheral::dma_access_notice`]) — the model's form of the DMA's
    /// end-of-transfer signal, which the STM32 FIFO SPI needs for
    /// `CR2.LDMA_TX` / `LDMA_RX` (RM0444 §35.5.10). `None` for memory-to-memory
    /// and for engines that do not report it.
    pub remaining: Option<u32>,
}

/// One DMA channel a peripheral request line currently drives
/// ([`Peripheral::dma_route_request`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmaRoute {
    /// The DMA controller to request on, by peripheral id. `None`: the
    /// peripheral that answered (a DMA controller routing to itself); `Some`
    /// for a request multiplexer (STM32 DMAMUX) naming the controller it
    /// feeds.
    pub controller: Option<String>,
    /// The controller's channel / stream, 1-based (the
    /// [`Peripheral::dma_request`] numbering).
    pub channel: u32,
}

/// The [`PeripheralTickResult::dma_signals`] ids a peripheral raises for its
/// two DMA request lines. The bus names the request `<peripheral id>_tx` /
/// `<peripheral id>_rx` (e.g. `spi1_tx`) and asks the chip's DMA controllers
/// and multiplexers which channels that request drives
/// ([`Peripheral::dma_route_request`]).
pub const DMA_SIGNAL_TX: u32 = 1;
/// See [`DMA_SIGNAL_TX`].
pub const DMA_SIGNAL_RX: u32 = 2;

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

// ── Flash-store ownership ─────────────────────────────────────────────────

/// What the bus does with a CPU store into the flash region once a
/// flash-controller peripheral has seen it ([`Peripheral::flash_cpu_store`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashStoreOutcome {
    /// Commit the stored bytes to the flash array as written (the behaviour of
    /// a bus without a flash-store owner).
    Commit,
    /// The controller consumed the store (an address latch, or a write while
    /// not in a programming mode); the array is left alone.
    Consumed,
    /// Program the stored bytes: each array byte becomes `existing & new`, the
    /// 1-to-0 rule of NOR flash.
    Program,
}

// ── Tick result ───────────────────────────────────────────────────────────

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

// ── MMIO access classification ───────────────────────────────────────────

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

    /// Byte RX DMA request: `Some(true)` while a received byte waits in the
    /// register at `offset` (a DMA read of it consumes the byte). `None`
    /// refuses this endpoint; `Some(false)` is a supported endpoint with no
    /// byte waiting.
    fn dma_rx_request(&self, _offset: u64) -> Option<bool> {
        None
    }

    /// The DMAC trigger sources (TX, RX) this peripheral raises, as its chip
    /// description declares them. `None`: it declares none.
    fn dma_triggers(&self) -> Option<(u32, u32)> {
        None
    }

    /// Peripheral side of [`crate::Bus::take_dma_trigger`]: consume one
    /// pending request of DMAC trigger source `source` if this peripheral
    /// raises it. Default `None`: it raises none.
    fn take_dma_trigger(&mut self, _source: u32) -> Option<bool> {
        None
    }

    /// Controller-local byte TX DMA request. `None` refuses this endpoint;
    /// `Some(false)` is a supported endpoint that is not ready. This does not
    /// grant bus access: the bus must separately enforce clocks and routing.
    /// DMA requests are independent of CPU interrupt-enable bits.
    fn dma_tx_request(&self, _offset: u64) -> Option<bool> {
        None
    }

    /// Refresh bus-sourced transport control inputs before an admitted TX beat.
    /// This is not a tick: implementations must not advance time, drain bytes
    /// or commit DMA progress. The bus calls this only after clock/readiness
    /// admission, before the byte acceptance and its observer notification.
    fn prepare_dma_tx(&mut self, _bus: &mut dyn Bus) {}

    /// Atomically accept one TX byte, or return `false` without changing
    /// state when not ready. Unsupported endpoints refuse explicitly.
    /// A successful ordinary register store is not an acceptance certificate.
    /// The bus remains responsible for gating, observers and scheduler wakes.
    fn try_dma_write_tx_u8(&mut self, _offset: u64, _value: u8) -> SimResult<bool> {
        Err(SimulationError::NotImplemented(
            "peripheral has no byte TX DMA endpoint".into(),
        ))
    }
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
    /// Return the value of `count` identical word reads when this register is
    /// guaranteed not to change until the next scheduled peripheral event.
    ///
    /// The default declines. Implementations may opt in only for offsets with
    /// no read side effects or unscheduled external producer; the containing
    /// bus remains responsible for access accounting. CPU poll-loop coalescers
    /// are bounded by the scheduler's next-event clamp before calling this.
    fn repeat_stable_read_u32(&self, _offset: u64, _count: u32) -> Option<SimResult<u32>> {
        None
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
    /// Side-effect-free value probe used for debug/observer bookkeeping.
    /// Implementations should return `None` when such probing is not supported.
    fn peek(&self, _offset: u64) -> Option<u8> {
        None
    }
    /// Latched CPU-reset request raised by this peripheral, e.g. a watchdog
    /// time-out with reset enabled. Returns `true` once per event and clears
    /// the latch. The machine drains it at a clean instruction boundary and
    /// reboots the CPU through the existing reset path (see
    /// `Machine::advance`). Default: no reset requests.
    ///
    /// Read [`Self::latched_reset_cause`] before this call. It is `None` once
    /// the latch has been taken.
    fn take_reset_request(&mut self) -> bool {
        false
    }

    /// Why [`Self::take_reset_request`] would return `true` right now.
    /// `None` when nothing is latched, or when the peripheral does not name a
    /// cause (the CPU still reboots; no reset-cause flag is recorded).
    fn latched_reset_cause(&self) -> Option<PeripheralResetCause> {
        None
    }

    /// A peripheral's reset request was drained and the CPU is about to
    /// reboot. Sticky cause registers (RESETCAUSE and its equivalents) record
    /// `cause` here. Called after the requesting peripheral has cleared its
    /// own latch. Default: ignore.
    fn on_peripheral_reset(&mut self, _cause: PeripheralResetCause) {}

    /// A system reset (Cortex-M `AIRCR.SYSRESETREQ`, or a peripheral-requested
    /// reboot such as a watchdog time-out) just restarted the core: return the
    /// registers to their power-on values, as silicon does. Keep what is not
    /// chip state: attached devices, wires, the cycle clock and non-volatile
    /// contents. Called after [`Self::on_peripheral_reset`].
    ///
    /// Default: nothing, which keeps the historical behaviour for every model
    /// that has not opted in.
    fn on_system_reset(&mut self) {}

    /// `true` if this peripheral can ever return `true` from
    /// [`Self::take_reset_request`]. The machine caches the bus indices of
    /// such peripherals once at construction, so peripherals that never
    /// request a reset add nothing to the per-instruction drain.
    fn reset_request_capable(&self) -> bool {
        false
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

    /// Cycles until this device would raise a processor-clock-tied IRQ that a
    /// compiled JIT block of that length would skip. SysTick is the only
    /// in-tree override (Cortex-M analogue of RISC-V `mtime`). Default `None`.
    fn systick_ticks_until_fire(&self) -> Option<u64> {
        None
    }

    /// Consume `n` processor cycles on a core-tied timer after a compiled
    /// block that was clamped not to wrap. Default no-op.
    fn systick_consume_cycles(&mut self, _n: u64) {}
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

    /// Edge sink: observe an externally driven GPIO pad transition with its
    /// explicit previous and current levels. Return true if work was latched
    /// that needs a scheduler wake. Default no-op; STM32 EXTI uses the port
    /// mux here.
    ///
    /// `port` is the [`PortId`](pins::PortId) the bus gave the GPIO port when
    /// it was attached (port A = 0); the bus calls every sink from
    /// [`SystemBus::set_pad_external`](crate::bus::SystemBus::set_pad_external).
    ///
    /// `line_source` is the port the chip's EXTI line-source mux selects for
    /// line `pin` when that mux lives OUTSIDE the EXTI block (AFIO_EXTICRx on
    /// STM32F1, SYSCFG_EXTICRx on STM32F4), as answered by
    /// [`exti_line_source`](Self::exti_line_source); `None` when no
    /// peripheral on the bus owns one. An EXTI with its own mux (G0, U5)
    /// ignores it.
    fn gpio_input_edge(
        &mut self,
        _port: u8,
        _pin: u8,
        _before: bool,
        _after: bool,
        _line_source: Option<u8>,
    ) -> bool {
        false
    }

    /// EXTI line-source mux: the GPIO port index (0 = A) this peripheral
    /// routes to EXTI line `line` (0..15), for a mux that sits outside the
    /// EXTI block (STM32F1 AFIO_EXTICRx, STM32F4 SYSCFG_EXTICRx). `None` for
    /// every peripheral that is not such a mux.
    fn exti_line_source(&self, _line: u8) -> Option<u8> {
        None
    }

    /// Cross-peripheral GPIO change hook: bus snapshots GPIO IN registers
    /// each tick and calls this with a list of `(port, pin, new_level)`
    /// transitions. GPIOTE overrides to drive EVENTS_IN[i] when a channel
    /// is configured to watch a matching (port, pin) with a matching
    /// polarity. Default no-op.
    ///
    /// Returns whether this peripheral LATCHED work from the edge that it now
    /// needs a scheduler wake for. The bus harvests `take_scheduled_events`
    /// only from the peripherals that say `true`: an edge is a cross-peripheral
    /// activation the per-MMIO-write harvest choke never sees, but harvesting
    /// from everybody re-arms a SECOND wake on models that already have one in
    /// flight and cannot latch anything from a GPIO edge at all. A duplicate
    /// wake at an earlier deadline drains an in-flight multi-cycle model on the
    /// spot — that is how a `board_io` button on the nRF52840-DK collapsed
    /// RADIO air time to the EasyDMA cycle. Default `false`: a model that does
    /// not observe GPIO cannot have latched anything.
    fn observe_gpio_change(&mut self, _changes: &[(u8, u8, u8)]) -> bool {
        false
    }

    /// True if this peripheral CONSUMES GPIO edges — i.e. its
    /// [`Self::observe_gpio_change`] does real work.
    ///
    /// ⚠️ This is what keeps the per-cycle GPIO edge-detection pass alive on a
    /// walk-free bus. That pass lives inside the phase-1 body which
    /// [`SystemBus::per_cycle_tick_is_trivial`] skips wholesale, and its old
    /// guard was "is there a peripheral NAMED `gpio0`/`gpio1`" — a Nordic
    /// spelling. A Silicon Labs part names its ports `gpioa`..`gpiod`, so the
    /// guard answered "nothing to scan" and the fast path would have deleted
    /// the only path by which an EXTI line ever sees a pad move: a button press
    /// would set no flag, run no ISR, and nothing anywhere would say so.
    ///
    /// Answering the question directly instead of by port name is what makes
    /// the guard mean what it says. It is deliberately NOT derived from having
    /// GPIO ports: an STM32 bus has four of them and no edge consumer at all
    /// (its EXTI is driven by MMIO, not by the pad snapshot), and such a bus
    /// must keep the fast path.
    ///
    /// Default `false`: a model that does not override `observe_gpio_change`
    /// cannot consume an edge.
    fn observes_gpio_edges(&self) -> bool {
        false
    }

    /// Family-wide STM32 UART pad selector override. G0 uses AF1 where the
    /// shared GPIO register window cannot distinguish it from L0. Other
    /// families keep the per-window routing tables and return None.
    fn stm32_uart_pad_af_override(&self) -> Option<u8> {
        None
    }

    /// Clock-controller capability: resolve a symbolic clock-enable register
    /// name from a peripheral's `clock:` declaration (`"apb1enr"`, `"clken2"`,
    /// …) to its byte offset inside THIS peripheral.
    ///
    /// Only a chip's clock controller implements it. The alternative — the bus
    /// downcasting to one concrete model — meant clock gating existed for
    /// exactly the family that model belonged to, and a second vendor's clock
    /// unit could not gate anything no matter what its yaml declared.
    /// `None` for every other peripheral, and for a name this controller does
    /// not have (which `resolve_clock_gates` turns into a hard config error
    /// rather than a silently ungated peripheral).
    fn clock_gate_reg_offset(&self, _name: &str) -> Option<u64> {
        None
    }

    /// Cortex-M7 L1 cache capability: the I/D cache geometry this peripheral
    /// declares (`cortex_m7_cache`). `configure_cortex_m` asks every
    /// peripheral and hands the answer to the SCB's CLIDR/CTR/CCSIDR, so the
    /// cache model does not have to be found by downcasting. `None` for every
    /// other peripheral, and on chips without the caches.
    fn l1_cache_geometry(&self) -> Option<crate::peripherals::cortex_m7_cache::L1CacheGeometry> {
        None
    }

    /// Bind a power controller's backup-domain write-enable signal to its
    /// clock controller. Models without that protection ignore the signal.
    fn bind_backup_domain_write_enable(
        &mut self,
        _enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
    }

    /// Camera-slave capability: bind a DVP sensor's pixel output to this
    /// peripheral's camera input. Returns `true` when this peripheral has one
    /// and took the sensor.
    ///
    /// Only a camera interface implements it (the ESP32-S3 `LCD_CAM`). Without
    /// it, every camera kit would have to downcast to that one concrete model,
    /// and a second chip's camera interface could never receive a sensor.
    fn attach_dvp_sensor(
        &mut self,
        _sensor: std::sync::Arc<dyn crate::peripherals::components::DvpSensor>,
    ) -> bool {
        false
    }

    /// PWM capability: notify `obs` of each duty an LEDC channel commits
    /// (`ledcWrite`). Implemented by the ESP32-family LEDC models; `false`
    /// (observer dropped) for every other peripheral.
    fn add_ledc_duty_observer(
        &mut self,
        _obs: Arc<dyn crate::peripherals::LedcDutyObserver>,
    ) -> bool {
        false
    }

    /// PWM capability: notify `obs` of each duty an MCPWM operator commits
    /// (`mcpwm_set_duty`). Implemented by the ESP32-family MCPWM models;
    /// `false` (observer dropped) for every other peripheral.
    fn add_mcpwm_duty_observer(
        &mut self,
        _obs: Arc<dyn crate::peripherals::McpwmDutyObserver>,
    ) -> bool {
        false
    }

    /// GPIO capability: read the firmware-visible input level for `pin`.
    /// Non-GPIO peripherals return `None`.
    fn read_gpio_input(&self, _pin: u8) -> Option<bool> {
        None
    }

    /// GPIO capability: the firmware-visible input levels of the WHOLE port as
    /// one word, bit `n` being pin `n`. Zero for a non-GPIO peripheral.
    ///
    /// The per-tick edge-detection pass in [`SystemBus::tick_peripherals_fully`]
    /// wants the whole port, not one pin, and it runs on every boundary tick.
    /// Assembling the word from 32 [`Peripheral::read_gpio_input`] calls costs
    /// 32 evaluations of the port's input register per port per tick — and on
    /// these models that register is COMPUTED, not stored: `GpioPort` resolves
    /// it through `effective_idr`, which folds the output latch, the open-drain
    /// mask and the pad levels together on every call. Measured under callgrind
    /// on the perf-spin fixture, that loop was 60% of all instructions the
    /// simulator retired on efr32mg26 (4 ports) and ~33% on nrf52840 (2 ports).
    ///
    /// The default builds the word exactly the way that pass built it before
    /// this method existed, so a model that does not override it is bit-for-bit
    /// unchanged: pins are read low to high, and the first `None` ends the port
    /// (a port narrower than 32 pins leaves the rest of the word clear).
    /// A port that keeps its inputs in one register overrides this and answers
    /// in a single read.
    fn read_gpio_input_word(&self) -> u32 {
        let mut word = 0u32;
        for pin in 0..32u8 {
            match self.read_gpio_input(pin) {
                Some(true) => word |= 1 << pin,
                Some(false) => {}
                None => break,
            }
        }
        word
    }

    /// GPIO capability: read the firmware-visible output latch for `pin`.
    /// Non-GPIO peripherals return `None`.
    fn read_gpio_output(&self, _pin: u8) -> Option<bool> {
        None
    }

    /// GPIO capability: is `pin` configured as a general-purpose OUTPUT, read
    /// from the model's direction register (AVR `DDRx`, STM32 `MODER` or F1
    /// `CRL`/`CRH`, nRF `DIR`, Kinetis `PDDR`, SAM `DIR`, EFR32 mode nibble, the
    /// ESP32 enable register plus output matrix).
    ///
    /// `Some(false)` for an input, analog or alternate-function pad: the latch
    /// [`read_gpio_output`](Self::read_gpio_output) reads is not what drives
    /// such a pad. `None` means the model cannot say, which co-simulation
    /// refuses when a session is built rather than treating as "input".
    ///
    /// The default derives the answer from [`gpio_routing`](Self::gpio_routing),
    /// so a family that reports routing reports direction too; a model whose
    /// routing is `Unknown` answers `None`.
    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        use crate::peripherals::gpio::GpioMode;
        match self.gpio_routing(pin)?.mode {
            GpioMode::Output => Some(true),
            GpioMode::Input | GpioMode::Af | GpioMode::Analog => Some(false),
            GpioMode::Unknown => None,
        }
    }

    /// GPIO capability: whether an input pad (`read_gpio_is_output == Some(false)`)
    /// is released whatever its output latch holds. True for the ATmega port,
    /// where `pinMode(INPUT)` clears `PORTx` yet the pad drives nothing; the
    /// default `false` keeps the latch as the released level (STM32 `ODR`).
    fn gpio_input_is_released(&self) -> bool {
        false
    }

    /// GPIO capability: the offsets of this port's output latch and input
    /// register within its window, for a port whose register bit `n` IS pad
    /// `n` (an STM32/nRF/SAM/EFR32/Kinetis `GpioPort`, an ATmega `PORTx`).
    ///
    /// `None` for everything else, including a GPIO block that banks its pads
    /// across several registers (the ESP32 family's single `gpio` block, whose
    /// pads 32 and up sit in a second output register as bank-relative bits).
    /// Pin resolution and co-simulation ask this instead of downcasting to each
    /// port model they know about.
    fn gpio_port_offsets(&self) -> Option<crate::peripherals::gpio::GpioPortOffsets> {
        None
    }

    /// Pin capability: this peripheral's pads, through the one pad interface
    /// ([`pins::PinPort`]). `None` (the default) for a peripheral that owns
    /// no pads. Every GPIO model answers `Some(self)`; the GPIO methods
    /// below that predate the interface are shims over it.
    fn pins(&self) -> Option<&dyn pins::PinPort> {
        None
    }

    /// Mutable half of [`pins`](Self::pins).
    fn pins_mut(&mut self) -> Option<&mut dyn pins::PinPort> {
        None
    }

    /// GPIO capability: the pad level a logic probe clipped to `pin` would
    /// see. Shim over [`pins::PinPort::level`] (kept because many callers
    /// only want a level); a peripheral without pins falls back to the output
    /// latch, then the input level.
    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        if let Some(port) = self.pins() {
            return port.level(pin);
        }
        self.read_gpio_output(pin)
            .or_else(|| self.read_gpio_input(pin))
    }

    /// GPIO capability: who drives `pin` as a probe sees it — the pad's own
    /// output stage, an external device, both disagreeing, or nothing
    /// (high-Z). Shim over [`pins::probe_drive`]. `None` when the model
    /// cannot say; the trace then has no four-state lane for the pad rather
    /// than a guessed "driven".
    fn read_gpio_pad_drive(&self, pin: u8) -> Option<crate::logic_capture::PadDrive> {
        self.pins().and_then(|port| pins::probe_drive(port, pin))
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
    ///
    /// Shim over [`pins::PinPort::set_external`] with
    /// [`External::Level`](pins::External::Level), kept for the callers that
    /// only hold a level. It does not raise edge events: a caller with a bus
    /// goes through [`SystemBus::set_pad_external`](crate::bus::SystemBus::set_pad_external).
    fn set_gpio_input(&mut self, pin: u8, level: bool) -> bool {
        self.pins_mut()
            .is_some_and(|port| pins::set_level(port, pin, level))
    }

    /// GPIO capability: drain the level changes on pads the mux currently
    /// hands to a timer input (STM32 `TIMx_CHn` through the AF / F1 input
    /// mapping), recorded since the last drain. Each entry names the timer by
    /// bus index and its input (0 = TI1). Only a port with capture routes
    /// wired ever returns anything; see
    /// [`SystemBus::deliver_timer_input_edges`](crate::bus::SystemBus::deliver_timer_input_edges).
    fn take_timer_input_edges(&mut self) -> Vec<crate::peripherals::gpio::TimerInputEdge> {
        Vec::new()
    }

    /// GPIO capability: drain the outside levels that reached pads routed to
    /// a peripheral line (an SPI MISO, an I²C SDA) since the last drain. Each
    /// names the line cell, which the bus maps back to the peripheral owning
    /// it; see [`SystemBus::deliver_wire_input_edges`](crate::bus::SystemBus::deliver_wire_input_edges).
    fn take_wire_input_edges(&mut self) -> Vec<crate::peripherals::pad_lines::WireInputEdge> {
        Vec::new()
    }

    /// Wire capability: the level an outside driver holds on the pad routed
    /// to line `line` of this peripheral's [`Self::wire_lines`] changed to
    /// `level` at absolute engine cycle `cycle` (a world `gpio_net`
    /// delivering a peer's edge). The new level is also readable from
    /// [`PadLines::input`](crate::peripherals::pad_lines::PadLines::input).
    /// Returns `true` when the peripheral may have work to schedule, so the
    /// bus collects its scheduled events. Default: not a listener.
    fn wire_input_edge(&mut self, _line: usize, _level: bool, _cycle: u64) -> bool {
        false
    }

    /// Pin-select capability: `true` for a peripheral that names the pads it
    /// drives in its own registers (nRF `PSEL.*`), so a register write can
    /// move a pad between its GPIO latch and one of this peripheral's lines.
    /// On a bus with a world `gpio_net` the bus brackets such writes so the
    /// pin ports report what moved and hand the net's levels to newly routed
    /// lines. Default: no.
    fn claims_pads(&self) -> bool {
        false
    }

    /// GPIO capability: `true` for a pin port whose pad routing follows the
    /// pin selects of [`Self::claims_pads`] peripherals. Default: no.
    fn reads_pad_claims(&self) -> bool {
        false
    }

    /// RTC IO capability (ESP32 / ESP32-S3 `RTC_IO`): the RTC IO MUX state
    /// this block shares with its part's GPIO model, which stays the pads'
    /// only pin port and asks it which domain owns each pad. The bus hands it
    /// over at attach ([`Self::attach_rtc_io_pads`]) and brackets every write
    /// to this block with the GPIO port's push capture. Default: not an RTC
    /// IO block.
    fn rtc_io_pads(&self) -> Option<std::sync::Arc<crate::peripherals::esp_rtc_io::RtcIoPads>> {
        None
    }

    /// GPIO capability: follow the RTC IO MUX `pads` (see
    /// [`Self::rtc_io_pads`]). `true` when this port took them (an ESP32 /
    /// ESP32-S3 GPIO model of the same part). Default: refused.
    fn attach_rtc_io_pads(
        &mut self,
        _pads: std::sync::Arc<crate::peripherals::esp_rtc_io::RtcIoPads>,
    ) -> bool {
        false
    }

    /// GPIO capability: a write to the attached RTC IO block just completed;
    /// pads may have moved between the digital and the RTC domain. The bus
    /// calls it inside the push-capture bracket of that write. Default:
    /// nothing to do.
    fn rtc_io_written(&mut self) {}

    /// True when a READ of this peripheral can clear the status flag behind
    /// its level IRQ (STM32 input capture: reading CCRx clears CCxIF). The
    /// bus then reconciles the NVIC pend after the read, as it does after
    /// every write. Default `false`: reads have no side effects.
    fn reads_can_deassert_irq(&self) -> bool {
        false
    }

    /// Timer capability: which input stage this timer has, so the bus can
    /// route pads to its channels. `None` for everything that is not a timer
    /// with capture channels.
    fn timer_input_stage(&self) -> Option<crate::peripherals::timer::TimerInputStage> {
        None
    }

    /// Timer capability: the channel-OUTPUT wire (`CH1`..`CH4`), created on
    /// first call, so the bus can route it onto GPIO pads. With `arm` the
    /// timer also starts generating its outputs onto the wire (a pad was
    /// bound); without it the wire exists but stays inert, so a timer no pad
    /// can reach costs nothing. `None` for everything that is not a timer
    /// with output channels.
    fn timer_output_lines(
        &mut self,
        _arm: bool,
    ) -> Option<std::sync::Arc<crate::peripherals::pad_lines::PadLines>> {
        None
    }

    /// Family of the waveform outputs behind [`Self::timer_output_lines`], for
    /// bus wiring passes that bind them to a chip family's pad table (the
    /// SAMD51 TC pass binds only `samd51_tc`). `None` by default.
    fn timer_output_family(&self) -> Option<&'static str> {
        None
    }

    /// GPIO capability: bind pad `pin` CONDITIONALLY to `line` of a timer's
    /// channel-output wire `cell` — live while the pad is an alternate-function
    /// output and the timer drives that line (STM32 F1 fixed mapping). A port
    /// whose register layout is not F1 refuses it and returns `false`.
    fn bind_timer_output_pad(
        &mut self,
        _cell: &std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
        _pin: u8,
        _line: usize,
        _func: &'static str,
    ) -> bool {
        false
    }

    /// GPIO capability: bind pad `pin` to input `ti` of the timer at bus
    /// index `timer` — live while the pad selects `af` (an STM32 V2 AFR
    /// nibble), or while it is a digital input when `af` is `None` (STM32 F1
    /// fixed mapping). A port whose register layout is not the row's shape
    /// refuses it and returns `false`.
    fn bind_timer_capture_pad(
        &mut self,
        _pin: u8,
        _af: Option<u8>,
        _timer: usize,
        _ti: u8,
        _func: &'static str,
    ) -> bool {
        false
    }

    /// Timer capability: input `ti` (0 = TI1 … 3 = TI4) changed to `level`
    /// at absolute engine cycle `cycle`. A timer with capture channels latches
    /// CNT as of that cycle into CCRx per its CCMR/CCER configuration and
    /// runs its slave-mode trigger. Returns `false` when this peripheral has
    /// no timer inputs.
    fn timer_input_edge(&mut self, _ti: u8, _level: bool, _cycle: u64) -> bool {
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

    /// Keep `cell` equal to the level of pad `pin`, updated at every write that
    /// can move it. Returns `false` (the default) when this GPIO model cannot
    /// promise that, so a caller that needs an exact level at the moment of a
    /// UART write (an RS-485 driver-enable pin) can refuse the board instead of
    /// sampling late.
    ///
    /// Independent of [`Self::install_logic_tap`]: the logic analyzer owns that
    /// hook and replaces its watch set wholesale, so a part that needs a pin
    /// level for its own purposes cannot borrow it.
    fn watch_pad_level(
        &mut self,
        _pin: u8,
        _cell: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> bool {
        false
    }

    /// Wire capability: this peripheral's OWN line names, in a stable order.
    ///
    /// The POSITION of a name here IS the `line` index of a
    /// [`LogicSource::Wire`](crate::logic_capture::LogicSource::Wire) channel,
    /// so the order is part of the contract and may never be permuted — doing
    /// so would silently re-point every armed wire probe at a different signal.
    /// Names are the datasheet's role labels (`"TX"`, `"SCL"`, `"SCK"`), and
    /// resolution through [`Machine::resolve_wire_source`] ignores case.
    ///
    /// `&[]` — the default — means "this model publishes no wire", and is what
    /// makes a wire probe on it a clear error instead of a silent channel zero.
    ///
    /// This is deliberately independent of [`Self::wire_lines`]: the names are
    /// a property of the silicon and are known before any lab wires a pad,
    /// while the cell they describe may not exist yet.
    fn line_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// Wire capability: the narration cell this peripheral publishes into.
    ///
    /// This is the SAME cell a routed pad reads through
    /// [`PadRoutes`](crate::peripherals::pad_routing::PadRoutes) — one wire,
    /// one home. A wire channel registers on it directly, so it captures
    /// whether or not any pad is muxed to the signal.
    ///
    /// `None` — the default — is the honest answer both for a model that owns
    /// no wire at all and for one whose cell is created lazily at pad-wiring
    /// time and has not been created. Levels published here must always be the
    /// state of the WIRE (for an open-drain bus, the wired-AND of every
    /// driver), never of a register.
    fn wire_lines(&self) -> Option<&crate::peripherals::pad_lines::PadLines> {
        None
    }

    /// Bus-aware tick hook for peripherals that need to read or write the
    /// bus themselves (e.g. Easy DMA on RADIO). Default no-op.
    fn tick_with_bus(&mut self, _bus: &mut dyn Bus) {}

    /// True if this peripheral wants the bus to call `tick_with_bus`.
    /// Default false so the bus skips the swap dance for everyone else.
    fn needs_bus_tick(&self) -> bool {
        false
    }

    /// Bare-CPU-oracle twin of [`Self::needs_bus_tick`] — the `tick_with_bus`
    /// counterpart of [`Self::tick_elapsed_forced`].
    ///
    /// A model whose real work lives in `tick_with_bus` (it moves bytes over
    /// the bus) and which self-guards that hook on `scheduler_mode()` is
    /// invisible to the forced walk: `tick_elapsed_forced` never reaches the
    /// transfer engine, and the bus-tick pass never even selects the model
    /// because `needs_bus_tick()` already reported `false`. Overriding this
    /// pair re-exposes the historical one-tick transfer to the oracle only.
    ///
    /// The default forwards to `needs_bus_tick`, so the ordinary walk and every
    /// model that does not opt in are unchanged.
    #[doc(hidden)]
    fn needs_bus_tick_forced(&self) -> bool {
        self.needs_bus_tick()
    }

    /// Bare-CPU-oracle twin of [`Self::tick_with_bus`]; see
    /// [`Self::needs_bus_tick_forced`]. Default forwards to `tick_with_bus`.
    #[doc(hidden)]
    fn tick_with_bus_forced(&mut self, bus: &mut dyn Bus) {
        self.tick_with_bus(bus)
    }

    /// True if this peripheral's `tick_with_bus` must keep running at a bounded
    /// cadence even while the CPU is idle-fast-forwarding — because it services
    /// an *external* medium (a WiFi station polling a shared-AP inbox and
    /// beaconing) whose frames would otherwise be starved for the whole skip
    /// window, breaking association. Idle fast-forward caps its skip to a small
    /// poll quantum and runs the bus-tick pass at the deadline when any bus
    /// peripheral returns true here. Default false: every self-contained
    /// peripheral drives entirely off CPU cycles / scheduler events and never
    /// needs the idle window shortened. Peripherals that opt in MUST key their
    /// internal cadence (beacons, timeouts) on device cycles — NOT on
    /// tick_with_bus call count — since the call frequency now varies.
    fn idle_poll_bus_tick(&self) -> bool {
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
    /// True if this peripheral's dynamic legacy-tick activity can change when
    /// another peripheral is written through MMIO. The bus refreshes these
    /// observers after successful MMIO stores so shared fabrics (for example,
    /// INPUTMUX/IOCON state consumed by PINT) can wake an idle model without
    /// a per-cycle scan. Default false.
    fn legacy_tick_observes_peer_mmio(&self) -> bool {
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
    /// walk on. For a peripheral that does not use the scheduler, override to
    /// `false` ONLY when BOTH hold for every reachable state:
    ///
    /// - the peripheral's `tick()`/`tick_elapsed()` is a genuine no-op
    ///   (a pure register bank / stub, or a purely lazy model that advances its
    ///   state on MMIO access rather than in `tick`), AND
    /// - it never emits an IRQ, DMA request, mmio-write, or fired event from
    ///   the walk.
    ///
    /// Scheduler-driven peripherals need NOT override this (they are handled by
    /// `uses_scheduler()` in the derivation). In builds without
    /// `event-scheduler`, their active legacy fallback remains indexed even if
    /// this returns `false`. Getting this wrong on a non-scheduler model (a
    /// `false` when it actually does walk work) silently starves the peripheral
    /// of ticks once the bus derives walk-deletion — so the honest direction
    /// under any doubt is to leave it `true`.
    fn needs_legacy_walk(&self) -> bool {
        true
    }
    /// The Rust path of the type implementing this peripheral, e.g.
    /// `labwired_core::peripherals::nrf54l::uarte::Nrf54lUarte`.
    ///
    /// A default body that needs no per-model override: `type_name::<Self>()`
    /// in a default method resolves through the vtable to the CONCRETE type,
    /// so every model answers correctly for free.
    ///
    /// Exists for `model_paths_drift_gate`, which asserts that every model a
    /// board actually registers appears in that board's `models:` list in
    /// `validation/manifest.yaml` — the list whose contents are hashed into
    /// the board's drift digest. A model the list omits can be rewritten
    /// without moving the digest, so a stale `drift_ack` goes on covering a
    /// tree it was never written for.
    ///
    /// Asking the ENGINE rather than parsing the factories is the point. The
    /// first version of that gate mined `type:` -> source from the factory
    /// match arms and could resolve 4 of 67 types while reporting itself
    /// complete; the arms come in too many shapes. This cannot go stale or
    /// blind, because it is the same dispatch the bus itself used.
    fn impl_type_path(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }

    /// Deliver this console's TX bytes into a merged console stream as channel
    /// `ch` (see [`crate::console::ConsoleMerge`]). Returns false for a
    /// peripheral that is not a console that can be merged. Default false.
    fn set_console_merge(
        &mut self,
        _merge: std::sync::Arc<crate::console::ConsoleMerge>,
        _ch: usize,
    ) -> bool {
        false
    }

    /// This peripheral as an inter-chip UART cross-link endpoint, if it is one.
    ///
    /// Named capability rather than a downcast: `attach_uart_stream_by_id` used
    /// to require the concrete [`crate::peripherals::uart::Uart`], which made
    /// every family with its own UART model (the whole ESP32 line) unwireable.
    /// Default `None` = not a UART.
    fn as_uart_stream_host(&mut self) -> Option<&mut dyn crate::peripherals::uart::UartStreamHost> {
        None
    }

    /// The queue that `uart_injections` fill for this UART, if it is one.
    ///
    /// Named capability rather than a downcast in
    /// `SystemBus::attach_uart_rx_source_named`. Default `None` = not a UART
    /// with an injection queue (or one still found by that function's
    /// older downcasts).
    fn uart_rx_source(&self) -> Option<Arc<std::sync::Mutex<std::collections::VecDeque<u8>>>> {
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

    /// Authoritative simulated wall-clock, in microseconds, if this peripheral
    /// is a system time source firmware reads for *elapsed time*.
    ///
    /// This is the honest generalization of what the nRF54L TWIM already does by
    /// hand — reading the GRTC SYSCOUNTER (µs) over the bus before servicing a
    /// slave. Instead of a hardcoded register address, the machine asks every
    /// peripheral this question and uses the answer to drive attached I²C
    /// devices' `advance_time_us` data-ready clocks (see
    /// [`Machine::advance`] / `SystemBus::advance_central_i2c_time`).
    ///
    /// Only implemented by peripherals that model a genuine *absolute* µs-grade
    /// counter firmware uses to measure elapsed time — the ESP32 SYSTIMER (a
    /// silicon 16 MHz free-running counter). A wrapping down-counter (Cortex-M
    /// SysTick) or a wrap-at-ARR timer (STM32 TIMx) does NOT qualify: it holds
    /// no absolute time, and fabricating one from a per-board core frequency
    /// would be a pinned-clock cheat. Those families return `None` (default), so
    /// a `delay_us` device on them stays effectively always-ready, exactly as
    /// before this hook existed — no observable behavior changes for them.
    ///
    /// The value MUST be monotonic within a run except across an explicit
    /// counter reload (the machine re-anchors on a backward jump rather than
    /// advancing devices by a negative delta).
    fn sim_time_us(&self) -> Option<u64> {
        None
    }

    /// Optional fixed-pin I2C attachment capability. Other controllers retain
    /// their existing routing-aware factory dispatch.
    fn i2c_slave_host(&mut self) -> Option<&mut dyn peripherals::device::I2cSlaveHost> {
        None
    }

    /// True if this controller hosts I²C slaves whose data-ready clocks the
    /// machine's central time drive should advance (see
    /// [`Self::advance_attached_i2c_us`]). Default `false`.
    ///
    /// The nRF54L TWIM deliberately does NOT opt in: it drives its slaves'
    /// `advance_time_us` itself off the GRTC, per transaction, so opting into
    /// the central drive too would advance time twice. Every other I²C
    /// controller that hosts attachable slaves returns `true`, and the bus
    /// caches their indices at assembly so the per-slice drive is O(controllers)
    /// with no per-tick peripheral walk.
    fn drives_central_i2c_time(&self) -> bool {
        false
    }

    /// Advance every attached I²C slave's free-running sample/measurement clock
    /// by `us` microseconds (see [`crate::peripherals::i2c::I2cDevice::advance_time_us`]).
    ///
    /// The machine calls this once per scheduler slice on each controller that
    /// [`drives_central_i2c_time`](Self::drives_central_i2c_time), handing over
    /// the microseconds that elapsed on the chip's authoritative
    /// [`sim_time_us`](Self::sim_time_us) source since the previous slice. A
    /// controller overriding this walks its attached-device list and forwards
    /// `us` to each. Default no-op (non-I²C peripherals, and the self-driving
    /// nRF54L TWIM).
    fn advance_attached_i2c_us(&mut self, _us: u64) {}

    /// True if this controller hosts **any** off-chip device — I²C slave or SPI
    /// device — whose free-running clock the machine's central device-time drive
    /// should advance.
    ///
    /// This is the question the machine actually asks; it exists because the
    /// drive is no longer I²C-only (Phase A of the YAML device machine fans the
    /// same elapsed µs out to `SpiDevice`s too). The default forwards to the
    /// historical I²C-only spelling
    /// [`drives_central_i2c_time`](Self::drives_central_i2c_time) so every I²C
    /// controller that already opted in stays opted in, unchanged, and the
    /// nRF54L TWIM's deliberate *non*-opt-in keeps excluding it here as well.
    /// SPI controllers override THIS method.
    fn drives_central_device_time(&self) -> bool {
        self.drives_central_i2c_time()
    }

    /// True when this controller currently has at least one attached device
    /// whose free-running clock the central drive would advance. Controllers
    /// without an attached-device list keep the historical `true`.
    fn has_attached_time_devices(&self) -> bool {
        true
    }

    /// Advance every attached off-chip device's free-running clock by `us`
    /// microseconds — the generalization of
    /// [`advance_attached_i2c_us`](Self::advance_attached_i2c_us) over both bus
    /// vocabularies ([`crate::peripherals::i2c::I2cDevice::advance_time_us`] and
    /// [`crate::peripherals::spi::SpiDevice::advance_time_us`]).
    ///
    /// The machine calls this once per scheduler slice on each controller that
    /// [`drives_central_device_time`](Self::drives_central_device_time). The
    /// default forwards to the I²C-only spelling, so no I²C controller needed a
    /// single edit; SPI controllers override THIS method.
    fn advance_attached_device_time_us(&mut self, us: u64) {
        self.advance_attached_i2c_us(us);
    }

    /// **Tier 2 pin drive.** Collect `(device id, pin role, level)` from every
    /// off-chip device this controller hosts and clear their queues.
    ///
    /// The controller half of the same seam
    /// [`for_each_attached_sim_input`](Self::for_each_attached_sim_input) is: an
    /// I²C slave or SPI device is owned by its CONTROLLER, so a walk over
    /// `SystemBus::peripherals` alone cannot see it, and a declarative part's
    /// INT line would be unreachable from the bus that has to put it on a pad.
    ///
    /// Implementations forward to
    /// [`crate::peripherals::device::drain_i2c_pin_drives`] /
    /// [`drain_spi_pin_drives`](crate::peripherals::device::drain_spi_pin_drives)
    /// rather than reading the device themselves, so the id resolution stays in
    /// one place. Default: nothing, which is correct for every non-controller.
    ///
    /// ⚠️ A controller that hosts attachable devices and does NOT implement this
    /// silently subtracts every Tier-2 pin those devices drive — the same
    /// invisible-device failure the sim-input seam was introduced for. The
    /// rule of thumb is identical: if a type appears in
    /// [`crate::bus::SystemBus::attach_i2c_slave`] or `attach_spi_device`, it
    /// owes an implementation here.
    fn drain_attached_pin_drives(&mut self, _out: &mut Vec<(String, String, bool)>) {}

    fn dma_request(&mut self, _request_id: u32) {}
    /// Flash-controller capability: `true` for the peripheral that decides what
    /// a CPU store into the flash region does (NXP JN51xx / K32W flash
    /// controllers latch addresses and program pages through stores to the
    /// flash address space). The bus resolves at most one such peripheral when
    /// its peripheral list changes and offers it every flash-region CPU store
    /// ([`Self::flash_cpu_store`]). Default `false`: flash stores commit as before.
    fn owns_flash_stores(&self) -> bool {
        false
    }
    /// A CPU store of `width` bytes (1, 2 or 4) with `value` landed at byte
    /// `offset` of the flash array. Only called on the peripheral that
    /// [`Self::owns_flash_stores`], and never for device-originated writes
    /// (`device_write_bypass`): the controller programs and erases the array
    /// through those.
    fn flash_cpu_store(&mut self, _offset: u64, _width: u8, _value: u32) -> FlashStoreOutcome {
        FlashStoreOutcome::Commit
    }
    /// Opt-in manual flash page buffer and instruction-boundary commit
    /// capability (SAMD51 NVMCTRL manual mode). Discovery and access use the
    /// same interface, without a concrete peripheral downcast. Controllers that
    /// commit on the store itself use [`Self::flash_cpu_store`] instead.
    fn manual_flash_storage(
        &self,
    ) -> Option<&dyn crate::peripherals::sam::nvmctrl::ManualFlashStorage> {
        None
    }
    /// Mutable counterpart of [`Self::manual_flash_storage`].
    fn manual_flash_storage_mut(
        &mut self,
    ) -> Option<&mut dyn crate::peripherals::sam::nvmctrl::ManualFlashStorage> {
        None
    }
    /// DMA request routing: append the channels the peripheral request
    /// `request` (`<peripheral id>_tx` / `_rx`, see [`DMA_SIGNAL_TX`]) drives
    /// on this DMA controller — or, for a request multiplexer, on the
    /// controller it feeds — as the routing registers stand NOW (STM32F4
    /// `SxCR.CHSEL`, L4 `CSELR`, G0/G4/WB `DMAMUX_CxCR.DMAREQ_ID`). Default:
    /// not a DMA controller, routes nothing.
    fn dma_route_request(&self, _request: &str, _out: &mut Vec<DmaRoute>) {}
    /// Request multiplexer only: a request this peripheral routed to its
    /// output `channel` (the 1-based [`DmaRoute::channel`] it answered with)
    /// is about to be forwarded. `false` holds it back (STM32 DMAMUX request
    /// synchronisation); the multiplexer then forwards it later as one of its
    /// own `dma_signals` ([`Self::dma_mux_output`]). Default: forward.
    fn dma_mux_forward(&mut self, _channel: u32) -> bool {
        true
    }
    /// Request multiplexer only: the DMA channel one of this peripheral's
    /// own `dma_signals` requests on (a request it held, or generated, and
    /// now forwards). Default: `None` — the signal is an ordinary peripheral
    /// request line ([`DMA_SIGNAL_TX`] / [`DMA_SIGNAL_RX`]).
    fn dma_mux_output(&self, _signal: u32) -> Option<DmaRoute> {
        None
    }
    /// A peripheral-paced DMA item is about to access this peripheral at
    /// `offset`: `read` when the peripheral is the item's source (a receive),
    /// `remaining` the items the channel still has after this one (see
    /// [`DmaRequest::remaining`]). Default: ignored.
    fn dma_access_notice(&mut self, _offset: u64, _read: bool, _remaining: u32) {}
    /// A [`DmaDirection::Load`] item this DMA controller issued has read
    /// `value` (little-endian, `transform.src_width` bytes) from its source:
    /// the data now enters the controller's FIFO. Default: not a FIFO DMA.
    fn dma_load_data(&mut self, _item: &DmaRequest, _value: u32) {}
    /// A DMA item this controller issued faulted on the bus (an unmapped
    /// address: the bus error a DMA reports as a transfer error, STM32
    /// `TEIF` / GPDMA `DTEF`). The bus calls it on the issuer right after
    /// the failed access. Default: ignored.
    fn dma_transfer_error(&mut self, _item: &DmaRequest) {}
    /// Espressif GDMA → GP-SPI: an OUT link bound to this controller pushed
    /// `bytes` into its transmit FIFO. Default: not a GDMA-coupled SPI.
    fn gdma_out_load(&mut self, _bytes: &[u8]) {}
    /// Espressif GP-SPI → GDMA: the bytes a finished `DMA_RX_ENA`
    /// transaction holds for the IN link, if any.
    fn gdma_in_take(&mut self) -> Option<Vec<u8>> {
        None
    }
    /// Espressif GDMA → GP-SPI: the IN link has the received bytes in
    /// memory; the transaction may complete.
    fn gdma_in_delivered(&mut self) {}
    /// Level of one of this peripheral's DMA request lines, for DMA engines
    /// that sample requests (the i.MX RT eDMA through its DMAMUX). `line` is
    /// peripheral-specific (e.g. 0 = transmit, 1 = receive). Default: never
    /// requesting.
    fn dma_request_active(&self, _line: u8) -> bool {
        false
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn restore(&mut self, _state: serde_json::Value) -> SimResult<()> {
        Ok(())
    }

    /// The supply supervisor just released the core from a reset it caused
    /// ([`crate::power`]): record `cause` wherever the chip reports its reset
    /// cause (STM32 `RCC_CSR`, ESP32 `RTC_CNTL` reset state, ...).
    ///
    /// Default: nothing. A peripheral that does not hold a reset-cause
    /// register has nothing to record.
    fn on_supply_reset(&mut self, _cause: crate::power::SupplyResetCause) {}

    /// The node this peripheral belongs to was reset through its reset pin
    /// or `SYSRESETREQ` (a world's scripted node reset). A peripheral that
    /// must return to its reset state for the restarted firmware to behave
    /// as on silicon does so here. Default: nothing — the register state is
    /// kept, and firmware that re-initialises the block is unaffected.
    fn on_node_reset(&mut self) {}

    /// Put this peripheral on a timed network link
    /// ([`crate::network::timed_uart`]). Only a USART with a timed mode
    /// accepts; the default refuses, so a link is never silently untimed.
    fn attach_timed_uart_port(
        &mut self,
        _port: crate::network::timed_uart::TimedUartPort,
    ) -> anyhow::Result<()> {
        anyhow::bail!("this peripheral has no timed UART mode (not an STM32 F1/F2/F4 USART)")
    }

    /// A timed USART's next due cycle after `now`, claimed so the caller can
    /// schedule it (see `Machine::timed_uart_sync`). Default: none.
    fn claim_timed_uart_wake(&self, _now: u64) -> Option<u64> {
        None
    }

    /// This peripheral as a controller that can join a world's timed CAN bus
    /// ([`crate::network::timed_can`]). Default: none — a bus with such an
    /// endpoint stays on the untimed fan-out.
    fn timed_can(&self) -> Option<&dyn crate::network::timed_can::TimedCanController> {
        None
    }

    /// Mutable twin of [`Self::timed_can`].
    fn timed_can_mut(&mut self) -> Option<&mut dyn crate::network::timed_can::TimedCanController> {
        None
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

    /// Walk the external (off-chip) devices attached to this controller,
    /// including anything nested behind an I²C bus switch.
    ///
    /// The inspect counterpart of
    /// [`Self::for_each_attached_sim_input`], and it exists for the same
    /// reason: an I²C slave or SPI display is owned by its CONTROLLER, not by
    /// [`crate::bus::SystemBus::peripherals`], so a walk over the peripheral
    /// list alone cannot see it. On a real customer rig — a TCA9548A plus four
    /// VCNL4010s plus an ILI9341 — `inspect` reported 52 chip-internal
    /// peripherals and not one of the six devices the author actually placed.
    ///
    /// Implementations should delegate to
    /// [`crate::inspect::visit_i2c_device`] /
    /// [`crate::inspect::visit_spi_device`] rather than emitting records
    /// themselves, so the mux unfolding stays in one place. Default: no
    /// attached devices, which is correct for every non-controller peripheral.
    ///
    /// This is the CONTROLLER half of device binding. It is not the whole of
    /// it: a device with no bus address (an HC-SR04 on TRIG/ECHO, a servo, a
    /// CAN tester node) cannot live inside a controller and is held on the bus
    /// instead. Both halves are walked together by
    /// [`crate::bus::SystemBus::for_each_attached_device`], which is the seam
    /// `inspect` actually consumes — call that, not this, to enumerate devices.
    fn for_each_attached_device(&self, _f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {}

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

    /// The named logs this model records during a run, one text line per
    /// entry (see [`crate::peripheral_log`]). `labwired test` asserts on them
    /// with `peripheral_log`.
    ///
    /// Return every log the model keeps, also an empty one: the names given
    /// here are the only list of valid names, so a log that is left out when
    /// empty makes a correct script fail as a config error. Default: no logs.
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        Vec::new()
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

    /// Opt in when service represents ONE clocked peripheral tick, rather
    /// than an absolute deadline. Both walk and event delivery check the bus's
    /// live clock gate. While gated, an event retries after one CPU cycle
    /// without calling the model, so work freezes without losing its wake.
    /// Deadline-driven models must not opt in: they need their own elapsed-time
    /// accounting. Default preserves existing scheduler delivery semantics.
    fn requires_clocked_tick(&self) -> bool {
        false
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

    /// Diagnostic capability: pin a model to its retained legacy reference.
    /// Returns false when unsupported. Use only before execution, then call
    /// `SystemBus::recompute_walk_deletable` and `refresh_peripheral_index`.
    /// This avoids concrete-type reaches in differential harnesses.
    fn force_legacy_walk(&mut self) -> bool {
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

    /// The peripheral whose scheduler wake this one's writes must arm, by bus
    /// index, when that is NOT itself.
    ///
    /// An ALIAS WINDOW is a second bus entry onto another peripheral's state:
    /// classic-ESP32 registers `uart0_ahb_fifo` at `0x6000_0000` over the same
    /// `tx_fifo` as `uart0`, because IDF and arduino-esp32 write TX there
    /// rather than at the APB base. `SystemBus::collect_scheduled_events` runs
    /// on the index that was WRITTEN, so without this the harvest lands on the
    /// alias — which schedules nothing and whose `uses_scheduler()` is the
    /// default `false` — and the model that actually owns the drain is never
    /// woken. Under the legacy walk that could not matter, since the owner was
    /// ticked every cycle whatever address the firmware used; deleting the walk
    /// is what made the write site load-bearing.
    ///
    /// An INDEX, not a name: this is consulted on every MMIO write, and
    /// `find_peripheral_index_by_name` is a linear scan. Resolve it once at
    /// registration, where the owner's index is already in hand.
    ///
    /// Default `None` — only alias windows override.
    fn scheduler_wake_owner(&self) -> Option<usize> {
        None
    }

    /// Cross-core IPI lines this peripheral asserts for `core_id`, as a mask
    /// of CPU interrupt slots. `0` for everything that is not a cross-core
    /// interrupt source.
    ///
    /// A trait method rather than a downcast because the ONE caller,
    /// `SystemBus::dport_cross_core_pending`, runs on the per-instruction
    /// interrupt check. Profiling classic ESP32 at 10M steps put
    /// `pending_cpu_irqs` at 2.70% of the whole run inside `core::any`, with
    /// another 2.25% in `<T as Any>::type_id` — and neither appears at all on
    /// nrf52840 or esp32c3, because their `dport_idx` is `None` and the
    /// function returns before the downcast. The index lookup was already
    /// O(1); it was the `as_any()` + `TypeId` comparison being charged per
    /// instruction.
    fn cross_core_pending(&self, _core_id: u8) -> u32 {
        0
    }

    /// The SPI devices attached to this controller, if it is one.
    ///
    /// A trait pair rather than a downcast chain because `maybe_latch_dc`
    /// runs from all three MMIO WRITE paths, so every write to every
    /// peripheral was asking "are you an SPI?" by trying `Spi`, `Esp32Spi`,
    /// `Esp32c3Spi` and `Esp32s3Spi` in turn -- four `TypeId` comparisons,
    /// all four failing, on the overwhelming majority of writes, which go to
    /// something that is not an SPI at all.
    ///
    /// Profiling classic ESP32 at 10M steps put `maybe_latch_dc` at 1.63% of
    /// the run inside `core::any`, and it appears on neither nrf52840 nor
    /// esp32c3. `None` here is one vtable call and no `TypeId`.
    fn spi_attached_devices(&self) -> Option<&Vec<Box<dyn crate::peripherals::device::SpiDevice>>> {
        None
    }

    /// Mutable twin of [`Self::spi_attached_devices`], for the latch phase.
    fn spi_attached_devices_mut(
        &mut self,
    ) -> Option<&mut Vec<Box<dyn crate::peripherals::device::SpiDevice>>> {
        None
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

    /// The CURRENT state of this peripheral's interrupt line, for level
    /// reconciliation. `Some(level)` marks the model as a LEVEL source —
    /// its line is a live conjunction (status & enable), not a pulse —
    /// and the bus then keeps the NVIC pending bit in step with the line
    /// in BOTH directions: asserted → pended, deasserted → un-pended
    /// (unless the bit came from a software ISPR write). The default
    /// `None` keeps pulse semantics: a pend, once set, is only consumed
    /// by exception entry.
    ///
    /// Silicon behaviour, ARMv6-M/ARMv7-M: for a level-sensitive
    /// interrupt whose line deasserts while the exception is pending or
    /// active-and-repended, the pending state clears — firmware that
    /// clears the source's status flag inside the handler is not
    /// re-entered for the same event. The timer double-entry (two
    /// handler entries per update event, measured 1.95/event against a
    /// cycle-exact grid) was exactly this method's absence.
    fn irq_line_level(&self) -> Option<bool> {
        None
    }

    /// Multi-line twin of [`Self::irq_line_level`], for a LEVEL source that
    /// drives SEVERAL NVIC lines from one model (the STM32 EXTI: EXTI0..4,
    /// EXTI9_5, EXTI15_10 on F1/F4/L4; EXTI0_1/2_3/4_15 on G0; EXTI0..15 on
    /// U5). `irq_line_level` keys on the entry's single `irq`, so such a
    /// model cannot use it; it reports here instead, calling
    /// `report(nvic_irq, level)` once for EVERY line it owns, asserted or not.
    ///
    /// The bus keeps each reported line's NVIC pending bit in step with its
    /// level exactly as for `irq_line_level`: the walk / event path marks the
    /// pend it raises (`NvicState::level_pended`), and the MMIO write choke
    /// drops a marked pend once the line reads low. Without it, the per-cycle
    /// re-pend raised while the handler was still running survived the
    /// handler's write-1-to-clear of EXTI_PR, and a handler that clears PR
    /// without re-checking it ran twice per edge.
    ///
    /// Report ONLY lines this model owns exclusively: a deasserted report
    /// drops any marked pend on that NVIC line, including one another level
    /// source raised. Lines shared with another peripheral keep pulse
    /// semantics (deliver them through `explicit_irqs` only).
    ///
    /// Default: reports nothing (pulse semantics on every explicit IRQ).
    fn irq_line_levels(&self, _report: &mut dyn FnMut(u32, bool)) {}

    /// True for a device that only stores and serves bytes: it does not use
    /// the scheduler, has no scheduler wake owner, no SPI-attached devices, no
    /// IRQ line level, no bus tick and no legacy tick. `SystemBus`'s MMIO
    /// store path then skips the hooks that exist for devices with those
    /// properties. On the ESP32 perf fixture that is ~150 of the 411 Ir a
    /// stack store cost, spent calling hooks that could do nothing for RAM
    /// (`docs/performance/2026-09-24-xtensa-step-loop-plan.md`, step 1).
    ///
    /// A promise, not a hint: a device that returns `true` and has any of
    /// those properties silently loses its events. Debug builds re-check the
    /// promise on every store that takes the fast path
    /// (`SystemBus::debug_check_plain_memory`).
    fn is_plain_memory(&self) -> bool {
        false
    }

    /// Tell the peripheral the clock its system's core runs at, in Hz — the
    /// effective `SystemBus::cpu_hz`, i.e. the manifest override if there is
    /// one and otherwise `ChipDescriptor::cpu_hz`. Called from the same attach
    /// choke points as [`Peripheral::attach_cycle_clock`].
    ///
    /// For a model whose behaviour is specified in WALL time but driven in
    /// CPU cycles — a BLE controller told to advertise every 100 ms, say —
    /// this is the conversion factor. Taking it here rather than from a
    /// per-peripheral `config:` key keeps
    /// [`labwired_config::ChipDescriptor::cpu_hz`] the single source of the
    /// core clock: a yaml that restated the frequency next to the peripheral
    /// would be a second place to change it, and the two would diverge.
    ///
    /// Default no-op. A model that never receives it must have a usable
    /// default, since hand-built buses bypass the choke points.
    fn attach_cpu_hz(&mut self, _hz: u64) {}

    /// Move this peripheral onto a lab's own BLE air, replacing whatever air it
    /// was minted with. Called by [`crate::bus::SystemBus::attach_lab_air`] for
    /// every peripheral in a multi-node world.
    ///
    /// A BLE controller built by the ordinary factory joins the process-global
    /// air, because the factory has no lab identity to hand it. That is right
    /// for a single lab and wrong for two: without this, two labs in one
    /// process — two worker threads, or two tests — hear each other's
    /// advertisements as peer traffic.
    ///
    /// An implementation must also re-join the cursor at the new air's current
    /// sequence. A radio has no history buffer, and an air outlives a
    /// simulation restart, so a controller that kept a stale cursor would
    /// replay the previous run's backlog as live packets.
    ///
    /// Default no-op: a peripheral with no radio has no air to move.
    fn attach_ble_air(&mut self, _air: crate::peripherals::ble_air::BleAirBus) {}

    /// Drive the analog level, in millivolts, on one of this peripheral's ADC
    /// input channels. Returns whether it took it.
    ///
    /// The seam a `system.yaml` analog source (a potentiometer, an NTC) uses
    /// to move what firmware converts. `false` for a peripheral that is not an
    /// ADC, and for a channel this one does not have.
    ///
    /// New ADC models implement this; the five that predate it are still found
    /// by a downcast chain in `bus::sim_inputs`, which this is the replacement
    /// for. See the note there.
    fn set_adc_channel_input(&mut self, _channel: u8, _millivolts: u16) -> bool {
        false
    }

    /// Release a held ADC input. Returns false for unsupported peripherals or
    /// channels; a released input follows the peripheral's documented default.
    fn clear_adc_channel_input(&mut self, _channel: u8) -> bool {
        false
    }

    /// How many analog input channels this ADC has: channels `0..count` are
    /// the ones [`SystemBus::seed_adc_channel`](crate::bus::SystemBus) can
    /// drive on it. `None` for a peripheral that is not an ADC LabWired can
    /// drive.
    ///
    /// Each model answers from its own register layout, so a co-simulation
    /// route to a channel the converter does not have is refused when the
    /// session is built rather than written nowhere for the whole run.
    fn adc_channel_count(&self) -> Option<u8> {
        None
    }

    /// Set the ambient (die) temperature an on-chip temperature sensor
    /// reports, in °C. `true` when this peripheral has such a sensor and took
    /// the value; `false` otherwise (the default). The bus fans one world
    /// temperature out to every peripheral through
    /// [`crate::bus::SystemBus::set_ambient_temperature_c`].
    fn set_ambient_temperature_c(&mut self, _celsius: f64) -> bool {
        false
    }

    /// Hand the peripheral the machine's ONE universal bus trace, plus the name
    /// it should stamp events with. Called from the same registration choke
    /// points as [`Peripheral::attach_cycle_clock`] and
    /// [`Peripheral::attach_irq_line`] (`add_peripheral`,
    /// `replace_or_add_peripheral`, `push_peripheral`), so a model reaches it by
    /// being registered at all rather than by a per-family call site.
    ///
    /// I²C and SPI do NOT use this: their traffic is recorded by wrapping the
    /// attached slave (`bus_trace::wrap_i2c` / `wrap_spi`), which is structurally
    /// unbypassable. It exists for the buses with nothing to wrap — a UART or a
    /// CAN controller transacts against a wire, not against an attachable
    /// device, so the model itself has to record.
    ///
    /// Default no-op: the vast majority of peripherals carry no bus traffic.
    /// A model that DOES carry traffic and ignores this is exactly the fork
    /// `crate::tests::bus_trace_one_home` fails on.
    fn attach_bus_trace(&mut self, _name: &str, _trace: &crate::bus::bus_trace::BusTrace) {}

    /// The trace handle this model records into, if it records at all — `None`
    /// for the vast majority, which carry no bus traffic.
    ///
    /// Exists so the one-home gate can prove by `Arc` identity that a model
    /// which DOES record is recording into the machine's ring rather than into
    /// the private handle it was born with. That distinction is invisible from
    /// the outside: an orphaned ring accepts every write and simply is never
    /// read, so the instrument shows an empty panel and no error. Comparing
    /// snapshots cannot detect it (two empty rings are equal); comparing
    /// identity can.
    fn bus_trace_handle(&self) -> Option<crate::bus::bus_trace::BusTrace> {
        None
    }

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

    /// Whether this peripheral recorded a hardware operation that the machine
    /// boundary still has to drain — the H5/U5 FLASH erase/bank-swap cell.
    ///
    /// A capability rather than a concrete-type cast, for the reason the
    /// downcast ratchet gives: reach for the concrete type through a trait.
    /// Mirrors `matrix_irq_source_id`, which upstream added for exactly this.
    /// `SystemBus::has_pending_flash_op` probes this after every instruction on
    /// an op-modelling bus, so it is also the hot path. Default `false`.
    fn has_pending_op(&self) -> bool {
        false
    }

    /// Static interrupt-matrix source id this instance asserts, when the model
    /// has exactly one fixed source (e.g. the C3/C6 I2C_EXT0 engine). Lets the
    /// descriptor -> trait wiring be asserted without downcasting to a concrete
    /// type. Default `None` — models with several sources use
    /// [`Self::matrix_irq_sources_into`].
    fn matrix_irq_source_id(&self) -> Option<u32> {
        None
    }

    /// Convenience returning form (tests, one-shot callers). The hot per-batch
    /// poll uses [`Self::matrix_irq_sources_into`] with retained scratch instead.
    fn matrix_irq_sources(&self) -> Vec<u32> {
        let mut out = Vec::new();
        self.matrix_irq_sources_into(&mut out);
        out
    }
}

// ── Reset cause ───────────────────────────────────────────────────────────

/// Why a peripheral asked the machine to reboot the CPU.
///
/// `Watchdog` and `System` match the K32W041 RESETCAUSE bits WDTRESET and
/// SYSTEMRESET. Peripherals that only need the reboot leave the cause unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeripheralResetCause {
    Watchdog,
    System,
}
