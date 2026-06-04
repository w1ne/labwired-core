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
//! | 0x04   | INT_RAW    | TXFIFO_EMPTY(b1)=1 always; RXFIFO_FULL(b0)=rx?;    |
//! |        |            | TX_DONE(b14)=latched after a FIFO write.          |
//! | 0x08   | INT_ST     | INT_RAW & INT_ENA                                 |
//! | 0x0C   | INT_ENA    | round-trip storage (driver enables/disables ints) |
//! | 0x10   | INT_CLR    | W1C — clears the latched TX_DONE bit              |
//! | 0x14   | CLKDIV     | round-trip storage (baud divider — sim has no PHY)|
//! | 0x1C   | STATUS     | RXFIFO_CNT[9:0]=rx; TXFIFO_CNT[25:16]=0 (room)    |
//! | 0x20   | CONF0      | round-trip storage                                |
//! | 0x24   | CONF1      | round-trip storage                                |
//!
//! ## Interrupt-driven TX
//!
//! The ESP-IDF UART driver (which Arduino `Serial`/`HardwareSerial` uses)
//! does NOT poll the FIFO for normal writes: `uart_write_bytes` copies into a
//! TX ring buffer and enables the `TXFIFO_EMPTY` interrupt; the UART ISR then
//! drains the ring into the HW FIFO (`uart_ll_write_txfifo` → `hw->fifo.val`,
//! i.e. this FIFO register) and, once empty, disables `TXFIFO_EMPTY` again. So
//! a faithful UART must raise that interrupt or no output is ever produced.
//!
//! We model `TXFIFO_EMPTY` as a *level* condition (the sim drains the TX FIFO
//! instantly, so it is always "empty/has room"): `INT_RAW.TXFIFO_EMPTY` is
//! permanently asserted, and `tick()` emits the UART interrupt source ID while
//! `INT_ST != 0` (i.e. while the firmware has enabled any asserting interrupt).
//! The bus routes that source through the per-core interrupt matrix exactly
//! like the systimer tick. When the driver's ISR has drained the ring it
//! clears `INT_ENA.TXFIFO_EMPTY`, `INT_ST` drops, and emission stops — no
//! storm, mirroring real silicon (a threshold/level interrupt is quieted by
//! masking it, not by writing INT_CLR).
//!
//! Separate, self-contained type from the STM32-layout [`crate::peripherals::
//! uart::Uart`]; the S3 address/layout never perturbs the ARM UART model.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

const OFF_FIFO: u64 = 0x00;
const OFF_INT_RAW: u64 = 0x04;
const OFF_INT_ST: u64 = 0x08;
const OFF_INT_ENA: u64 = 0x0C;
const OFF_INT_CLR: u64 = 0x10;
const OFF_STATUS: u64 = 0x1C;
/// Registers that are pure round-trip storage (no PHY-level semantics here).
/// INT_ENA (0x0C) is also stored here but additionally gates INT_ST.
const ROUND_TRIP: [u64; 4] = [OFF_INT_ENA, 0x14 /*CLKDIV*/, 0x20 /*CONF0*/, 0x24 /*CONF1*/];

// INT_RAW / INT_ENA / INT_ST bit positions (soc/hal `uart_ll.h`).
const INT_RXFIFO_FULL: u32 = 1 << 0;
const INT_TXFIFO_EMPTY: u32 = 1 << 1;
const INT_TX_DONE: u32 = 1 << 14;

#[derive(Default)]
pub struct Esp32s3Uart {
    /// Optional byte-capture sink (for tests / output assertions).
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    /// Echo TX bytes to host stdout (live console).
    echo_stdout: bool,
    /// Interrupt-matrix source ID (UART0=27, UART1=28, UART2=29).
    source_id: u32,
    /// Round-trip register storage (INT_ENA/CLKDIV/CONF0/CONF1 …).
    regs: HashMap<u64, u32>,
    /// Bytes the host has injected for the firmware to read back via FIFO.
    rx: VecDeque<u8>,
    /// Latched TX_DONE — set after a FIFO write, cleared by INT_CLR.
    tx_done: bool,
}

impl std::fmt::Debug for Esp32s3Uart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Uart(src={}, sink={}, echo_stdout={}, rx={})",
            self.source_id,
            self.sink.is_some(),
            self.echo_stdout,
            self.rx.len(),
        )
    }
}

