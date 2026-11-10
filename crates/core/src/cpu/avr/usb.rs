// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ATmega32U4 USB device controller, and the host on the other end of the
//! cable (Microchip DS7766, chapters 21 "USB controller" and 22 "USB device
//! operating modes").
//!
//! Registers (data-space addresses):
//!
//! | Register | Addr | Kept |
//! |----------|------|------|
//! | `UHWCON`  | 0xD7 | stored (UVREGE) |
//! | `USBCON`  | 0xD8 | USBE, FRZCLK, OTGPADE, VBUSTE |
//! | `USBSTA`  | 0xD9 | VBUS = 1 (the cable is plugged), read-only |
//! | `USBINT`  | 0xDA | VBUSTI, write 0 to clear (never raised) |
//! | `UDCON`   | 0xE0 | DETACH (reset 1), RMWKUP, LSM, RSTCPU |
//! | `UDINT`   | 0xE1 | SOFI, EORSTI; write 0 to clear |
//! | `UDIEN`   | 0xE2 | interrupt enables |
//! | `UDADDR`  | 0xE3 | stored |
//! | `UDFNUML/H` | 0xE4/0xE5 | 11-bit frame number |
//! | `UEINTX`  | 0xE8 | endpoint flags of the selected endpoint |
//! | `UENUM`   | 0xE9 | selected endpoint 0..6 |
//! | `UERST`   | 0xEA | endpoint FIFO reset |
//! | `UECONX`  | 0xEB | EPEN, STALLRQ |
//! | `UECFG0X/1X` | 0xEC/0xED | type, direction, size, ALLOC |
//! | `UESTA0X` | 0xEE | CFGOK |
//! | `UEIENX`  | 0xF0 | endpoint interrupt enables |
//! | `UEDATX`  | 0xF1 | FIFO data |
//! | `UEBCLX`  | 0xF2 | bytes in the FIFO |
//! | `UEINT`   | 0xF4 | endpoints with an enabled flag set |
//!
//! The device side is the register contract the stock Arduino AVR core
//! (`USBCore.cpp`, `CDC.cpp`) runs on:
//! - attach: with USBE set, the clock unfrozen, the PLL on and DETACH clear,
//!   the host sees the pull-up and resets the bus one frame later (EORSTI);
//! - a start of frame (SOFI) every 1 ms after that. The core flushes the CDC
//!   IN bank from its SOF handler, so these are what get partial lines out;
//! - EP0: the host puts a SETUP in the FIFO (RXSTPI). Clearing RXSTPI frees
//!   the bank (TXINI). Every later TXINI clear is an IN packet the host takes
//!   at once, and TXINI comes back. A control write's data stage lands in the
//!   FIFO with RXOUTI when the SETUP is acknowledged;
//! - bulk/interrupt IN: bytes written to UEDATX fill a 64-byte bank; RWAL
//!   drops when it is full; clearing FIFOCON sends it, and the host drains it
//!   at once. Bulk IN data is the console;
//! - USB_GEN is level-sensitive on `UDINT & UDIEN`, USB_COM on any enabled
//!   endpoint with `UEINTX & UEIENX`.
//!
//! The host enumerates as an OS does ([`usb_host::enumeration_script`]), one
//! transfer per frame, and once the configuration is set it opens the CDC ACM
//! port the configuration descriptor announces the way a terminal does:
//! SET_LINE_CODING 115200 8N1, then SET_CONTROL_LINE_STATE with DTR and RTS.
//! That is what makes stock `Serial` (and `while (!Serial)`) go.
//!
//! Not modelled: suspend/resume (the bus is never idle, so SUSPI never sets),
//! remote wake-up, the OTG pad and VBUS transition interrupts, bulk OUT data
//! from the host, NAK and error flags, and double-bank ping-pong (the host
//! drains a released bank at once, so one bank is always free).

use std::cell::Cell;
use std::collections::VecDeque;

use super::Avr;
use crate::peripherals::usb_host::{self, HostStep, Setup, UsbHostLog};

