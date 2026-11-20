// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 USB-OTG (DWC_otg / Synopsys DesignWare USB 2.0 OTG) controller
//! in device mode, with a scripted USB host on the other end of the cable.
//!
//! This is the *second* USB block on the S3: the full-speed Synopsys DWC2
//! OTG core behind TinyUSB's `dwc2` port (arduino-esp32 builds with
//! `ARDUINO_USB_MODE=0`, e.g. the Arduino Nano ESP32), mapped at
//! `0x6008_0000`. It is **distinct** from the [`UsbSerialJtag`] CDC/JTAG
//! bridge at `0x6003_8000`.
//!
//! - Base address: `0x6008_0000`
//! - Modelled window: [`WINDOW_SIZE`] — the CSRs plus the per-endpoint data
//!   FIFO pages (`0x1000 * (n + 1)` for endpoint `n`, DWC2 databook §5.3)
//! - Interrupt source: `ETS_USB_INTR_SOURCE = 38`, level, through the
//!   interrupt matrix.
//!
//! ## Data path: slave (FIFO) mode
//!
//! TinyUSB runs the core without DMA on the ESP32-S3
//! (`CFG_TUD_DWC2_DMA_ENABLE` = 0), so all data moves through the FIFO
//! pages, which is what this model implements:
//!
//! * **OUT and SETUP** — the host side pushes packets into the shared
//!   receive FIFO. Each entry is a status word popped through `GRXSTSP`
//!   (`EPNUM`, `BCNT`, `PKTSTS`) followed by `BCNT` bytes read from the
//!   endpoint-0 FIFO page. `GINTSTS.RXFLVL` is set while the FIFO holds an
//!   entry. A SETUP arrives as `SETUP_RX` (8 bytes) + `SETUP_DONE`; OUT data
//!   as `RX_DATA` + `RX_COMPLETE`, and the core has already counted the
//!   packet out of `DOEPTSIZn` when the entry is visible, as on silicon.
//! * **IN** — firmware sets `DIEPTSIZn` and `DIEPCTLn.EPENA`, then writes the
//!   packet into the endpoint's FIFO page. The host takes one packet
//!   (`min(XFRSIZ, MPS)` bytes) per IN token; when `PKTCNT` reaches 0 the
//!   core clears `EPENA` and raises `DIEPINTn.XFRC`. `DIEPINTn.TXFE` reads 1
//!   whenever the endpoint FIFO is empty, and `DTXFSTSn` reports the free
//!   space, in words.
//! * Endpoint interrupts reach `DAINT` through `DIEPMSK`/`DOEPMSK` (TXFE
//!   through `DIEPEMPMSK`), and `GINTSTS.IEPINT/OEPINT` through `DAINTMSK`.
//!
//! ## The host
//!
//! Once the core is configured (`GAHBCFG.GINTMSK` set, `DCTL.SDIS` clear),
//! the host attaches, waits a debounce, drives a bus reset (`USBRST`, then
//! `ENUMDNE` with `DSTS.ENUMSPD` = full speed 48 MHz) and enumerates the
//! device the way an OS does ([`crate::peripherals::usb_host`]). When a
//! console reader is attached (`labwired test` / `run` capture), it then
//! does what a terminal does when it opens the first CDC-ACM port it finds
//! in the configuration descriptor — `SET_LINE_CODING` 115200 8N1 and
//! `SET_CONTROL_LINE_STATE` DTR|RTS — and reads every bulk IN endpoint into
//! the console. Nothing about the device is assumed: a firmware that never
//! connects, or that describes no CDC interface, is never read.
//!
//! Not modelled: DMA mode, isochronous and interrupt-IN timing (frames are
//! not scheduled; IN tokens are served as soon as data is in the FIFO),
//! data-toggle errors, suspend/resume and remote wakeup, OTG host mode.
//!
//! [`UsbSerialJtag`]: crate::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag

use crate::peripherals::usb_host::{
    after_control, enumeration_script, HostStep, Setup, UsbHostLog,
};
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

const HOST_EVENT_TOKEN: u32 = 1;

/// `ETS_USB_INTR_SOURCE` — DWC OTG core interrupt (interrupt matrix source).
pub const USB_OTG_SOURCE: u32 = 38;

// ── Core Global register offsets (Synopsys DWC2 map; S3 TRM "USB OTG") ──
const GOTGCTL: u64 = 0x000;
const GOTGINT: u64 = 0x004;
const GAHBCFG: u64 = 0x008;
const GUSBCFG: u64 = 0x00C;
const GRSTCTL: u64 = 0x010;
const GINTSTS: u64 = 0x014;
const GINTMSK: u64 = 0x018;
const GRXSTSR: u64 = 0x01C;
const GRXSTSP: u64 = 0x020;
const GRXFSIZ: u64 = 0x024;
const GNPTXFSIZ: u64 = 0x028; // also DIEPTXF0
const GNPTXSTS: u64 = 0x02C;
const GSNPSID: u64 = 0x040;
const GHWCFG1: u64 = 0x044;
const GHWCFG2: u64 = 0x048;
const GHWCFG3: u64 = 0x04C;
const GHWCFG4: u64 = 0x050;
const DIEPTXF1: u64 = 0x104;

// ── Device-mode core register offsets ──
const DCFG: u64 = 0x800;
const DCTL: u64 = 0x804;
const DSTS: u64 = 0x808;
const DIEPMSK: u64 = 0x810;
const DOEPMSK: u64 = 0x814;
const DAINT: u64 = 0x818;
const DAINTMSK: u64 = 0x81C;
const DIEPEMPMSK: u64 = 0x834;
/// IN endpoint `n` registers: `DIEP_BASE + 0x20 * n` + {CTL 0, INT 8,
/// TSIZ 0x10, TXFSTS 0x18}. OUT endpoints: `DOEP_BASE + 0x20 * n` + {CTL 0,
/// INT 8, TSIZ 0x10}.
const DIEP_BASE: u64 = 0x900;
const DOEP_BASE: u64 = 0xB00;
const EP_STRIDE: u64 = 0x20;
/// First data FIFO page (endpoint 0); endpoint `n` is at `FIFO_BASE + 0x1000 * n`.
const FIFO_BASE: u64 = 0x1000;
const FIFO_PAGE: u64 = 0x1000;

