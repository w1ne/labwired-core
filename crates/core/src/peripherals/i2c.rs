// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// I2C is one struct PER FAMILY behind the `I2c` enum:
//   * `F1I2c` — the legacy peripheral (CR1/CR2/OAR/DR/SR1/SR2/CCR/TRISE) AND
//     the full transaction state machine. START/STOP live in CR1.
//   * `L4I2c` — the modern peripheral (CR1/CR2/OAR/TIMINGR/ISR/ICR/RXDR/TXDR),
//     register-fidelity latching PLUS a minimal master transaction engine
//     (START/STOP/AUTOEND in CR2; address phase → ISR.NACKF when no slave acks).
// Each variant owns ALL of its own registers and state — an F1 I2C cannot
// carry TIMINGR/ISR, an L4 I2C cannot carry SR1/DR. CR1/CR2/OAR and the
// attached-device list exist on both because both families genuinely have
// them. The chip-yaml `profile` selects the variant.

use crate::peripherals::i2c_waveform::I2cNarrator;
use crate::peripherals::pad_lines::PadLines;
use crate::{CycleClock, SimResult};
use std::cell::{Cell, RefCell};
use std::str::FromStr;

pub trait I2cDevice: Send {
    fn address(&self) -> u8;
    fn read(&mut self) -> u8;
    fn write(&mut self, data: u8);
    fn start(&mut self) {}
    fn stop(&mut self) {}

    /// What this device can show of itself — its own inspect evidence.
    ///
    /// The ONE place a an I²C device's artifacts are decided is the model
    /// itself, next to the buffers it owns. Default: nothing, which is correct
    /// for a sensor with no display surface and honest for anything else —
    /// absent means "this engine has nothing to show", never "the screen was
    /// blank". See [`crate::inspect::DeviceEvidence`] for why this is not a
    /// central match on concrete types.
    ///
    /// Implementations must read the model's REAL buffer and synthesize
    /// nothing; a panel that was never painted reports zero.
    fn artifacts(
        &self,
        _id: &str,
        _opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        Vec::new()
    }

    /// Does this device answer to `addr` on the wire *right now*?
    ///
    /// A plain slave owns exactly one address, so the default is the obvious
    /// `self.address() == addr` — every existing model keeps its behaviour with
    /// no edit. The hook exists for devices whose answered-address set is not a
    /// singleton and is not static: an I²C **bus switch** (TCA9548A) answers to
    /// its own control address *and*, while a channel is enabled, to every
    /// address reachable behind that channel. That set changes whenever
    /// firmware rewrites the switch's control register, so it cannot be
    /// flattened into one `address()` at attach time.
    ///
    /// Controllers MUST resolve a slave with this, never by comparing
    /// `address()` — a flat `position(|d| d.address() == addr)` is first-match
    /// and makes four identical sensors behind a mux collapse into one.
    fn claims_address(&self, addr: u8) -> bool {
        self.address() == addr
    }

    /// Tell the device which address the master just selected, immediately
    /// after [`claims_address`](Self::claims_address) returned `true` for it and
    /// before any `start`/`write`/`read`/`stop` of that transaction.
    ///
    /// Default no-op: a single-address slave already knows who it is. A bus
    /// switch uses it to decide whether this transaction targets its own
    /// control register or is to be forwarded to the downstream device(s) that
    /// claim `addr` on the currently enabled channel(s).
    fn select_address(&mut self, addr: u8) {
        let _ = addr;
    }

    /// Walk every [`SimInput`](crate::sim_input::SimInput) surface this device
    /// exposes, including devices nested *behind* it. Returns `true` if `f`
    /// asked to stop early.
    ///
    /// The default is exactly the old behaviour — a device offers at most its
    /// own [`as_sim_input_mut`](Self::as_sim_input_mut). It is overridden by
    /// containers (the TCA9548A mux) so their children stay reachable from the
    /// ONE stimulus walk in [`crate::bus::SystemBus::for_each_sim_input`].
    /// Without it, putting a sensor behind a mux would silently subtract it
    /// from `list_inputs` / `set_input` — the same class of invisible-device
    /// bug the controller-level seam was introduced to kill.
    fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        match self.as_sim_input_mut() {
            Some(si) => f(si),
            None => false,
        }
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }
    /// Runtime-drivable view of this device, if it accepts simulated input.
    /// Overridden by input devices (accelerometers, …) so the generic
    /// [`crate::Machine::set_input`] resolver can reach them without a
    /// downcast. Default `None` = not an input device.
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        None
    }

    /// Advance this device's free-running sample/measurement clock by `us`
    /// microseconds of wall-clock time.
    ///
    /// Real sensors sample on their own oscillator, independent of when the CPU
    /// gets around to reading them: a PPG FIFO keeps filling at its configured
    /// rate whether or not firmware is draining it. A bus master that knows the
    /// elapsed wall-clock calls this on a slave immediately before servicing it,
    /// so a *late* poll observes exactly the samples that accrued while the CPU
    /// was busy elsewhere — and a FIFO that was allowed to overrun reports the
    /// overflow it really would have. Without this hook a model only advances on
    /// the very transactions that would have prevented the overflow, which hides
    /// precisely the CPU-starvation failures worth simulating.
    ///
    /// Default no-op: a purely register-mapped device has no clock to advance.
    fn advance_time_us(&mut self, _us: u64) {}
}

/// I2C register layout selector. STM32F1/F2/F4 share the legacy I2C
/// peripheral; STM32L4/F7/H5/G0 share the modern peripheral. The config-facing
/// value maps 1:1 to a dedicated family struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum I2cRegisterLayout {
    #[default]
    Stm32F1,
    /// STM32L4 family (also F7/H5/G0). Verified against real NUCLEO-L476RG
    /// silicon via SWD register dump.
    Stm32L4,
    /// NXP Kinetis classic I2C (KW41Z / K series): byte-oriented A1/F/C1/S/D,
    /// interrupt-driven master matching the fsl_i2c HAL.
    Kinetis,
    /// Silicon Labs EFR32/EFM32 Series-2: CMD/STATE/STATUS/TXDATA/RXDATA with
    /// an IF flag per event, driven by `emlib`'s `I2C_Transfer`.
    Efr32s2,
}

impl FromStr for I2cRegisterLayout {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32l4" | "l4" | "stm32f7" | "f7" | "stm32h5" | "h5" | "stm32g0" | "g0" => {
                Ok(Self::Stm32L4)
            }
            "kinetis" | "nxp" | "nxp_i2c" | "kw41z" | "mkw41z4" => Ok(Self::Kinetis),
            "efr32s2" | "efr32" | "efm32" | "gecko" => Ok(Self::Efr32s2),
            _ => Err(format!(
                "unsupported I2C register layout '{}'; supported: stm32f1, stm32l4, kinetis, efr32s2",
                value
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Default)]
enum I2cState {
    #[default]
    Idle,
    StartPending,
    AddressPending,
    DataPending,
    /// A data byte is on the wire: it has been handed to the slave, and the
    /// nine SCL bit-times it occupies are being charged before TC/STOPF land.
    ///
    /// Real silicon does not complete a byte instantly — the byte is clocked
    /// out one bit at a time and TXE/TC only follow. Modelling that time is
    /// what makes the data phase visible to an instrument: without it the
    /// transfer's waveform is longer than the cycles the transfer was ever
    /// charged, and the capture layer has nowhere to put it (see
    /// [`L4I2c::wire_flush`]).
    DataSending,
}

/// One element of a legacy-I²C transaction's narration, recorded as the phase
/// model performs it and replayed onto the pads at STOP (see
/// [`F1I2c::wire_flush`]).
///
/// The legacy controller is the one STM32 I²C generation where the SOFTWARE
/// drives every bus condition explicitly — CR1.START, CR1.STOP, and a repeated
/// START in between — so unlike the L4 model (which infers a single START/STOP
/// pair around an AUTOEND transfer) this records the conditions the firmware
/// actually asked for. A sensor read (`write register pointer, repeated START,
/// read back`) therefore narrates as the two addressed frames it really is,
/// not as one run-on transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireEvent {
    /// A START condition. Emitted again mid-transaction for a repeated START,
    /// which [`I2cNarrator::start`] renders as the SDA-release-then-fall the
    /// controller drives.
    Start,
    /// One 9-bit frame: the byte on the wire and whether the ACK slot was
    /// pulled low.
    Frame(u8, bool),
}

/// Events one transaction may buffer before the narration is abandoned.
///
/// Firmware always terminates an I²C transfer with a STOP — that is what
/// releases the bus — so this is reached only by firmware that has hung the
/// controller mid-transfer and keeps writing DR. Bounding the buffer keeps a
/// long-running sim from growing without limit; see [`F1I2c::wire_flush`] for
/// why hitting it publishes NOTHING rather than a truncated waveform.
const WIRE_EVENT_CAP: usize = 1024;

// ── STM32F1 legacy I2C (registers + transaction state machine) ───────────────
#[derive(serde::Serialize)]
pub struct F1I2c {
    cr1: u32,
    cr2: u32,
    oar1: u32,
    oar2: u32,
    dr: u32,
    sr1: u32,
    sr2: u32,
    /// NVIC vector for this instance's ERROR interrupt (e.g. I2C1_ER_IRQn = 32
    /// on STM32F4), distinct from the EVENT vector carried by the peripheral's
    /// `irq:` field. AF/BERR/ARLO/OVR are error conditions: silicon raises them
    /// on the ER line under CR2.ITERREN, and the HAL's ERROR handler is what
    /// clears AF and completes a NACKed transfer. Without this vector an
    /// interrupt-mode driver (STM32duino 3.x uses HAL_I2C_Master_Transmit_IT)
    /// never learns the address was NACKed and spins until its 100 ms timeout.
    irq_error: Option<u32>,
    ccr: u32,
    trise: u32,

    state: I2cState,
    cycles_remaining: u32,

    #[serde(skip)]
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    #[serde(skip)]
    current_target: Option<usize>,
    #[serde(skip)]
    is_reading: bool,
    #[serde(skip)]
    stop_requested: bool,
    #[serde(skip)]
    rxne_consumed: Cell<bool>,
    #[serde(skip)]
    read_dr_consumed: Cell<bool>,
    /// ADDR (SR1 bit1) software-clear sequence: set after SR1 is read while
    /// ADDR is set; consumed on the following SR2 read (RM0008 §26.6.6 —
    /// "ADDR is cleared by reading SR1 then SR2"). Held in a Cell so the
    /// clear can happen on a pure `&self` read path.
    #[serde(skip)]
    addr_sr1_seen: Cell<bool>,
    #[serde(skip)]
    addr_cleared: Cell<bool>,

    /// Bus-published cycle clock (walk-free campaign). `Some` once the bus
    /// registration choke attaches it; `None` keeps the model on the legacy
    /// walk. Mirrors the Kinetis variant — see `F1I2c::scheduler_mode`.
    #[serde(skip)]
    clock: Option<CycleClock>,
    /// Scheduler mode only: `true` while the per-cycle transaction-engine event
    /// is live in the scheduler heap. Armed when the transaction becomes active
    /// (a write starts a countdown, or a `&self` receive read latches a re-arm);
    /// self-perpetuates at delay 1 while the transfer stays active, stops when it
    /// returns fully idle. Same held-level self-pacing the Kinetis variant uses.
    #[serde(skip)]
    chain_live: bool,

    /// Wire levels published to AF-routed SCL/SDA pads, so a logic analyzer
    /// clipped to this bus measures a real waveform instead of a flat line.
    /// Created lazily by [`Self::pad_lines_arc`] at bus wiring time; `None`
    /// when no GPIO port routes this controller's pads. Mirrors [`L4I2c`].
    #[serde(skip)]
    lines: Option<std::sync::Arc<PadLines>>,
    /// Conditions and frames of the transaction in flight, oldest first —
    /// buffered so the whole transfer is narrated onto the pads as ONE
    /// contiguous waveform at STOP. See [`Self::wire_flush`].
    ///
    /// ⚠️ `RefCell`, unlike [`L4I2c::wire_frames`]'s plain `Vec`, because a
    /// legacy master-receive pulls its second and later bytes out of the slave
    /// on the `&self` DR read path (see [`Self::read`], the `read_dr_consumed`
    /// branch) — the same reason `rxne_consumed` and `read_dr_consumed` are
    /// `Cell`s. A `&mut`-only recorder would silently drop every byte of a
    /// multi-byte read after the first, which decodes as a SHORTER transfer
    /// than the one that crossed the bus.
    #[serde(skip)]
    wire_events: RefCell<Vec<WireEvent>>,
    /// Set when a transaction overran [`WIRE_EVENT_CAP`]. Sticky until the next
    /// flush, which then publishes nothing.
    #[serde(skip)]
    wire_overflow: Cell<bool>,
}

impl Default for F1I2c {
    fn default() -> Self {
        Self {
            cr1: 0,
            cr2: 0,
            oar1: 0,
            oar2: 0,
            dr: 0,
            sr1: 0,
            sr2: 0,
            irq_error: None,
            ccr: 0,
            // TRISE reset value is 0x0002 (RM0008 §26.6.9) — silicon-confirmed
            // on STM32F103 over SWD (reads 0x00000002 after RCC clock enable,
            // before any write).
            trise: 0x0002,
            state: I2cState::Idle,
            cycles_remaining: 0,
            attached_devices: Vec::new(),
            current_target: None,
            is_reading: false,
            stop_requested: false,
            rxne_consumed: Cell::new(false),
            read_dr_consumed: Cell::new(true),
            addr_sr1_seen: Cell::new(false),
            addr_cleared: Cell::new(false),
            clock: None,
            chain_live: false,
            lines: None,
            wire_events: RefCell::new(Vec::new()),
            wire_overflow: Cell::new(false),
        }
    }
}

impl F1I2c {
    /// The shared pad-line cell for this controller, created on first use.
    /// Called at bus wiring time by `wire_stm32_i2c_pads`; an open-drain bus
    /// with pull-ups idles high on both lines. Mirrors [`L4I2c::pad_lines_arc`].
    pub(crate) fn pad_lines_arc(&mut self) -> std::sync::Arc<PadLines> {
        self.lines
            .get_or_insert_with(|| std::sync::Arc::new(PadLines::new(I2C_LINES, &[true, true])))
            .clone()
    }

    /// The wire this controller publishes, for a
    /// [`LogicSource::Wire`](crate::logic_capture::LogicSource::Wire) channel.
    /// `None` until a lab wires its pads, because that is when the cell is
    /// created — the controller has nothing to show before then either.
    pub(crate) fn wire_lines(&self) -> Option<&PadLines> {
        self.lines.as_deref()
    }

    /// Engine cycles in one SCL period, derived from CCR exactly as the legacy
    /// silicon derives it (RM0008 §26.6.8 / RM0090 §27.6.8, `I2C_CCR`):
    ///
    /// * Standard mode (`F/S = 0`): `T_high = T_low = CCR × T_PCLK1`, so one
    ///   period is `2 × CCR`.
    /// * Fast mode, `DUTY = 0`: `T_low = 2 × CCR × T_PCLK1`,
    ///   `T_high = CCR × T_PCLK1` → `3 × CCR`.
    /// * Fast mode, `DUTY = 1` (16/9): `T_low = 16 × CCR × T_PCLK1`,
    ///   `T_high = 9 × CCR × T_PCLK1` → `25 × CCR`.
    ///
    /// Those are APB1 (`PCLK1`) periods, used here as engine cycles — the same
    /// identity the STM32 SPI bit engine already assumes for its own `2^BR`
    /// half-period derivation (see the module header of
    /// [`crate::peripherals::spi`]). It is exact at the RCC reset defaults
    /// (`RCC_CFGR.PPRE1 = 0b0xx`, APB1 not divided) and off by the APB1
    /// prescaler once firmware raises the core clock and divides APB1 — a
    /// factor of 2 on a typical F401 clock tree, 4 on an F407. Only the
    /// magnitude is load-bearing: the narrated frame CONTENTS are exact either
    /// way, and this is documented as the known limit on the measured bit rate,
    /// the same call [`L4I2c::address_phase_cycles`] makes with `CORE_PER_KCLK`.
    ///
    /// The floor of 2 keeps the low and high half-periods distinguishable when
    /// firmware has not programmed CCR at all (its reset value is 0).
    fn bit_time_cycles(&self) -> u64 {
        let ccr = u64::from(self.ccr & 0x0FFF);
        let fast = self.ccr & 0x8000 != 0;
        let duty16_9 = self.ccr & 0x4000 != 0;
        let period = match (fast, duty16_9) {
            (false, _) => 2 * ccr,
            (true, false) => 3 * ccr,
            (true, true) => 25 * ccr,
        };
        period.max(2)
    }

    /// Record a bus condition or frame this transfer put on the wire. Buffered,
    /// not published: see [`Self::wire_flush`]. No routed pads ⇒ nothing to
    /// record, and the call costs one branch.
    ///
    /// `&self` so the `&self` DR read path can record too (see
    /// [`Self::wire_events`]).
    fn wire_record(&self, event: WireEvent) {
        if self.lines.is_none() {
            return;
        }
        let mut events = self.wire_events.borrow_mut();
        if events.len() >= WIRE_EVENT_CAP {
            self.wire_overflow.set(true);
            return;
        }
        events.push(event);
    }

    /// Publish the completed transaction's waveform onto the routed pads.
    ///
    /// The phase model has already exchanged the bytes; this narrates the wire
    /// activity they imply so the bus is measurable (see
    /// [`crate::peripherals::i2c_waveform`] for what that does and does not
    /// model).
    ///
    /// Called at STOP — the one point the legacy controller says the
    /// transaction is over — so the whole transfer, repeated STARTs included,
    /// is emitted as ONE contiguous run ending at the present cycle. That is
    /// forced by the same timing model that forces it on [`L4I2c::wire_flush`]:
    /// this controller charges no wire time for a data byte (`cycles_remaining`
    /// is 0 or 1 on every legacy phase), so there is no room on the timeline to
    /// place each frame where it "happened", and narrating frame by frame would
    /// stamp later frames in the future, where the capture layer collapses them
    /// onto one cycle — a spike where a transaction belongs.
    ///
    /// ⚠️ An overrun transaction (see [`WIRE_EVENT_CAP`]) publishes NOTHING.
    /// The alternative is emitting the first 1024 frames as if they were the
    /// whole transfer, which decodes cleanly to a byte sequence that never
    /// crossed the bus as such — a confident wrong answer, which is worse than
    /// a flat line.
    fn wire_flush(&mut self) {
        let mut events = std::mem::take(&mut *self.wire_events.borrow_mut());
        let overflowed = self.wire_overflow.replace(false);
        let Some(lines) = self.lines.clone() else {
            return;
        };
        if events.is_empty() || overflowed {
            return;
        }
        let mut wave = I2cNarrator::new(LINE_SCL, LINE_SDA, self.bit_time_cycles());
        for event in events.drain(..) {
            match event {
                WireEvent::Start => wave.start(),
                WireEvent::Frame(byte, acked) => wave.frame(byte, acked),
            }
        }
        wave.stop();
        let now = lines.tap_clock().unwrap_or(0);
        // A transfer this early in a run has less history behind it than the
        // waveform needs; the narrator compresses to fit rather than emitting a
        // spike, and says so. Nothing here can act on that — the trace still
        // decodes to the right bytes — so the verdict is deliberately dropped,
        // exactly as `L4I2c::wire_flush` drops it.
        let _fit = wave.emit_ending_at(&lines, now);
    }

    /// The byte the master ACKs a received frame with: CR1.ACK (bit 10) is what
    /// firmware clears before the final byte of a read to tell the slave to stop
    /// driving (RM0008 §26.6.1).
    fn master_acks_reads(&self) -> bool {
        self.cr1 & 0x0400 != 0
    }

    /// True when the event scheduler owns this controller's transaction engine
    /// (feature on AND bus clock attached).
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Cycles on which the legacy `tick()` does observable work: any in-flight
    /// countdown (`state != Idle`), the master transfer window (SR2.BUSY), a
    /// pending `&self`-read RXNE re-arm, or a deferred STOP. Outside this window
    /// `tick()` is a proven no-op (the `rxne_consumed` drain and the countdown
    /// are the only side effects, and both are gated by exactly these flags), so
    /// the event chain may stop and let idle fast-forward engage — while any
    /// extra idle cycle it does run is observationally inert. Over-covering is
    /// therefore always safe; this predicate is deliberately generous so a
    /// receive re-arm latched by a `&self` DR read is never missed.
    #[inline]
    fn active(&self) -> bool {
        self.state != I2cState::Idle
            || (self.sr2 & 0x0002) != 0 // BUSY: master transfer in flight
            || self.rxne_consumed.get()
            || self.stop_requested
            // Level EV must keep walking while ITEVTEN/ITBUFEN flags are live.
            || self.irq_level()
    }

    /// Set the ERROR-line NVIC vector for this instance.
    pub fn set_error_irq(&mut self, irq: u32) {
        self.irq_error = Some(irq);
    }

    /// Vectors to pend on the ERROR line this cycle.
    ///
    /// SR1 error bits (RM0090 §27.6.6): BERR 8, ARLO 9, AF 10, OVR 11,
    /// PECERR 12, TIMEOUT 14, SMBALERT 15. Bit 13 is reserved. Gated on
    /// CR2.ITERREN (bit 8), exactly as silicon gates the ER line.
    fn error_irqs(&self) -> Option<Vec<u32>> {
        const ERR_MASK: u32 =
            (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12) | (1 << 14) | (1 << 15);
        if (self.cr2 & (1 << 8)) != 0 && (self.sr1 & ERR_MASK) != 0 {
            self.irq_error.map(|n| vec![n])
        } else {
            None
        }
    }