pub(super) const ADDR_UHWCON: u16 = 0xD7;
pub(super) const ADDR_UEINT: u16 = 0xF4;
const ADDR_USBCON: u16 = 0xD8;
const ADDR_USBSTA: u16 = 0xD9;
const ADDR_USBINT: u16 = 0xDA;
const ADDR_UDCON: u16 = 0xE0;
const ADDR_UDINT: u16 = 0xE1;
const ADDR_UDIEN: u16 = 0xE2;
const ADDR_UDADDR: u16 = 0xE3;
const ADDR_UDFNUML: u16 = 0xE4;
const ADDR_UDFNUMH: u16 = 0xE5;
const ADDR_UDMFN: u16 = 0xE6;
const ADDR_UEINTX: u16 = 0xE8;
const ADDR_UENUM: u16 = 0xE9;
const ADDR_UERST: u16 = 0xEA;
const ADDR_UECONX: u16 = 0xEB;
const ADDR_UECFG0X: u16 = 0xEC;
const ADDR_UECFG1X: u16 = 0xED;
const ADDR_UESTA0X: u16 = 0xEE;
const ADDR_UESTA1X: u16 = 0xEF;
const ADDR_UEIENX: u16 = 0xF0;
const ADDR_UEDATX: u16 = 0xF1;
const ADDR_UEBCLX: u16 = 0xF2;
const ADDR_UEBCHX: u16 = 0xF3;
/// `PLLCSR` (I/O 0x29): PINDIV, PLLE, PLOCK.
pub(super) const ADDR_PLLCSR: u16 = 0x49;

const USBCON_USBE: u8 = 1 << 7;
const USBCON_FRZCLK: u8 = 1 << 5;
const UDCON_DETACH: u8 = 1 << 0;
const UDINT_SOFI: u8 = 1 << 2;
const UDINT_EORSTI: u8 = 1 << 3;
/// UDINT/UDIEN bits with an interrupt: SUSPI, SOFI, EORSTI, WAKEUPI, EORSMI, UPRSMI.
const UDINT_IRQ_MASK: u8 = 0x7D;
const PLLCSR_PINDIV: u8 = 1 << 4;
const PLLCSR_PLLE: u8 = 1 << 1;
const PLLCSR_PLOCK: u8 = 1 << 0;

const FIFOCON: u8 = 1 << 7;
const RWAL: u8 = 1 << 5;
const RXSTPI: u8 = 1 << 3;
const RXOUTI: u8 = 1 << 2;
const TXINI: u8 = 1 << 0;
/// UEINTX flags that can raise USB_COM (FIFOCON and RWAL cannot).
const UEINTX_IRQ_MASK: u8 = 0x5F;
const UECONX_EPEN: u8 = 1 << 0;
const UECONX_STALLRQC: u8 = 1 << 4;
const UECONX_STALLRQ: u8 = 1 << 5;
const UECFG1X_ALLOC: u8 = 1 << 1;
const UESTA0X_CFGOK: u8 = 1 << 7;

/// Endpoints 0..6.
const ENDPOINTS: usize = 7;

/// One endpoint: its registers and the bank the CPU sees.
#[derive(Debug, Clone, Default)]
struct Endpoint {
    ueconx: u8,
    uecfg0x: u8,
    uecfg1x: u8,
    ueienx: u8,
    ueintx: u8,
    fifo: Vec<u8>,
    /// Read position in `fifo` for OUT/SETUP data. A `Cell` because a
    /// `UEDATX` read pops a byte, and data-space reads take `&self`.
    rd: Cell<usize>,
}

impl Endpoint {
    fn enabled(&self) -> bool {
        self.ueconx & UECONX_EPEN != 0
    }
    fn configured(&self) -> bool {
        self.enabled() && self.uecfg1x & UECFG1X_ALLOC != 0
    }
    fn is_control(&self) -> bool {
        self.uecfg0x >> 6 == 0
    }
    fn is_bulk(&self) -> bool {
        self.uecfg0x >> 6 == 2
    }
    fn is_in(&self) -> bool {
        self.uecfg0x & 1 != 0
    }
    /// EPSIZE: 8 << n bytes (n = 0..3 on the 32U4's 64-byte endpoints).
    fn size(&self) -> usize {
        8 << ((self.uecfg1x >> 4) & 0x07).min(3)
    }
    fn byte_count(&self) -> usize {
        self.fifo.len().saturating_sub(self.rd.get())
    }
    /// An empty bank on an IN endpoint: free to fill, nothing to send.
    fn reset_in_bank(&mut self) {
        self.fifo.clear();
        self.rd.set(0);
        if self.is_in() && !self.is_control() {
            self.ueintx |= FIFOCON | RWAL | TXINI;
        }
    }
}

