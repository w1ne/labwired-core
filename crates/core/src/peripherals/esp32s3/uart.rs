// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 UART controller (UART0/1/2) — full digital twin.
//!
//! Models the real controller's TX/RX FIFOs (occupancy + baud-rate shifting),
//! FIFO-reset pulses, and the full interrupt set (latched edge bits + live
//! level bits, W1C INT_CLR). Register layout (ESP32-S3 TRM §26,
//! `soc/uart_reg.h`), at `DR_REG_UART{,1,2}_BASE` = `0x6000_0000` /
//! `0x6001_0000` / `0x6002_E000`:
//!
//! | offset | reg     | behavior                                              |
//! |--------|---------|-------------------------------------------------------|
//! | 0x00   | FIFO    | W: push TX byte. R: pop one RX byte (read consumes).  |
//! | 0x04   | INT_RAW | sticky edge bits | live RXFIFO_FULL/TXFIFO_EMPTY      |
//! | 0x08   | INT_ST  | INT_RAW & INT_ENA                                     |
//! | 0x0C   | INT_ENA | enable mask                                           |
//! | 0x10   | INT_CLR | W1C — clears the latched (edge) raw bits              |
//! | 0x14   | CLKDIV  | clkdiv[11:0] → baud = sclk / clkdiv                   |
//! | 0x1C   | STATUS  | RXFIFO_CNT[9:0], TXFIFO_CNT[25:16] — live occupancy   |
//! | 0x20   | CONF0   | RXFIFO_RST(b17) / TXFIFO_RST(b18) flush the FIFOs     |
//! | 0x24   | CONF1   | RXFIFO_FULL_THRHD[9:0], TXFIFO_EMPTY_THRHD[19:10]     |
//!
//! ## FIFOs + baud timing
//!
//! A 128-entry (`SOC_UART_FIFO_LEN`) TX FIFO: a write enqueues a byte,
//! `STATUS.TXFIFO_CNT` reflects true occupancy, and the transmitter shifts one
//! 10-bit frame out every ~`10 * clkdiv` UART-source-clock cycles (scaled to
//! the CPU tick rate — 1 sim tick ≈ 1 CPU cycle, as the systimer assumes). The
//! byte is emitted to the sink/stdout when it shifts out, not when written.
//! Writing a full TX FIFO drops the byte (no flow control). The RX FIFO is fed
//! by `push_rx`; a FIFO read pops one byte (the hardware read side-effect),
//! using interior mutability since `Peripheral::read` takes `&self`. Pushing a
//! full RX FIFO latches `RXFIFO_OVF`.
//!
//! ## Interrupts (`uart_ll.h` bit positions)
//!
//! * Level (recomputed each read from FIFO state, not latched):
//!   `TXFIFO_EMPTY`(b1) while `TXFIFO_CNT < TXFIFO_EMPTY_THRHD`;
//!   `RXFIFO_FULL`(b0) while `RXFIFO_CNT >= RXFIFO_FULL_THRHD`.
//! * Latched edge (set on the event, cleared W1C via INT_CLR):
//!   `TX_DONE`(b14) when the FIFO empties; `RXFIFO_OVF`(b4) on RX overflow;
//!   `RXFIFO_TOUT`(b8) when RX data is waiting.
//!
//! `tick()` emits the UART interrupt-matrix source (UART0=27/1=28/2=29) while
//! `INT_ST != 0`; the bus routes it through the per-core interrupt matrix like
//! the systimer tick. Self-contained type, distinct from the STM32 `Uart`.
//!
//! HW-verified (JTAG): after the same firmware runs, UART0 CLKDIV/CONF0/CONF1
//! read back byte-identical to real ESP32-S3 silicon.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::RefCell;
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
/// Pure round-trip config registers (interpreted elsewhere).
const ROUND_TRIP: [u64; 4] = [OFF_INT_ENA, OFF_CLKDIV, OFF_CONF0, OFF_CONF1];

// INT_RAW / INT_ENA / INT_ST bit positions (`uart_ll.h`).
const INT_RXFIFO_FULL: u32 = 1 << 0;
const INT_TXFIFO_EMPTY: u32 = 1 << 1;
const INT_RXFIFO_OVF: u32 = 1 << 4;
const INT_RXFIFO_TOUT: u32 = 1 << 8;
const INT_TX_DONE: u32 = 1 << 14;
/// Live (non-latched) interrupt conditions, recomputed from FIFO state.
#[allow(dead_code)]
const LEVEL_BITS: u32 = INT_RXFIFO_FULL | INT_TXFIFO_EMPTY;

// CONF0 FIFO-reset bits.
const CONF0_RXFIFO_RST: u32 = 1 << 17;
const CONF0_TXFIFO_RST: u32 = 1 << 18;

