// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP LPC-style FLEXCOMM in USART mode (i.MX RT500 FLEXCOMM0-12, the same
//! block as LPC55xx/LPC54xxx).
//!
//! Source: the MCUXpresso SDK device header `MIMXRT595S_cm33.h` (`USART_Type`
//! and `FLEXCOMM_Type` @ `USART0_BASE` = `FLEXCOMM0_BASE` = 0x4010_6000) for
//! the register layout and every bit field used here; UM11147 (i.MX RT500
//! reference manual) chapters "Flexcomm Interface serial communication" and
//! "USARTs" for the semantics.
//!
//! One 4 KiB window carries both views: the USART registers at 0x000..0xE4C
//! and the FLEXCOMM `PSELID` @ 0xFF8 / `PID` @ 0xFFC. The USART registers only
//! respond while `PSELID.PERSEL` = 1 (USART); with another function selected
//! they read 0 and ignore writes (the SPI/I2C/I2S views are not modelled).
//!
//! Modelled:
//! - `PSELID`: PERSEL \[2:0\] writable until LOCK \[3\] is set (LOCK is
//!   write-once); the read-only present bits report USART/SPI/I2C/I2S
//!   (`config.present`, default 0xF0 = all four, as on RT500 FLEXCOMM0-7).
//! - `CFG`, `CTL`, `BRG`, `OSR` (reset 0xF), `ADDR`, `INTENSET`/`INTENCLR`,
//!   `FIFOTRIG`, `FIFOINTENSET`/`FIFOINTENCLR`: stored.
//! - TX: a `FIFOWR` byte is transmitted at once (to the console sink and the
//!   bus trace) when `CFG.ENABLE` and `FIFOCFG.ENABLETX` are set and
//!   `CTL.TXDIS` is clear; otherwise it is dropped. So the TX FIFO always
//!   reads empty: `FIFOSTAT.TXEMPTY`/`TXNOTFULL` = 1, `TXLVL` = 0,
//!   `STAT.TXIDLE` = 1. `STAT.TXDISSTAT` follows `CTL.TXDIS`.
//! - RX: a 16-entry FIFO filled from the injection queue
//!   ([`Peripheral::uart_rx_source`]) while `FIFOCFG.ENABLERX` is set;
//!   `FIFORD` pops, `FIFORDNOPOP` peeks; `FIFOSTAT.RXNOTEMPTY`/`RXFULL`/
//!   `RXLVL` follow the queue. `FIFOCFG.EMPTYRX` flushes it.
//! - `FIFOCFG.SIZE` reads 0 (16 entries) and `FIFOSIZE` reads 16.
//! - `FIFOINTSTAT`/`INTSTAT` are computed from the enables and the levels.
//!
//! NOT modelled: the NVIC line (status is polled; no interrupt is raised),
//! baud/frame timing (BRG/OSR are stored only; a byte leaves in zero time),
//! DMA requests, 9-bit/address/LIN/synchronous modes, CTS/RTS, break and the
//! receive error flags (never set), `FIFOSTAT.TXERR`/`RXERR` (never set: an
//! RX byte arriving into a full FIFO stays in the injection queue), and the
//! `PID`/`ID` identification values (read 0).

use crate::bus::bus_trace::{BusDir, BusPayload, BusTrace};
use crate::peripherals::uart::UartStreamHost;
use crate::{Peripheral, SimResult};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const CFG: u64 = 0x000;
const CTL: u64 = 0x004;
const STAT: u64 = 0x008;
const INTENSET: u64 = 0x00C;
const INTENCLR: u64 = 0x010;
const BRG: u64 = 0x020;
const INTSTAT: u64 = 0x024;
const OSR: u64 = 0x028;
const ADDR: u64 = 0x02C;
const FIFOCFG: u64 = 0xE00;
const FIFOSTAT: u64 = 0xE04;
const FIFOTRIG: u64 = 0xE08;
const FIFOINTENSET: u64 = 0xE10;
const FIFOINTENCLR: u64 = 0xE14;
const FIFOINTSTAT: u64 = 0xE18;
const FIFOWR: u64 = 0xE20;
const FIFORD: u64 = 0xE30;
const FIFORDNOPOP: u64 = 0xE40;
const FIFOSIZE: u64 = 0xE48;
const PSELID: u64 = 0xFF8;
const PID: u64 = 0xFFC;

