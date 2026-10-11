// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The STM32 F1/F2/F4 legacy I²C (CR1/CR2/OAR1/OAR2/DR/SR1/SR2/CCR/TRISE) on
//! a world `gpio_net`: a bit-level master and slave between chips.
//!
//! Off a net the controller in `i2c.rs` is a phase model exchanging bytes
//! with attached device models. On a net (a pad that can carry SCL or SDA is
//! put on one, [`PadLines::on_net`]) the wire is driven by the shared engine
//! in [`crate::peripherals::i2c_bitbus`] and this module maps it onto the
//! legacy register semantics (RM0008 §26, RM0368 §18):
//!
//! * **Master.** `CR1.START` puts START (or a repeated START) on the wire and
//!   sets SB/MSL/BUSY. The DR write that follows is the address byte; its
//!   ACK sets ADDR (TRA from R/W), a NACK sets AF. After ADDR is cleared
//!   (SR1 then SR2 read) a transmitter takes DR (TXE) and holds SCL low with
//!   BTF when DR is empty after a byte; a receiver clocks bytes in, ACKs
//!   them by `CR1.ACK` (with `CR1.POS` the value in force one byte earlier),
//!   and holds SCL low with BTF while DR is still full. `CR1.STOP` sends STOP
//!   at once from a held bus, otherwise after the byte in flight.
//! * **Slave.** It answers its `OAR1` 7-bit address (and `OAR2` with
//!   `ENDUAL`, address 0 with `ENGC`) when `CR1.ACK` is set, sets ADDR (TRA,
//!   DUALF, GENCALL) and stretches SCL until ADDR is cleared; then it
//!   receives (RXNE, BTF with SCL stretched while DR is full, ACK by
//!   `CR1.ACK`) or transmits (TXE, BTF with SCL stretched while DR is empty;
//!   the master's NACK sets AF and ends it). STOP after a write sets STOPF.
//!   With `CR1.NOSTRETCH` it does not stretch after the address.
//!
//! Timing is CCR's (`I2C_CCR`): standard mode high = low = CCR, fast mode
//! low = 2·CCR, high = CCR, or 16/9 with DUTY, in PCLK1 periods taken as
//! engine cycles, the same identity the phase model documents.
//!
//! Interrupts: the event line (`ITEVTEN`: SB, ADDR, BTF, STOPF; with
//! `ITBUFEN` also TXE and RXNE) and the error line (`ITERREN`: AF, ARLO)
//! are raised when a flag is set and when an enable is written while its
//! flag is up.
//!
//! The one poll: clearing ADDR and reading DR are register *reads*, which
//! cannot wake the model, so while the bus is held waiting for one the
//! engine looks every quarter SCL low period (and at every register write).
//!
//! Not modelled on a net: 10-bit addressing (ADD10), SMBus, PEC, DMA, the
//! bus error (BERR) detector and timeouts.

use super::F1I2c;
use crate::peripherals::i2c_bitbus::{BitBus, BusEvent, Timing};
use crate::peripherals::pad_lines::PadLines;

const CR1_PE: u32 = 1 << 0;
const CR1_ENGC: u32 = 1 << 6;
const CR1_NOSTRETCH: u32 = 1 << 7;
const CR1_START: u32 = 1 << 8;
const CR1_STOP: u32 = 1 << 9;
const CR1_ACK: u32 = 1 << 10;
const CR1_POS: u32 = 1 << 11;
const CR1_SWRST: u32 = 1 << 15;

const CR2_ITERREN: u32 = 1 << 8;
const CR2_ITEVTEN: u32 = 1 << 9;
const CR2_ITBUFEN: u32 = 1 << 10;

const SR1_SB: u32 = 1 << 0;
const SR1_ADDR: u32 = 1 << 1;
const SR1_BTF: u32 = 1 << 2;
const SR1_STOPF: u32 = 1 << 4;
const SR1_RXNE: u32 = 1 << 6;
const SR1_TXE: u32 = 1 << 7;
const SR1_ARLO: u32 = 1 << 9;
const SR1_AF: u32 = 1 << 10;
const SR1_OVR: u32 = 1 << 11;

const SR2_MSL: u32 = 1 << 0;
const SR2_BUSY: u32 = 1 << 1;
const SR2_TRA: u32 = 1 << 2;
const SR2_GENCALL: u32 = 1 << 4;
const SR2_DUALF: u32 = 1 << 7;

