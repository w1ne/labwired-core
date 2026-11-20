// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nordic nRF52 USBD, the full-speed USB device controller (`USBD` at
//! `0x4002_7000`, NVIC line 39; nRF52840 PS v1.x §6.35), with a scripted USB
//! host on the other end of the cable.
//!
//! Endpoint data moves through EasyDMA in two steps, as on silicon. A
//! `STARTEPIN[n]` task copies `EPIN[n].MAXCNT` bytes from `EPIN[n].PTR` into
//! the endpoint buffer (`ENDEPIN[n]`); the next IN token sends it
//! (`EP0DATADONE`, or `EPDATA` with `EPDATASTATUS.EPINn`). An OUT packet
//! lands in the endpoint buffer (`SIZE.EPOUT[n]`, `EP0DATADONE` or `EPDATA`
//! with `EPDATASTATUS.EPOUTn`); a `STARTEPOUT[n]` task copies it to
//! `EPOUT[n].PTR` (`ENDEPOUT[n]`) and the endpoint takes the next packet.
//! Endpoint 0 answers a data stage only after `EP0RCVOUT` (OUT) or a loaded
//! buffer (IN), a status stage only after `EP0STATUS`, and stalls both after
//! `EP0STALL` until the next SETUP. SET_ADDRESS is completed by the
//! controller itself; `USBADDR` changes after its status stage.
//!
//! Start-up follows the PS and two published anomalies:
//!
//! * `ENABLE = 1` starts the core; `EVENTCAUSE.READY` and `USBEVENT` follow a
//!   few microseconds later. Until then every register but `ENABLE` reads 0
//!   and ignores writes.
//! * nRF52840 anomaly 187 ("USBD: USB cannot be enabled"): the core starts
//!   only while the test register `0x4006_ED14` holds 3, which is what the
//!   published workaround writes. The gate applies when the chip descriptor
//!   sets `anomaly_187: true`. A core that did not start keeps
//!   `ENABLE` at 1 through a write of 0 — both read back on an nRF52840-DK
//!   (`hw-oracle/tests/nrf52_full_register_conformance.rs`).
//! * The USB regulator belongs to POWER (`clock`). With VBUS present and the
//!   core started, this model reports `USBREGSTATUS.OUTPUTRDY` and
//!   `EVENTS_USBPWRRDY` there; it reports `VBUSDETECT` and
//!   `EVENTS_USBDETECTED` at power-on. VBUS is the host: `host: true` in the
//!   peripheral config plugs the cable in.
//!
//! The host ([`crate::peripherals::usb_host`]) attaches once the device
//! drives its pull-up (`USBPULLUP`, core started). It resets the bus,
//! enumerates as an OS does, opens a CDC ACM port when the configuration has
//! one and a console reader is attached (SET_LINE_CODING 115200 8N1,
//! SET_CONTROL_LINE_STATE DTR|RTS), then reads every bulk IN endpoint into
//! the console sink. It sends a start of frame every millisecond. Nothing
//! about the device is assumed: what the host learned is in the `host`
//! peripheral log.
//!
//! Model limits, refused with `NotImplemented` where firmware can reach
//! them: the isochronous endpoint (`STARTISOIN`, `STARTISOOUT`), remote
//! wake-up (`DPDMDRIVE`) and `LOWPOWER`. Not modelled and silent:
//! suspend/resume (the host never suspends), VBUS removal, wire-level timing
//! inside a packet, CRC and bit-stuff errors, and the internal test
//! registers at `0x800`/`0x804` and `0xC1C`, which store what firmware
//! writes (anomalies 166 and 199 write them).
//!
//! Assumptions, not taken from the PS: the regulator settles in
//! [`REGULATOR_SETTLE_US`]; an enabled OUT endpoint takes its first packet
//! without a `SIZE.EPOUT[n]` write; a bus reset leaves `EPINEN`/`EPOUTEN`
//! alone (nrfx and TinyUSB both rewrite them).

use crate::peripherals::imxrt::{wake_hint, Timebase};
use crate::peripherals::usb_host::{self, HostStep, Setup, UsbHostLog};
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Bulk/interrupt/control endpoints per direction (endpoint 8 is ISO).
const ENDPOINTS: usize = 8;
/// Endpoint buffer size of every non-ISO endpoint.
const PACKET: usize = 64;

const TASKS_STARTEPIN: u64 = 0x004;
const TASKS_STARTISOIN: u64 = 0x024;
const TASKS_STARTEPOUT: u64 = 0x028;
const TASKS_STARTISOOUT: u64 = 0x048;
const TASKS_EP0RCVOUT: u64 = 0x04C;
const TASKS_EP0STATUS: u64 = 0x050;
const TASKS_EP0STALL: u64 = 0x054;
const TASKS_DPDMDRIVE: u64 = 0x058;
const TASKS_DPDMNODRIVE: u64 = 0x05C;
const EVENTS_BASE: u64 = 0x100;
const EVENTS_END: u64 = 0x160;
const SHORTS: u64 = 0x200;
const INTEN: u64 = 0x300;
const INTENSET: u64 = 0x304;
const INTENCLR: u64 = 0x308;
const EVENTCAUSE: u64 = 0x400;
const HALTED_EPIN: u64 = 0x420;
const HALTED_EPOUT: u64 = 0x444;
const EPSTATUS: u64 = 0x468;
const EPDATASTATUS: u64 = 0x46C;
const USBADDR: u64 = 0x470;
const BMREQUESTTYPE: u64 = 0x480;
const WLENGTHH: u64 = 0x49C;
const SIZE_EPOUT: u64 = 0x4A0;
const SIZE_ISOOUT: u64 = 0x4C0;
const ENABLE: u64 = 0x500;
const USBPULLUP: u64 = 0x504;
const DPDMVALUE: u64 = 0x508;
const DTOGGLE: u64 = 0x50C;
const EPINEN: u64 = 0x510;
const EPOUTEN: u64 = 0x514;
const EPSTALL: u64 = 0x518;
const ISOSPLIT: u64 = 0x51C;
const FRAMECNTR: u64 = 0x520;
const LOWPOWER: u64 = 0x52C;
const ISOINCONFIG: u64 = 0x530;
/// `EPIN[n]` / `ISOIN` and `EPOUT[n]` / `ISOOUT`: PTR, MAXCNT, AMOUNT.
const EPIN_BASE: u64 = 0x600;
const EPOUT_BASE: u64 = 0x700;
const DMA_STRIDE: u64 = 0x14;
/// Internal test registers: an index/data pair and one plain word.
const TEST_INDEX: u64 = 0x800;
const TEST_DATA: u64 = 0x804;
const TEST_C1C: u64 = 0xC1C;

/// Event numbers: the register is `EVENTS_BASE + 4 * n`, the INTEN bit is `n`.
const EV_USBRESET: u32 = 0;
const EV_STARTED: u32 = 1;
const EV_ENDEPIN0: u32 = 2;
const EV_EP0DATADONE: u32 = 10;
const EV_ENDEPOUT0: u32 = 12;
const EV_SOF: u32 = 21;
const EV_USBEVENT: u32 = 22;
const EV_EP0SETUP: u32 = 23;
const EV_EPDATA: u32 = 24;
const EVENT_MASK: u32 = 0x01FF_FFFF;

const SHORT_EP0DATADONE_STARTEPIN0: u32 = 1 << 0;
const SHORT_EP0DATADONE_STARTEPOUT0: u32 = 1 << 1;
const SHORT_EP0DATADONE_EP0STATUS: u32 = 1 << 2;
const SHORT_ENDEPOUT0_EP0STATUS: u32 = 1 << 3;
const SHORT_ENDEPOUT0_EP0RCVOUT: u32 = 1 << 4;

