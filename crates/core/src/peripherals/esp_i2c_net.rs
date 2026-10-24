// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The Espressif command-list I²C on a world `gpio_net`: a bit-level master
//! and slave between chips, on the shared wire engine
//! ([`crate::peripherals::i2c_bitbus`]).
//!
//! Off a net the controller models exchange bytes with byte-level device
//! models (`esp32c3::i2c`, which the ESP32-C6 also uses, runs its own
//! cycle-accurate engine for the waveform; `esp32s3::i2c` and `esp32::i2c`
//! run a command list instantly). The C3/C6, the S3 and the classic ESP32
//! all host this engine on a net; the classic part names its own opcodes and
//! has no `SCL_STRETCH_CONF`. When the GPIO matrix routes SCL or
//! SDA to a pad on a net ([`PadLines::on_net`]) this engine owns the wire. It
//! is chip-neutral: the host answers the [`EspI2cHost`] calls from its own
//! register file and names its COMD opcodes ([`Opcodes`]); the register
//! offsets and bits used here are the family's (`esp_i2c_core`).
//!
//! * **Master** (`CTR.MS_MODE` = 1). `CTR.TRANS_START` runs `COMD0..` in
//!   order on the wire: `RSTART` puts START (or a repeated START) out;
//!   `WRITE n` sends `n` bytes from the TX FIFO (the address byte is the first
//!   of them, as esp-idf and esp-hal queue it) and, with `ack_check_en`,
//!   compares each ACK slot with `ack_exp`; `READ n` clocks `n` bytes into
//!   the RX FIFO, answering each with `ack_value`; `STOP` sends STOP and
//!   latches `TRANS_COMPLETE`; `END` holds SCL low with `END_DETECT` until the
//!   next `TRANS_START`, which runs the list again from `COMD0`. Each command
//!   sets its `command_done` bit. An ACK slot that does not match latches
//!   `NACK`, clears `SR.RESP_REC` and ends the transfer with STOP (on silicon
//!   esp-hal and esp-idf reset the controller after a NACK, which frees the
//!   bus as well; the STOP keeps the other chips' view of the bus sane
//!   without modelling that reset). `SR.RESP_REC` is set by every ACK the
//!   master receives. The SCL high period is counted from when SCL is seen
//!   high, so a target stretching the clock holds the master. A released SDA
//!   read back low while sending is lost arbitration (`ARBITRATION_LOST`).
//!   A `WRITE` that finds the TX FIFO empty holds SCL low until firmware
//!   writes `DATA`.
//! * **Slave** (`MS_MODE` = 0). It ACKs its 7-bit `SLAVE_ADDR`, sets
//!   `SR.SLAVE_ADDRESSED` and `SR.SLAVE_RW`; written bytes go to the RX FIFO
//!   (`RXFIFO_WM` at the `FIFO_CONF` threshold; a full FIFO NACKs with
//!   `RXFIFO_OVF`); a read is served from the TX FIFO. With the TX FIFO empty
//!   and `SCL_STRETCH_CONF.SLAVE_SCL_STRETCH_EN` set it holds SCL low with
//!   `SLAVE_STRETCH` (`SR.STRETCH_CAUSE` = 1) until firmware writes
//!   `SLAVE_SCL_STRETCH_CLR`; without stretching an empty FIFO sends `0xFF`.
//!   STOP after it was addressed latches `TRANS_COMPLETE`.
//!
//! Not modelled on a net: 10-bit addressing, the general call, address
//! broadcasting, `SLAVE_BYTE_ACK_CTL`, the FIFO-address (`FIFO_ADDR_CFG_EN`)
//! mode, `NONFIFO` mode, the filters and the SCL/FSM timeouts.

use crate::peripherals::i2c_bitbus::{BitBus, BusEvent, Timing};
use crate::peripherals::pad_lines::PadLines;

pub(crate) const INT_RXFIFO_WM: u32 = 1 << 0;
pub(crate) const INT_RXFIFO_OVF: u32 = 1 << 2;
pub(crate) const INT_ARBITRATION_LOST: u32 = 1 << 5;
pub(crate) const INT_TRANS_COMPLETE: u32 = 1 << 7;
pub(crate) const INT_NACK: u32 = 1 << 10;
pub(crate) const INT_SLAVE_STRETCH: u32 = 1 << 16;

