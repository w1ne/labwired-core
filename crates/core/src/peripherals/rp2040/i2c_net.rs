// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The RP2040 `DW_apb_i2c` on a world `gpio_net`: a bit-level master and
//! slave between chips, on the shared wire engine
//! ([`crate::peripherals::i2c_bitbus`]).
//!
//! Off a net `i2c.rs` exchanges each command with attached device models
//! inside the `IC_DATA_CMD` write. On a net (an SCL or SDA pad routed by
//! IO_BANK0 `FUNCSEL` = I2C is put on one) the controller runs this engine:
//!
//! * **Master** (`IC_CON.MASTER_MODE`). `IC_DATA_CMD` writes queue in the
//!   16-deep TX FIFO: a byte to write, or a read (`CMD`), with `STOP` and
//!   `RESTART` flags. While enabled and the FIFO is not empty the master
//!   sends START and the `IC_TAR` address, then works through the FIFO: a
//!   change of direction or `RESTART` is a repeated START with the address
//!   again, `STOP` ends the transfer after its byte. A read is ACKed unless it
//!   carries `STOP`. An empty FIFO without `STOP` holds SCL low until the next
//!   command (the RP2040 builds `IC_EMPTYFIFO_HOLD_MASTER_EN = 1`). A NACKed
//!   address or data byte aborts: `TX_ABRT` with `ABRT_7B_ADDR_NOACK` or
//!   `ABRT_TXDATA_NOACK`, the FIFO flushed, STOP; commands written while
//!   `TX_ABRT` is up are dropped until `IC_CLR_TX_ABRT` is read. Timing is
//!   `IC_SS/FS_SCL_LCNT` low and `HCNT` high (clk_sys cycles), SDA changed
//!   `IC_SDA_HOLD` after SCL falls.
//! * **Slave** (`MASTER_MODE = 0`, `IC_SLAVE_DISABLE = 0`). It ACKs its
//!   7-bit `IC_SAR`; received bytes go to the 16-deep RX FIFO (the first one
//!   after the address has `FIRST_DATA_BYTE`, bit 11, set on read); a read
//!   is served from the TX FIFO, and while that is empty the slave holds SCL
//!   low with `RD_REQ` until firmware writes `IC_DATA_CMD`. The master's
//!   NACK ends a read with `RX_DONE`.
//!
//! Interrupt bits follow the DW names (`RX_FULL` above `IC_RX_TL`,
//! `TX_EMPTY` at or below `IC_TX_TL`, `RD_REQ`, `TX_ABRT`, `RX_DONE`,
//! `ACTIVITY`, `STOP_DET` (only when addressed with `STOP_DET_IFADDRESSED`),
//! `START_DET`), with the read-to-clear `IC_CLR_*` registers. The interrupt
//! line is raised as the raw bits rise; the tests poll. Not modelled on a
//! net: 10-bit addressing, the general call, SMBus, DMA, spike suppression
//! and `IC_RX_FULL_HLD_CTRL` (a full RX FIFO drops the byte with
//! `RX_OVER`).

use super::Rp2040I2c;
use crate::peripherals::i2c_bitbus::{BitBus, BusEvent, Timing};
use crate::peripherals::pad_lines::PadLines;
use std::cell::RefCell;
use std::collections::VecDeque;

pub(super) const IC_SAR: u64 = 0x08;
pub(super) const IC_RX_TL: u64 = 0x38;
pub(super) const IC_TX_TL: u64 = 0x3c;
pub(super) const IC_CLR_INTR: u64 = 0x40;
pub(super) const IC_CLR_RX_OVER: u64 = 0x48;
pub(super) const IC_CLR_RD_REQ: u64 = 0x50;
pub(super) const IC_CLR_RX_DONE: u64 = 0x58;
pub(super) const IC_CLR_START_DET: u64 = 0x64;
pub(super) const IC_SDA_HOLD: u64 = 0x7c;

const CON_MASTER: u32 = 1 << 0;
const CON_SLAVE_DISABLE: u32 = 1 << 6;
const CON_STOP_DET_IFADDRESSED: u32 = 1 << 7;
const CON_TX_EMPTY_CTRL: u32 = 1 << 8;