pub const CFG_ENABLE: u32 = 1 << 0;
const CFG_DATALEN_MASK: u32 = 0x3 << 2;
pub const CTL_TXDIS: u32 = 1 << 6;
pub const STAT_RXIDLE: u32 = 1 << 1;
pub const STAT_TXIDLE: u32 = 1 << 3;
pub const STAT_TXDISSTAT: u32 = 1 << 6;
pub const FIFOCFG_ENABLETX: u32 = 1 << 0;
pub const FIFOCFG_ENABLERX: u32 = 1 << 1;
/// FIFOCFG bits that are stored (ENABLETX/RX, DMATX/RX, WAKETX/RX, POPDBG).
const FIFOCFG_STORED: u32 = 0x3 | (0x3 << 12) | (0x3 << 14) | (1 << 18);
const FIFOCFG_EMPTYTX: u32 = 1 << 16;
const FIFOCFG_EMPTYRX: u32 = 1 << 17;
pub const FIFOSTAT_TXEMPTY: u32 = 1 << 4;
pub const FIFOSTAT_TXNOTFULL: u32 = 1 << 5;
pub const FIFOSTAT_RXNOTEMPTY: u32 = 1 << 6;
pub const FIFOSTAT_RXFULL: u32 = 1 << 7;
const FIFOTRIG_TXLVLENA: u32 = 1 << 0;
const FIFOTRIG_RXLVLENA: u32 = 1 << 1;
const FIFOINT_TXLVL: u32 = 1 << 2;
const FIFOINT_RXLVL: u32 = 1 << 3;
pub const PSELID_PERSEL_MASK: u32 = 0x7;
pub const PSELID_LOCK: u32 = 1 << 3;
pub const PERSEL_USART: u32 = 1;
/// USARTPRESENT | SPIPRESENT | I2CPRESENT | I2SPRESENT.
const PRESENT_ALL: u32 = 0xF0;

/// `FIFOSIZE` / `FIFOCFG.SIZE` = 0: 16 entries.
pub const FIFO_DEPTH: usize = 16;
/// `OSR` reset value: 16x oversampling.
const OSR_RESET: u32 = 0xF;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Regs {
    pselid: u32,
    cfg: u32,
    ctl: u32,
    intenset: u32,
    brg: u32,
    osr: u32,
    addr: u32,
    fifocfg: u32,
    fifotrig: u32,
    fifointenset: u32,
}

/// One FLEXCOMM in USART mode (see the module docs for what is modelled).
pub struct LpcFlexcommUsart {
    regs: Regs,
    /// Read-only PSELID present bits.
    present: u32,
    /// The RX FIFO proper (at most [`FIFO_DEPTH`] bytes). Interior mutability
    /// because `FIFORD` pops on a read.
    rx_fifo: Mutex<VecDeque<u8>>,
    /// Host injection queue: bytes waiting on the far side of the RX pin.
    rx_inject: Arc<Mutex<VecDeque<u8>>>,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
    tx_log: Vec<u8>,
    trace: BusTrace,
    trace_name: String,
}

impl std::fmt::Debug for LpcFlexcommUsart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LpcFlexcommUsart")
            .field("regs", &self.regs)
            .field("tx_bytes", &self.tx_log.len())
            .finish_non_exhaustive()
    }
}

impl Default for LpcFlexcommUsart {
    fn default() -> Self {
        Self::new(PRESENT_ALL)
    }
}

impl LpcFlexcommUsart {
    /// `present` is the PSELID present-bit field (bits 7:4).
    pub fn new(present: u32) -> Self {
        Self {
            regs: Self::reset_regs(),
            present: present & PRESENT_ALL,
            rx_fifo: Mutex::new(VecDeque::new()),
            rx_inject: Arc::new(Mutex::new(VecDeque::new())),
            sink: None,
            echo_stdout: false,
            tx_log: Vec::new(),
            trace: crate::bus::bus_trace::new_log(),
            trace_name: String::new(),
        }
    }

