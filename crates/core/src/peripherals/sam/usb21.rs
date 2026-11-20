// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SAM D21 USB controller in device mode (`USB` at `0x4100_5000`, NVIC line
//! 7; SAM D21/DA1 Family Data Sheet DS40001882H §32) with a scripted USB host
//! on the other end of the cable.
//!
//! The endpoint descriptor table lives in RAM, as on silicon: `DESCADD`
//! points at eight entries of two 16-byte banks (bank 0 = OUT/SETUP, bank 1 =
//! IN), each with `ADDR` and `PCKSIZE` (`BYTE_COUNT`, `MULTI_PACKET_SIZE`,
//! `SIZE`, `AUTO_ZLP`). A host token moves bytes between the host and the
//! buffer `ADDR` names and reports through `EPSTATUS` (`BK0RDY`, `BK1RDY`,
//! data toggles) and `EPINTFLAG` (`RXSTP`, `TRCPTn`, `TRFAILn`, `STALLn`).
//!
//! The host ([`crate::peripherals::usb_host`]) attaches once the device
//! drives its pull-up: `CTRLA.ENABLE`, device mode, `CTRLB.DETACH` clear, the
//! USB bus clock on in `PM.APBBMASK` and PA24/PA25 muxed to function G. It
//! resets the bus, enumerates as an OS does, opens a CDC ACM port when the
//! configuration has one and a console reader is attached (SET_LINE_CODING
//! 115200 8N1, SET_CONTROL_LINE_STATE DTR|RTS), then reads every bulk IN
//! endpoint into the console sink. It sends a start of frame every
//! millisecond. Nothing about the device is assumed: what the host learned is
//! in the `host` peripheral log.
//!
//! Model limits, refused with `NotImplemented` where firmware can reach
//! them: host mode, high speed and the test modes, remote wakeup, `GNAK`,
//! `NREPLY`, LPM, isochronous and dual-bank endpoints. Not modelled and
//! silent: suspend/resume (the host never suspends), wire-level timing
//! inside a packet, CRC and bit-stuff errors, and the GCLK_USB generic clock
//! (GCLK.CLKCTRL is a plain register bank on this chip).
//!
//! One judgement call: `MULTI_PACKET_SIZE` of an IN bank is read and updated
//! only for a transfer longer than one packet. ArduinoCore-samd's bulk
//! `send()` never clears the field between single-packet transfers and works
//! on silicon, so a single packet does not consult it here.

use crate::peripherals::imxrt::{wake_hint, Timebase};
use crate::peripherals::usb_host::{self, HostStep, Setup, UsbHostLog};
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const ENDPOINTS: usize = 8;

const CTRLA: u64 = 0x00;
const QOSCTRL: u64 = 0x03;
const CTRLB: u64 = 0x08;
const DADD: u64 = 0x0A;
const STATUS: u64 = 0x0C;
const FSMSTATUS: u64 = 0x0D;
const FNUM: u64 = 0x10;
const INTENCLR: u64 = 0x14;
const INTENSET: u64 = 0x18;
const INTFLAG: u64 = 0x1C;
const EPINTSMRY: u64 = 0x20;
const DESCADD: u64 = 0x24;
const PADCAL: u64 = 0x28;
const EP_BASE: u64 = 0x100;
const EP_STRIDE: u64 = 0x20;

const CTRLA_SWRST: u8 = 1 << 0;
const CTRLA_ENABLE: u8 = 1 << 1;
const CTRLA_RUNSTDBY: u8 = 1 << 2;
const CTRLA_MODE_HOST: u8 = 1 << 7;
const CTRLB_DETACH: u16 = 1 << 0;
const CTRLB_SPDCONF_LS: u16 = 1 << 2;
const DADD_ADDEN: u8 = 1 << 7;

const INT_SOF: u16 = 1 << 2;
const INT_EORST: u16 = 1 << 3;
const INT_RAMACER: u16 = 1 << 7;
const INT_MASK: u16 = 0x03FF;

const ST_DTGLOUT: u8 = 1 << 0;
const ST_DTGLIN: u8 = 1 << 1;
const ST_STALLRQ0: u8 = 1 << 4;
const ST_STALLRQ1: u8 = 1 << 5;
const ST_BK0RDY: u8 = 1 << 6;
const ST_BK1RDY: u8 = 1 << 7;
const ST_MASK: u8 = 0xF7;

const FL_TRCPT0: u8 = 1 << 0;
const FL_TRCPT1: u8 = 1 << 1;
const FL_TRFAIL0: u8 = 1 << 2;
const FL_TRFAIL1: u8 = 1 << 3;
const FL_RXSTP: u8 = 1 << 4;
const FL_STALL0: u8 = 1 << 5;
const FL_STALL1: u8 = 1 << 6;
const FL_MASK: u8 = 0x7F;

const EPTYPE_CONTROL: u8 = 1;
const EPTYPE_BULK: u8 = 3;
const EPTYPE_INTERRUPT: u8 = 4;

const PCK_BYTE_COUNT: u32 = 0x3FFF;
const PCK_MULTI_SHIFT: u32 = 14;
const PCK_AUTO_ZLP: u32 = 1 << 31;

/// PM.APBBMASK (bit 5 = USB) and the PORT group A registers that put
/// PA24/PA25 on function G (USB/DM, USB/DP).
const PM_APBBMASK: u64 = 0x4000_041C;
const PM_APBBMASK_USB: u32 = 1 << 5;
const PORTA_PMUX12: u64 = 0x4100_443C;
const PORTA_PINCFG24: u64 = 0x4100_4458;
const PMUX_G: u8 = 6;

/// Host-side delay before the first bus reset after the pull-up appears
/// (USB 2.0 §7.1.7.3 asks for >= 100 ms; kept short to save simulated time).
const ATTACH_DEBOUNCE_US: u64 = 2_000;
/// Bus reset length (USB 2.0: >= 10 ms; shortened).
const BUS_RESET_US: u64 = 1_000;
/// Gap between two packets the host starts. Longer than a full-speed token
/// plus data packet, so firmware has read a SETUP before its data stage.
const HOST_GAP_US: u64 = 100;
/// Retry period after a NAK.
const HOST_POLL_US: u64 = 50;
/// A transfer the device does not complete within this is abandoned.
const HOST_TIMEOUT_US: u64 = 500_000;
/// Full-speed frame.
const FRAME_US: u64 = 1_000;

#[derive(Debug, Clone, Copy, Default)]
struct Endpoint {
    cfg: u8,
    status: u8,
    flags: u8,
    enables: u8,
    /// The IN transfer ended on a full packet with AUTO_ZLP set: one
    /// zero-length packet is still owed before TRCPT1.
    zlp_owed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// No pull-up on the bus.
    Detached,
    Debounce,
    Resetting,
    /// Between transfers.
    Idle,
    Ctl(CtlStage),
    /// Non-control transfer of a script step.
    Ep,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CtlStage {
    Setup,
    Data,
    Status,
}

/// What the device answered to one token.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    /// No data moved: bank not ready, endpoint disabled or address mismatch.
    Nak,
    Stall,
    /// OUT/SETUP accepted.
    Ack,
    /// IN data and the endpoint's packet size.
    Data(Vec<u8>, usize),
}