const CAUSE_READY: u32 = 1 << 11;

/// POWER registers this model reports the USB supply through.
const POWER_EVENTS_USBDETECTED: u64 = 0x11C;
const POWER_EVENTS_USBPWRRDY: u64 = 0x124;
const POWER_USBREGSTATUS: u64 = 0x438;
const USBREG_VBUSDETECT: u32 = 1 << 0;
const USBREG_OUTPUTRDY: u32 = 1 << 1;
/// Anomaly 187 test register (outside this window; chip memory region).
const ANOMALY_187_REG: u64 = 0x4006_ED14;

/// ENABLE to `EVENTCAUSE.READY` ("a few us" in the nrfx driver).
const CORE_START_US: u64 = 5;
/// Core start to `USBPWRRDY`. Assumption; the PS gives a worst-case settling
/// time that firmware waits out through the event.
const REGULATOR_SETTLE_US: u64 = 1_000;
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

/// One EasyDMA channel.
#[derive(Debug, Clone, Copy, Default)]
struct Dma {
    ptr: u32,
    maxcnt: u32,
    amount: u32,
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
    /// No data moved: buffer not ready, endpoint disabled or address mismatch.
    Nak,
    Stall,
    /// OUT/SETUP accepted.
    Ack,
    /// IN data.
    Data(Vec<u8>),
}

#[derive(Debug)]
pub struct Nrf52Usbd {
    /// ENABLE as it reads back.
    enable: bool,
    /// The core runs: registers are reachable.
    started: bool,
    /// Cycle at which a written ENABLE starts the core.
    start_at: Option<u64>,
    /// Cycle at which the regulator output is ready.
    regulator_at: Option<u64>,
    regulator_ready: bool,
    /// USBREGSTATUS as last reported to POWER.
    reported_supply: u32,
    pullup: bool,
    /// Bit n = event n.
    events: u32,
    shorts: u32,
    inten: u32,
    eventcause: u32,
    epstatus: u32,
    epdatastatus: u32,
    usbaddr: u8,
    setup: [u8; 8],
    size_out: [u32; ENDPOINTS],
    dpdmvalue: u32,
    dtoggle: u32,
    epinen: u32,
    epouten: u32,
    isosplit: u32,
    isoinconfig: u32,
    frame: u16,
    /// Index 8 is the ISO channel (registers only).
    dma_in: [Dma; ENDPOINTS + 1],
    dma_out: [Dma; ENDPOINTS + 1],
    test_index: u32,
    test: BTreeMap<u32, u32>,
    test_c1c: u32,

    /// Loaded by STARTEPIN, sent by the next IN token.
    in_buf: [Option<Vec<u8>>; ENDPOINTS],
    /// Received from the host, not yet copied out by STARTEPOUT.
    out_buf: [Option<Vec<u8>>; ENDPOINTS],
    halted_in: [bool; ENDPOINTS],
    halted_out: [bool; ENDPOINTS],
    data1_in: [bool; ENDPOINTS],
    data1_out: [bool; ENDPOINTS],
    /// EP0RCVOUT: the next OUT data packet of the control transfer is taken.
    ep0_rcvout: bool,
    /// EP0STATUS: the status stage is answered.
    ep0_status: bool,
    ep0_stall: bool,
    /// Address of a SET_ADDRESS whose status stage has not finished.
    new_addr: Option<u8>,
    /// EasyDMA starts waiting for bus access: (IN direction, endpoint).
    dma_queue: VecDeque<(bool, usize)>,

    power_base: Option<u64>,
    anomaly_187: bool,
    time: Timebase,

    host_enabled: bool,
    phase: Phase,
    /// Cycle at which the host acts next.
    host_at: u64,
    /// Start of the current transfer, for the timeout.
    transfer_start: u64,
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
    /// Bulk IN endpoints of the configuration, read into the console sink.
    console_eps: Vec<usize>,
    log: UsbHostLog,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
}

impl Default for Nrf52Usbd {
    fn default() -> Self {
        Self::new()
    }
}

fn refuse<T>(what: &str) -> SimResult<T> {
    Err(SimulationError::NotImplemented(format!(
        "nrf52_usbd: {what}"
    )))
}

/// `LABWIRED_TRACE_USB=1` logs the host's progress.
fn trace_usb() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LABWIRED_TRACE_USB").is_some())
}

/// `(endpoint, register)` of an address in an `EPIN`/`EPOUT` cluster array.
fn dma_slot(offset: u64, base: u64) -> Option<(usize, u64)> {
    let rel = offset.checked_sub(base)?;
    let n = (rel / DMA_STRIDE) as usize;
    (n <= ENDPOINTS && rel % DMA_STRIDE < 12).then_some((n, rel % DMA_STRIDE))
}

/// Endpoint numbers of the bulk IN endpoints in a configuration descriptor.
fn bulk_in_endpoints(cfg: &[u8]) -> Vec<usize> {
    let mut eps = Vec::new();
    let mut i = 0;
    while i + 1 < cfg.len() && cfg[i] >= 2 {
        let d = &cfg[i..(i + usize::from(cfg[i])).min(cfg.len())];
        if d[1] == 5 && d.len() >= 4 && d[2] & 0x80 != 0 && d[3] & 3 == 2 {
            eps.push(usize::from(d[2] & 0x0F));
        }
        i += usize::from(cfg[i]);
    }
    eps.retain(|ep| (1..ENDPOINTS).contains(ep));
    eps
}

impl Nrf52Usbd {
    pub fn new() -> Self {
        Self {
            enable: false,
            started: false,
            start_at: None,
            regulator_at: None,
            regulator_ready: false,
            reported_supply: 0,
            pullup: false,
            events: 0,
            shorts: 0,
            inten: 0,
            eventcause: 0,
            epstatus: 0,
            epdatastatus: 0,
            usbaddr: 0,
            setup: [0; 8],
            size_out: [0; ENDPOINTS],
            dpdmvalue: 0,
            dtoggle: 0,
            epinen: 1,
            epouten: 1,
            isosplit: 0,
            isoinconfig: 0,
            frame: 0,
            dma_in: [Dma::default(); ENDPOINTS + 1],
            dma_out: [Dma::default(); ENDPOINTS + 1],
            test_index: 0,
            test: BTreeMap::new(),
            test_c1c: 0,
            in_buf: Default::default(),
            out_buf: Default::default(),
            halted_in: [false; ENDPOINTS],
            halted_out: [false; ENDPOINTS],
            data1_in: [false; ENDPOINTS],
            data1_out: [false; ENDPOINTS],
            ep0_rcvout: false,
            ep0_status: false,
            ep0_stall: false,
            new_addr: None,
            dma_queue: VecDeque::new(),
            power_base: None,
            anomaly_187: false,
            time: Timebase::default(),
            host_enabled: false,
            phase: Phase::Detached,
            host_at: 0,
            transfer_start: 0,
            host_addr: 0,
            bus_live: false,
            next_sof: 0,
            script: VecDeque::new(),
            after_enum: VecDeque::new(),
            configured_steps: Vec::new(),
            current: None,
            ctl_buf: Vec::new(),
            cdc_opened: false,
            console_eps: Vec::new(),
            log: UsbHostLog::default(),
            sink: None,
        }
    }

    /// Base of POWER, which shows the USB supply status and events.
    pub fn with_power(mut self, power_base: Option<u64>) -> Self {
        self.power_base = power_base;
        self
    }

