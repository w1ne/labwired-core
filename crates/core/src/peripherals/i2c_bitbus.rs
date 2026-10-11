// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A bit-level I²C wire engine for a controller whose SCL/SDA pads sit on a
//! world `gpio_net`, shared by register models that do not carry their own
//! (today the STM32 F1/F2/F4 legacy I²C, `i2c_legacy_net`).
//!
//! The engine owns the wire and nothing else: open-drain drive of SCL and
//! SDA, START/STOP generation and detection, bit timing, the ACK slot,
//! arbitration and clock stretching. Every register-level decision belongs
//! to the host model, which drives the engine with commands and reacts to the
//! [`BusEvent`]s it reports:
//!
//! * **Controller.** [`BitBus::m_start`] puts START (or a repeated START from
//!   a held bus) on the wire; [`BitBus::m_send`] clocks a byte out and reports
//!   the ACK slot; [`BitBus::m_recv`] clocks 8 bits in and stops before the
//!   ACK slot so the host can decide it ([`BitBus::m_ack`]);
//!   [`BitBus::m_stop`] sends STOP. Between commands the controller holds SCL
//!   low, which is what a controller waiting for its firmware does on
//!   silicon. The SCL high period is counted from the moment SCL is *seen*
//!   high at the pad, so a target stretching the clock delays the bit. SDA is
//!   sampled at the end of the high period; a released 1 read back as 0 while
//!   sending is lost arbitration.
//! * **Target.** While its controller side is idle the engine watches the
//!   edges the net delivers: START/STOP (SDA moving while SCL is high),
//!   address and data bits on SCL rising edges. At each point a target
//!   decides something (ACK its address, ACK a byte, the next byte to send,
//!   go on after the address) it reports an event and waits. If the host has
//!   not answered by the time it finishes handling the edge
//!   ([`BitBus::settle`]), the engine holds SCL low (clock stretching) until
//!   it does.
//!
//! Timed steps run at their exact cycle from [`BitBus::deadline`]; edges run
//! when the net delivers them. Nothing is polled here.

use crate::peripherals::pad_lines::{LineDrive, PadLines};

/// Line indices, as in `I2C_LINES`.
pub(crate) const SCL: usize = 0;
pub(crate) const SDA: usize = 1;

/// Bus timing in engine cycles.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timing {
    /// SCL low period.
    pub low: u64,
    /// SCL high period, counted from when SCL is seen high.
    pub high: u64,
    /// Delay from SCL falling to the controller changing SDA (data hold).
    pub sda_delay: u64,
}

impl Timing {
    /// Clamp to a period the engine can step through.
    pub(crate) fn new(low: u64, high: u64, sda_delay: u64) -> Self {
        let low = low.max(4);
        Self {
            low,
            high: high.max(2),
            sda_delay: sda_delay.clamp(1, low - 2),
        }
    }
}

/// What the wire did, for the host to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BusEvent {
    /// START (or repeated START) is on the wire and SCL is held low.
    MStarted,
    /// A byte went out; the ACK slot read `acked`. SCL is held low.
    MSent { acked: bool },
    /// Eight bits came in. SCL is held low until [`BitBus::m_ack`].
    MByteIn(u8),
    /// The ACK slot of a received byte is done. SCL is held low.
    MAckDone,
    /// STOP is on the wire; the controller let go of the bus.
    MStopped,
    /// Another controller won the bus; this one let go of it.
    MArbLost,
    /// START or repeated START seen on the bus.
    TStart,
    /// STOP seen on the bus.
    TStop,
    /// An address byte came in (address << 1 | R/W). Answer with
    /// [`BitBus::t_ack`].
    TAddress(u8),
    /// This target ACKed its address and the ACK slot is over. Answer with
    /// [`BitBus::t_continue`] (written to) or [`BitBus::t_send`] (read
    /// from).
    TAddressed { read: bool },
    /// A data byte came in. Answer with [`BitBus::t_ack`].
    TByteIn(u8),
    /// The controller ACKed (`true`) or NACKed the byte this target sent.
    /// After an ACK, answer with [`BitBus::t_send`].
    TByteOut { acked: bool },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum MPhase {
    #[default]
    Idle,
    /// Waiting for both lines to be high before START.
    WaitBus,
    /// SDA low (START); SCL falls at the deadline.
    StartHold,
    /// SCL low; SDA takes its value at the deadline.
    LowSetup,
    /// SCL low; released at the deadline.
    LowWait,
    /// SCL released, waiting to see it high.
    WaitHigh,
    /// SCL high until the deadline.
    High,
    /// SCL held low, waiting for the host's next command.
    Hold,
}