#[derive(Debug)]
pub struct Samd21Usb {
    ctrla: u8,
    qosctrl: u8,
    ctrlb: u16,
    dadd: u8,
    frame: u16,
    intflag: u16,
    inten: u16,
    descadd: u32,
    padcal: u16,
    eps: [Endpoint; ENDPOINTS],
    time: Timebase,

    host_enabled: bool,
    phase: Phase,
    /// Cycle at which the host acts next.
    host_at: u64,
    /// Start of the current transfer, for the timeout.
    started: u64,
    /// Address the host assigned with SET_ADDRESS (0 before).
    host_addr: u8,
    /// The bus is out of reset: frames run.
    bus_live: bool,
    next_sof: u64,
    script: VecDeque<HostStep>,
    /// Work queued for after enumeration (class requests, transfers).
    after_enum: VecDeque<HostStep>,
    /// Steps the chip descriptor asked for; queued again on every attach.
    configured_steps: Vec<HostStep>,
    current: Option<HostStep>,
    ctl_buf: Vec<u8>,
    cdc_opened: bool,
    log: UsbHostLog,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
}

impl Default for Samd21Usb {
    fn default() -> Self {
        Self::new()
    }
}

fn refuse<T>(what: &str) -> SimResult<T> {
    Err(SimulationError::NotImplemented(format!(
        "samd21_usb: {what}"
    )))
}

/// `LABWIRED_TRACE_USB=1` logs the host's progress.
fn trace_usb() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LABWIRED_TRACE_USB").is_some())
}

fn le16(value: u16, byte: u64) -> u8 {
    (value >> (8 * byte)) as u8
}

impl Samd21Usb {
    pub fn new() -> Self {
        Self {
            ctrla: 0,
            qosctrl: 0x05,
            ctrlb: CTRLB_DETACH,
            dadd: 0,
            frame: 0,
            intflag: 0,
            inten: 0,
            descadd: 0,
            padcal: 0,
            eps: [Endpoint::default(); ENDPOINTS],
            time: Timebase::default(),
            host_enabled: true,
            phase: Phase::Detached,
            host_at: 0,
            started: 0,
            host_addr: 0,
            bus_live: false,
            next_sof: 0,
            script: VecDeque::new(),
            after_enum: VecDeque::new(),
            configured_steps: Vec::new(),
            current: None,
            ctl_buf: Vec::new(),
            cdc_opened: false,
            log: UsbHostLog::default(),
            sink: None,
        }
    }

    /// Disconnect the simulated host (no enumeration will happen).
    pub fn set_host_enabled(&mut self, on: bool) {
        self.host_enabled = on;
    }

    /// Queue extra host work (class requests, transfers) after enumeration.
    pub fn queue_host_steps(&mut self, steps: impl IntoIterator<Item = HostStep>) {
        let steps: Vec<HostStep> = steps.into_iter().collect();
        self.configured_steps.extend(steps.iter().cloned());
        self.after_enum.extend(steps);
        if self.phase == Phase::Done {
            self.phase = Phase::Idle;
            self.host_at = self.time.now();
        }
    }