    /// Start the core only under the anomaly 187 workaround (module doc).
    pub fn set_anomaly_187(&mut self, on: bool) {
        self.anomaly_187 = on;
    }

    /// Plug the cable in: VBUS is present and a host is on the other end.
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

    /// Registers and the endpoint engine back to their reset state.
    fn reset_registers(&mut self) {
        let keep = std::mem::take(self);
        *self = Self {
            power_base: keep.power_base,
            anomaly_187: keep.anomaly_187,
            time: keep.time,
            host_enabled: keep.host_enabled,
            configured_steps: keep.configured_steps,
            log: keep.log,
            sink: keep.sink,
            ..Self::new()
        };
    }

    fn detach(&mut self) {
        self.phase = Phase::Detached;
        self.host_at = 0;
        self.bus_live = false;
        self.current = None;
    }

    /// The device asks for its pull-up.
    fn pullup(&self) -> bool {
        self.started && self.pullup
    }

    fn irq(&self) -> bool {
        self.started && self.events & self.inten != 0
    }

    fn refresh_irq(&self) {
        self.time.set_level(self.irq());
    }

    fn raise(&mut self, event: u32) {
        self.events |= 1 << event;
    }

    fn setup_in(&self) -> bool {
        self.setup[0] & 0x80 != 0
    }

    fn setup_length(&self) -> u16 {
        u16::from_le_bytes([self.setup[6], self.setup[7]])
    }

    // ── registers ───────────────────────────────────────────────────────────

    fn reg(&self, offset: u64) -> u32 {
        if offset == ENABLE {
            return u32::from(self.enable);
        }
        if !self.started {
            return 0;
        }
        if let Some((n, reg)) = dma_slot(offset, EPIN_BASE) {
            return self.dma_reg(&self.dma_in[n], reg);
        }
        if let Some((n, reg)) = dma_slot(offset, EPOUT_BASE) {
            return self.dma_reg(&self.dma_out[n], reg);
        }
        match offset {
            EVENTS_BASE..=EVENTS_END if offset.is_multiple_of(4) => {
                (self.events >> ((offset - EVENTS_BASE) / 4)) & 1
            }
            SHORTS => self.shorts,
            INTEN | INTENSET | INTENCLR => self.inten,
            EVENTCAUSE => self.eventcause,
            HALTED_EPIN..=0x43C if offset.is_multiple_of(4) => {
                u32::from(self.halted_in[((offset - HALTED_EPIN) / 4) as usize])
            }
            HALTED_EPOUT..=0x460 if offset.is_multiple_of(4) => {
                u32::from(self.halted_out[((offset - HALTED_EPOUT) / 4) as usize])
            }
            EPSTATUS => self.epstatus,
            EPDATASTATUS => self.epdatastatus,
            USBADDR => u32::from(self.usbaddr),
            BMREQUESTTYPE..=WLENGTHH if offset.is_multiple_of(4) => {
                u32::from(self.setup[((offset - BMREQUESTTYPE) / 4) as usize])
            }
            SIZE_EPOUT..=0x4BC if offset.is_multiple_of(4) => {
                self.size_out[((offset - SIZE_EPOUT) / 4) as usize]
            }
            USBPULLUP => u32::from(self.pullup),
            DPDMVALUE => self.dpdmvalue,
            DTOGGLE => {
                let ep = (self.dtoggle & 7) as usize;
                let data1 = if self.dtoggle & 0x80 != 0 {
                    self.data1_in[ep]
                } else {
                    self.data1_out[ep]
                };
                (self.dtoggle & 0x87) | if data1 { 2 << 8 } else { 1 << 8 }
            }
            EPINEN => self.epinen,
            EPOUTEN => self.epouten,
            ISOSPLIT => self.isosplit,
            FRAMECNTR => u32::from(self.frame),
            ISOINCONFIG => self.isoinconfig,
            TEST_INDEX => self.test_index,
            TEST_DATA => self.test.get(&self.test_index).copied().unwrap_or(0),
            TEST_C1C => self.test_c1c,
            // Tasks, EPSTALL (write-only), SIZE.ISOOUT, LOWPOWER (never set).
            0x000..=0x0FC | EPSTALL | SIZE_ISOOUT | LOWPOWER => 0,
            _ => {
                crate::census_reg!("nrf52.usbd:Nrf52Usbd", offset, "read");
                0
            }
        }
    }

    fn dma_reg(&self, dma: &Dma, reg: u64) -> u32 {
        match reg {
            0 => dma.ptr,
            4 => dma.maxcnt,
            _ => dma.amount,
        }
    }

    fn write_enable(&mut self, value: u32) {
        let on = value & 1 != 0;
        if on && !self.enable {
            self.enable = true;
            self.start_at = Some(self.time.now() + self.time.us(CORE_START_US));
        } else if !on && self.started {
            // The core stops: the pull-up goes and the regulator with it.
            self.enable = false;
            self.started = false;
            self.pullup = false;
            self.regulator_at = None;
            self.regulator_ready = false;
            self.detach();
        }
        // A core that never started keeps ENABLE at 1 (module doc).
    }

    fn write_reg(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset == ENABLE {
            self.write_enable(value);
            return Ok(());
        }
        if !self.started {
            return Ok(());
        }
        let trigger = value & 1 != 0;
        if let Some((n, reg)) = dma_slot(offset, EPIN_BASE) {
            Self::write_dma(&mut self.dma_in[n], reg, value);
            return Ok(());
        }
        if let Some((n, reg)) = dma_slot(offset, EPOUT_BASE) {
            Self::write_dma(&mut self.dma_out[n], reg, value);
            return Ok(());
        }
        match offset {
            TASKS_STARTEPIN..=0x020 if offset.is_multiple_of(4) => {
                if trigger {
                    self.dma_queue
                        .push_back((true, ((offset - TASKS_STARTEPIN) / 4) as usize));
                }
            }
            TASKS_STARTEPOUT..=0x044 if offset.is_multiple_of(4) => {
                if trigger {
                    self.dma_queue
                        .push_back((false, ((offset - TASKS_STARTEPOUT) / 4) as usize));
                }
            }
            TASKS_STARTISOIN | TASKS_STARTISOOUT if trigger => {
                return refuse("the isochronous endpoint is not modelled");
            }
            TASKS_EP0RCVOUT if trigger => self.ep0_rcvout = true,
            TASKS_EP0STATUS if trigger => self.ep0_status = true,
            TASKS_EP0STALL if trigger => self.ep0_stall = true,
            TASKS_DPDMDRIVE if trigger => {
                return refuse("remote wake-up (DPDMDRIVE) is not modelled");
            }
            TASKS_STARTISOIN | TASKS_STARTISOOUT | TASKS_EP0RCVOUT | TASKS_EP0STATUS
            | TASKS_EP0STALL | TASKS_DPDMDRIVE | TASKS_DPDMNODRIVE => {}
            // A write of 0 clears an event; a software write of 1 is ignored.
            EVENTS_BASE..=EVENTS_END if offset.is_multiple_of(4) => {
                if value == 0 {
                    self.events &= !(1 << ((offset - EVENTS_BASE) / 4));
                }
            }
            SHORTS => self.shorts = value & 0x1F,
            INTEN => self.inten = value & EVENT_MASK,
            INTENSET => self.inten |= value & EVENT_MASK,
            INTENCLR => self.inten &= !value,
            EVENTCAUSE => self.eventcause &= !value,
            EPSTATUS => self.epstatus &= !value,
            EPDATASTATUS => self.epdatastatus &= !value,
            // Any write releases the packet the endpoint holds.
            SIZE_EPOUT..=0x4BC if offset.is_multiple_of(4) => {
                let n = ((offset - SIZE_EPOUT) / 4) as usize;
                self.out_buf[n] = None;
                self.size_out[n] = 0;
            }
            USBPULLUP => {
                self.pullup = trigger;
                if !trigger {
                    self.detach();
                }
            }
            DPDMVALUE => self.dpdmvalue = value & 0x1F,
            DTOGGLE => {
                self.dtoggle = value & 0x87;
                let ep = (value & 7) as usize;
                let toggles = if value & 0x80 != 0 {
                    &mut self.data1_in
                } else {
                    &mut self.data1_out
                };
                match (value >> 8) & 3 {
                    1 => toggles[ep] = false,
                    2 => toggles[ep] = true,
                    _ => {}
                }
            }
            EPINEN => self.epinen = value & 0x1FF,
            EPOUTEN => self.epouten = value & 0x1FF,
            EPSTALL => {
                let ep = (value & 7) as usize;
                let stall = value & 0x100 != 0;
                if value & 0x80 != 0 {
                    self.halted_in[ep] = stall;
                } else {
                    self.halted_out[ep] = stall;
                }
            }
            ISOSPLIT => self.isosplit = value & 0xFFFF,
            LOWPOWER => {
                if trigger {
                    return refuse("LOWPOWER (suspend) is not modelled");
                }
            }
            ISOINCONFIG => self.isoinconfig = value & 1,
            TEST_INDEX => self.test_index = value,
            TEST_DATA => {
                self.test.insert(self.test_index, value);
            }
            TEST_C1C => self.test_c1c = value,
            _ => {
                crate::census_reg!("nrf52.usbd:Nrf52Usbd", offset, "write");
            }
        }
        Ok(())
    }

