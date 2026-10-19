// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! i.MX RT1060 USB OTG controller in device mode (USB1 `0x402E_0000`,
//! IMXRT1060RM §42 "Universal Serial Bus Controller", a ChipIdea/EHCI-style
//! core) plus its UTMI PHY (USBPHY1 `0x400D_9000`, §43) and a scripted USB
//! host on the other end of the cable.
//!
//! Device-mode data structures live in RAM, exactly as on silicon:
//! `ENDPTLISTADDR` points at the endpoint queue heads (dQH, 64 bytes each,
//! OUT then IN per endpoint) and each dQH chains transfer descriptors (dTD,
//! 32 bytes). Firmware primes an endpoint through `ENDPTPRIME`; the
//! controller copies the dQH's next-dTD into its overlay, reports the
//! endpoint in `ENDPTSTAT`, moves data between the host and the dTD buffers,
//! retires the dTD (status and remaining byte count written back), and
//! reports completion in `ENDPTCOMPLETE` + `USBSTS.UI`. SETUP packets land in
//! the dQH setup buffer and flag `ENDPTSETUPSTAT`.
//!
//! The host enumerates the device the way an OS does (bus reset,
//! GET_DESCRIPTOR device, SET_ADDRESS, configuration and string descriptors,
//! SET_CONFIGURATION) and then runs optional class requests. Everything it
//! learned is kept in [`UsbHostLog`]; nothing about the device is assumed.

use super::{byte_of, Timebase};
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::collections::VecDeque;

const ID: u32 = 0x000;
const CAPLENGTH: u32 = 0x100;
const USBCMD: u32 = 0x140;
const USBSTS: u32 = 0x144;
const USBINTR: u32 = 0x148;
const FRINDEX: u32 = 0x14C;
const ENDPTLISTADDR: u32 = 0x158;
const PORTSC1: u32 = 0x184;
const OTGSC: u32 = 0x1A4;
const USBMODE: u32 = 0x1A8;
const ENDPTSETUPSTAT: u32 = 0x1AC;
const ENDPTPRIME: u32 = 0x1B0;
const ENDPTFLUSH: u32 = 0x1B4;
const ENDPTSTAT: u32 = 0x1B8;
const ENDPTCOMPLETE: u32 = 0x1BC;
const ENDPTCTRL0: u32 = 0x1C0;

const CMD_RS: u32 = 1 << 0;
const CMD_RST: u32 = 1 << 1;
const STS_UI: u32 = 1 << 0;
const STS_PCI: u32 = 1 << 2;
const STS_SRI: u32 = 1 << 7;
const STS_URI: u32 = 1 << 6;
/// Write-1-to-clear USBSTS bits.
const STS_W1C: u32 = 0x0300_01FF;

const DTD_ACTIVE: u32 = 0x80;
const DTD_T: u32 = 1;

/// Controller reset (USBCMD.RST) duration.
const RESET_US: u64 = 10;
/// Host-side delay before the first bus reset after the device attaches
/// (debounce, USB 2.0 §7.1.7.3: >= 100 ms; kept short to save sim time).
const ATTACH_DEBOUNCE_US: u64 = 2_000;
/// Bus reset length (USB 2.0: >= 10 ms; shortened).
const BUS_RESET_US: u64 = 1_000;
/// Gap the host leaves between two transactions.
const HOST_GAP_US: u64 = 20;
/// A control transfer the device does not answer within this is abandoned.
const HOST_TIMEOUT_US: u64 = 500_000;
/// Microframe (high speed SOF) period.
const MICROFRAME_US: u64 = 125;

fn trace_usb() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LABWIRED_TRACE_USB").is_some())
}

/// One standard/class control request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
    /// Host -> device data stage (OUT); empty for IN or no-data requests.
    pub data: Vec<u8>,
}

impl Setup {
    pub fn get_descriptor(kind: u8, index: u8, lang: u16, len: u16) -> Self {
        Self {
            bm_request_type: 0x80,
            b_request: 6,
            w_value: ((kind as u16) << 8) | index as u16,
            w_index: lang,
            w_length: len,
            data: Vec::new(),
        }
    }
    fn bytes(&self) -> [u8; 8] {
        let v = self.w_value.to_le_bytes();
        let i = self.w_index.to_le_bytes();
        let l = self.w_length.to_le_bytes();
        [
            self.bm_request_type,
            self.b_request,
            v[0],
            v[1],
            i[0],
            i[1],
            l[0],
            l[1],
        ]
    }
    fn dir_in(&self) -> bool {
        self.bm_request_type & 0x80 != 0
    }
}

/// A step of the host script.
#[derive(Debug, Clone)]
pub enum HostStep {
    Control(Setup),
    /// Interrupt/bulk OUT data to endpoint `ep` (1..7).
    Out {
        ep: u8,
        data: Vec<u8>,
    },
    /// Read up to `max` bytes from IN endpoint `ep` (one transfer).
    In {
        ep: u8,
        max: usize,
    },
}