/// What the clock pulse in progress is for.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Job {
    #[default]
    None,
    Send(u8),
    Recv,
    Ack(bool),
    Stop,
    Restart,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum TPhase {
    #[default]
    Idle,
    Addr,
    Rx,
    Tx,
    /// Not addressed, or done: wait for STOP or START.
    Ignore,
}

/// A decision the target is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TAwait {
    AddrAck,
    Continue { read: bool },
    RxAck,
    TxByte,
}

/// The engine state. See the module docs.
#[derive(Debug, Default)]
pub(crate) struct BitBus {
    pub(crate) timing: Timing,
    m: MPhase,
    job: Job,
    /// Bit within the job: 0..=7 data, 8 the ACK slot of a send.
    bit: u8,
    rx: u8,
    deadline: Option<u64>,
    t: TPhase,
    t_bits: u8,
    t_shift: u8,
    t_tx: u8,
    t_acked_addr: bool,
    t_master_acked: bool,
    t_await: Option<TAwait>,
    /// This target is holding SCL low.
    t_stretch: bool,
    /// The cycle the stretch ends: after a data setup time when the answer
    /// changed SDA, so the change is on the wire before SCL rises.
    t_release: Option<u64>,
    /// Levels last seen at the pads (`None` before the first one).
    scl: Option<bool>,
    sda: Option<bool>,
    events: Vec<BusEvent>,
}

impl BitBus {
    fn drive(pads: &PadLines, line: usize, level: bool) {
        pads.drive_line(line, LineDrive::OpenDrain, level);
    }

    fn wire(pads: &PadLines, line: usize) -> bool {
        pads.wire_level(line)
    }

    /// Events since the last call, oldest first.
    pub(crate) fn take_events(&mut self) -> Vec<BusEvent> {
        std::mem::take(&mut self.events)
    }