    fn write_dma(dma: &mut Dma, reg: u64, value: u32) {
        match reg {
            0 => dma.ptr = value,
            4 => dma.maxcnt = value & 0x7F,
            _ => {}
        }
    }

    // ── EasyDMA ─────────────────────────────────────────────────────────────

    /// STARTEPIN[n]: RAM to the endpoint buffer.
    fn dma_load(&mut self, bus: &mut dyn Bus, n: usize) {
        let dma = self.dma_in[n];
        let count = (dma.maxcnt as usize).min(PACKET);
        let mut data = Vec::with_capacity(count);
        for i in 0..count {
            match bus.read_u8(u64::from(dma.ptr) + i as u64) {
                Ok(b) => data.push(b),
                Err(_) => break,
            }
        }
        self.dma_in[n].amount = data.len() as u32;
        self.in_buf[n] = Some(data);
        self.epstatus |= 1 << n;
        self.raise(EV_STARTED);
        self.raise(EV_ENDEPIN0 + n as u32);
    }

    /// STARTEPOUT[n]: the endpoint buffer to RAM. The endpoint is free for
    /// the next packet afterwards.
    fn dma_store(&mut self, bus: &mut dyn Bus, n: usize) {
        let dma = self.dma_out[n];
        let data = self.out_buf[n].take().unwrap_or_default();
        let count = data.len().min(dma.maxcnt as usize);
        let mut stored = 0;
        crate::with_device_write_bypass(bus, |bus| {
            for (i, b) in data[..count].iter().enumerate() {
                if bus.write_u8(u64::from(dma.ptr) + i as u64, *b).is_err() {
                    break;
                }
                stored += 1;
            }
        });
        self.dma_out[n].amount = stored;
        self.epstatus |= 1 << (16 + n);
        self.raise(EV_STARTED);
        self.raise(EV_ENDEPOUT0 + n as u32);
        if n == 0 {
            if self.shorts & SHORT_ENDEPOUT0_EP0STATUS != 0 {
                self.ep0_status = true;
            }
            if self.shorts & SHORT_ENDEPOUT0_EP0RCVOUT != 0 {
                self.ep0_rcvout = true;
            }
        }
    }

    fn run_dma(&mut self, bus: &mut dyn Bus) {
        while let Some((dir_in, n)) = self.dma_queue.pop_front() {
            if dir_in {
                self.dma_load(bus, n);
            } else {
                self.dma_store(bus, n);
            }
        }
    }

    /// EP0DATADONE and the shortcuts hanging off it.
    fn ep0_data_done(&mut self) {
        self.raise(EV_EP0DATADONE);
        if self.shorts & SHORT_EP0DATADONE_STARTEPIN0 != 0 {
            self.dma_queue.push_back((true, 0));
        }
        if self.shorts & SHORT_EP0DATADONE_STARTEPOUT0 != 0 {
            self.dma_queue.push_back((false, 0));
        }
        if self.shorts & SHORT_EP0DATADONE_EP0STATUS != 0 {
            self.ep0_status = true;
        }
    }

    // ── supply and start-up ─────────────────────────────────────────────────

    /// Start the core once its start time has come, and the regulator after
    /// it; report the USB supply to POWER when it changed.
    fn service_supply(&mut self, bus: &mut dyn Bus) {
        let now = self.time.now();
        if self.start_at.is_some_and(|at| now >= at) {
            self.start_at = None;
            let gated = self.anomaly_187 && bus.read_u32(ANOMALY_187_REG).ok() != Some(3);
            if gated {
                self.log
                    .notes
                    .push("ENABLE without the anomaly 187 sequence: the core did not start".into());
            } else {
                self.started = true;
                self.eventcause |= CAUSE_READY;
                self.raise(EV_USBEVENT);
                if self.host_enabled {
                    self.regulator_at = Some(now + self.time.us(REGULATOR_SETTLE_US));
                }
            }
        }
        if self.regulator_at.is_some_and(|at| now >= at) {
            self.regulator_at = None;
            self.regulator_ready = true;
        }
        let Some(power) = self.power_base else {
            return;
        };
        let supply = self.supply();
        if self.reported_supply == supply {
            return;
        }
        let rose = supply & !self.reported_supply;
        self.reported_supply = supply;
        let _ = bus.write_u32(power + POWER_USBREGSTATUS, supply);
        if rose & USBREG_VBUSDETECT != 0 {
            let _ = bus.write_u32(power + POWER_EVENTS_USBDETECTED, 1);
        }
        if rose & USBREG_OUTPUTRDY != 0 {
            let _ = bus.write_u32(power + POWER_EVENTS_USBPWRRDY, 1);
        }
    }

    /// POWER.USBREGSTATUS as the supply stands now.
    fn supply(&self) -> u32 {
        let mut supply = 0;
        if self.host_enabled {
            supply |= USBREG_VBUSDETECT;
        }
        if self.regulator_ready {
            supply |= USBREG_OUTPUTRDY;
        }
        supply
    }

    fn supply_pending(&self) -> bool {
        self.start_at.is_some()
            || self.regulator_at.is_some()
            || (self.power_base.is_some() && self.reported_supply != self.supply())
    }

    // ── tokens ──────────────────────────────────────────────────────────────

    fn addressed(&self) -> bool {
        self.started && self.usbaddr == self.host_addr
    }

    /// A SETUP token and its 8-byte DATA0 packet on endpoint 0. A SETUP is
    /// always accepted and ends whatever the previous transfer left behind.
    fn token_setup(&mut self, packet: [u8; 8]) -> Answer {
        if !self.addressed() {
            return Answer::Nak;
        }
        self.setup = packet;
        self.ep0_rcvout = false;
        self.ep0_status = false;
        self.ep0_stall = false;
        self.in_buf[0] = None;
        self.out_buf[0] = None;
        self.new_addr = None;
        self.data1_in[0] = true;
        self.data1_out[0] = true;
        // SET_ADDRESS: the controller runs the status stage itself.
        if packet[0] == 0x00 && packet[1] == 5 {
            self.new_addr = Some(packet[2] & 0x7F);
            self.ep0_status = true;
        }
        self.raise(EV_EP0SETUP);
        Answer::Ack
    }