/// What the host observed.
#[derive(Debug, Default, Clone)]
pub struct UsbHostLog {
    /// Bus resets the host issued.
    pub resets: u32,
    /// (setup, data returned / sent, completed)
    pub control: Vec<(Setup, Vec<u8>, bool)>,
    /// (endpoint, data) of completed IN transfers on non-control endpoints.
    pub ins: Vec<(u8, Vec<u8>)>,
    /// (endpoint, bytes accepted) of completed OUT transfers.
    pub outs: Vec<(u8, usize)>,
    /// Address assigned with SET_ADDRESS.
    pub address: Option<u8>,
    pub device_descriptor: Vec<u8>,
    pub config_descriptor: Vec<u8>,
    /// String descriptors decoded to text, by index.
    pub strings: Vec<(u8, String)>,
    /// Human-readable progress / failure notes.
    pub notes: Vec<String>,
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl UsbHostLog {
    /// The log as text lines, for the `host` peripheral log. Facts first
    /// (device IDs, address, interfaces, strings), then every transfer in
    /// order, then the notes:
    ///
    /// ```text
    /// device 1fc9:0135 class 00/00/00 usb 0200 ep0 64
    /// address 5
    /// interface 0 alt 0 class 03/00/00 endpoints 2
    /// endpoint 0x81 attr 0x03 max 64
    /// string 2 "Product"
    /// control 80 06 0100 0000 0012 ok 12 01 00 02 ...
    /// in 1 01 02
    /// out 2 64
    /// note ...
    /// ```
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.resets > 0 {
            lines.push(format!("resets {}", self.resets));
        }
        let dd = &self.device_descriptor;
        if dd.len() >= 18 {
            lines.push(format!(
                "device {:04x}:{:04x} class {:02x}/{:02x}/{:02x} usb {:04x} ep0 {}",
                u16::from_le_bytes([dd[8], dd[9]]),
                u16::from_le_bytes([dd[10], dd[11]]),
                dd[4],
                dd[5],
                dd[6],
                u16::from_le_bytes([dd[2], dd[3]]),
                dd[7]
            ));
        }
        if let Some(address) = self.address {
            lines.push(format!("address {address}"));
        }
        let cfg = &self.config_descriptor;
        let mut i = 0;
        while i + 1 < cfg.len() && cfg[i] >= 2 {
            let d = &cfg[i..(i + cfg[i] as usize).min(cfg.len())];
            match d[1] {
                4 if d.len() >= 9 => lines.push(format!(
                    "interface {} alt {} class {:02x}/{:02x}/{:02x} endpoints {}",
                    d[2], d[3], d[5], d[6], d[7], d[4]
                )),
                5 if d.len() >= 7 => lines.push(format!(
                    "endpoint {:#04x} attr {:#04x} max {}",
                    d[2],
                    d[3],
                    u16::from_le_bytes([d[4], d[5]])
                )),
                _ => {}
            }
            i += cfg[i] as usize;
        }
        for (index, text) in &self.strings {
            lines.push(format!("string {index} {text:?}"));
        }
        for (setup, data, completed) in &self.control {
            let status = if *completed { "ok" } else { "incomplete" };
            let mut line = format!(
                "control {:02x} {:02x} {:04x} {:04x} {:04x} {status}",
                setup.bm_request_type,
                setup.b_request,
                setup.w_value,
                setup.w_index,
                setup.w_length
            );
            if !data.is_empty() {
                line.push(' ');
                line.push_str(&hex_bytes(data));
            }
            lines.push(line);
        }
        for (ep, data) in &self.ins {
            lines.push(format!("in {ep} {}", hex_bytes(data)));
        }
        for (ep, n) in &self.outs {
            lines.push(format!("out {ep} {n}"));
        }
        for note in &self.notes {
            lines.push(format!("note {note}"));
        }
        lines
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// Waiting for the device to connect (RS=1 in device mode, VBUS).
    Detached,
    Debounce {
        until: u64,
    },
    Resetting {
        until: u64,
    },
    /// Between transactions.
    Idle {
        until: u64,
    },
    /// Control transfer in progress.
    Ctl {
        stage: CtlStage,
        started: u64,
    },
    /// Non-control transfer waiting for the device to prime.
    Ep {
        started: u64,
    },
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CtlStage {
    /// SETUP delivered; waiting for the device to take it and prime data.
    Data,
    Status,
}

#[derive(Debug)]
pub struct ImxrtUsb {
    regs: Vec<u32>,
    time: Timebase,
    reset_until: u64,
    /// Primed endpoint bits (ENDPTSTAT layout: OUT 0..7, IN 16..23).
    stat: u32,
    complete: u32,
    setupstat: u32,
    prime: u32,
    usbsts: u32,
    vbus: bool,
    /// Pending host script and state.
    script: VecDeque<HostStep>,
    phase: Phase,
    /// Data gathered in the current control data stage.
    ctl_buf: Vec<u8>,
    current: Option<HostStep>,
    log: UsbHostLog,
    /// Last microframe index published through FRINDEX / SOF.
    last_uframe: u64,
    run_since: u64,
    host_enabled: bool,
}

impl Default for ImxrtUsb {
    fn default() -> Self {
        Self::new()
    }
}

/// SVD reset values of the plain registers (offset, value).
const RESETS: &[(u32, u32)] = &[
    (0x000, 0xE4A1_FA05), // ID
    (0x004, 0x0000_0035), // HWGENERAL
    (0x008, 0x1002_0001), // HWHOST
    (0x00C, 0x0000_0011), // HWDEVICE
    (0x010, 0x8008_0B08), // HWTXBUF
    (0x014, 0x0000_0808), // HWRXBUF
    (0x090, 0x0000_0002), // SBUSCFG
    (0x100, 0x0100_0040), // CAPLENGTH | HCIVERSION << 16
    (0x104, 0x0001_0011), // HCSPARAMS
    (0x108, 0x0000_0006), // HCCPARAMS
    (0x120, 0x0000_0001), // DCIVERSION
    (0x124, 0x0000_0188), // DCCPARAMS: 8 endpoints, DC, HC
    (0x140, 0x0008_0000), // USBCMD
    (0x160, 0x0000_0808), // BURSTSIZE
    (0x180, 0x0000_0001), // CONFIGFLAG
    (0x184, 0x1000_0000), // PORTSC1
    (0x1A4, 0x0000_1120), // OTGSC
    (0x1A8, 0x0000_5000), // USBMODE
    (0x1C0, 0x0080_0080), // ENDPTCTRL0
];

impl ImxrtUsb {
    pub fn new() -> Self {
        let mut s = Self {
            regs: vec![0; 0x200 / 4],
            time: Timebase::default(),
            reset_until: 0,
            stat: 0,
            complete: 0,
            setupstat: 0,
            prime: 0,
            usbsts: 0,
            vbus: true,
            script: VecDeque::new(),
            phase: Phase::Detached,
            ctl_buf: Vec::new(),
            current: None,
            log: UsbHostLog::default(),
            last_uframe: 0,
            run_since: 0,
            host_enabled: true,
        };
        s.reset_regs();
        s.script = Self::enumeration_script();
        s
    }

    fn reset_regs(&mut self) {
        self.regs.iter_mut().for_each(|r| *r = 0);
        for &(o, v) in RESETS {
            self.regs[(o / 4) as usize] = v;
        }
        self.stat = 0;
        self.complete = 0;
        self.setupstat = 0;
        self.prime = 0;
        self.usbsts = 0;
    }

    /// The enumeration an OS performs. Configuration-dependent requests
    /// (full config length, strings) are appended as answers arrive.
    fn enumeration_script() -> VecDeque<HostStep> {
        VecDeque::from(vec![
            HostStep::Control(Setup::get_descriptor(1, 0, 0, 64)),
            HostStep::Control(Setup {
                bm_request_type: 0x00,
                b_request: 5, // SET_ADDRESS
                w_value: 5,
                w_index: 0,
                w_length: 0,
                data: Vec::new(),
            }),
            HostStep::Control(Setup::get_descriptor(1, 0, 0, 18)),
            HostStep::Control(Setup::get_descriptor(2, 0, 0, 9)),
        ])
    }

    /// Queue extra host work (class requests, HID reports) after enumeration.
    pub fn queue_host_steps(&mut self, steps: impl IntoIterator<Item = HostStep>) {
        self.script.extend(steps);
        if self.phase == Phase::Done {
            self.phase = Phase::Idle {
                until: self.time.now(),
            };
        }
    }

    /// Disconnect the simulated host (no enumeration will happen).
    pub fn set_host_enabled(&mut self, on: bool) {
        self.host_enabled = on;
    }

    pub fn host_log(&self) -> &UsbHostLog {
        &self.log
    }

    fn reg(&self, off: u32) -> u32 {
        self.regs.get((off / 4) as usize).copied().unwrap_or(0)
    }

    fn device_mode(&self) -> bool {
        self.reg(USBMODE) & 3 == 2
    }

    fn running(&self) -> bool {
        self.reg(USBCMD) & CMD_RS != 0 && self.device_mode()
    }

    /// Time-derived status: SOF / microframe counter and the GP timers are
    /// not modelled beyond FRINDEX + SRI.
    fn frindex(&self) -> u32 {
        if !self.running() || matches!(self.phase, Phase::Detached | Phase::Debounce { .. }) {
            return self.reg(FRINDEX);
        }
        let us = self.time.now().saturating_sub(self.run_since) / self.time.us(1).max(1);
        ((us / MICROFRAME_US) & 0x3FFF) as u32
    }

    fn usbsts_view(&self) -> u32 {
        let mut v = self.usbsts;
        if self.time.now() >= self.reset_until {
            // HCH is a host-mode bit; in device mode it reads 0.
        }
        if self.running() && self.frindex() != (self.last_uframe as u32 & 0x3FFF) {
            v |= STS_SRI;
        }
        v
    }

    pub fn read_reg(&self, off: u32) -> u32 {
        let now = self.time.now();
        match off & !3 {
            USBCMD => {
                let v = self.reg(USBCMD);
                if now < self.reset_until {
                    v | CMD_RST
                } else {
                    v & !CMD_RST
                }
            }
            USBSTS => self.usbsts_view(),
            FRINDEX => self.frindex(),
            PORTSC1 => {
                let mut v = self.reg(PORTSC1) & !((3 << 26) | 1 | (1 << 8) | (1 << 2));
                match self.phase {
                    Phase::Detached | Phase::Debounce { .. } => {}
                    Phase::Resetting { .. } => v |= 1 | (1 << 8),
                    _ => v |= 1 | (1 << 2) | (2 << 26), // CCS, PE, high speed
                }
                v
            }
            OTGSC => {
                let mut v = self.reg(OTGSC) & !((1 << 9) | (1 << 10) | (1 << 11) | (1 << 12));
                if self.vbus {
                    v |= (1 << 9) | (1 << 10) | (1 << 11); // AVV ASV BSV
                } else {
                    v |= 1 << 12; // BSE
                }
                v | (1 << 8) // ID = B device
            }
            ENDPTSETUPSTAT => self.setupstat,
            ENDPTPRIME => self.prime,
            ENDPTFLUSH => 0,
            ENDPTSTAT => self.stat,
            ENDPTCOMPLETE => self.complete,
            ID | CAPLENGTH => self.reg(off & !3),
            o => self.reg(o),
        }
    }

    pub fn write_reg(&mut self, off: u32, value: u32, mask: u32) {
        let now = self.time.now();
        if trace_usb() {
            eprintln!("USB W {off:#x}={value:#x} now={now} phase={:?}", self.phase);
        }
        let v = value & mask;
        let old = self.reg(off & !3);
        let merged = (old & !mask) | v;
        match off & !3 {
            USBCMD => {
                if v & CMD_RST != 0 {
                    // Controller reset: registers to defaults; the host sees
                    // the device disconnect.
                    self.reset_regs();
                    self.reset_until = now + self.time.us(RESET_US);
                    self.phase = Phase::Detached;
                    return;
                }
                let was_running = self.running();
                self.regs[(USBCMD / 4) as usize] = merged & !CMD_RST;
                if !was_running && self.running() {
                    self.run_since = now;
                }
            }
            USBSTS => {
                self.usbsts &= !(v & STS_W1C);
                if v & STS_SRI != 0 {
                    self.last_uframe = self.frindex() as u64;
                }
            }
            ENDPTSETUPSTAT => self.setupstat &= !v,
            ENDPTCOMPLETE => self.complete &= !v,
            ENDPTPRIME => self.prime |= v & 0x00FF_00FF,
            ENDPTFLUSH => {
                self.stat &= !(v & 0x00FF_00FF);
                self.prime &= !(v & 0x00FF_00FF);
            }
            ENDPTSTAT | CAPLENGTH => {}
            o if o < 0x080 || (0x100..0x140).contains(&o) => {}
            OTGSC => {
                // Interrupt status bits 16..22 are w1c; the rest writable.
                let keep = (old & !0x007F_0000 & !mask) | (v & !0x007F_0000);
                let status = old & 0x007F_0000 & !(v & 0x007F_0000);
                self.regs[(OTGSC / 4) as usize] = keep | status;
            }
            o => self.regs[(o / 4) as usize] = merged,
        }
    }

    fn irq(&self) -> bool {
        let sts = self.usbsts_view();
        sts & self.reg(USBINTR) & 0x030D_05FF != 0
    }

    // ---- data structure access -------------------------------------------

    fn dqh(&self, ep: u8, dir_in: bool) -> u64 {
        (self.reg(ENDPTLISTADDR) & !0x7FF) as u64 + (ep as u64) * 128 + if dir_in { 64 } else { 0 }
    }

    fn bit(ep: u8, dir_in: bool) -> u32 {
        1 << (ep as u32 + if dir_in { 16 } else { 0 })
    }

    /// Latch primes: copy dQH.next into the overlay and mark the endpoint.
    fn service_primes(&mut self, bus: &mut dyn Bus) {
        if self.prime == 0 {
            return;
        }
        for bitpos in 0..32 {
            if self.prime & (1 << bitpos) == 0 {
                continue;
            }
            let (ep, dir_in) = if bitpos >= 16 {
                ((bitpos - 16) as u8, true)
            } else {
                (bitpos as u8, false)
            };
            let q = self.dqh(ep, dir_in);
            let next =
                crate::with_device_write_bypass(bus, |bus| bus.read_u32(q + 8)).unwrap_or(DTD_T);
            if next & DTD_T == 0 {
                self.stat |= 1 << bitpos;
                let _ =
                    crate::with_device_write_bypass(bus, |bus| bus.write_u32(q + 4, next & !0x1F));
            }
            self.prime &= !(1 << bitpos);
        }
    }

    /// The dTD currently at the head of a primed endpoint (overlay).
    fn head_dtd(&self, bus: &mut dyn Bus, ep: u8, dir_in: bool) -> Option<u64> {
        if self.stat & Self::bit(ep, dir_in) == 0 {
            return None;
        }
        let q = self.dqh(ep, dir_in);
        let cur = crate::with_device_write_bypass(bus, |bus| bus.read_u32(q + 4)).ok()? & !0x1F;
        if cur == 0 {
            return None;
        }
        let token =
            crate::with_device_write_bypass(bus, |bus| bus.read_u32(cur as u64 + 4)).ok()?;
        (token & DTD_ACTIVE != 0).then_some(cur as u64)
    }

    /// Complete the head dTD after `moved` bytes; advance the endpoint.
    fn retire(&mut self, bus: &mut dyn Bus, ep: u8, dir_in: bool, dtd: u64, remaining: u32) {
        let token = crate::with_device_write_bypass(bus, |bus| bus.read_u32(dtd + 4)).unwrap_or(0);
        let new_token = (token & !(0x7FFF << 16) & !0xFF) | ((remaining & 0x7FFF) << 16);
        let _ = crate::with_device_write_bypass(bus, |bus| bus.write_u32(dtd + 4, new_token));
        let q = self.dqh(ep, dir_in);
        let _ = crate::with_device_write_bypass(bus, |bus| bus.write_u32(q + 0xC, new_token));
        if token & (1 << 15) != 0 {
            self.complete |= Self::bit(ep, dir_in);
            self.usbsts |= STS_UI;
        }
        let next = crate::with_device_write_bypass(bus, |bus| bus.read_u32(dtd)).unwrap_or(DTD_T);
        if next & DTD_T == 0 {
            let _ = crate::with_device_write_bypass(bus, |bus| bus.write_u32(q + 4, next & !0x1F));
            let _ = crate::with_device_write_bypass(bus, |bus| bus.write_u32(q + 8, next));
        } else {
            let _ = crate::with_device_write_bypass(bus, |bus| bus.write_u32(q + 8, DTD_T));
            self.stat &= !Self::bit(ep, dir_in);
        }
    }

    /// Device -> host: read the head dTD's data (at most `max`).
    fn take_in(&mut self, bus: &mut dyn Bus, ep: u8, max: usize) -> Option<Vec<u8>> {
        let dtd = self.head_dtd(bus, ep, true)?;
        let token = crate::with_device_write_bypass(bus, |bus| bus.read_u32(dtd + 4)).ok()?;
        let total = ((token >> 16) & 0x7FFF) as usize;
        let n = total.min(max);
        let data = read_dtd_buffer(bus, dtd, n);
        self.retire(bus, ep, true, dtd, (total - n) as u32);
        Some(data)
    }

    /// Host -> device: write `data` into the head dTD.
    fn give_out(&mut self, bus: &mut dyn Bus, ep: u8, data: &[u8]) -> Option<usize> {
        let dtd = self.head_dtd(bus, ep, false)?;
        let token = crate::with_device_write_bypass(bus, |bus| bus.read_u32(dtd + 4)).ok()?;
        let total = ((token >> 16) & 0x7FFF) as usize;
        let n = total.min(data.len());
        write_dtd_buffer(bus, dtd, &data[..n]);
        self.retire(bus, ep, false, dtd, (total - n) as u32);
        Some(n)
    }

    fn deliver_setup(&mut self, bus: &mut dyn Bus, setup: &Setup) {
        if trace_usb() {
            eprintln!(
                "USB host SETUP {:02x?} now={}",
                setup.bytes(),
                self.time.now()
            );
        }
        let q = self.dqh(0, false);
        let b = setup.bytes();
        let _ = crate::with_device_write_bypass(bus, |bus| {
            bus.write_u32(q + 0x28, u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        });
        let _ = crate::with_device_write_bypass(bus, |bus| {
            bus.write_u32(q + 0x2C, u32::from_le_bytes([b[4], b[5], b[6], b[7]]))
        });
        // A SETUP flushes whatever EP0 had primed (RM §42.5.6.4).
        self.stat &= !(Self::bit(0, false) | Self::bit(0, true));
        self.setupstat |= 1;
        self.usbsts |= STS_UI;
    }

    /// Advance the host one step.
    fn host_step(&mut self, bus: &mut dyn Bus) {
        let now = self.time.now();
        let attached = self.running() && self.vbus && now >= self.reset_until;
        if !attached {
            if !matches!(self.phase, Phase::Detached) {
                self.phase = Phase::Detached;
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
                self.usbsts |= STS_URI;
                self.stat = 0;
                self.setupstat = 0;
                self.phase = Phase::Resetting {
                    until: now + self.time.us(BUS_RESET_US),
                };
            }
            Phase::Resetting { until } if now >= until => {
                // End of reset: port enabled at high speed.
                self.usbsts |= STS_PCI;
                self.phase = Phase::Idle {
                    until: now + self.time.us(HOST_GAP_US),
                };
            }
            Phase::Idle { until } if now >= until => {
                // SETUP stays owned by the host until the device consumed it.
                let Some(step) = self.script.pop_front() else {
                    self.phase = Phase::Done;
                    return;
                };
                match &step {
                    HostStep::Control(setup) => {
                        let setup = setup.clone();
                        self.deliver_setup(bus, &setup);
                        self.ctl_buf.clear();
                        self.phase = Phase::Ctl {
                            stage: CtlStage::Data,
                            started: now,
                        };
                    }
                    _ => {
                        self.phase = Phase::Ep { started: now };
                    }
                }
                self.current = Some(step);
            }
            Phase::Ctl { stage, started } => {
                let Some(HostStep::Control(setup)) = self.current.clone() else {
                    self.phase = Phase::Idle { until: now };
                    return;
                };
                if now.saturating_sub(started) > self.time.us(HOST_TIMEOUT_US) {
                    self.log.notes.push(format!(
                        "control {:02x} {:02x} {:04x} timed out in {:?}",
                        setup.bm_request_type, setup.b_request, setup.w_value, stage
                    ));
                    self.log.control.push((setup, self.ctl_buf.clone(), false));
                    self.phase = Phase::Idle {
                        until: now + self.time.us(HOST_GAP_US),
                    };
                    return;
                }
                if self.setupstat & 1 != 0 {
                    return; // device has not taken the SETUP yet
                }
                // A protocol STALL on EP0 (ENDPTCTRL0.TXS/RXS) ends the
                // request; the next SETUP clears it (RM: control endpoint
                // stall is cleared by hardware on SETUP).
                if self.reg(ENDPTCTRL0) & ((1 << 16) | 1) != 0 {
                    self.regs[(ENDPTCTRL0 / 4) as usize] &= !((1 << 16) | 1);
                    self.log.notes.push(format!(
                        "control {:02x} {:02x} {:04x} stalled",
                        setup.bm_request_type, setup.b_request, setup.w_value
                    ));
                    self.log.control.push((setup, self.ctl_buf.clone(), false));
                    self.phase = Phase::Idle {
                        until: now + self.time.us(HOST_GAP_US),
                    };
                    return;
                }
                match stage {
                    CtlStage::Data => {
                        if setup.w_length == 0 {
                            self.phase = Phase::Ctl {
                                stage: CtlStage::Status,
                                started,
                            };
                        } else if setup.dir_in() {
                            let want = setup.w_length as usize - self.ctl_buf.len();
                            if let Some(chunk) = self.take_in(bus, 0, want) {
                                let short = chunk.len() % 64 != 0 || chunk.is_empty();
                                self.ctl_buf.extend(chunk);
                                if short || self.ctl_buf.len() >= setup.w_length as usize {
                                    self.phase = Phase::Ctl {
                                        stage: CtlStage::Status,
                                        started,
                                    };
                                }
                            }
                        } else if let Some(n) = self.give_out(bus, 0, &setup.data) {
                            self.ctl_buf.extend_from_slice(&setup.data[..n]);
                            self.phase = Phase::Ctl {
                                stage: CtlStage::Status,
                                started,
                            };
                        }
                    }
                    CtlStage::Status => {
                        // Status stage runs opposite to the data stage (IN
                        // for no-data requests).
                        let done = if setup.dir_in() && setup.w_length > 0 {
                            self.give_out(bus, 0, &[]).is_some()
                        } else {
                            self.take_in(bus, 0, 0).is_some()
                        };
                        if done {
                            self.finish_control(setup);
                            self.phase = Phase::Idle {
                                until: now + self.time.us(HOST_GAP_US),
                            };
                        }
                    }
                }
            }
            Phase::Ep { started } => {
                let step = self.current.clone();
                if now.saturating_sub(started) > self.time.us(HOST_TIMEOUT_US) {
                    self.log
                        .notes
                        .push(format!("{step:?} timed out: endpoint never primed"));
                    self.phase = Phase::Idle {
                        until: now + self.time.us(HOST_GAP_US),
                    };
                    return;
                }
                let done = match &step {
                    Some(HostStep::Out { ep, data }) => {
                        let r = self.give_out(bus, *ep, data);
                        if let Some(n) = r {
                            self.log.outs.push((*ep, n));
                        }
                        r.is_some()
                    }
                    Some(HostStep::In { ep, max }) => {
                        let r = self.take_in(bus, *ep, *max);
                        if let Some(d) = &r {
                            self.log.ins.push((*ep, d.clone()));
                        }
                        r.is_some()
                    }
                    _ => true,
                };
                if done {
                    self.phase = Phase::Idle {
                        until: now + self.time.us(HOST_GAP_US),
                    };
                }
            }
            _ => {}
        }
    }

    /// Record a completed control transfer and schedule what it implies.
    fn finish_control(&mut self, setup: Setup) {
        let data = self.ctl_buf.clone();
        if setup.b_request == 5 && setup.bm_request_type == 0 {
            self.log.address = Some(setup.w_value as u8);
        }
        if setup.b_request == 6 && setup.bm_request_type == 0x80 {
            let kind = (setup.w_value >> 8) as u8;
            let index = setup.w_value as u8;
            match kind {
                1 => {
                    self.log.device_descriptor = data.clone();
                    if setup.w_length == 18 && data.len() >= 18 {
                        // Strings the device announced, then nothing else yet.
                        let mut strings: Vec<u8> = [data[14], data[15], data[16]]
                            .into_iter()
                            .filter(|&i| i != 0)
                            .collect();
                        strings.dedup();
                        if !strings.is_empty() {
                            self.script
                                .push_back(HostStep::Control(Setup::get_descriptor(3, 0, 0, 255)));
                            for i in strings {
                                self.script
                                    .push_back(HostStep::Control(Setup::get_descriptor(
                                        3, i, 0x0409, 255,
                                    )));
                            }
                        }
                    }
                }
                2 => {
                    if setup.w_length == 9 && data.len() >= 4 {
                        let total = u16::from_le_bytes([data[2], data[3]]);
                        self.script
                            .push_front(HostStep::Control(Setup::get_descriptor(2, 0, 0, total)));
                    } else {
                        self.log.config_descriptor = data.clone();
                        let cfg = data.get(5).copied().unwrap_or(1);
                        self.script.push_back(HostStep::Control(Setup {
                            bm_request_type: 0,
                            b_request: 9, // SET_CONFIGURATION
                            w_value: cfg as u16,
                            w_index: 0,
                            w_length: 0,
                            data: Vec::new(),
                        }));
                    }
                }
                3 if index != 0 && data.len() >= 2 => {
                    let units: Vec<u16> = data[2..]
                        .chunks(2)
                        .filter(|c| c.len() == 2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    self.log
                        .strings
                        .push((index, String::from_utf16_lossy(&units)));
                }
                _ => {}
            }
        }
        self.log.control.push((setup, data, true));
    }
}

fn dtd_page(bus: &mut dyn Bus, dtd: u64, k: usize) -> u64 {
    crate::with_device_write_bypass(bus, |bus| bus.read_u32(dtd + 8 + 4 * k as u64)).unwrap_or(0)
        as u64
}

/// Byte `i` of a dTD's buffer (page 0 carries the start offset).
fn dtd_addr(bus: &mut dyn Bus, dtd: u64, i: usize) -> u64 {
    let p0 = dtd_page(bus, dtd, 0);
    let off = (p0 & 0xFFF) as usize + i;
    let page = off / 4096;
    let base = if page == 0 {
        p0 & !0xFFF
    } else {
        dtd_page(bus, dtd, page.min(4)) & !0xFFF
    };
    base + (off % 4096) as u64
}

fn read_dtd_buffer(bus: &mut dyn Bus, dtd: u64, n: usize) -> Vec<u8> {
    crate::with_device_write_bypass(bus, |bus| {
        (0..n)
            .map(|i| {
                let a = dtd_addr(bus, dtd, i);
                bus.read_u8(a).unwrap_or(0)
            })
            .collect()
    })
}

fn write_dtd_buffer(bus: &mut dyn Bus, dtd: u64, data: &[u8]) {
    crate::with_device_write_bypass(bus, |bus| {
        for (i, b) in data.iter().enumerate() {
            let a = dtd_addr(bus, dtd, i);
            let _ = bus.write_u8(a, *b);
        }
    });
}

impl ImxrtUsb {
    /// Recompute the cached interrupt line (see `Timebase::level`).
    fn refresh_irq(&self) {
        self.time.set_level(self.irq());
    }

    fn tick_inner(&mut self, cycles: u64) -> PeripheralTickResult {
        self.time.advance(cycles);
        let now = self.time.now();
        // While the host is active the controller's flags move from the bus
        // tick; re-check the interrupt line every microsecond.
        let until = (self.running() && self.host_enabled).then(|| now + self.time.us(1));
        super::wake_hint(now, until)
    }
}

impl Peripheral for ImxrtUsb {
    /// Walked only while timed work is in flight or the interrupt line is
    /// asserted (so its deassert is reconciled); MMIO re-arms it.
    fn legacy_tick_active(&self) -> bool {
        (self.host_enabled && self.running() && !matches!(self.phase, Phase::Done))
            || self.time.level()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let v = byte_of(self.read_reg(offset as u32), offset);
        self.refresh_irq();
        Ok(v)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        self.write_reg(offset as u32, (value as u32) << shift, 0xFF << shift);
        self.refresh_irq();
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = self.read_reg(offset as u32);
        self.refresh_irq();
        Ok(v)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset as u32, value, u32::MAX);
        self.refresh_irq();
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        let r = self.tick_inner(cycles);
        self.refresh_irq();
        r
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.time.level())
    }
    fn needs_bus_tick(&self) -> bool {
        self.prime != 0
            || (self.host_enabled && self.running() && !matches!(self.phase, Phase::Done))
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.service_primes(bus);
        if self.host_enabled {
            self.host_step(bus);
        }
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.time.attach_clock(clock);
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.time.attach_cpu_hz(hz);
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
            "peripheral": "imxrt_usb",
            "usbcmd": self.read_reg(USBCMD),
            "usbmode": self.reg(USBMODE),
            "address": self.log.address,
            "controls": self.log.control.len(),
            "phase": format!("{:?}", self.phase),
            "endptstat": self.stat,
            "prime": self.prime,
        })
    }
}