pub(super) const INTR_RX_OVER: u32 = 1 << 1;
pub(super) const INTR_RX_FULL: u32 = 1 << 2;
pub(super) const INTR_RD_REQ: u32 = 1 << 5;
pub(super) const INTR_RX_DONE: u32 = 1 << 7;
pub(super) const INTR_START_DET: u32 = 1 << 10;
const INTR_TX_EMPTY: u32 = 1 << 4;
const INTR_TX_ABRT: u32 = 1 << 6;
const INTR_ACTIVITY: u32 = 1 << 8;
const INTR_STOP_DET: u32 = 1 << 9;

const ABRT_7B_ADDR_NOACK: u32 = 1 << 0;
const ABRT_TXDATA_NOACK: u32 = 1 << 3;

const CMD_READ: u32 = 1 << 8;
const CMD_STOP: u32 = 1 << 9;
const CMD_RESTART: u32 = 1 << 10;
const FIRST_DATA_BYTE: u32 = 1 << 11;

const STATUS_ACTIVITY: u32 = 1 << 0;
const STATUS_TFNF: u32 = 1 << 1;
const STATUS_TFE: u32 = 1 << 2;
const STATUS_RFNE: u32 = 1 << 3;
const STATUS_RFF: u32 = 1 << 4;
const STATUS_MST_ACTIVITY: u32 = 1 << 5;
const STATUS_SLV_ACTIVITY: u32 = 1 << 6;

const FIFO_DEPTH: usize = 16;

/// Scheduler token for the net engine (the interrupt chain uses its own
/// sequence numbers, which start at 1 and never reach this).
pub(super) const NET_TOKEN: u32 = 0x4943_0000;

/// What the held master waits for.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum MState {
    #[default]
    Idle,
    /// START (or repeated START) on its way; the address follows.
    Starting,
    /// The address byte is out.
    Address,
    /// A command's byte is on the wire.
    Busy,
    /// Holding SCL low for the next command.
    Hold,
    /// STOP on its way.
    Stopping,
}

/// The net state, kept beside the word-level model's.
#[derive(Debug, Default)]
pub(super) struct DwNet {
    bus: BitBus,
    tx: VecDeque<u32>,
    pub(super) rx: RefCell<VecDeque<u32>>,
    m: MState,
    /// Direction of the transfer the master is in (`true`: reading).
    m_read: bool,
    /// The command whose byte is on the wire.
    cur: u32,
    // slave
    t_addressed: bool,
    t_first: bool,
    t_wait_tx: bool,
    t_reading: bool,
    pub(super) sar: u32,
    pub(super) rx_tl: u32,
    pub(super) tx_tl: u32,
    pub(super) sda_hold: u32,
    armed: Vec<u64>,
}

impl Rp2040I2c {
    fn net_pads(&self) -> Option<std::sync::Arc<PadLines>> {
        self.lines.clone()
    }

    /// `true` when this controller shares SCL/SDA with other chips through
    /// a world `gpio_net`.
    pub(super) fn net_mode(&self) -> bool {
        self.lines.as_deref().is_some_and(PadLines::any_on_net)
    }

    fn net_now(&self) -> u64 {
        self.clock.as_ref().map_or(0, crate::CycleClock::now)
    }

    fn net_timing(&self) -> Timing {
        let fast = (self.con >> 1) & 0b11 >= 2;
        let (h, l) = if fast {
            (self.fs_scl_hcnt, self.fs_scl_lcnt)
        } else {
            (self.ss_scl_hcnt, self.ss_scl_lcnt)
        };
        let hold = u64::from(self.net.sda_hold & 0xFFFF).max(1);
        Timing::new(u64::from(l).max(8), u64::from(h).max(8), hold)
    }

    fn net_enabled(&self) -> bool {
        self.enable & 1 != 0
    }

    fn net_is_master(&self) -> bool {
        self.con & CON_MASTER != 0
    }

    /// Raw interrupt bits in net mode: the latched ones plus the FIFO
    /// levels.
    pub(super) fn net_raw(&self) -> u32 {
        let mut raw = self.raw_intr.get() & !(INTR_TX_EMPTY | INTR_RX_FULL);
        let tx_empty = self.net.tx.len() as u32 <= self.net.tx_tl
            && (self.con & CON_TX_EMPTY_CTRL == 0
                || !matches!(
                    self.net.m,
                    MState::Busy | MState::Address | MState::Starting
                ));
        if tx_empty {
            raw |= INTR_TX_EMPTY;
        }
        if self.net.rx.borrow().len() as u32 > self.net.rx_tl {
            raw |= INTR_RX_FULL;
        }
        raw
    }

