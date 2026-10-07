// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The STM32 modern I²C controller (L4/G0 register file) on a world
//! `gpio_net`: a bit-level controller and target between chips.
//!
//! Off a net the controller in `i2c.rs` is a phase model: it exchanges whole
//! bytes with attached device models and narrates the waveform afterwards.
//! That cannot work across chips, where the other side's ACK, its data and
//! its clock stretching are only known when they arrive on the wire. So when
//! a pad that can carry SCL or SDA is put on a net ([`PadLines::on_net`]) the
//! controller switches to this engine:
//!
//! * **Open drain.** Both lines are [`LineDrive::OpenDrain`]: published low
//!   pulls the wire down, published high releases it; the net's pull-up does
//!   the rest. A line the controller releases reads what the wire holds.
//! * **Controller (master).** CR2.START runs START, the 7-bit address and
//!   R/W, then NBYTES data bytes, with SCL low for `(SCLL+1)` and high for
//!   `(SCLH+1)` prescaled periods (TIMINGR) and SDA changed `SDADEL` after
//!   SCL falls. The high period is counted from the moment SCL is *seen*
//!   high at the pad, so a target holding SCL low (clock stretching) simply
//!   delays the bit, as on silicon. SDA is sampled at the end of the high
//!   period: the ACK bit, read data, and arbitration (a released 1 read back
//!   as 0 is ARLO). A NACK sets NACKF and sends STOP. NBYTES reached:
//!   AUTOEND sends STOP (STOPF), otherwise TC holds SCL low until firmware
//!   writes START (repeated start) or STOP.
//! * **Target.** With OAR1.OA1EN set the controller watches the wire edges
//!   the net delivers ([`crate::Peripheral::wire_input_edge`]): START/STOP
//!   (SDA moving while SCL is high), address bits on SCL rising edges, its
//!   own 7-bit address ACKed with ADDR/DIR/ADDCODE set, and SCL held low
//!   (stretched) until firmware clears ADDR, reads RXDR or writes TXDR, as
//!   RM0444 describes with NOSTRETCH=0. A different address is not ACKed.
//!
//! Everything is event-scheduled: timed steps run at their exact cycle, edges
//! run when the net delivers them. The one poll is while SCL is stretched
//! waiting for firmware to read RXDR (a register read cannot wake the model),
//! at a quarter of the SCL low period.
//!
//! Not modelled on a net: 10-bit addressing, OAR2, general call, RELOAD
//! (NBYTES above 255), SMBus/PEC, NOSTRETCH=1, the analog/digital filters and
//! timeouts. A transfer the engine cannot do is refused by doing nothing
//! (the controller stays idle) rather than by inventing a waveform.

use std::cell::Cell;

use super::{L4I2c, LINE_SCL, LINE_SDA};
use crate::peripherals::pad_lines::{LineDrive, PadLines};

const CR1_PE: u32 = 1 << 0;
const CR1_TXIE: u32 = 1 << 1;
const CR1_RXIE: u32 = 1 << 2;
const CR1_ADDRIE: u32 = 1 << 3;
const CR1_NACKIE: u32 = 1 << 4;
const CR1_STOPIE: u32 = 1 << 5;
const CR1_TCIE: u32 = 1 << 6;
const CR1_ERRIE: u32 = 1 << 7;

const CR2_RD_WRN: u32 = 1 << 10;
const CR2_START: u32 = 1 << 13;
const CR2_STOP: u32 = 1 << 14;
const CR2_AUTOEND: u32 = 1 << 25;

const OAR1_EN: u32 = 1 << 15;

pub(super) const ISR_TXE: u32 = 1 << 0;
pub(super) const ISR_TXIS: u32 = 1 << 1;
pub(super) const ISR_RXNE: u32 = 1 << 2;
const ISR_ADDR: u32 = 1 << 3;
const ISR_NACKF: u32 = 1 << 4;
const ISR_STOPF: u32 = 1 << 5;
const ISR_TC: u32 = 1 << 6;
const ISR_ARLO: u32 = 1 << 9;
const ISR_BUSY: u32 = 1 << 15;
const ISR_DIR: u32 = 1 << 16;
const ISR_ADDCODE_SHIFT: u32 = 17;

