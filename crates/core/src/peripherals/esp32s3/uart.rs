// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 UART controller (UART0/1/2).
//!
//! Real silicon register layout (ESP32-S3 TRM §26, `soc/uart_reg.h`), at
//! `DR_REG_UART_BASE = 0x6000_0000` (UART0), `0x6001_0000` (UART1),
//! `0x6002_E000` (UART2):
//!
//! | offset | reg        | modeled behavior                                  |
//! |--------|------------|---------------------------------------------------|
//! | 0x00   | FIFO       | W: TX byte = bits[7:0] → sink/stdout. R: pop RX.   |
//! | 0x04   | INT_RAW    | read-as-zero (no pending; we drain TX instantly)  |
//! | 0x08   | INT_ST     | read-as-zero                                      |
//! | 0x0C   | INT_ENA    | round-trip storage                                |
//! | 0x10   | INT_CLR    | W1C — accepted, no-op (nothing pending)           |
//! | 0x14   | CLKDIV     | round-trip storage (baud divider — sim has no PHY)|
//! | 0x1C   | STATUS     | RXFIFO_CNT[9:0] = pending RX; TXFIFO_CNT[25:16]=0  |
//! | 0x20   | CONF0      | round-trip storage                                |
//! | 0x24   | CONF1      | round-trip storage                                |
//!
//! This is the faithful counterpart to the legacy STM32F1-layout [`crate::
//! peripherals::uart::Uart`]; it is a **separate, self-contained type** so the
//! ESP32-S3 UART address/layout never perturbs the ARM/STM32 UART model.
//!
//! TX is drained instantly: `STATUS.TXFIFO_CNT` always reads 0 (room
//! available) so a driver's "wait for FIFO space" / "wait for TX empty" poll
//! exits immediately — matching a real UART that has clocked out the byte.

use crate::{Peripheral, SimResult};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

const OFF_FIFO: u64 = 0x00;
const OFF_INT_CLR: u64 = 0x10;
const OFF_STATUS: u64 = 0x1C;
/// Registers we simply round-trip (no PHY-level semantics in the sim).
const ROUND_TRIP: [u64; 4] = [0x0C /*INT_ENA*/, 0x14 /*CLKDIV*/, 0x20 /*CONF0*/, 0x24 /*CONF1*/];

#[derive(Default)]
pub struct Esp32s3Uart {
    /// Optional byte-capture sink (for tests / output assertions).
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    /// Echo TX bytes to host stdout (live console).
    echo_stdout: bool,
    /// Round-trip register storage (CONF0/CONF1/CLKDIV/INT_ENA …).
    regs: HashMap<u64, u32>,
    /// Bytes the host has injected for the firmware to read back via FIFO.
    rx: VecDeque<u8>,
}

impl std::fmt::Debug for Esp32s3Uart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Uart(sink={}, echo_stdout={}, rx={})",
            self.sink.is_some(),
            self.echo_stdout,
            self.rx.len(),
        )
    }
}

impl Esp32s3Uart {
    /// A UART instance. `echo_stdout` true routes TX to the host console
    /// (use for UART0, the typical ESP-IDF/Arduino `Serial`); false keeps it
    /// capture-only.
    pub fn new(echo_stdout: bool) -> Self {
        Self {
            sink: None,
            echo_stdout,
            regs: HashMap::new(),
            rx: VecDeque::new(),
        }
    }

    /// Set or clear the byte-capture sink (does not change `echo_stdout`).
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>) {
        self.sink = sink;
    }

    /// Inject a byte for the firmware to read back via the RX FIFO.
    pub fn push_rx(&mut self, byte: u8) {
        self.rx.push_back(byte);
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    /// STATUS (0x1C): RXFIFO_CNT in bits[9:0], TXFIFO_CNT in bits[25:16].
    /// TX FIFO is always empty (instant drain); RX count reflects `rx`.
    fn status_word(&self) -> u32 {
        (self.rx.len() as u32) & 0x3FF
    }
}

impl Peripheral for Esp32s3Uart {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte = (offset & 3) * 8;
        let word = match word_off {
            OFF_FIFO => self.rx.front().copied().unwrap_or(0) as u32,
            OFF_STATUS => self.status_word(),
            o if ROUND_TRIP.contains(&o) => self.reg(o),
            _ => 0,
        };
        Ok(((word >> byte) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        match word_off {
            OFF_FIFO => {
                // Only the low byte of a FIFO write is the data byte.
                if offset & 3 == 0 {
                    if let Some(sink) = &self.sink {
                        if let Ok(mut g) = sink.lock() {
                            g.push(value);
                        }
                    }
                    if self.echo_stdout {
                        let _ = io::stdout().write_all(&[value]);
                        let _ = io::stdout().flush();
                    }
                }
            }
            OFF_INT_CLR => { /* W1C: nothing pending to clear */ }
            o if ROUND_TRIP.contains(&o) => {
                let mut w = self.reg(o);
                let shift = (offset & 3) * 8;
                w &= !(0xFFu32 << shift);
                w |= (value as u32) << shift;
                self.regs.insert(o, w);
            }
            _ => {}
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset & !3 {
            OFF_FIFO => self.rx.front().copied().unwrap_or(0) as u32,
            OFF_STATUS => self.status_word(),
            o if ROUND_TRIP.contains(&o) => self.reg(o),
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            OFF_FIFO => {
                let byte = (value & 0xFF) as u8;
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
            OFF_INT_CLR => {}
            o if ROUND_TRIP.contains(&o) => {
                self.regs.insert(o, value);
            }
            _ => {}
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

    #[test]
    fn fifo_write_goes_to_sink() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut u = Esp32s3Uart::new(false);
        u.set_sink(Some(sink.clone()));
        for &b in b"Hi" {
            u.write(OFF_FIFO, b).unwrap();
        }
        u.write_u32(OFF_FIFO, b'!' as u32).unwrap();
        assert_eq!(sink.lock().unwrap().as_slice(), b"Hi!");
    }

    #[test]
    fn status_reports_tx_empty_and_rx_count() {
        let mut u = Esp32s3Uart::new(false);
        // TX always empty (bits[25:16] == 0) so writers never block; no RX yet.
        assert_eq!(u.read_u32(OFF_STATUS).unwrap(), 0);
        u.push_rx(b'A');
        u.push_rx(b'B');
        // RXFIFO_CNT == 2 in bits[9:0]; TXFIFO_CNT still 0.
        assert_eq!(u.read_u32(OFF_STATUS).unwrap(), 2);
        assert_eq!(u.read_u32(OFF_STATUS).unwrap() & (0x3FF << 16), 0);
    }

    #[test]
    fn rx_fifo_read_pops_front() {
        let mut u = Esp32s3Uart::new(false);
        u.push_rx(b'X');
        assert_eq!(u.read(OFF_FIFO).unwrap(), b'X');
    }

    #[test]
    fn conf_registers_round_trip() {
        let mut u = Esp32s3Uart::new(false);
        u.write_u32(0x20, 0x1234_5678).unwrap(); // CONF0
        u.write_u32(0x14, 0x0000_028B).unwrap(); // CLKDIV
        assert_eq!(u.read_u32(0x20).unwrap(), 0x1234_5678);
        assert_eq!(u.read_u32(0x14).unwrap(), 0x0000_028B);
    }
}