    /// IC_STATUS in net mode.
    pub(super) fn net_status(&self) -> u32 {
        let mut s = 0;
        let tx = self.net.tx.len();
        if tx < FIFO_DEPTH {
            s |= STATUS_TFNF;
        }
        if tx == 0 {
            s |= STATUS_TFE;
        }
        let rx = self.net.rx.borrow().len();
        if rx > 0 {
            s |= STATUS_RFNE;
        }
        if rx >= FIFO_DEPTH {
            s |= STATUS_RFF;
        }
        if self.net.m != MState::Idle {
            s |= STATUS_MST_ACTIVITY | STATUS_ACTIVITY;
        }
        if self.net.t_addressed {
            s |= STATUS_SLV_ACTIVITY | STATUS_ACTIVITY;
        }
        s
    }

    /// `IC_DATA_CMD` read in net mode: pop the RX FIFO.
    pub(super) fn net_pop_rx(&self) -> u32 {
        self.net.rx.borrow_mut().pop_front().unwrap_or(0)
    }

    fn net_push_rx(&self, value: u32) {
        let mut rx = self.net.rx.borrow_mut();
        if rx.len() < FIFO_DEPTH {
            rx.push_back(value);
        } else {
            drop(rx);
            self.set_raw(INTR_RX_OVER);
        }
    }

    /// Register writes in net mode. Returns `true` when handled here.
    pub(super) fn net_write(&mut self, offset: u64, value: u32) -> bool {
        let Some(pads) = self.net_pads() else {
            return false;
        };
        let now = self.net_now();
        pads.set_stamp(Some(now));
        let handled = match offset {
            super::IC_ENABLE => {
                let was = self.net_enabled();
                self.enable = value;
                if was && !self.net_enabled() {
                    self.net.bus.reset(&pads);
                    self.net.tx.clear();
                    self.net.m = MState::Idle;
                    self.net.t_addressed = false;
                    self.net.t_wait_tx = false;
                }
                self.net_kick(now, &pads);
                true
            }
            super::IC_DATA_CMD => {
                self.net_data_cmd(value, now, &pads);
                true
            }
            IC_SAR => {
                self.net.sar = value & 0x3FF;
                true
            }
            IC_RX_TL => {
                self.net.rx_tl = value & 0xFF;
                true
            }
            IC_TX_TL => {
                self.net.tx_tl = value & 0xFF;
                true
            }
            IC_SDA_HOLD => {
                self.net.sda_hold = value;
                true
            }
            _ => false,
        };
        self.net_process(now, &pads);
        pads.set_stamp(None);
        handled
    }

    fn net_data_cmd(&mut self, value: u32, now: u64, pads: &PadLines) {
        if !self.net_enabled() {
            return;
        }
        if self.raw_intr.get() & INTR_TX_ABRT != 0 {
            // Flushed until IC_CLR_TX_ABRT is read.
            return;
        }
        if self.net.tx.len() >= FIFO_DEPTH {
            return;
        }
        self.net.tx.push_back(value & 0x7FF);
        if self.net_is_master() {
            match self.net.m {
                MState::Idle => self.net_kick(now, pads),
                MState::Hold => self.net_master_next(now, pads),
                _ => {}
            }
        } else if self.net.t_wait_tx {
            self.net_slave_send(now, pads);
        }
    }

    // ── master ──────────────────────────────────────────────────────────

    fn net_kick(&mut self, now: u64, pads: &PadLines) {
        self.net.bus.timing = self.net_timing();
        if !self.net_enabled() || !self.net_is_master() || self.net.m != MState::Idle {
            return;
        }
        let Some(&first) = self.net.tx.front() else {
            return;
        };
        self.net.m_read = first & CMD_READ != 0;
        self.net.m = MState::Starting;
        self.set_raw(INTR_ACTIVITY);
        self.net.bus.m_start(now, pads);
    }

    /// The bus is held: the next command, a repeated START, or wait.
    fn net_master_next(&mut self, now: u64, pads: &PadLines) {
        let Some(&cmd) = self.net.tx.front() else {
            self.net.m = MState::Hold;
            return;
        };
        let read = cmd & CMD_READ != 0;
        if read != self.net.m_read || cmd & CMD_RESTART != 0 {
            // Clear RESTART so the command runs after the new address.
            if let Some(front) = self.net.tx.front_mut() {
                *front &= !CMD_RESTART;
            }
            self.net.m_read = read;
            self.net.m = MState::Starting;
            self.net.bus.m_start(now, pads);
            return;
        }
        self.net.tx.pop_front();
        self.net.cur = cmd;
        self.net.m = MState::Busy;
        if read {
            self.net.bus.m_recv(now, pads);
        } else {
            self.net.bus.m_send(cmd as u8, now, pads);
        }
    }