const OAR1_ADDMODE: u32 = 1 << 15;
const OAR2_ENDUAL: u32 = 1 << 0;

/// What a held master waits for.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum MWait {
    #[default]
    None,
    /// SB set: the address byte (a DR write).
    Address,
    /// ADDR set: firmware clearing it (SR1 then SR2 read; polled).
    AddrClear,
    /// Transmitter: the next byte in DR (BTF, or TXE right after ADDR).
    TxData,
    /// Receiver: DR read while a byte waits behind it (BTF; polled).
    RxRead,
    /// After AF or the last received byte: STOP or a repeated START.
    Command,
}

/// What the slave side waits for while it stretches SCL.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum TWait {
    #[default]
    None,
    /// ADDR set: firmware clearing it (polled).
    AddrClear { read: bool },
    /// Transmitter: the next byte in DR.
    TxData,
    /// Receiver: DR read while a byte waits behind it (polled).
    RxRead,
}

/// The net state, kept beside the phase model's.
#[derive(Debug, Default)]
pub(super) struct LegacyNet {
    bus: BitBus,
    // ── master ──
    m_wait: MWait,
    m_addr_phase: bool,
    m_reading: bool,
    /// POS latch: the ACK the next received byte gets.
    m_pos_ack: bool,
    m_last_acked: bool,
    m_rx: u8,
    /// `CR1.STOP` written while a byte was in flight.
    stop_req: bool,
    // ── slave ──
    t_wait: TWait,
    t_reading: bool,
    /// Addressed as a receiver since the last START: STOP sets STOPF.
    t_stopf: bool,
    // ── both ──
    /// DR holds a byte not yet moved to the shift register.
    dr_full: bool,
    irq_pending: bool,
    err_pending: bool,
    poll: Option<u64>,
    armed: Vec<u64>,
    /// Bytes this slave received / sent and transfers it was addressed in
    /// (diagnostics).
    pub(super) slave_rx_bytes: u64,
    pub(super) slave_tx_bytes: u64,
}

/// Scheduler token for the legacy net engine (the phase-model chain uses 0).
pub(super) const LEGACY_NET_TOKEN: u32 = 0x4E45_5401;

impl F1I2c {
    fn lnet_pads(&self) -> Option<std::sync::Arc<PadLines>> {
        self.lines.clone()
    }

    /// `true` when this controller shares SCL/SDA with other chips through a
    /// world `gpio_net`.
    pub(super) fn net_mode(&self) -> bool {
        self.lines.as_deref().is_some_and(PadLines::any_on_net)
    }

    fn lnet_now(&self) -> u64 {
        self.clock.as_ref().map_or(0, crate::CycleClock::now)
    }

    /// (low, high) SCL periods from CCR, in engine cycles.
    fn lnet_timing(&self) -> Timing {
        let ccr = u64::from(self.ccr & 0x0FFF).max(1);
        let (low, high) = match (self.ccr & 0x8000 != 0, self.ccr & 0x4000 != 0) {
            (false, _) => (ccr, ccr),
            (true, false) => (2 * ccr, ccr),
            (true, true) => (16 * ccr, 9 * ccr),
        };
        Timing::new(low, high, low / 4)
    }

    fn lnet_flag(&mut self, flag: u32) {
        self.sr1 |= flag;
        let event = flag & (SR1_SB | SR1_ADDR | SR1_BTF | SR1_STOPF) != 0;
        let buffer = flag & (SR1_TXE | SR1_RXNE) != 0;
        let error = flag & (SR1_AF | SR1_ARLO | SR1_OVR) != 0;
        if (event && self.cr2 & CR2_ITEVTEN != 0)
            || (buffer && self.cr2 & CR2_ITEVTEN != 0 && self.cr2 & CR2_ITBUFEN != 0)
        {
            self.net.irq_pending = true;
        }
        if error && self.cr2 & CR2_ITERREN != 0 {
            self.net.err_pending = true;
        }
        // RXNE follows the receive pipeline (`effective_sr1`), not SR1.
        self.sr1 &= !SR1_RXNE;
    }