/// Device endpoints (GHWCFG2.NUMDEVEPS + 1 = 7 on the S3).
const NUM_EPS: usize = 7;

/// Size of the modelled MMIO window: CSRs + one FIFO page per endpoint.
pub const WINDOW_SIZE: u64 = FIFO_BASE + FIFO_PAGE * NUM_EPS as u64;

// ── GRSTCTL bits ──
const GRSTCTL_CSRST: u32 = 1 << 0; // core soft reset (self-clearing)
const GRSTCTL_RXFFLSH: u32 = 1 << 4;
const GRSTCTL_TXFFLSH: u32 = 1 << 5;
const GRSTCTL_TXFNUM_SHIFT: u32 = 6;
const GRSTCTL_AHBIDLE: u32 = 1 << 31; // AHB master idle (reads 1)

// ── GAHBCFG bits ──
const GAHBCFG_GINTMSK: u32 = 1 << 0; // global interrupt enable

// ── GINTSTS / GINTMSK bits ──
const GINTSTS_RXFLVL: u32 = 1 << 4; // RX FIFO non-empty (RO)
const GINTSTS_NPTXFE: u32 = 1 << 5; // non-periodic TX FIFO empty (RO)
const GINTSTS_GINAKEFF: u32 = 1 << 6; // global IN NAK effective (RO)
const GINTSTS_BOUTNAKEFF: u32 = 1 << 7; // global OUT NAK effective (RO)
const GINTSTS_USBRST: u32 = 1 << 12; // USB reset detected
const GINTSTS_ENUMDONE: u32 = 1 << 13; // enumeration done
const GINTSTS_IEPINT: u32 = 1 << 18; // IN endpoint interrupt (RO)
const GINTSTS_OEPINT: u32 = 1 << 19; // OUT endpoint interrupt (RO)
/// Bits the core derives from other state; writes do not clear them.
const GINTSTS_DERIVED: u32 = GINTSTS_RXFLVL
    | GINTSTS_NPTXFE
    | GINTSTS_GINAKEFF
    | GINTSTS_BOUTNAKEFF
    | GINTSTS_IEPINT
    | GINTSTS_OEPINT;

// ── DCTL bits ──
const DCTL_SDIS: u32 = 1 << 1; // soft disconnect
const DCTL_SGINAK: u32 = 1 << 7;
const DCTL_CGINAK: u32 = 1 << 8;
const DCTL_SGONAK: u32 = 1 << 9;
const DCTL_CGONAK: u32 = 1 << 10;
/// Write-only strobes: never stored.
const DCTL_STROBES: u32 = DCTL_SGINAK | DCTL_CGINAK | DCTL_SGONAK | DCTL_CGONAK;

// ── DIEPCTLn / DOEPCTLn bits ──
const EPCTL_EPENA: u32 = 1 << 31;
const EPCTL_EPDIS: u32 = 1 << 30;
const EPCTL_SNAK: u32 = 1 << 27;
const EPCTL_CNAK: u32 = 1 << 26;
const EPCTL_STALL: u32 = 1 << 21;
const EPCTL_NAKSTS: u32 = 1 << 17;
const EPCTL_TYPE_SHIFT: u32 = 18;
const EPCTL_TYPE_BULK: u32 = 2;

// ── DIEPINTn / DOEPINTn bits ──
const EPINT_XFRC: u32 = 1 << 0;
const EPINT_EPDISD: u32 = 1 << 1;
const DOEPINT_STUP: u32 = 1 << 3;
const DIEPINT_INEPNE: u32 = 1 << 6;
const DIEPINT_TXFE: u32 = 1 << 7; // RO: TX FIFO empty

// ── DIEPTSIZn / DOEPTSIZn fields ──
const TSIZ_XFRSIZ_MASK: u32 = 0x7_FFFF;
const TSIZ_PKTCNT_SHIFT: u32 = 19;
const TSIZ_PKTCNT_MASK: u32 = 0x3FF;

// ── GRXSTSP fields / packet status ──
const PKTSTS_RX_DATA: u32 = 2;
const PKTSTS_RX_COMPLETE: u32 = 3;
const PKTSTS_SETUP_DONE: u32 = 4;
const PKTSTS_SETUP_RX: u32 = 6;

/// Synopsys release id reported by `GSNPSID` ("OT" + 4.00a). TinyUSB's dwc2
/// port asserts the "OT" id and, below 4.20a, polls `GRSTCTL.CSRST` clear.
pub const GSNPSID_VALUE: u32 = 0x4F54_400A;

// Hardware-capability words of the S3 full-speed OTG core. GHWCFG2 carries
// what the driver reads back: NUMDEVEPS[13:10] = 6 (7 endpoints), FS PHY
// type 1 (bits 9:8), no HS PHY (bits 7:6), internal-DMA architecture.
pub const GHWCFG1_VALUE: u32 = 0x0000_0000;
pub const GHWCFG2_VALUE: u32 = 0x224D_D930;
pub const GHWCFG3_VALUE: u32 = 0x0200_1E58;
pub const GHWCFG4_VALUE: u32 = 0xDFF0_5E08;

/// `DSTS.ENUMSPD` (bits 2:1) = 3: full speed, 48 MHz PHY clock.
const DSTS_ENUMSPD_FS48: u32 = 0b11 << 1;

/// Control endpoint max packet size used by every arduino-esp32 build.
const EP0_MPS: usize = 64;