/// The control transfer the host is running on EP0.
#[derive(Debug, Clone)]
struct ControlXfer {
    setup: Setup,
    /// The device cleared RXSTPI.
    acked: bool,
    /// IN data stage so far.
    data: Vec<u8>,
    /// The device read the OUT data stage (ClearOUT).
    out_taken: bool,
}

/// Where the cable is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    /// Pull-up off (DETACH, USB off, clock frozen or PLL down).
    Detached,
    /// The host saw the pull-up; the bus reset ends at `next_event`.
    Resetting,
    /// Frames run every 1 ms.
    Running,
}

/// The USB device controller and its scripted host.
#[derive(Debug, Clone)]
pub(super) struct UsbDevice {
    uhwcon: u8,
    usbcon: u8,
    usbint: u8,
    udcon: u8,
    udint: u8,
    udien: u8,
    udaddr: u8,
    frame: u16,
    uenum: u8,
    eps: [Endpoint; ENDPOINTS],
    pllcsr: u8,
    link: Link,
    /// CPU cycle of the next bus event (end of reset, or the next SOF).
    next_event: u64,
    script: VecDeque<HostStep>,
    /// Frames the host stays idle (a `wait_us` step).
    idle_frames: u32,
    xfer: Option<ControlXfer>,
    cdc_opened: bool,
    pub(super) log: UsbHostLog,
}

impl Default for UsbDevice {
    fn default() -> Self {
        Self {
            uhwcon: 0,
            // USBCON resets with FRZCLK set; UDCON with DETACH set.
            usbcon: USBCON_FRZCLK,
            usbint: 0,
            udcon: UDCON_DETACH,
            udint: 0,
            udien: 0,
            udaddr: 0,
            frame: 0,
            uenum: 0,
            eps: Default::default(),
            pllcsr: 0,
            link: Link::Detached,
            next_event: u64::MAX,
            script: VecDeque::new(),
            idle_frames: 0,
            xfer: None,
            cdc_opened: false,
            log: UsbHostLog::default(),
        }
    }
}

impl UsbDevice {
    /// The pull-up is on: USB enabled, clock running, PLL locked, attached.
    fn pulled_up(&self) -> bool {
        self.usbcon & USBCON_USBE != 0
            && self.usbcon & USBCON_FRZCLK == 0
            && self.udcon & UDCON_DETACH == 0
            && self.pllcsr & PLLCSR_PLLE != 0
    }

    /// Bus events are pending (the CPU must keep stepping toward them).
    pub(super) fn active(&self) -> bool {
        self.link != Link::Detached
    }

    /// CPU cycles in one 1 ms frame. PINDIV says which crystal the firmware
    /// divides for the 48 MHz PLL: set for 16 MHz, clear for 8 MHz.
    fn frame_cycles(&self) -> u64 {
        if self.pllcsr & PLLCSR_PINDIV != 0 {
            16_000
        } else {
            8_000
        }
    }

    fn ep(&self) -> &Endpoint {
        &self.eps[usize::from(self.uenum).min(ENDPOINTS - 1)]
    }

    fn ep_mut(&mut self) -> &mut Endpoint {
        &mut self.eps[usize::from(self.uenum).min(ENDPOINTS - 1)]
    }

    fn gen_irq(&self) -> bool {
        self.udint & self.udien & UDINT_IRQ_MASK != 0
    }

    fn ueint(&self) -> u8 {
        self.eps.iter().enumerate().fold(0, |acc, (i, ep)| {
            if ep.enabled() && ep.ueintx & ep.ueienx & UEINTX_IRQ_MASK != 0 {
                acc | (1 << i)
            } else {
                acc
            }
        })
    }