    /// Route bytes the device sends on its bulk IN endpoints (CDC data) to a
    /// console capture sink.
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>) {
        self.sink = sink;
    }

    pub fn host_log(&self) -> &UsbHostLog {
        &self.log
    }

    /// Registers back to their reset values (CTRLA.SWRST, system reset).
    fn reset_registers(&mut self) {
        self.ctrla = 0;
        self.qosctrl = 0x05;
        self.ctrlb = CTRLB_DETACH;
        self.dadd = 0;
        self.frame = 0;
        self.intflag = 0;
        self.inten = 0;
        self.descadd = 0;
        self.padcal = 0;
        self.eps = [Endpoint::default(); ENDPOINTS];
        self.detach();
    }

    fn detach(&mut self) {
        self.phase = Phase::Detached;
        self.host_at = 0;
        self.bus_live = false;
        self.current = None;
    }

    /// The device asks for its pull-up.
    fn pullup(&self) -> bool {
        self.ctrla & CTRLA_ENABLE != 0 && self.ctrlb & CTRLB_DETACH == 0
    }

    fn irq(&self) -> bool {
        self.ctrla & CTRLA_ENABLE != 0
            && (self.intflag & self.inten != 0 || self.eps.iter().any(|e| e.flags & e.enables != 0))
    }

    fn refresh_irq(&self) {
        self.time.set_level(self.irq());
    }

    fn endpoint(offset: u64) -> Option<(usize, u64)> {
        let rel = offset.checked_sub(EP_BASE)?;
        let n = (rel / EP_STRIDE) as usize;
        (n < ENDPOINTS).then_some((n, rel % EP_STRIDE))
    }

    fn reg(&self, offset: u64) -> u8 {
        if let Some((n, reg)) = Self::endpoint(offset) {
            let e = &self.eps[n];
            return match reg {
                0x00 => e.cfg,
                // EPSTATUSCLR / EPSTATUSSET are write-only.
                0x06 => e.status,
                0x07 => e.flags,
                0x08 | 0x09 => e.enables,
                _ => 0,
            };
        }
        match offset {
            CTRLA => self.ctrla,
            QOSCTRL => self.qosctrl,
            0x08 | 0x09 => le16(self.ctrlb, offset - CTRLB),
            DADD => self.dadd,
            // SPEED (0 = full, 1 = low) and LINESTATE: idle J on a live bus,
            // SE0 otherwise.
            STATUS => {
                let low = self.ctrlb & CTRLB_SPDCONF_LS != 0;
                let line = match (self.bus_live, low) {
                    (false, _) => 0,
                    (true, false) => 1,
                    (true, true) => 2,
                };
                (line << 6) | ((low as u8) << 2)
            }
            // FSMSTATE: OFF 0x01, ON 0x02.
            FSMSTATUS => {
                if self.bus_live {
                    0x02
                } else {
                    0x01
                }
            }
            // FNUM[13:3]; MFNUM is high speed only.
            0x10 | 0x11 => le16(self.frame << 3, offset - FNUM),
            0x14 | 0x15 => le16(self.inten, offset - INTENCLR),
            0x18 | 0x19 => le16(self.inten, offset - INTENSET),
            0x1C | 0x1D => le16(self.intflag, offset - INTFLAG),
            0x20 | 0x21 => {
                let summary = self.eps.iter().enumerate().fold(0u16, |s, (i, e)| {
                    s | (((e.flags & e.enables != 0) as u16) << i)
                });
                le16(summary, offset - EPINTSMRY)
            }
            0x24..=0x27 => (self.descadd >> (8 * (offset - DESCADD))) as u8,
            0x28 | 0x29 => le16(self.padcal, offset - PADCAL),
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if let Some((n, reg)) = Self::endpoint(offset) {
            let e = &mut self.eps[n];
            match reg {
                0x00 => {
                    let valid = |t: u8| {
                        matches!(t, 0 | EPTYPE_BULK | EPTYPE_INTERRUPT)
                            || (t == EPTYPE_CONTROL && n == 0)
                    };
                    if !valid(value & 7) || !valid((value >> 4) & 7) {
                        return refuse(&format!(
                            "EPCFG{n} = {value:#04x}: only a control endpoint 0 and bulk/interrupt endpoints"
                        ));
                    }
                    e.cfg = value & 0x77;
                }
                0x04 => {
                    e.status &= !(value & ST_MASK);
                    if value & ST_BK1RDY != 0 {
                        e.zlp_owed = false;
                    }
                }
                0x05 => e.status |= value & ST_MASK,
                0x07 => e.flags &= !value,
                0x08 => e.enables &= !value,
                0x09 => e.enables |= value & FL_MASK,
                _ => {}
            }
            return Ok(());
        }
        match offset {
            CTRLA => {
                if value & CTRLA_SWRST != 0 {
                    self.reset_registers();
                    return Ok(());
                }
                if value & CTRLA_MODE_HOST != 0 {
                    return refuse("host mode (CTRLA.MODE = 1)");
                }
                self.ctrla = value & (CTRLA_ENABLE | CTRLA_RUNSTDBY);
                if !self.pullup() {
                    self.detach();
                }
            }
            QOSCTRL => self.qosctrl = value & 0x0F,
            0x08 => {
                if value & 0xF2 != 0 {
                    return refuse(&format!(
                        "CTRLB = {value:#04x}: remote wakeup, NREPLY and test modes"
                    ));
                }
                if value & 0x08 != 0 {
                    return refuse("high speed (CTRLB.SPDCONF >= 2)");
                }
                self.ctrlb = (self.ctrlb & 0xFF00) | u16::from(value);
                if !self.pullup() {
                    self.detach();
                }
            }
            0x09 if value != 0 => {
                return refuse(&format!(
                    "CTRLB[15:8] = {value:#04x}: OPMODE2, GNAK and LPM handshake"
                ));
            }
            DADD => self.dadd = value,
            0x14 | 0x15 => self.inten &= !(u16::from(value) << (8 * (offset - INTENCLR))),
            0x18 | 0x19 => self.inten |= (u16::from(value) << (8 * (offset - INTENSET))) & INT_MASK,
            0x1C | 0x1D => self.intflag &= !(u16::from(value) << (8 * (offset - INTFLAG))),
            0x24..=0x27 => {
                let shift = 8 * (offset - DESCADD);
                self.descadd = (self.descadd & !(0xFF << shift)) | (u32::from(value) << shift);
            }
            0x28 | 0x29 => {
                let shift = 8 * (offset - PADCAL);
                self.padcal = (self.padcal & !(0xFF << shift)) | (u16::from(value) << shift);
            }
            _ => {}
        }
        Ok(())
    }

    // ── descriptor table ────────────────────────────────────────────────────

    fn bank(&self, ep: usize, bank: u64) -> u64 {
        u64::from(self.descadd) + ep as u64 * 32 + bank * 16
    }

    /// (ADDR, PCKSIZE) of a bank; a bus error raises RAMACER.
    fn read_bank(&mut self, bus: &mut dyn Bus, ep: usize, bank: u64) -> Option<(u64, u32)> {
        let at = self.bank(ep, bank);
        match (bus.read_u32(at), bus.read_u32(at + 4)) {
            (Ok(addr), Ok(pck)) => Some((u64::from(addr), pck)),
            _ => {
                self.intflag |= INT_RAMACER;
                None
            }
        }
    }

    fn write_pcksize(&mut self, bus: &mut dyn Bus, ep: usize, bank: u64, pck: u32) {
        let at = self.bank(ep, bank) + 4;
        if crate::with_device_write_bypass(bus, |bus| bus.write_u32(at, pck)).is_err() {
            self.intflag |= INT_RAMACER;
        }
    }

    /// Packet size of PCKSIZE.SIZE: 8 << n, 1023 for n = 7.
    fn packet_size(pck: u32) -> usize {
        match (pck >> 28) & 7 {
            7 => 1023,
            n => 8 << n,
        }
    }

    /// The device answers tokens for the address the host uses.
    fn addressed(&self) -> bool {
        let own = if self.dadd & DADD_ADDEN != 0 {
            self.dadd & 0x7F
        } else {
            0
        };
        own == self.host_addr
    }

    // ── tokens ──────────────────────────────────────────────────────────────

    /// A SETUP token and its 8-byte DATA0 packet on endpoint 0.
    fn token_setup(&mut self, bus: &mut dyn Bus, packet: [u8; 8]) -> Answer {
        if !self.addressed() || self.eps[0].cfg & 7 != EPTYPE_CONTROL {
            return Answer::Nak;
        }
        // An unread RXSTP still owns the buffer.
        if self.eps[0].flags & FL_RXSTP != 0 {
            return Answer::Nak;
        }
        let Some((addr, pck)) = self.read_bank(bus, 0, 0) else {
            return Answer::Nak;
        };
        let wrote = crate::with_device_write_bypass(bus, |bus| {
            packet
                .iter()
                .enumerate()
                .try_for_each(|(i, b)| bus.write_u8(addr + i as u64, *b))
        });
        if wrote.is_err() {
            self.intflag |= INT_RAMACER;
            return Answer::Nak;
        }
        self.write_pcksize(bus, 0, 0, (pck & !PCK_BYTE_COUNT) | 8);
        // A SETUP is always accepted: it drops pending stalls and the IN
        // bank, and the data stage starts on DATA1 both ways.
        let e = &mut self.eps[0];
        e.status &= !(ST_STALLRQ0 | ST_STALLRQ1 | ST_BK1RDY);
        e.status |= ST_BK0RDY | ST_DTGLOUT | ST_DTGLIN;
        e.zlp_owed = false;
        e.flags |= FL_RXSTP;
        Answer::Ack
    }

    /// An OUT token and one data packet.
    fn token_out(&mut self, bus: &mut dyn Bus, ep: usize, data: &[u8]) -> Answer {
        let e = self.eps[ep];
        if !self.addressed() || e.cfg & 7 == 0 {
            return Answer::Nak;
        }
        if e.status & ST_STALLRQ0 != 0 {
            self.eps[ep].flags |= FL_STALL0;
            return Answer::Stall;
        }
        if e.status & ST_BK0RDY != 0 {
            self.eps[ep].flags |= FL_TRFAIL0;
            return Answer::Nak;
        }
        let Some((addr, pck)) = self.read_bank(bus, ep, 0) else {
            return Answer::Nak;
        };
        let size = Self::packet_size(pck);
        let total = ((pck >> PCK_MULTI_SHIFT) & PCK_BYTE_COUNT) as usize;
        // A transfer longer than one packet appends at BYTE_COUNT.
        let multi = total > size;
        let have = if multi {
            (pck & PCK_BYTE_COUNT) as usize
        } else {
            0
        };
        let n = data.len().min(size);
        let wrote = crate::with_device_write_bypass(bus, |bus| {
            data[..n]
                .iter()
                .enumerate()
                .try_for_each(|(i, b)| bus.write_u8(addr + (have + i) as u64, *b))
        });
        if wrote.is_err() {
            self.intflag |= INT_RAMACER;
            return Answer::Nak;
        }
        let count = have + n;
        self.write_pcksize(bus, ep, 0, (pck & !PCK_BYTE_COUNT) | count as u32);
        self.eps[ep].status ^= ST_DTGLOUT;
        if !multi || n < size || count >= total {
            self.eps[ep].status |= ST_BK0RDY;
            self.eps[ep].flags |= FL_TRCPT0;
        }
        Answer::Ack
    }

    /// An IN token.
    fn token_in(&mut self, bus: &mut dyn Bus, ep: usize) -> Answer {
        let e = self.eps[ep];
        if !self.addressed() || (e.cfg >> 4) & 7 == 0 {
            return Answer::Nak;
        }
        if e.status & ST_STALLRQ1 != 0 {
            self.eps[ep].flags |= FL_STALL1;
            return Answer::Stall;
        }
        if e.status & ST_BK1RDY == 0 {
            self.eps[ep].flags |= FL_TRFAIL1;
            return Answer::Nak;
        }
        let Some((addr, pck)) = self.read_bank(bus, ep, 1) else {
            return Answer::Nak;
        };
        let size = Self::packet_size(pck);
        if e.zlp_owed {
            self.complete_in(ep);
            return Answer::Data(Vec::new(), size);
        }
        let total = (pck & PCK_BYTE_COUNT) as usize;
        let multi = total > size;
        let sent = if multi {
            (((pck >> PCK_MULTI_SHIFT) & PCK_BYTE_COUNT) as usize).min(total)
        } else {
            0
        };
        let n = (total - sent).min(size);
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            match bus.read_u8(addr + (sent + i) as u64) {
                Ok(b) => data.push(b),
                Err(_) => {
                    self.intflag |= INT_RAMACER;
                    return Answer::Nak;
                }
            }
        }
        if multi {
            let progress = ((sent + n) as u32) << PCK_MULTI_SHIFT;
            let pck = (pck & !(PCK_BYTE_COUNT << PCK_MULTI_SHIFT)) | progress;
            self.write_pcksize(bus, ep, 1, pck);
        }
        self.eps[ep].status ^= ST_DTGLIN;
        if sent + n == total {
            if pck & PCK_AUTO_ZLP != 0 && n == size {
                self.eps[ep].zlp_owed = true;
            } else {
                self.complete_in(ep);
            }
        }
        Answer::Data(data, size)
    }

    fn complete_in(&mut self, ep: usize) {
        let e = &mut self.eps[ep];
        e.zlp_owed = false;
        e.status &= !ST_BK1RDY;
        e.flags |= FL_TRCPT1;
    }

    // ── host ────────────────────────────────────────────────────────────────

    /// The pull-up reaches the host: the block is clocked and owns its pads.
    fn pads_driven(bus: &mut dyn Bus) -> bool {
        let clocked = bus
            .read_u32(PM_APBBMASK)
            .is_ok_and(|m| m & PM_APBBMASK_USB != 0);
        let muxed = bus
            .read_u8(PORTA_PMUX12)
            .is_ok_and(|m| m == (PMUX_G << 4) | PMUX_G)
            && (0..2).all(|i| {
                bus.read_u8(PORTA_PINCFG24 + i)
                    .is_ok_and(|cfg| cfg & 1 != 0)
            });
        if trace_usb() {
            tracing::info!("samd21_usb: pull-up asked; clocked={clocked} pads muxed={muxed}");
        }
        clocked && muxed
    }

    /// End of a bus reset (DS40001882H §32.6.2.3): address, frame number,
    /// endpoint interrupts, stalls and toggles clear, and every endpoint but
    /// a control one is disabled.
    fn end_of_reset(&mut self) {
        self.dadd = 0;
        self.frame = 0;
        for e in &mut self.eps {
            if e.cfg & 7 != EPTYPE_CONTROL {
                e.cfg = 0;
            }
            e.status &= !(ST_STALLRQ0 | ST_STALLRQ1 | ST_DTGLOUT | ST_DTGLIN);
            e.flags = 0;
            e.enables = 0;
            e.zlp_owed = false;
        }
        self.intflag |= INT_EORST;
    }

    fn idle_after(&mut self, us: u64) {
        self.phase = Phase::Idle;
        self.host_at = self.time.now() + self.time.us(us);
    }

    fn poll_again(&mut self) {
        self.host_at = self.time.now() + self.time.us(HOST_POLL_US);
    }

    fn fail_control(&mut self, setup: Setup, why: &str) {
        if trace_usb() {
            tracing::info!(
                "samd21_usb: control {:02x?} {why} at cycle {}",
                setup.bytes(),
                self.time.now()
            );
        }
        self.log.notes.push(format!(
            "control {:02x} {:02x} {:04x} {why}",
            setup.bm_request_type, setup.b_request, setup.w_value
        ));
        self.log
            .control
            .push((setup, std::mem::take(&mut self.ctl_buf), false));
        self.idle_after(HOST_GAP_US);
    }

    fn finish_control(&mut self, setup: Setup) {
        if trace_usb() {
            tracing::info!(
                "samd21_usb: control {:02x?} done, {} byte(s), at cycle {}",
                setup.bytes(),
                self.ctl_buf.len(),
                self.time.now()
            );
        }
        if setup.bm_request_type == 0 && setup.b_request == 5 {
            self.host_addr = setup.w_value as u8;
        }
        let data = std::mem::take(&mut self.ctl_buf);
        usb_host::after_control(&mut self.log, &mut self.script, setup, data);
        if self.script.is_empty() && !self.cdc_opened && self.sink.is_some() {
            self.cdc_opened = true;
            for step in usb_host::cdc_open_steps(&self.log.config_descriptor)
                .into_iter()
                .rev()
            {
                self.after_enum.push_front(step);
            }
        }
        self.idle_after(HOST_GAP_US);
    }

    /// Advance the host one step.
    fn host_step(&mut self, bus: &mut dyn Bus) {
        let now = self.time.now();
        if !self.pullup() {
            self.detach();
            return;
        }
        if now < self.host_at {
            return;
        }
        match self.phase.clone() {
            Phase::Detached => {
                if Self::pads_driven(bus) {
                    self.phase = Phase::Debounce;
                    self.host_at = now + self.time.us(ATTACH_DEBOUNCE_US);
                } else {
                    self.poll_again();
                }
            }
            Phase::Debounce => {
                self.log.resets += 1;
                self.host_addr = 0;
                self.script = usb_host::enumeration_script();
                self.after_enum = self.configured_steps.iter().cloned().collect();
                self.cdc_opened = false;
                self.bus_live = false;
                self.phase = Phase::Resetting;
                self.host_at = now + self.time.us(BUS_RESET_US);
            }
            Phase::Resetting => {
                self.end_of_reset();
                self.bus_live = true;
                self.next_sof = now + self.time.us(FRAME_US);
                self.idle_after(HOST_GAP_US);
            }
            Phase::Idle => {
                let Some(step) = self
                    .script
                    .pop_front()
                    .or_else(|| self.after_enum.pop_front())
                else {
                    self.phase = Phase::Done;
                    return;
                };
                self.started = now;
                self.ctl_buf.clear();
                match &step {
                    HostStep::Control(_) => self.phase = Phase::Ctl(CtlStage::Setup),
                    HostStep::Wait { us } => self.host_at = now + self.time.us(*us),
                    HostStep::Out { .. } | HostStep::In { .. } => self.phase = Phase::Ep,
                }
                self.current = Some(step);
            }
            Phase::Ctl(stage) => {
                let Some(HostStep::Control(setup)) = self.current.clone() else {
                    self.idle_after(0);
                    return;
                };
                if now.saturating_sub(self.started) > self.time.us(HOST_TIMEOUT_US) {
                    self.fail_control(setup, &format!("timed out in {stage:?}"));
                    return;
                }
                self.control_stage(bus, setup, stage);
            }
            Phase::Ep => {
                let step = self.current.clone();
                if now.saturating_sub(self.started) > self.time.us(HOST_TIMEOUT_US) {
                    self.log
                        .notes
                        .push(format!("{step:?} timed out: endpoint never ready"));
                    self.idle_after(HOST_GAP_US);
                    return;
                }
                let answer = match &step {
                    Some(HostStep::Out { ep, data }) => {
                        let answer = self.token_out(bus, usize::from(*ep & 7), data);
                        if answer == Answer::Ack {
                            self.log.outs.push((*ep, data.len()));
                        }
                        answer
                    }
                    Some(HostStep::In { ep, max }) => {
                        let answer = self.token_in(bus, usize::from(*ep & 7));
                        if let Answer::Data(data, _) = &answer {
                            let n = data.len().min(*max);
                            self.log.ins.push((*ep, data[..n].to_vec()));
                        }
                        answer
                    }
                    _ => Answer::Ack,
                };
                match answer {
                    Answer::Nak => self.poll_again(),
                    Answer::Stall => {
                        self.log.notes.push(format!("{step:?} stalled"));
                        self.idle_after(HOST_GAP_US);
                    }
                    Answer::Ack | Answer::Data(..) => self.idle_after(HOST_GAP_US),
                }
            }
            Phase::Done => {}
        }
    }

    fn control_stage(&mut self, bus: &mut dyn Bus, setup: Setup, stage: CtlStage) {
        let gap = self.time.us(HOST_GAP_US);
        let now = self.time.now();
        let length = usize::from(setup.w_length);
        match stage {
            CtlStage::Setup => match self.token_setup(bus, setup.bytes()) {
                Answer::Ack => {
                    self.phase = Phase::Ctl(if length == 0 {
                        CtlStage::Status
                    } else {
                        CtlStage::Data
                    });
                    self.host_at = now + gap;
                }
                _ => self.poll_again(),
            },
            CtlStage::Data if setup.dir_in() => match self.token_in(bus, 0) {
                Answer::Data(chunk, size) => {
                    let short = chunk.len() < size;
                    self.ctl_buf.extend(chunk);
                    self.ctl_buf.truncate(length);
                    if short || self.ctl_buf.len() >= length {
                        self.phase = Phase::Ctl(CtlStage::Status);
                    }
                    self.host_at = now + gap;
                }
                Answer::Stall => self.fail_control(setup, "stalled"),
                _ => self.poll_again(),
            },
            CtlStage::Data => {
                let sent = self.ctl_buf.len();
                let size = match self.read_bank(bus, 0, 0) {
                    Some((_, pck)) => Self::packet_size(pck),
                    None => 64,
                };
                let end = (sent + size).min(setup.data.len());
                match self.token_out(bus, 0, &setup.data[sent..end]) {
                    Answer::Ack => {
                        self.ctl_buf.extend_from_slice(&setup.data[sent..end]);
                        if end == setup.data.len() {
                            self.phase = Phase::Ctl(CtlStage::Status);
                        }
                        self.host_at = now + gap;
                    }
                    Answer::Stall => self.fail_control(setup, "stalled"),
                    _ => self.poll_again(),
                }
            }
            // The status stage runs opposite to the data stage (IN for a
            // request without data).
            CtlStage::Status => {
                let answer = if setup.dir_in() && length > 0 {
                    self.token_out(bus, 0, &[])
                } else {
                    self.token_in(bus, 0)
                };
                match answer {
                    Answer::Ack | Answer::Data(..) => self.finish_control(setup),
                    Answer::Stall => self.fail_control(setup, "stalled"),
                    Answer::Nak => self.poll_again(),
                }
            }
        }
    }

    fn bulk_in(&self, ep: usize) -> bool {
        (self.eps[ep].cfg >> 4) & 7 == EPTYPE_BULK
    }

    /// The port is open and a bulk IN endpoint holds data for the reader.
    fn console_pending(&self) -> bool {
        self.sink.is_some()
            && self.cdc_opened
            && self.phase == Phase::Done
            && (1..ENDPOINTS).any(|ep| self.bulk_in(ep) && self.eps[ep].status & ST_BK1RDY != 0)
    }

    /// The host side of the serial port: read every ready bulk IN endpoint.
    fn service_console(&mut self, bus: &mut dyn Bus) {
        if !self.console_pending() {
            return;
        }
        for ep in 1..ENDPOINTS {
            while self.bulk_in(ep) && self.eps[ep].status & ST_BK1RDY != 0 {
                let Answer::Data(data, _) = self.token_in(bus, ep) else {
                    break;
                };
                if let Some(sink) = &self.sink {
                    if let Ok(mut s) = sink.lock() {
                        s.extend_from_slice(&data);
                    }
                }
            }
        }
    }

    /// Start of frame: FNUM and the SOF flag. An open port's reader polls
    /// its bulk IN endpoints every frame; one with nothing to send NAKs.
    fn service_frames(&mut self) {
        if !self.bus_live {
            return;
        }
        let now = self.time.now();
        if now < self.next_sof {
            return;
        }
        let period = self.time.us(FRAME_US).max(1);
        let frames = 1 + (now - self.next_sof) / period;
        self.frame = ((u64::from(self.frame) + frames) & 0x7FF) as u16;
        self.next_sof += frames * period;
        self.intflag |= INT_SOF;
        if self.sink.is_some() && self.cdc_opened && self.phase == Phase::Done {
            for ep in 1..ENDPOINTS {
                if self.bulk_in(ep) && self.eps[ep].status & ST_BK1RDY == 0 {
                    self.eps[ep].flags |= FL_TRFAIL1;
                }
            }
        }
    }

    fn host_active(&self) -> bool {
        self.host_enabled && self.pullup() && self.phase != Phase::Done
    }
}

