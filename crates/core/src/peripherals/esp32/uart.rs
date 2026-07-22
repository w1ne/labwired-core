// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic UART controller (UART0/1/2) — full digital twin.
//!
//! Models the real controller's TX/RX FIFOs (occupancy + baud-rate shifting),
//! FIFO-reset pulses, and the interrupt set (latched edge bits + live level
//! bits, W1C INT_CLR), so the firmware's REAL UART driver (`uart_hal`,
//! `HardwareSerial`, `ets_printf`) runs against modeled registers — not a thunk.
//! Register layout (ESP32 TRM §13, `soc/uart_reg.h`) at `DR_REG_UART{,1,2}_BASE`
//! = `0x3FF4_0000` / `0x3FF5_0000` / `0x3FF6_E000`:
//!
//! | offset | reg     | behavior                                              |
//! |--------|---------|-------------------------------------------------------|
//! | 0x00   | FIFO    | W: push TX byte. R: pop one RX byte (read consumes).  |
//! | 0x04   | INT_RAW | sticky edge bits | live RXFIFO_FULL/TXFIFO_EMPTY      |
//! | 0x08   | INT_ST  | INT_RAW & INT_ENA                                     |
//! | 0x0C   | INT_ENA | enable mask                                           |
//! | 0x10   | INT_CLR | W1C — clears the latched (edge) raw bits              |
//! | 0x14   | CLKDIV  | clkdiv[19:0] → baud = sclk / clkdiv                   |
//! | 0x1C   | STATUS  | **RXFIFO_CNT[7:0], TXFIFO_CNT[23:16]** (classic)      |
//! | 0x20   | CONF0   | RXFIFO_RST(b17) / TXFIFO_RST(b18) flush the FIFOs     |
//! | 0x24   | CONF1   | **RXFIFO_FULL_THRHD[6:0], TXFIFO_EMPTY_THRHD[14:8]**  |
//!
//! Differences from the S3 twin (`esp32s3/uart.rs`): the STATUS FIFO-count
//! fields are 8-bit (`[7:0]`/`[23:16]`, vs the S3's 10-bit `[9:0]`/`[25:16]`),
//! the CONF1 thresholds are 7-bit (`[6:0]`/`[14:8]`), the FIFO is 128 deep, and
//! the interrupt-matrix sources are UART0=34 / UART1=35 / UART2=36
//! (`ETS_UART{0,1,2}_INTR_SOURCE`). The INT bit positions match.
//!
//! TX bytes shift out one 10-bit frame every ~`10 * clkdiv` UART-clock cycles
//! (scaled to the CPU tick), emitted to the sink/stdout when they shift out.
//! `tick()` emits the UART interrupt-matrix source while `INT_ST != 0`; the bus
//! routes it through the per-core interrupt matrix.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

const OFF_FIFO: u64 = 0x00;
const OFF_INT_RAW: u64 = 0x04;
const OFF_INT_ST: u64 = 0x08;
const OFF_INT_ENA: u64 = 0x0C;
const OFF_INT_CLR: u64 = 0x10;
const OFF_CLKDIV: u64 = 0x14;
const OFF_STATUS: u64 = 0x1C;
const OFF_CONF0: u64 = 0x20;
const OFF_CONF1: u64 = 0x24;

// INT_RAW / INT_ENA / INT_ST bit positions (`uart_ll.h`, classic == S3 here).
const INT_RXFIFO_FULL: u32 = 1 << 0;
const INT_TXFIFO_EMPTY: u32 = 1 << 1;
const INT_RXFIFO_OVF: u32 = 1 << 4;
const INT_RXFIFO_TOUT: u32 = 1 << 8;
const INT_TX_DONE: u32 = 1 << 14;
/// All implemented INT bits (classic ESP32 INT regs are 19 bits wide).
const INT_MASK: u32 = 0x0007_FFFF;

// CONF0 FIFO-reset bits.
const CONF0_RXFIFO_RST: u32 = 1 << 17;
const CONF0_TXFIFO_RST: u32 = 1 << 18;

/// Hardware TX/RX FIFO depth on classic ESP32 (`SOC_UART_FIFO_LEN`).
const FIFO_LEN: usize = 128;
/// UART source clock (APB) and CPU/tick clock — scale baud timing into ticks.
const UART_SCLK_HZ: u64 = 80_000_000;
const CPU_CLOCK_HZ: u64 = 240_000_000;
/// CLKDIV reset (≈80 MHz / 115200). Used when CLKDIV reads 0.
const RESET_CLKDIV: u32 = 0x0000_02B6;

