// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA I2C bus interface (RIIC), master mode.
//!
//! Register layout from FSP `R7FA8M1AH.h` (`R_IIC0_Type`, the same block on
//! the RA6M5): byte registers `ICCR1` +0x00, `ICCR2` +0x01, `ICMR1` +0x02,
//! `ICMR2` +0x03, `ICMR3` +0x04, `ICFER` +0x05, `ICSER` +0x06, `ICIER` +0x07,
//! `ICSR1` +0x08, `ICSR2` +0x09, `SARL0`..`SARU2` +0x0A..+0x0F, `ICBRL`
//! +0x10, `ICBRH` +0x11, `ICDRT` +0x12, `ICDRR` +0x13, `ICWUR` +0x16,
//! `ICWUR2` +0x17. One channel per 0x100 window. Behaviour from FSP
//! `r_iic_master.c`, which Zephyr's `i2c_renesas_ra_iic.c` drives.
//!
//! Modelled (master transmit and master receive, 7-bit addresses):
//! - `ICCR2.ST` / `RS` / `SP` issue start, restart and stop conditions. The
//!   bus state follows them: `BBSY`, `MST`, `TRS`; `ICSR2.START` / `STOP`.
//! - `ICDRT` writes in master transmit mode: the byte moves into the shift
//!   register at once when it is idle (`TDRE` goes back to 1) or waits in
//!   `ICDRT` until the byte being sent ends. The first byte after a
//!   (re)start is the address: an address no attached device claims is
//!   NACKed (`NACKF`, `ACKBR`) and the transfer stops; a read address turns
//!   the bus around (`TRS` = 0) and sets `RDRF` for the dummy `ICDRR` read.
//!   `TEND` goes 1 when a byte ends with nothing waiting in `ICDRT`.
//! - Master receive: each `ICDRR` read clocks the next byte from the
//!   selected device, which lands in `ICDRR` one byte time later with `RDRF`
//!   set. A stop or restart requested while a received byte is still unread
//!   is issued once it is read (FSP sets `SP` before it reads the last
//!   byte, as the hardware manual's receive flow does).
//! - `ICMR3.ACKBT` is writable only with `ACKWP` set in the same write and
//!   clears at a stop; `ICCR1.IICRST` resets the registers (`ICE` = 0) or
//!   the bus state and flags (`ICE` = 1); `ICSR2` flags 0-4 are
//!   write-0-to-clear.
//! - Timing: one SCL period is `(BRH + BRL + 2 * c)` cycles of
//!   `PCLKB / 2^CKS` (`c` = 3 + noise-filter stages for CKS = 0, else 2 +
//!   stages: the constant Zephyr's bit-rate calculation uses; rise and fall
//!   times are not added), a byte with its acknowledge 9 periods, a start,
//!   restart or stop one period, counted in `clock_hz` cycles (the rate the
//!   guest believes its cycles run at, the `ra_agt` convention).
//!
//! Interrupts are ELC events, not NVIC lines: RXI (`elc_event`), TXI (+1),
//! TEI (+2) and ERI (+3), as in FSP `bsp_elc.h` (RA8M1 `ELC_EVENT_IIC1_RXI`
//! 0x061; RA6M5 0x078). Each is raised on the rising edge of its flag ANDed
//! with its enable: TXI `TDRE & TIE`, TEI `TEND & TEIE`, RXI `RDRF & RIE`,
//! ERI any of `NACKF/STOP/START/AL/TMOF` with `NAKIE/SPIE/STIE/ALIE/TMOIE`
//! (the `ICSR2` and `ICIER` bits line up). An enable set while its flag is
//! already 1 is an edge too: FSP's restart sets `TIE` with `TDRE` = 1 and
//! relies on the TXI it queues. The ICU latches the event in `IELSRn.IR`
//! ([`super::ra_icu`]); FSP's TEI and ERI handlers clear `IR` at their END,
//! so an event raised inside one would be lost — the wire time between a
//! request and its condition is what keeps silicon (and this model) clear
//! of that.
//!
//! NOT modelled: slave mode (`SARLn/SARUn`, `ICSER` are stored only), 10-bit
//! addresses, a data byte NACKed by the device (devices always ACK data),
//! arbitration loss, the SCL-low timeout (`TMOF`), SMBus, wake-up
//! (`ICWUR*`), `RDRFS`, the `WAIT` hold timing (reception already waits for
//! each `ICDRR` read), the `ICCR1` SCL/SDA output overrides and extra-clock
//! (`CLO`; the line monitors read high), `ICMR1.BC`, DTC transfers, and any
//! skew when firmware moves PCLKB or the CPU clock away from the chip YAML.

use std::cell::Cell;

use crate::peripherals::device::{drain_i2c_pin_drives, I2cDevice};
use crate::{CycleClock, Peripheral, SimResult};

const ICCR1: u64 = 0x00;
const ICCR2: u64 = 0x01;
const ICMR1: u64 = 0x02;
const ICMR2: u64 = 0x03;
const ICMR3: u64 = 0x04;
const ICFER: u64 = 0x05;
const ICSER: u64 = 0x06;
const ICIER: u64 = 0x07;
const ICSR1: u64 = 0x08;
const ICSR2: u64 = 0x09;
const SARL0: u64 = 0x0A;
const SARU2: u64 = 0x0F;
const ICBRL: u64 = 0x10;
const ICBRH: u64 = 0x11;
const ICDRT: u64 = 0x12;
const ICDRR: u64 = 0x13;
const ICWUR: u64 = 0x16;
const ICWUR2: u64 = 0x17;