    /// SR1 with ADDR masked once the SR1→SR2 clear sequence has completed.
    #[inline]
    fn effective_sr1(&self) -> u32 {
        let mut s = self.sr1;
        if self.addr_cleared.get() {
            s &= !0x0002;
        }
        s
    }

    /// Level-sensitive I2C event IRQ (RM0008 §26.5 / NVIC): the EV line stays
    /// asserted while any enabled status flag is set. One-shot pulse-on-transition
    /// is not silicon — HAL_I2C_EV_IRQHandler chains SB → ADDR → TXE → BTF across
    /// re-entries, and each entry requires the line still high after the previous
    /// flag is cleared.
    ///
    /// ITEVTEN (CR2.9): SB, ADDR, ADD10, STOPF, BTF.
    /// ITBUFEN (CR2.10): TXE, RXNE (used with ITEVTEN by HAL Master_Transmit_IT).
    #[inline]
    fn irq_level(&self) -> bool {
        let itevt = (self.cr2 & (1 << 9)) != 0;
        let itbuf = (self.cr2 & (1 << 10)) != 0;
        if !itevt && !itbuf {
            return false;
        }
        let sr1 = self.effective_sr1();
        // SB=0, ADDR=1, BTF=2, STOPF=4 (ADD10=3 rare — leave in 0x1F with EVT).
        let evt_flags = sr1 & 0x001F;
        let buf_flags = sr1 & 0x00C0; // TXE=7, RXNE=6
        (itevt && evt_flags != 0) || (itbuf && buf_flags != 0)
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.oar1,
            0x0C => self.oar2,
            0x10 => self.dr,
            0x14 => {
                let s = self.effective_sr1();
                // Start of ADDR-clear sequence (RM0008 §26.6.6).
                if (s & 0x0002) != 0 {
                    self.addr_sr1_seen.set(true);
                }
                s
            }
            0x18 => {
                // Completing ADDR clear: SR1 was read with ADDR set, now SR2.
                if self.addr_sr1_seen.replace(false) {
                    self.addr_cleared.set(true);
                }
                self.sr2
            }
            0x1C => self.ccr,
            0x20 => self.trise,
            _ => {
                crate::census_reg!("i2c:F1I2c", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u16) {
        match offset {
            0x00 => {
                // CR1 writable mask 0xBFFB (bits 2,14 reserved) — silicon-
                // confirmed on F103. SWRST (bit 15) resets the peripheral on
                // real silicon; that side effect is not modelled here.
                self.cr1 = (value as u32) & 0xBFFB;
                if (value & 0x0100) != 0 && self.state == I2cState::Idle {
                    // Instant SB: Arduino/HAL Wire polls SR1.SB immediately
                    // after CR1.START; a multi-instruction tick interval would
                    // livelock the wait loop (matrix L3). One I2C bit time is
                    // always << firmware poll period here.
                    self.state = I2cState::StartPending;
                    self.cycles_remaining = 0;
                    // Software asked for a (possibly repeated) START; the wire
                    // gets one. Recorded here rather than in `tick` because
                    // this is the only place the request is distinguishable
                    // from the address phase that follows it.
                    self.wire_record(WireEvent::Start);
                    let _ = self.tick();
                }
                if (value & 0x0200) != 0 {
                    // STOP requested. Defer if a data phase is in flight so
                    // RXNE/BTF latch first (HAL "NACK+STOP → poll RXNE → read
                    // DR" ordering); otherwise complete synchronously.
                    if matches!(self.state, I2cState::DataPending | I2cState::AddressPending) {
                        self.stop_requested = true;
                    } else {
                        self.cr1 &= !0x0200;
                        // The transaction is over: everything it put on the bus
                        // goes onto the pads now, terminated by the STOP.
                        self.wire_flush();
                        // STOP clears master/busy/TRA (RM0008 SR2).
                        self.sr2 &= !0x0007;
                        // Drop the transmitter/bus-event flags so the level EV
                        // line deasserts — but NOT RXNE. A master-receive latches
                        // RXNE with the byte in DR and clears it only on the DR
                        // read (RM0090 §27.6.7); STOP releases the bus, it does
                        // not discard an already-received byte. Clearing RXNE here
                        // wiped the byte before a poll-mode 1-byte NACK read (set
                        // ACK=0+STOP, then poll RXNE) could observe it → hang.
                        self.sr1 &= !0x0087; // TXE|BTF|ADDR|SB (keep RXNE 0x40)
                        self.addr_cleared.set(false);
                        self.addr_sr1_seen.set(false);
                        if let Some(idx) = self.current_target {
                            self.attached_devices[idx].borrow_mut().stop();
                        }
                        self.current_target = None;
                        self.state = I2cState::Idle;
                    }
                }
            }
            // Writable masks silicon-confirmed on F103 (RM0008 §26.6):
            // CR2 0x1F3F, OAR1 0xC3FF, OAR2 0x00FF.
            0x04 => self.cr2 = (value as u32) & 0x1F3F,
            0x08 => self.oar1 = (value as u32) & 0xC3FF,
            0x0C => self.oar2 = (value as u32) & 0x00FF,
            0x10 => {
                self.dr = (value & 0xFF) as u32;
                if self.state == I2cState::Idle {
                    // SB uses effective SR1 (ADDR-clear overlay does not mask SB).
                    if (self.effective_sr1() & 0x01) != 0 {
                        self.state = I2cState::AddressPending;
                        // Instant ADDR/TXE: one I2C bit-time ≪ ISR/poll interval.
                        self.cycles_remaining = 0;
                        let addr = (self.dr >> 1) as u8;
                        self.is_reading = (self.dr & 1) != 0;
                        self.current_target = self
                            .attached_devices
                            .iter()
                            .position(|d| d.borrow().claims_address(addr));
                        if let Some(idx) = self.current_target {
                            let mut dev = self.attached_devices[idx].borrow_mut();
                            dev.select_address(addr);
                            dev.start();
                        }
                        let _ = self.tick();
                    } else if (self.effective_sr1() & 0x80) != 0 || (self.sr2 & 0x0001) != 0 {
                        // Data byte while master (TXE or MSL): shift out, clear TXE/BTF.
                        self.state = I2cState::DataPending;
                        self.cycles_remaining = 0;
                        self.sr1 &= !0x80;
                        self.sr1 &= !0x04;
                        if !self.is_reading {
                            if let Some(idx) = self.current_target {
                                self.attached_devices[idx].borrow_mut().write(self.dr as u8);
                                // The byte goes out on the wire here, so the
                                // wire says so. An addressed slave that took
                                // the byte ACKed it.
                                self.wire_record(WireEvent::Frame(self.dr as u8, true));
                            }
                        }
                        let _ = self.tick();
                    }
                }
            }
            0x14 => {
                // SR1 is NOT a plain register. The error bits (BERR 8, ARLO 9,
                // AF 10, OVR 11, PECERR 12, TIMEOUT 14, SMBALERT 15) are rc_w0
                // — writing 0 clears them — and every other bit is read-only,
                // set by hardware alone (RM0090 §27.6.6).
                //
                // A raw `self.sr1 = value` was catastrophic with real firmware:
                // the HAL clears a flag with `SR1 = ~FLAG` (e.g. 0xFFFFFBFF for
                // AF), which under a raw assignment SET every other flag at
                // once — SB, ADDR, BTF, TXE — and the driver's state machine
                // then chased events that never happened.
                const CLEARABLE: u32 =
                    (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12) | (1 << 14) | (1 << 15);
                let cleared = CLEARABLE & !(value as u32);
                self.sr1 &= !cleared;
            }
            // SR2 is entirely read-only (MSL/BUSY/TRA/GENCALL/DUALF and PEC).
            // Writes are discarded by hardware.
            0x18 => {}
            // CCR 0xCFFF (12-bit divider + DUTY + F/S), TRISE 0x3F (6-bit) —
            // silicon-confirmed on F103.
            0x1C => self.ccr = (value as u32) & 0xCFFF,
            0x20 => self.trise = (value as u32) & 0x3F,
            _ => {
                crate::census_reg!("i2c:F1I2c", offset, "write");
            }
        }
    }

    fn read(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        if reg_offset == 0x10 && byte_offset == 0 && self.is_reading && (self.sr1 & 0x0040) != 0 {
            if !self.read_dr_consumed.replace(true) {
                return (self.dr & 0xFF) as u8;
            }
            if let Some(idx) = self.current_target {
                let byte = self.attached_devices[idx].borrow_mut().read();
                // A second (or later) byte of a master receive, pulled straight
                // out of the slave on this read. It crossed the bus like any
                // other frame and must appear on the wire, or a multi-byte read
                // narrates one byte shorter than it really was.
                self.wire_record(WireEvent::Frame(byte, self.master_acks_reads()));
                return byte;
            }
        }

        let reg_val = self.read_reg(reg_offset);
        // Silicon clears RXNE when firmware reads DR; mark for next tick.
        if reg_offset == 0x10 && byte_offset == 0 && (self.sr1 & 0x40) != 0 {
            self.rxne_consumed.set(true);
        }
        ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
    }

    fn write(&mut self, offset: u64, value: u8) {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);
        let mask: u32 = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);
        self.write_reg(reg_offset, reg_val as u16);
    }

    fn peek(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        if byte_offset < 2 {
            ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
        } else {
            0
        }
    }

    /// One tick of the transaction state machine. Returns whether an IRQ
    /// should be raised. Logic relocated verbatim from the pre-split model.
    fn tick(&mut self) -> bool {
        let mut irq = false;

        // "RXNE clears on DR read" mirror, fires even when Idle.
        if self.rxne_consumed.replace(false) {
            self.sr1 &= !0x0040;
            self.sr1 &= !0x0004; // BTF tied to the same shift register
            if self.is_reading && self.current_target.is_some() {
                self.state = I2cState::DataPending;
                self.cycles_remaining = 1;
            }
        }

        if self.state != I2cState::Idle {
            self.cycles_remaining = self.cycles_remaining.saturating_sub(1);
            if self.cycles_remaining == 0 {
                match self.state {
                    I2cState::StartPending => {
                        self.sr1 = 0x0001; // Only SB set
                        self.cr1 &= !0x0100; // auto-clear START request
                        self.state = I2cState::Idle;
                    }
                    I2cState::AddressPending => {
                        self.sr1 &= !0x0001; // Clear SB

                        // The START condition, the seven address bits and the
                        // ACK slot were real wire activity: record the frame
                        // with the verdict this phase just resolved. A missing
                        // slave NACKs, which is exactly what an analyzer should
                        // show. `self.dr` still holds the address byte the DR
                        // write latched (address | R/W), which is the byte the
                        // controller clocked out.
                        self.wire_record(WireEvent::Frame(
                            self.dr as u8,
                            self.current_target.is_some(),
                        ));

                        // No slave at this address → NACK (SR1.AF), bus stays
                        // master+BUSY until firmware STOPs (matches F407 silicon).
                        if self.current_target.is_none() {
                            self.sr1 |= 0x0400; // AF
                            self.sr2 |= 0x0001; // MSL
                            self.sr2 |= 0x0002; // BUSY
                                                // TRA is NOT set on a NACKed address: TRA latches the
                                                // transmitter/receiver direction only "at the end of
                                                // the address phase" (RM0090 §27.6.7), which requires
                                                // an ACK (ADDR event). When the address is NACKed
                                                // (AF, no ADDR) the direction is never latched — real
                                                // NUCLEO-F407 silicon reads SR2=0x03 (MSL|BUSY) here,
                                                // not 0x07. Leave TRA at its reset/STOP-cleared 0.
                            self.state = I2cState::Idle;
                            if (self.cr2 & (1 << 8)) != 0 {
                                irq = true; // ITERR
                            }
                            return irq || self.irq_level();
                        }

                        self.sr1 |= 0x0002; // ADDR
                        self.sr2 |= 0x0001; // MSL
                        self.sr2 |= 0x0002; // BUSY
                                            // SR2.TRA (bit2): set in master transmitter after address
                                            // ACK with R/W=0. HAL_I2C_EV_IRQHandler gates the TXE/BTF
                                            // path on TRA — without it the ISR never writes data.
                        if self.is_reading {
                            self.sr2 &= !0x0004;
                        } else {
                            self.sr2 |= 0x0004;
                        }
                        // Fresh ADDR — cancel any prior software clear.
                        self.addr_cleared.set(false);
                        self.addr_sr1_seen.set(false);

                        if self.is_reading {
                            self.state = I2cState::DataPending;
                            self.cycles_remaining = 0;
                        } else {
                            self.sr1 |= 0x0080; // TXE
                            self.state = I2cState::Idle;
                        }
                    }
                    // Only the L4-generation controller charges data-phase wire
                    // time; the F1 model completes a byte in its DataPending
                    // arm below.
                    I2cState::DataSending => {}
                    I2cState::DataPending => {
                        if self.is_reading {
                            self.sr1 |= 0x0040; // RXNE
                            if let Some(idx) = self.current_target {
                                self.dr = self.attached_devices[idx].borrow_mut().read() as u32;
                                self.read_dr_consumed.set(false);
                                // The byte was clocked in off the wire here.
                                self.wire_record(WireEvent::Frame(
                                    self.dr as u8,
                                    self.master_acks_reads(),
                                ));
                            }
                            self.state = I2cState::Idle;
                        } else {
                            self.sr1 |= 0x0080; // TXE
                            self.sr1 |= 0x0004; // BTF
                            self.state = I2cState::Idle;
                        }
                        if self.stop_requested {
                            self.stop_requested = false;
                            self.cr1 &= !0x0200;
                            // Deferred STOP: same transaction boundary as the
                            // synchronous path in `write_reg`, so the wire is
                            // published here too. Missing this arm would leave
                            // every HAL "NACK+STOP → poll RXNE → read DR"
                            // receive silently unnarrated.
                            self.wire_flush();
                            self.sr2 &= !0x0007; // MSL|BUSY|TRA
                                                 // Keep RXNE (0x40): a deferred STOP on a master
                                                 // receive tears the bus down only after the byte has
                                                 // latched into DR; the firmware still has to read it.
                            self.sr1 &= !0x0087; // TXE|BTF|ADDR|SB
                            self.addr_cleared.set(false);
                            self.addr_sr1_seen.set(false);
                            if let Some(idx) = self.current_target {
                                self.attached_devices[idx].borrow_mut().stop();
                            }
                            self.current_target = None;
                        }
                    }
                    I2cState::Idle => {}
                }

                if self.irq_level() {
                    irq = true;
                }
            }
        }

        // Level re-assert every tick while enabled flags remain (see irq_level).
        irq || self.irq_level()
    }
}

// ── STM32L4 modern I2C (register-fidelity latching + minimal master engine) ──
#[derive(serde::Serialize)]
pub struct L4I2c {
    cr1: u32,
    cr2: u32,
    oar1: u32,
    oar2: u32,
    timingr: u32,
    timeoutr: u32,
    isr: u32,
    icr: u32,
    pecr: u32,
    rxdr: u32,
    txdr: u32,

    // Minimal master transaction engine (mirrors F1I2c, modern-register flavour).
    state: I2cState,
    cycles_remaining: u32,
    /// Latched CR2.NBYTES for the armed/in-flight transfer (0 = address-only).
    nbytes: u8,
    /// True once the first TXDR byte has been accepted for a multi-byte write.
    first_tx_loaded: bool,

    #[serde(skip)]
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    /// Index of the addressed slave for the armed/in-flight transfer (None when
    /// no attached device matches SADD — the tier-1 no-device case).
    #[serde(skip)]
    current_target: Option<usize>,
    #[serde(skip)]
    is_reading: bool,
    #[serde(skip)]
    autoend: bool,
    /// A byte was written to TXDR before the address phase (the L0/L4/G4 HAL
    /// preload ordering). TXDR is a real writable holding register: the byte
    /// waits in `txdr` and is transmitted once the address phase ACKs. Cleared
    /// when the START handler folds it into `first_tx_loaded` for the transfer.
    #[serde(skip)]
    tx_preloaded: bool,

    /// Bus-published cycle clock (walk-free campaign) — see `L4I2c::scheduler_mode`.
    #[serde(skip)]
    clock: Option<CycleClock>,
    /// Scheduler mode: `true` while the per-cycle engine event is live.
    #[serde(skip)]
    chain_live: bool,

    /// Wire levels published to AF-routed SCL/SDA pads, so a logic analyzer
    /// clipped to this bus measures a real waveform instead of a flat line.
    /// Created lazily by [`Self::pad_lines_arc`] at bus wiring time; `None`
    /// when no GPIO port routes this controller's pads.
    #[serde(skip)]
    lines: Option<std::sync::Arc<PadLines>>,
    /// Frames of the transaction in flight, `(byte, acked)`, oldest first —
    /// buffered so the whole transfer is narrated onto the pads as ONE
    /// contiguous waveform when it completes. See [`Self::wire_flush`].
    #[serde(skip)]
    wire_frames: Vec<(u8, bool)>,
}

impl Default for L4I2c {
    fn default() -> Self {
        Self {
            cr1: 0,
            cr2: 0,
            oar1: 0,
            oar2: 0,
            timingr: 0,
            timeoutr: 0,
            isr: 0x0000_0001, // TXE=1 at reset
            icr: 0,
            pecr: 0,
            rxdr: 0,
            txdr: 0,
            state: I2cState::Idle,
            cycles_remaining: 0,
            nbytes: 0,
            first_tx_loaded: false,
            attached_devices: Vec::new(),
            current_target: None,
            is_reading: false,
            autoend: false,
            tx_preloaded: false,
            clock: None,
            chain_live: false,
            lines: None,
            wire_frames: Vec::new(),
        }
    }
}

/// Line order for this controller's [`PadLines`]; the AF pad table routes
/// SCL/SDA pads to these indices.
pub(crate) const I2C_LINES: &[&str] = &["SCL", "SDA"];
pub(crate) const LINE_SCL: usize = 0;
pub(crate) const LINE_SDA: usize = 1;

impl L4I2c {
    /// The shared pad-line cell for this controller, created on first use.
    /// Called at bus wiring time by `wire_stm32_i2c_pads`; an open-drain bus
    /// with pull-ups idles high on both lines.
    pub(crate) fn pad_lines_arc(&mut self) -> std::sync::Arc<PadLines> {
        self.lines
            .get_or_insert_with(|| std::sync::Arc::new(PadLines::new(I2C_LINES, &[true, true])))
            .clone()
    }

    /// The wire this controller publishes, for a
    /// [`LogicSource::Wire`](crate::logic_capture::LogicSource::Wire) channel.
    /// `None` until a lab wires its pads, because that is when the cell is
    /// created — the controller has nothing to show before then either.
    pub(crate) fn wire_lines(&self) -> Option<&PadLines> {
        self.lines.as_deref()
    }

    /// Engine cycles in one SCL period, from TIMINGR — exactly the derivation
    /// [`Self::address_phase_cycles`] uses, of which it takes nine.
    fn bit_time_cycles(&self) -> u64 {
        u64::from(self.address_phase_cycles() / 9).max(2)
    }

    /// Record a frame this transfer put on the wire. Buffered, not published:
    /// see [`Self::wire_flush`].
    fn wire_push(&mut self, byte: u8, acked: bool) {
        if self.lines.is_some() {
            self.wire_frames.push((byte, acked));
        }
    }

    /// Publish the completed transaction's waveform onto the routed pads.
    ///
    /// The phase model has already exchanged the bytes; this narrates the wire
    /// activity they imply so the bus is measurable (see
    /// [`crate::peripherals::i2c_waveform`] for what that does and does not
    /// model). No routed pads → nothing to publish, and the call costs one
    /// branch.
    ///
    /// The whole transfer is emitted at once, ending at the present cycle,
    /// rather than frame by frame as each byte moves. That is forced by the
    /// timing model and is the honest arrangement: this controller charges wire
    /// time for the address phase only — a data byte crosses in zero modelled
    /// cycles — so there is no room on the timeline to place each frame where
    /// it "happened". Emitting the transaction as one contiguous run ending at
    /// completion gives a waveform with the right shape, the right bit rate and
    /// the right contents, positioned at the moment the transfer finished.
    /// Narrating frame by frame instead would stamp later frames in the future,
    /// where the capture layer collapses them onto a single cycle — a spike
    /// where a transaction belongs.
    fn wire_flush(&mut self) {
        let Some(lines) = self.lines.clone() else {
            self.wire_frames.clear();
            return;
        };
        if self.wire_frames.is_empty() {
            return;
        }
        let mut wave = I2cNarrator::new(LINE_SCL, LINE_SDA, self.bit_time_cycles());
        wave.start();
        for &(byte, acked) in &self.wire_frames {
            wave.frame(byte, acked);
        }
        wave.stop();
        self.wire_frames.clear();
        let now = lines.tap_clock().unwrap_or(0);
        // A transfer this early in a run has less history behind it than the
        // waveform needs; the narrator compresses to fit rather than emitting a
        // spike, and says so. Nothing here can act on that — the trace still
        // decodes to the right bytes — so the verdict is deliberately dropped.
        let _fit = wave.emit_ending_at(&lines, now);
    }