    /// Fold what firmware did through reads (ADDR cleared, DR read) into
    /// the engine. Runs at every register write and every poll.
    fn lnet_sync_reads(&mut self, now: u64, pads: &PadLines) {
        if self.addr_cleared.replace(false) {
            self.addr_sr1_seen.set(false);
            if self.sr1 & SR1_ADDR != 0 {
                self.sr1 &= !SR1_ADDR;
                self.lnet_after_addr(now, pads);
            }
        }
        if self.net.m_wait == MWait::RxRead && self.rx_shift.get().is_none() {
            self.net.m_wait = MWait::None;
            self.lnet_master_next_rx(now, pads);
        }
        if self.net.t_wait == TWait::RxRead && self.rx_shift.get().is_none() {
            // The waiting byte moved into DR; it is ACKed now.
            self.net.t_wait = TWait::None;
            self.net.bus.t_ack(self.cr1 & CR1_ACK != 0, now, pads);
        }
    }

    /// ADDR was cleared.
    fn lnet_after_addr(&mut self, now: u64, pads: &PadLines) {
        if self.net.m_wait == MWait::AddrClear {
            self.net.m_wait = MWait::None;
            if self.net.m_reading {
                self.net.bus.m_recv(now, pads);
            } else {
                self.lnet_master_need_tx(now, pads);
            }
        } else if let TWait::AddrClear { read } = self.net.t_wait {
            self.net.t_wait = TWait::None;
            if read {
                self.lnet_slave_need_tx(now, pads, false);
            } else {
                self.net.bus.t_continue(now, pads);
            }
        }
    }

    // ── register side ───────────────────────────────────────────────────

    /// Register writes in net mode. Returns `true` when handled here.
    pub(super) fn lnet_write_reg(&mut self, offset: u64, value: u16) -> bool {
        let Some(pads) = self.lnet_pads() else {
            return false;
        };
        let now = self.lnet_now();
        self.net.bus.timing = self.lnet_timing();
        pads.set_stamp(Some(now));
        self.lnet_sync_reads(now, &pads);
        let handled = match offset {
            0x00 => {
                self.lnet_write_cr1(u32::from(value), now, &pads);
                true
            }
            0x04 => {
                self.cr2 = u32::from(value) & 0x1F3F;
                // An enable written while its flag is up interrupts.
                if self.irq_level() {
                    self.net.irq_pending = true;
                }
                if self.cr2 & CR2_ITERREN != 0 && self.sr1 & (SR1_AF | SR1_ARLO | SR1_OVR) != 0 {
                    self.net.err_pending = true;
                }
                true
            }
            0x10 => {
                self.lnet_write_dr(u32::from(value) & 0xFF, now, &pads);
                true
            }
            _ => false,
        };
        self.lnet_process(now, &pads);
        pads.set_stamp(None);
        handled
    }

    fn lnet_write_cr1(&mut self, value: u32, now: u64, pads: &PadLines) {
        let was_on = self.cr1 & CR1_PE != 0;
        self.cr1 = value & 0xBFFB;
        // STOPF clears on the CR1 write that follows the SR1 read.
        self.sr1 &= !SR1_STOPF;
        if value & CR1_SWRST != 0 || (was_on && self.cr1 & CR1_PE == 0) {
            self.lnet_reset(pads);
            return;
        }
        if self.cr1 & CR1_PE == 0 {
            return;
        }
        if value & CR1_START != 0 {
            let bus = &mut self.net.bus;
            if !bus.master_active() || bus.master_holding() {
                // A START from a held bus is a repeated START. The pending
                // STOP of the byte in flight (if any) is overtaken.
                self.net.stop_req = false;
                self.net.m_wait = MWait::None;
                bus.m_start(now, pads);
            }
        }
        if value & CR1_STOP != 0 && value & CR1_START == 0 {
            if self.net.bus.master_holding() {
                self.net.m_wait = MWait::None;
                self.net.bus.m_stop(now, pads);
            } else if self.net.bus.master_active() {
                self.net.stop_req = true;
            } else {
                // Slave mode: STOP releases the lines (RM0008 CR1.STOP).
                self.cr1 &= !CR1_STOP;
            }
        }
    }

