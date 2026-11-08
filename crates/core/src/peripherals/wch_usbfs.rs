// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! WCH USBFS / USBOTG_FS controller in device mode (CH32V20x RM chapter
//! "USB full-speed host/device controller (USBFS)", `0x5000_0000`), plus the
//! scripted USB host of [`crate::peripherals::usb_host`] on the other end of
//! the cable.
//!
//! Register map (byte registers unless noted; ch32fun `USBOTG_FS_TypeDef`,
//! WCH EVT `ch32v20x_usb.h`):
//!
//! | Off | Register | Model |
//! |---|---|---|
//! | 0x00 | `BASE_CTRL` | `RESET_SIE` resets the SIE registers, `CLR_ALL` clears the flags and `RX_LEN`; `DEV_PU_EN` + `UDEV_CTRL.PORT_EN` attach the device (device mode only) |
//! | 0x01 | `UDEV_CTRL` | stored; `DP_PIN`/`DM_PIN` read the idle full-speed J state while attached |
//! | 0x02 | `INT_EN` | `BUS_RST`, `TRANSFER`, `SUSPEND`, `HST_SOF`, `FIFO_OV` gate the interrupt line |
//! | 0x03 | `DEV_ADDR` | the device answers only tokens sent to `DEV_ADDR & 0x7F` |
//! | 0x05 | `MIS_ST` | `SIE_FREE`, and `BUS_RESET` while the host holds the bus in reset |
//! | 0x06 | `INT_FG` | bits 0..4 write-1-to-clear; bit 7 `IS_NAK` (0), bit 6 `TOG_OK`, bit 5 `SIE_FREE` read-only |
//! | 0x07 | `INT_ST` | token PID (bits 5:4) and endpoint (3:0) of the last transfer, `TOG_OK` |
//! | 0x08 | `RX_LEN` (u16) | bytes the last OUT/SETUP delivered |
//! | 0x0C..0x0F | `UEPn_MOD` | `TX_EN`/`RX_EN` per endpoint 1..7 |
//! | 0x10..0x2C | `UEPn_DMA` (u32) | endpoint buffer address in system RAM |
//! | 0x30 + 4n | `UEPn_TX_LEN` (u16), `UEPn_TX_CTRL`, `UEPn_RX_CTRL` | handshake (`ACK`/`NONE`/`NAK`/`STALL`) per direction |
//! | 0x54, 0x58 | `OTG_CR`, `OTG_SR` | stored / reads 0 (the OTG block is a CH32V205/207/305/307 feature) |
//!
//! The controller is a bus master: a SETUP or OUT packet is written to the
//! endpoint's buffer in RAM, and an IN packet is read from it, exactly as on
//! silicon. Buffer layout (single buffer, `BUF_MOD` = 0): endpoint 0 shares
//! one 64-byte buffer for both directions at `UEP0_DMA`; on endpoints 1..7
//! the OUT buffer sits at `UEPn_DMA` and the IN buffer at `UEPn_DMA + 64`
//! when both directions are enabled, else at `UEPn_DMA`.
//!
//! Each transaction the host completes raises `INT_FG.TRANSFER` with the
//! token and endpoint in `INT_ST`; the host starts no further transaction
//! until firmware clears `TRANSFER` (with `BASE_CTRL.INT_BUSY` silicon NAKs
//! in that window; without it the model still waits). A handshake of `NAK`
//! or `NONE` makes the host retry; `STALL` ends the control transfer as
//! stalled. A full-speed packet takes about `len * 8 / 12` µs on the wire.
//!
//! DATA0/DATA1 sequencing is modelled on both sides, because ch32fun's EP0
//! code depends on it. An IN packet carries the PID in `T_TOG`. The host
//! expects DATA1 for the first data packet after a SETUP and for the status
//! stage, and DATA0 first on an interrupt endpoint after SET_CONFIGURATION.
//! A packet with the wrong PID is ACKed (the device sees a completed
//! transfer) and discarded, and the host asks again, as USB 2.0 §8.6.4
//! requires. ch32fun's control-read setup arms EP0 with `T_TOG` clear over
//! the stale SETUP bytes, and its IN handler copies the descriptor and flips
//! `T_TOG`. So the first packet of every control read is that discarded
//! DATA0. An OUT packet carries the host's PID, and `TOG_OK` (INT_FG bit 6,
//! INT_ST bit 6) says whether it matched `R_TOG`. `AUTO_TOG` is stored but
//! toggles nothing. WCH's EVT header says it toggles endpoints 1..3 after a
//! successful transfer. ch32fun sets it and also flips `T_TOG` by hand in
//! every IN handler, which only works if `AUTO_TOG` does nothing. Real
//! silicon has to settle this; the RM does not.
//!
//! Not modelled: double buffering (`BUF_MOD`); isochronous
//! transfers; SOF, suspend and resume (the host keeps the bus active, so
//! `SUSPEND` never sets); the `DEV_NAK` interrupt; host mode
//! (`BASE_CTRL.HOST_MODE`) and the OTG block; the USBFS wake-up interrupt.