    pub(super) fn read(&self, addr: u16) -> Option<u8> {
        let ep = self.ep();
        Some(match addr {
            ADDR_UHWCON => self.uhwcon,
            ADDR_USBCON => self.usbcon,
            // VBUS: the board is on a powered cable.
            ADDR_USBSTA => 0x01,
            ADDR_USBINT => self.usbint,
            ADDR_UDCON => self.udcon,
            ADDR_UDINT => self.udint,
            ADDR_UDIEN => self.udien,
            ADDR_UDADDR => self.udaddr,
            ADDR_UDFNUML => self.frame as u8,
            ADDR_UDFNUMH => (self.frame >> 8) as u8 & 0x07,
            ADDR_UDMFN => 0,
            ADDR_UEINTX => ep.ueintx,
            ADDR_UENUM => self.uenum,
            ADDR_UERST => 0,
            ADDR_UECONX => ep.ueconx,
            ADDR_UECFG0X => ep.uecfg0x,
            ADDR_UECFG1X => ep.uecfg1x,
            ADDR_UESTA0X => {
                if ep.uecfg1x & UECFG1X_ALLOC != 0 {
                    UESTA0X_CFGOK
                } else {
                    0
                }
            }
            ADDR_UESTA1X => 0,
            ADDR_UEIENX => ep.ueienx,
            ADDR_UEDATX => {
                let i = ep.rd.get();
                match ep.fifo.get(i) {
                    Some(&b) => {
                        ep.rd.set(i + 1);
                        b
                    }
                    None => 0,
                }
            }
            ADDR_UEBCLX => ep.byte_count() as u8,
            ADDR_UEBCHX => (ep.byte_count() >> 8) as u8,
            ADDR_UEINT => self.ueint(),
            ADDR_PLLCSR => {
                // The PLL locks as soon as it is on (lock time not modelled).
                let lock = if self.pllcsr & PLLCSR_PLLE != 0 {
                    PLLCSR_PLOCK
                } else {
                    0
                };
                (self.pllcsr & !PLLCSR_PLOCK) | lock
            }
            // Reserved addresses inside the USB block read as 0.
            a if (ADDR_UHWCON..=ADDR_UEINT).contains(&a) => 0,
            _ => return None,
        })
    }

    /// A register write. Returns bytes the host received on a bulk IN
    /// endpoint (console output), if this write released a bank.
    pub(super) fn write(&mut self, addr: u16, value: u8, now: u64) -> Vec<u8> {
        let mut console = Vec::new();
        match addr {
            ADDR_UHWCON => self.uhwcon = value & 0x01,
            ADDR_USBCON => {
                self.usbcon = value & 0xB1;
                if self.usbcon & USBCON_USBE == 0 {
                    // USBE clear resets the controller (DS7766 21.11).
                    let pll = self.pllcsr;
                    let log = std::mem::take(&mut self.log);
                    *self = Self {
                        usbcon: self.usbcon,
                        pllcsr: pll,
                        log,
                        ..Self::default()
                    };
                }
            }
            ADDR_USBINT => self.usbint &= value,
            ADDR_UDCON => self.udcon = value & 0x0F,
            ADDR_UDINT => self.udint &= value,
            ADDR_UDIEN => self.udien = value & UDINT_IRQ_MASK,
            ADDR_UDADDR => self.udaddr = value,
            ADDR_UENUM => self.uenum = value & 0x07,
            ADDR_UERST => {
                for (i, ep) in self.eps.iter_mut().enumerate() {
                    if value & (1 << i) != 0 {
                        ep.reset_in_bank();
                    }
                }
            }
            ADDR_UECONX => self.write_ueconx(value),
            ADDR_UECFG0X => self.ep_mut().uecfg0x = value & 0xC1,
            ADDR_UECFG1X => {
                let ep = self.ep_mut();
                ep.uecfg1x = value & 0x7E;
                ep.ueintx = 0;
                ep.reset_in_bank();
            }
            ADDR_UEIENX => self.ep_mut().ueienx = value & 0xDF,
            ADDR_UEDATX => {
                let ep = self.ep_mut();
                if ep.fifo.len() < ep.size() {
                    ep.fifo.push(value);
                }
                if !ep.is_control() && ep.is_in() && ep.fifo.len() >= ep.size() {
                    ep.ueintx &= !RWAL;
                }
            }
            ADDR_UEINTX => console = self.write_ueintx(value),
            ADDR_PLLCSR => self.pllcsr = value & (PLLCSR_PINDIV | PLLCSR_PLLE),
            _ => {}
        }
        self.update_link(now);
        console
    }

    fn write_ueconx(&mut self, value: u8) {
        let ep = self.ep_mut();
        ep.ueconx = (ep.ueconx & UECONX_STALLRQ) | (value & UECONX_EPEN);
        if value & UECONX_STALLRQC != 0 {
            ep.ueconx &= !UECONX_STALLRQ;
        }
        if value & UECONX_STALLRQ != 0 {
            ep.ueconx |= UECONX_STALLRQ;
            if self.uenum == 0 {
                // The host gets a STALL handshake for the rest of the transfer.
                if let Some(x) = self.xfer.take() {
                    self.log
                        .notes
                        .push(format!("stall on request {:02x}", x.setup.b_request));
                    self.log.control.push((x.setup, x.data, false));
                }
            }
        }
    }