pub(crate) const SR_RESP_REC: u32 = 1 << 0;
pub(crate) const SR_SLAVE_RW: u32 = 1 << 1;
pub(crate) const SR_ARB_LOST: u32 = 1 << 3;
pub(crate) const SR_BUS_BUSY: u32 = 1 << 4;
pub(crate) const SR_SLAVE_ADDRESSED: u32 = 1 << 5;
/// `SR.STRETCH_CAUSE` [15:14].
pub(crate) const SR_STRETCH_CAUSE_SHIFT: u32 = 14;

const CTR_RX_FULL_ACK_LEVEL: u32 = 1 << 3;
const CTR_MS_MODE: u32 = 1 << 4;
/// `SCL_STRETCH_CONF.SLAVE_SCL_STRETCH_EN`.
pub(crate) const STRETCH_EN: u32 = 1 << 10;
/// `SCL_STRETCH_CONF.SLAVE_SCL_STRETCH_CLR` (write 1 to release SCL).
pub(crate) const STRETCH_CLR: u32 = 1 << 11;

const CMD_ACK_CHECK_EN: u32 = 1 << 8;
const CMD_ACK_EXP: u32 = 1 << 9;
const CMD_ACK_VALUE: u32 = 1 << 10;
const CMD_DONE: u32 = 1 << 31;

/// Scheduler token for the net engine.
pub(crate) const NET_TOKEN: u32 = 0x4932_0000;

/// A part's COMD opcode numbers (they differ between the classic ESP32 and
/// the C3/C6/S3; see `esp_i2c_core`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Opcodes {
    pub(crate) rstart: u32,
    pub(crate) write: u32,
    pub(crate) read: u32,
    pub(crate) stop: u32,
    pub(crate) end: u32,
}

/// ESP32-C3 / C6 / S3 opcodes (`i2c0::comd`): 6 RSTART, 1 WRITE, 3 READ,
/// 2 STOP, 4 END.
pub(crate) const OPCODES_C3: Opcodes = Opcodes {
    rstart: 6,
    write: 1,
    read: 3,
    stop: 2,
    end: 4,
};

/// What the engine needs from the controller model that hosts it.
pub(crate) trait EspI2cHost {
    /// The part's COMD opcode numbers.
    fn opcodes(&self) -> Opcodes {
        OPCODES_C3
    }
    fn ctr(&self) -> u32;
    fn slave_addr(&self) -> u32;
    fn fifo_conf(&self) -> u32;
    fn stretch_conf(&self) -> u32;
    fn cmd(&self, i: usize) -> u32;
    fn set_cmd(&mut self, i: usize, value: u32);
    fn cmd_count(&self) -> usize;
    fn tx_pop(&mut self) -> Option<u8>;
    fn rx_push(&mut self, byte: u8) -> bool;
    fn rx_len(&self) -> usize;
    /// Latch bits in `INT_RAW`.
    fn raise(&mut self, bits: u32);
    /// Set and clear bits of the engine-owned part of `SR`.
    fn sr_update(&mut self, set: u32, clear: u32);
    /// Bus timing in CPU cycles, from the timing registers.
    fn timing(&self) -> Timing;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum M {
    #[default]
    Idle,
    /// RSTART on its way.
    Starting,
    /// A WRITE's byte is on the wire.
    Sending,
    /// A WRITE waits for the TX FIFO (SCL held low).
    TxStall,
    /// A READ's byte is on the wire, or its ACK slot.
    Receiving,
    /// STOP on its way; `nack` when it ends a NACKed transfer.
    Stopping { nack: bool },
    /// END: SCL held low until the next `TRANS_START`.
    Paused,
}

/// The engine state, kept beside the controller's register file.
#[derive(Debug, Default)]
pub(crate) struct EspI2cNet {
    bus: BitBus,
    m: M,
    cmd: usize,
    left: usize,
    // slave
    t_addressed: bool,
    /// Waiting for `SLAVE_SCL_STRETCH_CLR` before the next byte goes out.
    t_wait_clr: bool,
    armed: Vec<u64>,
    master_transfers: u64,
    slave_transfers: u64,
}

impl EspI2cNet {
    /// The cycle of the next timed step.
    pub(crate) fn deadline(&self) -> Option<u64> {
        self.bus.deadline()
    }