    fn net_abort(&mut self, source: u32, now: u64, pads: &PadLines) {
        self.tx_abrt_source.set(self.tx_abrt_source.get() | source);
        self.set_raw(INTR_TX_ABRT);
        self.net.tx.clear();
        self.net.m = MState::Stopping;
        self.net.bus.m_stop(now, pads);
    }

    fn net_on_master(&mut self, event: BusEvent, now: u64, pads: &PadLines) {
        match event {
            BusEvent::MStarted => {
                self.net.m = MState::Address;
                let addr = (((self.tar & 0x7F) as u8) << 1) | u8::from(self.net.m_read);
                self.net.bus.m_send(addr, now, pads);
            }
            BusEvent::MSent { acked } if self.net.m == MState::Address => {
                if acked {
                    self.net_master_next(now, pads);
                } else {
                    self.net_abort(ABRT_7B_ADDR_NOACK, now, pads);
                }
            }
            BusEvent::MSent { acked } => {
                if !acked {
                    self.net_abort(ABRT_TXDATA_NOACK, now, pads);
                } else {
                    self.net_master_after(now, pads);
                }
            }
            BusEvent::MByteIn(byte) => {
                self.net_push_rx(u32::from(byte));
                let ack = self.net.cur & CMD_STOP == 0;
                self.net.bus.m_ack(ack, now, pads);
            }
            BusEvent::MAckDone => self.net_master_after(now, pads),
            BusEvent::MStopped => {
                self.net.m = MState::Idle;
                self.set_raw(INTR_STOP_DET);
                self.activity.set(false);
                self.net_kick(now, pads);
            }
            BusEvent::MArbLost => {
                self.tx_abrt_source
                    .set(self.tx_abrt_source.get() | (1 << 12));
                self.set_raw(INTR_TX_ABRT);
                self.net.tx.clear();
                self.net.m = MState::Idle;
            }
            _ => {}
        }
    }

    /// A command's byte is done: STOP if it asked for one, else go on.
    fn net_master_after(&mut self, now: u64, pads: &PadLines) {
        if self.net.cur & CMD_STOP != 0 {
            self.net.m = MState::Stopping;
            self.net.bus.m_stop(now, pads);
        } else {
            self.net_master_next(now, pads);
        }
    }

    // ── slave ───────────────────────────────────────────────────────────

    fn net_slave_send(&mut self, now: u64, pads: &PadLines) {
        if let Some(cmd) = self.net.tx.pop_front() {
            self.net.t_wait_tx = false;
            self.net.bus.t_send(cmd as u8, now, pads);
        } else {
            self.net.t_wait_tx = true;
            self.set_raw(INTR_RD_REQ);
        }
    }

    fn net_on_slave(&mut self, event: BusEvent, now: u64, pads: &PadLines) {
        let slave =
            self.net_enabled() && !self.net_is_master() && self.con & CON_SLAVE_DISABLE == 0;
        if !slave {
            // A master sees its own START/STOP (and other masters') on the
            // wire; only a slave reports them, and only a slave answers.
            if let BusEvent::TAddress(_) = event {
                self.net.bus.t_ack(false, now, pads);
            }
            return;
        }
        match event {
            BusEvent::TStart => {
                self.set_raw(INTR_START_DET);
                self.net.t_wait_tx = false;
            }
            BusEvent::TStop => {
                let was = std::mem::take(&mut self.net.t_addressed);
                self.net.t_wait_tx = false;
                if was || self.con & CON_STOP_DET_IFADDRESSED == 0 {
                    self.set_raw(INTR_STOP_DET);
                }
            }
            BusEvent::TAddress(byte) => {
                let ours = self.net_enabled()
                    && !self.net_is_master()
                    && self.con & CON_SLAVE_DISABLE == 0
                    && u32::from(byte >> 1) == self.net.sar & 0x7F;
                self.net.bus.t_ack(ours, now, pads);
            }
            BusEvent::TAddressed { read } => {
                self.net.t_addressed = true;
                self.net.t_reading = read;
                self.set_raw(INTR_ACTIVITY);
                if read {
                    self.net_slave_send(now, pads);
                } else {
                    self.net.t_first = true;
                    self.net.bus.t_continue(now, pads);
                }
            }
            BusEvent::TByteIn(byte) => {
                let first = std::mem::take(&mut self.net.t_first);
                let flag = if first { FIRST_DATA_BYTE } else { 0 };
                self.net_push_rx(u32::from(byte) | flag);
                self.net.bus.t_ack(true, now, pads);
            }
            BusEvent::TByteOut { acked } => {
                if acked {
                    self.net_slave_send(now, pads);
                } else {
                    self.set_raw(INTR_RX_DONE);
                }
            }
            _ => {}
        }
    }