/// Config registers that are simple masked storage: (offset, reset, mask).
/// The firmware writes CLKDIV/CONF0/CONF1/INT_ENA during `uart_hal` setup and
/// reads them back; other offsets fall through to round-trip storage.
const CONFIG_REGS: &[(u64, u32, u32)] = &[
    (OFF_CLKDIV, RESET_CLKDIV, 0x00FF_FFFF),
    (OFF_CONF0, 0x0000_001C, 0xFFFF_FFFF),
    (OFF_CONF1, 0x0000_6060, 0x00FF_FFFF),
    (OFF_INT_ENA, 0x0000_0000, INT_MASK),
];

/// Shared FIFO + config state for APB window and AHB FIFO alias.
///
/// Classic ESP32 TRM / `uart_ll_write_txfifo`: TX bytes are written to
/// `UART_FIFO_AHB_REG` (0x6000_0000 / 0x6001_0000 / 0x6002_E000) while STATUS
/// / INT / CLKDIV live on the APB base (0x3FF4_0000 …). Both windows must
/// share one FIFO or `Serial.println` never reaches the sink.
#[derive(Default)]
struct UartCore {
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    regs: HashMap<u64, u32>,
    tx_fifo: VecDeque<u8>,
    rx_fifo: VecDeque<u8>,
    int_raw_sticky: u32,
    drain_accum: u64,
    tx_active: bool,
}

pub struct Esp32Uart {
    core: Arc<Mutex<UartCore>>,
    echo_stdout: bool,
    /// Interrupt-matrix source ID (UART0=34, UART1=35, UART2=36).
    source_id: u32,
}

/// AHB-bus FIFO alias (`UART_FIFO_AHB_REG(i)`). Write-only TX push into the
/// paired [`Esp32Uart`]'s shared core.
pub struct Esp32UartAhbFifo {
    core: Arc<Mutex<UartCore>>,
}

impl std::fmt::Debug for Esp32Uart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let core = self.core.lock().unwrap();
        write!(
            f,
            "Esp32Uart(src={}, tx={}, rx={}, echo={})",
            self.source_id,
            core.tx_fifo.len(),
            core.rx_fifo.len(),
            self.echo_stdout,
        )
    }
}

impl std::fmt::Debug for Esp32UartAhbFifo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Esp32UartAhbFifo")
    }
}

impl Esp32Uart {
    /// A UART instance. `echo_stdout` true routes shifted-out TX to the host
    /// console (use for UART0, the typical `Serial`); false keeps it
    /// capture-only. `source_id` is the intr-matrix source (34/35/36).
    pub fn new(echo_stdout: bool, source_id: u32) -> Self {
        let mut regs = HashMap::new();
        for &(off, reset, _mask) in CONFIG_REGS {
            regs.insert(off, reset);
        }
        Self {
            core: Arc::new(Mutex::new(UartCore {
                sink: None,
                regs,
                tx_fifo: VecDeque::new(),
                rx_fifo: VecDeque::new(),
                int_raw_sticky: 0,
                drain_accum: 0,
                tx_active: false,
            })),
            echo_stdout,
            source_id,
        }
    }

    /// AHB FIFO window paired with this APB UART (same FIFO/sink/state).
    pub fn ahb_fifo_alias(&self) -> Esp32UartAhbFifo {
        Esp32UartAhbFifo {
            core: Arc::clone(&self.core),
        }
    }

    /// Set or clear the byte-capture sink (does not change `echo_stdout`).
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>) {
        self.core.lock().unwrap().sink = sink;
    }

    /// Inject a byte into the RX FIFO (latches OVF if full, else TOUT).
    pub fn push_rx(&mut self, byte: u8) {
        let mut core = self.core.lock().unwrap();
        if core.rx_fifo.len() < FIFO_LEN {
            core.rx_fifo.push_back(byte);
            core.int_raw_sticky |= INT_RXFIFO_TOUT;
        } else {
            core.int_raw_sticky |= INT_RXFIFO_OVF;
        }
    }
}

impl UartCore {
    fn reg(&self, off: u64) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    fn is_config(off: u64) -> Option<u32> {
        CONFIG_REGS
            .iter()
            .find(|&&(o, _, _)| o == off)
            .map(|&(_, _, m)| m)
    }

    fn txfifo_empty_thrhd(&self) -> usize {
        ((self.reg(OFF_CONF1) >> 8) & 0x7F) as usize
    }

    fn rxfifo_full_thrhd(&self) -> usize {
        (self.reg(OFF_CONF1) & 0x7F) as usize
    }

    fn cycles_per_byte(&self) -> u64 {
        let clkdiv = (self.reg(OFF_CLKDIV) & 0xF_FFFF) as u64;
        let clkdiv = if clkdiv == 0 {
            RESET_CLKDIV as u64
        } else {
            clkdiv
        };
        (10 * clkdiv * CPU_CLOCK_HZ / UART_SCLK_HZ).max(1)
    }