    /// `true` while the master runs a command on the wire, including the
    /// moments it waits for the net to show SCL high (no deadline then: a
    /// host on the per-cycle walk must keep ticking it).
    pub(crate) fn master_running(&self) -> bool {
        !matches!(self.m, M::Idle | M::Paused | M::TxStall)
    }

    /// Transfers the master ended with STOP.
    pub(crate) fn master_transfers(&self) -> u64 {
        self.master_transfers
    }

    /// Transfers that addressed the slave and ended with STOP.
    pub(crate) fn slave_transfers(&self) -> u64 {
        self.slave_transfers
    }

    fn is_master(host: &dyn EspI2cHost) -> bool {
        host.ctr() & CTR_MS_MODE != 0
    }

    /// `CTR.FSM_RST`: let go of both lines and forget the transfer.
    pub(crate) fn reset(&mut self, pads: &PadLines) {
        self.bus.reset(pads);
        self.m = M::Idle;
        self.t_addressed = false;
        self.t_wait_clr = false;
    }

    // ── master ──────────────────────────────────────────────────────────

    /// `CTR.TRANS_START`: run the command list from `COMD0`.
    pub(crate) fn start(&mut self, host: &mut dyn EspI2cHost, now: u64, pads: &PadLines) {
        if !Self::is_master(host) || !matches!(self.m, M::Idle | M::Paused) {
            return;
        }
        self.bus.timing = host.timing();
        host.sr_update(0, SR_RESP_REC);
        self.cmd = 0;
        self.exec(host, now, pads);
    }

    /// `DATA` written: a WRITE waiting for its byte goes on.
    pub(crate) fn tx_written(&mut self, host: &mut dyn EspI2cHost, now: u64, pads: &PadLines) {
        // (A stretching slave waits for SLAVE_SCL_STRETCH_CLR, not for DATA.)
        if self.m == M::TxStall {
            self.send_next(host, now, pads);
        }
    }

    fn done(&mut self, host: &mut dyn EspI2cHost) {
        let i = self.cmd;
        if i < host.cmd_count() {
            host.set_cmd(i, host.cmd(i) | CMD_DONE);
        }
        self.cmd += 1;
    }

    /// Execute the command at `self.cmd`.
    fn exec(&mut self, host: &mut dyn EspI2cHost, now: u64, pads: &PadLines) {
        let ops = host.opcodes();
        loop {
            if self.cmd >= host.cmd_count() {
                // The list ran out without STOP or END.
                self.m = if self.bus.master_active() {
                    M::Paused
                } else {
                    M::Idle
                };
                host.raise(INT_TRANS_COMPLETE);
                return;
            }
            let word = host.cmd(self.cmd);
            let op = (word >> 11) & 7;
            let n = (word & 0xFF) as usize;
            if op == ops.rstart {
                self.m = M::Starting;
                self.bus.m_start(now, pads);
                return;
            } else if op == ops.write || op == ops.read {
                if n == 0 {
                    self.done(host);
                    continue;
                }
                self.left = n;
                if op == ops.write {
                    self.send_next(host, now, pads);
                } else {
                    self.m = M::Receiving;
                    if !self.bus.m_recv(now, pads) {
                        self.abandon(host);
                    }
                }
                return;
            } else if op == ops.stop {
                self.m = M::Stopping { nack: false };
                if !self.bus.m_stop(now, pads) {
                    self.done(host);
                    self.m = M::Idle;
                    host.raise(INT_TRANS_COMPLETE);
                }
                return;
            } else if op == ops.end {
                self.done(host);
                self.m = M::Paused;
                host.raise(crate::peripherals::esp_i2c_core::INT_END_DETECT);
                return;
            } else {
                self.abandon(host);
                return;
            }
        }
    }