    /// The address byte on the wire for the armed transfer: SADD[7:1] shifted
    /// up with the direction bit, exactly as the controller clocks it out.
    fn address_byte(&self) -> u8 {
        let addr = ((self.cr2 >> 1) & 0x7F) as u8;
        (addr << 1) | u8::from(self.is_reading)
    }
    /// True when the event scheduler owns this controller's engine.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Clearing CR1.PE puts the transaction engine and every status bit back to
    /// their reset values, exactly as silicon does (RM0367 §26.7.1 / RM0351
    /// §39.7.1: "When PE=0 ... internal state machines and status bits are put
    /// back to their reset value"). ISR returns to 0x1 (TXE), so BUSY reads 0
    /// while the peripheral is disabled.
    ///
    /// The model used to store CR1 and nothing else, which leaked a whole
    /// transfer's worth of state through a disable. That is the standard HAL
    /// recovery path — `HAL_I2C_Master_Transmit` NACKs an absent slave, and
    /// with AUTOEND=0 nothing clears BUSY, so firmware toggles PE to reset the
    /// block. On the NUCLEO-L073RZ demo (no I²C device on the board, so every
    /// probe NACKs) that left BUSY latched for the rest of the run.
    ///
    /// The cost was not just a wrong register read. `active()` is true while
    /// BUSY is set, so the scheduler chain (`on_event` → `reschedule_delay: 1`)
    /// re-armed one cycle ahead forever, and `plan_cpu_window`'s scheduler
    /// deadline clamp pinned the CPU quantum to a single instruction for the
    /// life of the machine — the board ran at 1.00 steps/batch while its
    /// siblings batched 512 (#835).
    ///
    /// CONFIG registers are deliberately untouched. The RM resets the state
    /// machine and status bits; OAR1/OAR2/TIMINGR/TIMEOUTR/CR2 are control
    /// state that survives a disable, and firmware re-arms CR2 before the next
    /// START anyway. A stale CR2 cannot re-fire on its own — this model acts on
    /// START only at the instant of the CR2 write.
    fn disable_reset(&mut self) {
        self.isr = 0x0000_0001; // TXE=1, BUSY clear — the reset value
        self.icr = 0;
        self.state = I2cState::Idle;
        self.cycles_remaining = 0;
        self.nbytes = 0;
        self.first_tx_loaded = false;
        self.is_reading = false;
        self.autoend = false;
        self.tx_preloaded = false;
        if let Some(idx) = self.current_target.take() {
            // Release the addressed slave: silicon drops SCL/SDA, so a device
            // left mid-transfer must see the bus go away rather than stay
            // selected into whatever the next transfer addresses.
            self.attached_devices[idx].borrow_mut().stop();
        }
    }

    /// Engine ticks the START + address + ACK phase occupies the bus before
    /// CR2.START self-clears and the ACK/NACK verdict lands. Real silicon
    /// (RM0351 §37.7.5): after software sets START the controller drives the
    /// Start condition, the 7-bit address and the ACK slot — nine SCL bit-times
    /// — and only THEN clears START. Firmware that reads CR2/ISR in the few
    /// instructions after arming a transfer must still see START set and no
    /// NACKF (silicon-pinned: CR2=0x000120A0, ISR=0x00008001 on NUCLEO-L476RG),
    /// which a zero-time completion would violate.
    ///
    /// Derive the SCL bit-time from TIMINGR exactly as the hardware does —
    /// (SCLL+1)+(SCLH+1) prescaled by (PRESC+1) I2CCLK periods (RM0351 §37.7.5) —
    /// and take nine of them (START + 8 address bits + ACK).
    ///
    /// The countdown decrements once per engine tick, i.e. once per CORE cycle;
    /// the I2C kernel clock runs SLOWER than the core on these parts (the L0/L4/
    /// G4 boot raises the core via PLL but leaves I2C1SEL on its lower-rate reset
    /// source, e.g. HSI16), so a kernel period spans several core cycles. The
    /// live ratio is not visible on the walk path (no `CycleClock` attached), so
    /// scale by `CORE_PER_KCLK`, the reset-default core-to-kernel multiple. Only
    /// the magnitude matters and it is deterministic: it must exceed the handful
    /// of instructions before firmware re-reads CR2/ISR (on the L476 survival
    /// fixture that gap is a UART status print, ~21.5k core cycles, and the
    /// silicon capture pins START still set + no NACKF there), while staying far
    /// under the HAL transfer timeout so a real transfer still completes. The
    /// walk and the scheduler both decrement one step per cycle, so the timing
    /// is byte-identical in each. The floor keeps the pending window observable
    /// when TIMINGR is left at reset (e.g. bare unit tests).
    fn address_phase_cycles(&self) -> u32 {
        let presc = ((self.timingr >> 28) & 0xF) + 1;
        let scll = (self.timingr & 0xFF) + 1;
        let sclh = ((self.timingr >> 8) & 0xFF) + 1;
        let bit_time_kclk = (scll + sclh) * presc;
        const CORE_PER_KCLK: u32 = 8;
        (bit_time_kclk * 9 * CORE_PER_KCLK).max(64)
    }

    /// Cycles on which the legacy `tick()` does observable work: an in-flight
    /// countdown, the BUSY master-transfer window, or a live enabled IRQ flag
    /// (TXIS/TC/STOPF/NACKF) that Master_Transmit_IT still needs delivered.
    #[inline]
    fn active(&self) -> bool {
        if self.state != I2cState::Idle || (self.isr & (1 << 15)) != 0 {
            return true;
        }
        // Level IRQ bits that can still need a walk tick after the engine idles.
        let pending = self.isr
            & ((((self.cr1 & (1 << 1)) != 0) as u32 * (1 << 1)) // TXIE→TXIS
                | (((self.cr1 & (1 << 2)) != 0) as u32 * (1 << 2)) // RXIE→RXNE
                | (((self.cr1 & (1 << 4)) != 0) as u32 * (1 << 4)) // NACKIE
                | (((self.cr1 & (1 << 5)) != 0) as u32 * (1 << 5)) // STOPIE
                | (((self.cr1 & (1 << 6)) != 0) as u32 * (1 << 6))); // TCIE
        pending != 0
    }

    /// Level-triggered EV IRQ: any enabled status flag still latched.
    #[inline]
    fn irq_level(&self) -> bool {
        let cr1 = self.cr1;
        let isr = self.isr;
        ((cr1 & (1 << 1)) != 0 && (isr & (1 << 1)) != 0) // TXIE & TXIS
            || ((cr1 & (1 << 2)) != 0 && (isr & (1 << 2)) != 0) // RXIE & RXNE
            || ((cr1 & (1 << 4)) != 0 && (isr & (1 << 4)) != 0) // NACKIE & NACKF
            || ((cr1 & (1 << 5)) != 0 && (isr & (1 << 5)) != 0) // STOPIE & STOPF
            || ((cr1 & (1 << 6)) != 0 && (isr & (1 << 6)) != 0) // TCIE & TC
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.oar1,
            0x0C => self.oar2,
            0x10 => self.timingr,
            0x14 => self.timeoutr,
            0x18 => self.isr,
            0x1C => self.icr,
            0x20 => self.pecr,
            0x24 => self.rxdr,
            0x28 => self.txdr,
            _ => {
                crate::census_reg!("i2c:L4I2c", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                let was_enabled = (self.cr1 & 1) != 0;
                self.cr1 = value & 0x00FF_E1FF;
                if was_enabled && (self.cr1 & 1) == 0 {
                    self.disable_reset();
                }
            }
            0x04 => {
                // START (bit13) / STOP (bit14) self-clear in silicon — but only
                // once the corresponding condition is actually generated on the
                // bus, NOT at the instant of the CR2 write. Firmware that polls
                // CR2 immediately after arming a transfer reads START still set
                // (real NUCLEO-L476RG: CR2=0x000120A0 with START high in the
                // "start pending" window). So store CR2 verbatim here and clear
                // each trigger only when it is consumed: START when the address
                // phase runs, STOP in the STOP handler below. By then any Zephyr
                // LL_I2C_SetTransferSize RMW happens with START already cleared,
                // so the RMW cannot re-fire it.
                self.cr2 = value;
                if (value & (1 << 13)) != 0 {
                    // START: latch BUSY and run the addressed transfer. Capture
                    // the addressed slave (SADD[7:1] in 7-bit mode), direction
                    // (RD_WRN), NBYTES and AUTOEND.
                    if (self.cr1 & 1) != 0 {
                        self.isr |= 1 << 15; // BUSY
                        let addr = ((value >> 1) & 0x7F) as u8;
                        self.is_reading = (value & (1 << 10)) != 0; // RD_WRN
                        self.autoend = (value & (1 << 25)) != 0;
                        self.nbytes = ((value >> 16) & 0xFF) as u8;
                        // Carry a pre-START TXDR write (the L0/L4/G4 HAL preload
                        // ordering) into this transfer as the first data byte;
                        // the byte already sits in self.txdr.
                        self.first_tx_loaded = self.tx_preloaded;
                        self.tx_preloaded = false;
                        self.current_target = self
                            .attached_devices
                            .iter()
                            .position(|d| d.borrow().claims_address(addr));
                        if let Some(idx) = self.current_target {
                            let mut dev = self.attached_devices[idx].borrow_mut();
                            dev.select_address(addr);
                            dev.start();
                        }
                        // Real silicon begins the addressed transfer the instant
                        // START is set — for reads AND writes — but the Start
                        // condition + address + ACK take wire time before START
                        // self-clears and the verdict lands. Enter AddressPending
                        // with a TIMINGR-derived countdown; START stays readable
                        // in CR2 and NACKF stays clear until tick() drains it (see
                        // `address_phase_cycles`). When the countdown completes,
                        // tick() clears START and resolves ACK→(preloaded byte
                        // transmits / TXIS asserts + DataPending) or NACK→NACKF
                        // (+STOPF with AUTOEND). START is NOT cleared here.
                        self.state = I2cState::AddressPending;
                        self.cycles_remaining = self.address_phase_cycles();
                    }
                }
                if (value & (1 << 14)) != 0 {
                    // STOP (software, AUTOEND=0 path — Zephyr stm32 v2 poll):
                    // silicon sets STOPF and clears BUSY when the stop is done.
                    self.cr2 &= !(1 << 14); // STOP consumed
                    self.wire_flush();
                    self.isr |= 1 << 5; // STOPF
                    self.isr &= !(1 << 15); // clear BUSY
                    if let Some(idx) = self.current_target {
                        self.attached_devices[idx].borrow_mut().stop();
                    }
                    self.current_target = None;
                    self.state = I2cState::Idle;
                    self.nbytes = 0;
                    self.first_tx_loaded = false;
                    self.tx_preloaded = false;
                }
            }
            0x08 => self.oar1 = value,
            0x0C => self.oar2 = value,
            0x10 => self.timingr = value,
            0x14 => self.timeoutr = value,
            0x18 => {
                let rw_mask: u32 = 0x0000_0001; // TXE is RW
                self.isr = (self.isr & !rw_mask) | (value & rw_mask);
            }
            0x1C => {
                let clearable: u32 = 0x0000_3F38;
                self.isr &= !(value & clearable);
                self.icr = 0;
            }
            0x20 => self.pecr = value,
            0x24 => self.rxdr = value & 0xFF,
            0x28 => {
                self.txdr = value & 0xFF;
                self.isr &= !0x0000_0003; // writing TXDR clears TXE+TXIS
                if self.state == I2cState::DataPending {
                    // Post-TXIS path: the address phase already ACKed and asserted
                    // TXIS; firmware (HAL IT / poll) commits the data byte here.
                    self.first_tx_loaded = true;
                    if let Some(idx) = self.current_target {
                        self.attached_devices[idx]
                            .borrow_mut()
                            .write(self.txdr as u8);
                        // The byte goes out on the wire here, so the wire says so.
                        let byte = self.txdr as u8;
                        self.wire_push(byte, true);
                    }
                    // The byte now occupies the bus for nine SCL bit-times;
                    // TXE/TC/STOPF land when tick() drains that countdown.
                    self.isr |= 1 << 0; // TXE — TXDR is free again immediately
                    self.state = I2cState::DataSending;
                    self.cycles_remaining = self.address_phase_cycles();
                } else if self.state == I2cState::AddressPending {
                    // TXDR committed while the address phase is still on the wire
                    // (STM32Cube writes the first byte right after START, before
                    // the ACK): hold it as this transfer's first data byte — it
                    // transmits when the address ACKs (tick's first_tx_loaded
                    // path). The value is already stored in self.txdr above.
                    self.first_tx_loaded = true;
                } else {
                    // TXDR written before START (the L0/L4/G4 HAL preload
                    // ordering): TXDR is a writable holding register; the byte
                    // waits and is folded into the transfer as first_tx_loaded
                    // when START arms (see the CR2 START handler). The value is
                    // already stored in self.txdr above.
                    self.tx_preloaded = true;
                }
            }
            _ => {
                crate::census_reg!("i2c:L4I2c", offset, "write");
            }
        }
    }

    fn read(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
    }

    fn write(&mut self, offset: u64, value: u8) {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);
        let mask: u32 = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);
        self.write_reg(reg_offset, reg_val);
    }

    fn peek(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        if byte_offset < 2 {
            ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
        } else {
            0
        }
    }

    /// One tick of the minimal master transaction engine. Returns whether an
    /// IRQ should be raised. Structure mirrors `F1I2c::tick` but uses the modern
    /// ISR/ICR/CR2 register set (NACKF/STOPF/TC, START/STOP/AUTOEND in CR2).
    fn tick(&mut self) -> bool {
        let mut irq = false;
        if self.state == I2cState::Idle {
            // Still re-assert level IRQs while flags are latched (IT completion).
            return self.irq_level();
        }
        self.cycles_remaining = self.cycles_remaining.saturating_sub(1);
        if self.cycles_remaining != 0 {
            return self.irq_level();
        }
        if self.state == I2cState::DataSending {
            // The data byte has finished clocking out: TC (and, with AUTOEND,
            // the STOP) land now, and the transaction's waveform goes onto the
            // pads with its full wire time behind it.
            self.isr |= 1 << 6; // TC
            if self.autoend {
                self.isr |= 1 << 5; // STOPF
                self.isr &= !(1 << 15); // BUSY
                if let Some(i) = self.current_target {
                    self.attached_devices[i].borrow_mut().stop();
                }
                self.current_target = None;
                self.wire_flush();
            }
            self.state = I2cState::Idle;
            self.nbytes = 0;
            self.first_tx_loaded = false;
            if (self.cr1 & (1 << 6)) != 0 {
                irq = true; // TCIE
            }
            if (self.cr1 & (1 << 5)) != 0 && (self.isr & (1 << 5)) != 0 {
                irq = true; // STOPIE
            }
            return irq || self.irq_level();
        }
        if self.state == I2cState::AddressPending {
            // The Start condition + address + ACK have now been driven on the
            // bus (the countdown elapsed) → hardware clears CR2.START.
            self.cr2 &= !(1 << 13);
            // Those nine bit-times were real wire activity: publish them onto
            // the routed pads so the bus is measurable. A missing slave NACKs,
            // which is exactly what an analyzer should show.
            let acked = self.current_target.is_some();
            self.wire_push(self.address_byte(), acked);
            if !acked {
                // Nobody answered: the transaction is over, so the wire shows
                // the address frame NACKed and a STOP.
                self.wire_flush();
            }
            match self.current_target {
                None => {
                    // No slave ACKed the address → NACKF (matches L476 silicon:
                    // a write to an absent device sets ISR.NACKF, and AUTOEND
                    // auto-generates STOP, clearing BUSY and setting STOPF).
                    self.isr |= 1 << 4; // NACKF
                    self.isr &= !(1 << 1); // no further byte requested (TXIS off)
                    if self.autoend {
                        self.isr |= 1 << 5; // STOPF
                        self.isr &= !(1 << 15); // BUSY released
                    }
                    if (self.cr1 & (1 << 4)) != 0 {
                        irq = true; // NACKIE
                    }
                    if (self.cr1 & (1 << 5)) != 0 && (self.isr & (1 << 5)) != 0 {
                        irq = true; // STOPIE
                    }
                    self.state = I2cState::Idle;
                    self.nbytes = 0;
                    self.first_tx_loaded = false;
                }
                Some(idx) => {
                    // Slave ACKed.
                    if self.is_reading && self.nbytes > 0 {
                        self.rxdr = self.attached_devices[idx].borrow_mut().read() as u32;
                        // The master ACKs every byte but the last, which it
                        // NACKs to tell the slave to stop driving.
                        let more = self.nbytes > 1;
                        self.wire_push(self.rxdr as u8, more);
                        if self.autoend {
                            self.wire_flush();
                        }
                        self.isr |= 1 << 2; // RXNE
                        self.isr |= 1 << 6; // TC
                        if self.autoend {
                            self.isr |= 1 << 5; // STOPF
                            self.isr &= !(1 << 15);
                            self.attached_devices[idx].borrow_mut().stop();
                            self.current_target = None;
                        }
                        if (self.cr1 & (1 << 6)) != 0 {
                            irq = true; // TCIE
                        }
                        if (self.cr1 & (1 << 2)) != 0 {
                            irq = true; // RXIE
                        }
                        self.state = I2cState::Idle;
                        self.nbytes = 0;
                        self.first_tx_loaded = false;
                    } else if !self.is_reading && self.nbytes > 0 && self.first_tx_loaded {
                        // TXDR already loaded (legacy unit-test ordering).
                        self.attached_devices[idx]
                            .borrow_mut()
                            .write(self.txdr as u8);
                        self.wire_push(self.txdr as u8, true);
                        // Same as the post-TXIS path: the byte takes nine SCL
                        // bit-times before TC/STOPF land.
                        self.isr |= 1 << 0; // TXE
                        self.state = I2cState::DataSending;
                        self.cycles_remaining = self.address_phase_cycles();
                    } else if !self.is_reading && self.nbytes > 0 {
                        // Silicon order: address ACKed → TXIS requests first
                        // data byte. Stay in DataPending until TXDR is written
                        // (Arduino/Zephyr Master_Transmit_IT path).
                        self.isr |= 1 << 1; // TXIS
                        self.isr |= 1 << 0; // TXE
                        self.state = I2cState::DataPending;
                        if (self.cr1 & (1 << 1)) != 0 {
                            irq = true; // TXIE
                        }
                        // Keep nbytes / current_target / BUSY for the data phase.
                    } else {
                        // Address-only (NBYTES=0): TC without data path.
                        self.isr |= 1 << 0; // TXE
                        self.isr |= 1 << 6; // TC
                        if self.autoend {
                            self.isr |= 1 << 5; // STOPF
                            self.isr &= !(1 << 15);
                            self.attached_devices[idx].borrow_mut().stop();
                            self.current_target = None;
                        }
                        if (self.cr1 & (1 << 6)) != 0 {
                            irq = true; // TCIE
                        }
                        self.state = I2cState::Idle;
                        self.nbytes = 0;
                        self.first_tx_loaded = false;
                    }
                }
            }
        }
        irq || self.irq_level()
    }
}

// ── NXP Kinetis I2C (classic Freescale module: A1/F/C1/S/D/C2/FLT, byte-oriented,
//    interrupt-driven master) ──────────────────────────────────────────────────
//
// 1-byte registers: A1=0x00, F=0x01, C1=0x02, S=0x03, D=0x04, C2=0x05, FLT=0x06,
// RA=0x07, SMB=0x08, A2=0x09, SLTH=0x0A, SLTL=0x0B, S2=0x0C.
//   C1 bits: IICEN 0x80, IICIE 0x40, MST 0x20, TX 0x10, TXAK 0x08, RSTA 0x04.
//   S  bits: TCF 0x80, IAAS 0x40, BUSY 0x20, ARBL 0x10, SRW 0x04, IICIF 0x02, RXAK 0x01.
//
// The NXP fsl_i2c HAL drives each transfer byte-by-byte from the I2C ISR
// (I2C_MasterTransferHandleIRQ): START is C1.MST 0→1 then the slave address is
// written to D; a repeated START is C1.RSTA then the new address to D; entering
// master-receive clears C1.TX and the HAL dummy-reads D once to release the bus;
// STOP is C1.MST 1→0. Every byte the firmware moves through D "completes"
// synchronously here — we raise S.TCF|S.IICIF and set S.RXAK from whether a
// slave answered the address. The interrupt is LEVEL-driven: tick() asserts the
// IRQ while (S.IICIF & C1.IICIE), because the HAL enables IICIE only AFTER the
// opening address byte is already on the wire (I2C_MasterTransferNonBlocking),
// so an edge model would drop the first interrupt and hang the transfer.
const KI_C1_IICIE: u8 = 0x40;
const KI_C1_MST: u8 = 0x20;
const KI_C1_TX: u8 = 0x10;
const KI_C1_RSTA: u8 = 0x04;
const KI_S_TCF: u8 = 0x80;
const KI_S_BUSY: u8 = 0x20;
const KI_S_ARBL: u8 = 0x10;
const KI_S_IICIF: u8 = 0x02;
const KI_S_RXAK: u8 = 0x01;

#[derive(serde::Serialize)]
pub struct KinetisI2c {
    a1: u8,
    f: u8,
    c1: u8,
    s: Cell<u8>,
    d: Cell<u8>,
    c2: u8,
    flt: u8,
    ra: u8,
    smb: u8,
    a2: u8,
    slth: u8,
    sltl: u8,

    /// Next byte written to D is a slave address (after START / repeated START).
    expect_address: bool,
    /// Next read of D is the HAL bus-release dummy (return junk, no device byte).
    rx_dummy_pending: Cell<bool>,
    /// Current transfer is a master read (set from the address R/W bit).
    is_reading: bool,

    #[serde(skip)]
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    #[serde(skip)]
    current_target: Option<usize>,

    /// Bus-published cycle clock (walk-free plan Part 1). `Some` once the bus
    /// registration choke attaches it; `None` keeps the model on the legacy
    /// walk. Only the Kinetis variant migrates (see the `I2c` `Peripheral`
    /// impl): its `tick()` is a pure level-IRQ re-assertion, all byte/device
    /// work being synchronous in read/write, so the timer/systimer held-level
    /// re-pend event pattern reproduces it cycle-exactly.
    #[serde(skip)]
    clock: Option<CycleClock>,
    /// Scheduler mode only: `true` while the level-check event is live in the
    /// scheduler heap. Armed when IICIE becomes set; self-perpetuates at delay
    /// 1 while IICIE stays set (so a `&self` `D`-read that latches IICIF is
    /// caught the next cycle — exactly like the walk), stops when IICIE clears.
    #[serde(skip)]
    chain_live: bool,
}