/// Hardware TX/RX FIFO depth (`SOC_UART_FIFO_LEN`).
const FIFO_LEN: usize = 128;
/// UART source clock (APB) and CPU/tick clock — scale baud timing into ticks.
const UART_SCLK_HZ: u64 = 80_000_000;
const CPU_CLOCK_HZ: u64 = 240_000_000;
/// Reset defaults from `uart_reg.h`: CLKDIV=694 (115200 @ 80 MHz), thrhd=96.
const RESET_CLKDIV: u32 = 694;
const RESET_THRHD: u32 = 96;

#[derive(Default)]
pub struct Esp32s3Uart {
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
    /// Interrupt-matrix source ID (UART0=27, UART1=28, UART2=29).
    source_id: u32,
    /// Round-trip config register storage (INT_ENA/CLKDIV/CONF0/CONF1).
    regs: HashMap<u64, u32>,
    /// TX FIFO (≤ FIFO_LEN); shifts out at the baud rate.
    tx_fifo: VecDeque<u8>,
    /// RX FIFO; a FIFO read pops one byte (read side-effect → interior mut).
    rx_fifo: RefCell<VecDeque<u8>>,
    /// Latched edge interrupt bits (TX_DONE, RXFIFO_OVF, RXFIFO_TOUT …).
    int_raw_sticky: u32,
    /// Sub-byte cycle accumulator for baud-rate draining.
    drain_accum: u64,
    /// True while TX bytes are in flight (so emptying is a TX_DONE edge).
    tx_active: bool,
}

impl std::fmt::Debug for Esp32s3Uart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Uart(src={}, tx={}, rx={}, echo={})",
            self.source_id,
            self.tx_fifo.len(),
            self.rx_fifo.borrow().len(),
            self.echo_stdout,
        )
    }
}

impl Esp32s3Uart {
    /// A UART instance. `echo_stdout` true routes shifted-out TX to the host
    /// console (use for UART0, the typical ESP-IDF/Arduino `Serial`); false
    /// keeps it capture-only. `source_id` is the intr-matrix source (27/28/29).
    pub fn new(echo_stdout: bool, source_id: u32) -> Self {
        let mut regs = HashMap::new();
        regs.insert(OFF_CLKDIV, RESET_CLKDIV);
        regs.insert(OFF_CONF1, RESET_THRHD | (RESET_THRHD << 10));
        Self {
            sink: None,
            echo_stdout,
            source_id,
            regs,
            tx_fifo: VecDeque::new(),
            rx_fifo: RefCell::new(VecDeque::new()),
            int_raw_sticky: 0,
            drain_accum: 0,
            tx_active: false,
        }
    }

    /// Set or clear the byte-capture sink (does not change `echo_stdout`).
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>) {
        self.sink = sink;
    }

    /// Inject a byte into the RX FIFO. Latches RXFIFO_OVF if the FIFO is full
    /// (the byte is dropped, as on silicon); otherwise latches RXFIFO_TOUT to
    /// signal waiting data.
    pub fn push_rx(&mut self, byte: u8) {
        let mut rx = self.rx_fifo.borrow_mut();
        if rx.len() < FIFO_LEN {
            rx.push_back(byte);
            self.int_raw_sticky |= INT_RXFIFO_TOUT;
        } else {
            self.int_raw_sticky |= INT_RXFIFO_OVF;
        }
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    fn txfifo_empty_thrhd(&self) -> usize {
        ((self.reg(OFF_CONF1) >> 10) & 0x3FF) as usize
    }

    fn rxfifo_full_thrhd(&self) -> usize {
        (self.reg(OFF_CONF1) & 0x3FF) as usize
    }

    /// Sim ticks to shift one 10-bit frame: `10 * clkdiv` UART-clock cycles,
    /// scaled to CPU-clock ticks. Clamped to ≥1 so progress is always made.
    fn cycles_per_byte(&self) -> u64 {
        let clkdiv = (self.reg(OFF_CLKDIV) & 0xFFF) as u64;
        let clkdiv = if clkdiv == 0 {
            RESET_CLKDIV as u64
        } else {
            clkdiv
        };
        (10 * clkdiv * CPU_CLOCK_HZ / UART_SCLK_HZ).max(1)
    }

    /// INT_RAW = latched edge bits OR the live level conditions.
    fn int_raw(&self) -> u32 {
        let mut v = self.int_raw_sticky;
        if self.tx_fifo.len() < self.txfifo_empty_thrhd() {
            v |= INT_TXFIFO_EMPTY;
        }
        if self.rx_fifo.borrow().len() >= self.rxfifo_full_thrhd().max(1) {
            v |= INT_RXFIFO_FULL;
        }
        v
    }

    /// STATUS (0x1C): live RXFIFO_CNT[9:0] + TXFIFO_CNT[25:16].
    fn status_word(&self) -> u32 {
        ((self.rx_fifo.borrow().len() as u32) & 0x3FF)
            | (((self.tx_fifo.len() as u32) & 0x3FF) << 16)
    }

    /// Word value for a non-FIFO register read.
    fn read_reg_word(&self, word_off: u64) -> u32 {
        match word_off {
            OFF_INT_RAW => self.int_raw(),
            OFF_INT_ST => self.int_raw() & self.reg(OFF_INT_ENA),
            OFF_STATUS => self.status_word(),
            o if ROUND_TRIP.contains(&o) => self.reg(o),
            _ => 0,
        }
    }

    /// Pop one RX byte (the FIFO-read side effect).
    fn pop_rx(&self) -> u8 {
        self.rx_fifo.borrow_mut().pop_front().unwrap_or(0)
    }

    fn push_tx(&mut self, byte: u8) {
        if self.tx_fifo.len() < FIFO_LEN {
            self.tx_fifo.push_back(byte);
            self.tx_active = true;
        }
    }

    /// Apply a CONF0 write: the RXFIFO_RST/TXFIFO_RST pulse bits flush a FIFO.
    fn apply_conf0(&mut self, value: u32) {
        if value & CONF0_RXFIFO_RST != 0 {
            self.rx_fifo.borrow_mut().clear();
        }
        if value & CONF0_TXFIFO_RST != 0 {
            self.tx_fifo.clear();
            self.tx_active = false;
            self.drain_accum = 0;
        }
    }
}