/// Host timing (shortened from the USB 2.0 minimums to save simulated time).
const ATTACH_DEBOUNCE_US: u64 = 2_000;
const BUS_RESET_US: u64 = 1_000;
const HOST_GAP_US: u64 = 20;
const HOST_TIMEOUT_US: u64 = 500_000;
/// While the host waits on the device it looks again this often, as a host
/// controller retries a NAKed token within the frame. The one armed wake is
/// not pulled earlier by MMIO writes, so this bounds the reaction time.
const HOST_POLL_US: u64 = 50;
/// Fallback core clock when the bus does not publish one (S3 = 240 MHz).
const DEFAULT_CPU_HZ: u64 = 240_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for the device to connect (core configured, SDIS clear).
    Detached,
    Debounce {
        until: u64,
    },
    /// Bus reset in progress; ENUMDNE follows at `until`.
    Resetting {
        until: u64,
    },
    /// Reset signalled; waiting for firmware to service USBRST/ENUMDNE.
    AwaitReset {
        started: u64,
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
    /// Script done; only the console reader runs.
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CtlStage {
    /// SETUP queued; waiting for firmware to pop SETUP_DONE.
    Setup,
    Data,
    Status,
}

/// One entry of the shared receive FIFO.
#[derive(Debug, Clone)]
struct RxEntry {
    status: u32,
    data: Vec<u8>,
}

#[derive(Debug, Default, Clone)]
struct InEp {
    ctl: u32,
    int: u32,
    tsiz: u32,
    /// Bytes written into this endpoint's TX FIFO page, as words.
    fifo: VecDeque<u8>,
    /// FIFO depth in words (DIEPTXFn.INEPTXFD), 0 = not allocated.
    depth_words: u32,
}

#[derive(Debug, Default, Clone)]
struct OutEp {
    ctl: u32,
    int: u32,
    tsiz: u32,
}

/// ESP32-S3 DWC_otg USB 2.0 OTG core (full-speed), device mode.
pub struct Esp32s3UsbOtg {
    /// Interrupt-matrix source raised via `explicit_irqs` (= 38).
    source_id: u32,

    gotgctl: u32,
    gotgint: u32,
    gahbcfg: u32,
    gusbcfg: u32,
    /// Latched (W1C) GINTSTS bits; RXFLVL/IEPINT/OEPINT are derived.
    gintsts: u32,
    gintmsk: u32,
    grxfsiz: u32,
    gnptxfsiz: u32,
    dcfg: u32,
    dctl: u32,
    diepmsk: u32,
    doepmsk: u32,
    daintmsk: u32,
    diepempmsk: u32,
    gonak: bool,
    ginak: bool,
    ep_in: [InEp; NUM_EPS],
    ep_out: [OutEp; NUM_EPS],
    /// Shared receive FIFO. Interior mutability: popping `GRXSTSP` and
    /// reading the FIFO page are read side effects, as on silicon.
    rx_fifo: RefCell<VecDeque<RxEntry>>,
    /// Data of the entry last popped through `GRXSTSP`, read out of the FIFO.
    rx_data: RefCell<VecDeque<u8>>,
    /// Upper bytes of a FIFO word read one byte at a time.
    rx_word_latch: Cell<u32>,
    /// Set when firmware pops a SETUP_DONE entry (the SETUP was taken).
    setup_taken: Cell<bool>,

    /// Lossless round-trip store for every other in-window word offset.
    other: HashMap<u64, u32>,

    // ── Host ──
    host_enabled: bool,
    phase: Phase,
    script: VecDeque<HostStep>,
    /// Requests queued once enumeration has run dry (the terminal opening
    /// the CDC port).
    after_enum: VecDeque<HostStep>,
    current: Option<Setup>,
    ctl_buf: Vec<u8>,
    out_sent: usize,
    log: UsbHostLog,
    cdc_opened: bool,
    sink: Option<Arc<Mutex<Vec<u8>>>>,

    // ── Time ──
    clock: Option<CycleClock>,
    ticked: u64,
    cpu_hz: u64,
    /// A host event is armed in the scheduler.
    scheduled: bool,
}

impl std::fmt::Debug for Esp32s3UsbOtg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3UsbOtg(src={}, gintsts=0x{:08x}, gintmsk=0x{:08x}, phase={:?})",
            self.source_id,
            self.gintsts_view(),
            self.gintmsk,
            self.phase,
        )
    }
}

fn tsiz_pktcnt(tsiz: u32) -> u32 {
    (tsiz >> TSIZ_PKTCNT_SHIFT) & TSIZ_PKTCNT_MASK
}

fn tsiz_xfrsiz(tsiz: u32) -> u32 {
    tsiz & TSIZ_XFRSIZ_MASK
}

fn tsiz_set(tsiz: u32, xfrsiz: u32, pktcnt: u32) -> u32 {
    (tsiz & !(TSIZ_XFRSIZ_MASK | (TSIZ_PKTCNT_MASK << TSIZ_PKTCNT_SHIFT)))
        | (xfrsiz & TSIZ_XFRSIZ_MASK)
        | ((pktcnt & TSIZ_PKTCNT_MASK) << TSIZ_PKTCNT_SHIFT)
}

/// Endpoint max packet size from DxEPCTLn (EP0 uses a 2-bit code).
fn ep_mps(ep: usize, ctl: u32) -> usize {
    if ep == 0 {
        match ctl & 3 {
            0 => 64,
            1 => 32,
            2 => 16,
            _ => 8,
        }
    } else {
        (ctl & 0x7FF) as usize
    }
}