// ICCR1.
const ICE: u8 = 1 << 7;
const IICRST: u8 = 1 << 6;
/// SOWP reads 1; SCLO/SDAO and the SCLI/SDAI line monitors read high.
const ICCR1_LINES_HIGH: u8 = 0x1F;

// ICCR2.
const ST: u8 = 1 << 1;
const RS: u8 = 1 << 2;
const SP: u8 = 1 << 3;
const TRS: u8 = 1 << 5;
const MST: u8 = 1 << 6;
const BBSY: u8 = 1 << 7;

// ICMR1.
const BCWP: u8 = 1 << 3;
const MTWP: u8 = 1 << 7;

// ICMR3.
const NF_MASK: u8 = 0x03;
const ACKBR: u8 = 1 << 2;
const ACKBT: u8 = 1 << 3;
const ACKWP: u8 = 1 << 4;

// ICFER.
const NFE: u8 = 1 << 5;

// ICSR2 (and the matching ICIER enables).
const TDRE: u8 = 1 << 7;
const TEND: u8 = 1 << 6;
const RDRF: u8 = 1 << 5;
const NACKF: u8 = 1 << 4;
const STOP: u8 = 1 << 3;
const START: u8 = 1 << 2;
/// The write-0-to-clear flags (NACKF, STOP, START, AL, TMOF): the ERI set.
const ERI_FLAGS: u8 = 0x1F;

/// Bits per byte on the wire, the acknowledge included.
const BYTE_PERIODS: u64 = 9;

/// RA8M1 PCLKB under Zephyr/FSP's EK-RA8M1 clock tree (PLL1P 480 MHz / 8).
pub const DEFAULT_PCLKB_HZ: u64 = 60_000_000;
/// The rate the bus cycles run at (Zephyr's EK-RA8M1 ICLK, 480 MHz).
pub const DEFAULT_CLOCK_HZ: u64 = 480_000_000;
/// `ELC_EVENT_IIC1_RXI` on the RA8M1.
pub const DEFAULT_ELC_EVENT: u32 = 0x061;

/// What the wire is doing; it ends at the cycle stored beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
enum Op {
    Start,
    Restart,
    Stop,
    /// A byte shifting out.
    Tx(u8),
    /// Master receive: the earliest cycle the next byte can be in `ICDRR`,
    /// once the one there now has been read.
    RxNext,
}

/// A stop or restart requested while a received byte was still unread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
enum Cond {
    Restart,
    Stop,
}