    fn write_ueintx(&mut self, value: u8) -> Vec<u8> {
        let ep0 = self.uenum == 0;
        let ep = self.ep_mut();
        let old = ep.ueintx;
        // Flags clear on a written 0; a written 1 changes nothing. RWAL is
        // read-only.
        ep.ueintx = old & (value | RWAL);
        let cleared = old & !ep.ueintx;
        if ep0 || ep.is_control() {
            self.ep0_flags_cleared(cleared);
            return Vec::new();
        }
        if cleared & FIFOCON == 0 {
            return Vec::new();
        }
        let ep = self.ep_mut();
        if ep.is_in() {
            // The bank goes on the bus; the host takes it at once.
            let data = std::mem::take(&mut ep.fifo);
            let bulk = ep.is_bulk();
            ep.reset_in_bank();
            let n = self.uenum;
            if bulk && !data.is_empty() {
                self.log.ins.push((n, data.clone()));
                return data;
            }
        } else {
            // OUT bank released: empty until the host sends more.
            ep.fifo.clear();
            ep.rd.set(0);
            ep.ueintx &= !(RWAL | RXOUTI);
        }
        Vec::new()
    }

    /// EP0 flags the firmware just cleared.
    fn ep0_flags_cleared(&mut self, cleared: u8) {
        if cleared & RXSTPI != 0 {
            // SETUP acknowledged: the bank is free for the data/status stage.
            let ep = &mut self.eps[0];
            ep.fifo.clear();
            ep.rd.set(0);
            ep.ueintx |= TXINI;
            if let Some(x) = self.xfer.as_mut() {
                x.acked = true;
                if !x.setup.dir_in() && !x.setup.data.is_empty() {
                    ep.fifo = x.setup.data.clone();
                    ep.ueintx |= RXOUTI;
                }
            }
            return;
        }
        if cleared & RXOUTI != 0 {
            let ep = &mut self.eps[0];
            ep.fifo.clear();
            ep.rd.set(0);
            if let Some(x) = self.xfer.as_mut() {
                x.out_taken = true;
            }
        }
        if cleared & TXINI != 0 {
            // An IN packet: the host takes it, and the bank is free again.
            // During a control write the bank still holds the OUT data stage
            // and the IN packet is the zero-length status.
            let dir_in = self.xfer.as_ref().is_some_and(|x| x.setup.dir_in());
            let ep = &mut self.eps[0];
            let packet = if dir_in || ep.ueintx & RXOUTI == 0 {
                ep.rd.set(0);
                std::mem::take(&mut ep.fifo)
            } else {
                Vec::new()
            };
            ep.ueintx |= TXINI;
            let max = ep.size();
            let Some(x) = self.xfer.as_mut().filter(|x| x.acked) else {
                return;
            };
            let done = if x.setup.dir_in() {
                x.data.extend_from_slice(&packet);
                packet.len() < max || x.data.len() >= usize::from(x.setup.w_length)
            } else {
                // The status stage of a control write, once its data (if
                // any) was read. The core's early ClearIN before reading the
                // data stage is not one.
                x.setup.data.is_empty() || x.out_taken
            };
            if done {
                self.finish_control();
            }
        }
    }

    fn finish_control(&mut self) {
        let Some(x) = self.xfer.take() else {
            return;
        };
        let mut data = x.data;
        if !x.setup.dir_in() {
            data = x.setup.data.clone();
        }
        data.truncate(usize::from(x.setup.w_length));
        let set_config = x.setup.bm_request_type == 0 && x.setup.b_request == 9;
        usb_host::after_control(&mut self.log, &mut self.script, x.setup, data);
        if set_config && !self.cdc_opened {
            self.cdc_opened = true;
            self.queue_cdc_open();
        }
    }