impl Default for KinetisI2c {
    fn default() -> Self {
        Self {
            a1: 0,
            f: 0,
            c1: 0,
            // TCF=1 (idle, transfer complete), everything else clear (RM §49.3.4).
            s: Cell::new(KI_S_TCF),
            d: Cell::new(0),
            c2: 0,
            flt: 0,
            ra: 0,
            smb: 0,
            a2: 0,
            slth: 0,
            sltl: 0,
            expect_address: false,
            rx_dummy_pending: Cell::new(false),
            is_reading: false,
            attached_devices: Vec::new(),
            current_target: None,
            clock: None,
            chain_live: false,
        }
    }
}

impl KinetisI2c {
    /// True when the event scheduler owns this controller's level IRQ (feature
    /// on AND bus clock attached).
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// The level the legacy `tick()` re-asserts every cycle: IICIF latched AND
    /// IICIE enabled.
    #[inline]
    fn irq_level(&self) -> bool {
        (self.s.get() & KI_S_IICIF) != 0 && (self.c1 & KI_C1_IICIE) != 0
    }
    /// Mark a byte transfer complete: TCF + IICIF latch; RXAK mirrors the slave ack.
    fn byte_complete(&self, acked: bool) {
        let mut s = self.s.get() | KI_S_TCF | KI_S_IICIF;
        if acked {
            s &= !KI_S_RXAK;
        } else {
            s |= KI_S_RXAK;
        }
        self.s.set(s);
    }

    fn read_reg(&self, offset: u64) -> u8 {
        match offset {
            0x00 => self.a1,
            0x01 => self.f,
            0x02 => self.c1,
            0x03 => self.s.get(),
            0x04 => {
                // Bus-release dummy read after entering RX: HAL discards it.
                if self.rx_dummy_pending.replace(false) {
                    self.byte_complete(true);
                    return 0xFF;
                }
                if self.is_reading {
                    let byte = match self.current_target {
                        Some(idx) => self.attached_devices[idx].borrow_mut().read(),
                        None => 0xFF,
                    };
                    self.d.set(byte);
                    self.byte_complete(true);
                    return byte;
                }
                self.d.get()
            }
            0x05 => self.c2,
            0x06 => self.flt,
            0x07 => self.ra,
            0x08 => self.smb,
            0x09 => self.a2,
            0x0A => self.slth,
            0x0B => self.sltl,
            // S2: EMPTY=1 always (double-buffer TX FIFO empty) — the HAL polls
            // this before every D write on parts with double buffering.
            0x0C => 0x01,
            _ => {
                crate::census_reg!("i2c:KinetisI2c", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u8) {
        match offset {
            0x00 => self.a1 = value,
            0x01 => self.f = value,
            0x02 => {
                let old = self.c1;
                self.c1 = value;
                let mst_old = old & KI_C1_MST != 0;
                let mst_new = value & KI_C1_MST != 0;
                let tx_old = old & KI_C1_TX != 0;
                let tx_new = value & KI_C1_TX != 0;

                if !mst_old && mst_new {
                    // START: the next D write is the slave address.
                    self.expect_address = true;
                    self.s.set(self.s.get() | KI_S_BUSY);
                } else if mst_old && !mst_new {
                    // STOP. Keep current_target so a trailing last-byte D read
                    // (the HAL issues STOP just before reading it) still resolves.
                    if let Some(idx) = self.current_target {
                        self.attached_devices[idx].borrow_mut().stop();
                    }
                    self.s.set(self.s.get() & !KI_S_BUSY);
                }
                if value & KI_C1_RSTA != 0 && mst_new {
                    // Repeated START: next D write is a fresh address; RSTA self-clears.
                    self.expect_address = true;
                    self.c1 &= !KI_C1_RSTA;
                }
                if tx_old && !tx_new && mst_new {
                    // Entering master-receive: HAL dummy-reads D next to release the bus.
                    self.rx_dummy_pending.set(true);
                }
            }
            0x03 => {
                // S: IICIF and ARBL are write-1-to-clear.
                let mut s = self.s.get();
                if value & KI_S_IICIF != 0 {
                    s &= !KI_S_IICIF;
                }
                if value & KI_S_ARBL != 0 {
                    s &= !KI_S_ARBL;
                }
                self.s.set(s);
            }
            0x04 => {
                self.d.set(value);
                if self.expect_address {
                    let addr = value >> 1;
                    self.is_reading = (value & 1) != 0;
                    self.current_target = self
                        .attached_devices
                        .iter()
                        .position(|dev| dev.borrow().claims_address(addr));
                    if let Some(idx) = self.current_target {
                        {
                            let mut dev = self.attached_devices[idx].borrow_mut();
                            dev.select_address(addr);
                            dev.start();
                        }
                        self.byte_complete(true);
                    } else {
                        self.byte_complete(false); // address NAK
                    }
                    self.expect_address = false;
                } else {
                    if let Some(idx) = self.current_target {
                        self.attached_devices[idx].borrow_mut().write(value);
                    }
                    self.byte_complete(true);
                }
            }
            0x05 => self.c2 = value,
            0x06 => self.flt = value,
            0x07 => self.ra = value,
            0x08 => self.smb = value,
            0x09 => self.a2 = value,
            0x0A => self.slth = value,
            0x0B => self.sltl = value,
            _ => {
                crate::census_reg!("i2c:KinetisI2c", offset, "write");
            }
        }
    }

    /// LEVEL interrupt: asserted while a byte is pending (IICIF) and IICIE is set.
    fn tick(&mut self) -> bool {
        (self.s.get() & KI_S_IICIF) != 0 && (self.c1 & KI_C1_IICIE) != 0
    }
}

// ── Silicon Labs EFR32 Series-2 I²C ────────────────────────────────────────

/// EFR32/EFM32 I²C master.
///
/// # Sources
///
/// Offsets walked from `I2C_TypeDef` in `efr32mg26_i2c.h` (`simplicity_sdk`
/// tag `sisdk-2025.6`) — `IPVERSION_SET` lands at exactly `+0x1000`, which is
/// the check that the walk is right. Bit positions are the
/// `_I2C_<REG>_<FIELD>_SHIFT` defines from the same header.
///
/// # The flow this models, which is `emlib`'s
///
/// `I2C_TransferInit` / `I2C_Transfer` drive the controller like this, and so
/// does every Gecko SDK driver above them:
///
/// 1. `EN.EN = 1`, `CLKDIV` for the bit rate, `CTRL.SLAVE = 0`.
/// 2. `CMD.START`, then `TXDATA = (addr << 1) | rw`.
/// 3. The slave's answer arrives as `IF.ACK` or `IF.NACK`.
/// 4. Writing: `TXDATA` per byte, each answered by `IF.ACK`/`IF.NACK`.
///    Reading: wait `IF.RXDATAV`, read `RXDATA`, then `CMD.ACK` for another
///    byte or `CMD.NACK` to end.
/// 5. `CMD.STOP`, answered by `IF.MSTOP`.
///
/// # Faithfully modelled
///
/// * `EN.EN` gating: a disabled controller accepts nothing and flags nothing.
/// * Address matching through [`I2cDevice::claims_address`], never
///   `address()` — a flat address compare is first-match and collapses four
///   identical sensors behind a mux into one.
/// * An address no attached device claims raises `IF.NACK`, not `IF.ACK`.
///   That is the whole point of having a bus: a sketch that talks to a sensor
///   nobody wired gets the silicon's answer.
/// * `STATUS.TXBL`/`TXC` (the transmit buffer is always ready here, since a
///   byte completes synchronously) and `STATUS.RXDATAV`, plus the matching
///   `IF` flags. `IF` is write-1-to-clear.
/// * `STATE.BUSY`/`MASTER`/`TRANSMITTER`/`NACKED`, which `I2C_TransferInit`
///   reads before starting.
/// * `CMD.ABORT` and `CMD.CLEARTX` return the controller to idle.
///
/// # Idealised — present, but not physical
///
/// * **A byte transfers instantly.** `CLKDIV` is stored and ignored, so a
///   transaction costs no simulated time and no SCL edges are published — this
///   controller does not appear on a logic analyzer yet.
/// * **No arbitration, no bus errors, no timeouts.** `ARBLOST`, `BUSERR`,
///   `BITO` and `CLTO` never fire; a multi-master bus is not modelled.
/// * **Master only.** `CTRL.SLAVE`, `SADDR` and `SADDRMASK` store; the
///   controller never answers as a slave, and `IF.ADDR`/`SSTOP` never fire.
/// * **No double buffering.** `RXDOUBLE`/`TXDOUBLE`/`RXDOUBLEP` read the
///   single-byte path, and `AUTOACK`/`AUTOSE`/`AUTOSN` are stored and ignored.
#[derive(serde::Serialize)]
pub struct Efr32s2I2c {
    en: u32,
    ctrl: u32,
    clkdiv: u32,
    saddr: u32,
    saddrmask: u32,
    iflag: Cell<u32>,
    ien: u32,

    /// A START has been issued and the next `TXDATA` write is the address.
    expect_address: bool,
    /// The transaction in flight is a master read.
    is_reading: bool,
    /// Byte waiting in RXDATA for firmware, if any.
    rx_byte: Cell<Option<u8>>,
    /// The controller holds the bus (START seen, no STOP yet).
    busy: bool,
    /// The last address was not claimed by any attached device.
    nacked: bool,
    /// A transfer has completed since reset — what `STATUS.TXC` reports. Out of
    /// reset none has, which is why STATUS reads 0x80 (TXBL alone) and not 0xC0.
    txc: bool,
    /// The controller has seen the bus reach a known-idle state — a STOP or an
    /// ABORT. FALSE out of reset, which is why `STATE` reads 0x1 (BUSY) on a
    /// chip nothing has driven yet, and why emlib's `I2C_Init` opens with an
    /// ABORT. Once set it stays set; BUSY then follows the transfer.
    bus_idle_known: bool,

    #[serde(skip)]
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    #[serde(skip)]
    current_target: Option<usize>,
}

impl Default for Efr32s2I2c {
    fn default() -> Self {
        Self {
            en: 0,
            ctrl: 0,
            clkdiv: 0,
            saddr: 0,
            saddrmask: 0,
            // ⚠️ `_I2C_IF_RESETVALUE` is 0, and a BRD2709A reads 0 over SWD at
            // reset-halt. This used to seed TXBL|TXC on the theory that the
            // buffer is empty and the last transfer completed — but IF is a
            // FLAG register, and a flag that was never raised is not pending.
            // STATUS is where "the buffer is free" lives, and that still reads
            // TXBL out of reset.
            iflag: Cell::new(0),
            txc: false,
            bus_idle_known: false,
            ien: 0,
            expect_address: false,
            is_reading: false,
            rx_byte: Cell::new(None),
            busy: false,
            nacked: false,
            attached_devices: Vec::new(),
            current_target: None,
        }
    }
}

// Register offsets, walked from `I2C_TypeDef`.
const EFR_I2C_IPVERSION: u64 = 0x00;
const EFR_I2C_EN: u64 = 0x04;
const EFR_I2C_CTRL: u64 = 0x08;
const EFR_I2C_CMD: u64 = 0x0C;
const EFR_I2C_STATE: u64 = 0x10;
const EFR_I2C_STATUS: u64 = 0x14;
const EFR_I2C_CLKDIV: u64 = 0x18;
const EFR_I2C_SADDR: u64 = 0x1C;
const EFR_I2C_SADDRMASK: u64 = 0x20;
const EFR_I2C_RXDATA: u64 = 0x24;
const EFR_I2C_RXDOUBLE: u64 = 0x28;
const EFR_I2C_RXDATAP: u64 = 0x2C;
const EFR_I2C_RXDOUBLEP: u64 = 0x30;
const EFR_I2C_TXDATA: u64 = 0x34;
const EFR_I2C_TXDOUBLE: u64 = 0x38;
const EFR_I2C_IF: u64 = 0x3C;
const EFR_I2C_IEN: u64 = 0x40;

const EFR_I2C_IPVERSION_RESET: u32 = 3;

// CMD bits.
const EFR_CMD_START: u32 = 1 << 0;
const EFR_CMD_STOP: u32 = 1 << 1;
const EFR_CMD_ACK: u32 = 1 << 2;
const EFR_CMD_NACK: u32 = 1 << 3;
const EFR_CMD_ABORT: u32 = 1 << 5;
const EFR_CMD_CLEARTX: u32 = 1 << 6;

// STATE bits.
const EFR_STATE_BUSY: u32 = 1 << 0;
const EFR_STATE_MASTER: u32 = 1 << 1;
const EFR_STATE_TRANSMITTER: u32 = 1 << 2;
const EFR_STATE_NACKED: u32 = 1 << 3;

// STATUS bits.
const EFR_STATUS_TXC: u32 = 1 << 6;
const EFR_STATUS_TXBL: u32 = 1 << 7;
const EFR_STATUS_RXDATAV: u32 = 1 << 8;

// IF bits.
const EFR_IF_START: u32 = 1 << 0;
const EFR_IF_RSTART: u32 = 1 << 1;
const EFR_IF_TXC: u32 = 1 << 3;
const EFR_IF_TXBL: u32 = 1 << 4;
const EFR_IF_RXDATAV: u32 = 1 << 5;
const EFR_IF_ACK: u32 = 1 << 6;
const EFR_IF_NACK: u32 = 1 << 7;
const EFR_IF_MSTOP: u32 = 1 << 8;

const EFR_EN_EN: u32 = 1 << 0;

impl Efr32s2I2c {
    pub fn new() -> Self {
        Self::default()
    }

    fn enabled(&self) -> bool {
        self.en & EFR_EN_EN != 0
    }

    fn set_if(&self, bits: u32) {
        self.iflag.set(self.iflag.get() | bits);
    }

    /// Resolve `addr` to an attached device index, through `claims_address`.
    fn resolve(&self, addr: u8) -> Option<usize> {
        self.attached_devices
            .iter()
            .position(|d| d.borrow().claims_address(addr))
    }

    /// A `TXDATA` write: the address byte after a START, or a data byte.
    fn tx(&mut self, byte: u8) {
        if !self.enabled() {
            return;
        }
        if self.expect_address {
            self.expect_address = false;
            let addr = byte >> 1;
            self.is_reading = byte & 1 != 0;
            self.current_target = self.resolve(addr);
            match self.current_target {
                Some(idx) => {
                    self.nacked = false;
                    self.attached_devices[idx].borrow_mut().start();
                    self.set_if(EFR_IF_ACK);
                    // A read transaction's first byte is fetched now, so
                    // RXDATAV is set by the time firmware polls it.
                    if self.is_reading {
                        let b = self.attached_devices[idx].borrow_mut().read();
                        self.rx_byte.set(Some(b));
                        self.set_if(EFR_IF_RXDATAV);
                    }
                }
                None => {
                    // Nobody on the bus answers to this address. NACK, exactly
                    // as the silicon does, so a sketch talking to a sensor that
                    // was never wired finds out.
                    self.nacked = true;
                    self.set_if(EFR_IF_NACK);
                }
            }
            self.txc = true;
            self.txc = true;
        self.set_if(EFR_IF_TXC | EFR_IF_TXBL);
            return;
        }
        match self.current_target {
            Some(idx) => {
                self.attached_devices[idx].borrow_mut().write(byte);
                self.set_if(EFR_IF_ACK);
            }
            None => self.set_if(EFR_IF_NACK),
        }
        self.txc = true;
        self.set_if(EFR_IF_TXC | EFR_IF_TXBL);
    }

    /// A `RXDATA` read: hand over the pending byte. Reading consumes it, so
    /// `RXDATAV` drops until the next `CMD.ACK` fetches another.
    fn rx(&self) -> u8 {
        let byte = self.rx_byte.take().unwrap_or(0);
        let f = self.iflag.get() & !EFR_IF_RXDATAV;
        self.iflag.set(f);
        byte
    }

    fn apply_cmd(&mut self, value: u32) {
        if !self.enabled() {
            return;
        }
        if value & EFR_CMD_START != 0 {
            // A START while the bus is already held is a REPEATED start, which
            // firmware uses between a register write and the read of it.
            self.set_if(if self.busy {
                EFR_IF_RSTART
            } else {
                EFR_IF_START
            });
            self.busy = true;
            self.expect_address = true;
        }
        if value & EFR_CMD_ACK != 0 {
            // Master ACK: take another byte from the slave.
            if let Some(idx) = self.current_target {
                if self.is_reading {
                    let b = self.attached_devices[idx].borrow_mut().read();
                    self.rx_byte.set(Some(b));
                    self.set_if(EFR_IF_RXDATAV);
                }
            }
        }
        if value & EFR_CMD_NACK != 0 {
            // Master NACK ends a read; no further byte is fetched.
            self.rx_byte.set(None);
        }
        if value & EFR_CMD_CLEARTX != 0 {
            self.txc = true;
            self.set_if(EFR_IF_TXBL | EFR_IF_TXC);
        }
        if value & (EFR_CMD_STOP | EFR_CMD_ABORT) != 0 {
            // Either one tells the controller where the bus is, which is what
            // clears the power-on BUSY. emlib issues ABORT for exactly this.
            self.bus_idle_known = true;
            if let Some(idx) = self.current_target {
                self.attached_devices[idx].borrow_mut().stop();
            }
            self.busy = false;
            self.expect_address = false;
            self.is_reading = false;
            self.current_target = None;
            self.rx_byte.set(None);
            if value & EFR_CMD_STOP != 0 {
                self.set_if(EFR_IF_MSTOP);
            }
        }
    }

    fn state_word(&self) -> u32 {
        // ⚠️ BUSY is set OUT OF RESET, and stays set until firmware issues the
        // ABORT that emlib's `I2C_Init` always sends. `_I2C_STATE_RESETVALUE`
        // is 0x00000001 (BUSY alone) and a BRD2709A reads exactly that over SWD
        // at reset-halt. It reads as a quirk and is not one: the controller
        // cannot know the bus is idle until something drives it, so it comes up
        // claiming the bus.
        //
        // This model previously returned 0 at reset and set TRANSMITTER
        // unconditionally whenever it was not reading, which made a freshly
        // reset controller read 0x4 — neither the header's value nor the die's.
        let mut s = 0;
        if self.busy || !self.bus_idle_known {
            s |= EFR_STATE_BUSY;
        }
        if self.busy {
            s |= EFR_STATE_MASTER;
            if !self.is_reading {
                s |= EFR_STATE_TRANSMITTER;
            }
        }
        if self.nacked {
            s |= EFR_STATE_NACKED;
        }
        s
    }

    fn status_word(&self) -> u32 {
        // TXBL is always set: a byte completes synchronously here, so the
        // transmit buffer is never occupied.
        //
        // ⚠️ TXC is NOT. This used to set both and call it "the idealisation the
        // header lists" — the header lists `_I2C_STATUS_RESETVALUE 0x00000080`,
        // which is TXBL (bit 7) ALONE, and a BRD2709A reads 0x80 over SWD at
        // reset-halt. TXC means a transfer has completed; out of reset none has.
        let mut s = EFR_STATUS_TXBL;
        if self.txc {
            s |= EFR_STATUS_TXC;
        }
        if self.rx_byte.get().is_some() {
            s |= EFR_STATUS_RXDATAV;
        }
        s
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            EFR_I2C_IPVERSION => EFR_I2C_IPVERSION_RESET,
            EFR_I2C_EN => self.en,
            EFR_I2C_CTRL => self.ctrl,
            EFR_I2C_CMD => 0, // write-only
            EFR_I2C_STATE => self.state_word(),
            EFR_I2C_STATUS => self.status_word(),
            EFR_I2C_CLKDIV => self.clkdiv,
            EFR_I2C_SADDR => self.saddr,
            EFR_I2C_SADDRMASK => self.saddrmask,
            EFR_I2C_RXDATA | EFR_I2C_RXDOUBLE => self.rx() as u32,
            // The PEEK registers read the buffer WITHOUT consuming it. A model
            // that aliased them onto RXDATA would drop a byte every time a
            // driver looked.
            EFR_I2C_RXDATAP | EFR_I2C_RXDOUBLEP => self.rx_byte.get().unwrap_or(0) as u32,
            EFR_I2C_IF => self.iflag.get(),
            EFR_I2C_IEN => self.ien,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            EFR_I2C_EN => {
                self.en = value & EFR_EN_EN;
                if !self.enabled() {
                    self.busy = false;
                    self.expect_address = false;
                    self.current_target = None;
                    self.rx_byte.set(None);
                }
            }
            EFR_I2C_CTRL => self.ctrl = value,
            EFR_I2C_CMD => self.apply_cmd(value),
            EFR_I2C_CLKDIV => self.clkdiv = value,
            EFR_I2C_SADDR => self.saddr = value,
            EFR_I2C_SADDRMASK => self.saddrmask = value,
            EFR_I2C_TXDATA | EFR_I2C_TXDOUBLE => self.tx((value & 0xFF) as u8),
            EFR_I2C_IF => self.iflag.set(self.iflag.get() & !value),
            EFR_I2C_IEN => self.ien = value,
            _ => {}
        }
    }

    fn irq_pending(&self) -> bool {
        self.iflag.get() & self.ien != 0
    }
}

/// I2C peripheral — one variant per chip family. Register sets fully isolated.
#[derive(serde::Serialize)]
pub enum I2c {
    Stm32F1(F1I2c),
    Stm32L4(L4I2c),
    Kinetis(KinetisI2c),
    Efr32s2(Efr32s2I2c),
}

impl core::fmt::Debug for I2c {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            I2c::Stm32F1(i) => f.debug_struct("I2c::F1").field("state", &i.state).finish(),
            I2c::Stm32L4(_) => f.debug_struct("I2c::L4").finish(),
            I2c::Kinetis(i) => f
                .debug_struct("I2c::Kinetis")
                .field("c1", &i.c1)
                .field("s", &i.s.get())
                .finish(),
            I2c::Efr32s2(i) => f
                .debug_struct("I2c::Efr32s2")
                .field("busy", &i.busy)
                .field("if", &i.iflag.get())
                .finish(),
        }
    }
}