/// Core cycles per I²C kernel-clock period: the same reset-default multiple
/// the phase model uses (`L4I2c::address_phase_cycles`).
const CORE_PER_KCLK: u64 = 8;

/// What the clock pulse in progress is for.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Pulse {
    #[default]
    Bit,
    Stop,
    Restart,
}

/// Where the controller (master) is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum MPhase {
    #[default]
    Idle,
    /// Waiting for the bus to be free before START.
    WaitBus,
    /// SDA is low (START); SCL falls at the deadline.
    StartHold,
    /// SCL is low; SDA takes its value at the deadline (SDADEL).
    LowSetup,
    /// SCL is low; it is released at the deadline (end of SCLL).
    LowWait,
    /// SCL released; waiting to see it high at the pad.
    WaitHigh,
    /// SCL high; the end of the high period is the deadline.
    High,
    /// SCL held low, waiting for firmware to fill TXDR.
    WaitTxdr,
    /// SCL held low, waiting for firmware to read RXDR (polled).
    WaitRxRead,
    /// SCL held low after NBYTES (TC): waiting for START or STOP.
    WaitCmd,
}

/// Where the target (slave) is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum TPhase {
    #[default]
    Idle,
    Addr,
    Rx,
    Tx,
    /// Addressed someone else, or NACKed: wait for STOP or a new START.
    Ignore,
}

/// Why the target is holding SCL low.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Stretch {
    #[default]
    None,
    Addr,
    Txdr,
    RxRead,
}

/// The net engine's state, kept beside the phase model's.
#[derive(Debug, Default)]
pub(super) struct I2cNet {
    // ── controller ──
    m: MPhase,
    pulse: Pulse,
    deadline: Option<u64>,
    /// Bit within the byte, 0..=7 data (MSB first), 8 the ACK slot.
    bit: u8,
    /// The byte being sent, `None` while receiving.
    tx: Option<u8>,
    rx: u8,
    /// The byte in flight is the address byte.
    addr_phase: bool,
    reading: bool,
    count: u32,
    nbytes: u32,
    autoend: bool,
    // ── target ──
    t: TPhase,
    t_bits: u8,
    t_shift: u8,
    t_reading: bool,
    /// This target ACKed its address since the last START.
    t_addressed: bool,
    t_master_acked: bool,
    t_tx: u8,
    stretch: Stretch,
    /// Levels last seen at the pads (`None` before the first one).
    scl: Option<bool>,
    sda: Option<bool>,
    // ── plumbing ──
    /// Firmware read RXDR since the engine last looked (reads are `&self`).
    pub(super) rx_taken: Cell<bool>,
    irq_pending: bool,
    /// Event deadlines already armed with the scheduler.
    armed: Vec<u64>,
}

impl L4I2c {
    fn net_pads(&self) -> Option<&PadLines> {
        self.lines.as_deref()
    }

    /// `true` when this controller shares its SCL/SDA with other chips
    /// through a world `gpio_net`.
    pub(super) fn net_mode(&self) -> bool {
        self.net_pads().is_some_and(PadLines::any_on_net)
    }

    fn net_now(&self) -> u64 {
        self.clock.as_ref().map_or(0, crate::CycleClock::now)
    }

    /// (t_low, t_high, sdadel) in core cycles, from TIMINGR.
    fn net_timing(&self) -> (u64, u64, u64) {
        let t = self.timingr;
        let presc = u64::from((t >> 28) & 0xF) + 1;
        let scll = u64::from(t & 0xFF) + 1;
        let sclh = u64::from((t >> 8) & 0xFF) + 1;
        let sdadel = u64::from((t >> 16) & 0xF);
        let tp = presc * CORE_PER_KCLK;
        let t_low = (scll * tp).max(4);
        let sdadel = (sdadel * tp).clamp(1, t_low - 2);
        (t_low, (sclh * tp).max(2), sdadel)
    }

    fn net_flag(&mut self, flag: u32) {
        self.isr |= flag;
        let enable = match flag {
            ISR_TXIS => CR1_TXIE,
            ISR_RXNE => CR1_RXIE,
            ISR_ADDR => CR1_ADDRIE,
            ISR_NACKF => CR1_NACKIE,
            ISR_STOPF => CR1_STOPIE,
            ISR_TC => CR1_TCIE,
            ISR_ARLO => CR1_ERRIE,
            _ => 0,
        };
        if self.cr1 & enable != 0 {
            self.net.irq_pending = true;
        }
    }