    /// Open the CDC ACM port of the configuration, as a terminal does.
    fn queue_cdc_open(&mut self) {
        let cfg = &self.log.config_descriptor;
        let mut i = 0;
        let mut acm = None;
        while i + 1 < cfg.len() && cfg[i] >= 2 {
            let d = &cfg[i..(i + usize::from(cfg[i])).min(cfg.len())];
            // Interface descriptor, class 02 (CDC) subclass 02 (ACM).
            if d[1] == 4 && d.len() >= 9 && d[5] == 0x02 && d[6] == 0x02 {
                acm = Some(u16::from(d[2]));
                break;
            }
            i += usize::from(cfg[i]);
        }
        let Some(iface) = acm else {
            return;
        };
        // 115200 baud, 1 stop bit, no parity, 8 data bits.
        let mut coding = 115_200u32.to_le_bytes().to_vec();
        coding.extend_from_slice(&[0, 0, 8]);
        self.script.push_back(HostStep::Control(Setup {
            bm_request_type: 0x21,
            b_request: 0x20, // SET_LINE_CODING
            w_value: 0,
            w_index: iface,
            w_length: 7,
            data: coding,
        }));
        self.script.push_back(HostStep::Control(Setup {
            bm_request_type: 0x21,
            b_request: 0x22, // SET_CONTROL_LINE_STATE
            w_value: 0x0003, // DTR | RTS
            w_index: iface,
            w_length: 0,
            data: Vec::new(),
        }));
    }

    fn update_link(&mut self, now: u64) {
        match (self.link, self.pulled_up()) {
            (Link::Detached, true) => {
                // The host notices the device and holds the bus in reset.
                self.link = Link::Resetting;
                self.next_event = now + self.frame_cycles();
            }
            (Link::Resetting | Link::Running, false) => {
                self.link = Link::Detached;
                self.next_event = u64::MAX;
                self.xfer = None;
            }
            _ => {}
        }
    }

    /// Run the bus up to CPU cycle `now`.
    pub(super) fn tick(&mut self, now: u64) {
        while now >= self.next_event {
            match self.link {
                Link::Detached => {
                    self.next_event = u64::MAX;
                }
                Link::Resetting => {
                    self.log.resets += 1;
                    self.udint |= UDINT_EORSTI;
                    self.udaddr = 0;
                    for ep in &mut self.eps {
                        ep.fifo.clear();
                        ep.rd.set(0);
                    }
                    self.script = usb_host::enumeration_script();
                    self.cdc_opened = false;
                    self.xfer = None;
                    self.link = Link::Running;
                    self.next_event += self.frame_cycles();
                }
                Link::Running => {
                    self.frame = (self.frame + 1) & 0x07FF;
                    self.udint |= UDINT_SOFI;
                    self.host_frame();
                    self.next_event += self.frame_cycles();
                }
            }
        }
    }

    /// Cycles until the next bus event, for a sleeping core.
    pub(super) fn wake_cycles(&self, now: u64) -> Option<u64> {
        self.active()
            .then(|| self.next_event.saturating_sub(now).max(1))
    }

    /// One frame of the host: start the next transfer if EP0 is ready.
    fn host_frame(&mut self) {
        if self.xfer.is_some() {
            return;
        }
        if self.idle_frames > 0 {
            self.idle_frames -= 1;
            return;
        }
        let ep0 = &self.eps[0];
        if !ep0.configured() || !ep0.is_control() || ep0.ueintx & RXSTPI != 0 {
            return;
        }
        match self.script.pop_front() {
            Some(HostStep::Control(setup)) => {
                let ep = &mut self.eps[0];
                // A new SETUP clears a stall (DS7766 22.12.2).
                ep.ueconx &= !UECONX_STALLRQ;
                ep.fifo = setup.bytes().to_vec();
                ep.rd.set(0);
                ep.ueintx = (ep.ueintx & !(TXINI | RXOUTI)) | RXSTPI;
                self.xfer = Some(ControlXfer {
                    setup,
                    acked: false,
                    data: Vec::new(),
                    out_taken: false,
                });
            }
            Some(HostStep::Wait { us }) => {
                self.idle_frames = u32::try_from(us / 1000).unwrap_or(u32::MAX);
            }
            Some(other) => {
                self.log
                    .notes
                    .push(format!("host step not supported here: {other:?}"));
            }
            None => {}
        }
    }
}

/// Internal vector numbers past the 328P's 26 (see `m32u4_vector`).
pub const VEC_USB_GEN: u32 = 27;
pub const VEC_USB_COM: u32 = 28;

impl Avr {
    /// USB register read on the ATmega32U4.
    pub(super) fn usb_read(&self, addr: u16) -> Option<u8> {
        self.usb.read(addr)
    }

    /// USB register write on the ATmega32U4. Bytes the host takes from a
    /// bulk IN endpoint go to the console.
    pub(super) fn usb_write(&mut self, addr: u16, value: u8) {
        let console = self.usb.write(addr, value, self.cycles);
        self.usb_console(console);
        self.sync_usb_irq();
    }