    fn reset_regs() -> Regs {
        Regs {
            osr: OSR_RESET,
            ..Regs::default()
        }
    }

    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }

    /// Every byte transmitted so far.
    pub fn tx_log(&self) -> &[u8] {
        &self.tx_log
    }

    fn usart_selected(&self) -> bool {
        self.regs.pselid & PSELID_PERSEL_MASK == PERSEL_USART
    }

    /// Move injected bytes into the RX FIFO while the receiver is enabled.
    fn refill_rx(&self) {
        if self.regs.fifocfg & FIFOCFG_ENABLERX == 0 || !self.usart_selected() {
            return;
        }
        let mut fifo = self.rx_fifo.lock().unwrap();
        let mut inject = self.rx_inject.lock().unwrap();
        while fifo.len() < FIFO_DEPTH {
            let Some(byte) = inject.pop_front() else {
                break;
            };
            self.trace.push(
                &self.trace_name,
                BusPayload::Uart {
                    direction: BusDir::Rx,
                    byte,
                },
            );
            fifo.push_back(byte);
        }
    }

    fn rx_level(&self) -> usize {
        self.refill_rx();
        self.rx_fifo.lock().unwrap().len()
    }

    fn stat(&self) -> u32 {
        let mut stat = STAT_TXIDLE | STAT_RXIDLE;
        if self.regs.ctl & CTL_TXDIS != 0 {
            stat |= STAT_TXDISSTAT;
        }
        stat
    }

    fn fifostat(&self) -> u32 {
        let level = self.rx_level();
        let mut stat = FIFOSTAT_TXEMPTY | FIFOSTAT_TXNOTFULL | ((level as u32) << 16);
        if level > 0 {
            stat |= FIFOSTAT_RXNOTEMPTY;
        }
        if level >= FIFO_DEPTH {
            stat |= FIFOSTAT_RXFULL;
        }
        stat
    }

    fn fifointstat(&self) -> u32 {
        let trig = self.regs.fifotrig;
        let mut raw = 0;
        // TX level is always 0, so "TX level <= TXLVL" holds whenever enabled.
        if trig & FIFOTRIG_TXLVLENA != 0 {
            raw |= FIFOINT_TXLVL;
        }
        let rx_trig = ((trig >> 16) & 0xF) as usize;
        if trig & FIFOTRIG_RXLVLENA != 0 && self.rx_level() > rx_trig {
            raw |= FIFOINT_RXLVL;
        }
        raw & self.regs.fifointenset
    }

    fn data_mask(&self) -> u32 {
        // DATALEN: 0 = 7 bits, 1 = 8 bits, 2 = 9 bits.
        match (self.regs.cfg & CFG_DATALEN_MASK) >> 2 {
            0 => 0x7F,
            1 => 0xFF,
            _ => 0x1FF,
        }
    }

    fn transmit(&mut self, value: u32) {
        let enabled = self.regs.cfg & CFG_ENABLE != 0
            && self.regs.fifocfg & FIFOCFG_ENABLETX != 0
            && self.regs.ctl & CTL_TXDIS == 0;
        if !enabled {
            return;
        }
        // The console carries 8-bit characters; a 9th data bit is dropped.
        let byte = (value & self.data_mask()) as u8;
        self.trace.push(
            &self.trace_name,
            BusPayload::Uart {
                direction: BusDir::Tx,
                byte,
            },
        );
        self.tx_log.push(byte);
        if let Some(sink) = &self.sink {
            if let Ok(mut s) = sink.lock() {
                s.push(byte);
            }
        }
        if self.echo_stdout {
            use std::io::Write;
            let mut out = std::io::stdout();
            let _ = out.write_all(&[byte]);
            let _ = out.flush();
        }
    }

    fn read_fiford(&self, pop: bool) -> u32 {
        self.refill_rx();
        let mut fifo = self.rx_fifo.lock().unwrap();
        let byte = if pop {
            fifo.pop_front()
        } else {
            fifo.front().copied()
        };
        u32::from(byte.unwrap_or(0))
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            PSELID => self.regs.pselid | self.present,
            PID => 0,
            _ if !self.usart_selected() => 0,
            CFG => self.regs.cfg,
            CTL => self.regs.ctl,
            STAT => self.stat(),
            INTENSET => self.regs.intenset,
            BRG => self.regs.brg,
            INTSTAT => self.stat() & self.regs.intenset,
            OSR => self.regs.osr,
            ADDR => self.regs.addr,
            FIFOCFG => self.regs.fifocfg,
            FIFOSTAT => self.fifostat(),
            FIFOTRIG => self.regs.fifotrig,
            FIFOINTENSET => self.regs.fifointenset,
            FIFOINTSTAT => self.fifointstat(),
            FIFORD => self.read_fiford(true),
            FIFORDNOPOP => self.read_fiford(false),
            FIFOSIZE => FIFO_DEPTH as u32,
            // INTENCLR, FIFOINTENCLR and FIFOWR are write-only.
            INTENCLR | FIFOINTENCLR | FIFOWR => 0,
            _ => {
                crate::census_reg!("lpc_flexcomm_usart:LpcFlexcommUsart", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if offset == PSELID {
            if self.regs.pselid & PSELID_LOCK == 0 {
                self.regs.pselid = value & (PSELID_PERSEL_MASK | PSELID_LOCK);
            }
            return;
        }
        if offset == PID || !self.usart_selected() {
            return;
        }
        match offset {
            CFG => self.regs.cfg = value,
            CTL => self.regs.ctl = value,
            // The STAT write-1-to-clear flags are never set here.
            STAT => {}
            INTENSET => self.regs.intenset |= value,
            INTENCLR => self.regs.intenset &= !value,
            BRG => self.regs.brg = value & 0xFFFF,
            OSR => self.regs.osr = value & 0xF,
            ADDR => self.regs.addr = value & 0xFF,
            FIFOCFG => {
                self.regs.fifocfg = value & FIFOCFG_STORED;
                if value & FIFOCFG_EMPTYRX != 0 {
                    self.rx_fifo.lock().unwrap().clear();
                }
                // EMPTYTX: the TX FIFO is always empty already.
                let _ = value & FIFOCFG_EMPTYTX;
            }
            // TXERR/RXERR are write-1-to-clear and never set here.
            FIFOSTAT => {}
            FIFOTRIG => self.regs.fifotrig = value & 0x000F_0F03,
            FIFOINTENSET => self.regs.fifointenset |= value & 0x1F,
            FIFOINTENCLR => self.regs.fifointenset &= !value,
            FIFOWR => self.transmit(value),
            INTSTAT | FIFOINTSTAT | FIFORD | FIFORDNOPOP | FIFOSIZE => {}
            _ => {
                crate::census_reg!("lpc_flexcomm_usart:LpcFlexcommUsart", offset, "write");
            }
        }
    }
}

impl UartStreamHost for LpcFlexcommUsart {
    fn attach_console_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo: bool) -> bool {
        self.set_sink(sink, echo);
        true
    }

    fn accepts_stream_attachment(&self) -> bool {
        false
    }

    fn attach_stream_device(&mut self, _dev: Box<dyn crate::peripherals::uart::UartStreamDevice>) {
        // Unreachable through `try_attach_stream_device`:
        // `accepts_stream_attachment` is false (no cross-link peers yet).
    }

    fn detach_console_sink(&mut self) {
        self.set_sink(None, false);
    }

    fn hosts_protocol_peer(&self) -> bool {
        false
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot {
    regs: Regs,
    rx_fifo: Vec<u8>,
}

impl Peripheral for LpcFlexcommUsart {
    /// No `tick()`: TX completes inside the `FIFOWR` store and RX is pulled
    /// from the injection queue on access.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn attach_bus_trace(&mut self, name: &str, trace: &BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.into();
    }

    fn bus_trace_handle(&self) -> Option<BusTrace> {
        Some(self.trace.clone())
    }

    /// A byte load of lane 0 has the word's read side effect (a `FIFORD`
    /// pop); the upper lanes do not.
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = if offset & 3 == 0 {
            self.read_reg(offset)
        } else {
            self.peek_reg(offset & !3)
        };
        Ok((word >> ((offset & 3) * 8)) as u8)
    }

    /// A byte store writes its lane of the word (the other lanes as 0), the
    /// same as a narrow AHB/APB write reaching a 32-bit register.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let word = if aligned == FIFOWR {
            u32::from(value) << shift
        } else {
            (self.peek_reg(aligned) & !(0xFF << shift)) | (u32::from(value) << shift)
        };
        self.write_reg(aligned, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some((self.peek_reg(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn as_uart_stream_host(&mut self) -> Option<&mut dyn UartStreamHost> {
        Some(self)
    }

    fn uart_rx_source(&self) -> Option<Arc<Mutex<VecDeque<u8>>>> {
        Some(self.rx_inject.clone())
    }

    fn on_node_reset(&mut self) {
        self.regs = Self::reset_regs();
        self.rx_fifo.lock().unwrap().clear();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(Snapshot {
            regs: self.regs.clone(),
            rx_fifo: self.rx_fifo.lock().unwrap().iter().copied().collect(),
        })
        .unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let snap: Snapshot = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "LpcFlexcommUsart snapshot decode: {error}"
            ))
        })?;
        self.regs = snap.regs;
        *self.rx_fifo.lock().unwrap() = snap.rx_fifo.into();
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&Snapshot {
            regs: self.regs.clone(),
            rx_fifo: self.rx_fifo.lock().unwrap().iter().copied().collect(),
        })
        .expect("bincode serialize LpcFlexcommUsart")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let snap: Snapshot = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "LpcFlexcommUsart runtime snapshot decode: {error}"
            ))
        })?;
        self.regs = snap.regs;
        *self.rx_fifo.lock().unwrap() = snap.rx_fifo.into();
        Ok(())
    }
}