    fn net_process(&mut self, now: u64, pads: &PadLines) {
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
                    | BusEvent::MArbLost => self.net_on_master(event, now, pads),
                    _ => self.net_on_slave(event, now, pads),
                }
            }
        }
        self.net.bus.settle(pads);
    }

    /// The net delivered a new level on SCL or SDA at this chip's pad.
    pub(super) fn net_edge(&mut self, line: usize, level: bool, now: u64) -> bool {
        if !self.net_mode() {
            return false;
        }
        let Some(pads) = self.net_pads() else {
            return false;
        };
        self.net.bus.timing = self.net_timing();
        pads.set_stamp(Some(now));
        self.net.bus.on_edge(line, level, now, &pads);
        self.net_process(now, &pads);
        pads.set_stamp(None);
        self.net.bus.deadline().is_some() || self.irq_pending()
    }

    fn net_step(&mut self, now: u64) {
        let Some(pads) = self.net_pads() else {
            return;
        };
        self.net.bus.timing = self.net_timing();
        while let Some(due) = self.net.bus.deadline() {
            if due > now {
                break;
            }
            self.net.bus.step(due, &pads);
            pads.set_stamp(Some(due));
            self.net_process(due, &pads);
        }
        pads.set_stamp(None);
    }

    /// Walk path: one tick.
    pub(super) fn net_tick(&mut self) {
        let now = self.net_now();
        if self.net.bus.deadline().is_some_and(|d| d <= now) {
            self.net_step(now);
        }
    }

    /// Scheduler path: the engine's next timed step (delay from now).
    pub(super) fn net_take_events(&mut self) -> Vec<(u64, u32)> {
        let now = self.net_now();
        match self.net.bus.deadline() {
            Some(due) => {
                let due = due.max(now + 1);
                if self.net.armed.contains(&due) {
                    return Vec::new();
                }
                self.net.armed.push(due);
                vec![(due - now - 1, NET_TOKEN)]
            }
            None => Vec::new(),
        }
    }

    /// Scheduler path: an armed event fired at `now`.
    pub(super) fn net_on_event(&mut self, now: u64) -> crate::sched::EventResult {
        self.net.armed.retain(|&t| t > now);
        self.net_step(now);
        let mut res = crate::sched::EventResult {
            raise_own_irq: self.irq_pending(),
            ..Default::default()
        };
        if let Some(due) = self.net.bus.deadline() {
            let due = due.max(now + 1);
            if !self.net.armed.contains(&due) {
                self.net.armed.push(due);
                res.reschedule_delay = Some(due - now);
            }
        }
        res
    }

    /// Read-to-clear registers in net mode.
    pub(super) fn net_read_clear(&self, offset: u64) -> Option<u32> {
        let bits = match offset {
            IC_CLR_INTR => !0,
            IC_CLR_RX_OVER => INTR_RX_OVER,
            IC_CLR_RD_REQ => INTR_RD_REQ,
            IC_CLR_RX_DONE => INTR_RX_DONE,
            IC_CLR_START_DET => INTR_START_DET,
            _ => return None,
        };
        if offset == IC_CLR_INTR {
            self.tx_abrt_source.set(0);
        }
        self.clr_raw(bits);
        Some(0)
    }

    /// Plain registers this file adds, for reads.
    pub(super) fn net_read_reg(&self, offset: u64) -> Option<u32> {
        Some(match offset {
            IC_SAR => self.net.sar,
            IC_RX_TL => self.net.rx_tl,
            IC_TX_TL => self.net.tx_tl,
            IC_SDA_HOLD => self.net.sda_hold,
            super::IC_TXFLR => self.net.tx.len() as u32,
            super::IC_RXFLR => self.net.rx.borrow().len() as u32,
            _ => return None,
        })
    }
}