impl Default for I2c {
    fn default() -> Self {
        Self::Stm32F1(F1I2c::default())
    }
}

impl I2c {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forward an ERROR-line NVIC vector to the variant that models one.
    /// Only the STM32 legacy peripheral splits EV/ER this way; other families
    /// carry a single vector and ignore this.
    pub fn set_error_irq(&mut self, irq: u32) {
        if let Self::Stm32F1(i) = self {
            i.set_error_irq(irq);
        }
    }

    /// The shared pad-line cell of a controller that publishes a wire, or
    /// `None` for a family that has no wire model yet.
    ///
    /// This is the seam the GPIO pad routing binds to; a family gains a
    /// measurable bus by returning `Some` here and narrating (or bit-banging)
    /// into that cell.
    pub(crate) fn pad_lines_arc(&mut self) -> Option<std::sync::Arc<PadLines>> {
        match self {
            Self::Stm32F1(i) => Some(i.pad_lines_arc()),
            Self::Stm32L4(i) => Some(i.pad_lines_arc()),
            Self::Kinetis(_) => None,
            // No wire model yet: a byte transfers instantly and no SCL edge is
            // published, so this controller cannot be probed.
            Self::Efr32s2(_) => None,
        }
    }

    /// Which register generation this instance models.
    ///
    /// Read by `wire_stm32_i2c_pads` to pick the alternate-function table: the
    /// legacy (F1/F2/F4) and modern (L4/F7/H5/G0) controllers do NOT share a
    /// pinout, and the difference is not cosmetic. On the STM32L476 (DS10198
    /// Table 17) PA7/AF4 is I2C3_SCL and PB4/AF4 is I2C3_SDA; on the STM32F401
    /// (DS10086 Rev 5 Table 9, pages 45-47) AF4 on both of those pads is
    /// UNASSIGNED — F401 puts I2C3 on PA8/AF4 (SCL) and PC9/AF4 (SDA), with
    /// PB4's I2C3_SDA living on AF9 instead. One shared table would publish an
    /// I²C waveform onto pads the silicon leaves empty.
    pub(crate) fn register_layout(&self) -> I2cRegisterLayout {
        match self {
            Self::Stm32F1(_) => I2cRegisterLayout::Stm32F1,
            Self::Stm32L4(_) => I2cRegisterLayout::Stm32L4,
            Self::Kinetis(_) => I2cRegisterLayout::Kinetis,
            Self::Efr32s2(_) => I2cRegisterLayout::Efr32s2,
        }
    }

    pub fn new_with_layout(layout: I2cRegisterLayout) -> Self {
        match layout {
            I2cRegisterLayout::Stm32F1 => Self::Stm32F1(F1I2c::default()),
            I2cRegisterLayout::Stm32L4 => Self::Stm32L4(L4I2c::default()),
            I2cRegisterLayout::Kinetis => Self::Kinetis(KinetisI2c::default()),
            I2cRegisterLayout::Efr32s2 => Self::Efr32s2(Efr32s2I2c::default()),
        }
    }

    /// Attach a slave to a bare (off-bus) controller, wrapping it into `trace`.
    /// The trace handle is mandatory, so there is no untraced attach — this is
    /// the off-bus counterpart of the on-bus choke point
    /// [`crate::bus::SystemBus::attach_i2c_slave`], and both funnel through the
    /// one wrap helper `bus_trace::wrap_i2c`. Used by standalone tests that
    /// drive an `I2c` directly (no `SystemBus`).
    pub fn attach_traced(
        &mut self,
        bus_name: &str,
        trace: &crate::bus::bus_trace::BusTrace,
        device: Box<dyn I2cDevice>,
    ) {
        self.push_slave(crate::bus::bus_trace::wrap_i2c(bus_name, trace, device));
    }

    /// Raw slave push — does NOT wrap for tracing. The only caller is the bus
    /// choke point [`crate::bus::SystemBus::attach_i2c_slave`], which wraps the
    /// device first; nothing else should attach directly (that would bypass the
    /// universal bus trace).
    pub(crate) fn push_slave(&mut self, device: Box<dyn I2cDevice>) {
        match self {
            Self::Stm32F1(i) => i.attached_devices.push(RefCell::new(device)),
            Self::Stm32L4(i) => i.attached_devices.push(RefCell::new(device)),
            Self::Kinetis(i) => i.attached_devices.push(RefCell::new(device)),
            Self::Efr32s2(i) => i.attached_devices.push(RefCell::new(device)),
        }
    }

    /// Drain every attached slave (AVR parks kits on a bus `i2c` controller,
    /// then moves them onto the CPU TWI model after `from_config`).
    pub fn take_slaves(&mut self) -> Vec<Box<dyn I2cDevice>> {
        let cells = match self {
            Self::Stm32F1(i) => std::mem::take(&mut i.attached_devices),
            Self::Stm32L4(i) => std::mem::take(&mut i.attached_devices),
            Self::Kinetis(i) => std::mem::take(&mut i.attached_devices),
            Self::Efr32s2(i) => std::mem::take(&mut i.attached_devices),
        };
        cells.into_iter().map(RefCell::into_inner).collect()
    }

    /// Attached I2C devices (used by config/bus validation + tests).
    pub fn attached_devices(&self) -> &[RefCell<Box<dyn I2cDevice>>] {
        match self {
            Self::Stm32F1(i) => &i.attached_devices,
            Self::Stm32L4(i) => &i.attached_devices,
            Self::Kinetis(i) => &i.attached_devices,
            Self::Efr32s2(i) => &i.attached_devices,
        }
    }

    /// True when the event scheduler owns this instance's IRQ delivery. All
    /// three variants migrate. Kinetis: its `tick()` is a pure level-IRQ
    /// re-assertion. STM32 F1/L4: their `cycles_remaining` transaction engine is
    /// self-paced by a delay-1 event chain that runs `tick()` every cycle while
    /// the transfer is *active* (see `F1I2c::active`) — the SAME held-level
    /// self-perpetuating pattern Kinetis uses. The `&self`-read side effects
    /// (`rxne_consumed` / device byte pulls) mutate `Cell`/`RefCell` state that
    /// the already-live chain's next `on_event` observes exactly as the walk's
    /// next `tick()` would, so no event needs arming from the read path. Idle
    /// fast-forward still engages: the chain stops the moment the transfer goes
    /// fully idle (BUSY clear, no countdown), which on a real lab is between
    /// transactions when the firmware is not busy-polling anyway.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        match self {
            Self::Stm32F1(i) => i.scheduler_mode(),
            Self::Stm32L4(i) => i.scheduler_mode(),
            Self::Kinetis(i) => i.scheduler_mode(),
            // The EFR32 model has no cycle-paced transaction engine: a byte
            // completes inside the register write, so there is nothing for the
            // scheduler to pace and it stays on the legacy walk.
            Self::Efr32s2(_) => false,
        }
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy walk path. Used by the walk-on-vs-scheduler differential gates to
    /// build the reference config from the same assembly.
    pub fn force_legacy_walk(&mut self) {
        match self {
            Self::Stm32F1(i) => i.clock = None,
            Self::Stm32L4(i) => i.clock = None,
            Self::Kinetis(i) => i.clock = None,
            // Never on the scheduler in the first place.
            Self::Efr32s2(_) => {}
        }
    }
}