    fn lnet_write_dr(&mut self, value: u32, now: u64, pads: &PadLines) {
        self.dr = value;
        if self.net.m_wait == MWait::Address && self.sr1 & SR1_SB != 0 {
            // SB is cleared by the SR1 read and this DR write: the address.
            self.sr1 &= !SR1_SB;
            self.net.m_wait = MWait::None;
            self.net.m_addr_phase = true;
            self.net.m_reading = value & 1 != 0;
            self.net.bus.m_send(value as u8, now, pads);
            return;
        }
        if self.sr2 & SR2_MSL != 0 && !self.net.m_reading {
            self.sr1 &= !(SR1_TXE | SR1_BTF);
            self.net.dr_full = true;
            if self.net.m_wait == MWait::TxData {
                self.net.m_wait = MWait::None;
                self.lnet_master_need_tx(now, pads);
            }
            return;
        }
        if self.sr2 & SR2_MSL == 0 && self.net.t_reading {
            self.sr1 &= !(SR1_TXE | SR1_BTF);
            self.net.dr_full = true;
            if self.net.t_wait == TWait::TxData {
                self.net.t_wait = TWait::None;
                self.lnet_slave_need_tx(now, pads, false);
            }
        }
    }

    fn lnet_reset(&mut self, pads: &PadLines) {
        self.net.bus.reset(pads);
        let (rx, tx) = (self.net.slave_rx_bytes, self.net.slave_tx_bytes);
        self.net = LegacyNet {
            slave_rx_bytes: rx,
            slave_tx_bytes: tx,
            ..LegacyNet::default()
        };
        self.sr1 = 0;
        self.sr2 = 0;
        self.rx_dr.set(None);
        self.rx_shift.set(None);
        self.addr_cleared.set(false);
        self.addr_sr1_seen.set(false);
    }

    // ── master ──────────────────────────────────────────────────────────

    /// The transmitter needs its next byte with SCL held: DR, or BTF.
    fn lnet_master_need_tx(&mut self, now: u64, pads: &PadLines) {
        if self.net.stop_req {
            self.net.stop_req = false;
            self.net.bus.m_stop(now, pads);
        } else if self.net.dr_full {
            self.net.dr_full = false;
            self.net.bus.m_send(self.dr as u8, now, pads);
            self.lnet_flag(SR1_TXE);
        } else {
            if self.sr1 & SR1_TXE != 0 {
                // A byte went out and DR is still empty.
                self.lnet_flag(SR1_BTF);
            } else {
                self.lnet_flag(SR1_TXE);
            }
            self.net.m_wait = MWait::TxData;
        }
    }

    /// After a received byte's ACK slot (and DR emptied): what next.
    fn lnet_master_next_rx(&mut self, now: u64, pads: &PadLines) {
        if self.net.stop_req {
            self.net.stop_req = false;
            self.net.bus.m_stop(now, pads);
        } else if !self.net.m_last_acked {
            self.net.m_wait = MWait::Command;
        } else {
            self.net.bus.m_recv(now, pads);
        }
    }