impl Peripheral for Samd21Usb {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.reg(offset))
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let r = self.write_reg(offset, value);
        self.refresh_irq();
        r
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        Some(self.reg(offset))
    }
    /// Walked only while timed work is in flight or the interrupt line is
    /// asserted (so its deassert is reconciled); MMIO re-arms it.
    fn legacy_tick_active(&self) -> bool {
        self.host_active() || self.bus_live || self.time.level()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.time.advance(cycles);
        self.service_frames();
        self.refresh_irq();
        let host = self.host_active().then_some(self.host_at);
        let sof = self.bus_live.then_some(self.next_sof);
        let until = match (host, sof) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        wake_hint(self.time.now(), until)
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.time.level())
    }
    fn needs_bus_tick(&self) -> bool {
        // The bus caches this answer between MMIO accesses, so it cannot
        // depend on time: `host_step` returns early until the host is due.
        self.host_active() || self.console_pending()
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if self.host_enabled {
            self.host_step(bus);
            self.service_console(bus);
        }
        self.refresh_irq();
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.time.attach_clock(clock);
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.time.attach_cpu_hz(hz);
    }
    fn attach_usb_cdc_console(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>) -> bool {
        self.set_sink(sink);
        true
    }
    fn on_system_reset(&mut self) {
        self.reset_registers();
        self.refresh_irq();
    }
    fn on_node_reset(&mut self) {
        self.on_system_reset();
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        vec![crate::peripheral_log::PeripheralLog::new(
            "host",
            self.log.lines(),
        )]
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "samd21_usb",
            "ctrla": self.ctrla,
            "ctrlb": self.ctrlb,
            "dadd": self.dadd,
            "frame": self.frame,
            "intflag": self.intflag,
            "descadd": self.descadd,
            "phase": format!("{:?}", self.phase),
            "address": self.log.address,
            "controls": self.log.control.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CycleClock, DmaRequest, SimulationConfig};
    use std::collections::HashMap;

    const TABLE: u32 = 0x2000_0100;
    const EP0_OUT: u32 = 0x2000_0400;
    const EP0_IN: u32 = 0x2000_0440;
    const EP1_IN: u32 = 0x2000_0500;

    /// RAM at 0x2000_0000 plus the PM / PORT registers the pull-up gate reads.
    struct Memory {
        bytes: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl Memory {
        fn new() -> Self {
            let mut m = Self {
                bytes: HashMap::new(),
                config: SimulationConfig::default(),
            };
            m.word(PM_APBBMASK, PM_APBBMASK_USB);
            m.bytes.insert(PORTA_PMUX12, 0x66);
            m.bytes.insert(PORTA_PINCFG24, 1);
            m.bytes.insert(PORTA_PINCFG24 + 1, 1);
            m
        }
        fn word(&mut self, address: u64, value: u32) {
            for (i, b) in value.to_le_bytes().into_iter().enumerate() {
                self.bytes.insert(address + i as u64, b);
            }
        }
        /// Point a bank at `buffer`: 64-byte packets, `count` bytes armed.
        fn bank(&mut self, ep: u64, bank: u64, buffer: u32, pck: u32) {
            let at = u64::from(TABLE) + ep * 32 + bank * 16;
            self.word(at, buffer);
            self.word(at + 4, (3 << 28) | pck);
        }
        fn pck(&self, ep: u64, bank: u64) -> u32 {
            self.read_u32(u64::from(TABLE) + ep * 32 + bank * 16 + 4)
                .unwrap()
        }
    }

    impl Bus for Memory {
        fn read_u8(&self, address: u64) -> SimResult<u8> {
            if let Some(value) = self.bytes.get(&address) {
                return Ok(*value);
            }
            if (0x2000_0000..0x2000_8000).contains(&address) {
                return Ok(0);
            }
            Err(SimulationError::NotImplemented("test unmapped read".into()))
        }
        fn write_u8(&mut self, address: u64, value: u8) -> SimResult<()> {
            if !(0x2000_0000..0x2000_8000).contains(&address) {
                return Err(SimulationError::NotImplemented(
                    "test unmapped write".into(),
                ));
            }
            self.bytes.insert(address, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            vec![]
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    /// An enabled, attached device with endpoint 0 as a control endpoint.
    fn device() -> (Samd21Usb, Memory, CycleClock) {
        let mut u = Samd21Usb::new();
        let clock = CycleClock::default();
        u.attach_cycle_clock(clock.clone());
        u.attach_cpu_hz(48_000_000);
        let mut m = Memory::new();
        m.bank(0, 0, EP0_OUT, 0);
        m.bank(0, 1, EP0_IN, 0);
        for (i, b) in TABLE.to_le_bytes().into_iter().enumerate() {
            u.write(DESCADD + i as u64, b).unwrap();
        }
        u.write(CTRLA, CTRLA_ENABLE).unwrap();
        u.write(CTRLB, 0).unwrap();
        u.write(EP_BASE, 0x11).unwrap();
        (u, m, clock)
    }

    /// A device the host has reset, with `script` as the host's whole plan.
    fn attached(script: Vec<HostStep>) -> (Samd21Usb, Memory, CycleClock) {
        let (mut u, mut m, clock) = device();
        assert!(run(&mut u, &mut m, &clock, 5_000, |u| u.bus_live));
        u.script = VecDeque::from(script);
        (u, m, clock)
    }

    /// Run the host until `done` or `us` microseconds pass.
    fn run(
        u: &mut Samd21Usb,
        m: &mut Memory,
        clock: &CycleClock,
        us: u64,
        mut done: impl FnMut(&Samd21Usb) -> bool,
    ) -> bool {
        let end = clock.now() + 48 * us;
        while clock.now() < end {
            if done(u) {
                return true;
            }
            clock.publish(clock.now() + 48);
            u.tick_elapsed(0);
            if u.needs_bus_tick() {
                u.tick_with_bus(m);
            }
        }
        done(u)
    }

    #[test]
    fn host_stays_away_until_the_block_is_clocked_and_owns_its_pads() {
        let (mut u, mut m, clock) = device();
        m.bytes.insert(PORTA_PMUX12, 0);
        assert!(!run(&mut u, &mut m, &clock, 5_000, |u| u.bus_live));
        assert_eq!(u.log.resets, 0);
        m.bytes.insert(PORTA_PMUX12, 0x66);
        assert!(run(&mut u, &mut m, &clock, 5_000, |u| u.bus_live));
        assert_eq!(u.log.resets, 1);
        // DETACH takes the pull-up away again.
        u.write(CTRLB, 1).unwrap();
        assert!(!u.bus_live);
        assert_eq!(u.reg(FSMSTATUS), 0x01);
    }

    #[test]
    fn bus_reset_flags_eorst_and_keeps_only_the_control_endpoint() {
        let (mut u, mut m, clock) = device();
        u.write(EP_BASE + EP_STRIDE, 0x30).unwrap();
        u.write(EP_BASE + 0x09, FL_RXSTP).unwrap();
        u.write(DADD, DADD_ADDEN | 9).unwrap();
        u.write(INTENSET, INT_EORST as u8).unwrap();
        assert!(run(&mut u, &mut m, &clock, 5_000, |u| u.bus_live));
        assert_eq!(u.reg(INTFLAG) & INT_EORST as u8, INT_EORST as u8);
        assert_eq!(u.reg(EP_BASE), 0x11);
        assert_eq!(u.reg(EP_BASE + EP_STRIDE), 0);
        assert_eq!(u.reg(EP_BASE + 0x09), 0);
        assert_eq!(u.reg(DADD), 0);
        assert!(u.irq_line_level().unwrap());
        u.write(INTFLAG, INT_EORST as u8).unwrap();
        assert!(!u.irq_line_level().unwrap());
    }

    #[test]
    fn frames_count_and_flag_sof_every_millisecond() {
        let (mut u, mut m, clock) = attached(Vec::new());
        let frame = |u: &Samd21Usb| u16::from_le_bytes([u.reg(FNUM), u.reg(FNUM + 1)]) >> 3;
        let start = frame(&u);
        u.write(INTFLAG, INT_SOF as u8).unwrap();
        run(&mut u, &mut m, &clock, 3_000, |_| false);
        assert_eq!(frame(&u), start + 3);
        assert_ne!(u.reg(INTFLAG) & INT_SOF as u8, 0);
    }

    #[test]
    fn setup_lands_in_bank_0_and_drops_the_in_bank_and_stalls() {
        let (mut u, mut m, _clock) = device();
        u.write(EP_BASE + 0x05, ST_BK1RDY | ST_STALLRQ1).unwrap();
        let packet = [0x80, 6, 0, 1, 0, 0, 18, 0];
        assert_eq!(u.token_setup(&mut m, packet), Answer::Ack);
        for (i, b) in packet.into_iter().enumerate() {
            assert_eq!(m.read_u8(u64::from(EP0_OUT) + i as u64).unwrap(), b);
        }
        assert_eq!(m.pck(0, 0) & PCK_BYTE_COUNT, 8);
        assert_eq!(
            u.reg(EP_BASE + 0x06),
            ST_BK0RDY | ST_DTGLOUT | ST_DTGLIN,
            "BK0RDY set; BK1RDY and STALLRQ1 dropped"
        );
        assert_eq!(u.reg(EP_BASE + 0x07), FL_RXSTP);
        // A second SETUP waits for firmware to read the first.
        assert_eq!(u.token_setup(&mut m, packet), Answer::Nak);
        // RXSTP interrupts only when enabled, and shows in EPINTSMRY.
        assert!(!u.irq());
        u.write(EP_BASE + 0x09, FL_RXSTP).unwrap();
        assert!(u.irq());
        assert_eq!(u.reg(EPINTSMRY), 1);
    }

    #[test]
    fn in_longer_than_a_packet_goes_out_in_packets_and_tracks_progress() {
        let (mut u, mut m, _clock) = device();
        for i in 0..75u64 {
            m.bytes.insert(u64::from(EP0_IN) + i, i as u8);
        }
        m.bank(0, 1, EP0_IN, 75);
        // Not ready: the IN is NAKed and TRFAIL1 says so.
        assert_eq!(u.token_in(&mut m, 0), Answer::Nak);
        assert_eq!(u.reg(EP_BASE + 0x07), FL_TRFAIL1);
        u.write(EP_BASE + 0x07, FL_TRFAIL1).unwrap();
        u.write(EP_BASE + 0x05, ST_BK1RDY).unwrap();
        let Answer::Data(first, 64) = u.token_in(&mut m, 0) else {
            panic!("first packet");
        };
        assert_eq!(first, (0..64).collect::<Vec<u8>>());
        assert_eq!((m.pck(0, 1) >> PCK_MULTI_SHIFT) & PCK_BYTE_COUNT, 64);
        assert_eq!(u.reg(EP_BASE + 0x07), 0, "transfer still running");
        let Answer::Data(rest, 64) = u.token_in(&mut m, 0) else {
            panic!("second packet");
        };
        assert_eq!(rest, (64..75).collect::<Vec<u8>>());
        assert_eq!(u.reg(EP_BASE + 0x07), FL_TRCPT1);
        assert_eq!(u.reg(EP_BASE + 0x06) & ST_BK1RDY, 0);
    }

    #[test]
    fn single_packet_in_ignores_a_stale_multi_packet_size() {
        let (mut u, mut m, _clock) = device();
        u.write(EP_BASE + EP_STRIDE, 0x30).unwrap();
        m.bytes.insert(u64::from(EP1_IN), b'A');
        m.bytes.insert(u64::from(EP1_IN) + 1, b'B');
        m.bank(1, 1, EP1_IN, (10 << PCK_MULTI_SHIFT) | 2);
        u.write(EP_BASE + EP_STRIDE + 0x05, ST_BK1RDY).unwrap();
        assert_eq!(u.token_in(&mut m, 1), Answer::Data(b"AB".to_vec(), 64));
        assert_eq!(u.reg(EP_BASE + EP_STRIDE + 0x07), FL_TRCPT1);
    }

    #[test]
    fn auto_zlp_follows_a_full_last_packet() {
        let (mut u, mut m, _clock) = device();
        u.write(EP_BASE + EP_STRIDE, 0x30).unwrap();
        m.bank(1, 1, EP1_IN, PCK_AUTO_ZLP | 64);
        u.write(EP_BASE + EP_STRIDE + 0x05, ST_BK1RDY).unwrap();
        assert!(matches!(u.token_in(&mut m, 1), Answer::Data(d, 64) if d.len() == 64));
        assert_eq!(u.reg(EP_BASE + EP_STRIDE + 0x07), 0);
        assert_eq!(u.token_in(&mut m, 1), Answer::Data(Vec::new(), 64));
        assert_eq!(u.reg(EP_BASE + EP_STRIDE + 0x07), FL_TRCPT1);
    }

    #[test]
    fn out_fills_bank_0_and_a_full_bank_naks() {
        let (mut u, mut m, _clock) = device();
        m.bank(0, 0, EP0_OUT, 64 << PCK_MULTI_SHIFT);
        assert_eq!(u.token_out(&mut m, 0, &[1, 2, 3]), Answer::Ack);
        assert_eq!(m.read_u8(u64::from(EP0_OUT) + 2).unwrap(), 3);
        assert_eq!(m.pck(0, 0) & PCK_BYTE_COUNT, 3);
        assert_eq!(u.reg(EP_BASE + 0x07), FL_TRCPT0);
        assert_ne!(u.reg(EP_BASE + 0x06) & ST_BK0RDY, 0);
        assert_eq!(u.token_out(&mut m, 0, &[4]), Answer::Nak);
        assert_eq!(u.reg(EP_BASE + 0x07), FL_TRCPT0 | FL_TRFAIL0);
        // A stall request answers STALL and flags it.
        u.write(EP_BASE + 0x04, ST_BK0RDY).unwrap();
        u.write(EP_BASE + 0x05, ST_STALLRQ0).unwrap();
        assert_eq!(u.token_out(&mut m, 0, &[4]), Answer::Stall);
        assert_ne!(u.reg(EP_BASE + 0x07) & FL_STALL0, 0);
    }

    #[test]
    fn tokens_for_another_address_get_no_answer() {
        let (mut u, mut m, _clock) = device();
        u.write(DADD, DADD_ADDEN | 5).unwrap();
        assert_eq!(u.token_setup(&mut m, [0; 8]), Answer::Nak);
        u.host_addr = 5;
        assert_eq!(u.token_setup(&mut m, [0; 8]), Answer::Ack);
    }

    #[test]
    fn a_descriptor_table_outside_memory_raises_ramacer() {
        let (mut u, mut m, _clock) = device();
        for (i, b) in 0x1000_0000u32.to_le_bytes().into_iter().enumerate() {
            u.write(DESCADD + i as u64, b).unwrap();
        }
        assert_eq!(u.token_setup(&mut m, [0; 8]), Answer::Nak);
        assert_ne!(u.reg(INTFLAG) & INT_RAMACER as u8, 0);
    }

    #[test]
    fn status_set_and_clear_read_zero_and_swrst_restores_reset_values() {
        let (mut u, _m, _clock) = device();
        u.write(EP_BASE + 0x05, ST_BK1RDY).unwrap();
        assert_eq!(u.reg(EP_BASE + 0x04), 0);
        assert_eq!(u.reg(EP_BASE + 0x05), 0);
        assert_eq!(u.reg(EP_BASE + 0x06), ST_BK1RDY);
        u.write(CTRLA, CTRLA_SWRST).unwrap();
        assert_eq!(u.reg(CTRLA), 0);
        assert_eq!(u.reg(CTRLB), 1);
        assert_eq!(u.reg(QOSCTRL), 5);
        assert_eq!(u.reg(EP_BASE), 0);
        assert_eq!(u.reg(DESCADD), 0);
    }

    #[test]
    fn unmodelled_modes_are_refused() {
        let mut u = Samd21Usb::new();
        assert!(u.write(CTRLA, CTRLA_MODE_HOST | CTRLA_ENABLE).is_err());
        assert!(u.write(CTRLB, 0x08).is_err(), "high speed");
        assert!(u.write(CTRLB, 0x02).is_err(), "remote wakeup");
        assert!(u.write(CTRLB + 1, 0x02).is_err(), "GNAK");
        assert!(u.write(EP_BASE, 0x02).is_err(), "isochronous");
        assert!(
            u.write(EP_BASE + EP_STRIDE, 0x01).is_err(),
            "control endpoint other than 0"
        );
    }

    /// A control IN transfer end to end against a device that answers a
    /// GET_DESCRIPTOR with 18 bytes, then takes the status ZLP.
    #[test]
    fn host_runs_a_control_in_transfer_and_logs_the_descriptor() {
        let (mut u, mut m, clock) =
            attached(vec![HostStep::Control(Setup::get_descriptor(1, 0, 0, 18))]);
        assert!(run(&mut u, &mut m, &clock, 10_000, |u| {
            u.reg(EP_BASE + 0x07) & FL_RXSTP != 0
        }));
        // Firmware: read the SETUP, free bank 0, arm 18 bytes on bank 1.
        // No string indices (bytes 14..16), so the host asks nothing more.
        let descriptor = [
            18, 1, 0, 2, 0, 0, 0, 64, 0x41, 0x23, 0x57, 0x80, 0, 1, 0, 0, 0, 1,
        ];
        for (i, b) in descriptor.iter().enumerate() {
            m.bytes.insert(u64::from(EP0_IN) + i as u64, *b);
        }
        m.bank(0, 1, EP0_IN, 18);
        u.write(EP_BASE + 0x04, ST_BK0RDY).unwrap();
        u.write(EP_BASE + 0x05, ST_BK1RDY).unwrap();
        u.write(EP_BASE + 0x07, FL_MASK).unwrap();
        assert!(run(&mut u, &mut m, &clock, 10_000, |u| u.phase == Phase::Done));
        assert_eq!(u.log.device_descriptor, descriptor);
        assert!(u.log.control[0].2, "completed");
        // The status stage was an OUT ZLP into bank 0.
        assert_eq!(m.pck(0, 0) & PCK_BYTE_COUNT, 0);
        assert_ne!(u.reg(EP_BASE + 0x07) & FL_TRCPT0, 0);
    }

    #[test]
    fn a_stalled_control_request_is_logged_as_incomplete() {
        let (mut u, mut m, clock) =
            attached(vec![HostStep::Control(Setup::get_descriptor(6, 0, 0, 10))]);
        assert!(run(&mut u, &mut m, &clock, 10_000, |u| {
            u.reg(EP_BASE + 0x07) & FL_RXSTP != 0
        }));
        u.write(EP_BASE + 0x05, ST_STALLRQ1).unwrap();
        assert!(run(&mut u, &mut m, &clock, 10_000, |u| u.phase == Phase::Done));
        assert!(!u.log.control[0].2);
        assert!(u.log.notes[0].ends_with("stalled"), "{:?}", u.log.notes);
    }

    #[test]
    fn open_port_reader_drains_bulk_in_into_the_console_sink() {
        let (mut u, mut m, clock) = attached(Vec::new());
        let sink = Arc::new(Mutex::new(Vec::new()));
        assert!(u.attach_usb_cdc_console(Some(sink.clone())));
        assert!(run(&mut u, &mut m, &clock, 10_000, |u| u.phase == Phase::Done));
        u.cdc_opened = true;
        u.write(EP_BASE + EP_STRIDE, 0x30).unwrap();
        for (i, b) in b"hi\n".iter().enumerate() {
            m.bytes.insert(u64::from(EP1_IN) + i as u64, *b);
        }
        m.bank(1, 1, EP1_IN, 3);
        u.write(EP_BASE + EP_STRIDE + 0x05, ST_BK1RDY).unwrap();
        assert!(u.needs_bus_tick());
        u.tick_with_bus(&mut m);
        assert_eq!(*sink.lock().unwrap(), b"hi\n");
        assert_eq!(u.reg(EP_BASE + EP_STRIDE + 0x07), FL_TRCPT1);
        // With nothing armed the reader's next poll is NAKed.
        u.write(EP_BASE + EP_STRIDE + 0x07, FL_MASK).unwrap();
        run(&mut u, &mut m, &clock, 1_100, |_| false);
        assert_eq!(u.reg(EP_BASE + EP_STRIDE + 0x07), FL_TRFAIL1);
    }
}