impl Peripheral for Esp32s3Uart {
    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset & !3 == OFF_FIFO {
            // Only the low byte carries RX data and consumes a FIFO entry.
            return Ok(if offset & 3 == 0 { self.pop_rx() } else { 0 });
        }
        let word = self.read_reg_word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset & !3 == OFF_FIFO {
            return Ok(self.pop_rx() as u32);
        }
        Ok(self.read_reg_word(offset & !3))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        match word_off {
            OFF_FIFO if offset & 3 == 0 => self.push_tx(value),
            OFF_INT_CLR => self.int_raw_sticky &= !((value as u32) << ((offset & 3) * 8)),
            o if ROUND_TRIP.contains(&o) => {
                let mut w = self.reg(o);
                let shift = (offset & 3) * 8;
                w &= !(0xFFu32 << shift);
                w |= (value as u32) << shift;
                self.regs.insert(o, w);
                if o == OFF_CONF0 {
                    self.apply_conf0(w);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            OFF_FIFO => self.push_tx((value & 0xFF) as u8),
            OFF_INT_CLR => self.int_raw_sticky &= !value, // W1C
            OFF_CONF0 => {
                self.regs.insert(OFF_CONF0, value);
                self.apply_conf0(value);
            }
            o if ROUND_TRIP.contains(&o) => {
                self.regs.insert(o, value);
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Shift TX bytes out at the baud rate.
        if !self.tx_fifo.is_empty() {
            self.drain_accum += 1;
            let per_byte = self.cycles_per_byte();
            while self.drain_accum >= per_byte && !self.tx_fifo.is_empty() {
                self.drain_accum -= per_byte;
                if let Some(byte) = self.tx_fifo.pop_front() {
                    if let Some(sink) = &self.sink {
                        if let Ok(mut g) = sink.lock() {
                            g.push(byte);
                        }
                    }
                    if self.echo_stdout {
                        let _ = io::stdout().write_all(&[byte]);
                        let _ = io::stdout().flush();
                    }
                }
                if self.tx_fifo.is_empty() && self.tx_active {
                    self.int_raw_sticky |= INT_TX_DONE; // last byte shifted out
                    self.tx_active = false;
                }
            }
        } else {
            self.drain_accum = 0;
        }

        let asserting = self.int_raw() & self.reg(OFF_INT_ENA);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn drain_all(u: &mut Esp32s3Uart) {
        for _ in 0..(u.cycles_per_byte() * (FIFO_LEN as u64 + 2)) {
            if u.tx_fifo.is_empty() {
                break;
            }
            u.tick();
        }
    }

    #[test]
    fn tx_fifo_fills_then_drains_to_sink() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut u = Esp32s3Uart::new(false, 27);
        u.set_sink(Some(sink.clone()));
        for &b in b"Hi!" {
            u.push_tx(b);
        }
        assert_eq!(
            (u.status_word() >> 16) & 0x3FF,
            3,
            "TXFIFO_CNT reflects fill"
        );
        assert!(sink.lock().unwrap().is_empty(), "nothing shifted out yet");
        drain_all(&mut u);
        assert_eq!(sink.lock().unwrap().as_slice(), b"Hi!");
        assert_eq!((u.status_word() >> 16) & 0x3FF, 0, "FIFO drained");
    }

    #[test]
    fn fifo_full_drops_excess() {
        let mut u = Esp32s3Uart::new(false, 27);
        for i in 0..(FIFO_LEN + 10) {
            u.push_tx(i as u8);
        }
        assert_eq!(u.tx_fifo.len(), FIFO_LEN);
    }

    #[test]
    fn rx_read_pops_and_consumes() {
        let mut u = Esp32s3Uart::new(false, 28);
        u.push_rx(b'A');
        u.push_rx(b'B');
        assert_eq!(u.status_word() & 0x3FF, 2, "RXFIFO_CNT=2");
        assert_eq!(
            u.read_u32(OFF_FIFO).unwrap(),
            b'A' as u32,
            "first read pops A"
        );
        assert_eq!(u.status_word() & 0x3FF, 1, "one consumed");
        assert_eq!(u.read(OFF_FIFO).unwrap(), b'B', "byte read pops B");
        assert_eq!(u.status_word() & 0x3FF, 0, "RX FIFO empty");
        assert_eq!(u.read_u32(OFF_FIFO).unwrap(), 0, "empty read → 0");
    }

    #[test]
    fn txfifo_empty_is_level_below_threshold() {
        let mut u = Esp32s3Uart::new(false, 27);
        assert_eq!(u.int_raw() & INT_TXFIFO_EMPTY, INT_TXFIFO_EMPTY);
        u.write_u32(OFF_INT_ENA, INT_TXFIFO_EMPTY).unwrap();
        assert_eq!(u.tick().explicit_irqs, Some(vec![27]));
        for i in 0..100u8 {
            u.push_tx(i);
        }
        assert_eq!(u.int_raw() & INT_TXFIFO_EMPTY, 0, "above threshold");
    }

    #[test]
    fn tx_done_latches_and_clears_via_int_clr_w1c() {
        let mut u = Esp32s3Uart::new(false, 27);
        u.push_tx(b'Z');
        assert!(!u.tx_active || u.int_raw() & INT_TX_DONE == 0);
        drain_all(&mut u);
        assert_eq!(u.int_raw() & INT_TX_DONE, INT_TX_DONE);
        u.write_u32(OFF_INT_CLR, INT_TX_DONE).unwrap();
        assert_eq!(u.int_raw() & INT_TX_DONE, 0);
    }

    #[test]
    fn rx_overflow_latches_ovf() {
        let mut u = Esp32s3Uart::new(false, 27);
        for i in 0..(FIFO_LEN + 5) {
            u.push_rx(i as u8);
        }
        assert_eq!(u.rx_fifo.borrow().len(), FIFO_LEN, "RX capped");
        assert_eq!(
            u.int_raw() & INT_RXFIFO_OVF,
            INT_RXFIFO_OVF,
            "overflow latched"
        );
        u.write_u32(OFF_INT_CLR, INT_RXFIFO_OVF).unwrap();
        assert_eq!(u.int_raw() & INT_RXFIFO_OVF, 0);
    }

    #[test]
    fn conf0_reset_bits_flush_fifos() {
        let mut u = Esp32s3Uart::new(false, 27);
        u.push_tx(b'x');
        u.push_rx(b'y');
        // TXFIFO_RST (b18) + RXFIFO_RST (b17) pulse → both FIFOs cleared.
        u.write_u32(OFF_CONF0, CONF0_TXFIFO_RST | CONF0_RXFIFO_RST)
            .unwrap();
        assert_eq!(u.tx_fifo.len(), 0, "TX FIFO flushed");
        assert_eq!(u.rx_fifo.borrow().len(), 0, "RX FIFO flushed");
    }

    #[test]
    fn baud_timing_scales_with_clkdiv() {
        let u = Esp32s3Uart::new(false, 27);
        assert_eq!(u.cycles_per_byte(), 20820); // 115200: 10*694*240/80
    }

    #[test]
    fn config_registers_round_trip() {
        let mut u = Esp32s3Uart::new(false, 27);
        u.write_u32(OFF_CLKDIV, 0x0030_015b).unwrap();
        u.write_u32(OFF_CONF1, 0x0080_0078).unwrap();
        assert_eq!(u.read_u32(OFF_CLKDIV).unwrap(), 0x0030_015b);
        assert_eq!(u.read_u32(OFF_CONF1).unwrap(), 0x0080_0078);
    }
}