    fn lnet_on_master_event(&mut self, event: BusEvent, now: u64, pads: &PadLines) {
        match event {
            BusEvent::MStarted => {
                // A (repeated) START clears the byte flags of what went
                // before (BTF is cleared by START or STOP).
                self.sr1 &= !(SR1_BTF | SR1_TXE);
                self.net.dr_full = false;
                self.cr1 &= !CR1_START;
                self.sr2 |= SR2_MSL | SR2_BUSY;
                self.net.m_wait = MWait::Address;
                self.lnet_flag(SR1_SB);
            }
            BusEvent::MSent { acked } if self.net.m_addr_phase => {
                self.net.m_addr_phase = false;
                if acked {
                    if self.net.m_reading {
                        self.sr2 &= !SR2_TRA;
                    } else {
                        self.sr2 |= SR2_TRA;
                    }
                    self.net.m_pos_ack = self.cr1 & CR1_ACK != 0;
                    self.rx_dr.set(None);
                    self.rx_shift.set(None);
                    self.sr1 &= !SR1_TXE;
                    self.net.dr_full = false;
                    self.addr_cleared.set(false);
                    self.addr_sr1_seen.set(false);
                    self.net.m_wait = MWait::AddrClear;
                    self.lnet_flag(SR1_ADDR);
                } else {
                    self.net.m_wait = MWait::Command;
                    self.lnet_flag(SR1_AF);
                }
                if self.net.stop_req {
                    self.net.stop_req = false;
                    self.net.m_wait = MWait::None;
                    self.net.bus.m_stop(now, pads);
                }
            }
            BusEvent::MSent { acked } => {
                if acked {
                    self.lnet_master_need_tx(now, pads);
                } else {
                    self.lnet_flag(SR1_AF);
                    self.net.m_wait = MWait::Command;
                    if std::mem::take(&mut self.net.stop_req) {
                        self.net.m_wait = MWait::None;
                        self.net.bus.m_stop(now, pads);
                    }
                }
            }
            BusEvent::MByteIn(byte) => {
                let ack_now = self.cr1 & CR1_ACK != 0;
                let ack = if self.cr1 & CR1_POS != 0 {
                    std::mem::replace(&mut self.net.m_pos_ack, ack_now)
                } else {
                    ack_now
                };
                self.net.m_last_acked = ack;
                self.net.m_rx = byte;
                self.net.bus.m_ack(ack, now, pads);
            }
            BusEvent::MAckDone => {
                let byte = self.net.m_rx;
                if self.rx_dr.get().is_none() {
                    self.rx_dr.set(Some(byte));
                    self.lnet_flag(SR1_RXNE);
                    self.lnet_master_next_rx(now, pads);
                } else {
                    // DR still full: the byte waits in the shift register
                    // (BTF) and SCL stays low until DR is read.
                    self.rx_shift.set(Some(byte));
                    self.lnet_flag(SR1_BTF);
                    self.sr1 &= !SR1_BTF; // BTF is derived from the pipeline
                    self.net.m_wait = MWait::RxRead;
                }
            }
            BusEvent::MStopped => {
                self.cr1 &= !CR1_STOP;
                self.sr2 &= !(SR2_MSL | SR2_BUSY | SR2_TRA);
                self.sr1 &= !(SR1_TXE | SR1_BTF | SR1_ADDR | SR1_SB);
                self.net.m_wait = MWait::None;
                self.net.stop_req = false;
                self.net.dr_full = false;
            }
            BusEvent::MArbLost => {
                self.sr2 &= !(SR2_MSL | SR2_TRA);
                self.cr1 &= !(CR1_START | CR1_STOP);
                self.net.m_wait = MWait::None;
                self.net.stop_req = false;
                self.lnet_flag(SR1_ARLO);
            }
            _ => {}
        }
    }

    // ── slave ───────────────────────────────────────────────────────────

    /// The slave transmitter needs its next byte with SCL low.
    fn lnet_slave_need_tx(&mut self, now: u64, pads: &PadLines, after_byte: bool) {
        if self.net.dr_full {
            self.net.dr_full = false;
            self.net.slave_tx_bytes += 1;
            self.net.bus.t_send(self.dr as u8, now, pads);
            self.lnet_flag(SR1_TXE);
        } else {
            if after_byte {
                self.lnet_flag(SR1_BTF);
            }
            self.lnet_flag(SR1_TXE);
            self.net.t_wait = TWait::TxData;
        }
    }

    /// Does `byte` (address << 1 | R/W) select this slave? Returns the SR2
    /// bits the match sets.
    fn lnet_match(&self, byte: u8) -> Option<u32> {
        if self.cr1 & CR1_PE == 0 || self.cr1 & CR1_ACK == 0 {
            return None;
        }
        let addr = u32::from(byte >> 1);
        if self.oar1 & OAR1_ADDMODE == 0 && addr == (self.oar1 >> 1) & 0x7F {
            return Some(0);
        }
        if self.oar2 & OAR2_ENDUAL != 0 && addr == (self.oar2 >> 1) & 0x7F {
            return Some(SR2_DUALF);
        }
        if addr == 0 && byte & 1 == 0 && self.cr1 & CR1_ENGC != 0 {
            return Some(SR2_GENCALL);
        }
        None
    }