impl crate::Peripheral for I2c {
    fn line_names(&self) -> &'static [&'static str] {
        match self {
            // The Kinetis I2C model owns no narration cell, so it publishes no
            // wire — an honest empty answer, not a guess at SCL/SDA.
            Self::Stm32F1(_) | Self::Stm32L4(_) => I2C_LINES,
            // Neither the Kinetis nor the EFR32 model owns a narration cell,
            // so neither publishes a wire — an honest empty answer, not a
            // guess at SCL/SDA.
            Self::Kinetis(_) | Self::Efr32s2(_) => &[],
        }
    }

    fn wire_lines(&self) -> Option<&PadLines> {
        match self {
            Self::Stm32F1(i) => i.wire_lines(),
            Self::Stm32L4(i) => i.wire_lines(),
            Self::Kinetis(_) => None,
            // No wire model yet: a byte transfers instantly and no SCL edge is
            // published, so this controller cannot be probed.
            Self::Efr32s2(_) => None,
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match self {
            Self::Stm32F1(i) => i.read(offset),
            Self::Stm32L4(i) => i.read(offset),
            Self::Kinetis(i) => i.read_reg(offset),
            Self::Efr32s2(i) => {
                let word = i.read_word(offset & !3);
                ((word >> ((offset % 4) * 8)) & 0xFF) as u8
            }
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match self {
            Self::Stm32F1(i) => i.write(offset, value),
            Self::Stm32L4(i) => i.write(offset, value),
            Self::Kinetis(i) => i.write_reg(offset, value),
            Self::Efr32s2(i) => {
                let reg = offset & !3;
                let shift = (offset % 4) * 8;
                let merged = (i.read_word(reg) & !(0xFFu32 << shift)) | ((value as u32) << shift);
                i.write_word(reg, merged);
            }
        }
        Ok(())
    }

    fn drives_central_i2c_time(&self) -> bool {
        true
    }

    /// Advance every attached slave's data-ready clock. Slaves live behind
    /// `RefCell` here (the transaction engine hands out interior-mutable borrows
    /// mid-transfer), so borrow each cell in turn. On STM32/Kinetis the machine
    /// only calls this when a `sim_time_us` source is present; those families
    /// model no absolute-µs counter today, so in practice this stays inert until
    /// one is added — the override is here so the fan-out is complete the moment
    /// it is.
    fn advance_attached_i2c_us(&mut self, us: u64) {
        if us == 0 {
            return;
        }
        for cell in self.attached_devices() {
            cell.borrow_mut().advance_time_us(us);
        }
    }

    /// Atomic word writes: STM32 HAL stores CR2 as a single STR (START, NBYTES,
    /// and AUTOEND together). Default Peripheral::write_u32 byte-slices and would
    /// assert START before AUTOEND lands, breaking the NBYTES=0 probe path.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match self {
            Self::Stm32F1(i) => {
                i.write_reg(offset & !1, (value & 0xFFFF) as u16);
            }
            Self::Stm32L4(i) => {
                i.write_reg(offset & !3, value);
            }
            Self::Kinetis(i) => {
                i.write_reg(offset, (value & 0xFF) as u8);
                i.write_reg(offset.wrapping_add(1), ((value >> 8) & 0xFF) as u8);
                i.write_reg(offset.wrapping_add(2), ((value >> 16) & 0xFF) as u8);
                i.write_reg(offset.wrapping_add(3), ((value >> 24) & 0xFF) as u8);
            }
            // Every EFR32 register is a 32-bit word and firmware writes it as
            // one. Byte-slicing CMD would apply START and STOP as separate
            // commands.
            Self::Efr32s2(i) => i.write_word(offset & !3, value),
        }
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        // Scheduler-mode instances are walk-skipped (the guard keeps a stray
        // direct call from double-advancing the engine the event chain owns).
        if self.scheduler_mode() {
            return crate::PeripheralTickResult::default();
        }
        let irq = match self {
            Self::Stm32F1(i) => i.tick(),
            Self::Stm32L4(i) => i.tick(),
            Self::Kinetis(i) => i.tick(),
            // A transaction completes inside the register write, so there is
            // nothing to advance here — only the level IRQ to re-assert.
            Self::Efr32s2(i) => i.irq_pending(),
        };
        // Errors ride the ER vector, not the EV vector the `irq` flag pends.
        let explicit_irqs = match self {
            Self::Stm32F1(i) => i.error_irqs(),
            _ => None,
        };
        crate::PeripheralTickResult {
            irq,
            cycles: 0,
            explicit_irqs,
            ..Default::default()
        }
    }

    fn uses_scheduler(&self) -> bool {
        // Any variant with a bus clock attached (event-scheduler builds). See
        // `I2c::scheduler_mode`.
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        // Scheduler-mode: the transaction engine (F1/L4) or level re-assertion
        // (Kinetis) is fully driven by the event chain, so the walk is
        // unnecessary. Feature off / no clock: real per-cycle walk work → `true`.
        !self.scheduler_mode()
    }

    fn sync_to(&mut self, _now_cycle: u64) {
        // No lazily-accumulated state to reconcile: the F1/L4 transaction
        // countdown is advanced cycle-by-cycle by the self-perpetuating event
        // chain (drained up to the current cycle by `Machine::step` before any
        // MMIO access observes it), and the Kinetis registers / device byte
        // stream / IICIF all mutate synchronously in read/write. Explicit no-op
        // for symmetry with the other scheduler-migrated models.
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        match self {
            // Never on the scheduler: nothing to arm.
            Self::Efr32s2(_) => Vec::new(),
            Self::Kinetis(i) => {
                if !i.scheduler_mode() {
                    return Vec::new();
                }
                // Arm the self-perpetuating level-check the moment interrupts are
                // armed (IICIE set) and no chain is live. The chain then re-polls
                // every cycle while IICIE stays set (delay-0 → deadline
                // `current_cycle + 1`, the cycle the legacy walk's next tick would
                // first check the level), so a `&self` `D`-read that latches IICIF
                // is picked up the next cycle, exactly as the walk would. The
                // `chain_live` guard prevents duplicate chains across the multiple
                // C1/D/S writes of a transfer.
                if (i.c1 & KI_C1_IICIE) != 0 && !i.chain_live {
                    i.chain_live = true;
                    vec![(0u64, 0u32)]
                } else {
                    Vec::new()
                }
            }
            // STM32 F1/L4: arm the per-cycle transaction-engine chain the moment
            // a write makes the transfer active (START/DR countdown, BUSY). The
            // chain then self-perpetuates every cycle while the transfer stays
            // active — including across the `&self` receive reads that cannot arm
            // an event themselves (their re-arm is caught by the already-live
            // chain's next `on_event`, exactly as the walk's next tick would).
            // delay-0 → deadline `current_cycle + 1` = the walk's next tick.
            Self::Stm32F1(i) => {
                if i.scheduler_mode() && i.active() && !i.chain_live {
                    i.chain_live = true;
                    vec![(0u64, 0u32)]
                } else {
                    Vec::new()
                }
            }
            Self::Stm32L4(i) => {
                if i.scheduler_mode() && i.active() && !i.chain_live {
                    i.chain_live = true;
                    vec![(0u64, 0u32)]
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let _ = sched;
        match self {
            // Never on the scheduler: no event can be delivered here.
            Self::Efr32s2(_) => crate::sched::EventResult::default(),
            Self::Kinetis(i) => {
                if !i.scheduler_mode() {
                    return crate::sched::EventResult::default();
                }
                // Pend the peripheral's own NVIC line while the level
                // (IICIF & IICIE) is asserted — the event-path equivalent of the
                // legacy `tick()` returning its level bool every cycle. Perpetuate
                // at delay 1 while IICIE stays set so a byte completion latched by
                // a `&self` read is caught the next cycle; stop when firmware
                // disables IICIE.
                let iicie = (i.c1 & KI_C1_IICIE) != 0;
                i.chain_live = iicie;
                crate::sched::EventResult {
                    raise_own_irq: i.irq_level(),
                    reschedule_delay: iicie.then_some(1),
                    ..Default::default()
                }
            }
            // STM32 F1: run one cycle of the transaction engine — byte-for-byte
            // the same `F1I2c::tick()` the walk runs — and pend the NVIC line on
            // its IRQ verdict. Re-check `active()` AFTER the tick (it may have
            // just delivered the last byte and cleared BUSY) and perpetuate at
            // delay 1 while still active; stop when fully idle so fast-forward can
            // engage. An extra idle cycle would be inert, so the tight stop is
            // safe. The `on_event` runs at the same per-cycle cadence as the walk,
            // so the countdown timing and IRQ edges are identical.
            Self::Stm32F1(i) => {
                if !i.scheduler_mode() {
                    return crate::sched::EventResult::default();
                }
                let irq = i.tick();
                let active = i.active();
                i.chain_live = active;
                crate::sched::EventResult {
                    raise_own_irq: irq,
                    reschedule_delay: active.then_some(1),
                    ..Default::default()
                }
            }
            Self::Stm32L4(i) => {
                if !i.scheduler_mode() {
                    return crate::sched::EventResult::default();
                }
                let irq = i.tick();
                let active = i.active();
                i.chain_live = active;
                crate::sched::EventResult {
                    raise_own_irq: irq,
                    reschedule_delay: active.then_some(1),
                    ..Default::default()
                }
            }
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        // All three variants opt into the scheduler once the bus attaches its
        // clock (event-scheduler builds); featureless builds ignore it via
        // `scheduler_mode`.
        match self {
            Self::Stm32F1(i) => i.clock = Some(clock),
            Self::Stm32L4(i) => i.clock = Some(clock),
            Self::Kinetis(i) => i.clock = Some(clock),
            // The EFR32 model completes a byte inside the register write, so
            // it has no cycle-paced engine to hand a clock to.
            Self::Efr32s2(_) => {}
        }
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(match self {
            Self::Stm32F1(i) => i.peek(offset),
            Self::Stm32L4(i) => i.peek(offset),
            // Kinetis registers are side-effect-free to read except D; peek D
            // without consuming a device byte.
            Self::Kinetis(i) => {
                if offset == 0x04 {
                    i.d.get()
                } else {
                    i.read_reg(offset)
                }
            }
            // ⚠️ `peek` is a side-effect-free probe. Going through the ordinary
            // read would CONSUME the RX byte, and an observer attached to the
            // bus would silently eat the firmware's data — the same trap the
            // IADC hit.
            Self::Efr32s2(i) => {
                let reg = offset & !3;
                let word = if reg == EFR_I2C_RXDATA || reg == EFR_I2C_RXDOUBLE {
                    i.rx_byte.get().unwrap_or(0) as u32
                } else {
                    i.read_word(reg)
                };
                ((word >> ((offset % 4) * 8)) & 0xFF) as u8
            }
        })
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    /// Slaves live behind `RefCell` here (the transaction engine hands out
    /// interior-mutable borrows mid-transfer), so the walk borrows each cell in
    /// turn rather than taking one long `&mut` over the vector.
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for cell in self.attached_devices() {
            // `for_each_sim_input`, not `as_sim_input_mut`: a container slave
            // (TCA9548A mux) exposes the inputs of the devices behind it, which
            // a single-surface accessor cannot represent.
            if cell.borrow_mut().for_each_sim_input(f) {
                return true;
            }
        }
        false
    }

    /// Slaves live behind `RefCell` here for the same reason
    /// [`Self::for_each_attached_sim_input`] borrows each cell in turn.
    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for cell in self.attached_devices() {
            crate::inspect::visit_i2c_device(&**cell.borrow(), f);
        }
    }

    /// Custom inspection: the generic register decode plus a `framebuffer`
    /// artifact for any attached panel. This is the pattern the ~10 bespoke
    /// `get_*_framebuffer` wasm accessors generalize into — the controller walks
    /// its own attached devices and emits panel artifacts, one code path instead
    /// of a bespoke accessor per panel. Summary mode omits the bytes and carries
    /// a cheap `generation` hash so callers skip unchanged buffers.
    ///
    /// What each panel's artifact CONTAINS is not decided here: it comes from
    /// the device model's own [`I2cDevice::artifacts`], the single emitter
    /// shared with the machine-level device walk, so a panel cannot report one
    /// thing on this controller and something else on another.
    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        let mut pi = crate::inspect::default_inspect(self, base, name, opts);
        pi.kind = "i2c".to_string();
        for dev_cell in self.attached_devices() {
            let dev = dev_cell.borrow();
            let addr = dev.address();
            pi.artifacts
                .extend(dev.artifacts(&format!("i2c@0x{:02x}", addr), opts));
        }
        pi
    }

    fn snapshot(&self) -> serde_json::Value {
        match self {
            Self::Stm32F1(i) => serde_json::to_value(i),
            Self::Stm32L4(i) => serde_json::to_value(i),
            Self::Kinetis(i) => serde_json::to_value(i),
            Self::Efr32s2(i) => serde_json::to_value(i),
        }
        .unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::{I2c, I2cDevice, KinetisI2c, KI_C1_MST, KI_C1_TX};
    use crate::Peripheral;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    /// The I2C controller's custom `inspect()` emits a `framebuffer` artifact
    /// for an attached SSD1306 OLED: metadata always present; the (large) byte
    /// payload only when `include_bytes` is requested. This is the pattern that
    /// generalizes the bespoke `get_*_framebuffer` accessors.
    #[test]
    fn inspect_emits_ssd1306_framebuffer_artifact() {
        use crate::inspect::InspectOpts;
        use crate::peripherals::components::Ssd1306;

        let mut i2c = I2c::new();
        i2c.push_slave(Box::new(Ssd1306::new(0x3C)));

        // Summary mode: metadata present, bytes omitted.
        let summary = i2c.inspect(0x4000_5400, "i2c1", &InspectOpts::default());
        assert_eq!(summary.kind, "i2c");
        let fb = summary
            .artifacts
            .iter()
            .find(|a| a.kind == "framebuffer")
            .expect("framebuffer artifact present");
        assert_eq!(fb.id, "i2c@0x3c");
        assert_eq!(fb.meta["w"], 128);
        assert_eq!(fb.meta["h"], 64);
        assert_eq!(fb.meta["format"], "ssd1306_page");
        assert!(
            fb.meta["generation"].is_u64(),
            "cheap change-detection hash"
        );
        assert!(fb.bytes.is_none(), "bytes omitted in summary mode");

        // include_bytes: full GDDRAM payload attached.
        let full = i2c.inspect(
            0x4000_5400,
            "i2c1",
            &InspectOpts {
                include_bytes: true,
                peripheral: None,
            },
        );
        let fb = full
            .artifacts
            .iter()
            .find(|a| a.kind == "framebuffer")
            .expect("framebuffer artifact present");
        assert_eq!(
            fb.bytes.as_ref().map(|b| b.len()),
            Some(128 * 8),
            "1024-byte page-major GDDRAM"
        );
    }

    struct CountingDevice {
        address: u8,
        reads: Arc<AtomicUsize>,
    }

    impl CountingDevice {
        fn new(address: u8, reads: Arc<AtomicUsize>) -> Self {
            Self { address, reads }
        }
    }

    impl I2cDevice for CountingDevice {
        fn address(&self) -> u8 {
            self.address
        }
        fn read(&mut self) -> u8 {
            self.reads.fetch_add(1, Ordering::SeqCst) as u8
        }
        fn write(&mut self, _data: u8) {}
    }

    #[test]
    fn test_i2c_reset_values() {
        let i2c = I2c::new();
        assert_eq!(i2c.read(0x00).unwrap(), 0); // CR1
        assert_eq!(i2c.read(0x04).unwrap(), 0); // CR2
    }

    #[test]
    fn test_i2c_start_bit() {
        let mut i2c = I2c::new();
        // Instant SB: Wire/HAL polls SR1.SB immediately after CR1.START.
        i2c.write(0x01, 0x01).unwrap(); // CR1 START (bit 8) → SR1.SB
        assert_ne!(
            i2c.peek(0x14).unwrap() & 0x01,
            0,
            "SB latches on START write"
        );
    }

    #[test]
    fn test_i2c_full_transfer_flow() {
        use crate::peripherals::components::Mpu6050;
        let mut i2c = I2c::new();
        i2c.push_slave(Box::new(Mpu6050::new(0x50)));

        i2c.write(0x01, 0x01).unwrap(); // START
        for _ in 0..10 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0); // SB

        i2c.write(0x10, 0xA0).unwrap(); // addr 0x50<<1 | W
        for _ in 0..20 {
            i2c.tick();
        }
        assert_eq!(i2c.peek(0x14).unwrap() & 0x01, 0); // SB cleared
        assert_ne!(i2c.peek(0x14).unwrap() & 0x02, 0); // ADDR
        assert_ne!(i2c.peek(0x18).unwrap() & 0x01, 0); // MSL
                                                       // TRA (SR2 bit2) must rise on write-address ACK — HAL EV IRQ gates
                                                       // the TXE/BTF path on TRA (RM0008 §26.6.7).
        assert_ne!(
            i2c.peek(0x18).unwrap() & 0x04,
            0,
            "TRA set after write-address ACK"
        );

        i2c.write(0x10, 0x42).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x80, 0); // TXE
        assert_ne!(i2c.peek(0x14).unwrap() & 0x04, 0); // BTF

        i2c.write(0x01, 0x02).unwrap(); // STOP (bit 9)
        for _ in 0..10 {
            i2c.tick();
        }
        assert_eq!(
            i2c.peek(0x18).unwrap() & 0x07,
            0,
            "STOP must clear MSL+BUSY+TRA"
        );
    }

    #[test]
    fn f1_write_address_sets_tra_and_level_ev_stays_asserted() {
        use crate::Peripheral;
        struct Ack {
            address: u8,
        }
        impl I2cDevice for Ack {
            fn address(&self) -> u8 {
                self.address
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, _: u8) {}
        }
        let mut i2c = I2c::new_with_layout(super::I2cRegisterLayout::Stm32F1);
        i2c.push_slave(Box::new(Ack { address: 0x40 }));
        // Enable ITEVTEN|ITBUFEN like HAL_I2C_Master_Transmit_IT.
        i2c.write_u32(0x04, (1 << 9) | (1 << 10)).unwrap();
        i2c.write(0x01, 0x01).unwrap(); // START → SB
        assert!(i2c.tick().irq, "SB with ITEVTEN asserts EV");
        i2c.write(0x10, 0x80).unwrap(); // 0x40 write
                                        // After address ACK: ADDR+TXE+TRA; level EV stays high across ticks.
        assert_ne!(i2c.peek(0x18).unwrap() & 0x04, 0, "TRA");
        assert_ne!(i2c.peek(0x14).unwrap() & 0x80, 0, "TXE");
        assert!(i2c.tick().irq, "level EV while TXE+ITBUFEN");
        assert!(i2c.tick().irq, "level EV re-assert next tick");
        // Clear ADDR via SR1 then SR2 (silicon sequence).
        let _ = i2c.read_u32(0x14).unwrap();
        let _ = i2c.read_u32(0x18).unwrap();
        assert_eq!(i2c.peek(0x14).unwrap() & 0x02, 0, "ADDR cleared by SR1→SR2");
        // TXE still live → EV still asserted for MasterTransmit_TXE.
        assert!(i2c.tick().irq, "TXE keeps EV high after ADDR clear");
    }

    #[test]
    fn test_adxl345_devid_and_axis_read() {
        use crate::peripherals::components::Adxl345;

        let mut i2c = I2c::new();
        let mut sensor = Adxl345::new(0x53);
        sensor.set_sample(256, -128, 64);
        i2c.push_slave(Box::new(sensor));

        i2c.write(0x00, 0x01).unwrap();
        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0);

        i2c.write(0x10, 0xA6).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x02, 0);

        i2c.write(0x10, 0x00).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }

        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        i2c.write(0x10, 0xA7).unwrap();
        for _ in 0..40 {
            i2c.tick();
        }
        assert_eq!(i2c.read(0x10).unwrap(), 0xE5);

        i2c.write(0x01, 0x02).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }

        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        i2c.write(0x10, 0xA6).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        i2c.write(0x10, 0x32).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        i2c.write(0x10, 0xA7).unwrap();
        for _ in 0..40 {
            i2c.tick();
        }

        assert_eq!(i2c.read(0x10).unwrap(), 0x00);
        assert_eq!(i2c.read(0x10).unwrap(), 0x01);
        assert_eq!(i2c.read(0x10).unwrap(), 0x80);
        assert_eq!(i2c.read(0x10).unwrap(), 0xFF);
        assert_eq!(i2c.read(0x10).unwrap(), 0x40);
        assert_eq!(i2c.read(0x10).unwrap(), 0x00);
    }

    #[test]
    fn test_i2c_single_byte_read_advances_device_once() {
        let reads = Arc::new(AtomicUsize::new(0));
        let mut i2c = I2c::new();
        i2c.push_slave(Box::new(CountingDevice::new(0x42, reads.clone())));

        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }

        i2c.write(0x10, 0x85).unwrap();
        for _ in 0..40 {
            i2c.tick();
        }

        assert_ne!(i2c.peek(0x14).unwrap() & 0x40, 0);
        assert_eq!(i2c.read(0x10).unwrap(), 0);
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    // ── STM32L4 (modern) transaction engine ──────────────────────────────────

    /// Configure CR2 for a 1-byte 7-bit master write to `addr` with AUTOEND,
    /// then load TXDR — the no-device case the tier-1 fixtures exercise.
    /// CR2 is a single 32-bit store (matches STM32 HAL).
    fn l4_write_xfer(i2c: &mut I2c, addr: u8, byte: u8) {
        use crate::Peripheral;
        i2c.write(0x00, 1).unwrap(); // CR1.PE
        let cr2: u32 = ((addr as u32) << 1) | (1 << 16) | (1 << 25) | (1 << 13);
        i2c.write_u32(0x04, cr2).unwrap();
        i2c.write(0x28, byte).unwrap(); // TXDR: first (only) byte
    }

    /// Address-only master write (NBYTES=0 + AUTOEND + START) — Wire probe.
    fn l4_addr_probe(i2c: &mut I2c, addr: u8) {
        use crate::Peripheral;
        i2c.write(0x00, 1).unwrap(); // CR1.PE
        let cr2: u32 = ((addr as u32) << 1) | (1 << 25) | (1 << 13); // NBYTES=0
        i2c.write_u32(0x04, cr2).unwrap();
    }

    /// Tick the engine past the address-phase wire-time window so the ACK/NACK
    /// verdict lands (TIMINGR left at reset → 144-cycle phase; 256 is safe margin).
    /// Run the controller until an armed transfer has fully settled.
    ///
    /// A write transfer costs two phases of wire time — the address phase and
    /// the data byte, each `address_phase_cycles()` (floor 64, 144 at the
    /// TIMINGR reset value these bare tests use) — so the budget has to clear
    /// both with room to spare.
    fn l4_settle(i2c: &mut I2c) {
        use crate::Peripheral;
        for _ in 0..1024 {
            i2c.tick();
        }
    }

    #[test]
    fn test_l4_i2c_nack_on_no_device() {
        use super::I2cRegisterLayout;
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);

        // Pending window: right after arming START the address phase is still on
        // the wire — START readable, BUSY set, NO NACKF yet (silicon fingerprint).
        l4_write_xfer(&mut i2c, 0x52, 0xAB);
        assert_ne!(
            i2c.peek(0x19).unwrap() & (1 << 7),
            0,
            "BUSY set while pending"
        ); // ISR.BUSY (bit15)
        assert_ne!(
            i2c.read_u32(0x04).unwrap() & (1 << 13),
            0,
            "START still readable"
        );
        assert_eq!(
            i2c.peek(0x18).unwrap() & (1 << 4),
            0,
            "no NACKF while pending"
        );

        // After the wire-time window: NACK on the absent device (AUTOEND clears BUSY).
        l4_settle(&mut i2c);
        assert_ne!(
            i2c.peek(0x18).unwrap() & (1 << 4),
            0,
            "ISR.NACKF when no slave"
        );
        assert_eq!(
            i2c.read_u32(0x04).unwrap() & (1 << 13),
            0,
            "START cleared after phase"
        );
        assert_eq!(i2c.peek(0x19).unwrap() & (1 << 7), 0, "AUTOEND clears BUSY");
        assert_ne!(i2c.peek(0x18).unwrap() & (1 << 5), 0, "AUTOEND sets STOPF");

        // ICR.NACKCF (bit4) + STOPCF (bit5) clear the flags.
        i2c.write(0x1C, (1 << 4) | (1 << 5)).unwrap();
        assert_eq!(
            i2c.peek(0x18).unwrap() & (1 << 4),
            0,
            "NACKF cleared by ICR"
        );
    }

    #[test]
    fn test_l4_i2c_nbytes0_probe_acks_device() {
        use super::I2cRegisterLayout;
        struct AckOnly {
            address: u8,
        }
        impl I2cDevice for AckOnly {
            fn address(&self) -> u8 {
                self.address
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, _: u8) {}
        }

        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
        i2c.push_slave(Box::new(AckOnly { address: 0x40 }));

        l4_addr_probe(&mut i2c, 0x40);
        l4_settle(&mut i2c);
        assert_eq!(
            i2c.peek(0x18).unwrap() & (1 << 4),
            0,
            "no NACKF on present device"
        );
        assert_ne!(
            i2c.peek(0x18).unwrap() & (1 << 6),
            0,
            "TC after address-only"
        );
        assert_ne!(i2c.peek(0x18).unwrap() & (1 << 5), 0, "STOPF via AUTOEND");
        assert_eq!(i2c.peek(0x19).unwrap() & (1 << 7), 0, "BUSY cleared");
    }

    #[test]
    fn test_l4_i2c_ack_delivers_byte_to_device() {
        use super::I2cRegisterLayout;
        use std::sync::atomic::AtomicUsize;
        let writes = Arc::new(AtomicUsize::new(0));

        struct WriteCounter {
            address: u8,
            writes: Arc<AtomicUsize>,
        }
        impl I2cDevice for WriteCounter {
            fn address(&self) -> u8 {
                self.address
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, _data: u8) {
                self.writes.fetch_add(1, Ordering::SeqCst);
            }
        }

        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
        i2c.push_slave(Box::new(WriteCounter {
            address: 0x3C,
            writes: writes.clone(),
        }));

        l4_write_xfer(&mut i2c, 0x3C, 0x42);
        l4_settle(&mut i2c);
        // Attached device ACKs → no NACKF, the byte reaches the device, TC set.
        assert_eq!(
            i2c.peek(0x18).unwrap() & (1 << 4),
            0,
            "no NACKF when device present"
        );
        assert_ne!(
            i2c.peek(0x18).unwrap() & (1 << 6),
            0,
            "TC after byte transferred"
        );
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }

    /// Minimal ACK-and-count slave for the master-write ordering tests.
    struct WriteSink {
        address: u8,
        writes: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl I2cDevice for WriteSink {
        fn address(&self) -> u8 {
            self.address
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, _data: u8) {
            self.writes.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// PRELOAD ordering (STM32Cube L0/L4/G4/WB `HAL_I2C_Master_Transmit_IT`):
    /// firmware writes TXDR BEFORE arming CR2/START. TXDR is a real holding
    /// register — the byte must be transmitted once the address phase ACKs, and
    /// the 1-byte AUTOEND transfer completes (TC + STOPF) with no tick loop.
    #[test]
    fn test_l4_i2c_write_preload_before_start() {
        use super::I2cRegisterLayout;
        use std::sync::atomic::AtomicUsize;
        let writes = Arc::new(AtomicUsize::new(0));
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
        i2c.push_slave(Box::new(WriteSink {
            address: 0x40,
            writes: writes.clone(),
        }));

        i2c.write(0x00, 1).unwrap(); // CR1.PE
        i2c.write(0x28, 0x00).unwrap(); // TXDR preloaded FIRST
        let cr2: u32 = (0x40 << 1) | (1 << 16) | (1 << 25) | (1 << 13); // NBYTES=1|AUTOEND|START
        i2c.write_u32(0x04, cr2).unwrap(); // CR2/START after the preload

        // Address phase takes wire time: nothing sent, START still readable.
        assert_eq!(
            writes.load(Ordering::SeqCst),
            0,
            "no byte during address phase"
        );
        assert_ne!(
            i2c.read_u32(0x04).unwrap() & (1 << 13),
            0,
            "START still readable"
        );
        l4_settle(&mut i2c);
        assert_eq!(
            writes.load(Ordering::SeqCst),
            1,
            "preloaded byte reaches slave"
        );
        assert_eq!(
            i2c.peek(0x18).unwrap() & (1 << 4),
            0,
            "no NACKF (slave present)"
        );
        assert_ne!(i2c.peek(0x18).unwrap() & (1 << 6), 0, "TC after transfer");
        assert_ne!(i2c.peek(0x18).unwrap() & (1 << 5), 0, "STOPF via AUTOEND");
        assert_eq!(i2c.peek(0x19).unwrap() & (1 << 7), 0, "BUSY cleared");
    }

    /// IT ordering (STM32Cube H5 `HAL_I2C_Master_Transmit_IT`): CR2/START first;
    /// hardware ACKs the address and asserts ISR.TXIS; only then does the ISR
    /// write TXDR. The model must set TXIS on the address ACK (park in
    /// DataPending), then complete the byte on the post-TXIS TXDR write.
    #[test]
    fn test_l4_i2c_write_txis_then_txdr() {
        use super::I2cRegisterLayout;
        use std::sync::atomic::AtomicUsize;
        let writes = Arc::new(AtomicUsize::new(0));
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
        i2c.push_slave(Box::new(WriteSink {
            address: 0x40,
            writes: writes.clone(),
        }));

        i2c.write(0x00, 1).unwrap(); // CR1.PE
        let cr2: u32 = (0x40 << 1) | (1 << 16) | (1 << 25) | (1 << 13); // NBYTES=1|AUTOEND|START
        i2c.write_u32(0x04, cr2).unwrap(); // START first — NO preloaded byte

        // Address phase in flight: no TXIS yet, START still readable.
        assert_eq!(
            i2c.peek(0x18).unwrap() & (1 << 1),
            0,
            "no TXIS during address phase"
        );
        assert_ne!(
            i2c.read_u32(0x04).unwrap() & (1 << 13),
            0,
            "START still readable"
        );

        // After the wire-time window the address ACKed → hardware requests the
        // first byte via TXIS (bit 1), nothing sent yet.
        l4_settle(&mut i2c);
        assert_ne!(
            i2c.peek(0x18).unwrap() & (1 << 1),
            0,
            "TXIS asserted after address ACK"
        );
        assert_eq!(
            writes.load(Ordering::SeqCst),
            0,
            "no byte before TXDR write"
        );

        i2c.write(0x28, 0x00).unwrap(); // ISR writes TXDR after TXIS
        assert_eq!(writes.load(Ordering::SeqCst), 1, "byte sent on TXDR write");
        // Completion is not instant: the byte occupies nine SCL bit-times
        // before TC (and the AUTOEND STOP) land, exactly as on silicon.
        assert_eq!(
            i2c.peek(0x18).unwrap() & (1 << 6),
            0,
            "TC must wait for the byte to clock out"
        );
        l4_settle(&mut i2c);
        assert_ne!(i2c.peek(0x18).unwrap() & (1 << 6), 0, "TC after transfer");
        assert_ne!(i2c.peek(0x18).unwrap() & (1 << 5), 0, "STOPF via AUTOEND");
    }

    /// Address-NACK must set ISR.NACKF (+STOPF via AUTOEND) in BOTH the preload
    /// and IT orderings, so the HAL returns error rather than hanging.
    #[test]
    fn test_l4_i2c_write_nack_both_orderings() {
        use super::I2cRegisterLayout;

        // Preload ordering: TXDR then START, absent slave.
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
        i2c.write(0x00, 1).unwrap();
        i2c.write(0x28, 0xAB).unwrap();
        let cr2: u32 = (0x52 << 1) | (1 << 16) | (1 << 25) | (1 << 13);
        i2c.write_u32(0x04, cr2).unwrap();
        l4_settle(&mut i2c);
        assert_ne!(
            i2c.peek(0x18).unwrap() & (1 << 4),
            0,
            "NACKF (preload order)"
        );
        assert_ne!(
            i2c.peek(0x18).unwrap() & (1 << 5),
            0,
            "STOPF via AUTOEND (preload order)"
        );
        assert_eq!(
            i2c.peek(0x19).unwrap() & (1 << 7),
            0,
            "BUSY cleared (preload order)"
        );

        // IT ordering: START first, absent slave.
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
        i2c.write(0x00, 1).unwrap();
        let cr2: u32 = (0x52 << 1) | (1 << 16) | (1 << 25) | (1 << 13);
        i2c.write_u32(0x04, cr2).unwrap();
        l4_settle(&mut i2c);
        assert_ne!(i2c.peek(0x18).unwrap() & (1 << 4), 0, "NACKF (IT order)");
        assert_ne!(
            i2c.peek(0x18).unwrap() & (1 << 5),
            0,
            "STOPF via AUTOEND (IT order)"
        );
        assert_eq!(
            i2c.peek(0x19).unwrap() & (1 << 7),
            0,
            "BUSY cleared (IT order)"
        );
    }

    #[test]
    fn i2c_attach_wraps_device_into_shared_log() {
        use crate::bus::bus_trace::{new_log, wrap_i2c, BusPayload};
        use crate::Peripheral;

        let log = new_log();
        let mut i2c = I2c::Kinetis(KinetisI2c::default());

        // device at 0x1E
        struct D;
        impl I2cDevice for D {
            fn address(&self) -> u8 {
                0x1E
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, _: u8) {}
        }
        // The bus choke point wraps before push; emulate it here.
        i2c.push_slave(wrap_i2c("i2c1", &log, Box::new(D)));

        // Drive START + addr(W) + one data byte through the Kinetis register
        // model via the public `Peripheral::write` MMIO path (the same path
        // every other Kinetis-adjacent test in this module uses to poke
        // registers — `write_reg` itself is private).
        i2c.write(0x02, KI_C1_MST | KI_C1_TX).unwrap(); // START
        i2c.write(0x04, 0x3C).unwrap(); // addr 0x1E + W -> selects device, start()
        i2c.write(0x04, 0xAF).unwrap(); // data -> device.write -> wrapper records

        let snap = log.snapshot();
        assert!(snap
            .iter()
            .any(|e| matches!(&e.payload, BusPayload::I2c { byte, .. } if *byte == 0xAF)));
    }

    // ── TCA9548A driven through the STM32L4 and Kinetis controllers ─────────
    //
    // `tests/i2c_mux_tca9548a.rs` proves the switch works through the STM32F1
    // legacy peripheral. The L4 and Kinetis engines are separate state machines
    // in this file with their own address-resolution sites, and both got the
    // `claims_address` / `select_address` change without any switch ever being
    // driven through them. These two modules close that.
    mod mux_stm32l4 {
        use super::super::{I2c, I2cRegisterLayout};
        use crate::peripherals::components::mux_fixture::{
            bytes_written_to, mux_with_tags, tag_for, MUX_ADDR, SENSOR_ADDR,
        };
        use crate::peripherals::components::tca9548a::Tca9548a;
        use crate::Peripheral;

        /// ICR.NACKCF (bit 4) + STOPCF (bit 5): clear the previous transfer's
        /// verdict so this one's NACKF assertion is about this one.
        fn clear_flags(i2c: &mut I2c) {
            i2c.write(0x1C, (1 << 4) | (1 << 5)).unwrap();
        }

        /// Did the last address phase NACK? ISR.NACKF is bit 4.
        fn nacked(i2c: &I2c) -> bool {
            i2c.peek(0x18).unwrap() & (1 << 4) != 0
        }

        /// One-byte master write (NBYTES=1 + AUTOEND), settled.
        fn write_one(i2c: &mut I2c, addr: u8, byte: u8) {
            clear_flags(i2c);
            super::l4_write_xfer(i2c, addr, byte);
            super::l4_settle(i2c);
        }

        /// One-byte master read (RD_WRN + NBYTES=1 + AUTOEND), settled. The
        /// byte lands in RXDR when the address phase ACKs.
        fn read_one(i2c: &mut I2c, addr: u8) -> u8 {
            clear_flags(i2c);
            i2c.write(0x00, 1).unwrap(); // CR1.PE
            let cr2: u32 = ((addr as u32) << 1)
                | (1 << 10)  // RD_WRN
                | (1 << 16)  // NBYTES = 1
                | (1 << 25)  // AUTOEND
                | (1 << 13); // START
            i2c.write_u32(0x04, cr2).unwrap();
            super::l4_settle(i2c);
            i2c.read(0x24).unwrap() // RXDR
        }

        /// Address-only probe (NBYTES=0): ACK/NACK with no data phase.
        fn probe_acked(i2c: &mut I2c, addr: u8) -> bool {
            clear_flags(i2c);
            super::l4_addr_probe(i2c, addr);
            super::l4_settle(i2c);
            !nacked(i2c)
        }

        fn bus() -> I2c {
            let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
            let trace = crate::bus::bus_trace::new_log();
            i2c.attach_traced("i2c1", &trace, Box::new(mux_with_tags(4)));
            i2c
        }

        /// Borrow the switch back out of the controller.
        fn with_mux<R>(i2c: &I2c, f: impl FnOnce(&Tca9548a) -> R) -> R {
            let cell = &i2c.attached_devices()[0];
            let traced = cell.borrow();
            let mux = traced
                .as_any()
                .and_then(|a| a.downcast_ref::<Tca9548a>())
                .expect("slave 0 is the switch");
            f(mux)
        }

        /// THE promise: four sensors that cannot be re-addressed, each reached
        /// independently through the L4 engine's own address resolution.
        #[test]
        fn four_sensors_at_one_address_answer_independently() {
            let mut i2c = bus();
            for ch in 0..4u8 {
                write_one(&mut i2c, MUX_ADDR, 1 << ch);
                assert_eq!(
                    read_one(&mut i2c, SENSOR_ADDR),
                    tag_for(ch),
                    "channel {ch} must be answered by the sensor wired to it"
                );
            }
        }

        /// Out-of-order selection: a controller that resolved the address once
        /// and cached it would keep answering with the first channel's sensor.
        #[test]
        fn switching_channels_changes_which_sensor_answers() {
            let mut i2c = bus();
            for ch in [2u8, 0, 3, 1, 3, 0] {
                write_one(&mut i2c, MUX_ADDR, 1 << ch);
                assert_eq!(read_one(&mut i2c, SENSOR_ADDR), tag_for(ch), "channel {ch}");
            }
        }

        #[test]
        fn control_register_reads_back_over_the_bus() {
            let mut i2c = bus();
            write_one(&mut i2c, MUX_ADDR, 0b0000_1010);
            assert!(
                probe_acked(&mut i2c, MUX_ADDR),
                "the switch must ACK its own address"
            );
            // No register pointer on the TCA9548A: a plain read returns the
            // control register.
            assert_eq!(read_one(&mut i2c, MUX_ADDR), 0b0000_1010);
        }

        #[test]
        fn a_sensor_on_a_disabled_channel_does_not_answer() {
            let mut i2c = bus();

            // Reset state: every channel isolated.
            assert!(
                !probe_acked(&mut i2c, SENSOR_ADDR),
                "with all channels disabled the sensor address must NACK, exactly \
                 as an empty bus does"
            );

            // Enable channel 1 only — 0x13 answers, and with channel 1's tag.
            write_one(&mut i2c, MUX_ADDR, 1 << 1);
            assert!(probe_acked(&mut i2c, SENSOR_ADDR));
            assert_eq!(read_one(&mut i2c, SENSOR_ADDR), tag_for(1));

            // Isolate again: it stops answering.
            write_one(&mut i2c, MUX_ADDR, 0x00);
            assert!(
                !probe_acked(&mut i2c, SENSOR_ADDR),
                "re-isolating the switch must take the sensor off the bus again"
            );
        }

        /// A data byte addressed to the sensor must reach the SELECTED
        /// channel's device and no other.
        #[test]
        fn a_write_reaches_only_the_selected_channel() {
            let mut i2c = bus();
            write_one(&mut i2c, MUX_ADDR, 1 << 2);
            write_one(&mut i2c, SENSOR_ADDR, 0x5A);

            with_mux(&i2c, |mux| {
                assert_eq!(bytes_written_to(mux, 2), vec![0x5A]);
                for ch in [0u8, 1, 3] {
                    assert!(
                        bytes_written_to(mux, ch).is_empty(),
                        "channel {ch} is isolated and must receive nothing"
                    );
                }
            });
        }
    }

    mod mux_kinetis {
        use super::super::{I2c, I2cRegisterLayout, KI_C1_MST, KI_C1_TX, KI_S_RXAK};
        use crate::peripherals::components::mux_fixture::{
            bytes_written_to, mux_with_tags, tag_for, MUX_ADDR, SENSOR_ADDR,
        };
        use crate::peripherals::components::tca9548a::Tca9548a;
        use crate::Peripheral;

        const REG_C1: u64 = 0x02;
        const REG_S: u64 = 0x03;
        const REG_D: u64 = 0x04;

        /// Did the slave ACK the most recent byte? S.RXAK is set on NAK.
        fn acked(i2c: &I2c) -> bool {
            i2c.peek(REG_S).unwrap() & KI_S_RXAK == 0
        }

        /// START + address(W) + one data byte + STOP, the fsl_i2c byte-at-a-time
        /// master-transmit shape.
        fn write_one(i2c: &mut I2c, addr: u8, byte: u8) {
            i2c.write(REG_C1, KI_C1_MST | KI_C1_TX).unwrap(); // START
            i2c.write(REG_D, addr << 1).unwrap(); // address + W
            i2c.write(REG_D, byte).unwrap();
            i2c.write(REG_C1, KI_C1_TX).unwrap(); // STOP (MST 1→0)
        }

        /// START + address(R), enter master-receive (the HAL's bus-release dummy
        /// read), then clock one real byte out.
        fn read_one(i2c: &mut I2c, addr: u8) -> u8 {
            i2c.write(REG_C1, KI_C1_MST | KI_C1_TX).unwrap(); // START
            i2c.write(REG_D, (addr << 1) | 1).unwrap(); // address + R
            i2c.write(REG_C1, KI_C1_MST).unwrap(); // TX 1→0: enter RX
            let _dummy = i2c.read(REG_D).unwrap(); // HAL bus-release read
            let byte = i2c.read(REG_D).unwrap();
            i2c.write(REG_C1, KI_C1_TX).unwrap(); // STOP
            byte
        }

        /// START + address(W) only: did anything on the bus ACK?
        fn probe_acked(i2c: &mut I2c, addr: u8) -> bool {
            i2c.write(REG_C1, KI_C1_MST | KI_C1_TX).unwrap();
            i2c.write(REG_D, addr << 1).unwrap();
            let ack = acked(i2c);
            i2c.write(REG_C1, KI_C1_TX).unwrap(); // STOP
            ack
        }

        fn bus() -> I2c {
            let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Kinetis);
            let trace = crate::bus::bus_trace::new_log();
            i2c.attach_traced("i2c0", &trace, Box::new(mux_with_tags(4)));
            i2c
        }

        fn with_mux<R>(i2c: &I2c, f: impl FnOnce(&Tca9548a) -> R) -> R {
            let cell = &i2c.attached_devices()[0];
            let traced = cell.borrow();
            let mux = traced
                .as_any()
                .and_then(|a| a.downcast_ref::<Tca9548a>())
                .expect("slave 0 is the switch");
            f(mux)
        }

        #[test]
        fn four_sensors_at_one_address_answer_independently() {
            let mut i2c = bus();
            for ch in 0..4u8 {
                write_one(&mut i2c, MUX_ADDR, 1 << ch);
                assert_eq!(
                    read_one(&mut i2c, SENSOR_ADDR),
                    tag_for(ch),
                    "channel {ch} must be answered by the sensor wired to it"
                );
            }
        }

        #[test]
        fn switching_channels_changes_which_sensor_answers() {
            let mut i2c = bus();
            for ch in [2u8, 0, 3, 1, 3, 0] {
                write_one(&mut i2c, MUX_ADDR, 1 << ch);
                assert_eq!(read_one(&mut i2c, SENSOR_ADDR), tag_for(ch), "channel {ch}");
            }
        }

        #[test]
        fn control_register_reads_back_over_the_bus() {
            let mut i2c = bus();
            write_one(&mut i2c, MUX_ADDR, 0b0000_1010);
            assert!(
                probe_acked(&mut i2c, MUX_ADDR),
                "the switch must ACK its own address"
            );
            assert_eq!(read_one(&mut i2c, MUX_ADDR), 0b0000_1010);
        }

        #[test]
        fn a_sensor_on_a_disabled_channel_does_not_answer() {
            let mut i2c = bus();
            assert!(
                !probe_acked(&mut i2c, SENSOR_ADDR),
                "with all channels disabled the sensor address must NAK (S.RXAK), \
                 exactly as an empty bus does"
            );

            write_one(&mut i2c, MUX_ADDR, 1 << 1);
            assert!(probe_acked(&mut i2c, SENSOR_ADDR));
            assert_eq!(read_one(&mut i2c, SENSOR_ADDR), tag_for(1));

            write_one(&mut i2c, MUX_ADDR, 0x00);
            assert!(
                !probe_acked(&mut i2c, SENSOR_ADDR),
                "re-isolating the switch must take the sensor off the bus again"
            );
        }

        #[test]
        fn a_write_reaches_only_the_selected_channel() {
            let mut i2c = bus();
            write_one(&mut i2c, MUX_ADDR, 1 << 2);
            write_one(&mut i2c, SENSOR_ADDR, 0x5A);

            with_mux(&i2c, |mux| {
                assert_eq!(bytes_written_to(mux, 2), vec![0x5A]);
                for ch in [0u8, 1, 3] {
                    assert!(
                        bytes_written_to(mux, ch).is_empty(),
                        "channel {ch} is isolated and must receive nothing"
                    );
                }
            });
        }
    }
}

// ── Walk-free (batch B4) differential: Kinetis level-IRQ walk vs scheduler ────
#[cfg(all(test, feature = "event-scheduler"))]
mod kinetis_scheduler {
    use super::*;
    use crate::Peripheral;

    /// A slave that returns an incrementing byte pattern on each read (so a
    /// master-receive advances observably) and records writes.
    struct RampDevice {
        address: u8,
        next: std::cell::Cell<u8>,
    }
    impl I2cDevice for RampDevice {
        fn address(&self) -> u8 {
            self.address
        }
        fn read(&mut self) -> u8 {
            let v = self.next.get();
            self.next.set(v.wrapping_add(1));
            v
        }
        fn write(&mut self, _data: u8) {}
    }

    fn ramp_slave() -> Box<dyn I2cDevice> {
        Box::new(RampDevice {
            address: 0x1E,
            next: std::cell::Cell::new(0x40),
        })
    }

    fn kinetis(scheduler: bool) -> I2c {
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Kinetis);
        i2c.push_slave(ramp_slave());
        if scheduler {
            i2c.attach_cycle_clock(CycleClock::default());
        }
        i2c
    }

    fn f1(scheduler: bool) -> I2c {
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32F1);
        i2c.push_slave(ramp_slave());
        if scheduler {
            i2c.attach_cycle_clock(CycleClock::default());
        }
        i2c
    }

    fn l4(scheduler: bool) -> I2c {
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
        i2c.push_slave(ramp_slave());
        if scheduler {
            i2c.attach_cycle_clock(CycleClock::default());
        }
        i2c
    }

    /// Clone the bus clock a scheduler-mode instance latched (any variant).
    fn clock_of(i2c: &I2c) -> CycleClock {
        match i2c {
            I2c::Stm32F1(i) => i.clock.clone(),
            I2c::Stm32L4(i) => i.clock.clone(),
            I2c::Kinetis(i) => i.clock.clone(),
            // The EFR32 controller has no cycle clock at all: it completes a
            // transfer inside the register write and never schedules an event,
            // so it is not one of the variants this harness can drive.
            I2c::Efr32s2(_) => panic!("EFR32 I2C is not a scheduler-mode variant"),
        }
        .expect("scheduler-mode instance has a clock")
    }

    #[derive(Clone, Copy, Debug)]
    enum Op {
        Write(u64, u8),
        Read(u64),
    }

    /// Drive a scheduler-mode Kinetis I2C exactly the way `Machine` +
    /// `SystemBus` do at tick interval 1: publish the clock each cycle, arm
    /// write-harvested events at `cycle + 1 + delay`, and drain due events
    /// through `on_event` (rescheduling at `now + delay`), recording the cycles
    /// the level chain pends the own-IRQ.
    struct SchedHarness {
        i2c: I2c,
        clock: CycleClock,
        bus: crate::bus::SystemBus,
        events: Vec<(u64, u32)>,
        now: u64,
        pends: Vec<u64>,
    }

    impl SchedHarness {
        fn new(build: &dyn Fn(bool) -> I2c) -> Self {
            let i2c = build(true);
            let clock = clock_of(&i2c);
            Self {
                i2c,
                clock,
                bus: crate::bus::SystemBus::new(),
                events: Vec::new(),
                now: 0,
                pends: Vec::new(),
            }
        }

        fn write(&mut self, off: u64, val: u8) {
            self.i2c.sync_to(self.now);
            self.i2c.write(off, val).unwrap();
            for (delay, token) in self.i2c.take_scheduled_events() {
                self.events.push((self.now + 1 + delay, token));
            }
        }

        /// A `&self` register read — never arms an event (mirrors the bus read
        /// path); a `D` read that latches IICIF is caught by the already-live
        /// perpetual chain.
        fn read(&mut self, off: u64) -> u8 {
            self.i2c.read(off).unwrap()
        }

        fn step(&mut self) {
            self.now += 1;
            self.clock.publish(self.now);
            let due: Vec<(u64, u32)> = self
                .events
                .iter()
                .copied()
                .filter(|(d, _)| *d <= self.now)
                .collect();
            self.events.retain(|(d, _)| *d > self.now);
            let mut sched = crate::sched::EventScheduler::new();
            sched.advance_to(self.now);
            for (_, token) in due {
                let res = self.i2c.on_event(token, &mut sched, &mut self.bus);
                if res.raise_own_irq {
                    self.pends.push(self.now);
                }
                if let Some(delay) = res.reschedule_delay {
                    self.events.push((self.now + delay, token));
                }
            }
        }
    }

    /// Legacy per-tick oracle.
    fn walk_tick(i2c: &mut I2c) -> bool {
        i2c.tick().irq
    }

    /// The heart of the gate: replay the SAME op script against (a) the legacy
    /// per-tick walk and (b) the event path, comparing the full register
    /// snapshot AND every returned read byte at every cycle, plus the exact set
    /// of NVIC-pend cycles. An `Op` scheduled at cycle `c` is applied before
    /// that cycle's tick.
    fn assert_walk_identical_with(
        build: &dyn Fn(bool) -> I2c,
        script: &[(u64, Op)],
        cycles: u64,
        what: &str,
    ) {
        let mut walk = build(false);
        let mut sched = SchedHarness::new(build);
        let mut walk_pends: Vec<u64> = Vec::new();

        for c in 1..=cycles {
            for (sc, op) in script {
                if *sc == c {
                    match *op {
                        Op::Write(off, val) => {
                            walk.write(off, val).unwrap();
                            sched.now = c - 1;
                            sched.write(off, val);
                        }
                        Op::Read(off) => {
                            let w = walk.read(off).unwrap();
                            sched.now = c - 1;
                            let s = sched.read(off);
                            assert_eq!(w, s, "{what}: read(0x{off:02x}) diverged at cycle {c}");
                        }
                    }
                }
            }
            if walk_tick(&mut walk) {
                walk_pends.push(c);
            }
            sched.now = c - 1;
            sched.step();
            assert_eq!(
                walk.snapshot(),
                sched.i2c.snapshot(),
                "{what}: register state diverged at cycle {c}"
            );
        }
        assert_eq!(walk_pends, sched.pends, "{what}: NVIC pend cycles diverged");
    }

    /// Kinetis-variant convenience wrapper.
    fn assert_walk_identical(script: &[(u64, Op)], cycles: u64, what: &str) {
        assert_walk_identical_with(&kinetis, script, cycles, what);
    }

    #[test]
    fn clock_attach_flips_to_scheduler_and_walk_tick_is_inert() {
        let mut i2c = kinetis(true);
        assert!(i2c.uses_scheduler());
        assert!(!i2c.needs_legacy_walk());
        // Latch a level (address byte with IICIE) then confirm tick() is inert.
        i2c.write(0x02, KI_C1_MST | KI_C1_TX | KI_C1_IICIE).unwrap();
        i2c.write(0x04, 0x3C).unwrap(); // address → byte_complete sets IICIF
        assert!(!i2c.tick().irq, "tick must be inert in scheduler mode");
    }

    #[test]
    fn all_three_variants_flip_to_scheduler_and_walk_tick_is_inert() {
        // With a clock attached (event-scheduler builds) every I2C variant now
        // migrates: the F1/L4 transaction engine is self-paced by the same
        // held-level event chain the Kinetis variant uses, so the per-cycle walk
        // is no longer needed. The walk-guarded `tick()` is inert in that mode.
        for build in [&f1 as &dyn Fn(bool) -> I2c, &l4, &kinetis] {
            let mut i2c = build(true);
            assert!(i2c.uses_scheduler());
            assert!(!i2c.needs_legacy_walk());
            assert!(
                !i2c.tick().irq && i2c.tick().cycles == 0,
                "walk tick must be inert in scheduler mode"
            );
            // Clock detached (differential reference / featureless): back to walk.
            i2c.force_legacy_walk();
            assert!(!i2c.uses_scheduler());
            assert!(i2c.needs_legacy_walk());
        }
    }

    // ── STM32 F1 transaction-engine walk-vs-scheduler byte identity ───────────

    /// Master WRITE: START → address(W) → data byte → STOP, with ITEVTEN|ITBUFEN
    /// enabled so the completion IRQs are pend-compared. Every register snapshot,
    /// read byte and NVIC-pend cycle must be byte-identical between the per-cycle
    /// walk and the event-scheduled engine.
    #[test]
    fn f1_master_write_walk_identity() {
        let addr_w = 0x1E << 1; // 0x3C
        let script = [
            (1u64, Op::Write(0x05, 0x06)), // CR2 = ITEVTEN|ITBUFEN (bits 9,10)
            (1, Op::Write(0x01, 0x01)),    // CR1.START (bit 8)
            (4, Op::Read(0x14)),           // poll SR1 (SB)
            (5, Op::Write(0x10, addr_w)),  // DR = address(W) → AddressPending
            (28, Op::Read(0x14)),          // poll SR1 (ADDR/TXE)
            (28, Op::Read(0x18)),          // poll SR2 (MSL/BUSY)
            (30, Op::Write(0x10, 0xAF)),   // DR = data byte → DataPending
            (54, Op::Read(0x14)),          // poll SR1 (TXE/BTF)
            (56, Op::Write(0x01, 0x02)),   // CR1.STOP (bit 9)
        ];
        assert_walk_identical_with(&f1, &script, 64, "f1 master write");
    }

    /// Master READ: START → address(R) → multi-byte receive (the `&self` DR-read
    /// path that the prior model claimed could not be event-scheduled) → STOP.
    /// The receive bytes come straight from the device in `read()`; the engine
    /// only paces the START/ADDR/first-byte countdowns. The already-live chain
    /// keeps the register state identical across the read-gated stream.
    #[test]
    fn f1_master_read_multibyte_walk_identity() {
        let addr_r = (0x1E << 1) | 1; // 0x3D
        let script = [
            (1u64, Op::Write(0x05, 0x06)), // CR2 = ITEVTEN|ITBUFEN
            (1, Op::Write(0x01, 0x01)),    // START
            (5, Op::Write(0x10, addr_r)),  // DR = address(R) → AddressPending(read)
            (30, Op::Read(0x14)),          // poll SR1 (ADDR)
            (54, Op::Read(0x14)),          // poll SR1 (RXNE after first byte)
            (54, Op::Read(0x10)),          // read byte 0 (buffered dr)
            (55, Op::Read(0x10)),          // read byte 1 (device pull)
            (56, Op::Read(0x10)),          // read byte 2 (device pull)
            (57, Op::Read(0x18)),          // SR2 still BUSY
            (58, Op::Write(0x01, 0x02)),   // STOP
        ];
        assert_walk_identical_with(&f1, &script, 66, "f1 master read multibyte");
    }

    /// Address NACK (no slave at the addressed target) — the AF/MSL/BUSY latch
    /// and the ITERREN-gated error IRQ must match. Uses a mismatched address so
    /// `current_target` is `None`.
    #[test]
    fn f1_address_nack_walk_identity() {
        let script = [
            (1u64, Op::Write(0x05, 0x01)), // CR2 ITERREN (bit 8) → byte at offset 0x05
            (1, Op::Write(0x01, 0x01)),    // START
            (5, Op::Write(0x10, 0x40)),    // DR = address 0x20<<1 (no device) → NACK
            (30, Op::Read(0x14)),          // poll SR1 (AF)
            (30, Op::Read(0x18)),          // poll SR2 (MSL/BUSY held)
            (32, Op::Write(0x01, 0x02)),   // STOP releases the bus
        ];
        assert_walk_identical_with(&f1, &script, 40, "f1 address NACK");
    }

    // ── STM32 L4 transaction-engine walk-vs-scheduler byte identity ───────────

    /// L4 master WRITE via CR2 START/AUTOEND + TXDR, with TCIE|NACKIE enabled.
    #[test]
    fn l4_master_write_walk_identity() {
        // CR1.PE (bit0) | TCIE (bit6) | NACKIE (bit4) = 0x51.
        // CR2 = SADD(0x1E<<1) | NBYTES=1<<16 | AUTOEND<<25 | START<<13.
        let cr2: u32 = ((0x1E << 1) as u32) | (1 << 16) | (1 << 25) | (1 << 13);
        let script = [
            (1u64, Op::Write(0x00, 0x51)), // CR1 = PE|TCIE|NACKIE
            (2, Op::Write(0x04, (cr2 & 0xFF) as u8)),
            (2, Op::Write(0x05, ((cr2 >> 8) & 0xFF) as u8)),
            (2, Op::Write(0x06, ((cr2 >> 16) & 0xFF) as u8)),
            (2, Op::Write(0x07, ((cr2 >> 24) & 0xFF) as u8)), // START latches BUSY
            (3, Op::Read(0x19)),                              // ISR byte3 (BUSY bit15)
            (4, Op::Write(0x28, 0xAF)),                       // TXDR → AddressPending
            (28, Op::Read(0x18)),                             // ISR byte0 (TXE/TC)
            (28, Op::Read(0x19)),                             // ISR byte3 (BUSY cleared by AUTOEND)
        ];
        assert_walk_identical_with(&l4, &script, 36, "l4 master write");
    }

    /// L4 address NACK (no device) — NACKF + AUTOEND STOPF, NACKIE IRQ.
    #[test]
    fn l4_address_nack_walk_identity() {
        let cr2: u32 = ((0x20 << 1) as u32) | (1 << 16) | (1 << 25) | (1 << 13);
        let script = [
            (1u64, Op::Write(0x00, 0x51)), // CR1 = PE|TCIE|NACKIE
            (2, Op::Write(0x04, (cr2 & 0xFF) as u8)),
            (2, Op::Write(0x05, ((cr2 >> 8) & 0xFF) as u8)),
            (2, Op::Write(0x06, ((cr2 >> 16) & 0xFF) as u8)),
            (2, Op::Write(0x07, ((cr2 >> 24) & 0xFF) as u8)),
            (4, Op::Write(0x28, 0xAF)), // TXDR → AddressPending → NACK
            (28, Op::Read(0x18)),       // ISR (NACKF/STOPF)
            (28, Op::Read(0x19)),       // ISR byte3 (BUSY)
        ];
        assert_walk_identical_with(&l4, &script, 36, "l4 address NACK");
    }

    #[test]
    fn master_write_level_irq_walk_identity() {
        // START, address (byte_complete latches IICIF), enable IICIE (level
        // high), let it pend for a few cycles (ISR latency), clear IICIF + send
        // a data byte (re-latch), clear again, then STOP.
        let addr_w = 0x1E << 1; // write
        let script = [
            (1u64, Op::Write(0x02, KI_C1_MST | KI_C1_TX)), // START
            (1, Op::Write(0x04, addr_w)),                  // address → IICIF
            (2, Op::Write(0x02, KI_C1_MST | KI_C1_TX | KI_C1_IICIE)), // enable IICIE
            (6, Op::Write(0x03, KI_S_IICIF)),              // ISR clears IICIF
            (6, Op::Write(0x04, 0xAA)),                    // next byte → IICIF
            (11, Op::Write(0x03, KI_S_IICIF)),             // clear
            (11, Op::Write(0x04, 0xBB)),                   // byte → IICIF
            (16, Op::Write(0x03, KI_S_IICIF)),             // clear
            (17, Op::Write(0x02, 0)),                      // STOP (MST 1→0)
        ];
        assert_walk_identical(&script, 24, "kinetis master write level IRQ");
    }

    #[test]
    fn master_read_dread_latches_irq_walk_identity() {
        // The crux: a master-receive `D` read latches IICIF via a `&self` read
        // (which cannot arm an event) — the already-live perpetual level chain
        // must pend on the SAME cycle as the walk.
        let addr_r = (0x1E << 1) | 1; // read
        let script = [
            (1u64, Op::Write(0x02, KI_C1_MST | KI_C1_TX | KI_C1_IICIE)), // START + IICIE
            (1, Op::Write(0x04, addr_r)), // address(R) → IICIF, is_reading
            (5, Op::Write(0x03, KI_S_IICIF)), // ISR clears IICIF
            (5, Op::Write(0x02, KI_C1_MST | KI_C1_IICIE)), // TX=0 → enter RX (rx_dummy_pending)
            (6, Op::Read(0x04)),          // dummy read → IICIF (bus release)
            (10, Op::Write(0x03, KI_S_IICIF)), // clear
            (11, Op::Read(0x04)),         // data read → device byte + IICIF
            (15, Op::Write(0x03, KI_S_IICIF)), // clear
            (16, Op::Read(0x04)),         // data read → IICIF
            (20, Op::Write(0x03, KI_S_IICIF)), // clear
            (21, Op::Write(0x02, 0)),     // STOP
        ];
        assert_walk_identical(&script, 28, "kinetis master read D-latch level IRQ");
    }

    #[test]
    fn iicie_disabled_never_pends_walk_identity() {
        // IICIF latched but IICIE never set: the level is low, no pend in either
        // mode, and the chain must not even arm.
        let script = [
            (1u64, Op::Write(0x02, KI_C1_MST | KI_C1_TX)), // START, no IICIE
            (1, Op::Write(0x04, 0x1E << 1)),               // address → IICIF (but IICIE off)
            (5, Op::Write(0x04, 0x55)),                    // byte → IICIF
        ];
        assert_walk_identical(&script, 12, "kinetis IICIE-off no pend");
    }
}