/// One RA RIIC channel.
#[derive(serde::Serialize)]
pub struct RaIic {
    clock_hz: u64,
    pclkb_hz: u64,
    elc_event: u32,
    iccr1: u8,
    iccr2: u8,
    icmr1: u8,
    icmr2: u8,
    icmr3: u8,
    icfer: u8,
    icser: u8,
    icier: u8,
    icsr1: u8,
    icsr2: u8,
    sar: [u8; 6],
    icbrl: u8,
    icbrh: u8,
    icdrt: u8,
    icdrr: u8,
    icwur: u8,
    icwur2: u8,
    /// The wire operation in progress and the cycle it ends.
    op: Option<(u64, Op)>,
    /// A byte written to `ICDRT` while the shift register was busy.
    held: bool,
    /// The next transmitted byte is the slave address.
    addr_phase: bool,
    deferred: Option<Cond>,
    /// Index of the device that ACKed the current address.
    selected: Option<usize>,
    /// `ICSR2 & ICIER` as last seen, for the edge detection.
    last_level: u8,
    /// Events raised and not yet handed to the bus.
    due: Vec<u32>,
    /// The deadline of the one live scheduler wake (token `generation`).
    armed_at: Option<u64>,
    generation: u32,
    #[serde(skip)]
    new_wake: bool,
    /// `ICDRR` was read while `RDRF` was set. `Peripheral::read` takes
    /// `&self`, so the read is noted here and acted on at the next wake.
    #[serde(skip)]
    popped: Cell<bool>,
    #[serde(skip)]
    slaves: Vec<Box<dyn I2cDevice>>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl std::fmt::Debug for RaIic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RaIic")
            .field("elc_event", &self.elc_event)
            .field("iccr2", &self.iccr2)
            .field("icsr2", &self.icsr2)
            .field("op", &self.op)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl RaIic {
    pub fn new(clock_hz: u64, pclkb_hz: u64, elc_event: u32) -> Self {
        let mut iic = Self {
            clock_hz: clock_hz.max(1),
            pclkb_hz: pclkb_hz.max(1),
            elc_event,
            iccr1: ICCR1_LINES_HIGH,
            iccr2: 0,
            icmr1: 0,
            icmr2: 0,
            icmr3: 0,
            icfer: 0,
            icser: 0,
            icier: 0,
            icsr1: 0,
            icsr2: 0,
            sar: [0; 6],
            icbrl: 0,
            icbrh: 0,
            icdrt: 0,
            icdrr: 0,
            icwur: 0,
            icwur2: 0,
            op: None,
            held: false,
            addr_phase: false,
            deferred: None,
            selected: None,
            last_level: 0,
            due: Vec::new(),
            armed_at: None,
            generation: 0,
            new_wake: false,
            popped: Cell::new(false),
            slaves: Vec::new(),
            clock: None,
        };
        iic.reset_registers();
        iic
    }

    crate::cycle_clock::scheduler_mode!();

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    /// Register reset values (hardware manual, "I2C bus interface").
    fn reset_registers(&mut self) {
        self.iccr2 = 0;
        self.icmr1 = BCWP;
        self.icmr2 = 0x06;
        self.icmr3 = 0;
        self.icfer = 0x72;
        self.icser = 0x09;
        self.icier = 0;
        self.icsr1 = 0;
        self.icsr2 = 0;
        self.sar = [0; 6];
        self.icbrl = 0xFF;
        self.icbrh = 0xFF;
        self.icdrt = 0xFF;
        self.icdrr = 0xFF;
        self.icwur = 0x10;
        self.icwur2 = 0xFD;
        self.release_bus();
    }

    /// Abandon whatever the wire was doing: no condition is generated, the
    /// selected device is told the transfer ended, and events raised but not
    /// yet delivered are dropped.
    fn release_bus(&mut self) {
        self.iccr2 &= !(BBSY | MST | TRS | ST | RS | SP);
        self.icsr2 &= !(TDRE | TEND | RDRF);
        self.op = None;
        self.due.clear();
        self.held = false;
        self.addr_phase = false;
        self.deferred = None;
        self.popped.set(false);
        self.deselect();
        self.last_level = self.level();
        if self.armed_at.take().is_some() {
            // The wake still queued is now stale.
            self.generation = self.generation.wrapping_add(1);
        }
    }

    fn deselect(&mut self) {
        if let Some(i) = self.selected.take() {
            self.slaves[i].stop();
        }
    }

    pub fn attach(&mut self, device: Box<dyn I2cDevice>) {
        self.slaves.push(device);
    }

    /// One SCL period in bus cycles.
    fn scl_period(&self) -> u64 {
        let cks = u32::from((self.icmr1 >> 4) & 7);
        let nf = if self.icfer & NFE != 0 {
            u64::from(self.icmr3 & NF_MASK) + 1
        } else {
            0
        };
        let constant = if cks == 0 { 3 + nf } else { 2 + nf };
        let counts = u64::from(self.icbrh & 0x1F) + u64::from(self.icbrl & 0x1F) + 2 * constant;
        let pclk_cycles = u128::from(counts) << cks;
        let cycles = (pclk_cycles * u128::from(self.clock_hz)).div_ceil(u128::from(self.pclkb_hz));
        (cycles as u64).max(1)
    }

    fn receiving(&self) -> bool {
        self.iccr2 & (MST | TRS) == MST && self.selected.is_some()
    }

    /// `RDRF` as firmware sees it: a read of `ICDRR` clears it at once.
    fn icsr2_now(&self) -> u8 {
        if self.popped.get() {
            self.icsr2 & !RDRF
        } else {
            self.icsr2
        }
    }

    fn level(&self) -> u8 {
        self.icsr2_now() & self.icier
    }

    /// Raise an event for every flag-and-enable that went from 0 to 1.
    fn update_level(&mut self) {
        let level = self.level();
        let rising = level & !self.last_level;
        self.last_level = level;
        if rising & RDRF != 0 {
            self.due.push(self.elc_event);
        }
        if rising & TDRE != 0 {
            self.due.push(self.elc_event + 1);
        }
        if rising & TEND != 0 {
            self.due.push(self.elc_event + 2);
        }
        if rising & ERI_FLAGS != 0 {
            self.due.push(self.elc_event + 3);
        }
    }

    fn begin(&mut self, at: u64, op: Op) {
        let periods = match op {
            Op::Tx(_) | Op::RxNext => BYTE_PERIODS,
            Op::Start | Op::Restart | Op::Stop => 1,
        };
        self.op = Some((at + periods * self.scl_period(), op));
    }

    /// Move `ICDRT` into the shift register at `at`.
    fn shift_out(&mut self, at: u64) {
        self.held = false;
        self.icsr2 |= TDRE;
        self.begin(at, Op::Tx(self.icdrt));
        self.update_level();
    }

    /// Issue a stop or restart now, after the byte on the wire, or after the
    /// unread received byte. `false` when this channel is not bus master.
    fn request(&mut self, cond: Cond, at: u64) -> bool {
        if self.iccr2 & (BBSY | MST) != BBSY | MST {
            return false;
        }
        let unread = self.receiving() && self.icsr2_now() & RDRF != 0;
        match self.op {
            // Reception parked on a read that already happened.
            Some((_, Op::RxNext)) | None if !unread => self.begin(at, Self::cond_op(cond)),
            _ => self.deferred = Some(cond),
        }
        true
    }

    fn cond_op(cond: Cond) -> Op {
        match cond {
            Cond::Restart => Op::Restart,
            Cond::Stop => Op::Stop,
        }
    }

    /// The operation in progress ended at `at`.
    fn complete(&mut self, at: u64, op: Op) {
        self.op = None;
        if matches!(op, Op::Start | Op::Restart | Op::Stop) {
            self.popped.set(false);
        }
        match op {
            Op::Start | Op::Restart => {
                self.iccr2 = (self.iccr2 & !(ST | RS)) | BBSY | MST | TRS;
                self.icsr2 = (self.icsr2 & !(TEND | RDRF)) | TDRE | START;
                self.addr_phase = true;
                self.held = false;
            }
            Op::Stop => {
                self.iccr2 &= !(SP | BBSY | MST | TRS);
                self.icsr2 = (self.icsr2 & !(TDRE | TEND | RDRF)) | STOP;
                self.icmr3 &= !ACKBT;
                self.addr_phase = false;
                self.held = false;
                self.deselect();
            }
            Op::Tx(byte) => self.byte_sent(at, byte),
            Op::RxNext => {
                if !self.popped.replace(false) {
                    // ICDRR not read yet: SCL stays low; look again a byte
                    // time later.
                    self.begin(at, Op::RxNext);
                    return;
                }
                self.icsr2 &= !RDRF;
                self.update_level();
                if let Some(cond) = self.deferred.take() {
                    self.begin(at, Self::cond_op(cond));
                } else if let Some(i) = self.selected {
                    self.icdrr = self.slaves[i].read();
                    self.icsr2 |= RDRF;
                    self.begin(at, Op::RxNext);
                }
            }
        }
        self.update_level();
    }

    fn byte_sent(&mut self, at: u64, byte: u8) {
        if std::mem::take(&mut self.addr_phase) {
            self.deselect(); // a repeated start ends the previous transfer
            let address = byte >> 1;
            let Some(i) = self.slaves.iter().position(|d| d.claims_address(address)) else {
                // NACK: the transfer stops; firmware issues the stop.
                self.icsr2 |= NACKF;
                self.icmr3 |= ACKBR;
                self.held = false;
                return;
            };
            self.icmr3 &= !ACKBR;
            self.slaves[i].select_address(address);
            self.slaves[i].start();
            self.selected = Some(i);
            if byte & 1 != 0 {
                // Master receive: RDRF for the dummy ICDRR read.
                self.iccr2 &= !TRS;
                self.icsr2 = (self.icsr2 & !(TDRE | TEND)) | RDRF;
                self.held = false;
                self.begin(at, Op::RxNext);
                return;
            }
        } else if let Some(i) = self.selected {
            self.slaves[i].write(byte);
        }
        if self.held {
            self.shift_out(at);
        } else if let Some(cond) = self.deferred.take() {
            self.icsr2 |= TEND;
            self.begin(at, Self::cond_op(cond));
        } else {
            self.icsr2 |= TEND;
        }
    }

    /// Complete every operation due by `now` and hand over the events.
    fn service(&mut self, now: u64) -> Vec<u32> {
        // A read with nothing pending still lowers RDRF in the edge record.
        self.update_level();
        while let Some((at, op)) = self.op.filter(|&(at, _)| at <= now) {
            self.complete(at, op);
        }
        std::mem::take(&mut self.due)
    }

    fn next_deadline(&self, now: u64) -> Option<u64> {
        let op = self.op.map(|(at, _)| at);
        let due = (!self.due.is_empty()).then_some(now);
        op.into_iter().chain(due).min()
    }

    /// Arm a new wake when the next deadline comes before the live one.
    fn arm(&mut self) {
        let now = self.now_cycle();
        let Some(next) = self.next_deadline(now) else {
            return;
        };
        if self.armed_at.is_none_or(|armed| next < armed) {
            self.armed_at = Some(next);
            self.generation = self.generation.wrapping_add(1);
            self.new_wake = true;
        }
    }

    fn read_reg(&self, offset: u64) -> u8 {
        match offset {
            ICCR1 => self.iccr1 | ICCR1_LINES_HIGH,
            ICCR2 => self.iccr2,
            ICMR1 => self.icmr1 | BCWP,
            ICMR2 => self.icmr2,
            ICMR3 => self.icmr3,
            ICFER => self.icfer,
            ICSER => self.icser,
            ICIER => self.icier,
            ICSR1 => self.icsr1,
            ICSR2 => self.icsr2_now(),
            SARL0..=SARU2 => self.sar[(offset - SARL0) as usize],
            ICBRL => self.icbrl,
            ICBRH => self.icbrh,
            ICDRT => self.icdrt,
            ICDRR => self.icdrr,
            ICWUR => self.icwur,
            ICWUR2 => self.icwur2,
            _ => 0,
        }
    }

    fn write_iccr1(&mut self, value: u8) {
        let old = self.iccr1;
        self.iccr1 = (value & (ICE | IICRST | 0x20)) | ICCR1_LINES_HIGH;
        if value & IICRST != 0 {
            if value & ICE == 0 {
                // Reset with the interface off: every register.
                self.reset_registers();
            } else {
                // Internal reset: bus state, flags and the shift register.
                self.icsr1 = 0;
                self.icsr2 = 0;
                self.release_bus();
            }
        } else if value & ICE == 0 && old & ICE != 0 {
            self.release_bus();
        }
    }

    fn enabled(&self) -> bool {
        self.iccr1 & (ICE | IICRST) == ICE
    }

    fn write_iccr2(&mut self, value: u8) {
        let now = self.now_cycle();
        if self.icmr1 & MTWP != 0 {
            self.iccr2 = (self.iccr2 & !(MST | TRS)) | (value & (MST | TRS));
        }
        if !self.enabled() {
            return;
        }
        if value & ST != 0 && self.iccr2 & BBSY == 0 && self.op.is_none() {
            self.iccr2 |= ST;
            self.begin(now, Op::Start);
        }
        if value & RS != 0 && self.request(Cond::Restart, now) {
            self.iccr2 |= RS;
        }
        if value & SP != 0 && self.request(Cond::Stop, now) {
            self.iccr2 |= SP;
        }
    }

    fn write_icdrt(&mut self, value: u8) {
        self.icdrt = value;
        if !self.enabled() || self.iccr2 & (BBSY | MST | TRS) != BBSY | MST | TRS {
            return;
        }
        if self.icsr2 & NACKF != 0 {
            // Transmission stays suspended until the stop.
            return;
        }
        self.icsr2 &= !(TDRE | TEND);
        self.update_level();
        match self.op {
            None => self.shift_out(self.now_cycle()),
            Some(_) => self.held = true,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u8) {
        match offset {
            ICCR1 => self.write_iccr1(value),
            ICCR2 => self.write_iccr2(value),
            ICMR1 => self.icmr1 = (value & !0x07) | BCWP,
            ICMR2 => self.icmr2 = value & 0xF7,
            ICMR3 => {
                let ackbt = if value & ACKWP != 0 {
                    value & ACKBT
                } else {
                    self.icmr3 & ACKBT
                };
                self.icmr3 = (value & !(ACKBT | ACKBR)) | ackbt | (self.icmr3 & ACKBR);
            }
            ICFER => self.icfer = value,
            ICSER => self.icser = value,
            ICIER => self.icier = value,
            ICSR1 => self.icsr1 &= value,
            ICSR2 => self.icsr2 &= value | !ERI_FLAGS,
            SARL0..=SARU2 => self.sar[(offset - SARL0) as usize] = value,
            ICBRL => self.icbrl = value | 0xE0,
            ICBRH => self.icbrh = value | 0xE0,
            ICDRT => self.write_icdrt(value),
            ICDRR => {}
            ICWUR => self.icwur = value,
            ICWUR2 => self.icwur2 = value,
            _ => {
                crate::census_reg!("ra:RaIic", offset, "write");
            }
        }
        self.update_level();
        self.arm();
    }
}

impl Default for RaIic {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_HZ, DEFAULT_PCLKB_HZ, DEFAULT_ELC_EVENT)
    }
}