    fn lnet_on_slave_event(&mut self, event: BusEvent, now: u64, pads: &PadLines) {
        match event {
            BusEvent::TStart => {
                self.sr2 |= SR2_BUSY;
                self.net.t_stopf = false;
                self.net.t_wait = TWait::None;
            }
            BusEvent::TStop => {
                if std::mem::take(&mut self.net.t_stopf) {
                    self.lnet_flag(SR1_STOPF);
                }
                self.net.t_wait = TWait::None;
                self.net.t_reading = false;
                self.sr2 &= !(SR2_BUSY | SR2_TRA | SR2_DUALF | SR2_GENCALL);
                // A byte loaded into DR that never went out stays there: it
                // is the first one the next read gets.
            }
            BusEvent::TAddress(byte) => match self.lnet_match(byte) {
                Some(bits) => {
                    self.sr2 = (self.sr2 & !(SR2_DUALF | SR2_GENCALL)) | bits;
                    self.net.bus.t_ack(true, now, pads);
                }
                None => self.net.bus.t_ack(false, now, pads),
            },
            BusEvent::TAddressed { read } => {
                self.net.t_reading = read;
                self.net.t_stopf = !read;
                self.sr2 =
                    (self.sr2 & !(SR2_MSL | SR2_TRA)) | SR2_BUSY | if read { SR2_TRA } else { 0 };
                self.addr_cleared.set(false);
                self.addr_sr1_seen.set(false);
                if read {
                    self.sr1 &= !SR1_TXE;
                }
                self.lnet_flag(SR1_ADDR);
                if self.cr1 & CR1_NOSTRETCH != 0 {
                    // No stretching: the transfer goes on while ADDR is up.
                    if read {
                        if !self.net.dr_full {
                            // Nothing written: the slave sends DR as it is.
                            self.lnet_flag(SR1_OVR);
                            self.net.dr_full = true;
                        }
                        self.lnet_slave_need_tx(now, pads, false);
                    } else {
                        self.net.bus.t_continue(now, pads);
                    }
                } else {
                    self.net.t_wait = TWait::AddrClear { read };
                }
            }
            BusEvent::TByteIn(byte) => {
                self.net.slave_rx_bytes += 1;
                if self.rx_dr.get().is_none() {
                    self.rx_dr.set(Some(byte));
                    self.lnet_flag(SR1_RXNE);
                    self.net.bus.t_ack(self.cr1 & CR1_ACK != 0, now, pads);
                } else {
                    // DR full: BTF, SCL stretched until DR is read.
                    self.rx_shift.set(Some(byte));
                    self.lnet_flag(SR1_BTF);
                    self.sr1 &= !SR1_BTF;
                    self.net.t_wait = TWait::RxRead;
                }
            }
            BusEvent::TByteOut { acked } => {
                if acked {
                    self.lnet_slave_need_tx(now, pads, true);
                } else {
                    // The master's NACK ends a slave transmission: AF.
                    self.net.t_stopf = false;
                    self.lnet_flag(SR1_AF);
                }
            }
            _ => {}
        }
    }

    /// Handle every event the engine has reported, then let a waiting
    /// slave stretch.
    fn lnet_process(&mut self, now: u64, pads: &PadLines) {
        loop {
            let events = self.net.bus.take_events();
            if events.is_empty() {
                break;
            }
            for event in events {
                match event {
                    BusEvent::MStarted
                    | BusEvent::MSent { .. }
                    | BusEvent::MByteIn(_)
                    | BusEvent::MAckDone
                    | BusEvent::MStopped
                    | BusEvent::MArbLost => self.lnet_on_master_event(event, now, pads),
                    _ => self.lnet_on_slave_event(event, now, pads),
                }
            }
        }
        self.net.bus.settle(pads);
        self.lnet_arm_poll(now);
    }

    fn lnet_arm_poll(&mut self, now: u64) {
        let waiting = matches!(self.net.m_wait, MWait::AddrClear | MWait::RxRead)
            || matches!(self.net.t_wait, TWait::AddrClear { .. } | TWait::RxRead);
        if !waiting {
            self.net.poll = None;
        } else if self.net.poll.is_none_or(|p| p <= now) {
            let t = self.net.bus.timing;
            self.net.poll = Some(now + (t.low / 4).max(1));
        }
    }

    // ── engine plumbing ─────────────────────────────────────────────────

    /// The net delivered a new level on SCL or SDA at this chip's pad.
    pub(super) fn lnet_wire_edge(&mut self, line: usize, level: bool, now: u64) -> bool {
        if !self.net_mode() {
            return false;
        }
        let Some(pads) = self.lnet_pads() else {
            return false;
        };
        self.net.bus.timing = self.lnet_timing();
        pads.set_stamp(Some(now));
        self.lnet_sync_reads(now, &pads);
        self.net.bus.on_edge(line, level, now, &pads);
        self.lnet_process(now, &pads);
        pads.set_stamp(None);
        self.lnet_wants_wake()
    }