    fn usb_console(&mut self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        self.serial_tx.extend_from_slice(&bytes);
        if let Some(sink) = &self.serial_sink {
            if let Ok(mut g) = sink.lock() {
                g.extend_from_slice(&bytes);
            }
        }
    }

    /// Run the bus to the current cycle.
    #[inline]
    pub(super) fn usb_tick(&mut self) {
        if self.cycles >= self.usb.next_event {
            self.usb.tick(self.cycles);
            self.sync_usb_irq();
        }
    }

    /// USB_GEN and USB_COM are levels: pending while a flag is set and enabled.
    pub(super) fn sync_usb_irq(&mut self) {
        let mut want = 0u64;
        if self.usb.gen_irq() {
            want |= 1 << VEC_USB_GEN;
        }
        if self.usb.ueint() != 0 {
            want |= 1 << VEC_USB_COM;
        }
        let mask = (1u64 << VEC_USB_GEN) | (1u64 << VEC_USB_COM);
        self.pending_irq = (self.pending_irq & !mask) | want;
    }

    /// What the scripted host saw (tests).
    pub fn usb_host_log(&self) -> &UsbHostLog {
        &self.usb.log
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev() -> UsbDevice {
        UsbDevice::default()
    }

    /// The core's attach sequence, then EP0 as `InitEP(0, control, 64)`.
    fn attached() -> UsbDevice {
        let mut u = dev();
        u.write(ADDR_UHWCON, 0x01, 0);
        u.write(ADDR_USBCON, USBCON_USBE | USBCON_FRZCLK, 0);
        u.write(ADDR_PLLCSR, PLLCSR_PINDIV, 0);
        u.write(ADDR_PLLCSR, PLLCSR_PINDIV | PLLCSR_PLLE, 0);
        assert_eq!(
            u.read(ADDR_PLLCSR).unwrap() & PLLCSR_PLOCK,
            PLLCSR_PLOCK,
            "PLLE locks the PLL"
        );
        u.write(ADDR_USBCON, USBCON_USBE | 0x10, 0);
        u.write(ADDR_UDCON, 0, 0);
        assert_eq!(u.link, Link::Resetting);
        u.tick(16_000);
        assert_eq!(u.udint & UDINT_EORSTI, UDINT_EORSTI, "end of reset");
        u.write(ADDR_UENUM, 0, 0);
        u.write(ADDR_UECONX, UECONX_EPEN, 0);
        u.write(ADDR_UECFG0X, 0, 0);
        u.write(ADDR_UECFG1X, 0x32, 0);
        u.write(ADDR_UEIENX, RXSTPI, 0);
        u
    }

    #[test]
    fn detached_until_the_core_attaches() {
        let mut u = dev();
        u.tick(1_000_000);
        assert!(!u.active());
        assert_eq!(u.read(ADDR_UDCON), Some(UDCON_DETACH));
    }

    #[test]
    fn sof_every_frame_after_reset() {
        let mut u = attached();
        u.udint = 0;
        u.tick(16_000 * 2 - 1);
        assert_eq!(u.udint & UDINT_SOFI, 0);
        u.tick(16_000 * 2);
        assert_eq!(u.udint & UDINT_SOFI, UDINT_SOFI);
        assert_eq!(u.read(ADDR_UDFNUML), Some(1));
    }

    #[test]
    fn get_descriptor_runs_through_ep0() {
        let mut u = attached();
        u.tick(32_000);
        let ep0 = &u.eps[0];
        assert_eq!(ep0.ueintx & RXSTPI, RXSTPI, "SETUP delivered");
        assert_eq!(u.ueint(), 1, "USB_COM from EP0");
        // Recv(&setup, 8)
        let setup: Vec<u8> = (0..8).map(|_| u.read(ADDR_UEDATX).unwrap()).collect();
        assert_eq!(setup, [0x80, 6, 0, 1, 0, 0, 64, 0]);
        // ClearSetupInt, then WaitIN sees TXINI.
        u.write(ADDR_UEINTX, !(RXSTPI | RXOUTI | TXINI), 0);
        assert_eq!(u.read(ADDR_UEINTX).unwrap() & TXINI, TXINI);
        for b in [
            18u8, 1, 0, 2, 0xEF, 2, 1, 64, 0x41, 0x23, 0x36, 0x80, 0, 1, 1, 2, 3, 1,
        ] {
            u.write(ADDR_UEDATX, b, 0);
        }
        // ClearIN: an 18-byte short packet ends the transfer.
        u.write(ADDR_UEINTX, !TXINI, 0);
        assert!(u.xfer.is_none());
        assert_eq!(u.log.device_descriptor.len(), 18);
    }

    #[test]
    fn control_write_waits_for_its_data_stage() {
        let mut u = attached();
        u.script.clear();
        u.script.push_back(HostStep::Control(Setup {
            bm_request_type: 0x21,
            b_request: 0x20,
            w_value: 0,
            w_index: 0,
            w_length: 7,
            data: vec![0x00, 0xC2, 0x01, 0x00, 0, 0, 8],
        }));
        u.tick(32_000);
        for _ in 0..8 {
            u.read(ADDR_UEDATX);
        }
        u.write(ADDR_UEINTX, !(RXSTPI | RXOUTI | TXINI), 0);
        // The core's ClearIN before reading the data is not the status stage.
        u.write(ADDR_UEINTX, !TXINI, 0);
        assert!(u.xfer.is_some());
        assert_eq!(u.read(ADDR_UEINTX).unwrap() & RXOUTI, RXOUTI);
        let data: Vec<u8> = (0..7).map(|_| u.read(ADDR_UEDATX).unwrap()).collect();
        assert_eq!(data, [0x00, 0xC2, 0x01, 0x00, 0, 0, 8]);
        u.write(ADDR_UEINTX, !RXOUTI, 0);
        u.write(ADDR_UEINTX, !TXINI, 0);
        assert!(u.xfer.is_none());
        assert!(u.log.control.last().unwrap().2);
    }

    #[test]
    fn bulk_in_bank_goes_out_when_fifocon_clears() {
        let mut u = attached();
        u.write(ADDR_UENUM, 3, 0);
        u.write(ADDR_UECONX, UECONX_EPEN, 0);
        u.write(ADDR_UECFG0X, 0x81, 0); // bulk IN
        u.write(ADDR_UECFG1X, 0x36, 0); // 64 bytes, two banks
        assert_eq!(u.read(ADDR_UEINTX).unwrap() & RWAL, RWAL);
        for b in b"hi\r\n" {
            u.write(ADDR_UEDATX, *b, 0);
        }
        assert_eq!(u.read(ADDR_UEBCLX), Some(4));
        // ReleaseTX
        let out = u.write(ADDR_UEINTX, 0x3A, 0);
        assert_eq!(out, b"hi\r\n");
        assert_eq!(u.read(ADDR_UEBCLX), Some(0));
        assert_eq!(
            u.read(ADDR_UEINTX).unwrap() & (RWAL | FIFOCON),
            RWAL | FIFOCON
        );
    }

    #[test]
    fn a_full_bank_drops_rwal() {
        let mut u = attached();
        u.write(ADDR_UENUM, 3, 0);
        u.write(ADDR_UECONX, UECONX_EPEN, 0);
        u.write(ADDR_UECFG0X, 0x81, 0);
        u.write(ADDR_UECFG1X, 0x36, 0);
        for _ in 0..64 {
            u.write(ADDR_UEDATX, b'x', 0);
        }
        assert_eq!(u.read(ADDR_UEINTX).unwrap() & RWAL, 0);
    }

    #[test]
    fn set_configuration_queues_the_cdc_port_open() {
        let mut u = attached();
        // An IAD + CDC ACM interface 0 + data interface 1.
        u.log.config_descriptor = vec![
            9, 2, 75, 0, 2, 1, 0, 0xC0, 0xFA, 8, 0x0B, 0, 2, 2, 2, 0, 0, 9, 4, 0, 0, 1, 2, 2, 0, 0,
        ];
        u.script.clear();
        u.xfer = Some(ControlXfer {
            setup: Setup {
                bm_request_type: 0,
                b_request: 9,
                w_value: 1,
                w_index: 0,
                w_length: 0,
                data: Vec::new(),
            },
            acked: true,
            data: Vec::new(),
            out_taken: false,
        });
        u.finish_control();
        let reqs: Vec<u8> = u
            .script
            .iter()
            .filter_map(|s| match s {
                HostStep::Control(s) => Some(s.b_request),
                _ => None,
            })
            .collect();
        assert_eq!(reqs, [0x20, 0x22]);
    }
}