    fn int_raw(&self) -> u32 {
        let mut v = self.int_raw_sticky;
        if self.tx_fifo.len() < self.txfifo_empty_thrhd() {
            v |= INT_TXFIFO_EMPTY;
        }
        if self.rx_fifo.len() >= self.rxfifo_full_thrhd().max(1) {
            v |= INT_RXFIFO_FULL;
        }
        v & INT_MASK
    }

    fn status_word(&self) -> u32 {
        ((self.rx_fifo.len() as u32) & 0xFF) | (((self.tx_fifo.len() as u32) & 0xFF) << 16)
    }

    fn read_reg_word(&self, word_off: u64) -> u32 {
        match word_off {
            OFF_INT_RAW => self.int_raw(),
            OFF_INT_ST => self.int_raw() & self.reg(OFF_INT_ENA),
            OFF_STATUS => self.status_word(),
            o => self.reg(o),
        }
    }

    fn pop_rx(&mut self) -> u8 {
        self.rx_fifo.pop_front().unwrap_or(0)
    }

    fn push_tx(&mut self, byte: u8) {
        if self.tx_fifo.len() < FIFO_LEN {
            self.tx_fifo.push_back(byte);
            self.tx_active = true;
        }
    }

    fn apply_conf0(&mut self, value: u32) {
        if value & CONF0_RXFIFO_RST != 0 {
            self.rx_fifo.clear();
        }
        if value & CONF0_TXFIFO_RST != 0 {
            self.tx_fifo.clear();
            self.tx_active = false;
            self.drain_accum = 0;
        }
    }

    fn write_config_reg(&mut self, off: u64, value: u32) {
        if let Some(mask) = Self::is_config(off) {
            let masked = value & mask;
            self.regs.insert(off, masked);
            if off == OFF_CONF0 {
                self.apply_conf0(masked);
            }
        } else {
            self.regs.insert(off, value);
        }
    }
}