    fn net_drive(&self, line: usize, level: bool) {
        if let Some(pads) = self.net_pads() {
            pads.drive_line(line, LineDrive::OpenDrain, level);
        }
    }

    /// The level the wire holds at this chip's pad.
    fn net_wire(&self, line: usize) -> bool {
        self.net_pads().is_none_or(|p| p.wire_level(line))
    }

    /// Fold a firmware RXDR read into ISR.
    fn net_sync_rx(&mut self) {
        if self.net.rx_taken.replace(false) {
            self.isr &= !ISR_RXNE;
        }
    }

    // ── register side ───────────────────────────────────────────────────

    /// Register writes in net mode. Returns `true` when handled here.
    pub(super) fn net_write_reg(&mut self, offset: u64, value: u32) -> bool {
        self.net_sync_rx();
        match offset {
            0x00 => {
                let was = self.cr1 & CR1_PE != 0;
                self.cr1 = value & 0x00FF_E1FF;
                if was && self.cr1 & CR1_PE == 0 {
                    self.net_reset();
                }
                true
            }
            0x04 => {
                self.cr2 = value & !(CR2_START | CR2_STOP);
                if value & CR2_START != 0 && self.cr1 & CR1_PE != 0 {
                    self.cr2 |= CR2_START;
                    self.net_master_start();
                } else if value & CR2_STOP != 0 && self.net.m == MPhase::WaitCmd {
                    let now = self.net_now();
                    self.net_begin_low(now, Pulse::Stop);
                }
                true
            }
            0x08 => {
                self.oar1 = value;
                true
            }
            0x1C => {
                // ICR: ADDRCF 3, NACKCF 4, STOPCF 5, BERRCF 8, ARLOCF 9, OVRCF 10.
                self.isr &= !(value & 0x0000_3F38);
                if value & ISR_ADDR != 0 && self.net.stretch == Stretch::Addr {
                    self.net_target_after_addr();
                }
                true
            }
            0x28 => {
                self.txdr = value & 0xFF;
                self.isr &= !(ISR_TXE | ISR_TXIS);
                let now = self.net_now();
                if self.net.m == MPhase::WaitTxdr {
                    self.net_master_next_tx(now);
                } else if self.net.stretch == Stretch::Txdr {
                    self.net.stretch = Stretch::None;
                    self.net_target_load_tx();
                    self.net_drive(LINE_SCL, true);
                }
                true
            }
            _ => false,
        }
    }

    fn net_reset(&mut self) {
        self.isr = ISR_TXE;
        self.net.m = MPhase::Idle;
        self.net.t = TPhase::Idle;
        self.net.stretch = Stretch::None;
        self.net.deadline = None;
        self.net_drive(LINE_SCL, true);
        self.net_drive(LINE_SDA, true);
    }

    // ── controller ──────────────────────────────────────────────────────

    fn net_master_start(&mut self) {
        let now = self.net_now();
        self.net.reading = self.cr2 & CR2_RD_WRN != 0;
        self.net.nbytes = (self.cr2 >> 16) & 0xFF;
        self.net.autoend = self.cr2 & CR2_AUTOEND != 0;
        self.net.count = 0;
        self.isr &= !(ISR_TC | ISR_NACKF);
        match self.net.m {
            // Repeated START from a TC hold: SCL is low.
            MPhase::WaitCmd => self.net_begin_low(now, Pulse::Restart),
            MPhase::Idle => {
                self.isr |= ISR_BUSY;
                if self.net_wire(LINE_SCL) && self.net_wire(LINE_SDA) {
                    self.net_drive(LINE_SDA, false);
                    let (_, t_high, _) = self.net_timing();
                    self.net.m = MPhase::StartHold;
                    self.net.deadline = Some(now + t_high);
                } else {
                    self.net.m = MPhase::WaitBus;
                }
            }
            _ => {}
        }
    }

