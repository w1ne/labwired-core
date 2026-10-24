//! DW_apb_uart, reg-shift 2. Immediate functional TX, bounded 16-byte RX FIFO.
//! DMA, baud/modem timing, fractional divisor and busy-detect are unsupported;
//! UCV/CPR/DLF report zero so Linux sees no additional DW capabilities.
use super::gic::Gic;
use crate::bus::bus_trace::{BusDir, BusPayload, BusTrace};
use crate::peripherals::device::{UartStreamDevice, UartStreamHost};
use crate::{Peripheral, SimResult, SimulationError};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
// Fixed functional receiver timeout; not a physical baud/timing model.
const RX_TIMEOUT_US: u64 = 4000;
const FIFO: usize = 16;
const CONSOLE: usize = 64 * 1024;
#[derive(Default)]
struct State {
    exclusive: bool,
    peer_epoch: Option<u64>,
    rx_age_us: u64,
    rx_signature: u64,
    dll: u8,
    dlm: u8,
    lcr: u8,
    mcr: u8,
    scr: u8,
    ier: u8,
    fcr: u8,
    thre: bool,
    overrun: bool,
    modem_delta: u8,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo: bool,
    streams: Vec<Box<dyn UartStreamDevice>>,
}
pub struct DwUart {
    state: Mutex<State>,
    rx: Arc<Mutex<VecDeque<u8>>>,
    gic: Arc<Mutex<Gic>>,
    intid: usize,
    trace: BusTrace,
    trace_name: String,
}
impl std::fmt::Debug for DwUart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DwUart")
            .field("intid", &self.intid)
            .finish_non_exhaustive()
    }
}
impl DwUart {
    pub fn new(gic: Arc<Mutex<Gic>>, intid: usize) -> Self {
        Self {
            state: Mutex::new(State::default()),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            gic,
            intid,
            trace: crate::bus::bus_trace::new_log(),
            trace_name: String::new(),
        }
    }
    fn capacity(s: &State) -> usize {
        if s.fcr & 1 != 0 {
            FIFO
        } else {
            1
        }
    }
    fn push_rx(&self, s: &mut State, v: u8) {
        let mut rx = self.rx.lock().unwrap();
        if rx.len() < Self::capacity(s) {
            self.trace.push(
                &self.trace_name,
                BusPayload::Uart {
                    direction: BusDir::Rx,
                    byte: v,
                },
            );
            rx.push_back(v);
            s.rx_age_us = 0;
            s.rx_signature = Self::signature(&rx);
        } else {
            s.overrun = true;
        }
    }
    fn modem(s: &State) -> u8 {
        if s.mcr & 16 == 0 {
            0
        } else {
            ((s.mcr & 2) << 3) | ((s.mcr & 1) << 5) | ((s.mcr & 4) << 4) | ((s.mcr & 8) << 4)
        }
    }
    fn signature(rx: &VecDeque<u8>) -> u64 {
        rx.iter().fold(rx.len() as u64, |h, b| {
            h.wrapping_mul(257).wrapping_add(*b as u64 + 1)
        })
    }
    fn stale_epoch(s: &State) -> bool {
        s.exclusive && s.streams.first().and_then(|p| p.stream_generation()) != s.peer_epoch
    }
    fn reconcile_rx(&self, s: &mut State) {
        if Self::stale_epoch(s) {
            self.rx.lock().unwrap().clear();
            s.rx_signature = 0;
            s.rx_age_us = 0;
            s.overrun = false;
            s.peer_epoch = s.streams.first().and_then(|p| p.stream_generation());
        }
        let mut rx = self.rx.lock().unwrap();
        if rx.len() > Self::capacity(s) {
            rx.truncate(Self::capacity(s));
            rx.shrink_to(Self::capacity(s));
            s.overrun = true;
        }
        let signature = Self::signature(&rx);
        if signature != s.rx_signature {
            s.rx_signature = signature;
            s.rx_age_us = 0;
        }
    }
    fn iir(&self, s: &State) -> u8 {
        let n = if Self::stale_epoch(s) {
            0
        } else {
            self.rx.lock().unwrap().len()
        };
        let threshold = if s.fcr & 1 == 0 {
            1
        } else {
            [1, 4, 8, 14][(s.fcr >> 6) as usize]
        };
        let id = if s.ier & 4 != 0 && s.overrun && !Self::stale_epoch(s) {
            6
        } else if s.ier & 1 != 0 && n >= threshold {
            4
        } else if s.ier & 1 != 0 && n > 0 && s.rx_age_us >= RX_TIMEOUT_US {
            12
        } else if s.ier & 2 != 0 && s.thre {
            2
        } else if s.ier & 8 != 0 && s.modem_delta != 0 {
            0
        } else {
            1
        };
        id | if s.fcr & 1 != 0 { 0xc0 } else { 0 }
    }
    fn sync(&self, s: &State) {
        self.gic
            .lock()
            .unwrap()
            .set_level(self.intid, self.iir(s) & 1 == 0);
    }
    pub fn inject_rx(&self, bytes: &[u8]) {
        let mut s = self.state.lock().unwrap();
        assert!(
            !s.exclusive,
            "exclusive protocol RX cannot mix host injections"
        );
        for &v in bytes {
            self.push_rx(&mut s, v);
        }
        self.sync(&s);
    }
    fn check(o: u64, w: u32) -> SimResult<()> {
        if matches!(w, 1 | 2 | 4) && o % 4 == 0 {
            Ok(())
        } else {
            Err(SimulationError::NotImplemented(format!(
                "DW UART offset {o:#x}, width {w}"
            )))
        }
    }
    fn read_width(&self, o: u64, w: u32) -> SimResult<u32> {
        Self::check(o, w)?;
        let mut s = self.state.lock().unwrap();
        self.reconcile_rx(&mut s);
        let v = match o {
            0 => {
                if s.lcr & 128 != 0 {
                    s.dll
                } else {
                    let mut rx = self.rx.lock().unwrap();
                    let byte = rx.pop_front().unwrap_or(0);
                    s.rx_age_us = 0;
                    s.rx_signature = Self::signature(&rx);
                    byte
                }
            }
            4 => {
                if s.lcr & 128 != 0 {
                    s.dlm
                } else {
                    s.ier
                }
            }
            8 => {
                let v = self.iir(&s);
                if v & 15 == 2 {
                    s.thre = false;
                }
                v
            }
            12 => s.lcr,
            16 => s.mcr,
            20 => {
                let v = 0x60
                    | u8::from(!self.rx.lock().unwrap().is_empty())
                    | if s.overrun { 2 } else { 0 };
                s.overrun = false;
                v
            }
            24 => {
                let v = Self::modem(&s) | s.modem_delta;
                s.modem_delta = 0;
                v
            }
            28 => s.scr,
            0x7c => {
                let n = self.rx.lock().unwrap().len();
                6 | if n > 0 { 8 } else { 0 } | if n >= Self::capacity(&s) { 16 } else { 0 }
            }
            0xb4 | 0xc0 | 0xf4 | 0xf8 | 0xfc => 0,
            _ => {
                return Err(SimulationError::NotImplemented(format!(
                    "DW UART read {o:#x}"
                )))
            }
        };
        self.sync(&s);
        Ok(v as u32)
    }
    fn write_width(&mut self, o: u64, w: u32, v: u32) -> SimResult<()> {
        Self::check(o, w)?;
        // Linux probes absent optional RE_EN and DLF before reading UCV/CPR.
        // These exact probe transactions read back zero and have no side effect.
        if o == 0xb4 && matches!(v, 0 | 1) {
            return Ok(());
        }
        if o == 0xc0 && w == 4 && v == u32::MAX {
            return Ok(());
        }
        if o == 0xc0 && v != 0 {
            return Err(SimulationError::NotImplemented(
                "DW UART fractional divisor".into(),
            ));
        }
        let mut s = self.state.lock().unwrap();
        self.reconcile_rx(&mut s);
        let v = v as u8;
        match o {
            0 => {
                if s.lcr & 128 != 0 {
                    s.dll = v;
                } else {
                    self.trace.push(
                        &self.trace_name,
                        BusPayload::Uart {
                            direction: BusDir::Tx,
                            byte: v,
                        },
                    );
                    if s.mcr & 16 != 0 {
                        self.push_rx(&mut s, v);
                    } else {
                        for peer in &mut s.streams {
                            peer.on_tx_byte(v);
                        }
                        if let Some(sink) = &s.sink {
                            let mut b = sink.lock().unwrap();
                            if b.len() >= CONSOLE {
                                let remove = b.len() - CONSOLE + 1;
                                b.drain(..remove);
                            }
                            b.push(v);
                        }
                        if s.echo {
                            use std::io::Write;
                            let _ = std::io::stdout().write_all(&[v]);
                        }
                    }
                    s.thre = true;
                }
            }
            4 => {
                if s.lcr & 128 != 0 {
                    s.dlm = v;
                } else {
                    if v & 2 != 0 && s.ier & 2 == 0 {
                        s.thre = true;
                    }
                    s.ier = v & 15;
                }
            }
            8 => {
                if v & 2 != 0 || (v ^ s.fcr) & 1 != 0 {
                    self.rx.lock().unwrap().clear();
                    s.overrun = false;
                    s.rx_age_us = 0;
                    s.rx_signature = 0;
                }
                if v & 4 != 0 {
                    s.thre = true;
                }
                s.fcr = v & 0xc9;
            }
            12 => s.lcr = v,
            16 => {
                let old = Self::modem(&s);
                s.mcr = v & 0x1f;
                let new = Self::modem(&s);
                s.modem_delta |= ((new ^ old) >> 4) & 0xb;
                if old & 0x40 != 0 && new & 0x40 == 0 {
                    s.modem_delta |= 4;
                }
            }
            28 => s.scr = v,
            0xc0 => {
                if v != 0 {
                    return Err(SimulationError::NotImplemented(
                        "DW UART fractional divisor".into(),
                    ));
                }
            }
            _ => {
                return Err(SimulationError::NotImplemented(format!(
                    "DW UART write {o:#x}"
                )))
            }
        }
        self.sync(&s);
        Ok(())
    }
}
impl UartStreamHost for DwUart {
    fn accepts_stream_attachment(&self) -> bool {
        !self.state.lock().unwrap().exclusive
    }
    fn attach_exclusive_stream_device(
        &mut self,
        dev: Box<dyn UartStreamDevice>,
    ) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        if !s.streams.is_empty() || !self.rx.lock().unwrap().is_empty() {
            return Err("exclusive protocol peer requires an empty UART and RX FIFO".into());
        }
        if dev.stream_generation().is_none() || !dev.carries_protocol_octets() {
            return Err(
                "exclusive peer must expose a stream generation and protocol capability".into(),
            );
        }
        s.peer_epoch = dev.stream_generation();
        s.exclusive = true;
        s.streams.push(dev);
        s.sink = None;
        s.echo = false;
        Ok(())
    }
    fn attach_stream_device(&mut self, dev: Box<dyn UartStreamDevice>) {
        let mut s = self.state.lock().unwrap();
        assert!(
            !s.exclusive && dev.stream_generation().is_none(),
            "generation peer requires exclusive attachment"
        );
        s.streams.push(dev);
    }
    fn detach_console_sink(&mut self) {
        let mut s = self.state.lock().unwrap();
        s.sink = None;
        s.echo = false;
    }
    fn attach_console_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo: bool) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.exclusive {
            return false;
        }
        s.sink = sink;
        s.echo = echo;
        true
    }
    fn hosts_protocol_peer(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .streams
            .iter()
            .any(|p| p.carries_protocol_octets())
    }
    fn peer_ids(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .streams
            .iter()
            .filter_map(|p| p.device_id().map(str::to_owned))
            .collect()
    }
    fn inject_peer_remote(&mut self, id: &str, bytes: &[u8]) -> Result<(), String> {
        crate::peripherals::device::inject_remote_into(
            &mut self.state.lock().unwrap().streams,
            id,
            bytes,
        )
    }
}
impl Peripheral for DwUart {
    fn attach_bus_trace(&mut self, name: &str, trace: &BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.into();
    }
    fn bus_trace_handle(&self) -> Option<BusTrace> {
        Some(self.trace.clone())
    }
    fn peek(&self, o: u64) -> Option<u8> {
        let s = self.state.lock().unwrap();
        let stale = Self::stale_epoch(&s);
        let rx = self.rx.lock().unwrap();
        let n = if stale { 0 } else { rx.len() };
        match o {
            0 => Some(if s.lcr & 128 != 0 {
                s.dll
            } else if stale {
                0
            } else {
                rx.front().copied().unwrap_or(0)
            }),
            4 => Some(if s.lcr & 128 != 0 { s.dlm } else { s.ier }),
            // Release the RX guard because iir observes the same canonical FIFO.
            8 => {
                drop(rx);
                Some(self.iir(&s))
            }
            12 => Some(s.lcr),
            16 => Some(s.mcr),
            20 => Some(0x60 | u8::from(n > 0) | if s.overrun && !stale { 2 } else { 0 }),
            24 => Some(Self::modem(&s) | s.modem_delta),
            28 => Some(s.scr),
            0x7c => {
                Some(6 | if n > 0 { 8 } else { 0 } | if n >= Self::capacity(&s) { 16 } else { 0 })
            }
            0xb4 | 0xc0 | 0xf4 | 0xf8 | 0xfc => Some(0),
            _ => None,
        }
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        self.state
            .lock()
            .unwrap()
            .streams
            .iter()
            .flat_map(|p| p.logs())
            .collect()
    }
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for p in &mut self.state.lock().unwrap().streams {
            if p.as_sim_input_mut().is_some_and(&mut *f) {
                return true;
            }
        }
        false
    }
    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for p in &self.state.lock().unwrap().streams {
            let evidence = crate::inspect::UartEvidence(&**p);
            f(crate::inspect::AttachedDeviceRef {
                transport: "uart",
                address: None,
                cs_pin: None,
                mux_address: None,
                channel: None,
                declared_id: p.device_id(),
                instance_id: None,
                bus: None,
                evidence: Some(&evidence),
            });
        }
    }
    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        let mut pi = crate::inspect::default_inspect(self, base, name, opts);
        pi.kind = "uart".into();
        for p in &self.state.lock().unwrap().streams {
            pi.artifacts
                .extend(p.artifacts(p.device_id().unwrap_or(name), opts));
        }
        pi
    }

    fn read(&self, o: u64) -> SimResult<u8> {
        Ok(self.read_width(o, 1)? as u8)
    }
    fn write(&mut self, o: u64, v: u8) -> SimResult<()> {
        self.write_width(o, 1, v as u32)
    }
    fn read_u16(&self, o: u64) -> SimResult<u16> {
        Ok(self.read_width(o, 2)? as u16)
    }
    fn read_u32(&self, o: u64) -> SimResult<u32> {
        self.read_width(o, 4)
    }
    fn write_u16(&mut self, o: u64, v: u16) -> SimResult<()> {
        self.write_width(o, 2, v as u32)
    }
    fn write_u32(&mut self, o: u64, v: u32) -> SimResult<()> {
        self.write_width(o, 4, v)
    }
    fn as_uart_stream_host(&mut self) -> Option<&mut dyn UartStreamHost> {
        Some(self)
    }
    fn uart_rx_source(&self) -> Option<Arc<Mutex<VecDeque<u8>>>> {
        if self.state.lock().unwrap().exclusive {
            None
        } else {
            Some(self.rx.clone())
        }
    }
    fn irq_line_level(&self) -> Option<bool> {
        let mut s = self.state.lock().unwrap();
        self.reconcile_rx(&mut s);
        Some(self.iir(&s) & 1 == 0)
    }
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn drives_central_device_time(&self) -> bool {
        true
    }
    fn advance_attached_device_time_us(&mut self, us: u64) {
        let mut s = self.state.lock().unwrap();
        self.reconcile_rx(&mut s);
        if !self.rx.lock().unwrap().is_empty() {
            s.rx_age_us = s.rx_age_us.saturating_add(us).min(RX_TIMEOUT_US);
        }
        let mut received = Vec::with_capacity(FIFO);
        let mut dropped = false;
        for peer in &mut s.streams {
            for n in 0..peer.max_bytes_per_tick().min(FIFO) {
                if let Some(v) = peer.poll(if n == 0 {
                    us.min(u32::MAX as u64) as u32
                } else {
                    0
                }) {
                    if received.len() < FIFO {
                        received.push(v);
                    } else {
                        dropped = true;
                    }
                } else {
                    break;
                }
            }
        }
        if dropped {
            s.overrun = true;
        }
        for v in received {
            self.push_rx(&mut s, v);
        }
        self.sync(&s);
    }
}