impl crate::peripherals::device::I2cSlaveHost for RaIic {
    fn attach_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.attach(device);
    }
}

impl Peripheral for RaIic {
    fn i2c_slave_host(&mut self) -> Option<&mut dyn crate::peripherals::device::I2cSlaveHost> {
        Some(self)
    }

    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && (self.op.is_some() || !self.due.is_empty())
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn explicit_irqs_are_link_events(&self) -> bool {
        true
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> crate::PeripheralTickResult {
        if self.scheduler_mode() {
            return crate::PeripheralTickResult::default();
        }
        let now = self.now_cycle();
        if self.next_deadline(now).is_none_or(|at| at > now) {
            return crate::PeripheralTickResult::default();
        }
        let events = self.service(now);
        crate::PeripheralTickResult {
            explicit_irqs: (!events.is_empty()).then_some(events),
            ..Default::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let offset = offset & 0xFF;
        let value = self.read_reg(offset);
        if offset == ICDRR && self.icsr2 & RDRF != 0 && self.receiving() {
            self.popped.set(true);
        }
        Ok(value)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(self.read_reg(offset & 0xFF))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_reg(offset & 0xFF, value);
        Ok(())
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !std::mem::take(&mut self.new_wake) {
            return Vec::new();
        }
        let now = self.now_cycle();
        self.armed_at
            .map(|at| vec![(at.saturating_sub(now + 1), self.generation)])
            .unwrap_or_default()
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token != self.generation || self.armed_at.is_none() {
            return crate::sched::EventResult::default();
        }
        let now = self.now_cycle();
        let events = self.service(now);
        // The drain does not harvest `take_scheduled_events` after an
        // event: the next deadline goes back as a reschedule of this token.
        self.new_wake = false;
        self.generation = event_token;
        self.armed_at = self.next_deadline(now);
        crate::sched::EventResult {
            explicit_irqs: events,
            reschedule_delay: self.armed_at.map(|at| at.saturating_sub(now).max(1)),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        self.iccr1 = ICCR1_LINES_HIGH;
        self.reset_registers();
    }

    fn drives_central_device_time(&self) -> bool {
        true
    }

    fn advance_attached_device_time_us(&mut self, us: u64) {
        for slave in &mut self.slaves {
            slave.advance_time_us(us);
        }
    }

    fn drain_attached_pin_drives(&mut self, out: &mut Vec<(String, String, bool)>) {
        for slave in &mut self.slaves {
            drain_i2c_pin_drives(&mut **slave, out);
        }
    }

    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for slave in &mut self.slaves {
            if slave.for_each_sim_input(f) {
                return true;
            }
        }
        false
    }

    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for slave in &self.slaves {
            crate::inspect::visit_i2c_device(&**slave, f);
        }
    }

    fn describe_registers(&self) -> Option<Vec<crate::inspect::RegisterSchema>> {
        let names = [
            (ICCR1, "ICCR1"),
            (ICCR2, "ICCR2"),
            (ICMR1, "ICMR1"),
            (ICMR2, "ICMR2"),
            (ICMR3, "ICMR3"),
            (ICFER, "ICFER"),
            (ICSER, "ICSER"),
            (ICIER, "ICIER"),
            (ICSR1, "ICSR1"),
            (ICSR2, "ICSR2"),
            (ICBRL, "ICBRL"),
            (ICBRH, "ICBRH"),
            (ICDRT, "ICDRT"),
            (ICDRR, "ICDRR"),
        ];
        Some(
            names
                .into_iter()
                .map(|(offset, name)| crate::inspect::RegisterSchema {
                    name: name.into(),
                    offset,
                    size: 8,
                    access: if offset == ICDRR { "ro" } else { "rw" },
                    fields: vec![],
                })
                .collect(),
        )
    }

    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        let mut pi = crate::inspect::default_inspect(self, base, name, opts);
        pi.kind = "i2c".into();
        for slave in &self.slaves {
            pi.artifacts
                .extend(crate::inspect::i2c_artifacts(&**slave, opts));
        }
        pi
    }

    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        self.slaves.iter().flat_map(|s| s.logs()).collect()
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const RXI: u32 = DEFAULT_ELC_EVENT;
    const TXI: u32 = RXI + 1;
    const TEI: u32 = RXI + 2;
    const ERI: u32 = RXI + 3;
    /// FSP `IIC_MASTER_INTERRUPT_ENABLE_INIT_MASK` | STIE | SPIE.
    const RUN_IER: u8 = 0xB3 | 0x04 | 0x08;