    /// An OUT token and one data packet.
    fn token_out(&mut self, ep: usize, data: &[u8]) -> Answer {
        if !self.addressed() || self.epouten & (1 << ep) == 0 {
            return Answer::Nak;
        }
        if ep == 0 {
            return self.ep0_out(data);
        }
        if self.halted_out[ep] {
            return Answer::Stall;
        }
        if self.out_buf[ep].is_some() {
            return Answer::Nak;
        }
        let n = data.len().min(PACKET);
        self.out_buf[ep] = Some(data[..n].to_vec());
        self.size_out[ep] = n as u32;
        self.data1_out[ep] = !self.data1_out[ep];
        self.epdatastatus |= 1 << (16 + ep);
        self.raise(EV_EPDATA);
        Answer::Ack
    }

    fn ep0_out(&mut self, data: &[u8]) -> Answer {
        if self.ep0_stall || self.halted_out[0] {
            return Answer::Stall;
        }
        if self.setup_in() {
            // Status stage of a device-to-host transfer.
            if !self.ep0_status {
                return Answer::Nak;
            }
            self.ep0_status = false;
            return Answer::Ack;
        }
        if !self.ep0_rcvout || self.out_buf[0].is_some() {
            return Answer::Nak;
        }
        let n = data.len().min(PACKET);
        self.out_buf[0] = Some(data[..n].to_vec());
        self.size_out[0] = n as u32;
        self.ep0_rcvout = false;
        self.data1_out[0] = !self.data1_out[0];
        self.ep0_data_done();
        Answer::Ack
    }

    /// An IN token.
    fn token_in(&mut self, ep: usize) -> Answer {
        if !self.addressed() || self.epinen & (1 << ep) == 0 {
            return Answer::Nak;
        }
        if ep == 0 {
            return self.ep0_in();
        }
        if self.halted_in[ep] {
            return Answer::Stall;
        }
        let Some(data) = self.in_buf[ep].take() else {
            return Answer::Nak;
        };
        self.data1_in[ep] = !self.data1_in[ep];
        self.epdatastatus |= 1 << ep;
        self.raise(EV_EPDATA);
        Answer::Data(data)
    }

    fn ep0_in(&mut self) -> Answer {
        if self.ep0_stall || self.halted_in[0] {
            return Answer::Stall;
        }
        if !self.setup_in() || self.setup_length() == 0 {
            // Status stage of a host-to-device or no-data transfer.
            if !self.ep0_status {
                return Answer::Nak;
            }
            self.ep0_status = false;
            if let Some(addr) = self.new_addr.take() {
                self.usbaddr = addr;
            }
            return Answer::Data(Vec::new());
        }
        match self.in_buf[0].take() {
            Some(data) => {
                self.data1_in[0] = !self.data1_in[0];
                self.ep0_data_done();
                Answer::Data(data)
            }
            // The status stage is open: the device has no more data.
            None if self.ep0_status => Answer::Stall,
            None => Answer::Nak,
        }
    }

    // ── host ────────────────────────────────────────────────────────────────