use crate::peripherals::imxrt::{byte_of, wake_hint, Timebase};
use crate::peripherals::usb_host::{self, HostStep, Setup, UsbHostLog};
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::collections::VecDeque;

const BASE_CTRL: usize = 0x00;
const UDEV_CTRL: usize = 0x01;
const INT_EN: usize = 0x02;
const DEV_ADDR: usize = 0x03;
const MIS_ST: usize = 0x05;
const INT_FG: usize = 0x06;
const INT_ST: usize = 0x07;
const RX_LEN: usize = 0x08;
const UEP4_1_MOD: usize = 0x0C;
const UEP_DMA: usize = 0x10;
const UEP_CTRL: usize = 0x30;
const OTG_SR: usize = 0x58;
const REG_BYTES: usize = 0x5C;

const UC_HOST_MODE: u8 = 1 << 7;
const UC_DEV_PU_EN: u8 = 1 << 5;
const UC_RESET_SIE: u8 = 1 << 2;
const UC_CLR_ALL: u8 = 1 << 1;
const UD_DP_PIN: u8 = 1 << 5;
const UD_DM_PIN: u8 = 1 << 4;
const UD_PORT_EN: u8 = 1 << 0;
const UMS_SIE_FREE: u8 = 1 << 5;
const UMS_BUS_RESET: u8 = 1 << 3;
const UIF_BUS_RST: u8 = 1 << 0;
const UIF_TRANSFER: u8 = 1 << 1;
/// INT_FG bits 0..4 are interrupt flags; 5..7 are read-only status.
const UIF_MASK: u8 = 0x1F;
const U_TOG_OK: u8 = 1 << 6;
const UIS_TOKEN_OUT: u8 = 0x00;
const UIS_TOKEN_IN: u8 = 0x20;
const UIS_TOKEN_SETUP: u8 = 0x30;

/// UEPn_TX_CTRL / UEPn_RX_CTRL handshake field.
const RES_MASK: u8 = 3;
const RES_ACK: u8 = 0;
const RES_STALL: u8 = 3;
/// UEPn_TX_CTRL.T_TOG / UEPn_RX_CTRL.R_TOG: 1 = DATA1.
const T_TOG: u8 = 1 << 2;
const R_TOG: u8 = 1 << 2;

const EP0_SIZE: usize = 64;

/// Host-side delay before the first bus reset after the device pulls up
/// (USB 2.0 §7.1.7.3 asks >= 100 ms; shortened to save simulated time).
const ATTACH_DEBOUNCE_US: u64 = 2_000;
/// Bus reset length (USB 2.0: >= 10 ms; shortened).
const BUS_RESET_US: u64 = 1_000;
/// Recovery after reset before the first SETUP (USB 2.0 TRSTRCY: 10 ms;
/// shortened).
const RESET_RECOVERY_US: u64 = 100;
/// Gap between two transactions.
const HOST_GAP_US: u64 = 2;
/// A step the device does not answer within this is abandoned.
const HOST_TIMEOUT_US: u64 = 500_000;