impl LpcFlexcommUsart {
    /// Side-effect-free register view (a `FIFORD` peek does not pop).
    fn peek_reg(&self, offset: u64) -> u32 {
        if offset == FIFORD {
            if self.usart_selected() {
                self.read_fiford(false)
            } else {
                0
            }
        } else {
            self.read_reg(offset)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usart() -> LpcFlexcommUsart {
        let mut u = LpcFlexcommUsart::default();
        u.write_u32(PSELID, PERSEL_USART).unwrap();
        u
    }

    fn enable_tx(u: &mut LpcFlexcommUsart) {
        u.write_u32(FIFOCFG, FIFOCFG_ENABLETX | FIFOCFG_ENABLERX)
            .unwrap();
        u.write_u32(CFG, CFG_ENABLE | (1 << 2)).unwrap();
    }

    #[test]
    fn pselid_reports_present_functions_and_locks() {
        let mut u = LpcFlexcommUsart::default();
        assert_eq!(u.read_u32(PSELID).unwrap(), 0xF0);
        u.write_u32(PSELID, PERSEL_USART | PSELID_LOCK).unwrap();
        assert_eq!(u.read_u32(PSELID).unwrap(), 0xF9);
        // LOCK is write-once: PERSEL can no longer change.
        u.write_u32(PSELID, 2).unwrap();
        assert_eq!(u.read_u32(PSELID).unwrap(), 0xF9);
    }

    #[test]
    fn usart_registers_are_dark_until_persel_is_usart() {
        let mut u = LpcFlexcommUsart::default();
        u.write_u32(CFG, CFG_ENABLE).unwrap();
        assert_eq!(u.read_u32(CFG).unwrap(), 0);
        assert_eq!(u.read_u32(FIFOSTAT).unwrap(), 0);
        u.write_u32(PSELID, PERSEL_USART).unwrap();
        assert_eq!(u.read_u32(OSR).unwrap(), 0xF);
        assert_eq!(u.read_u32(FIFOSIZE).unwrap(), 16);
    }

    #[test]
    fn fifowr_transmits_only_when_enabled() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut u = usart();
        u.set_sink(Some(sink.clone()), false);
        u.write_u32(FIFOWR, u32::from(b'x')).unwrap();
        assert!(sink.lock().unwrap().is_empty());
        enable_tx(&mut u);
        u.write_u32(FIFOWR, u32::from(b'o')).unwrap();
        u.write(FIFOWR, b'k').unwrap();
        assert_eq!(sink.lock().unwrap().as_slice(), b"ok");
        u.write_u32(CTL, CTL_TXDIS).unwrap();
        u.write_u32(FIFOWR, u32::from(b'!')).unwrap();
        assert_eq!(u.tx_log(), b"ok");
        assert_ne!(u.read_u32(STAT).unwrap() & STAT_TXDISSTAT, 0);
    }

    #[test]
    fn fifostat_reports_empty_tx_and_rx_level() {
        let mut u = usart();
        enable_tx(&mut u);
        let s = u.read_u32(FIFOSTAT).unwrap();
        assert_eq!(s, FIFOSTAT_TXEMPTY | FIFOSTAT_TXNOTFULL);
        u.uart_rx_source()
            .unwrap()
            .lock()
            .unwrap()
            .extend([b'a', b'b']);
        let s = u.read_u32(FIFOSTAT).unwrap();
        assert_ne!(s & FIFOSTAT_RXNOTEMPTY, 0);
        assert_eq!((s >> 16) & 0x1F, 2);
        assert_eq!(u.read_u32(FIFORDNOPOP).unwrap(), u32::from(b'a'));
        assert_eq!(u.read_u32(FIFORD).unwrap(), u32::from(b'a'));
        assert_eq!(u.read_u32(FIFORD).unwrap(), u32::from(b'b'));
        assert_eq!(u.read_u32(FIFOSTAT).unwrap() & FIFOSTAT_RXNOTEMPTY, 0);
    }

    #[test]
    fn rx_fifo_holds_sixteen_and_emptyrx_flushes() {
        let mut u = usart();
        enable_tx(&mut u);
        u.uart_rx_source().unwrap().lock().unwrap().extend(0..20u8);
        let s = u.read_u32(FIFOSTAT).unwrap();
        assert_ne!(s & FIFOSTAT_RXFULL, 0);
        assert_eq!((s >> 16) & 0x1F, 16);
        u.write_u32(
            FIFOCFG,
            FIFOCFG_ENABLETX | FIFOCFG_ENABLERX | FIFOCFG_EMPTYRX,
        )
        .unwrap();
        // The four bytes still queued outside flow in after the flush.
        assert_eq!((u.read_u32(FIFOSTAT).unwrap() >> 16) & 0x1F, 4);
        assert_eq!(u.read_u32(FIFORD).unwrap(), 16);
    }

    #[test]
    fn rx_disabled_leaves_bytes_queued() {
        let mut u = usart();
        u.uart_rx_source().unwrap().lock().unwrap().push_back(7);
        assert_eq!(u.read_u32(FIFOSTAT).unwrap() & FIFOSTAT_RXNOTEMPTY, 0);
        u.write_u32(FIFOCFG, FIFOCFG_ENABLERX).unwrap();
        assert_eq!(u.read_u32(FIFORD).unwrap(), 7);
    }

    #[test]
    fn fifo_interrupt_status_follows_trigger_and_enable() {
        let mut u = usart();
        enable_tx(&mut u);
        u.write_u32(FIFOTRIG, FIFOTRIG_TXLVLENA | FIFOTRIG_RXLVLENA)
            .unwrap();
        assert_eq!(u.read_u32(FIFOINTSTAT).unwrap(), 0);
        u.write_u32(FIFOINTENSET, FIFOINT_TXLVL | FIFOINT_RXLVL)
            .unwrap();
        assert_eq!(u.read_u32(FIFOINTSTAT).unwrap(), FIFOINT_TXLVL);
        u.uart_rx_source().unwrap().lock().unwrap().push_back(1);
        assert_eq!(
            u.read_u32(FIFOINTSTAT).unwrap(),
            FIFOINT_TXLVL | FIFOINT_RXLVL
        );
        u.write_u32(FIFOINTENCLR, FIFOINT_TXLVL).unwrap();
        assert_eq!(u.read_u32(FIFOINTSTAT).unwrap(), FIFOINT_RXLVL);
    }

    #[test]
    fn seven_bit_datalen_masks_the_character() {
        let mut u = usart();
        u.write_u32(FIFOCFG, FIFOCFG_ENABLETX).unwrap();
        u.write_u32(CFG, CFG_ENABLE).unwrap();
        u.write_u32(FIFOWR, 0xC1).unwrap();
        assert_eq!(u.tx_log(), &[0x41]);
    }

    #[test]
    fn snapshot_round_trips_registers_and_rx() {
        let mut u = usart();
        enable_tx(&mut u);
        u.write_u32(BRG, 25).unwrap();
        u.uart_rx_source().unwrap().lock().unwrap().push_back(9);
        let _ = u.read_u32(FIFOSTAT).unwrap();
        let bytes = u.runtime_snapshot();
        let mut v = LpcFlexcommUsart::default();
        v.restore_runtime_snapshot(&bytes).unwrap();
        assert_eq!(v.read_u32(BRG).unwrap(), 25);
        assert_eq!(v.read_u32(FIFORD).unwrap(), 9);
    }
}