    /// A register-pointer device: the first byte written selects the
    /// register, later writes store and reads return with auto-increment.
    #[derive(Debug, Default)]
    struct Regs {
        mem: Vec<u8>,
        ptr: u8,
        fresh: bool,
        log: Vec<String>,
    }

    #[derive(Debug, Clone)]
    struct Dev(Arc<Mutex<Regs>>, u8);

    impl I2cDevice for Dev {
        fn address(&self) -> u8 {
            self.1
        }
        fn start(&mut self) {
            let mut r = self.0.lock().unwrap();
            r.fresh = true;
            r.log.push("start".into());
        }
        fn stop(&mut self) {
            self.0.lock().unwrap().log.push("stop".into());
        }
        fn write(&mut self, data: u8) {
            let mut r = self.0.lock().unwrap();
            if std::mem::take(&mut r.fresh) {
                r.ptr = data;
            } else {
                let p = r.ptr as usize;
                r.mem[p] = data;
                r.ptr = r.ptr.wrapping_add(1);
            }
            r.log.push(format!("w{data:02x}"));
        }
        fn read(&mut self) -> u8 {
            let mut r = self.0.lock().unwrap();
            r.fresh = false;
            let v = r.mem[r.ptr as usize];
            r.ptr = r.ptr.wrapping_add(1);
            r.log.push(format!("r{v:02x}"));
            v
        }
    }