fn trace_usb() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LABWIRED_TRACE_USB").is_some())
}

/// Full-speed wire time of a data packet (12 Mbit/s, no bit stuffing).
fn packet_us(len: usize) -> u64 {
    1 + (len as u64 * 8) / 12
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// Waiting for the device to pull up D+.
    Detached,
    Debounce {
        until: u64,
    },
    Resetting {
        until: u64,
    },
    /// Between steps; the next one may start at `until`.
    Idle {
        until: u64,
    },
    /// Control transfer in progress.
    Ctl {
        stage: CtlStage,
        started: u64,
    },
    /// Interrupt/bulk IN transfer waiting for the device to arm the endpoint.
    In {
        started: u64,
    },
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CtlStage {
    Setup,
    Data,
    Status,
}

/// What one transaction attempt came to.
enum Handshake {
    /// Data moved (IN: the bytes the device sent, with the PID the host
    /// expected).
    Ack(Vec<u8>),
    /// NAK or no answer: retry later.
    Retry,
    Stall,
}

#[derive(Debug)]
pub struct WchUsbfs {
    regs: [u8; REG_BYTES],
    time: Timebase,
    host_enabled: bool,
    /// Enumeration, then (once it has run dry) the descriptor's
    /// `host_steps`.
    script: VecDeque<HostStep>,
    user_steps: VecDeque<HostStep>,
    phase: Phase,
    current: Option<HostStep>,
    /// Address the host sends tokens to (0 until SET_ADDRESS completes).
    host_addr: u8,
    /// Control data stage: bytes moved so far.
    ctl_buf: Vec<u8>,
    /// Earliest cycle the host may start the next transaction.
    next_txn: u64,
    /// Host data toggles per endpoint: the PID the next IN must carry, and
    /// the PID the host's next OUT carries (true = DATA1).
    in_tog: [bool; 8],
    out_tog: [bool; 8],
    /// IN packets the host discarded for carrying the wrong PID, per
    /// endpoint.
    discarded: [u32; 8],
    log: UsbHostLog,
}

impl Default for WchUsbfs {
    fn default() -> Self {
        Self::new()
    }
}

impl WchUsbfs {
    pub fn new() -> Self {
        let mut s = Self {
            regs: [0; REG_BYTES],
            time: Timebase::default(),
            host_enabled: true,
            script: usb_host::enumeration_script(),
            user_steps: VecDeque::new(),
            phase: Phase::Detached,
            current: None,
            host_addr: 0,
            ctl_buf: Vec::new(),
            next_txn: 0,
            in_tog: [false; 8],
            out_tog: [false; 8],
            discarded: [0; 8],
            log: UsbHostLog::default(),
        };
        s.reset_sie();
        s
    }

    /// Disconnect the simulated host (no enumeration will happen).
    pub fn set_host_enabled(&mut self, on: bool) {
        self.host_enabled = on;
    }

    /// Host work to run after enumeration (class requests, report polls).
    pub fn queue_host_steps(&mut self, steps: impl IntoIterator<Item = HostStep>) {
        self.user_steps.extend(steps);
    }

    pub fn host_log(&self) -> &UsbHostLog {
        &self.log
    }

    /// RESET_SIE: every register but BASE_CTRL to its reset value (0).
    fn reset_sie(&mut self) {
        let base = self.regs[BASE_CTRL];
        self.regs = [0; REG_BYTES];
        self.regs[BASE_CTRL] = base;
    }

    fn attached(&self) -> bool {
        let base = self.regs[BASE_CTRL];
        base & UC_DEV_PU_EN != 0
            && base & (UC_HOST_MODE | UC_RESET_SIE) == 0
            && self.regs[UDEV_CTRL] & UD_PORT_EN != 0
    }

    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes([
            self.regs[off],
            self.regs[off + 1],
            self.regs[off + 2],
            self.regs[off + 3],
        ])
    }

    fn dma(&self, ep: u8) -> u64 {
        self.u32_at(UEP_DMA + 4 * ep as usize) as u64
    }

    fn tx_ctrl(&self, ep: u8) -> u8 {
        self.regs[UEP_CTRL + 4 * ep as usize + 2]
    }

    fn rx_ctrl(&self, ep: u8) -> u8 {
        self.regs[UEP_CTRL + 4 * ep as usize + 3]
    }

    fn tx_len(&self, ep: u8) -> usize {
        let o = UEP_CTRL + 4 * ep as usize;
        (u16::from_le_bytes([self.regs[o], self.regs[o + 1]]) & 0x7F) as usize
    }

    /// (TX_EN, RX_EN) of endpoint 1..7 from the UEPn_MOD registers.
    fn ep_enabled(&self, ep: u8) -> (bool, bool) {
        let (reg, shift) = match ep {
            1 => (UEP4_1_MOD, 4),
            4 => (UEP4_1_MOD, 0),
            2 => (UEP4_1_MOD + 1, 0),
            3 => (UEP4_1_MOD + 1, 4),
            5 => (UEP4_1_MOD + 2, 0),
            6 => (UEP4_1_MOD + 2, 4),
            7 => (UEP4_1_MOD + 3, 0),
            _ => return (true, true),
        };
        let m = self.regs[reg] >> shift;
        (m & 0x04 != 0, m & 0x08 != 0)
    }

    fn tx_buffer(&self, ep: u8) -> u64 {
        let (_, rx) = self.ep_enabled(ep);
        self.dma(ep) + if ep != 0 && rx { 64 } else { 0 }
    }

    fn irq(&self) -> bool {
        self.regs[INT_FG] & self.regs[INT_EN] & UIF_MASK != 0
    }

    fn refresh_irq(&self) {
        self.time.set_level(self.irq());
    }

    fn read_reg(&self, off: usize) -> u8 {
        match off {
            UDEV_CTRL => {
                let v = self.regs[UDEV_CTRL] & !(UD_DP_PIN | UD_DM_PIN);
                // Idle full-speed J state: D+ high (held low during reset).
                if self.attached() && !matches!(self.phase, Phase::Resetting { .. }) {
                    v | UD_DP_PIN
                } else {
                    v
                }
            }
            MIS_ST => {
                let mut v = UMS_SIE_FREE;
                if matches!(self.phase, Phase::Resetting { .. }) {
                    v |= UMS_BUS_RESET;
                }
                v
            }
            INT_FG => {
                (self.regs[INT_FG] & UIF_MASK) | (self.regs[INT_ST] & U_TOG_OK) | UMS_SIE_FREE
            }
            OTG_SR..=0x5B => 0,
            o if o < REG_BYTES => self.regs[o],
            _ => 0,
        }
    }

    fn write_reg(&mut self, off: usize, value: u8) {
        if trace_usb() {
            eprintln!(
                "USBFS W {off:#04x}={value:#04x} now={} phase={:?}",
                self.time.now(),
                self.phase
            );
        }
        match off {
            BASE_CTRL => {
                self.regs[BASE_CTRL] = value;
                if value & UC_RESET_SIE != 0 {
                    self.reset_sie();
                }
                if value & UC_CLR_ALL != 0 {
                    self.regs[INT_FG] = 0;
                    self.regs[INT_ST] = 0;
                    self.regs[RX_LEN] = 0;
                    self.regs[RX_LEN + 1] = 0;
                }
            }
            MIS_ST | INT_ST | RX_LEN..=0x0B => {}
            INT_FG => self.regs[INT_FG] &= !(value & UIF_MASK),
            o if o < REG_BYTES => self.regs[o] = value,
            _ => {}
        }
    }

    /// Record a finished transaction: token and endpoint in INT_ST, TRANSFER
    /// raised, the next transaction no earlier than the packet's wire time.
    fn complete(&mut self, token: u8, ep: u8, len: usize, tog_ok: bool) {
        self.regs[INT_ST] = if tog_ok { U_TOG_OK } else { 0 } | token | (ep & 0x0F);
        self.regs[INT_FG] |= UIF_TRANSFER;
        self.next_txn = self.time.now() + self.time.us(packet_us(len) + HOST_GAP_US);
    }

    /// The SIE takes a token only while firmware has cleared TRANSFER, and
    /// only one addressed to it.
    fn sie_ready(&self) -> bool {
        self.regs[INT_FG] & UIF_TRANSFER == 0
            && self.time.now() >= self.next_txn
            && self.regs[DEV_ADDR] & 0x7F == self.host_addr
    }

    fn setup_txn(&mut self, bus: &mut dyn Bus, setup: &Setup) -> bool {
        if !self.sie_ready() {
            return false;
        }
        let b = setup.bytes();
        let at = self.dma(0);
        crate::with_device_write_bypass(bus, |bus| {
            for (i, byte) in b.iter().enumerate() {
                let _ = bus.write_u8(at + i as u64, *byte);
            }
        });
        self.regs[RX_LEN] = 8;
        self.regs[RX_LEN + 1] = 0;
        // SETUP is always DATA0; the data stage then starts at DATA1 in both
        // directions.
        self.complete(UIS_TOKEN_SETUP, 0, 8, true);
        self.in_tog[0] = true;
        self.out_tog[0] = true;
        true
    }

    fn in_txn(&mut self, bus: &mut dyn Bus, ep: u8, max: usize) -> Handshake {
        if !self.sie_ready() {
            return Handshake::Retry;
        }
        if ep != 0 && !self.ep_enabled(ep).0 {
            return Handshake::Retry;
        }
        match self.tx_ctrl(ep) & RES_MASK {
            RES_ACK => {
                let n = self.tx_len(ep).min(max).min(64);
                let at = self.tx_buffer(ep);
                let data = crate::with_device_write_bypass(bus, |bus| {
                    (0..n)
                        .map(|i| bus.read_u8(at + i as u64).unwrap_or(0))
                        .collect::<Vec<u8>>()
                });
                let pid = self.tx_ctrl(ep) & T_TOG != 0;
                self.complete(UIS_TOKEN_IN, ep, n, true);
                let e = ep as usize & 7;
                if pid != self.in_tog[e] {
                    // ACKed, so the device moves on; the data is a repeat
                    // as far as the host knows.
                    self.discarded[e] += 1;
                    return Handshake::Retry;
                }
                self.in_tog[e] = !pid;
                Handshake::Ack(data)
            }
            RES_STALL => Handshake::Stall,
            _ => Handshake::Retry,
        }
    }

    fn out_txn(&mut self, bus: &mut dyn Bus, ep: u8, data: &[u8]) -> Handshake {
        if !self.sie_ready() {
            return Handshake::Retry;
        }
        if ep != 0 && !self.ep_enabled(ep).1 {
            return Handshake::Retry;
        }
        match self.rx_ctrl(ep) & RES_MASK {
            RES_ACK => {
                let at = self.dma(ep);
                crate::with_device_write_bypass(bus, |bus| {
                    for (i, byte) in data.iter().enumerate() {
                        let _ = bus.write_u8(at + i as u64, *byte);
                    }
                });
                let len = (data.len() as u16).to_le_bytes();
                self.regs[RX_LEN] = len[0];
                self.regs[RX_LEN + 1] = len[1];
                let e = ep as usize & 7;
                let pid = self.out_tog[e];
                let tog_ok = pid == (self.rx_ctrl(ep) & R_TOG != 0);
                if !tog_ok {
                    self.log.notes.push(format!(
                        "out {ep}: DATA{} sent, device expected the other PID (TOG_OK clear)",
                        pid as u8
                    ));
                }
                self.complete(UIS_TOKEN_OUT, ep, data.len(), tog_ok);
                self.out_tog[e] = !pid;
                Handshake::Ack(Vec::new())
            }
            RES_STALL => Handshake::Stall,
            _ => Handshake::Retry,
        }
    }

    fn idle_after_step(&mut self) {
        self.phase = Phase::Idle {
            until: self.time.now() + self.time.us(HOST_GAP_US),
        };
    }

    fn end_control(&mut self, setup: Setup, completed: bool, why: &str) {
        if !completed {
            self.log.notes.push(format!(
                "control {:02x} {:02x} {:04x} {why}",
                setup.bm_request_type, setup.b_request, setup.w_value
            ));
            self.log.control.push((setup, self.ctl_buf.clone(), false));
        } else {
            let data = std::mem::take(&mut self.ctl_buf);
            if setup.bm_request_type == 0 && setup.b_request == 5 {
                // The new address applies from the next transaction on.
                self.host_addr = setup.w_value as u8 & 0x7F;
            }
            if setup.bm_request_type == 0 && setup.b_request == 9 {
                // SET_CONFIGURATION: every other endpoint restarts at DATA0.
                self.in_tog[1..].fill(false);
                self.out_tog[1..].fill(false);
            }
            usb_host::after_control(&mut self.log, &mut self.script, setup, data);
        }
        self.idle_after_step();
    }

    fn host_step(&mut self, bus: &mut dyn Bus) {
        let now = self.time.now();
        if !self.attached() {
            if self.phase != Phase::Detached {
                self.phase = Phase::Detached;
                self.host_addr = 0;
            }
            return;
        }
        match self.phase.clone() {
            Phase::Detached => {
                self.phase = Phase::Debounce {
                    until: now + self.time.us(ATTACH_DEBOUNCE_US),
                };
            }
            Phase::Debounce { until } if now >= until => {
                self.log.resets += 1;
                self.host_addr = 0;
                self.in_tog = [false; 8];
                self.out_tog = [false; 8];
                self.regs[INT_FG] |= UIF_BUS_RST;
                self.phase = Phase::Resetting {
                    until: now + self.time.us(BUS_RESET_US),
                };
            }
            Phase::Resetting { until } if now >= until => {
                self.phase = Phase::Idle {
                    until: now + self.time.us(RESET_RECOVERY_US),
                };
            }
            Phase::Idle { until } if now >= until => {
                let next = self
                    .script
                    .pop_front()
                    .or_else(|| self.user_steps.pop_front());
                let Some(step) = next else {
                    self.phase = Phase::Done;
                    return;
                };
                match &step {
                    HostStep::Control(_) => {
                        self.ctl_buf.clear();
                        self.phase = Phase::Ctl {
                            stage: CtlStage::Setup,
                            started: now,
                        };
                    }
                    HostStep::In { .. } => self.phase = Phase::In { started: now },
                    HostStep::Wait { us } => {
                        self.phase = Phase::Idle {
                            until: now + self.time.us(*us),
                        }
                    }
                    HostStep::Out { ep, .. } => {
                        self.log
                            .notes
                            .push(format!("OUT to endpoint {ep} is not modelled; skipped"));
                    }
                }
                self.current = Some(step);
            }
            Phase::Ctl { stage, started } => {
                let Some(HostStep::Control(setup)) = self.current.clone() else {
                    self.idle_after_step();
                    return;
                };
                if now.saturating_sub(started) > self.time.us(HOST_TIMEOUT_US) {
                    let why = format!("timed out in {stage:?}");
                    self.end_control(setup, false, &why);
                    return;
                }
                self.control_stage(bus, setup, stage, started);
            }
            Phase::In { started } => {
                let Some(HostStep::In { ep, max }) = self.current.clone() else {
                    self.idle_after_step();
                    return;
                };
                if now.saturating_sub(started) > self.time.us(HOST_TIMEOUT_US) {
                    self.log
                        .notes
                        .push(format!("in {ep} timed out: the device kept NAKing"));
                    self.idle_after_step();
                    return;
                }
                match self.in_txn(bus, ep, max) {
                    Handshake::Ack(data) => {
                        self.log.ins.push((ep, data));
                        self.idle_after_step();
                    }
                    Handshake::Stall => {
                        self.log.notes.push(format!("in {ep} stalled"));
                        self.idle_after_step();
                    }
                    Handshake::Retry => {}
                }
            }
            _ => {}
        }
    }

    /// The status stage is always DATA1, whichever way it runs.
    fn enter_status(&mut self, started: u64) {
        self.in_tog[0] = true;
        self.out_tog[0] = true;
        self.phase = Phase::Ctl {
            stage: CtlStage::Status,
            started,
        };
    }

    fn control_stage(&mut self, bus: &mut dyn Bus, setup: Setup, stage: CtlStage, started: u64) {
        let next = |stage| Phase::Ctl { stage, started };
        match stage {
            CtlStage::Setup => {
                if self.setup_txn(bus, &setup) {
                    if setup.w_length == 0 {
                        self.enter_status(started);
                    } else {
                        self.phase = next(CtlStage::Data);
                    }
                }
            }
            CtlStage::Data if setup.dir_in() => {
                let want = setup.w_length as usize - self.ctl_buf.len();
                match self.in_txn(bus, 0, want.min(EP0_SIZE)) {
                    Handshake::Ack(chunk) => {
                        let short = chunk.len() < EP0_SIZE;
                        self.ctl_buf.extend(chunk);
                        if short || self.ctl_buf.len() >= setup.w_length as usize {
                            self.enter_status(started);
                        }
                    }
                    Handshake::Stall => self.end_control(setup, false, "stalled"),
                    Handshake::Retry => {}
                }
            }
            CtlStage::Data => {
                let sent = self.ctl_buf.len();
                let end = (sent + EP0_SIZE).min(setup.data.len());
                match self.out_txn(bus, 0, &setup.data[sent..end]) {
                    Handshake::Ack(_) => {
                        self.ctl_buf.extend_from_slice(&setup.data[sent..end]);
                        if end >= setup.data.len() {
                            self.enter_status(started);
                        }
                    }
                    Handshake::Stall => self.end_control(setup, false, "stalled"),
                    Handshake::Retry => {}
                }
            }
            CtlStage::Status => {
                // Opposite direction to the data stage; IN for no-data.
                let r = if setup.dir_in() && setup.w_length > 0 {
                    self.out_txn(bus, 0, &[])
                } else {
                    self.in_txn(bus, 0, EP0_SIZE)
                };
                match r {
                    Handshake::Ack(zlp) => {
                        if !zlp.is_empty() {
                            self.log.notes.push(format!(
                                "control {:02x} {:02x} {:04x}: status stage carried {} bytes",
                                setup.bm_request_type,
                                setup.b_request,
                                setup.w_value,
                                zlp.len()
                            ));
                        }
                        self.end_control(setup, true, "");
                    }
                    Handshake::Stall => self.end_control(setup, false, "stalled in status"),
                    Handshake::Retry => {}
                }
            }
        }
    }

    fn host_busy(&self) -> bool {
        self.host_enabled && self.attached() && self.phase != Phase::Done
    }
}