    /// Pull SCL low at `now` and start the low half of a pulse.
    fn net_begin_low(&mut self, now: u64, pulse: Pulse) {
        self.net_drive(LINE_SCL, false);
        let (_, _, sdadel) = self.net_timing();
        self.net.pulse = pulse;
        self.net.m = MPhase::LowSetup;
        self.net.deadline = Some(now + sdadel);
    }

    /// Start a byte: the address byte after START, or a data byte.
    fn net_begin_byte(&mut self, now: u64, tx: Option<u8>, addr: bool) {
        self.net.tx = tx;
        self.net.rx = 0;
        self.net.bit = 0;
        self.net.addr_phase = addr;
        self.net_begin_low(now, Pulse::Bit);
    }

    fn net_address_byte(&self) -> u8 {
        ((((self.cr2 >> 1) & 0x7F) as u8) << 1) | u8::from(self.net.reading)
    }

    /// The SDA level this controller puts out for the bit in flight.
    fn net_master_sda_out(&self) -> bool {
        match self.net.pulse {
            Pulse::Stop => false,
            Pulse::Restart => true,
            Pulse::Bit => match (self.net.tx, self.net.bit) {
                (Some(byte), b) if b < 8 => (byte >> (7 - b)) & 1 != 0,
                // ACK slot while sending: release for the receiver's ACK.
                (Some(_), _) => true,
                (None, b) if b < 8 => true,
                // ACK slot while receiving: ACK all but the last byte.
                (None, _) => self.net.count + 1 >= self.net.nbytes,
            },
        }
    }

    /// Next data byte of a write: from TXDR, or stretch for it.
    fn net_master_next_tx(&mut self, now: u64) {
        if self.isr & ISR_TXE == 0 {
            let byte = self.txdr as u8;
            self.isr |= ISR_TXE;
            self.net_begin_byte(now, Some(byte), false);
        } else {
            self.net.m = MPhase::WaitTxdr;
            self.net.deadline = None;
            self.net_flag(ISR_TXIS);
        }
    }

    /// After the ACK slot of a byte, with SCL just pulled low: what next.
    fn net_master_after_byte(&mut self, now: u64, acked: bool) {
        if self.net.addr_phase {
            if !acked {
                self.net_flag(ISR_NACKF);
                self.net_begin_low(now, Pulse::Stop);
                return;
            }
            self.cr2 &= !CR2_START;
        } else if self.net.tx.is_some() {
            if !acked {
                self.net_flag(ISR_NACKF);
                self.net_begin_low(now, Pulse::Stop);
                return;
            }
            self.net.count += 1;
        } else {
            self.net.count += 1;
            self.rxdr = u32::from(self.net.rx);
            self.net.rx_taken.set(false);
            self.net_flag(ISR_RXNE);
        }
        if self.net.count >= self.net.nbytes {
            if self.net.autoend {
                self.net_begin_low(now, Pulse::Stop);
            } else {
                self.net.m = MPhase::WaitCmd;
                self.net.deadline = None;
                self.net_flag(ISR_TC);
            }
            return;
        }
        if self.net.reading {
            if self.isr & ISR_RXNE != 0 {
                self.net.m = MPhase::WaitRxRead;
                let (t_low, _, _) = self.net_timing();
                self.net.deadline = Some(now + (t_low / 4).max(1));
            } else {
                self.net_begin_byte(now, None, false);
            }
        } else {
            self.net_master_next_tx(now);
        }
    }

    /// A timed controller step is due.
    fn net_master_step(&mut self, now: u64) {
        let (t_low, t_high, sdadel) = self.net_timing();
        match self.net.m {
            MPhase::StartHold => {
                // START is on the wire: SCL falls and the address goes out.
                let addr = self.net_address_byte();
                self.net_begin_byte(now, Some(addr), true);
            }
            MPhase::LowSetup => {
                self.net_drive(LINE_SDA, self.net_master_sda_out());
                self.net.m = MPhase::LowWait;
                self.net.deadline = Some(now + t_low.saturating_sub(sdadel).max(1));
            }
            MPhase::LowWait => {
                self.net_drive(LINE_SCL, true);
                self.net.m = MPhase::WaitHigh;
                self.net.deadline = None;
                self.net_check_high(now);
            }
            MPhase::High => self.net_master_end_high(now, t_high),
            MPhase::WaitRxRead => {
                self.net_sync_rx();
                if self.isr & ISR_RXNE == 0 {
                    self.net_begin_byte(now, None, false);
                } else {
                    self.net.deadline = Some(now + (t_low / 4).max(1));
                }
            }
            _ => self.net.deadline = None,
        }
    }