    /// End of a bus reset: address 0, endpoint 0 back to idle, buffered
    /// packets, stalls and data toggles gone.
    fn end_of_reset(&mut self) {
        self.usbaddr = 0;
        self.frame = 0;
        self.in_buf = Default::default();
        self.out_buf = Default::default();
        self.halted_in = [false; ENDPOINTS];
        self.halted_out = [false; ENDPOINTS];
        self.data1_in = [false; ENDPOINTS];
        self.data1_out = [false; ENDPOINTS];
        self.ep0_rcvout = false;
        self.ep0_status = false;
        self.ep0_stall = false;
        self.new_addr = None;
        self.raise(EV_USBRESET);
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
                "nrf52_usbd: control {:02x?} {why} at cycle {}",
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
                "nrf52_usbd: control {:02x?} done, {} byte(s), at cycle {}",
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
            self.console_eps = bulk_in_endpoints(&self.log.config_descriptor);
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
    fn host_step(&mut self) {
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
                self.phase = Phase::Debounce;
                self.host_at = now + self.time.us(ATTACH_DEBOUNCE_US);
            }
            Phase::Debounce => {
                self.log.resets += 1;
                self.host_addr = 0;
                self.script = usb_host::enumeration_script();
                self.after_enum = self.configured_steps.iter().cloned().collect();
                self.cdc_opened = false;
                self.console_eps.clear();
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
                self.transfer_start = now;
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
                if now.saturating_sub(self.transfer_start) > self.time.us(HOST_TIMEOUT_US) {
                    self.fail_control(setup, &format!("timed out in {stage:?}"));
                    return;
                }
                self.control_stage(setup, stage);
            }
            Phase::Ep => {
                let step = self.current.clone();
                if now.saturating_sub(self.transfer_start) > self.time.us(HOST_TIMEOUT_US) {
                    self.log
                        .notes
                        .push(format!("{step:?} timed out: endpoint never ready"));
                    self.idle_after(HOST_GAP_US);
                    return;
                }
                let answer = match &step {
                    Some(HostStep::Out { ep, data }) => {
                        let answer = self.token_out(usize::from(*ep & 7), data);
                        if answer == Answer::Ack {
                            self.log.outs.push((*ep, data.len()));
                        }
                        answer
                    }
                    Some(HostStep::In { ep, max }) => {
                        let answer = self.token_in(usize::from(*ep & 7));
                        if let Answer::Data(data) = &answer {
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

    fn control_stage(&mut self, setup: Setup, stage: CtlStage) {
        let gap = self.time.us(HOST_GAP_US);
        let now = self.time.now();
        let length = usize::from(setup.w_length);
        match stage {
            CtlStage::Setup => match self.token_setup(setup.bytes()) {
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
            CtlStage::Data if setup.dir_in() => match self.token_in(0) {
                Answer::Data(chunk) => {
                    let short = chunk.len() < PACKET;
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
                let end = (sent + PACKET).min(setup.data.len());
                match self.token_out(0, &setup.data[sent..end]) {
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
                    self.token_out(0, &[])
                } else {
                    self.token_in(0)
                };
                match answer {
                    Answer::Ack | Answer::Data(..) => self.finish_control(setup),
                    Answer::Stall => self.fail_control(setup, "stalled"),
                    Answer::Nak => self.poll_again(),
                }
            }
        }
    }

    /// The port is open and a bulk IN endpoint holds data for the reader.
    fn console_pending(&self) -> bool {
        self.sink.is_some()
            && self.cdc_opened
            && self.phase == Phase::Done
            && self.console_eps.iter().any(|&ep| self.in_buf[ep].is_some())
    }

    /// The host side of the serial port: read every loaded bulk IN endpoint.
    fn service_console(&mut self) {
        if !self.console_pending() {
            return;
        }
        for ep in self.console_eps.clone() {
            if let Answer::Data(data) = self.token_in(ep) {
                if let Some(sink) = &self.sink {
                    if let Ok(mut s) = sink.lock() {
                        s.extend_from_slice(&data);
                    }
                }
            }
        }
    }

    /// Start of frame: FRAMECNTR and the SOF event.
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
        self.raise(EV_SOF);
    }

    fn host_active(&self) -> bool {
        self.host_enabled && self.pullup() && self.phase != Phase::Done
    }
}

impl Peripheral for Nrf52Usbd {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.reg(offset & !3) >> ((offset & 3) * 8)) as u8)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.reg(offset))
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let r = self.write_reg(offset, value);
        self.refresh_irq();
        r
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    /// Walked only while timed work is in flight or the interrupt line is
    /// asserted (so its deassert is reconciled); MMIO re-arms it.
    fn legacy_tick_active(&self) -> bool {
        self.host_active() || self.bus_live || self.supply_pending() || self.time.level()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.time.advance(cycles);
        self.service_frames();
        self.refresh_irq();
        let until = [
            self.host_active().then_some(self.host_at),
            self.bus_live.then_some(self.next_sof),
            self.start_at,
            self.regulator_at,
        ]
        .into_iter()
        .flatten()
        .min();
        wake_hint(self.time.now(), until)
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.time.level())
    }
    fn needs_bus_tick(&self) -> bool {
        // The bus caches this answer between MMIO accesses, so it cannot
        // depend on time: the services return early until they are due.
        self.host_active()
            || self.console_pending()
            || self.supply_pending()
            || !self.dma_queue.is_empty()
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.service_supply(bus);
        self.run_dma(bus);
        if self.host_enabled {
            self.host_step();
            // A token may have started EasyDMA through a shortcut.
            self.run_dma(bus);
            self.service_console();
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
            "peripheral": "nrf52_usbd",
            "enable": self.enable,
            "started": self.started,
            "pullup": self.pullup,
            "usbaddr": self.usbaddr,
            "frame": self.frame,
            "events": self.events,
            "inten": self.inten,
            "epinen": self.epinen,
            "epouten": self.epouten,
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

    const POWER: u64 = 0x4000_0000;
    const BUF_IN: u32 = 0x2000_0100;
    const BUF_OUT: u32 = 0x2000_0200;
    const CPU_MHZ: u64 = 64;

    /// RAM at 0x2000_0000, POWER and the anomaly 187 test register.
    struct Memory {
        bytes: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl Memory {
        fn new() -> Self {
            Self {
                bytes: HashMap::new(),
                config: SimulationConfig::default(),
            }
        }
        fn word(&mut self, address: u64, value: u32) {
            for (i, b) in value.to_le_bytes().into_iter().enumerate() {
                self.bytes.insert(address + i as u64, b);
            }
        }
        fn fill(&mut self, address: u32, data: &[u8]) {
            for (i, b) in data.iter().enumerate() {
                self.bytes.insert(u64::from(address) + i as u64, *b);
            }
        }
        fn bytes_at(&self, address: u32, n: usize) -> Vec<u8> {
            (0..n)
                .map(|i| self.read_u8(u64::from(address) + i as u64).unwrap())
                .collect()
        }
    }

    impl Bus for Memory {
        fn read_u8(&self, address: u64) -> SimResult<u8> {
            Ok(self.bytes.get(&address).copied().unwrap_or(0))
        }
        fn write_u8(&mut self, address: u64, value: u8) -> SimResult<()> {
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

    /// A plugged-in controller under the anomaly 187 gate, not yet enabled.
    fn plugged() -> (Nrf52Usbd, Memory, CycleClock) {
        let mut u = Nrf52Usbd::new().with_power(Some(POWER));
        u.set_anomaly_187(true);
        u.set_host_enabled(true);
        let clock = CycleClock::default();
        u.attach_cycle_clock(clock.clone());
        u.attach_cpu_hz(CPU_MHZ * 1_000_000);
        (u, Memory::new(), clock)
    }

    /// Run until `done` or `us` microseconds pass.
    fn run(
        u: &mut Nrf52Usbd,
        m: &mut Memory,
        clock: &CycleClock,
        us: u64,
        mut done: impl FnMut(&Nrf52Usbd) -> bool,
    ) -> bool {
        let end = clock.now() + CPU_MHZ * us;
        while clock.now() < end {
            if done(u) {
                return true;
            }
            clock.publish(clock.now() + CPU_MHZ);
            u.tick_elapsed(0);
            if u.needs_bus_tick() {
                u.tick_with_bus(m);
            }
        }
        done(u)
    }

    /// Enable the core the way the published workaround does.
    fn start(u: &mut Nrf52Usbd, m: &mut Memory, clock: &CycleClock) {
        m.word(ANOMALY_187_REG, 3);
        u.write_u32(ENABLE, 1).unwrap();
        assert!(run(u, m, clock, 100, |u| u.started));
        m.word(ANOMALY_187_REG, 0);
    }

    /// A started core the host has reset, with `script` as the host's plan.
    fn attached(script: Vec<HostStep>) -> (Nrf52Usbd, Memory, CycleClock) {
        let (mut u, mut m, clock) = plugged();
        start(&mut u, &mut m, &clock);
        u.write_u32(USBPULLUP, 1).unwrap();
        assert!(run(&mut u, &mut m, &clock, 5_000, |u| u.bus_live));
        u.script = VecDeque::from(script);
        (u, m, clock)
    }

    fn event(u: &Nrf52Usbd, n: u32) -> bool {
        u.read_u32(EVENTS_BASE + 4 * u64::from(n)).unwrap() == 1
    }

    fn clear(u: &mut Nrf52Usbd, n: u32) {
        u.write_u32(EVENTS_BASE + 4 * u64::from(n), 0).unwrap();
    }

    fn load_in(u: &mut Nrf52Usbd, m: &mut Memory, ep: u64, data: &[u8]) {
        m.fill(BUF_IN, data);
        u.write_u32(EPIN_BASE + ep * DMA_STRIDE, BUF_IN).unwrap();
        u.write_u32(EPIN_BASE + ep * DMA_STRIDE + 4, data.len() as u32)
            .unwrap();
        u.write_u32(TASKS_STARTEPIN + 4 * ep, 1).unwrap();
        u.tick_with_bus(m);
    }

    fn get_descriptor(len: u16) -> HostStep {
        HostStep::Control(Setup::get_descriptor(1, 0, 0, len))
    }

    #[test]
    fn a_core_enabled_without_the_anomaly_187_sequence_matches_the_bench() {
        let (mut u, mut m, clock) = plugged();
        u.write_u32(ENABLE, 1).unwrap();
        assert!(!run(&mut u, &mut m, &clock, 200, |u| u.started));
        assert_eq!(u.read_u32(ENABLE).unwrap(), 1);
        // ENABLE = 0 stays 1; USBPULLUP, EPINEN and EPOUTEN read 0; a
        // software write of 1 to an event is ignored.
        u.write_u32(ENABLE, 0).unwrap();
        assert_eq!(u.read_u32(ENABLE).unwrap(), 1);
        for reg in [USBPULLUP, EPINEN, EPOUTEN, EVENTS_BASE, 0x158, 0x15C] {
            u.write_u32(reg, 1).unwrap();
            assert_eq!(u.read_u32(reg).unwrap(), 0, "register {reg:#x}");
        }
        assert_eq!(u.read_u32(EVENTCAUSE).unwrap(), 0);
        assert!(u.log.notes[0].contains("anomaly 187"));
    }

    #[test]
    fn enable_under_the_workaround_starts_the_core_and_flags_ready() {
        let (mut u, mut m, clock) = plugged();
        start(&mut u, &mut m, &clock);
        assert_eq!(u.read_u32(EVENTCAUSE).unwrap(), CAUSE_READY);
        assert!(event(&u, EV_USBEVENT));
        u.write_u32(EVENTCAUSE, CAUSE_READY).unwrap();
        assert_eq!(u.read_u32(EVENTCAUSE).unwrap(), 0);
        // Endpoint 0 is enabled at reset; the registers hold what is written.
        assert_eq!(u.read_u32(EPINEN).unwrap(), 1);
        u.write_u32(EPINEN, 0x07).unwrap();
        assert_eq!(u.read_u32(EPINEN).unwrap(), 0x07);
        // Without the gate (a chip that does not set it) ENABLE is enough.
        let mut plain = Nrf52Usbd::new();
        plain.attach_cpu_hz(CPU_MHZ * 1_000_000);
        plain.write_u32(ENABLE, 1).unwrap();
        plain.tick_elapsed(CPU_MHZ * 2 * CORE_START_US);
        plain.tick_with_bus(&mut m);
        assert!(plain.started);
    }

    #[test]
    fn the_supply_is_reported_to_power_vbus_first_regulator_after_the_core() {
        let (mut u, mut m, clock) = plugged();
        assert!(u.needs_bus_tick());
        u.tick_with_bus(&mut m);
        assert_eq!(m.read_u32(POWER + POWER_USBREGSTATUS).unwrap(), 1);
        assert_eq!(m.read_u32(POWER + POWER_EVENTS_USBDETECTED).unwrap(), 1);
        assert_eq!(m.read_u32(POWER + POWER_EVENTS_USBPWRRDY).unwrap(), 0);
        assert!(!u.needs_bus_tick());

        start(&mut u, &mut m, &clock);
        assert_eq!(m.read_u32(POWER + POWER_USBREGSTATUS).unwrap(), 1);
        assert!(run(&mut u, &mut m, &clock, 2_000, |u| u.regulator_ready));
        u.tick_with_bus(&mut m);
        assert_eq!(m.read_u32(POWER + POWER_USBREGSTATUS).unwrap(), 3);
        assert_eq!(m.read_u32(POWER + POWER_EVENTS_USBPWRRDY).unwrap(), 1);

        // ENABLE = 0 on a started core turns the regulator off again.
        u.write_u32(ENABLE, 0).unwrap();
        assert_eq!(u.read_u32(ENABLE).unwrap(), 0);
        u.tick_with_bus(&mut m);
        assert_eq!(m.read_u32(POWER + POWER_USBREGSTATUS).unwrap(), 1);

        // No cable: no VBUS, no regulator, no host.
        let mut bare = Nrf52Usbd::new().with_power(Some(POWER));
        let mut m = Memory::new();
        assert!(!bare.needs_bus_tick());
        bare.write_u32(ENABLE, 1).unwrap();
        bare.tick_elapsed(1_000_000);
        bare.tick_with_bus(&mut m);
        bare.write_u32(USBPULLUP, 1).unwrap();
        bare.tick_elapsed(1_000_000);
        bare.tick_with_bus(&mut m);
        assert!(bare.started && !bare.regulator_ready && !bare.bus_live);
        assert_eq!(m.read_u32(POWER + POWER_USBREGSTATUS).unwrap(), 0);
    }

    #[test]
    fn the_host_waits_for_the_pull_up_then_resets_the_bus() {
        let (mut u, mut m, clock) = plugged();
        start(&mut u, &mut m, &clock);
        assert!(!run(&mut u, &mut m, &clock, 5_000, |u| u.bus_live));
        assert_eq!(u.log.resets, 0);
        u.write_u32(USBPULLUP, 1).unwrap();
        assert!(run(&mut u, &mut m, &clock, 5_000, |u| u.bus_live));
        assert_eq!(u.log.resets, 1);
        assert!(event(&u, EV_USBRESET));
        // The interrupt line follows INTEN.
        assert!(!u.irq());
        u.write_u32(INTENSET, 1 << EV_USBRESET).unwrap();
        assert!(u.irq_line_level().unwrap());
        clear(&mut u, EV_USBRESET);
        assert!(!u.irq_line_level().unwrap());
        // Frames run: SOF and FRAMECNTR.
        assert!(run(&mut u, &mut m, &clock, 3_000, |u| u.frame >= 2));
        assert!(event(&u, EV_SOF));
        // Dropping the pull-up detaches.
        u.write_u32(USBPULLUP, 0).unwrap();
        assert!(!u.bus_live);
    }

    #[test]
    fn a_control_read_moves_through_easydma_and_waits_for_ep0status() {
        let (mut u, mut m, clock) = attached(vec![get_descriptor(18)]);
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0SETUP
        )));
        let setup: Vec<u32> = (0..8)
            .map(|i| u.read_u32(BMREQUESTTYPE + 4 * i).unwrap())
            .collect();
        assert_eq!(setup, [0x80, 6, 0, 1, 0, 0, 18, 0]);
        // Nothing loaded: the data stage is NAKed.
        assert!(!run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0DATADONE
        )));

        let descriptor: Vec<u8> = (1..=18).collect();
        load_in(&mut u, &mut m, 0, &descriptor);
        assert!(event(&u, EV_ENDEPIN0) && event(&u, EV_STARTED));
        assert_eq!(u.read_u32(EPSTATUS).unwrap(), 1);
        assert_eq!(u.read_u32(EPIN_BASE + 8).unwrap(), 18, "AMOUNT");
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0DATADONE
        )));