impl Esp32s3UsbOtg {
    /// Construct the OTG core. `source_id` is the interrupt-matrix source the
    /// core asserts (the integrator passes `ETS_USB_INTR_SOURCE = 38`).
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            // CONIDSTS(bit16) = B-device, ASESVLD(18) | BSESVLD(19): the
            // board's VBUS is present (bus-powered from the host).
            gotgctl: (1 << 16) | (1 << 18) | (1 << 19),
            gotgint: 0,
            gahbcfg: 0,
            gusbcfg: 0,
            gintsts: 0,
            gintmsk: 0,
            grxfsiz: 0x0000_0100,
            gnptxfsiz: 0x0100_0100,
            dcfg: 0,
            // Soft-disconnected until firmware connects (DCTL reset value).
            dctl: DCTL_SDIS,
            diepmsk: 0,
            doepmsk: 0,
            daintmsk: 0,
            diepempmsk: 0,
            gonak: false,
            ginak: false,
            ep_in: Default::default(),
            ep_out: Default::default(),
            rx_fifo: RefCell::new(VecDeque::new()),
            rx_data: RefCell::new(VecDeque::new()),
            rx_word_latch: Cell::new(0),
            setup_taken: Cell::new(false),
            other: HashMap::new(),
            host_enabled: true,
            phase: Phase::Detached,
            script: enumeration_script(),
            after_enum: VecDeque::new(),
            current: None,
            ctl_buf: Vec::new(),
            out_sent: 0,
            log: UsbHostLog::default(),
            cdc_opened: false,
            sink: None,
            clock: None,
            ticked: 0,
            cpu_hz: DEFAULT_CPU_HZ,
            scheduled: false,
        }
    }

    /// Route bytes the device sends on its bulk IN endpoints (CDC data) to a
    /// console capture sink. With a sink attached the host also opens the
    /// device's first CDC-ACM port after enumeration, as a terminal does.
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>) {
        self.sink = sink;
    }

    /// Disconnect the simulated host (no enumeration will happen).
    pub fn set_host_enabled(&mut self, on: bool) {
        self.host_enabled = on;
    }

    pub fn host_log(&self) -> &UsbHostLog {
        &self.log
    }

    // ── Time ──

    fn now(&self) -> u64 {
        match &self.clock {
            Some(c) => c.now().max(self.ticked),
            None => self.ticked,
        }
    }

    fn us(&self, us: u64) -> u64 {
        (self.cpu_hz / 1_000_000).max(1) * us
    }

    /// The next look while waiting on the device: one poll interval from
    /// `now`, or the `deadline` if that comes first.
    fn poll_until(&self, now: u64, deadline: u64) -> u64 {
        deadline.min(now + self.us(HOST_POLL_US))
    }

    // ── Derived status ──

    fn ep_in_int_view(&self, n: usize) -> u32 {
        let ep = &self.ep_in[n];
        let txfe = if ep.fifo.is_empty() { DIEPINT_TXFE } else { 0 };
        ep.int | txfe
    }

    fn daint(&self) -> u32 {
        let mut v = 0;
        for n in 0..NUM_EPS {
            let mut mask = self.diepmsk & !DIEPINT_TXFE;
            if self.diepempmsk & (1 << n) != 0 {
                mask |= DIEPINT_TXFE;
            }
            if self.ep_in_int_view(n) & mask != 0 {
                v |= 1 << n;
            }
            if self.ep_out[n].int & self.doepmsk != 0 {
                v |= 1 << (16 + n);
            }
        }
        v
    }

    fn gintsts_view(&self) -> u32 {
        let mut v = self.gintsts & !GINTSTS_DERIVED;
        if !self.rx_fifo.borrow().is_empty() {
            v |= GINTSTS_RXFLVL;
        }
        if self.ep_in[0].fifo.is_empty() {
            v |= GINTSTS_NPTXFE;
        }
        if self.ginak {
            v |= GINTSTS_GINAKEFF;
        }
        if self.gonak {
            v |= GINTSTS_BOUTNAKEFF;
        }
        let daint = self.daint() & self.daintmsk;
        if daint & 0xFFFF != 0 {
            v |= GINTSTS_IEPINT;
        }
        if daint >> 16 != 0 {
            v |= GINTSTS_OEPINT;
        }
        v
    }

    fn irq_pending(&self) -> bool {
        self.gahbcfg & GAHBCFG_GINTMSK != 0 && self.gintsts_view() & self.gintmsk != 0
    }

    fn connected(&self) -> bool {
        self.gahbcfg & GAHBCFG_GINTMSK != 0 && self.dctl & DCTL_SDIS == 0
    }

    fn tx_fifo_free_words(&self, n: usize) -> u32 {
        let ep = &self.ep_in[n];
        let depth = if ep.depth_words != 0 {
            ep.depth_words
        } else {
            (EP0_MPS / 4) as u32
        };
        depth.saturating_sub(ep.fifo.len().div_ceil(4) as u32)
    }

    // ── Register file ──

    /// Side-effect-free 32-bit register read.
    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            GOTGCTL => self.gotgctl,
            GOTGINT => self.gotgint,
            GAHBCFG => self.gahbcfg,
            GUSBCFG => self.gusbcfg,
            // Resets and flushes complete at once; AHB master always idle.
            GRSTCTL => GRSTCTL_AHBIDLE,
            GINTSTS => self.gintsts_view(),
            GINTMSK => self.gintmsk,
            GRXSTSR | GRXSTSP => self.rx_fifo.borrow().front().map(|e| e.status).unwrap_or(0),
            GRXFSIZ => self.grxfsiz,
            GNPTXFSIZ => self.gnptxfsiz,
            // EP0 TX FIFO space (words) + 8 free request-queue slots.
            GNPTXSTS => (8 << 16) | self.tx_fifo_free_words(0),
            GSNPSID => GSNPSID_VALUE,
            GHWCFG1 => GHWCFG1_VALUE,
            GHWCFG2 => GHWCFG2_VALUE,
            GHWCFG3 => GHWCFG3_VALUE,
            GHWCFG4 => GHWCFG4_VALUE,
            DCFG => self.dcfg,
            DCTL => self.dctl,
            // Full speed, not suspended; FNSOF[21:8] from the 1 ms frame clock.
            DSTS => {
                let frame = (self.now() / self.us(1000)) as u32 & 0x3FFF;
                DSTS_ENUMSPD_FS48 | (frame << 8)
            }
            DIEPMSK => self.diepmsk,
            DOEPMSK => self.doepmsk,
            DAINT => self.daint(),
            DAINTMSK => self.daintmsk,
            DIEPEMPMSK => self.diepempmsk,
            o if (DIEP_BASE..DIEP_BASE + EP_STRIDE * NUM_EPS as u64).contains(&o) => {
                let n = ((o - DIEP_BASE) / EP_STRIDE) as usize;
                let ep = &self.ep_in[n];
                match (o - DIEP_BASE) % EP_STRIDE {
                    0x00 => {
                        let nak = if ep.ctl & EPCTL_SNAK != 0 {
                            EPCTL_NAKSTS
                        } else {
                            0
                        };
                        (ep.ctl & !(EPCTL_SNAK | EPCTL_CNAK)) | nak
                    }
                    0x08 => self.ep_in_int_view(n),
                    0x10 => ep.tsiz,
                    0x18 => self.tx_fifo_free_words(n),
                    _ => self.other.get(&o).copied().unwrap_or(0),
                }
            }
            o if (DOEP_BASE..DOEP_BASE + EP_STRIDE * NUM_EPS as u64).contains(&o) => {
                let n = ((o - DOEP_BASE) / EP_STRIDE) as usize;
                let ep = &self.ep_out[n];
                match (o - DOEP_BASE) % EP_STRIDE {
                    0x00 => {
                        let nak = if ep.ctl & EPCTL_SNAK != 0 {
                            EPCTL_NAKSTS
                        } else {
                            0
                        };
                        (ep.ctl & !(EPCTL_SNAK | EPCTL_CNAK)) | nak
                    }
                    0x08 => ep.int,
                    0x10 => ep.tsiz,
                    _ => self.other.get(&o).copied().unwrap_or(0),
                }
            }
            _ => self.other.get(&offset).copied().unwrap_or(0),
        }
    }

    /// Pop one word out of the receive FIFO (endpoint-0 FIFO page read).
    fn pop_rx_word(&self) -> u32 {
        let mut data = self.rx_data.borrow_mut();
        let mut w = 0u32;
        for i in 0..4 {
            if let Some(b) = data.pop_front() {
                w |= (b as u32) << (8 * i);
            }
        }
        w
    }

    /// Pop the receive-status word (`GRXSTSP`), exposing its data.
    fn pop_rx_status(&self) -> u32 {
        let Some(entry) = self.rx_fifo.borrow_mut().pop_front() else {
            return 0;
        };
        let pktsts = (entry.status >> 17) & 0xF;
        if pktsts == PKTSTS_SETUP_DONE {
            self.setup_taken.set(true);
        }
        *self.rx_data.borrow_mut() = entry.data.into();
        entry.status
    }

    /// Read with side effects (GRXSTSP pop, FIFO pop). `byte` selects the
    /// byte lane for a byte-wide access (`None` = whole word).
    fn read_access(&self, offset: u64, byte: Option<u64>) -> u32 {
        let word_off = offset & !3;
        let lane = byte.unwrap_or(0);
        if word_off >= FIFO_BASE {
            // Every FIFO page reads the shared receive FIFO.
            if lane == 0 {
                self.rx_word_latch.set(self.pop_rx_word());
            }
            return self.rx_word_latch.get();
        }
        if word_off == GRXSTSP {
            if lane == 0 {
                self.rx_word_latch.set(self.pop_rx_status());
            }
            return self.rx_word_latch.get();
        }
        self.read_word(word_off)
    }

    fn flush_tx(&mut self, txfnum: u32) {
        if txfnum == 0x10 {
            self.ep_in.iter_mut().for_each(|e| e.fifo.clear());
        } else if let Some(ep) = self.ep_in.get_mut(txfnum as usize) {
            ep.fifo.clear();
        }
    }

    fn write_in_ctl(&mut self, n: usize, value: u32) {
        let ep = &mut self.ep_in[n];
        let mut ctl = value & !(EPCTL_SNAK | EPCTL_CNAK | EPCTL_EPDIS | EPCTL_NAKSTS);
        // NAK state is sticky: SNAK sets it, CNAK clears it.
        let mut nak = ep.ctl & EPCTL_SNAK;
        if value & EPCTL_SNAK != 0 {
            nak = EPCTL_SNAK;
            ep.int |= DIEPINT_INEPNE;
        }
        if value & EPCTL_CNAK != 0 {
            nak = 0;
        }
        ctl |= nak;
        if value & EPCTL_EPDIS != 0 && ep.ctl & EPCTL_EPENA != 0 {
            ctl &= !EPCTL_EPENA;
            ep.int |= EPINT_EPDISD;
        }
        ep.ctl = ctl;
    }

    fn write_out_ctl(&mut self, n: usize, value: u32) {
        let ep = &mut self.ep_out[n];
        let mut ctl = value & !(EPCTL_SNAK | EPCTL_CNAK | EPCTL_EPDIS | EPCTL_NAKSTS);
        let mut nak = ep.ctl & EPCTL_SNAK;
        if value & EPCTL_SNAK != 0 {
            nak = EPCTL_SNAK;
        }
        if value & EPCTL_CNAK != 0 {
            nak = 0;
        }
        ctl |= nak;
        if value & EPCTL_EPDIS != 0 && ep.ctl & EPCTL_EPENA != 0 {
            ctl &= !EPCTL_EPENA;
            ep.int |= EPINT_EPDISD;
        }
        ep.ctl = ctl;
    }

    /// 32-bit register write with the per-register semantics.
    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            // VBUS stays present regardless of writes.
            GOTGCTL => self.gotgctl = value | (1 << 16) | (1 << 18) | (1 << 19),
            GOTGINT => self.gotgint &= !value,
            GAHBCFG => self.gahbcfg = value,
            GUSBCFG => self.gusbcfg = value,
            GRSTCTL => {
                if value & GRSTCTL_CSRST != 0 {
                    self.core_soft_reset();
                }
                if value & GRSTCTL_RXFFLSH != 0 {
                    self.rx_fifo.borrow_mut().clear();
                    self.rx_data.borrow_mut().clear();
                }
                if value & GRSTCTL_TXFFLSH != 0 {
                    self.flush_tx((value >> GRSTCTL_TXFNUM_SHIFT) & 0x1F);
                }
            }
            GINTSTS => self.gintsts &= !(value & !GINTSTS_DERIVED),
            GINTMSK => self.gintmsk = value,
            GRXFSIZ => self.grxfsiz = value,
            GNPTXFSIZ => {
                self.gnptxfsiz = value;
                self.ep_in[0].depth_words = value >> 16;
            }
            DCFG => self.dcfg = value,
            DCTL => {
                if value & DCTL_SGONAK != 0 {
                    self.gonak = true;
                }
                if value & DCTL_CGONAK != 0 {
                    self.gonak = false;
                }
                if value & DCTL_SGINAK != 0 {
                    self.ginak = true;
                }
                if value & DCTL_CGINAK != 0 {
                    self.ginak = false;
                }
                self.dctl = value & !DCTL_STROBES;
            }
            DIEPMSK => self.diepmsk = value,
            DOEPMSK => self.doepmsk = value,
            DAINTMSK => self.daintmsk = value,
            DIEPEMPMSK => self.diepempmsk = value,
            GRXSTSR | GRXSTSP | GSNPSID | GHWCFG1 | GHWCFG2 | GHWCFG3 | GHWCFG4 | DSTS | DAINT
            | GNPTXSTS => {}
            o if (DIEPTXF1..DIEPTXF1 + 4 * (NUM_EPS as u64 - 1)).contains(&o) => {
                let n = 1 + ((o - DIEPTXF1) / 4) as usize;
                self.ep_in[n].depth_words = value >> 16;
                self.other.insert(o, value);
            }
            o if (DIEP_BASE..DIEP_BASE + EP_STRIDE * NUM_EPS as u64).contains(&o) => {
                let n = ((o - DIEP_BASE) / EP_STRIDE) as usize;
                match (o - DIEP_BASE) % EP_STRIDE {
                    0x00 => self.write_in_ctl(n, value),
                    // W1C; TXFE is read-only.
                    0x08 => self.ep_in[n].int &= !(value & !DIEPINT_TXFE),
                    0x10 => self.ep_in[n].tsiz = value,
                    0x18 => {}
                    _ => {
                        self.other.insert(o, value);
                    }
                }
            }
            o if (DOEP_BASE..DOEP_BASE + EP_STRIDE * NUM_EPS as u64).contains(&o) => {
                let n = ((o - DOEP_BASE) / EP_STRIDE) as usize;
                match (o - DOEP_BASE) % EP_STRIDE {
                    0x00 => self.write_out_ctl(n, value),
                    0x08 => self.ep_out[n].int &= !value,
                    0x10 => self.ep_out[n].tsiz = value,
                    _ => {
                        self.other.insert(o, value);
                    }
                }
            }
            _ => {
                self.other.insert(offset, value);
            }
        }
    }

    /// `GRSTCTL.CSRST`: clears the interrupt and FIFO state of the core
    /// (the CSRs that configure the PHY and AHB survive, databook §5.3.4).
    fn core_soft_reset(&mut self) {
        self.gintsts = 0;
        self.rx_fifo.borrow_mut().clear();
        self.rx_data.borrow_mut().clear();
        for ep in self.ep_in.iter_mut() {
            ep.fifo.clear();
            ep.ctl &= !EPCTL_EPENA;
            ep.int = 0;
        }
        for ep in self.ep_out.iter_mut() {
            ep.ctl &= !EPCTL_EPENA;
            ep.int = 0;
        }
        self.gonak = false;
        self.ginak = false;
    }

    /// A word (or byte) written into endpoint `n`'s TX FIFO page.
    fn push_tx(&mut self, n: usize, bytes: &[u8]) {
        if let Some(ep) = self.ep_in.get_mut(n) {
            ep.fifo.extend(bytes.iter().copied());
        }
    }

    // ── Host-side bus transactions ──

    /// Bus reset: what the core does on its own when it detects reset.
    fn bus_reset(&mut self) {
        self.gintsts |= GINTSTS_USBRST;
        self.rx_fifo.borrow_mut().clear();
        self.rx_data.borrow_mut().clear();
        for ep in self.ep_in.iter_mut() {
            ep.int = 0;
        }
        for ep in self.ep_out.iter_mut() {
            ep.int = 0;
        }
        self.log.resets += 1;
    }

    /// SETUP token + 8 data bytes to endpoint 0 (always ACKed by the core).
    fn deliver_setup(&mut self, setup: &Setup) {
        // A SETUP clears an EP0 protocol stall (USB 2.0 §8.5.3.4).
        self.ep_in[0].ctl &= !EPCTL_STALL;
        self.ep_out[0].ctl &= !EPCTL_STALL;
        self.setup_taken.set(false);
        let mut rx = self.rx_fifo.borrow_mut();
        rx.push_back(RxEntry {
            status: (PKTSTS_SETUP_RX << 17) | (8 << 4),
            data: setup.bytes().to_vec(),
        });
        rx.push_back(RxEntry {
            status: PKTSTS_SETUP_DONE << 17,
            data: Vec::new(),
        });
        drop(rx);
        self.ep_out[0].int |= DOEPINT_STUP;
    }

    /// IN token to endpoint `n`: the next packet if the endpoint is enabled
    /// and its FIFO holds the whole packet; `None` = NAK.
    fn take_in(&mut self, n: usize) -> Option<Vec<u8>> {
        let ep = &mut self.ep_in[n];
        if ep.ctl & EPCTL_EPENA == 0 || ep.ctl & (EPCTL_SNAK | EPCTL_STALL) != 0 {
            return None;
        }
        let pktcnt = tsiz_pktcnt(ep.tsiz);
        if pktcnt == 0 {
            return None;
        }
        let xfrsiz = tsiz_xfrsiz(ep.tsiz) as usize;
        let len = xfrsiz.min(ep_mps(n, ep.ctl));
        if ep.fifo.len() < len {
            return None;
        }
        let data: Vec<u8> = ep.fifo.drain(..len).collect();
        // The packet was pushed as whole words: drop the padding of the last.
        let pad = (4 - len % 4) % 4;
        let pad = pad.min(ep.fifo.len());
        ep.fifo.drain(..pad);
        ep.tsiz = tsiz_set(ep.tsiz, (xfrsiz - len) as u32, pktcnt - 1);
        if pktcnt - 1 == 0 {
            ep.ctl &= !EPCTL_EPENA;
            ep.int |= EPINT_XFRC;
        }
        Some(data)
    }

    /// OUT token + data (one packet) to endpoint `n`; `None` = NAK.
    fn give_out(&mut self, n: usize, data: &[u8]) -> Option<usize> {
        let ep = &mut self.ep_out[n];
        if ep.ctl & EPCTL_EPENA == 0 || ep.ctl & (EPCTL_SNAK | EPCTL_STALL) != 0 || self.gonak {
            return None;
        }
        let pktcnt = tsiz_pktcnt(ep.tsiz);
        if pktcnt == 0 {
            return None;
        }
        let mps = ep_mps(n, ep.ctl);
        let len = data.len().min(mps);
        let xfrsiz = tsiz_xfrsiz(ep.tsiz) as usize;
        let pktcnt = pktcnt - 1;
        ep.tsiz = tsiz_set(ep.tsiz, xfrsiz.saturating_sub(len) as u32, pktcnt);
        let done = pktcnt == 0 || len < mps;
        if done {
            ep.ctl &= !EPCTL_EPENA;
        }
        let mut rx = self.rx_fifo.borrow_mut();
        rx.push_back(RxEntry {
            status: (PKTSTS_RX_DATA << 17) | ((len as u32) << 4) | n as u32,
            data: data[..len].to_vec(),
        });
        if done {
            rx.push_back(RxEntry {
                status: (PKTSTS_RX_COMPLETE << 17) | n as u32,
                data: Vec::new(),
            });
        }
        drop(rx);
        if done {
            self.ep_out[n].int |= EPINT_XFRC;
        }
        Some(len)
    }

    fn ep0_stalled(&self) -> bool {
        (self.ep_in[0].ctl | self.ep_out[0].ctl) & EPCTL_STALL != 0
    }

    /// The first CDC-ACM communications interface (class 02, subclass 02)
    /// in the configuration descriptor the host read.
    fn cdc_acm_interface(&self) -> Option<u16> {
        let cfg = &self.log.config_descriptor;
        let mut i = 0;
        while i + 1 < cfg.len() && cfg[i] >= 2 {
            let d = &cfg[i..(i + cfg[i] as usize).min(cfg.len())];
            if d[1] == 4 && d.len() >= 9 && d[5] == 0x02 && d[6] == 0x02 {
                return Some(d[2] as u16);
            }
            i += cfg[i] as usize;
        }
        None
    }

    /// What a terminal does when it opens the port: SET_LINE_CODING
    /// 115200 8N1, then SET_CONTROL_LINE_STATE DTR|RTS.
    fn open_cdc_port(&mut self) {
        if self.cdc_opened || self.sink.is_none() {
            return;
        }
        let Some(itf) = self.cdc_acm_interface() else {
            return;
        };
        self.cdc_opened = true;
        self.after_enum.push_back(HostStep::Control(Setup {
            bm_request_type: 0x21,
            b_request: 0x20,
            w_value: 0,
            w_index: itf,
            w_length: 7,
            data: vec![0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08],
        }));
        self.after_enum.push_back(HostStep::Control(Setup {
            bm_request_type: 0x21,
            b_request: 0x22,
            w_value: 0x0003,
            w_index: itf,
            w_length: 0,
            data: Vec::new(),
        }));
    }

    /// The host side of the serial port: read every bulk IN endpoint.
    fn service_console(&mut self) {
        if self.sink.is_none() || !self.cdc_opened {
            return;
        }
        for n in 1..NUM_EPS {
            if (self.ep_in[n].ctl >> EPCTL_TYPE_SHIFT) & 3 != EPCTL_TYPE_BULK {
                continue;
            }
            while let Some(data) = self.take_in(n) {
                if let Some(sink) = &self.sink {
                    if let Ok(mut s) = sink.lock() {
                        s.extend_from_slice(&data);
                    }
                }
            }
        }
    }

    fn finish_control(&mut self, ok: bool) {
        let Some(setup) = self.current.take() else {
            return;
        };
        let data = std::mem::take(&mut self.ctl_buf);
        if ok {
            after_control(&mut self.log, &mut self.script, setup, data);
            if self.script.is_empty() {
                self.open_cdc_port();
            }
        } else {
            self.log.control.push((setup, data, false));
        }
    }

    fn abandon_control(&mut self, why: &str, now: u64) {
        if let Some(s) = &self.current {
            self.log.notes.push(format!(
                "control {:02x} {:02x} {:04x} {why}",
                s.bm_request_type, s.b_request, s.w_value
            ));
        }
        self.finish_control(false);
        self.phase = Phase::Idle {
            until: now + self.us(HOST_GAP_US),
        };
    }

    /// Advance the host as far as it can go now. Returns the cycle of the
    /// next timed step, or `None` when it waits on the device (an MMIO write
    /// wakes it) or has nothing left to do.
    fn host_step(&mut self) -> Option<u64> {
        if !self.host_enabled {
            return None;
        }
        let now = self.now();
        if !self.connected() {
            self.phase = Phase::Detached;
            return None;
        }
        // Several transitions can happen at one instant; bound the loop.
        for _ in 0..16 {
            match self.phase {
                Phase::Detached => {
                    self.phase = Phase::Debounce {
                        until: now + self.us(ATTACH_DEBOUNCE_US),
                    };
                }
                Phase::Debounce { until } | Phase::Resetting { until } if now < until => {
                    return Some(until);
                }
                Phase::Debounce { .. } => {
                    self.bus_reset();
                    self.phase = Phase::Resetting {
                        until: now + self.us(BUS_RESET_US),
                    };
                }
                Phase::Resetting { .. } => {
                    self.gintsts |= GINTSTS_ENUMDONE;
                    self.phase = Phase::AwaitReset { started: now };
                }
                Phase::AwaitReset { started } => {
                    if self.gintsts & (GINTSTS_USBRST | GINTSTS_ENUMDONE) != 0 {
                        if now.saturating_sub(started) > self.us(HOST_TIMEOUT_US) {
                            self.log
                                .notes
                                .push("device never serviced bus reset".into());
                            self.phase = Phase::Done;
                            return None;
                        }
                        return Some(self.poll_until(now, started + self.us(HOST_TIMEOUT_US) + 1));
                    }
                    self.phase = Phase::Idle {
                        until: now + self.us(HOST_GAP_US),
                    };
                }
                Phase::Idle { until } if now < until => return Some(until),
                Phase::Idle { .. } => {
                    let step = self
                        .script
                        .pop_front()
                        .or_else(|| self.after_enum.pop_front());
                    match step {
                        Some(HostStep::Control(setup)) => {
                            self.deliver_setup(&setup);
                            self.current = Some(setup);
                            self.ctl_buf.clear();
                            self.out_sent = 0;
                            self.phase = Phase::Ctl {
                                stage: CtlStage::Setup,
                                started: now,
                            };
                        }
                        Some(HostStep::Wait { us }) => {
                            self.phase = Phase::Idle {
                                until: now + self.us(us),
                            };
                        }
                        Some(other) => {
                            self.log
                                .notes
                                .push(format!("{other:?}: not supported by this host"));
                        }
                        None => {
                            self.phase = Phase::Done;
                            self.service_console();
                            return None;
                        }
                    }
                }
                Phase::Ctl { stage, started } => {
                    let deadline = started + self.us(HOST_TIMEOUT_US);
                    if now > deadline {
                        self.abandon_control(&format!("timed out in {stage:?}"), now);
                        continue;
                    }
                    if stage != CtlStage::Setup && self.ep0_stalled() {
                        self.abandon_control("stalled", now);
                        continue;
                    }
                    let Some(setup) = self.current.clone() else {
                        self.phase = Phase::Idle { until: now };
                        continue;
                    };
                    match stage {
                        CtlStage::Setup => {
                            if !self.setup_taken.get() {
                                return Some(self.poll_until(now, deadline + 1));
                            }
                            self.phase = Phase::Ctl {
                                stage: if setup.w_length == 0 {
                                    CtlStage::Status
                                } else {
                                    CtlStage::Data
                                },
                                started,
                            };
                        }
                        CtlStage::Data if setup.dir_in() => {
                            let Some(chunk) = self.take_in(0) else {
                                return Some(self.poll_until(now, deadline + 1));
                            };
                            let short = chunk.len() < EP0_MPS;
                            self.ctl_buf.extend(chunk);
                            if short || self.ctl_buf.len() >= setup.w_length as usize {
                                self.phase = Phase::Ctl {
                                    stage: CtlStage::Status,
                                    started,
                                };
                            }
                        }
                        CtlStage::Data => {
                            let rest = &setup.data[self.out_sent..];
                            let Some(n) = self.give_out(0, rest) else {
                                return Some(self.poll_until(now, deadline + 1));
                            };
                            self.ctl_buf.extend_from_slice(&rest[..n]);
                            self.out_sent += n;
                            if self.out_sent >= setup.data.len() {
                                self.phase = Phase::Ctl {
                                    stage: CtlStage::Status,
                                    started,
                                };
                            }
                        }
                        CtlStage::Status => {
                            // Opposite direction to the data stage (IN for
                            // no-data requests).
                            let done = if setup.dir_in() && setup.w_length > 0 {
                                self.give_out(0, &[]).is_some()
                            } else {
                                self.take_in(0).is_some()
                            };
                            if !done {
                                return Some(self.poll_until(now, deadline + 1));
                            }
                            self.finish_control(true);
                            self.phase = Phase::Idle {
                                until: now + self.us(HOST_GAP_US),
                            };
                        }
                    }
                }
                Phase::Done => {
                    self.service_console();
                    return None;
                }
            }
        }
        Some(now + self.us(1))
    }

    /// Host work exists that an MMIO access may have unblocked.
    fn host_live(&self) -> bool {
        self.host_enabled && (self.connected() || self.phase != Phase::Detached)
    }

    fn tick_result(&self) -> PeripheralTickResult {
        PeripheralTickResult {
            explicit_irqs: self.irq_pending().then(|| vec![self.source_id]),
            ..PeripheralTickResult::default()
        }
    }
}