#[cfg(test)]
mod l4_disable_tests {
    use super::L4I2c;

    // I2C v2 register map (RM0367 §26.7 / RM0351 §39.7).
    const CR1: u64 = 0x00;
    const CR2: u64 = 0x04;
    const TIMINGR: u64 = 0x10;
    const ISR: u64 = 0x18;

    const PE: u32 = 1 << 0;
    const BUSY: u32 = 1 << 15;
    const START: u32 = 1 << 13;

    /// CR2 arming a 1-byte write to an unattached slave, AUTOEND=0 — the shape
    /// the NUCLEO-L073RZ demo issues, and the one the HAL recovers from by
    /// toggling PE. Address 0x52 in SADD[7:1].
    const ARM_WRITE: u32 = START | (0x52 << 1) | (1 << 16);

    fn armed() -> L4I2c {
        let mut i2c = L4I2c::default();
        i2c.write_reg(TIMINGR, 0x0010_0000);
        i2c.write_reg(CR1, PE);
        i2c.write_reg(CR2, ARM_WRITE);
        assert_eq!(
            i2c.read_reg(ISR) & BUSY,
            BUSY,
            "precondition: arming a transfer latches BUSY"
        );
        i2c
    }

    /// RM0367 §26.7.1 / RM0351 §39.7.1: "When PE=0 ... internal state machines
    /// and status bits are put back to their reset value." The model used to
    /// store CR1 and leave everything else alone, so BUSY read 1 forever after
    /// the HAL's standard NACK recovery (#835).
    #[test]
    fn clearing_pe_clears_busy() {
        let mut i2c = armed();
        i2c.write_reg(CR1, 0);
        assert_eq!(
            i2c.read_reg(ISR),
            0x0000_0001,
            "PE=0 must return ISR to its reset value (TXE set, BUSY clear)"
        );
    }