    fn lnet_wants_wake(&self) -> bool {
        self.net.irq_pending
            || self.net.err_pending
            || self.net.bus.deadline().is_some()
            || self.net.poll.is_some()
    }

    fn lnet_next_due(&self) -> Option<u64> {
        match (self.net.bus.deadline(), self.net.poll) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Run every timed step and poll due by `now`.
    fn lnet_step(&mut self, now: u64) {
        let Some(pads) = self.lnet_pads() else {
            return;
        };
        self.net.bus.timing = self.lnet_timing();
        while let Some(due) = self.lnet_next_due() {
            if due > now {
                break;
            }
            pads.set_stamp(Some(due));
            if self.net.poll.is_some_and(|p| p <= due) {
                self.net.poll = None;
                self.lnet_sync_reads(due, &pads);
            }
            if self.net.bus.deadline().is_some_and(|d| d <= due) {
                self.net.bus.step(due, &pads);
                pads.set_stamp(Some(due));
            }
            self.lnet_process(due, &pads);
        }
        pads.set_stamp(None);
    }

    /// Walk path: one tick. Returns the event-line verdict (level, as the
    /// phase model's walk does).
    pub(super) fn lnet_tick(&mut self) -> bool {
        let now = self.lnet_now();
        if self.lnet_next_due().is_some_and(|d| d <= now) {
            self.lnet_step(now);
        }
        self.net.err_pending = false;
        std::mem::take(&mut self.net.irq_pending) || self.irq_level()
    }

    /// Scheduler path: events to arm (delays from now).
    pub(super) fn lnet_take_events(&mut self) -> Vec<(u64, u32)> {
        let now = self.lnet_now();
        let mut out = Vec::new();
        if (self.net.irq_pending || self.net.err_pending) && !self.net.armed.contains(&(now + 1)) {
            self.net.armed.push(now + 1);
            out.push((0, LEGACY_NET_TOKEN));
        }
        if let Some(due) = self.lnet_next_due() {
            let due = due.max(now + 1);
            if !self.net.armed.contains(&due) {
                self.net.armed.push(due);
                out.push((due - now - 1, LEGACY_NET_TOKEN));
            }
        }
        out
    }

    /// Scheduler path: an armed event fired at `now`.
    pub(super) fn lnet_on_event(&mut self, now: u64) -> crate::sched::EventResult {
        self.net.armed.retain(|&t| t > now);
        self.lnet_step(now);
        let raise = std::mem::take(&mut self.net.irq_pending);
        let err = std::mem::take(&mut self.net.err_pending);
        let mut res = crate::sched::EventResult {
            raise_own_irq: raise || (err && self.irq_error.is_none()),
            explicit_irqs: if err {
                self.irq_error.map(|n| vec![n]).unwrap_or_default()
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        if let Some(due) = self.lnet_next_due() {
            let due = due.max(now + 1);
            if !self.net.armed.contains(&due) {
                self.net.armed.push(due);
                res.reschedule_delay = Some(due - now);
            }
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c_bitbus::{SCL, SDA};
    use crate::peripherals::pad_lines::LineDrive;

    /// A legacy slave on a net, addressed by a bare bit-level master with
    /// zero wire latency.
    struct Rig {
        slave: F1I2c,
        master: BitBus,
        pm: PadLines,
        level: [bool; 2],
        now: u64,
    }

    impl Rig {
        fn new(oar1: u16, oar2: u16, cr1: u16) -> Self {
            let mut slave = F1I2c::default();
            let lines = slave.pad_lines_arc();
            lines.mark_on_net(SCL);
            lines.mark_on_net(SDA);
            slave.write_reg(0x1C, 50); // CCR
            slave.write_reg(0x08, oar1);
            slave.write_reg(0x0C, oar2);
            slave.write_reg(0x00, cr1);
            let pm = PadLines::new(&["SCL", "SDA"], &[true, true]);
            pm.set_mode(SCL, LineDrive::OpenDrain);
            pm.set_mode(SDA, LineDrive::OpenDrain);
            let mut master = BitBus::default();
            master.timing = Timing::new(8, 8, 2);
            let mut rig = Self {
                slave,
                master,
                pm,
                level: [true, true],
                now: 0,
            };
            for line in [SCL, SDA] {
                rig.deliver(line, true);
            }
            rig
        }

        fn deliver(&mut self, line: usize, level: bool) {
            self.pm.set_input(line, level);
            self.master.on_edge(line, level, self.now, &self.pm);
            let pads = self.slave.lines.clone().unwrap();
            pads.set_input(line, level);
            self.slave.lnet_wire_edge(line, level, self.now);
        }

        fn settle(&mut self) {
            let pads = self.slave.lines.clone().unwrap();
            loop {
                let mut moved = false;
                for line in [SCL, SDA] {
                    let level = !(self.pm.drives(line) || pads.drives(line));
                    if level != self.level[line] {
                        self.level[line] = level;
                        moved = true;
                        self.deliver(line, level);
                    }
                }
                if !moved {
                    return;
                }
            }
        }

        /// Run the master until it reports something.
        fn run(&mut self) -> Vec<BusEvent> {
            for _ in 0..10_000 {
                self.settle();
                let events = self.master.take_events();
                if !events.is_empty() {
                    return events;
                }
                let due = match (self.master.deadline(), self.slave.lnet_next_due()) {
                    (Some(a), Some(b)) => a.min(b),
                    (Some(a), None) | (None, Some(a)) => a,
                    (None, None) => panic!("stalled"),
                };
                self.now = due;
                self.master.step(due, &self.pm);
                self.slave.lnet_step(due);
            }
            panic!("runaway");
        }

        /// START and one address byte; the ACK slot's verdict.
        fn address(&mut self, byte: u8) -> bool {
            self.master.m_start(self.now, &self.pm);
            assert_eq!(self.run(), vec![BusEvent::MStarted]);
            self.master.m_send(byte, self.now, &self.pm);
            match self.run().as_slice() {
                [BusEvent::MSent { acked }] => *acked,
                other => panic!("{other:?}"),
            }
        }
    }

    const PE_ACK: u16 = (CR1_PE | CR1_ACK) as u16;

    #[test]
    fn the_own_address_is_acked_with_addr_and_tra() {
        let mut rig = Rig::new(0x4000 | (0x42 << 1), 0, PE_ACK);
        assert!(rig.address(0x85));
        assert_ne!(rig.slave.sr1 & SR1_ADDR, 0);
        assert_ne!(rig.slave.sr2 & SR2_TRA, 0, "R/W = 1: the slave transmits");
        assert_ne!(rig.slave.sr2 & SR2_BUSY, 0);
        // ADDR is up: the slave stretches SCL.
        assert!(rig.slave.lines.as_ref().unwrap().drives(SCL));
    }

    #[test]
    fn another_address_is_not_acked() {
        let mut rig = Rig::new(0x4000 | (0x42 << 1), 0, PE_ACK);
        assert!(!rig.address(0x86));
        assert_eq!(rig.slave.sr1 & SR1_ADDR, 0);
    }

    #[test]
    fn without_cr1_ack_the_slave_does_not_answer() {
        let mut rig = Rig::new(0x4000 | (0x42 << 1), 0, CR1_PE as u16);
        assert!(!rig.address(0x84));
    }

    #[test]
    fn oar2_with_endual_is_acked_with_dualf() {
        let mut rig = Rig::new(0x4000 | (0x42 << 1), (0x30 << 1) | 1, PE_ACK);
        assert!(rig.address(0x60));
        assert_ne!(rig.slave.sr2 & SR2_DUALF, 0);
    }

    #[test]
    fn the_general_call_is_acked_only_with_engc() {
        let mut rig = Rig::new(0x4000 | (0x42 << 1), 0, PE_ACK);
        assert!(!rig.address(0x00));
        let mut rig = Rig::new(0x4000 | (0x42 << 1), 0, PE_ACK | CR1_ENGC as u16);
        assert!(rig.address(0x00));
        assert_ne!(rig.slave.sr2 & SR2_GENCALL, 0);
    }

    #[test]
    fn nostretch_lets_the_transfer_run_while_addr_is_up() {
        let mut rig = Rig::new(0x4000 | (0x42 << 1), 0, PE_ACK | CR1_NOSTRETCH as u16);
        assert!(rig.address(0x84));
        assert_ne!(rig.slave.sr1 & SR1_ADDR, 0);
        assert!(!rig.slave.lines.as_ref().unwrap().drives(SCL));
    }
}