    /// 1 MHz CPU and PCLKB; CKS 0, BRL = BRH = 2: a 10-cycle SCL period.
    fn iic() -> (RaIic, CycleClock, Arc<Mutex<Regs>>) {
        let clock = CycleClock::default();
        let mut i = RaIic::new(1_000_000, 1_000_000, RXI);
        i.attach_cycle_clock(clock.clone());
        let regs = Arc::new(Mutex::new(Regs {
            mem: vec![0; 256],
            ..Regs::default()
        }));
        i.attach(Box::new(Dev(regs.clone(), 0x76)));
        // FSP open: interface off, reset, configure under reset, release.
        i.write(ICCR1, 0x1F).unwrap();
        i.write(ICCR1, 0x5F).unwrap();
        i.write(ICCR1, 0xDF).unwrap();
        i.write(ICBRL, 0xE2).unwrap();
        i.write(ICBRH, 0xE2).unwrap();
        i.write(ICMR1, 0x08).unwrap();
        i.write(ICFER, 0x77 & !NFE).unwrap();
        i.write(ICIER, 0xB3).unwrap();
        i.write(ICCR1, 0x9F).unwrap();
        (i, clock, regs)
    }

    /// Run to the next deadline and return the events raised there.
    fn step(i: &mut RaIic, clock: &CycleClock) -> (u64, Vec<u32>) {
        let now = clock.now();
        let at = i.next_deadline(now).expect("something pending");
        clock.publish(at.max(now));
        let events = i.service(clock.now());
        (clock.now(), events)
    }

    fn set_bits(i: &mut RaIic, off: u64, bits: u8) {
        let v = i.read_reg(off);
        i.write(off, v | bits).unwrap();
    }

    fn clear_bits(i: &mut RaIic, off: u64, bits: u8) {
        let v = i.read_reg(off);
        i.write(off, v & !bits).unwrap();
    }

    #[test]
    fn reset_values() {
        let i = RaIic::default();
        assert_eq!(i.read(ICCR1).unwrap(), 0x1F);
        assert_eq!(i.read(ICMR1).unwrap(), 0x08);
        assert_eq!(i.read(ICMR2).unwrap(), 0x06);
        assert_eq!(i.read(ICFER).unwrap(), 0x72);
        assert_eq!(i.read(ICSER).unwrap(), 0x09);
        assert_eq!(i.read(ICBRL).unwrap(), 0xFF);
        assert_eq!(i.read(ICDRR).unwrap(), 0xFF);
        assert_eq!(i.read(ICCR2).unwrap() & BBSY, 0);
    }

    #[test]
    fn scl_period_follows_brl_brh_cks() {
        let (mut i, _c, _r) = iic();
        // (2 + 2 + 2*3) PCLKB counts at CKS 0.
        assert_eq!(i.scl_period(), 10);
        i.write(ICMR1, 0x08 | (2 << 4)).unwrap();
        // (2 + 2 + 2*2) << 2.
        assert_eq!(i.scl_period(), 32);
    }