    /// The cycle of the next timed step, if one is due.
    pub(crate) fn deadline(&self) -> Option<u64> {
        match (self.deadline, self.t_release) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// `true` while the controller side owns the bus (START sent, STOP not
    /// yet).
    pub(crate) fn master_active(&self) -> bool {
        !matches!(self.m, MPhase::Idle | MPhase::WaitBus)
    }

    /// `true` while the controller holds SCL low waiting for a command.
    pub(crate) fn master_holding(&self) -> bool {
        self.m == MPhase::Hold
    }

    /// Release both lines and forget everything (peripheral disabled).
    pub(crate) fn reset(&mut self, pads: &PadLines) {
        let timing = self.timing;
        let (scl, sda) = (self.scl, self.sda);
        *self = Self {
            timing,
            scl,
            sda,
            ..Self::default()
        };
        Self::drive(pads, SCL, true);
        Self::drive(pads, SDA, true);
    }

    // ── controller ──────────────────────────────────────────────────────

    /// START, or a repeated START from a held bus. Waits for a free bus
    /// (both lines high) when another controller has it.
    pub(crate) fn m_start(&mut self, now: u64, pads: &PadLines) {
        match self.m {
            MPhase::Hold => self.begin_low(now, Job::Restart, pads),
            MPhase::Idle | MPhase::WaitBus => {
                if Self::wire(pads, SCL) && Self::wire(pads, SDA) {
                    // Cancel any target-side involvement: this chip is the
                    // controller now.
                    self.t = TPhase::Idle;
                    Self::drive(pads, SDA, false);
                    self.m = MPhase::StartHold;
                    self.deadline = Some(now + self.timing.high);
                } else {
                    self.m = MPhase::WaitBus;
                }
            }
            _ => {}
        }
    }

    /// Clock `byte` out and read the ACK slot. Only from a held bus.
    pub(crate) fn m_send(&mut self, byte: u8, now: u64, pads: &PadLines) -> bool {
        if self.m != MPhase::Hold {
            return false;
        }
        self.bit = 0;
        self.begin_low(now, Job::Send(byte), pads);
        true
    }

    /// Clock 8 bits in. Only from a held bus.
    pub(crate) fn m_recv(&mut self, now: u64, pads: &PadLines) -> bool {
        if self.m != MPhase::Hold {
            return false;
        }
        self.bit = 0;
        self.rx = 0;
        self.begin_low(now, Job::Recv, pads);
        true
    }

    /// The ACK slot after [`BusEvent::MByteIn`]: ACK (`true`) or NACK.
    pub(crate) fn m_ack(&mut self, ack: bool, now: u64, pads: &PadLines) -> bool {
        if self.m != MPhase::Hold {
            return false;
        }
        self.begin_low(now, Job::Ack(ack), pads);
        true
    }

    /// STOP from a held bus.
    pub(crate) fn m_stop(&mut self, now: u64, pads: &PadLines) -> bool {
        if self.m != MPhase::Hold {
            return false;
        }
        self.begin_low(now, Job::Stop, pads);
        true
    }

    /// Pull SCL low (it may already be) and start the low half of a pulse.
    fn begin_low(&mut self, now: u64, job: Job, pads: &PadLines) {
        Self::drive(pads, SCL, false);
        self.job = job;
        self.m = MPhase::LowSetup;
        self.deadline = Some(now + self.timing.sda_delay);
    }

    /// The SDA level this controller puts out for the pulse in progress.
    fn sda_out(&self) -> bool {
        match self.job {
            Job::Send(byte) if self.bit < 8 => (byte >> (7 - self.bit)) & 1 != 0,
            Job::Send(_) | Job::Recv | Job::None | Job::Restart => true,
            Job::Ack(ack) => !ack,
            Job::Stop => false,
        }
    }

    /// Run every timed step due by `now`, each at its own cycle.
    pub(crate) fn step(&mut self, now: u64, pads: &PadLines) {
        while let Some(due) = self.deadline() {
            if due > now {
                break;
            }
            pads.set_stamp(Some(due));
            if self.t_release.is_some_and(|t| t <= due) {
                self.t_release = None;
                if std::mem::take(&mut self.t_stretch) {
                    Self::drive(pads, SCL, true);
                }
            }
            if self.deadline.is_some_and(|t| t <= due) {
                self.deadline = None;
                self.step_at(due, pads);
            }
        }
        pads.set_stamp(None);
    }

    fn step_at(&mut self, now: u64, pads: &PadLines) {
        let t = self.timing;
        match self.m {
            MPhase::StartHold => {
                Self::drive(pads, SCL, false);
                self.m = MPhase::Hold;
                self.events.push(BusEvent::MStarted);
            }
            MPhase::LowSetup => {
                Self::drive(pads, SDA, self.sda_out());
                self.m = MPhase::LowWait;
                self.deadline = Some(now + t.low.saturating_sub(t.sda_delay).max(1));
            }
            MPhase::LowWait => {
                Self::drive(pads, SCL, true);
                self.m = MPhase::WaitHigh;
                self.check_high(now, pads);
            }
            MPhase::High => self.end_high(now, pads),
            _ => {}
        }
    }

    /// SCL released: once it is seen high, the high period starts.
    fn check_high(&mut self, now: u64, pads: &PadLines) {
        if self.m == MPhase::WaitHigh && Self::wire(pads, SCL) {
            self.m = MPhase::High;
            self.deadline = Some(now + self.timing.high);
        }
    }

    fn hold(&mut self, pads: &PadLines, event: BusEvent) {
        Self::drive(pads, SCL, false);
        self.m = MPhase::Hold;
        self.job = Job::None;
        self.events.push(event);
    }

    fn end_high(&mut self, now: u64, pads: &PadLines) {
        let sda = Self::wire(pads, SDA);
        match self.job {
            Job::Stop => {
                // SDA rises while SCL is high: STOP.
                Self::drive(pads, SDA, true);
                self.m = MPhase::Idle;
                self.job = Job::None;
                self.events.push(BusEvent::MStopped);
            }
            Job::Restart => {
                // SDA falls while SCL is high: repeated START.
                Self::drive(pads, SDA, false);
                self.m = MPhase::StartHold;
                self.deadline = Some(now + self.timing.high);
            }
            Job::Send(_) if self.bit < 8 => {
                if self.sda_out() && !sda {
                    // A released 1 read back as 0: someone else won.
                    Self::drive(pads, SDA, true);
                    Self::drive(pads, SCL, true);
                    self.m = MPhase::Idle;
                    self.job = Job::None;
                    self.events.push(BusEvent::MArbLost);
                    return;
                }
                self.bit += 1;
                self.begin_low(now, self.job, pads);
            }
            Job::Send(_) => self.hold(pads, BusEvent::MSent { acked: !sda }),
            Job::Recv => {
                self.rx = (self.rx << 1) | u8::from(sda);
                self.bit += 1;
                if self.bit < 8 {
                    self.begin_low(now, Job::Recv, pads);
                } else {
                    let byte = self.rx;
                    self.hold(pads, BusEvent::MByteIn(byte));
                }
            }
            Job::Ack(_) => self.hold(pads, BusEvent::MAckDone),
            Job::None => {}
        }
    }

    // ── target ──────────────────────────────────────────────────────────

    /// ACK (`true`) or NACK the address or data byte just reported.
    pub(crate) fn t_ack(&mut self, ack: bool, now: u64, pads: &PadLines) {
        match self.t_await {
            Some(TAwait::AddrAck) => {
                self.t_await = None;
                if ack {
                    self.t_acked_addr = true;
                    Self::drive(pads, SDA, false);
                } else {
                    self.t = TPhase::Ignore;
                }
            }
            Some(TAwait::RxAck) => {
                self.t_await = None;
                Self::drive(pads, SDA, !ack);
                if !ack {
                    // A NACKed byte ends the transfer for this target.
                    self.t = TPhase::Ignore;
                }
            }
            _ => return,
        }
        self.release_stretch(now, pads);
    }

    /// Go on receiving after [`BusEvent::TAddressed`] with `read: false`.
    pub(crate) fn t_continue(&mut self, now: u64, pads: &PadLines) {
        if matches!(self.t_await, Some(TAwait::Continue { read: false })) {
            self.t_await = None;
            self.release_stretch(now, pads);
        }
    }

    /// Send `byte` next (after [`BusEvent::TAddressed`] with `read: true`, or
    /// after an ACKed [`BusEvent::TByteOut`]).
    pub(crate) fn t_send(&mut self, byte: u8, now: u64, pads: &PadLines) {
        if !matches!(
            self.t_await,
            Some(TAwait::TxByte | TAwait::Continue { read: true })
        ) {
            return;
        }
        self.t_await = None;
        self.t_tx = byte;
        Self::drive(pads, SDA, byte & 0x80 != 0);
        self.release_stretch(now, pads);
    }

    /// `true` while the target waits for its host to decide something.
    #[cfg(test)]
    pub(crate) fn target_waiting(&self) -> bool {
        self.t_await.is_some()
    }

    /// End a stretch, a data setup time after the answer went onto SDA:
    /// SCL must not rise in the cycle SDA changes, or the change would read
    /// as START or STOP on the wire.
    fn release_stretch(&mut self, now: u64, pads: &PadLines) {
        let _ = pads;
        if self.t_stretch && self.t_release.is_none() {
            self.t_release = Some(now + self.timing.sda_delay.max(1));
        }
    }

    /// After the host has handled the events of an edge: a target still
    /// waiting for a decision holds SCL low.
    pub(crate) fn settle(&mut self, pads: &PadLines) {
        if self.t_await.is_some() && !self.t_stretch && !self.master_active() {
            self.t_stretch = true;
            Self::drive(pads, SCL, false);
        }
    }

    /// The net delivered a new level on SCL or SDA at this chip's pad.
    pub(crate) fn on_edge(&mut self, line: usize, level: bool, now: u64, pads: &PadLines) {
        let prev_scl = self.scl;
        let prev_sda = self.sda;
        match line {
            SCL => self.scl = Some(level),
            SDA => self.sda = Some(level),
            _ => return,
        }
        // Controller: a released SCL seen high starts the high period; a
        // free bus lets a waiting START go.
        self.check_high(now, pads);
        if self.m == MPhase::WaitBus && Self::wire(pads, SCL) && Self::wire(pads, SDA) {
            self.m = MPhase::Idle;
            self.m_start(now, pads);
        }
        if self.master_active() {
            return;
        }
        if line == SDA && prev_scl == Some(true) && prev_sda.is_some() && prev_sda != Some(level) {
            self.t_stretch = false;
            self.t_release = None;
            self.t_await = None;
            if level {
                self.t = TPhase::Idle;
                self.t_acked_addr = false;
                Self::drive(pads, SDA, true);
                Self::drive(pads, SCL, true);
                self.events.push(BusEvent::TStop);
            } else {
                self.t = TPhase::Addr;
                self.t_acked_addr = false;
                self.t_bits = 0;
                self.t_shift = 0;
                self.events.push(BusEvent::TStart);
            }
        } else if line == SCL && prev_scl.is_some() && prev_scl != Some(level) {
            if level {
                self.scl_rise();
            } else {
                self.scl_fall(pads);
            }
        }
    }

    fn scl_rise(&mut self) {
        let sda = self.sda.unwrap_or(true);
        match self.t {
            TPhase::Addr | TPhase::Rx if self.t_bits < 8 => {
                self.t_shift = (self.t_shift << 1) | u8::from(sda);
                self.t_bits += 1;
            }
            TPhase::Tx if self.t_bits == 8 => {
                self.t_master_acked = !sda;
                self.t_bits += 1;
            }
            TPhase::Addr | TPhase::Rx | TPhase::Tx => self.t_bits += 1,
            _ => {}
        }
    }

    fn scl_fall(&mut self, pads: &PadLines) {
        match (self.t, self.t_bits) {
            (TPhase::Addr, 8) => {
                self.t_await = Some(TAwait::AddrAck);
                self.events.push(BusEvent::TAddress(self.t_shift));
            }
            (TPhase::Addr, 9) => {
                Self::drive(pads, SDA, true);
                let read = self.t_shift & 1 != 0;
                self.t = if read { TPhase::Tx } else { TPhase::Rx };
                self.t_bits = 0;
                self.t_shift = 0;
                self.t_await = Some(TAwait::Continue { read });
                self.events.push(BusEvent::TAddressed { read });
            }
            (TPhase::Rx, 8) => {
                self.t_await = Some(TAwait::RxAck);
                self.events.push(BusEvent::TByteIn(self.t_shift));
            }
            (TPhase::Rx, 9) => {
                Self::drive(pads, SDA, true);
                self.t_bits = 0;
                self.t_shift = 0;
            }
            (TPhase::Tx, b @ 1..=7) => Self::drive(pads, SDA, (self.t_tx >> (7 - b)) & 1 != 0),
            (TPhase::Tx, 8) => Self::drive(pads, SDA, true),
            (TPhase::Tx, 9) => {
                self.t_bits = 0;
                let acked = self.t_master_acked;
                if acked {
                    self.t_await = Some(TAwait::TxByte);
                } else {
                    self.t = TPhase::Ignore;
                }
                self.events.push(BusEvent::TByteOut { acked });
            }
            (TPhase::Ignore, 9) => {
                // A NACKed data byte's ACK slot is over: let SDA go.
                Self::drive(pads, SDA, true);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: &[&str] = &["SCL", "SDA"];

    fn pads() -> PadLines {
        let p = PadLines::new(LINES, &[true, true]);
        p.set_mode(SCL, LineDrive::OpenDrain);
        p.set_mode(SDA, LineDrive::OpenDrain);
        p
    }

    /// Two engines on one wired-AND bus with zero latency: every drive is
    /// resolved and delivered to both at once.
    struct Pair {
        a: BitBus,
        b: BitBus,
        pa: PadLines,
        pb: PadLines,
        level: [bool; 2],
        now: u64,
    }

    impl Pair {
        fn new() -> Self {
            let mut p = Self {
                a: BitBus::default(),
                b: BitBus::default(),
                pa: pads(),
                pb: pads(),
                level: [true, true],
                now: 0,
            };
            p.a.timing = Timing::new(8, 8, 2);
            p.b.timing = p.a.timing;
            for line in [SCL, SDA] {
                p.pa.set_input(line, true);
                p.pb.set_input(line, true);
                p.a.on_edge(line, true, 0, &p.pa);
                p.b.on_edge(line, true, 0, &p.pb);
            }
            p
        }

        /// Resolve the wire and deliver changes until nothing moves.
        fn settle(&mut self) {
            loop {
                let mut moved = false;
                for line in [SCL, SDA] {
                    let level = !(self.pa.drives(line) || self.pb.drives(line));
                    if level != self.level[line] {
                        self.level[line] = level;
                        moved = true;
                        self.pa.set_input(line, level);
                        self.pb.set_input(line, level);
                        self.a.on_edge(line, level, self.now, &self.pa);
                        self.b.on_edge(line, level, self.now, &self.pb);
                        self.b.settle(&self.pb);
                    }
                }
                if !moved {
                    return;
                }
            }
        }

        /// Advance `a` until it reports something or `b` waits for a
        /// decision. Returns the events of both sides.
        fn run(&mut self) -> (Vec<BusEvent>, Vec<BusEvent>) {
            let mut ea = Vec::new();
            let mut eb = Vec::new();
            for _ in 0..10_000 {
                self.settle();
                ea.extend(self.a.take_events());
                eb.extend(self.b.take_events());
                if !ea.is_empty() || self.b.target_waiting() {
                    return (ea, eb);
                }
                let due = match (self.a.deadline(), self.b.deadline()) {
                    (Some(x), Some(y)) => x.min(y),
                    (Some(x), None) | (None, Some(x)) => x,
                    (None, None) => return (ea, eb),
                };
                self.now = due;
                self.a.step(due, &self.pa);
                self.b.step(due, &self.pb);
            }
            panic!("runaway");
        }
    }

    #[test]
    fn a_controller_addresses_a_target_and_writes_a_byte() {
        let mut p = Pair::new();
        p.a.m_start(0, &p.pa);
        assert_eq!(p.run(), (vec![BusEvent::MStarted], vec![BusEvent::TStart]));
        assert!(p.a.m_send(0x84, p.now, &p.pa)); // 0x42, write
        assert_eq!(p.run(), (vec![], vec![BusEvent::TAddress(0x84)]));
        p.b.t_ack(true, p.now, &p.pb);
        assert_eq!(
            p.run(),
            (
                vec![BusEvent::MSent { acked: true }],
                vec![BusEvent::TAddressed { read: false }]
            )
        );
        // The target stretches SCL until its host goes on.
        assert!(p.pb.drives(SCL));
        p.a.m_send(0x5A, p.now, &p.pa);
        assert_eq!(p.run(), (vec![], vec![]));
        assert!(!p.level[SCL], "held low by the target");
        p.b.t_continue(p.now, &p.pb);
        assert_eq!(p.run(), (vec![], vec![BusEvent::TByteIn(0x5A)]));
        p.b.t_ack(false, p.now, &p.pb);
        assert_eq!(p.run(), (vec![BusEvent::MSent { acked: false }], vec![]));
        p.a.m_stop(p.now, &p.pa);
        // The controller's own target side sees the STOP too.
        assert_eq!(
            p.run(),
            (
                vec![BusEvent::MStopped, BusEvent::TStop],
                vec![BusEvent::TStop]
            )
        );
        assert!(p.level[SCL] && p.level[SDA]);
    }

    #[test]
    fn a_controller_reads_what_the_target_sends() {
        let mut p = Pair::new();
        p.a.m_start(0, &p.pa);
        p.run();
        p.a.m_send(0x85, p.now, &p.pa); // 0x42, read
        assert_eq!(p.run(), (vec![], vec![BusEvent::TAddress(0x85)]));
        p.b.t_ack(true, p.now, &p.pb);
        assert_eq!(
            p.run(),
            (
                vec![BusEvent::MSent { acked: true }],
                vec![BusEvent::TAddressed { read: true }]
            )
        );
        p.a.m_recv(p.now, &p.pa);
        assert_eq!(p.run(), (vec![], vec![]));
        p.b.t_send(0xC3, p.now, &p.pb);
        assert_eq!(p.run(), (vec![BusEvent::MByteIn(0xC3)], vec![]));
        p.a.m_ack(false, p.now, &p.pa);
        assert_eq!(
            p.run(),
            (
                vec![BusEvent::MAckDone],
                vec![BusEvent::TByteOut { acked: false }]
            )
        );
        p.a.m_stop(p.now, &p.pa);
        // The controller's own target side sees the STOP too.
        assert_eq!(
            p.run(),
            (
                vec![BusEvent::MStopped, BusEvent::TStop],
                vec![BusEvent::TStop]
            )
        );
    }
}