    /// The throughput half of #835: while BUSY is latched, `active()` stays true
    /// and the per-cycle engine chain re-arms at +1 forever, which pins the CPU
    /// quantum to one instruction for the life of the machine.
    #[test]
    fn clearing_pe_makes_the_engine_idle() {
        let mut i2c = armed();
        assert!(i2c.active(), "precondition: an armed transfer is active");
        i2c.write_reg(CR1, 0);
        assert!(
            !i2c.active(),
            "a disabled peripheral must not keep the scheduler chain alive"
        );
    }

    /// Re-enabling must start from a clean engine rather than resuming the
    /// transfer that was abandoned.
    #[test]
    fn re_enabling_starts_clean() {
        let mut i2c = armed();
        i2c.write_reg(CR1, 0);
        i2c.write_reg(CR1, PE);
        assert_eq!(
            i2c.read_reg(ISR) & BUSY,
            0,
            "re-enable must not restore BUSY"
        );
        assert!(!i2c.active());
    }

    /// A write that leaves PE set must not disturb a transfer in flight —
    /// firmware sets interrupt-enable bits in CR1 mid-transfer all the time.
    #[test]
    fn setting_other_cr1_bits_does_not_reset_the_engine() {
        let mut i2c = armed();
        i2c.write_reg(CR1, PE | (1 << 1) | (1 << 2)); // TXIE | RXIE
        assert_eq!(
            i2c.read_reg(ISR) & BUSY,
            BUSY,
            "an in-flight transfer must survive an unrelated CR1 write"
        );
        assert!(i2c.active());
    }

    /// Writing CR1 while already disabled is a no-op, not a second reset.
    #[test]
    fn writing_cr1_while_disabled_is_inert() {
        let mut i2c = L4I2c::default();
        i2c.write_reg(CR1, 0);
        assert_eq!(i2c.read_reg(ISR), 0x0000_0001);
        assert!(!i2c.active());
    }
}

#[cfg(test)]
mod efr32s2_tests {
    use super::*;
    use crate::Peripheral;

    /// A minimal register-file slave: the shape almost every I²C sensor has.
    /// Write one byte to select a register, then read to stream from it.
    #[derive(Debug)]
    struct FakeSensor {
        addr: u8,
        regs: [u8; 4],
        pointer: usize,
        starts: usize,
        stops: usize,
    }

    impl FakeSensor {
        fn new(addr: u8) -> Self {
            Self {
                addr,
                regs: [0xA1, 0xB2, 0xC3, 0xD4],
                pointer: 0,
                starts: 0,
                stops: 0,
            }
        }
    }

    impl I2cDevice for FakeSensor {
        fn address(&self) -> u8 {
            self.addr
        }
        fn read(&mut self) -> u8 {
            let b = self.regs[self.pointer % self.regs.len()];
            self.pointer += 1;
            b
        }
        fn write(&mut self, data: u8) {
            self.pointer = data as usize;
        }
        fn start(&mut self) {
            self.starts += 1;
        }
        fn stop(&mut self) {
            self.stops += 1;
        }
    }

    fn enabled() -> I2c {
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Efr32s2);
        i2c.write_u32(EFR_I2C_EN, EFR_EN_EN).unwrap();
        i2c
    }

    fn inner(i2c: &I2c) -> &Efr32s2I2c {
        match i2c {
            I2c::Efr32s2(i) => i,
            _ => panic!("wrong layout"),
        }
    }

    fn clear_flags(i2c: &mut I2c) {
        i2c.write_u32(EFR_I2C_IF, 0xFFFF_FFFF).unwrap();
    }

    fn flags(i2c: &I2c) -> u32 {
        i2c.read_u32(EFR_I2C_IF).unwrap()
    }

    /// `(addr << 1) | rw`, the byte firmware writes to TXDATA after a START.
    fn addr_byte(addr: u8, reading: bool) -> u32 {
        ((addr as u32) << 1) | u32::from(reading)
    }

    #[test]
    fn the_layout_resolves_by_name_and_reports_itself() {
        let i2c = I2c::new_with_layout(I2cRegisterLayout::Efr32s2);
        assert_eq!(i2c.register_layout(), I2cRegisterLayout::Efr32s2);
        assert_eq!(
            "efr32s2".parse::<I2cRegisterLayout>().unwrap(),
            I2cRegisterLayout::Efr32s2
        );
    }

    #[test]
    fn ipversion_reads_the_header_reset_value() {
        let i2c = I2c::new_with_layout(I2cRegisterLayout::Efr32s2);
        assert_eq!(i2c.read_u32(EFR_I2C_IPVERSION).unwrap(), 3);
    }

    /// The whole `Wire.beginTransmission / write / endTransmission` path.
    #[test]
    fn a_write_transaction_reaches_the_slave() {
        let mut i2c = enabled();
        i2c.push_slave(Box::new(FakeSensor::new(0x48)));
        clear_flags(&mut i2c);

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        assert_eq!(flags(&i2c) & EFR_IF_START, EFR_IF_START);

        i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, false))
            .unwrap();
        assert_eq!(flags(&i2c) & EFR_IF_ACK, EFR_IF_ACK, "the slave answered");
        assert_eq!(flags(&i2c) & EFR_IF_NACK, 0);

        i2c.write_u32(EFR_I2C_TXDATA, 2).unwrap(); // select register 2
        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_STOP).unwrap();
        assert_eq!(flags(&i2c) & EFR_IF_MSTOP, EFR_IF_MSTOP);

        let dev = &inner(&i2c).attached_devices[0];
        assert_eq!(dev.borrow().address(), 0x48);
    }

    /// An address nobody claims must NACK. This is the difference between a
    /// sketch finding out its sensor is not wired and one that appears to work.
    #[test]
    fn an_unclaimed_address_nacks() {
        let mut i2c = enabled();
        i2c.push_slave(Box::new(FakeSensor::new(0x48)));
        clear_flags(&mut i2c);

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x77, false))
            .unwrap();

        assert_eq!(flags(&i2c) & EFR_IF_NACK, EFR_IF_NACK);
        assert_eq!(flags(&i2c) & EFR_IF_ACK, 0);
        assert_eq!(
            i2c.read_u32(EFR_I2C_STATE).unwrap() & EFR_STATE_NACKED,
            EFR_STATE_NACKED
        );
    }

    /// `Wire.requestFrom`: START, address with R, then a byte per ACK.
    #[test]
    fn a_read_transaction_streams_bytes_from_the_slave() {
        let mut i2c = enabled();
        i2c.push_slave(Box::new(FakeSensor::new(0x48)));
        clear_flags(&mut i2c);

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, true))
            .unwrap();

        assert_eq!(
            i2c.read_u32(EFR_I2C_STATUS).unwrap() & EFR_STATUS_RXDATAV,
            EFR_STATUS_RXDATAV,
            "the first byte is ready once the address is acked"
        );
        assert_eq!(i2c.read_u32(EFR_I2C_RXDATA).unwrap(), 0xA1);

        // ACK asks for another byte.
        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_ACK).unwrap();
        assert_eq!(i2c.read_u32(EFR_I2C_RXDATA).unwrap(), 0xB2);
        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_ACK).unwrap();
        assert_eq!(i2c.read_u32(EFR_I2C_RXDATA).unwrap(), 0xC3);

        // NACK ends it: no further byte is fetched.
        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_NACK | EFR_CMD_STOP)
            .unwrap();
        assert_eq!(
            i2c.read_u32(EFR_I2C_STATUS).unwrap() & EFR_STATUS_RXDATAV,
            0
        );
    }

    /// Reading RXDATA CONSUMES; reading RXDATAP does not. A driver that peeks
    /// must not lose a byte.
    #[test]
    fn rxdatap_peeks_where_rxdata_consumes() {
        let mut i2c = enabled();
        i2c.push_slave(Box::new(FakeSensor::new(0x48)));
        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, true))
            .unwrap();

        assert_eq!(i2c.read_u32(EFR_I2C_RXDATAP).unwrap(), 0xA1);
        assert_eq!(i2c.read_u32(EFR_I2C_RXDATAP).unwrap(), 0xA1, "still there");
        assert_eq!(i2c.read_u32(EFR_I2C_RXDATA).unwrap(), 0xA1, "now taken");
        assert_eq!(
            i2c.read_u32(EFR_I2C_STATUS).unwrap() & EFR_STATUS_RXDATAV,
            0
        );
    }

    /// ⚠️ `peek` is a side-effect-free probe for observers. It must not consume
    /// the RX byte — the IADC hit exactly this and read back zeroes.
    #[test]
    fn peeking_the_data_register_does_not_consume_the_byte() {
        let mut i2c = enabled();
        i2c.push_slave(Box::new(FakeSensor::new(0x48)));
        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, true))
            .unwrap();

        assert_eq!(i2c.peek(EFR_I2C_RXDATA), Some(0xA1));
        assert_eq!(i2c.peek(EFR_I2C_RXDATA), Some(0xA1));
        assert_eq!(i2c.read_u32(EFR_I2C_RXDATA).unwrap(), 0xA1);
    }

    /// The register-then-read idiom: write a pointer, repeated START, read.
    #[test]
    fn a_repeated_start_switches_direction_without_releasing_the_bus() {
        let mut i2c = enabled();
        i2c.push_slave(Box::new(FakeSensor::new(0x48)));
        clear_flags(&mut i2c);

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, false))
            .unwrap();
        i2c.write_u32(EFR_I2C_TXDATA, 3).unwrap(); // pointer := 3
        clear_flags(&mut i2c);

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        assert_eq!(
            flags(&i2c) & EFR_IF_RSTART,
            EFR_IF_RSTART,
            "a START while the bus is held is a REPEATED start"
        );
        assert_eq!(flags(&i2c) & EFR_IF_START, 0);

        i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, true))
            .unwrap();
        assert_eq!(
            i2c.read_u32(EFR_I2C_RXDATA).unwrap(),
            0xD4,
            "reads from the register the pointer selected"
        );
    }

    #[test]
    fn a_disabled_controller_does_nothing_at_all() {
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Efr32s2);
        i2c.push_slave(Box::new(FakeSensor::new(0x48)));
        clear_flags(&mut i2c);

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, false))
            .unwrap();
        assert_eq!(flags(&i2c), 0, "no START, no ACK, no NACK");
        // BUSY is still the power-on 1 here: a DISABLED controller cannot have
        // driven the bus, so it has not learned the bus is idle either. What
        // this case is about is that nothing else moved — see `flags` above and
        // MASTER below, which a real START would have set.
        let state = i2c.read_u32(EFR_I2C_STATE).unwrap();
        assert_eq!(state & EFR_STATE_MASTER, 0, "a disabled controller never masters");
    }

    #[test]
    fn state_reports_busy_and_master_between_start_and_stop() {
        let mut i2c = enabled();
        i2c.push_slave(Box::new(FakeSensor::new(0x48)));
        // ⚠️ BUSY is SET on a controller nothing has driven yet — measured on a
        // BRD2709A over SWD, `_I2C_STATE_RESETVALUE` 0x00000001. It clears once
        // the controller learns where the bus is, which is what emlib's opening
        // ABORT is for.
        assert_eq!(
            i2c.read_u32(EFR_I2C_STATE).unwrap() & EFR_STATE_BUSY,
            EFR_STATE_BUSY,
            "power-on BUSY, before any ABORT"
        );
        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_ABORT).unwrap();
        assert_eq!(i2c.read_u32(EFR_I2C_STATE).unwrap() & EFR_STATE_BUSY, 0);

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        let state = i2c.read_u32(EFR_I2C_STATE).unwrap();
        assert_eq!(state & EFR_STATE_BUSY, EFR_STATE_BUSY);
        assert_eq!(state & EFR_STATE_MASTER, EFR_STATE_MASTER);

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_STOP).unwrap();
        assert_eq!(i2c.read_u32(EFR_I2C_STATE).unwrap() & EFR_STATE_BUSY, 0);
    }

    /// A slave must see the framing, not just the bytes: a sensor that latches
    /// on STOP (most of them) never commits if the controller does not deliver
    /// one.
    #[test]
    fn start_and_stop_reach_the_slave() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        #[derive(Debug)]
        struct Counting {
            starts: Arc<AtomicUsize>,
            stops: Arc<AtomicUsize>,
        }
        impl I2cDevice for Counting {
            fn address(&self) -> u8 {
                0x48
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, _data: u8) {}
            fn start(&mut self) {
                self.starts.fetch_add(1, Ordering::Relaxed);
            }
            fn stop(&mut self) {
                self.stops.fetch_add(1, Ordering::Relaxed);
            }
        }

        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut i2c = enabled();
        i2c.push_slave(Box::new(Counting {
            starts: starts.clone(),
            stops: stops.clone(),
        }));

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, false))
            .unwrap();
        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_STOP).unwrap();

        assert_eq!(starts.load(Ordering::Relaxed), 1);
        assert_eq!(stops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_flag_register_is_write_one_to_clear_and_ien_gates_the_irq() {
        let mut i2c = enabled();
        i2c.push_slave(Box::new(FakeSensor::new(0x48)));
        clear_flags(&mut i2c);

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        assert_eq!(flags(&i2c) & EFR_IF_START, EFR_IF_START);
        assert!(!i2c.tick().irq, "IEN clear: no interrupt");

        i2c.write_u32(EFR_I2C_IEN, EFR_IF_START).unwrap();
        assert!(i2c.tick().irq);

        i2c.write_u32(EFR_I2C_IF, EFR_IF_START).unwrap();
        assert_eq!(flags(&i2c) & EFR_IF_START, 0);
        assert!(!i2c.tick().irq);
    }

    /// A word write of CMD must apply the whole word at once. Byte-slicing it
    /// would apply START and STOP as two separate commands.
    #[test]
    fn a_word_write_of_cmd_is_one_command_not_four_bytes() {
        let mut i2c = enabled();
        i2c.push_slave(Box::new(FakeSensor::new(0x48)));
        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
        i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, true))
            .unwrap();
        clear_flags(&mut i2c);

        i2c.write_u32(EFR_I2C_CMD, EFR_CMD_NACK | EFR_CMD_STOP)
            .unwrap();
        assert_eq!(flags(&i2c) & EFR_IF_MSTOP, EFR_IF_MSTOP);
        assert_eq!(i2c.read_u32(EFR_I2C_STATE).unwrap() & EFR_STATE_BUSY, 0);
    }
}