impl Esp32s3Uart {
    /// A UART instance. `echo_stdout` true routes TX to the host console
    /// (use for UART0, the typical ESP-IDF/Arduino `Serial`); false keeps it
    /// capture-only. `source_id` is the interrupt-matrix source (27/28/29).
    pub fn new(echo_stdout: bool, source_id: u32) -> Self {
        Self {
            sink: None,
            echo_stdout,
            source_id,
            regs: HashMap::new(),
            rx: VecDeque::new(),
            tx_done: false,
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

    /// Emit one TX byte (to sink + optional stdout) and latch TX_DONE.
    fn tx_byte(&mut self, byte: u8) {
        if let Some(sink) = &self.sink {
            if let Ok(mut g) = sink.lock() {
                g.push(byte);
            }
        }
        if self.echo_stdout {
            let _ = io::stdout().write_all(&[byte]);
            let _ = io::stdout().flush();
        }
        self.tx_done = true;
    }

    /// INT_RAW (0x04): TXFIFO_EMPTY always (instant drain → FIFO has room),
    /// RXFIFO_FULL while RX bytes are queued, TX_DONE latched after a write.
    fn int_raw(&self) -> u32 {
        let mut v = INT_TXFIFO_EMPTY;
        if !self.rx.is_empty() {
            v |= INT_RXFIFO_FULL;
        }
        if self.tx_done {
            v |= INT_TX_DONE;
        }
        v
    }

    /// STATUS (0x1C): RXFIFO_CNT in bits[9:0], TXFIFO_CNT in bits[25:16].
    fn status_word(&self) -> u32 {
        (self.rx.len() as u32) & 0x3FF
    }

    fn read_word(&self, word_off: u64) -> u32 {
        match word_off {
            OFF_FIFO => self.rx.front().copied().unwrap_or(0) as u32,
            OFF_INT_RAW => self.int_raw(),
            OFF_INT_ST => self.int_raw() & self.reg(OFF_INT_ENA),
            OFF_STATUS => self.status_word(),
            o if ROUND_TRIP.contains(&o) => self.reg(o),
            _ => 0,
        }
    }
}

impl Peripheral for Esp32s3Uart {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        match word_off {
            OFF_FIFO => {
                if offset & 3 == 0 {
                    self.tx_byte(value);
                }
            }
            OFF_INT_CLR => { /* byte path: clear handled in write_u32 */ }
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
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            OFF_FIFO => self.tx_byte((value & 0xFF) as u8),
            OFF_INT_CLR => {
                // W1C — only TX_DONE is a latched bit we can clear; the level
                // conditions (TXFIFO_EMPTY / RXFIFO_FULL) re-assert until the
                // firmware masks them in INT_ENA (matches real silicon).
                if value & INT_TX_DONE != 0 {
                    self.tx_done = false;
                }
            }
            o if ROUND_TRIP.contains(&o) => {
                self.regs.insert(o, value);
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Level-sensitive: emit the UART source while any enabled interrupt is
        // asserting (INT_ST != 0). The bus routes it through the interrupt
        // matrix; the firmware's ISR drains the TX ring and clears INT_ENA,
        // which drops INT_ST and stops emission. No INT_ENA → no interrupt.
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

    #[test]
    fn fifo_write_goes_to_sink() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut u = Esp32s3Uart::new(false, 27);
        u.set_sink(Some(sink.clone()));
        for &b in b"Hi" {
            u.write(OFF_FIFO, b).unwrap();
        }
        u.write_u32(OFF_FIFO, b'!' as u32).unwrap();
        assert_eq!(sink.lock().unwrap().as_slice(), b"Hi!");
    }

    #[test]
    fn status_reports_tx_empty_and_rx_count() {
        let mut u = Esp32s3Uart::new(false, 27);
        assert_eq!(u.read_u32(OFF_STATUS).unwrap(), 0); // TX empty, no RX
        u.push_rx(b'A');
        u.push_rx(b'B');
        assert_eq!(u.read_u32(OFF_STATUS).unwrap(), 2); // RXFIFO_CNT=2
        assert_eq!(u.read_u32(OFF_STATUS).unwrap() & (0x3FF << 16), 0); // TXFIFO_CNT=0
    }

    #[test]
    fn no_interrupt_until_enabled() {
        // INT_ENA defaults 0 → no source emitted even though TXFIFO_EMPTY is raw-set.
        let mut u = Esp32s3Uart::new(false, 27);
        assert!(u.tick().explicit_irqs.is_none());
        assert_eq!(u.read_u32(OFF_INT_RAW).unwrap() & INT_TXFIFO_EMPTY, INT_TXFIFO_EMPTY);
    }

    #[test]
    fn txfifo_empty_interrupt_asserts_when_enabled_until_masked() {
        let mut u = Esp32s3Uart::new(false, 27);
        // Driver enables TXFIFO_EMPTY → INT_ST asserts → source 27 emitted (level).
        u.write_u32(OFF_INT_ENA, INT_TXFIFO_EMPTY).unwrap();
        assert_eq!(u.read_u32(OFF_INT_ST).unwrap() & INT_TXFIFO_EMPTY, INT_TXFIFO_EMPTY);
        assert_eq!(u.tick().explicit_irqs, Some(vec![27]));
        assert_eq!(u.tick().explicit_irqs, Some(vec![27]));
        // ISR drains the ring then masks the interrupt → emission stops.
        u.write_u32(OFF_INT_ENA, 0).unwrap();
        assert!(u.tick().explicit_irqs.is_none());
    }

    #[test]
    fn tx_done_latches_and_clears_via_int_clr() {
        let mut u = Esp32s3Uart::new(false, 28);
        u.write_u32(OFF_FIFO, b'X' as u32).unwrap();
        assert_eq!(u.read_u32(OFF_INT_RAW).unwrap() & INT_TX_DONE, INT_TX_DONE);
        u.write_u32(OFF_INT_CLR, INT_TX_DONE).unwrap();
        assert_eq!(u.read_u32(OFF_INT_RAW).unwrap() & INT_TX_DONE, 0);
    }

    #[test]
    fn conf_registers_round_trip() {
        let mut u = Esp32s3Uart::new(false, 27);
        u.write_u32(0x20, 0x1234_5678).unwrap(); // CONF0
        u.write_u32(0x14, 0x0000_028B).unwrap(); // CLKDIV
        assert_eq!(u.read_u32(0x20).unwrap(), 0x1234_5678);
        assert_eq!(u.read_u32(0x14).unwrap(), 0x0000_028B);
    }
}