impl Peripheral for Esp32Uart {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let mut core = self.core.lock().unwrap();
        if offset & !3 == OFF_FIFO {
            return Ok(if offset & 3 == 0 { core.pop_rx() } else { 0 });
        }
        let word = core.read_reg_word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let mut core = self.core.lock().unwrap();
        if offset & !3 == OFF_FIFO {
            return Ok(core.pop_rx() as u32);
        }
        Ok(core.read_reg_word(offset & !3))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let mut core = self.core.lock().unwrap();
        let word_off = offset & !3;
        match word_off {
            OFF_FIFO if offset & 3 == 0 => core.push_tx(value),
            OFF_INT_CLR => core.int_raw_sticky &= !((value as u32) << ((offset & 3) * 8)),
            OFF_STATUS => {}
            o => {
                let mut w = core.reg(o);
                let shift = (offset & 3) * 8;
                w &= !(0xFFu32 << shift);
                w |= (value as u32) << shift;
                core.write_config_reg(o, w);
            }
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let mut core = self.core.lock().unwrap();
        match offset & !3 {
            OFF_FIFO => core.push_tx((value & 0xFF) as u8),
            OFF_INT_CLR => core.int_raw_sticky &= !value,
            OFF_STATUS => {}
            o => core.write_config_reg(o, value),
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut core = self.core.lock().unwrap();
        if !core.tx_fifo.is_empty() {
            core.drain_accum += 1;
            let per_byte = core.cycles_per_byte();
            while core.drain_accum >= per_byte && !core.tx_fifo.is_empty() {
                core.drain_accum -= per_byte;
                if let Some(byte) = core.tx_fifo.pop_front() {
                    if let Some(sink) = &core.sink {
                        if let Ok(mut g) = sink.lock() {
                            g.push(byte);
                        }
                    }
                    if self.echo_stdout {
                        let _ = io::stdout().write_all(&[byte]);
                        let _ = io::stdout().flush();
                    }
                }
                if core.tx_fifo.is_empty() && core.tx_active {
                    core.int_raw_sticky |= INT_TX_DONE;
                    core.tx_active = false;
                }
            }
        } else {
            core.drain_accum = 0;
        }

        let asserting = core.int_raw() & core.reg(OFF_INT_ENA);
        PeripheralTickResult {
            explicit_irqs: if asserting != 0 {
                Some(vec![self.source_id])
            } else {
                None
            },
            ..PeripheralTickResult::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl Peripheral for Esp32UartAhbFifo {
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn legacy_tick_active(&self) -> bool {
        false
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        // AHB FIFO read returns RX byte (same as APB FIFO).
        let mut core = self.core.lock().unwrap();
        Ok(if offset & 3 == 0 { core.pop_rx() } else { 0 })
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let mut core = self.core.lock().unwrap();
        Ok(if offset & !3 == 0 {
            core.pop_rx() as u32
        } else {
            0
        })
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset & 3 == 0 {
            self.core.lock().unwrap().push_tx(value);
        }
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & !3 == 0 {
            self.core.lock().unwrap().push_tx((value & 0xFF) as u8);
        }
        Ok(())
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain_all(u: &mut Esp32Uart) {
        let per = {
            let core = u.core.lock().unwrap();
            core.cycles_per_byte()
        };
        for _ in 0..(per * (FIFO_LEN as u64 + 2)) {
            let empty = u.core.lock().unwrap().tx_fifo.is_empty();
            if empty {
                break;
            }
            u.tick();
        }
    }

    fn status(u: &Esp32Uart) -> u32 {
        u.core.lock().unwrap().status_word()
    }

    fn int_raw(u: &Esp32Uart) -> u32 {
        u.core.lock().unwrap().int_raw()
    }

    #[test]
    fn tx_fifo_fills_then_drains_to_sink() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut u = Esp32Uart::new(false, 34);
        u.set_sink(Some(sink.clone()));
        for &b in b"Hi!" {
            u.core.lock().unwrap().push_tx(b);
        }
        assert_eq!((status(&u) >> 16) & 0xFF, 3, "TXFIFO_CNT[23:16]=3");
        assert!(sink.lock().unwrap().is_empty(), "nothing shifted out yet");
        drain_all(&mut u);
        assert_eq!(sink.lock().unwrap().as_slice(), b"Hi!");
        assert_eq!((status(&u) >> 16) & 0xFF, 0, "FIFO drained");
    }

    #[test]
    fn rx_read_pops_and_consumes() {
        let mut u = Esp32Uart::new(false, 35);
        u.push_rx(b'A');
        u.push_rx(b'B');
        assert_eq!(status(&u) & 0xFF, 2, "RXFIFO_CNT[7:0]=2");
        assert_eq!(u.read_u32(OFF_FIFO).unwrap(), b'A' as u32, "pops A");
        assert_eq!(u.read(OFF_FIFO).unwrap(), b'B', "pops B");
        assert_eq!(status(&u) & 0xFF, 0, "RX empty");
    }

    #[test]
    fn tx_done_latches_and_clears_w1c() {
        let mut u = Esp32Uart::new(false, 34);
        u.core.lock().unwrap().push_tx(b'Z');
        drain_all(&mut u);
        assert_eq!(int_raw(&u) & INT_TX_DONE, INT_TX_DONE);
        u.write_u32(OFF_INT_CLR, INT_TX_DONE).unwrap();
        assert_eq!(int_raw(&u) & INT_TX_DONE, 0);
    }

    #[test]
    fn tx_done_raises_matrix_source_when_enabled() {
        let mut u = Esp32Uart::new(false, 34);
        u.write_u32(OFF_INT_ENA, INT_TX_DONE).unwrap();
        u.core.lock().unwrap().push_tx(b'Q');
        let per = u.core.lock().unwrap().cycles_per_byte();
        let mut irq = None;
        for _ in 0..(per * 4) {
            if let Some(v) = u.tick().explicit_irqs {
                irq = Some(v);
                break;
            }
        }
        assert_eq!(irq, Some(vec![34]), "UART0 source 34 asserted on TX_DONE");
    }

    #[test]
    fn conf0_reset_bits_flush_fifos() {
        let mut u = Esp32Uart::new(false, 34);
        u.core.lock().unwrap().push_tx(b'x');
        u.push_rx(b'y');
        u.write_u32(OFF_CONF0, CONF0_TXFIFO_RST | CONF0_RXFIFO_RST)
            .unwrap();
        let core = u.core.lock().unwrap();
        assert_eq!(core.tx_fifo.len(), 0, "TX flushed");
        assert_eq!(core.rx_fifo.len(), 0, "RX flushed");
    }

    #[test]
    fn config_registers_round_trip() {
        let mut u = Esp32Uart::new(false, 34);
        u.write_u32(OFF_CLKDIV, 0x0000_01B2).unwrap();
        u.write_u32(OFF_CONF1, 0x0000_7878).unwrap();
        assert_eq!(u.read_u32(OFF_CLKDIV).unwrap(), 0x0000_01B2);
        assert_eq!(u.read_u32(OFF_CONF1).unwrap(), 0x0000_7878 & 0x00FF_FFFF);
    }

    #[test]
    fn ahb_fifo_alias_shares_tx_fifo() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut u = Esp32Uart::new(false, 34);
        u.set_sink(Some(sink.clone()));
        let mut ahb = u.ahb_fifo_alias();
        ahb.write_u32(0, b'L' as u32).unwrap();
        ahb.write_u32(0, b'W' as u32).unwrap();
        assert_eq!((status(&u) >> 16) & 0xFF, 2);
        drain_all(&mut u);
        assert_eq!(sink.lock().unwrap().as_slice(), b"LW");
    }
}