    /// A command that cannot run on the bus as it is (data with no START
    /// before it, a reserved opcode): end the list.
    fn abandon(&mut self, host: &mut dyn EspI2cHost) {
        self.m = if self.bus.master_active() {
            M::Paused
        } else {
            M::Idle
        };
        host.raise(INT_TRANS_COMPLETE);
    }

    fn send_next(&mut self, host: &mut dyn EspI2cHost, now: u64, pads: &PadLines) {
        match host.tx_pop() {
            Some(byte) => {
                self.m = M::Sending;
                if !self.bus.m_send(byte, now, pads) {
                    self.abandon(host);
                }
            }
            None => self.m = M::TxStall,
        }
    }

    fn on_master(&mut self, host: &mut dyn EspI2cHost, event: BusEvent, now: u64, pads: &PadLines) {
        match event {
            BusEvent::MStarted => {
                self.done(host);
                self.exec(host, now, pads);
            }
            BusEvent::MSent { acked } => {
                if acked {
                    host.sr_update(SR_RESP_REC, 0);
                }
                let word = host.cmd(self.cmd.min(host.cmd_count().saturating_sub(1)));
                let check = word & CMD_ACK_CHECK_EN != 0;
                let expected_ack = word & CMD_ACK_EXP == 0;
                if check && acked != expected_ack {
                    host.sr_update(0, SR_RESP_REC);
                    host.raise(INT_NACK);
                    self.m = M::Stopping { nack: true };
                    self.bus.m_stop(now, pads);
                    return;
                }
                self.left -= 1;
                if self.left > 0 {
                    self.send_next(host, now, pads);
                } else {
                    self.done(host);
                    self.exec(host, now, pads);
                }
            }
            BusEvent::MByteIn(byte) => {
                if !host.rx_push(byte) {
                    host.raise(INT_RXFIFO_OVF);
                }
                let word = host.cmd(self.cmd.min(host.cmd_count().saturating_sub(1)));
                self.bus.m_ack(word & CMD_ACK_VALUE == 0, now, pads);
            }
            BusEvent::MAckDone => {
                self.left -= 1;
                if self.left > 0 {
                    self.bus.m_recv(now, pads);
                } else {
                    self.done(host);
                    self.exec(host, now, pads);
                }
            }
            BusEvent::MStopped => {
                let nack = matches!(self.m, M::Stopping { nack: true });
                self.m = M::Idle;
                self.master_transfers += 1;
                if !nack {
                    self.done(host);
                    host.raise(INT_TRANS_COMPLETE);
                }
            }
            BusEvent::MArbLost => {
                self.m = M::Idle;
                host.sr_update(SR_ARB_LOST, 0);
                host.raise(INT_ARBITRATION_LOST);
            }
            _ => {}
        }
    }

    // ── slave ───────────────────────────────────────────────────────────

    fn slave_send(&mut self, host: &mut dyn EspI2cHost, now: u64, pads: &PadLines) {
        if let Some(byte) = host.tx_pop() {
            self.bus.t_send(byte, now, pads);
        } else if host.stretch_conf() & STRETCH_EN != 0 {
            self.t_wait_clr = true;
            host.sr_update(1 << SR_STRETCH_CAUSE_SHIFT, 3 << SR_STRETCH_CAUSE_SHIFT);
            host.raise(INT_SLAVE_STRETCH);
        } else {
            self.bus.t_send(0xFF, now, pads);
        }
    }

    /// `SCL_STRETCH_CONF.SLAVE_SCL_STRETCH_CLR` written: a stretching slave
    /// lets SCL go with the byte now at the head of the TX FIFO.
    pub(crate) fn stretch_clear(&mut self, host: &mut dyn EspI2cHost, now: u64, pads: &PadLines) {
        if !std::mem::take(&mut self.t_wait_clr) {
            return;
        }
        let byte = host.tx_pop().unwrap_or(0xFF);
        self.bus.t_send(byte, now, pads);
    }