impl Peripheral for WchUsbfs {
    /// Walked only while the host has work or the interrupt line is up;
    /// MMIO re-arms it.
    fn legacy_tick_active(&self) -> bool {
        self.host_busy() || self.time.level()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let v = self.read_reg(offset as usize);
        self.refresh_irq();
        Ok(v)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_reg(offset as usize, value);
        self.refresh_irq();
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let o = offset as usize;
        let v = u32::from_le_bytes([
            self.read_reg(o),
            self.read_reg(o + 1),
            self.read_reg(o + 2),
            self.read_reg(o + 3),
        ]);
        self.refresh_irq();
        Ok(v)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        for (i, b) in value.to_le_bytes().into_iter().enumerate() {
            self.write_reg(offset as usize + i, b);
        }
        self.refresh_irq();
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        Some(byte_of(self.read_u32(offset & !3).unwrap_or(0), offset))
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.time.advance(cycles);
        self.refresh_irq();
        let now = self.time.now();
        // While the host has work the flags move from the bus tick; re-check
        // every microsecond.
        let until = self.host_busy().then(|| now + self.time.us(1));
        wake_hint(now, until)
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.time.level())
    }
    fn needs_bus_tick(&self) -> bool {
        self.host_busy()
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if self.host_enabled {
            self.host_step(bus);
        }
        self.refresh_irq();
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.time.attach_clock(clock);
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.time.attach_cpu_hz(hz);
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        let mut lines = self.log.lines();
        for (ep, n) in self.discarded.iter().enumerate().filter(|(_, n)| **n > 0) {
            lines.push(format!("discarded {ep} {n} (wrong DATA PID)"));
        }
        vec![crate::peripheral_log::PeripheralLog::new("host", lines)]
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "wch_usbfs",
            "base_ctrl": self.regs[BASE_CTRL],
            "dev_addr": self.regs[DEV_ADDR],
            "int_fg": self.regs[INT_FG],
            "host_addr": self.host_addr,
            "controls": self.log.control.len(),
            "phase": format!("{:?}", self.phase),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_fg_flags_are_write_one_to_clear_and_status_bits_read_only() {
        let mut u = WchUsbfs::new();
        u.regs[INT_FG] = UIF_TRANSFER | UIF_BUS_RST;
        u.write(INT_FG as u64, UIF_TRANSFER | 0xE0).unwrap();
        assert_eq!(u.regs[INT_FG], UIF_BUS_RST);
        assert_eq!(u.read(INT_FG as u64).unwrap(), UIF_BUS_RST | UMS_SIE_FREE);
        // TOG_OK mirrors the last transfer's INT_ST.
        u.complete(UIS_TOKEN_OUT, 0, 0, true);
        assert_eq!(u.read(INT_FG as u64).unwrap() & U_TOG_OK, U_TOG_OK);
        u.complete(UIS_TOKEN_OUT, 0, 0, false);
        assert_eq!(u.read(INT_FG as u64).unwrap() & U_TOG_OK, 0);
    }

    #[test]
    fn reset_sie_clears_registers_but_keeps_base_ctrl() {
        let mut u = WchUsbfs::new();
        u.write(DEV_ADDR as u64, 5).unwrap();
        u.write_u32(UEP_DMA as u64, 0x2000_0104).unwrap();
        u.write(BASE_CTRL as u64, UC_RESET_SIE | UC_CLR_ALL)
            .unwrap();
        assert_eq!(u.read(DEV_ADDR as u64).unwrap(), 0);
        assert_eq!(u.read_u32(UEP_DMA as u64).unwrap(), 0);
        assert_eq!(u.read(BASE_CTRL as u64).unwrap(), UC_RESET_SIE | UC_CLR_ALL);
        assert!(!u.attached());
    }

    #[test]
    fn interrupt_line_follows_enabled_flags() {
        let mut u = WchUsbfs::new();
        u.regs[INT_FG] = UIF_TRANSFER;
        u.refresh_irq();
        assert_eq!(u.irq_line_level(), Some(false));
        u.write(INT_EN as u64, UIF_TRANSFER).unwrap();
        assert_eq!(u.irq_line_level(), Some(true));
        u.write(INT_FG as u64, UIF_TRANSFER).unwrap();
        assert_eq!(u.irq_line_level(), Some(false));
    }

    #[test]
    fn endpoint_mode_bits_and_in_buffer_offset() {
        let mut u = WchUsbfs::new();
        // ch32fun USBOTG_InternalFinishSetup: EP1/EP4 TX, EP2/EP3 TX.
        u.write(UEP4_1_MOD as u64, 0x40 | 0x04).unwrap();
        u.write(UEP4_1_MOD as u64 + 1, 0x04 | 0x40).unwrap();
        assert_eq!(u.ep_enabled(1), (true, false));
        assert_eq!(u.ep_enabled(2), (true, false));
        assert_eq!(u.ep_enabled(3), (true, false));
        assert_eq!(u.ep_enabled(5), (false, false));
        u.write_u32(UEP_DMA as u64 + 4, 0x2000_0144).unwrap();
        assert_eq!(u.tx_buffer(1), 0x2000_0144);
        // With RX enabled too, IN data follows the 64-byte OUT buffer.
        u.write(UEP4_1_MOD as u64, 0xC0).unwrap();
        assert_eq!(u.tx_buffer(1), 0x2000_0184);
    }
}