    /// FSP write of [reg, value] to 0x76 with a stop: START+TXI, address,
    /// two data bytes, TEI, stop, ERI(STOP).
    #[test]
    fn master_transmit_with_stop() {
        let (mut i, c, regs) = iic();
        i.write(ICIER, RUN_IER).unwrap();
        i.write(ICCR2, ST).unwrap();
        assert!(i.read(ICCR2).unwrap() & ST != 0, "ST holds until the start");
        let (t, ev) = step(&mut i, &c);
        assert_eq!(t, 10, "start takes one SCL period");
        assert_eq!(ev, vec![TXI, ERI]);
        assert_eq!(
            i.read(ICCR2).unwrap() & (BBSY | MST | TRS | ST),
            BBSY | MST | TRS
        );
        assert!(i.read(ICSR2).unwrap() & START != 0);
        clear_bits(&mut i, ICSR2, ERI_FLAGS);

        // TXI: address. TDRE comes straight back (shift register was idle).
        i.write(ICDRT, 0x76 << 1).unwrap();
        assert!(i.read(ICSR2).unwrap() & TDRE != 0);
        let (t, ev) = step(&mut i, &c);
        assert_eq!((t, ev), (10, vec![TXI]));
        // TXI: first data byte waits in ICDRT behind the address.
        i.write(ICDRT, 0xF4).unwrap();
        assert_eq!(i.read(ICSR2).unwrap() & TDRE, 0);
        let (t, ev) = step(&mut i, &c);
        assert_eq!(t, 10 + 90, "the address byte took 9 periods");
        assert_eq!(ev, vec![TXI]);
        // Last byte; FSP drops TIE and sets TEIE.
        i.write(ICDRT, 0x27).unwrap();
        clear_bits(&mut i, ICIER, 0x80);
        set_bits(&mut i, ICIER, 0x40);
        let (t, ev) = step(&mut i, &c);
        assert_eq!(
            (t, ev),
            (190, vec![]),
            "0xF4 sent; 0x27 moves into the shifter"
        );
        let (t, ev) = step(&mut i, &c);
        assert_eq!((t, ev), (280, vec![TEI]));
        assert!(i.read(ICSR2).unwrap() & TEND != 0);

        // TEI: clear STOP, request it.
        clear_bits(&mut i, ICSR2, STOP);
        i.write(ICCR2, SP).unwrap();
        clear_bits(&mut i, ICIER, 0x40);
        let (t, ev) = step(&mut i, &c);
        assert_eq!((t, ev), (290, vec![ERI]));
        assert_eq!(i.read(ICCR2).unwrap() & (BBSY | MST | TRS | SP), 0);
        assert!(i.read(ICSR2).unwrap() & STOP != 0);
        let r = regs.lock().unwrap();
        assert_eq!(r.mem[0xF4], 0x27);
        assert_eq!(r.log, vec!["start", "wf4", "w27", "stop"]);
    }

    /// FSP read of 3 bytes: dummy read, WAIT/ACKBT bookkeeping, SP before
    /// the last read, the stop only after that read.
    #[test]
    fn master_receive_issues_the_stop_after_the_last_read() {
        let (mut i, c, regs) = iic();
        regs.lock().unwrap().mem[..3].copy_from_slice(&[0x11, 0x22, 0x33]);
        i.write(ICIER, RUN_IER).unwrap();
        i.write(ICCR2, ST).unwrap();
        assert_eq!(step(&mut i, &c).1, vec![TXI, ERI]);
        clear_bits(&mut i, ICSR2, ERI_FLAGS);
        i.write(ICDRT, (0x76 << 1) | 1).unwrap();
        assert_eq!(
            step(&mut i, &c).1,
            vec![TXI],
            "address moved to the shifter"
        );
        let (t, ev) = step(&mut i, &c);
        assert_eq!((t, ev), (100, vec![RXI]), "read address ACKed: RDRF");
        assert_eq!(i.read(ICCR2).unwrap() & TRS, 0);
        // Dummy read clocks byte 0.
        let _ = i.read(ICDRR).unwrap();
        assert_eq!(i.read(ICSR2).unwrap() & RDRF, 0, "the read clears RDRF");
        let mut got = vec![];
        let (t, ev) = step(&mut i, &c);
        assert_eq!((t, ev), (190, vec![RXI]));
        got.push(i.read(ICDRR).unwrap());
        let (_, ev) = step(&mut i, &c);
        assert_eq!(ev, vec![RXI]);
        // remain == 2 → NACK the last; ACKBT needs ACKWP in the same write.
        i.write(ICMR3, ACKBT).unwrap();
        assert_eq!(i.read(ICMR3).unwrap() & ACKBT, 0, "ACKWP clear: ignored");
        set_bits(&mut i, ICMR3, ACKWP);
        set_bits(&mut i, ICMR3, ACKBT);
        clear_bits(&mut i, ICMR3, ACKWP);
        assert!(i.read(ICMR3).unwrap() & ACKBT != 0);
        got.push(i.read(ICDRR).unwrap());
        let (_, ev) = step(&mut i, &c);
        assert_eq!(ev, vec![RXI]);
        // remain == 1: SP first, then the read.
        clear_bits(&mut i, ICSR2, STOP);
        i.write(ICCR2, SP).unwrap();
        assert!(
            i.read(ICCR2).unwrap() & BBSY != 0,
            "stop waits for the read"
        );
        got.push(i.read(ICDRR).unwrap());
        assert_eq!(got, vec![0x11, 0x22, 0x33]);
        let (_, ev) = step(&mut i, &c);
        assert_eq!(
            ev,
            Vec::<u32>::new(),
            "the read is noticed at the parked byte time"
        );
        let (_, ev) = step(&mut i, &c);
        assert_eq!(ev, vec![ERI], "STOP");
        assert_eq!(i.read(ICCR2).unwrap() & BBSY, 0);
        assert_eq!(
            i.read(ICMR3).unwrap() & ACKBT,
            0,
            "ACKBT clears at the stop"
        );
        assert_eq!(
            regs.lock().unwrap().log,
            vec!["start", "r11", "r22", "r33", "stop"]
        );
    }