/// USBPHY (UTMI transceiver): a register file with SET/CLR/TOG aliases.
#[derive(Debug)]
pub struct ImxrtUsbPhy {
    regs: [u32; 9],
}

/// (offset, reset, read-only)
const PHY_REGS: &[(u32, u32, bool)] = &[
    (0x00, 0x001E_1C00, false), // PWD
    (0x10, 0x1006_0607, false), // TX
    (0x20, 0x0000_0000, false), // RX
    (0x30, 0xC020_0000, false), // CTRL (SFTRST | CLKGATE at reset)
    (0x40, 0x0000_0000, false), // STATUS
    (0x50, 0x7F18_0000, false), // DEBUG
    (0x60, 0x0000_0000, true),  // DEBUG0_STATUS
    (0x70, 0x0000_1000, false), // DEBUG1
    (0x80, 0x0403_0000, true),  // VERSION
];

impl Default for ImxrtUsbPhy {
    fn default() -> Self {
        let mut regs = [0; 9];
        for (i, &(_, v, _)) in PHY_REGS.iter().enumerate() {
            regs[i] = v;
        }
        Self { regs }
    }
}

impl ImxrtUsbPhy {
    pub fn read_reg(&self, off: u32) -> u32 {
        let i = (off >> 4) as usize;
        if i < PHY_REGS.len() {
            self.regs[i]
        } else {
            0
        }
    }
    pub fn write_reg(&mut self, off: u32, value: u32, mask: u32) {
        let i = (off >> 4) as usize;
        if i >= PHY_REGS.len() || PHY_REGS[i].2 {
            return;
        }
        let v = value & mask;
        let old = self.regs[i];
        self.regs[i] = match off & 0xC {
            0x0 => (old & !mask) | v,
            0x4 => old | v,
            0x8 => old & !v,
            _ => old ^ v,
        };
    }
}