    fn on_slave(&mut self, host: &mut dyn EspI2cHost, event: BusEvent, now: u64, pads: &PadLines) {
        let slave = !Self::is_master(host);
        match event {
            BusEvent::TStart => {
                self.t_wait_clr = false;
            }
            BusEvent::TStop => {
                self.t_wait_clr = false;
                if std::mem::take(&mut self.t_addressed) {
                    host.sr_update(0, SR_SLAVE_ADDRESSED);
                    self.slave_transfers += 1;
                    host.raise(INT_TRANS_COMPLETE);
                }
            }
            BusEvent::TAddress(byte) => {
                let ours = slave && u32::from(byte >> 1) == host.slave_addr() & 0x7F;
                self.bus.t_ack(ours, now, pads);
            }
            BusEvent::TAddressed { read } => {
                self.t_addressed = true;
                host.sr_update(
                    SR_SLAVE_ADDRESSED | if read { SR_SLAVE_RW } else { 0 },
                    if read { 0 } else { SR_SLAVE_RW },
                );
                if read {
                    self.slave_send(host, now, pads);
                } else {
                    self.bus.t_continue(now, pads);
                }
            }
            BusEvent::TByteIn(byte) => {
                let full = host.rx_len() >= crate::peripherals::esp_i2c_core::FIFO_CAPACITY;
                if full {
                    host.raise(INT_RXFIFO_OVF);
                    let ack = host.ctr() & CTR_RX_FULL_ACK_LEVEL == 0;
                    self.bus.t_ack(ack, now, pads);
                    return;
                }
                host.rx_push(byte);
                let wm = (host.fifo_conf() & 0x1F) as usize;
                if host.rx_len() >= wm.max(1) {
                    host.raise(INT_RXFIFO_WM);
                }
                self.bus.t_ack(true, now, pads);
            }
            BusEvent::TByteOut { acked: true } => self.slave_send(host, now, pads),
            _ => {}
        }
    }