    /// i2c_burst_read: write the register with a restart, then read.
    #[test]
    fn restart_turns_the_bus_around() {
        let (mut i, c, regs) = iic();
        regs.lock().unwrap().mem[0xD0] = 0x60;
        i.write(ICIER, RUN_IER).unwrap();
        i.write(ICCR2, ST).unwrap();
        step(&mut i, &c);
        clear_bits(&mut i, ICSR2, ERI_FLAGS);
        i.write(ICDRT, 0x76 << 1).unwrap();
        step(&mut i, &c);
        i.write(ICDRT, 0xD0).unwrap();
        clear_bits(&mut i, ICIER, 0x80);
        set_bits(&mut i, ICIER, 0x40);
        step(&mut i, &c);
        assert_eq!(step(&mut i, &c).1, vec![TEI]);
        // TEI with restart: TIE back on (TDRE = 1: queues TXI), RS.
        set_bits(&mut i, ICIER, 0x80);
        i.write(ICCR2, RS).unwrap();
        clear_bits(&mut i, ICIER, 0x40);
        let (_, ev) = step(&mut i, &c);
        assert_eq!(ev, vec![TXI], "TIE set while TDRE = 1");
        let (_, ev) = step(&mut i, &c);
        assert_eq!(ev, vec![ERI], "restart: START");
        assert!(i.read(ICSR2).unwrap() & START != 0);
        clear_bits(&mut i, ICSR2, ERI_FLAGS);
        i.write(ICDRT, (0x76 << 1) | 1).unwrap();
        step(&mut i, &c);
        assert_eq!(step(&mut i, &c).1, vec![RXI]);
        let _ = i.read(ICDRR).unwrap();
        step(&mut i, &c);
        assert_eq!(i.read(ICDRR).unwrap(), 0x60);
        assert_eq!(
            regs.lock().unwrap().log,
            vec!["start", "wd0", "stop", "start", "r60"]
        );
    }

    #[test]
    fn an_unclaimed_address_is_nacked() {
        let (mut i, c, regs) = iic();
        i.write(ICIER, RUN_IER).unwrap();
        i.write(ICCR2, ST).unwrap();
        step(&mut i, &c);
        clear_bits(&mut i, ICSR2, ERI_FLAGS);
        i.write(ICDRT, 0x50 << 1).unwrap();
        step(&mut i, &c);
        i.write(ICDRT, 0xAA).unwrap(); // FSP's TXI loads the first byte
        let (_, ev) = step(&mut i, &c);
        assert_eq!(ev, vec![ERI]);
        assert!(i.read(ICSR2).unwrap() & NACKF != 0);
        assert!(i.read(ICMR3).unwrap() & ACKBR != 0);
        // ERI: SP; the held byte is never sent.
        clear_bits(&mut i, ICSR2, ERI_FLAGS);
        i.write(ICCR2, SP).unwrap();
        assert_eq!(step(&mut i, &c).1, vec![ERI]);
        assert!(i.read(ICSR2).unwrap() & STOP != 0);
        assert!(regs.lock().unwrap().log.is_empty());
    }

    #[test]
    fn internal_reset_drops_the_transfer_and_its_wake() {
        let (mut i, c, regs) = iic();
        i.write(ICIER, RUN_IER).unwrap();
        i.write(ICCR2, ST).unwrap();
        step(&mut i, &c);
        i.write(ICDRT, 0x76 << 1).unwrap();
        step(&mut i, &c);
        let token = i.generation;
        i.write(ICCR1, 0xDF).unwrap();
        assert_ne!(i.generation, token, "the queued wake is stale");
        assert_eq!(i.read(ICCR2).unwrap() & (BBSY | MST), 0);
        assert_eq!(i.read(ICSR2).unwrap(), 0);
        assert!(i.next_deadline(c.now()).is_none());
        assert!(
            regs.lock().unwrap().log.is_empty(),
            "address never completed"
        );
    }

    #[test]
    fn start_is_ignored_while_disabled_or_in_reset() {
        let (mut i, c, _r) = iic();
        i.write(ICCR1, 0xDF).unwrap();
        i.write(ICCR2, ST).unwrap();
        assert!(i.next_deadline(c.now()).is_none());
        i.write(ICCR1, 0x1F).unwrap();
        i.write(ICCR2, ST).unwrap();
        assert!(i.next_deadline(c.now()).is_none());
    }

    #[test]
    fn one_live_wake_across_a_transfer() {
        let (mut i, c, _r) = iic();
        if !i.scheduler_mode() {
            return;
        }
        let _ = i.take_scheduled_events();
        i.write(ICIER, RUN_IER).unwrap();
        i.write(ICCR2, ST).unwrap();
        let wakes = i.take_scheduled_events();
        assert_eq!(wakes, vec![(9, i.generation)], "fires at now + 1 + 9");
        // Later deadlines ride the live wake: no second one.
        i.write(ICMR2, 0x06).unwrap();
        assert!(i.take_scheduled_events().is_empty());
        let _ = c;
    }
}