impl Peripheral for ImxrtUsbPhy {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(byte_of(self.read_reg(offset as u32 & !3), offset))
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        self.write_reg(offset as u32 & !3, (value as u32) << shift, 0xFF << shift);
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset as u32))
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset as u32, value, u32::MAX);
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CycleClock;

    #[test]
    fn controller_reset_self_clears() {
        let mut u = ImxrtUsb::new();
        let c = CycleClock::default();
        u.attach_cycle_clock(c.clone());
        u.write_reg(USBCMD, CMD_RST, u32::MAX);
        assert_ne!(u.read_reg(USBCMD) & CMD_RST, 0);
        c.publish(u.time.us(RESET_US));
        assert_eq!(u.read_reg(USBCMD) & CMD_RST, 0);
        assert_eq!(u.read_reg(USBMODE), 0x5000);
    }

    #[test]
    fn host_log_lines_decode_ids_interfaces_and_strings() {
        let u = ImxrtUsb::new();
        let logs = u.logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].name, "host");
        assert!(logs[0].entries.is_empty(), "nothing enumerated yet");

        let log = UsbHostLog {
            resets: 1,
            device_descriptor: vec![
                18, 1, 0x00, 0x02, 0, 0, 0, 64, 0xc9, 0x1f, 0x35, 0x01, 0, 1, 1, 2, 0, 1,
            ],
            address: Some(5),
            // config, interface 3 (HID, 1 endpoint), endpoint 0x81 interrupt 64
            config_descriptor: vec![
                9, 2, 25, 0, 1, 1, 0, 0x80, 50, 9, 4, 3, 0, 1, 3, 0, 0, 0, 7, 5, 0x81, 3, 64, 0, 1,
            ],
            strings: vec![(2, "Widget".into())],
            control: vec![(
                Setup {
                    bm_request_type: 0,
                    b_request: 9,
                    w_value: 1,
                    w_index: 0,
                    w_length: 0,
                    data: Vec::new(),
                },
                Vec::new(),
                true,
            )],
            ins: vec![(1, vec![0xde, 0xad])],
            outs: vec![(2, 64)],
            notes: vec!["control 80 06 0300 stalled".into()],
        };
        assert_eq!(
            log.lines(),
            [
                "resets 1",
                "device 1fc9:0135 class 00/00/00 usb 0200 ep0 64",
                "address 5",
                "interface 3 alt 0 class 03/00/00 endpoints 1",
                "endpoint 0x81 attr 0x03 max 64",
                "string 2 \"Widget\"",
                "control 00 09 0001 0000 0000 ok",
                "in 1 de ad",
                "out 2 64",
                "note control 80 06 0300 stalled",
            ]
        );
    }

    #[test]
    fn phy_set_clr_aliases() {
        let mut p = ImxrtUsbPhy::default();
        p.write_reg(0x38, 0xC000_0000, u32::MAX); // CTRL_CLR SFTRST|CLKGATE
        assert_eq!(p.read_reg(0x30), 0x0020_0000);
        p.write_reg(0x00, 0, u32::MAX);
        assert_eq!(p.read_reg(0x00), 0);
    }
}