    /// SCL released: once it is seen high, the high period starts.
    fn net_check_high(&mut self, now: u64) {
        if self.net.m == MPhase::WaitHigh && self.net_wire(LINE_SCL) {
            let (_, t_high, _) = self.net_timing();
            self.net.m = MPhase::High;
            self.net.deadline = Some(now + t_high);
        }
    }

    fn net_master_end_high(&mut self, now: u64, t_high: u64) {
        match self.net.pulse {
            Pulse::Stop => {
                // SDA rises while SCL is high: STOP.
                self.net_drive(LINE_SDA, true);
                self.net.m = MPhase::Idle;
                self.net.deadline = None;
                self.isr &= !ISR_BUSY;
                self.cr2 &= !CR2_START;
                self.net_flag(ISR_STOPF);
            }
            Pulse::Restart => {
                // SDA falls while SCL is high: repeated START.
                self.net_drive(LINE_SDA, false);
                self.net.m = MPhase::StartHold;
                self.net.deadline = Some(now + t_high);
            }
            Pulse::Bit => {
                let sda = self.net_wire(LINE_SDA);
                let bit = self.net.bit;
                if bit < 8 {
                    if self.net.tx.is_some() {
                        if self.net_master_sda_out() && !sda {
                            // Someone else pulled a 1 to 0: lost arbitration.
                            self.net_drive(LINE_SDA, true);
                            self.net.m = MPhase::Idle;
                            self.net.deadline = None;
                            self.cr2 &= !CR2_START;
                            self.net_flag(ISR_ARLO);
                            return;
                        }
                    } else {
                        self.net.rx = (self.net.rx << 1) | u8::from(sda);
                    }
                    self.net.bit += 1;
                    self.net_begin_low(now, Pulse::Bit);
                } else {
                    let acked = !sda;
                    self.net_drive(LINE_SCL, false);
                    self.net.bit = 0;
                    self.net_master_after_byte(now, acked);
                }
            }
        }
    }

    // ── target ──────────────────────────────────────────────────────────

    fn net_target_enabled(&self) -> bool {
        self.cr1 & CR1_PE != 0 && self.oar1 & OAR1_EN != 0
    }

    fn net_target_load_tx(&mut self) {
        // TXDR to the shift register; ask for the next one.
        self.net.t_tx = self.txdr as u8;
        self.isr |= ISR_TXE;
        self.net_flag(ISR_TXIS);
        self.net_drive(LINE_SDA, (self.net.t_tx >> 7) & 1 != 0);
    }

    /// The transmitter needs its next byte with SCL low: load it, or
    /// stretch until firmware writes TXDR.
    fn net_target_need_tx(&mut self) {
        if self.isr & ISR_TXE == 0 {
            self.net_target_load_tx();
            self.net_drive(LINE_SCL, true);
        } else {
            self.net_drive(LINE_SDA, true);
            self.net.stretch = Stretch::Txdr;
            self.net_drive(LINE_SCL, false);
            self.net_flag(ISR_TXIS);
        }
    }

    /// Firmware cleared ADDR: let the transfer run.
    fn net_target_after_addr(&mut self) {
        self.net.stretch = Stretch::None;
        if self.net.t == TPhase::Tx {
            self.net_target_need_tx();
        } else {
            self.net_drive(LINE_SCL, true);
        }
    }

    fn net_target_deliver_rx(&mut self) {
        self.rxdr = u32::from(self.net.t_shift);
        self.net.rx_taken.set(false);
        self.net_flag(ISR_RXNE);
        self.net_drive(LINE_SDA, false); // ACK
    }

