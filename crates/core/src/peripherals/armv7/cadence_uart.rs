//! Cadence UART (Xilinx `xlnx,xuartps`), as on the Zynq UltraScale+ MPSoC
//! (UG1087 UART register reference). Immediate functional TX, bounded 64-byte
//! RX FIFO shared with host input. ISR condition bits re-latch while their
//! condition holds (QEMU `cadence_uart` semantics); the GIC level is
//! `ISR & IMR`. Baud timing, RX timeout, parity/framing errors, break, modem
//! lines, flow-control delay and DMA are not modelled.
use super::gic::Gic;
use crate::bus::bus_trace::{BusDir, BusPayload, BusTrace};
use crate::peripherals::device::{UartStreamDevice, UartStreamHost};
use crate::{Peripheral, SimResult, SimulationError};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
const FIFO: usize = 64;
const CONSOLE: usize = 64 * 1024;
// Channel status (SR) and interrupt (ISR/IMR) bit positions shared by both.
const RTRIG: u32 = 1 << 0;
const REMPTY: u32 = 1 << 1;
const RFUL: u32 = 1 << 2;
const TEMPTY: u32 = 1 << 3;
const ROVR: u32 = 1 << 5;
const TTRIG: u32 = 1 << 10;
const TNFUL: u32 = 1 << 11;
const IXR_MASK: u32 = 0x3fff;
struct State {
    cr: u32,
    mr: u32,
    imr: u32,
    isr: u32,
    baudgen: u32,
    bauddiv: u32,
    rxtout: u32,
    rxwm: u32,
    modemcr: u32,
    flowdel: u32,
    txwm: u32,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo: bool,
    streams: Vec<Box<dyn UartStreamDevice>>,
}
impl Default for State {
    fn default() -> Self {
        // UG1087 reset values.
        Self {
            cr: 0x128,
            mr: 0,
            imr: 0,
            isr: 0,
            baudgen: 0x28b,
            bauddiv: 0xf,
            rxtout: 0,
            rxwm: 0x20,
            modemcr: 0,
            flowdel: 0,
            txwm: 0x20,
            sink: None,
            echo: false,
            streams: Vec::new(),
        }
    }
}
pub struct CadenceUart {
    state: Mutex<State>,
    rx: Arc<Mutex<VecDeque<u8>>>,
    gic: Arc<Mutex<Gic>>,
    intid: usize,
    trace: BusTrace,
    trace_name: String,
}
impl std::fmt::Debug for CadenceUart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CadenceUart")
            .field("intid", &self.intid)
            .finish_non_exhaustive()
    }
}
impl CadenceUart {
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
    fn rx_enabled(s: &State) -> bool {
        s.cr & (1 << 2) != 0 && s.cr & (1 << 3) == 0
    }
    fn tx_enabled(s: &State) -> bool {
        s.cr & (1 << 4) != 0 && s.cr & (1 << 5) == 0
    }
    /// Channel status. TX drains immediately, so the TX FIFO is always empty.
    fn status(&self, s: &State) -> u32 {
        let n = self.rx.lock().unwrap().len();
        let mut sr = TEMPTY | TNFUL;
        if n == 0 {
            sr |= REMPTY;
        }
        if n >= FIFO {
            sr |= RFUL;
        }
        if s.rxwm != 0 && n >= s.rxwm as usize {
            sr |= RTRIG;
        }
        if s.txwm == 0 {
            sr |= TTRIG;
        }
        sr
    }
    fn update(&self, s: &mut State) {
        s.isr |= self.status(s) & (RTRIG | REMPTY | RFUL | TEMPTY | TTRIG | TNFUL);
        self.gic
            .lock()
            .unwrap()
            .set_level(self.intid, s.isr & s.imr != 0);
    }
    pub fn inject_rx(&self, bytes: &[u8]) {
        let mut s = self.state.lock().unwrap();
        {
            let mut rx = self.rx.lock().unwrap();
            for &v in bytes {
                if rx.len() >= FIFO {
                    s.isr |= ROVR;
                } else {
                    rx.push_back(v);
                }
            }
        }
        self.update(&mut s);
    }
    fn check(o: u64, w: u32) -> SimResult<()> {
        if w == 4 && o % 4 == 0 && o <= 0x48 {
            Ok(())
        } else {
            Err(SimulationError::NotImplemented(format!(
                "Cadence UART offset {o:#x}, width {w}"
            )))
        }
    }
    fn transmit(&self, s: &mut State, v: u8) {
        self.trace.push(
            &self.trace_name,
            BusPayload::Uart {
                direction: BusDir::Tx,
                byte: v,
            },
        );
        // MR.CHMODE 10 = local loopback: TX feeds RX, nothing leaves the pin.
        if (s.mr >> 8) & 3 == 2 {
            let mut rx = self.rx.lock().unwrap();
            if rx.len() >= FIFO {
                s.isr |= ROVR;
            } else {
                rx.push_back(v);
            }
            return;
        }
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
    fn read_word(&self, o: u64) -> SimResult<u32> {
        Self::check(o, 4)?;
        let mut s = self.state.lock().unwrap();
        let v = match o {
            0x00 => s.cr,
            0x04 => s.mr,
            // IER/IDR are write-only and read as zero.
            0x08 | 0x0c => 0,
            0x10 => s.imr,
            0x14 => s.isr,
            0x18 => s.baudgen,
            0x1c => s.rxtout,
            0x20 => s.rxwm,
            0x24 => s.modemcr,
            // DCD/RI/DSR/CTS inactive; no deltas.
            0x28 => 0,
            0x2c => self.status(&s),
            0x30 => {
                if Self::rx_enabled(&s) {
                    u32::from(self.rx.lock().unwrap().pop_front().unwrap_or(0))
                } else {
                    0
                }
            }
            0x34 => s.bauddiv,
            0x38 => s.flowdel,
            0x44 => s.txwm,
            // RX byte status: no parity/frame/break errors are ever recorded.
            0x48 => 0,
            _ => {
                return Err(SimulationError::NotImplemented(format!(
                    "Cadence UART read {o:#x}"
                )))
            }
        };
        self.update(&mut s);
        Ok(v)
    }
    fn write_word(&mut self, o: u64, v: u32) -> SimResult<()> {
        Self::check(o, 4)?;
        let mut s = self.state.lock().unwrap();
        match o {
            0x00 => {
                if v & 1 != 0 {
                    self.rx.lock().unwrap().clear();
                }
                // RXRST/TXRST self-clear; TORST, STTBRK and STPBRK have no
                // modelled effect.
                s.cr = v & 0x1fc;
            }
            0x04 => {
                if (v >> 8) & 3 == 3 || (v >> 8) & 3 == 1 {
                    return Err(SimulationError::NotImplemented(
                        "Cadence UART automatic echo / remote loopback mode".into(),
                    ));
                }
                s.mr = v & 0x3ff;
            }
            0x08 => s.imr |= v & IXR_MASK,
            0x0c => s.imr &= !(v & IXR_MASK),
            0x10 => {}
            0x14 => s.isr &= !(v & IXR_MASK),
            0x18 => s.baudgen = v & 0xffff,
            0x1c => s.rxtout = v & 0xff,
            0x20 => s.rxwm = v & 0x3f,
            0x24 => s.modemcr = v & 0x23,
            0x28 | 0x2c | 0x48 => {}
            0x30 => {
                if Self::tx_enabled(&s) {
                    self.transmit(&mut s, v as u8);
                }
            }
            0x34 => s.bauddiv = v & 0xff,
            0x38 => s.flowdel = v & 0x3f,
            0x44 => s.txwm = v & 0x3f,
            _ => {
                return Err(SimulationError::NotImplemented(format!(
                    "Cadence UART write {o:#x}"
                )))
            }
        }
        self.update(&mut s);
        Ok(())
    }
}
impl UartStreamHost for CadenceUart {
    fn attach_stream_device(&mut self, dev: Box<dyn UartStreamDevice>) {
        assert!(
            dev.stream_generation().is_none(),
            "Cadence UART has no exclusive protocol attachment"
        );
        self.state.lock().unwrap().streams.push(dev);
    }
    fn detach_console_sink(&mut self) {
        let mut s = self.state.lock().unwrap();
        s.sink = None;
        s.echo = false;
    }
    fn attach_console_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo: bool) -> bool {
        let mut s = self.state.lock().unwrap();
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
impl Peripheral for CadenceUart {
    fn attach_bus_trace(&mut self, name: &str, trace: &BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.into();
    }
    fn bus_trace_handle(&self) -> Option<BusTrace> {
        Some(self.trace.clone())
    }
    fn peek(&self, o: u64) -> Option<u8> {
        let s = self.state.lock().unwrap();
        let word = match o & !3 {
            0x00 => s.cr,
            0x04 => s.mr,
            0x10 => s.imr,
            0x14 => s.isr,
            0x18 => s.baudgen,
            0x20 => s.rxwm,
            0x2c => self.status(&s),
            0x30 => u32::from(self.rx.lock().unwrap().front().copied().unwrap_or(0)),
            0x34 => s.bauddiv,
            0x44 => s.txwm,
            o if o <= 0x48 => 0,
            _ => return None,
        };
        Some((word >> ((o & 3) * 8)) as u8)
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
    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        let mut pi = crate::inspect::default_inspect(self, base, name, opts);
        pi.kind = "uart".into();
        pi
    }
    fn read(&self, o: u64) -> SimResult<u8> {
        Err(SimulationError::NotImplemented(format!(
            "Cadence UART byte read {o:#x}"
        )))
    }
    fn write(&mut self, o: u64, _: u8) -> SimResult<()> {
        Err(SimulationError::NotImplemented(format!(
            "Cadence UART byte write {o:#x}"
        )))
    }
    fn read_u32(&self, o: u64) -> SimResult<u32> {
        self.read_word(o)
    }
    fn write_u32(&mut self, o: u64, v: u32) -> SimResult<()> {
        self.write_word(o, v)
    }
    fn as_uart_stream_host(&mut self) -> Option<&mut dyn UartStreamHost> {
        Some(self)
    }
    fn uart_rx_source(&self) -> Option<Arc<Mutex<VecDeque<u8>>>> {
        Some(self.rx.clone())
    }
    // TX is immediate and RX arrives through the shared queue reconciled by
    // `irq_line_level`; there is no per-cycle work to walk.
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn irq_line_level(&self) -> Option<bool> {
        let mut s = self.state.lock().unwrap();
        // Host input lands in the shared RX queue between instructions;
        // re-latch the derived conditions before reporting the level.
        s.isr |= self.status(&s) & (RTRIG | REMPTY | RFUL | TEMPTY | TTRIG | TNFUL);
        Some(s.isr & s.imr != 0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn uart() -> (CadenceUart, Arc<Mutex<Gic>>) {
        let gic = Arc::new(Mutex::new(Gic::default()));
        (CadenceUart::new(gic.clone(), 54), gic)
    }
    #[test]
    fn reset_status_and_polled_tx_reach_console() {
        let (mut u, _) = uart();
        assert_eq!(u.read_u32(0).unwrap(), 0x128);
        let sr = u.read_u32(0x2c).unwrap();
        assert_eq!(sr & (TEMPTY | REMPTY | (1 << 4)), TEMPTY | REMPTY);
        let sink = Arc::new(Mutex::new(Vec::new()));
        assert!(u.attach_console_sink(Some(sink.clone()), false));
        // TX disabled at reset (TXDIS set): bytes are dropped.
        u.write_u32(0x30, b'x' as u32).unwrap();
        u.write_u32(0, 0x14).unwrap();
        for &b in b"ok" {
            u.write_u32(0x30, b as u32).unwrap();
        }
        assert_eq!(&*sink.lock().unwrap(), b"ok");
    }
    #[test]
    fn rx_trigger_interrupt_follows_mask_and_w1c() {
        let (mut u, gic) = uart();
        u.write_u32(0, 0x14).unwrap();
        u.write_u32(0x20, 1).unwrap();
        u.write_u32(0x0c, IXR_MASK).unwrap();
        u.write_u32(0x14, IXR_MASK).unwrap();
        u.write_u32(0x08, RTRIG).unwrap();
        assert_eq!(u.irq_line_level(), Some(false));
        u.inject_rx(b"A");
        assert_eq!(u.irq_line_level(), Some(true));
        let _ = gic;
        assert_eq!(u.read_u32(0x30).unwrap(), b'A' as u32);
        u.write_u32(0x14, RTRIG).unwrap();
        assert_eq!(u.read_u32(0x14).unwrap() & RTRIG, 0);
        assert_eq!(u.irq_line_level(), Some(false));
    }
    #[test]
    fn local_loopback_and_unsupported_modes() {
        let (mut u, _) = uart();
        u.write_u32(0, 0x14).unwrap();
        u.write_u32(4, 2 << 8).unwrap();
        u.write_u32(0x30, 0x5a).unwrap();
        assert_eq!(u.read_u32(0x30).unwrap(), 0x5a);
        assert!(u.write_u32(4, 1 << 8).is_err());
        assert!(u.read(0).is_err());
        assert!(u.read_u32(0x4c).is_err());
    }
}