impl Peripheral for Esp32s3UsbOtg {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_access(offset, Some(offset & 3));
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_access(offset, None))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset >= FIFO_BASE {
            let n = ((offset - FIFO_BASE) / FIFO_PAGE) as usize;
            self.push_tx(n, &[value]);
            return Ok(());
        }
        let word_off = offset & !3;
        let shift = (offset & 3) * 8;
        // W1C registers must only see this byte's ones; everything else
        // splices the byte into the current value.
        let w1c = matches!(word_off, GINTSTS | GOTGINT)
            || (DIEP_BASE..DOEP_BASE + EP_STRIDE * NUM_EPS as u64).contains(&word_off)
                && word_off % EP_STRIDE == 0x08;
        let word = if w1c || word_off == GRSTCTL {
            (value as u32) << shift
        } else {
            let mut w = self.read_word(word_off);
            w &= !(0xFFu32 << shift);
            w | ((value as u32) << shift)
        };
        self.write_word(word_off, word);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset >= FIFO_BASE {
            let n = ((offset - FIFO_BASE) / FIFO_PAGE) as usize;
            self.push_tx(n, &value.to_le_bytes());
            return Ok(());
        }
        self.write_word(offset & !3, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let word = if offset >= FIFO_BASE {
            0
        } else {
            self.read_word(offset & !3)
        };
        Some(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }

    /// Legacy-walk path (no event scheduler): advance the host clock.
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.ticked = self.ticked.saturating_add(cycles);
        if self.host_live() {
            let _ = self.host_step();
        }
        self.tick_result()
    }

    fn uses_scheduler(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.uses_scheduler()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn attach_cpu_hz(&mut self, hz: u64) {
        if hz > 0 {
            self.cpu_hz = hz;
        }
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.irq_pending() {
            out.push(self.source_id);
        }
    }

    /// After an MMIO write: the device may have connected, taken a SETUP,
    /// armed an endpoint or filled a FIFO, so let the host look again.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.uses_scheduler() || self.scheduled || !self.host_live() {
            return Vec::new();
        }
        self.scheduled = true;
        vec![(self.us(1), HOST_EVENT_TOKEN)]
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        self.scheduled = false;
        let now = self.now();
        let next = self.host_step();
        let mut explicit_irqs = Vec::new();
        self.matrix_irq_sources_into(&mut explicit_irqs);
        let reschedule_delay = next.map(|at| at.saturating_sub(now).max(1));
        self.scheduled = reschedule_delay.is_some();
        crate::sched::EventResult {
            explicit_irqs,
            reschedule_delay,
            ..Default::default()
        }
    }

    fn attach_usb_cdc_console(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>) -> bool {
        self.set_sink(sink);
        true
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        vec![crate::peripheral_log::PeripheralLog::new(
            "host",
            self.log.lines(),
        )]
    }
}

#[cfg(test)]
#[path = "usb_otg_tests.rs"]
mod tests;