    fn net_target_scl_rise(&mut self) {
        let sda = self.net.sda.unwrap_or(true);
        match self.net.t {
            TPhase::Addr | TPhase::Rx if self.net.t_bits < 8 => {
                self.net.t_shift = (self.net.t_shift << 1) | u8::from(sda);
                self.net.t_bits += 1;
            }
            TPhase::Tx if self.net.t_bits == 8 => {
                self.net.t_master_acked = !sda;
                self.net.t_bits += 1;
            }
            TPhase::Addr | TPhase::Rx | TPhase::Tx => self.net.t_bits += 1,
            _ => {}
        }
    }

    fn net_target_scl_fall(&mut self, now: u64) {
        match (self.net.t, self.net.t_bits) {
            (TPhase::Addr, 8) => {
                let addr = self.net.t_shift >> 1;
                let own = ((self.oar1 >> 1) & 0x7F) as u8;
                if addr == own {
                    self.net.t_reading = self.net.t_shift & 1 != 0;
                    self.net.t_addressed = true;
                    self.net_drive(LINE_SDA, false);
                    self.isr = (self.isr & !(ISR_DIR | (0x7F << ISR_ADDCODE_SHIFT)))
                        | (u32::from(addr) << ISR_ADDCODE_SHIFT)
                        | if self.net.t_reading { ISR_DIR } else { 0 }
                        | ISR_BUSY;
                    self.net_flag(ISR_ADDR);
                } else {
                    self.net.t = TPhase::Ignore;
                }
            }
            (TPhase::Addr, 9) => {
                // After the address ACK: stretch until ADDR is cleared.
                self.net.t = if self.net.t_reading {
                    TPhase::Tx
                } else {
                    TPhase::Rx
                };
                self.net.t_bits = 0;
                self.net.t_shift = 0;
                self.net_drive(LINE_SDA, true);
                if self.isr & ISR_ADDR != 0 {
                    self.net.stretch = Stretch::Addr;
                    self.net_drive(LINE_SCL, false);
                } else {
                    self.net_target_after_addr();
                }
            }
            (TPhase::Rx, 8) => {
                self.net_sync_rx();
                if self.isr & ISR_RXNE != 0 {
                    // Previous byte unread: stretch until it is.
                    self.net.stretch = Stretch::RxRead;
                    self.net_drive(LINE_SCL, false);
                    let (t_low, _, _) = self.net_timing();
                    self.net.deadline = Some(now + (t_low / 4).max(1));
                } else {
                    self.net_target_deliver_rx();
                }
            }
            (TPhase::Rx, 9) => {
                self.net_drive(LINE_SDA, true);
                self.net.t_bits = 0;
                self.net.t_shift = 0;
            }
            (TPhase::Tx, b @ 1..=7) => {
                self.net_drive(LINE_SDA, (self.net.t_tx >> (7 - b)) & 1 != 0);
            }
            (TPhase::Tx, 8) => self.net_drive(LINE_SDA, true),
            (TPhase::Tx, 9) => {
                self.net.t_bits = 0;
                if self.net.t_master_acked {
                    self.net_target_need_tx();
                } else {
                    // The controller NACKed: done until STOP.
                    self.net_drive(LINE_SDA, true);
                    self.net_flag(ISR_NACKF);
                    self.net.t = TPhase::Ignore;
                }
            }
            _ => {}
        }
    }

    fn net_target_stop(&mut self) {
        let was_ours = std::mem::take(&mut self.net.t_addressed);
        self.net.t = TPhase::Idle;
        self.net.stretch = Stretch::None;
        if self.net.m == MPhase::Idle {
            self.net_drive(LINE_SDA, true);
            self.net_drive(LINE_SCL, true);
            self.isr &= !ISR_BUSY;
            if was_ours {
                self.net_flag(ISR_STOPF);
            }
        }
    }

    /// The net delivered a new level on SCL or SDA at this chip's pad.
    pub(super) fn net_wire_edge(&mut self, line: usize, level: bool, now: u64) -> bool {
        if !self.net_mode() {
            return false;
        }
        self.net_set_stamp(Some(now));
        let wake = self.net_wire_edge_inner(line, level, now);
        self.net_set_stamp(None);
        wake
    }

    fn net_set_stamp(&self, cycle: Option<u64>) {
        if let Some(pads) = self.net_pads() {
            pads.set_stamp(cycle);
        }
    }