        // The status stage waits for EP0STATUS.
        assert!(!run(&mut u, &mut m, &clock, 1_000, |u| !u
            .log
            .control
            .is_empty()));
        u.write_u32(TASKS_EP0STATUS, 1).unwrap();
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| !u
            .log
            .control
            .is_empty()));
        let (_, data, completed) = &u.log.control[0];
        assert!(*completed);
        assert_eq!(data, &descriptor);
    }

    #[test]
    fn the_ep0datadone_shortcut_opens_the_status_stage() {
        let (mut u, mut m, clock) = attached(vec![get_descriptor(64)]);
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0SETUP
        )));
        u.write_u32(SHORTS, SHORT_EP0DATADONE_EP0STATUS).unwrap();
        load_in(&mut u, &mut m, 0, &[9, 8, 7]);
        assert!(run(&mut u, &mut m, &clock, 2_000, |u| !u
            .log
            .control
            .is_empty()));
        assert_eq!(u.log.control[0].1, [9, 8, 7]);
    }

    #[test]
    fn a_control_write_needs_ep0rcvout_and_lands_in_ram_through_startepout() {
        let coding = vec![0x00, 0xC2, 0x01, 0x00, 0, 0, 8];
        let (mut u, mut m, clock) = attached(vec![HostStep::Control(Setup {
            bm_request_type: 0x21,
            b_request: 0x20,
            w_value: 0,
            w_index: 0,
            w_length: 7,
            data: coding.clone(),
        })]);
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0SETUP
        )));
        // Without EP0RCVOUT the data packet is NAKed.
        assert!(!run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0DATADONE
        )));
        u.write_u32(TASKS_EP0RCVOUT, 1).unwrap();
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0DATADONE
        )));
        assert_eq!(u.read_u32(SIZE_EPOUT).unwrap(), 7);

        u.write_u32(EPOUT_BASE, BUF_OUT).unwrap();
        u.write_u32(EPOUT_BASE + 4, 64).unwrap();
        u.write_u32(TASKS_STARTEPOUT, 1).unwrap();
        u.tick_with_bus(&mut m);
        assert!(event(&u, EV_ENDEPOUT0));
        assert_eq!(u.read_u32(EPSTATUS).unwrap(), 1 << 16);
        assert_eq!(u.read_u32(EPOUT_BASE + 8).unwrap(), 7, "AMOUNT");
        assert_eq!(m.bytes_at(BUF_OUT, 7), coding);

        u.write_u32(TASKS_EP0STATUS, 1).unwrap();
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| !u
            .log
            .control
            .is_empty()));
        assert!(u.log.control[0].2);
    }

    #[test]
    fn set_address_completes_without_firmware_and_moves_usbaddr_after_status() {
        let (mut u, mut m, clock) = attached(vec![HostStep::Control(Setup {
            bm_request_type: 0,
            b_request: 5,
            w_value: 5,
            w_index: 0,
            w_length: 0,
            data: Vec::new(),
        })]);
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0SETUP
        )));
        assert_eq!(u.read_u32(USBADDR).unwrap(), 0);
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| u.log.address == Some(5)));
        assert_eq!(u.read_u32(USBADDR).unwrap(), 5);
        // The host talks to address 5 from here on.
        u.script.push_back(get_descriptor(18));
        u.phase = Phase::Idle;
        clear(&mut u, EV_EP0SETUP);
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0SETUP
        )));
    }

    #[test]
    fn ep0stall_stalls_the_transfer_and_the_next_setup_clears_it() {
        let (mut u, mut m, clock) = attached(vec![get_descriptor(18), get_descriptor(18)]);
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0SETUP
        )));
        clear(&mut u, EV_EP0SETUP);
        u.write_u32(TASKS_EP0STALL, 1).unwrap();
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| !u
            .log
            .control
            .is_empty()));
        assert!(!u.log.control[0].2);
        assert!(u.log.notes.iter().any(|n| n.contains("stalled")));
        assert!(run(&mut u, &mut m, &clock, 1_000, |u| event(
            u,
            EV_EP0SETUP
        )));
        assert!(!u.ep0_stall);
    }

    #[test]
    fn a_bulk_in_packet_goes_out_once_and_reports_epdata() {
        let (mut u, mut m, clock) = attached(vec![
            HostStep::In { ep: 2, max: 64 },
            HostStep::In { ep: 2, max: 64 },
        ]);
        // A disabled endpoint does not answer.
        load_in(&mut u, &mut m, 2, b"hello");
        assert!(!run(&mut u, &mut m, &clock, 500, |u| event(u, EV_EPDATA)));
        u.write_u32(EPINEN, 0x05).unwrap();
        assert!(run(&mut u, &mut m, &clock, 500, |u| event(u, EV_EPDATA)));
        assert_eq!(u.read_u32(EPDATASTATUS).unwrap(), 1 << 2);
        u.write_u32(EPDATASTATUS, 1 << 2).unwrap();
        assert_eq!(u.read_u32(EPDATASTATUS).unwrap(), 0);
        assert_eq!(u.log.ins, [(2, b"hello".to_vec())]);
        // The buffer is empty again: the second IN is NAKed.
        assert!(!run(&mut u, &mut m, &clock, 500, |u| u.log.ins.len() > 1));
    }

    #[test]
    fn a_bulk_out_endpoint_holds_one_packet_until_startepout_or_a_size_write() {
        let (mut u, mut m, clock) = attached(vec![
            HostStep::Out {
                ep: 1,
                data: b"abc".to_vec(),
            },
            HostStep::Out {
                ep: 1,
                data: b"de".to_vec(),
            },
            HostStep::Out {
                ep: 1,
                data: b"f".to_vec(),
            },
        ]);
        u.write_u32(EPOUTEN, 0x03).unwrap();
        assert!(run(&mut u, &mut m, &clock, 500, |u| u.log.outs.len() == 1));
        assert!(event(&u, EV_EPDATA));
        assert_eq!(u.read_u32(EPDATASTATUS).unwrap(), 1 << 17);
        assert_eq!(u.read_u32(SIZE_EPOUT + 4).unwrap(), 3);
        // The second packet is NAKed while the first is held.
        assert!(!run(&mut u, &mut m, &clock, 500, |u| u.log.outs.len() == 2));

        u.write_u32(EPOUT_BASE + DMA_STRIDE, BUF_OUT).unwrap();
        u.write_u32(EPOUT_BASE + DMA_STRIDE + 4, 64).unwrap();
        u.write_u32(TASKS_STARTEPOUT + 4, 1).unwrap();
        u.tick_with_bus(&mut m);
        assert!(event(&u, EV_ENDEPOUT0 + 1));
        assert_eq!(m.bytes_at(BUF_OUT, 3), b"abc");
        assert!(run(&mut u, &mut m, &clock, 500, |u| u.log.outs.len() == 2));
        // A SIZE.EPOUT write drops the held packet.
        u.write_u32(SIZE_EPOUT + 4, 0).unwrap();
        assert!(run(&mut u, &mut m, &clock, 500, |u| u.log.outs.len() == 3));
        assert_eq!(u.read_u32(SIZE_EPOUT + 4).unwrap(), 1);
    }

    #[test]
    fn epstall_halts_an_endpoint_and_dtoggle_reads_the_selected_toggle() {
        let (mut u, mut m, clock) = attached(vec![HostStep::In { ep: 1, max: 64 }]);
        u.write_u32(EPINEN, 0x03).unwrap();
        u.write_u32(EPSTALL, 0x100 | 0x80 | 1).unwrap();
        assert_eq!(u.read_u32(HALTED_EPIN + 4).unwrap(), 1);
        assert!(run(&mut u, &mut m, &clock, 500, |u| !u
            .log
            .notes
            .is_empty()));
        assert!(u.log.notes[0].contains("stalled"));
        u.write_u32(EPSTALL, 0x80 | 1).unwrap();
        assert_eq!(u.read_u32(HALTED_EPIN + 4).unwrap(), 0);

        u.write_u32(DTOGGLE, (2 << 8) | 0x80 | 1).unwrap();
        assert_eq!(u.read_u32(DTOGGLE).unwrap(), (2 << 8) | 0x81);
        u.write_u32(DTOGGLE, (1 << 8) | 0x80 | 1).unwrap();
        assert_eq!(u.read_u32(DTOGGLE).unwrap(), (1 << 8) | 0x81);
    }

    #[test]
    fn the_unmodelled_parts_are_refused_and_the_test_registers_store() {
        let (mut u, mut m, clock) = plugged();
        start(&mut u, &mut m, &clock);
        for task in [
            TASKS_STARTISOIN,
            TASKS_STARTISOOUT,
            TASKS_DPDMDRIVE,
            LOWPOWER,
        ] {
            assert!(u.write_u32(task, 1).is_err(), "register {task:#x}");
        }
        u.write_u32(TEST_INDEX, 0x7E3).unwrap();
        u.write_u32(TEST_DATA, 0x40).unwrap();
        u.write_u32(TEST_INDEX, 0x7B4).unwrap();
        assert_eq!(u.read_u32(TEST_DATA).unwrap(), 0);
        u.write_u32(TEST_INDEX, 0x7E3).unwrap();
        assert_eq!(u.read_u32(TEST_DATA).unwrap(), 0x40);
        u.write_u32(TEST_C1C, 0x82).unwrap();
        assert_eq!(u.read_u32(TEST_C1C).unwrap(), 0x82);
    }

    #[test]
    fn bulk_in_endpoints_come_from_the_configuration_descriptor() {
        // config, CDC ACM interface with interrupt IN 0x81, CDC data
        // interface with bulk IN 0x82 and bulk OUT 0x02.
        let cfg = [
            9, 2, 39, 0, 2, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 2, 2, 0, 0, 7, 5, 0x81, 3, 64, 0, 16, 9,
            4, 1, 0, 2, 0x0A, 0, 0, 0, 7, 5, 0x82, 2, 64, 0, 0, 7, 5, 0x02, 2, 64, 0, 0,
        ];
        assert_eq!(bulk_in_endpoints(&cfg), [2]);
    }
}