    fn process(&mut self, host: &mut dyn EspI2cHost, now: u64, pads: &PadLines) {
        loop {
            let events = self.bus.take_events();
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
                    | BusEvent::MArbLost => self.on_master(host, event, now, pads),
                    _ => self.on_slave(host, event, now, pads),
                }
            }
        }
        self.bus.settle(pads);
        let busy = self.bus.master_active() || self.t_addressed;
        if busy {
            host.sr_update(SR_BUS_BUSY, 0);
        } else {
            host.sr_update(0, SR_BUS_BUSY);
        }
    }

    /// After a register write handled by the host: report what the bus did.
    pub(crate) fn after_write(&mut self, host: &mut dyn EspI2cHost, now: u64, pads: &PadLines) {
        self.process(host, now, pads);
    }

    /// The net delivered a new level on SCL or SDA.
    pub(crate) fn on_edge(
        &mut self,
        host: &mut dyn EspI2cHost,
        line: usize,
        level: bool,
        now: u64,
        pads: &PadLines,
    ) {
        self.bus.timing = host.timing();
        self.bus.on_edge(line, level, now, pads);
        self.process(host, now, pads);
    }

    /// Run every timed step due by `now`, each at its own cycle.
    pub(crate) fn step(&mut self, host: &mut dyn EspI2cHost, now: u64, pads: &PadLines) {
        while let Some(due) = self.bus.deadline() {
            if due > now {
                break;
            }
            self.bus.step(due, pads);
            pads.set_stamp(Some(due));
            self.process(host, due, pads);
        }
        pads.set_stamp(None);
    }

    /// The engine's next timed step as a scheduler delay from `now`.
    pub(crate) fn take_event(&mut self, now: u64) -> Option<(u64, u32)> {
        let due = self.bus.deadline()?.max(now + 1);
        if self.armed.contains(&due) {
            return None;
        }
        self.armed.push(due);
        Some((due - now - 1, NET_TOKEN))
    }

    /// An armed event fired at `now`.
    pub(crate) fn on_event(
        &mut self,
        host: &mut dyn EspI2cHost,
        now: u64,
        pads: &PadLines,
    ) -> Option<u64> {
        self.armed.retain(|&t| t > now);
        self.step(host, now, pads);
        let due = self.bus.deadline()?.max(now + 1);
        if self.armed.contains(&due) {
            return None;
        }
        self.armed.push(due);
        Some(due - now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c_bitbus::{SCL, SDA};
    use crate::peripherals::pad_lines::LineDrive;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct Host {
        ctr: u32,
        slave_addr: u32,
        stretch: u32,
        cmds: [u32; 8],
        tx: VecDeque<u8>,
        rx: VecDeque<u8>,
        int_raw: u32,
        sr: u32,
    }

    impl EspI2cHost for Host {
        fn ctr(&self) -> u32 {
            self.ctr
        }
        fn slave_addr(&self) -> u32 {
            self.slave_addr
        }
        fn fifo_conf(&self) -> u32 {
            1
        }
        fn stretch_conf(&self) -> u32 {
            self.stretch
        }
        fn cmd(&self, i: usize) -> u32 {
            self.cmds[i]
        }
        fn set_cmd(&mut self, i: usize, value: u32) {
            self.cmds[i] = value;
        }
        fn cmd_count(&self) -> usize {
            8
        }
        fn tx_pop(&mut self) -> Option<u8> {
            self.tx.pop_front()
        }
        fn rx_push(&mut self, byte: u8) -> bool {
            self.rx.push_back(byte);
            true
        }
        fn rx_len(&self) -> usize {
            self.rx.len()
        }
        fn raise(&mut self, bits: u32) {
            self.int_raw |= bits;
        }
        fn sr_update(&mut self, set: u32, clear: u32) {
            self.sr = (self.sr & !clear) | set;
        }
        fn timing(&self) -> Timing {
            Timing::new(8, 8, 2)
        }
    }

    fn pads() -> PadLines {
        let p = PadLines::new(&["SCL", "SDA"], &[true, true]);
        p.set_mode(SCL, LineDrive::OpenDrain);
        p.set_mode(SDA, LineDrive::OpenDrain);
        p
    }

    fn cmd(op: u32, n: u32, flags: u32) -> u32 {
        (op << 11) | n | flags
    }

    /// A master and a slave engine on one wired-AND bus with zero latency.
    struct Pair {
        a: EspI2cNet,
        b: EspI2cNet,
        ha: Host,
        hb: Host,
        pa: PadLines,
        pb: PadLines,
        level: [bool; 2],
        now: u64,
    }

    impl Pair {
        fn new() -> Self {
            let mut p = Self {
                a: EspI2cNet::default(),
                b: EspI2cNet::default(),
                ha: Host {
                    ctr: CTR_MS_MODE,
                    ..Default::default()
                },
                hb: Host {
                    slave_addr: 0x42,
                    ..Default::default()
                },
                pa: pads(),
                pb: pads(),
                level: [true, true],
                now: 0,
            };
            for line in [SCL, SDA] {
                p.pa.set_input(line, true);
                p.pb.set_input(line, true);
                p.a.on_edge(&mut p.ha, line, true, 0, &p.pa);
                p.b.on_edge(&mut p.hb, line, true, 0, &p.pb);
            }
            p
        }

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
                        self.a
                            .on_edge(&mut self.ha, line, level, self.now, &self.pa);
                        self.b
                            .on_edge(&mut self.hb, line, level, self.now, &self.pb);
                    }
                }
                if !moved {
                    return;
                }
            }
        }

        fn run(&mut self) {
            for _ in 0..100_000 {
                self.settle();
                let due = match (self.a.deadline(), self.b.deadline()) {
                    (Some(x), Some(y)) => x.min(y),
                    (Some(x), None) | (None, Some(x)) => x,
                    (None, None) => return,
                };
                self.now = due;
                self.a.step(&mut self.ha, due, &self.pa);
                self.b.step(&mut self.hb, due, &self.pb);
            }
            panic!("runaway");
        }
    }

    #[test]
    fn a_command_list_writes_then_reads_back_through_a_repeated_start() {
        let mut p = Pair::new();
        let ops = OPCODES_C3;
        // Write 0x84 (0x42 W), 0x10, 0xDE; STOP.
        p.ha.cmds[0] = cmd(ops.rstart, 0, 0);
        p.ha.cmds[1] = cmd(ops.write, 3, CMD_ACK_CHECK_EN);
        p.ha.cmds[2] = cmd(ops.stop, 0, 0);
        p.ha.tx.extend([0x84, 0x10, 0xDE]);
        p.a.start(&mut p.ha, 0, &p.pa);
        p.a.after_write(&mut p.ha, 0, &p.pa);
        p.run();
        assert_eq!(p.hb.rx, [0x10, 0xDE]);
        assert_ne!(p.ha.int_raw & INT_TRANS_COMPLETE, 0);
        assert_eq!(p.ha.int_raw & INT_NACK, 0);
        assert_ne!(p.ha.sr & SR_RESP_REC, 0);
        assert_ne!(p.hb.int_raw & INT_TRANS_COMPLETE, 0, "the slave saw STOP");
        assert!(p.ha.cmds[..3].iter().all(|c| c & CMD_DONE != 0));

        // Read two bytes: 0x85, READ 1 ACK, READ 1 NACK, STOP.
        p.ha.int_raw = 0;
        p.hb.tx.extend([0xDE, 0xAD]);
        p.ha.cmds = [0; 8];
        p.ha.cmds[0] = cmd(ops.rstart, 0, 0);
        p.ha.cmds[1] = cmd(ops.write, 1, CMD_ACK_CHECK_EN);
        p.ha.cmds[2] = cmd(ops.read, 1, 0);
        p.ha.cmds[3] = cmd(ops.read, 1, CMD_ACK_VALUE);
        p.ha.cmds[4] = cmd(ops.stop, 0, 0);
        p.ha.tx.push_back(0x85);
        p.a.start(&mut p.ha, p.now, &p.pa);
        p.a.after_write(&mut p.ha, p.now, &p.pa);
        p.run();
        assert_eq!(p.ha.rx, [0xDE, 0xAD]);
        assert_ne!(p.ha.int_raw & INT_TRANS_COMPLETE, 0);
        assert!(p.level[SCL] && p.level[SDA], "bus idle");
    }

    #[test]
    fn a_nack_latches_the_interrupt_and_frees_the_bus() {
        let mut p = Pair::new();
        let ops = OPCODES_C3;
        p.ha.cmds[0] = cmd(ops.rstart, 0, 0);
        p.ha.cmds[1] = cmd(ops.write, 2, CMD_ACK_CHECK_EN);
        p.ha.cmds[2] = cmd(ops.stop, 0, 0);
        p.ha.tx.extend([0xA0, 0x55]); // 0x50: nobody there
        p.a.start(&mut p.ha, 0, &p.pa);
        p.a.after_write(&mut p.ha, 0, &p.pa);
        p.run();
        assert_ne!(p.ha.int_raw & INT_NACK, 0);
        assert_eq!(p.ha.int_raw & INT_TRANS_COMPLETE, 0);
        assert_eq!(p.ha.sr & SR_RESP_REC, 0);
        assert_eq!(p.ha.tx, [0x55], "the data byte never went out");
        assert!(p.level[SCL] && p.level[SDA], "STOP freed the bus");
        assert!(p.hb.rx.is_empty());
    }

    #[test]
    fn a_slave_with_an_empty_fifo_stretches_until_firmware_clears_it() {
        let mut p = Pair::new();
        p.hb.stretch = STRETCH_EN;
        let ops = OPCODES_C3;
        p.ha.cmds[0] = cmd(ops.rstart, 0, 0);
        p.ha.cmds[1] = cmd(ops.write, 1, CMD_ACK_CHECK_EN);
        p.ha.cmds[2] = cmd(ops.read, 1, CMD_ACK_VALUE);
        p.ha.cmds[3] = cmd(ops.stop, 0, 0);
        p.ha.tx.push_back(0x85);
        p.a.start(&mut p.ha, 0, &p.pa);
        p.a.after_write(&mut p.ha, 0, &p.pa);
        p.run();
        assert_ne!(p.hb.int_raw & INT_SLAVE_STRETCH, 0);
        assert!(!p.level[SCL], "SCL held low by the slave");
        assert!(p.ha.rx.is_empty());
        p.hb.tx.push_back(0x5A);
        p.b.stretch_clear(&mut p.hb, p.now, &p.pb);
        p.b.after_write(&mut p.hb, p.now, &p.pb);
        p.run();
        assert_eq!(p.ha.rx, [0x5A]);
        assert_ne!(p.ha.int_raw & INT_TRANS_COMPLETE, 0);
    }
}