    fn net_wire_edge_inner(&mut self, line: usize, level: bool, now: u64) -> bool {
        self.net_sync_rx();
        let prev_scl = self.net.scl;
        let prev_sda = self.net.sda;
        if line == LINE_SCL {
            self.net.scl = Some(level);
        } else if line == LINE_SDA {
            self.net.sda = Some(level);
        } else {
            return false;
        }
        // Controller: a released SCL seen high starts the high period; a
        // free bus lets a waiting START go.
        self.net_check_high(now);
        if self.net.m == MPhase::WaitBus && self.net_wire(LINE_SCL) && self.net_wire(LINE_SDA) {
            self.net.m = MPhase::Idle;
            self.net_master_start();
        }
        // Target: only while this controller is not the one clocking.
        if self.net_target_enabled() && matches!(self.net.m, MPhase::Idle | MPhase::WaitBus) {
            if line == LINE_SDA && prev_scl == Some(true) && prev_sda.is_some() {
                if !level {
                    // START (or repeated START).
                    self.net.t = TPhase::Addr;
                    self.net.t_addressed = false;
                    self.net.t_bits = 0;
                    self.net.t_shift = 0;
                    self.isr |= ISR_BUSY;
                } else {
                    self.net_target_stop();
                }
            } else if line == LINE_SCL && prev_scl.is_some() && prev_scl != Some(level) {
                if level {
                    self.net_target_scl_rise();
                } else {
                    self.net_target_scl_fall(now);
                }
            }
        }
        self.net_wants_wake()
    }

    fn net_wants_wake(&self) -> bool {
        self.net.irq_pending || self.net.deadline.is_some()
    }

    /// A timed step is due at `now` (scheduler event or walk tick).
    fn net_step(&mut self, now: u64) {
        self.net_sync_rx();
        while let Some(due) = self.net.deadline {
            if due > now {
                break;
            }
            self.net.deadline = None;
            // Run the step at its own cycle, even when drained late.
            let now = due;
            self.net_set_stamp(Some(now));
            if self.net.stretch == Stretch::RxRead {
                if self.isr & ISR_RXNE == 0 {
                    self.net.stretch = Stretch::None;
                    self.net_target_deliver_rx();
                    self.net_drive(LINE_SCL, true);
                } else {
                    let (t_low, _, _) = self.net_timing();
                    self.net.deadline = Some(now + (t_low / 4).max(1));
                }
                continue;
            }
            self.net_master_step(now);
        }
        self.net_set_stamp(None);
    }

    /// Walk path: one tick. Returns the interrupt verdict.
    pub(super) fn net_tick(&mut self) -> bool {
        let now = self.net_now();
        if self.net.deadline.is_some_and(|d| d <= now) {
            self.net_step(now);
        }
        std::mem::take(&mut self.net.irq_pending)
    }

    /// Scheduler path: events to arm (delays from now).
    pub(super) fn net_take_events(&mut self) -> Vec<(u64, u32)> {
        let now = self.net_now();
        let mut out = Vec::new();
        if self.net.irq_pending && !self.net.armed.contains(&(now + 1)) {
            self.net.armed.push(now + 1);
            out.push((0, NET_TOKEN));
        }
        if let Some(due) = self.net.deadline {
            let due = due.max(now + 1);
            if !self.net.armed.contains(&due) {
                self.net.armed.push(due);
                out.push((due - now - 1, NET_TOKEN));
            }
        }
        out
    }

    /// Scheduler path: an armed event fired at `now`.
    pub(super) fn net_on_event(&mut self, now: u64) -> crate::sched::EventResult {
        self.net.armed.retain(|&t| t > now);
        self.net_step(now);
        let raise = std::mem::take(&mut self.net.irq_pending);
        let mut res = crate::sched::EventResult {
            raise_own_irq: raise,
            ..Default::default()
        };
        if let Some(due) = self.net.deadline {
            let due = due.max(now + 1);
            if !self.net.armed.contains(&due) {
                self.net.armed.push(due);
                res.reschedule_delay = Some(due - now);
            }
        }
        res
    }
}

/// Scheduler token for the net engine (the phase model's chain uses 0).
pub(super) const NET_TOKEN: u32 = 0x4E45_5400;
