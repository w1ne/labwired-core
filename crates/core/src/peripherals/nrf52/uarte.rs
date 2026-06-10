// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 UARTE (UART with EasyDMA).
//!
//! Source: nRF52840 PS rev 1.7 §6.33 (UARTE). Models the full register
//! surface including PSEL, BAUDRATE, CONFIG and DMA pointer/maxcnt/amount
//! registers used by zephyr/nrfx drivers. Dynamic operation (actual byte
//! transfer, DMA) is not modelled — firmware that programs the peripheral
//! and reads config registers back will see its writes round-trip.
//!
//! EVENTS: hardware-generated. SW write-1 is ignored; write-0 clears.

use crate::{Peripheral, SimResult};
use std::collections::BTreeMap;

// Task offsets (read-as-0, task starts on write-1)
const OFF_TASKS_STARTRX: u64 = 0x000;
const OFF_TASKS_STOPRX: u64 = 0x004;
const OFF_TASKS_STARTTX: u64 = 0x008;
const OFF_TASKS_STOPTX: u64 = 0x00C;
const OFF_TASKS_FLUSHRX: u64 = 0x02C;

// Event offsets (0x100..0x17C)
const OFF_EVENTS_CTS: u64 = 0x100;
const OFF_EVENTS_NCTS: u64 = 0x104;
const OFF_EVENTS_RXDRDY: u64 = 0x108;
const OFF_EVENTS_ENDRX: u64 = 0x110;
const OFF_EVENTS_TXDRDY: u64 = 0x11C;
const OFF_EVENTS_ENDTX: u64 = 0x120;
const OFF_EVENTS_ERROR: u64 = 0x124;
const OFF_EVENTS_RXTO: u64 = 0x144;
const OFF_EVENTS_RXSTARTED: u64 = 0x14C;
const OFF_EVENTS_TXSTARTED: u64 = 0x150;
const OFF_EVENTS_TXSTOPPED: u64 = 0x158;

// Interrupt registers
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;

// Error source (write-1-clear)
const OFF_ERRORSRC: u64 = 0x480;

// Enable
const OFF_ENABLE: u64 = 0x500;

// PSEL block (0x508..0x518): RTS, TXD, CTS, RXD — reset value = 0xFFFF_FFFF (disconnected)
const OFF_PSEL_RTS: u64 = 0x508;
const OFF_PSEL_TXD: u64 = 0x50C;
const OFF_PSEL_CTS: u64 = 0x510;
const OFF_PSEL_RXD: u64 = 0x514;

// BAUDRATE — reset value is BAUD_115200 = 0x01D7E000
const OFF_BAUDRATE: u64 = 0x524;

// RXD EasyDMA block
const OFF_RXD_PTR: u64 = 0x534;
const OFF_RXD_MAXCNT: u64 = 0x538;
const OFF_RXD_AMOUNT: u64 = 0x53C;

// TXD EasyDMA block
const OFF_TXD_PTR: u64 = 0x544;
const OFF_TXD_MAXCNT: u64 = 0x548;
const OFF_TXD_AMOUNT: u64 = 0x54C;

// CONFIG: bits [3:0] = hwfc|parity, bit 4 = paritytype; reset = 0
const OFF_CONFIG: u64 = 0x56C;

#[derive(Debug, Default)]
pub struct Nrf52Uarte {
    // Events (TASKS always read 0)
    events_cts: u32,
    events_ncts: u32,
    events_rxdrdy: u32,
    events_endrx: u32,
    events_txdrdy: u32,
    events_endtx: u32,
    events_error: u32,
    events_rxto: u32,
    events_rxstarted: u32,
    events_txstarted: u32,
    events_txstopped: u32,
    // Config / status
    inten: u32,
    errorsrc: u32,
    enable: u32,
    psel_rts: u32,
    psel_txd: u32,
    psel_cts: u32,
    psel_rxd: u32,
    baudrate: u32,
    // DMA registers (all read-write, no side effects in sim)
    rxd_ptr: u32,
    rxd_maxcnt: u32,
    rxd_amount: u32,
    txd_ptr: u32,
    txd_maxcnt: u32,
    txd_amount: u32,
    config: u32,
    // Overflow bucket for any unmodelled register
    extra: BTreeMap<u64, u32>,
}

impl Nrf52Uarte {
    pub fn new() -> Self {
        Self {
            // PSELs reset to disconnected (all bits set = 0xFFFF_FFFF)
            psel_rts: 0xFFFF_FFFF,
            psel_txd: 0xFFFF_FFFF,
            psel_cts: 0xFFFF_FFFF,
            psel_rxd: 0xFFFF_FFFF,
            // BAUDRATE reset: BAUD_115200
            baudrate: 0x01D7_E000,
            ..Self::default()
        }
    }
}

impl Peripheral for Nrf52Uarte {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // Tasks: always read 0
            OFF_TASKS_STARTRX | OFF_TASKS_STOPRX | OFF_TASKS_STARTTX | OFF_TASKS_STOPTX
            | OFF_TASKS_FLUSHRX => 0,
            // Events
            OFF_EVENTS_CTS => self.events_cts,
            OFF_EVENTS_NCTS => self.events_ncts,
            OFF_EVENTS_RXDRDY => self.events_rxdrdy,
            OFF_EVENTS_ENDRX => self.events_endrx,
            OFF_EVENTS_TXDRDY => self.events_txdrdy,
            OFF_EVENTS_ENDTX => self.events_endtx,
            OFF_EVENTS_ERROR => self.events_error,
            OFF_EVENTS_RXTO => self.events_rxto,
            OFF_EVENTS_RXSTARTED => self.events_rxstarted,
            OFF_EVENTS_TXSTARTED => self.events_txstarted,
            OFF_EVENTS_TXSTOPPED => self.events_txstopped,
            // Interrupts
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            // Status
            OFF_ERRORSRC => self.errorsrc,
            OFF_ENABLE => self.enable & 0xF,
            // PSEL
            OFF_PSEL_RTS => self.psel_rts,
            OFF_PSEL_TXD => self.psel_txd,
            OFF_PSEL_CTS => self.psel_cts,
            OFF_PSEL_RXD => self.psel_rxd,
            // BAUDRATE
            OFF_BAUDRATE => self.baudrate,
            // DMA
            OFF_RXD_PTR => self.rxd_ptr,
            OFF_RXD_MAXCNT => self.rxd_maxcnt & 0xFFFF,
            OFF_RXD_AMOUNT => self.rxd_amount & 0xFFFF,
            OFF_TXD_PTR => self.txd_ptr,
            OFF_TXD_MAXCNT => self.txd_maxcnt & 0xFFFF,
            OFF_TXD_AMOUNT => self.txd_amount & 0xFFFF,
            // CONFIG: bits [4:0]
            OFF_CONFIG => self.config & 0x1F,
            _ => self.extra.get(&offset).copied().unwrap_or(0),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // Tasks: write-only trigger, no state change needed for register surface
            OFF_TASKS_STARTRX | OFF_TASKS_STOPRX | OFF_TASKS_STARTTX | OFF_TASKS_STOPTX
            | OFF_TASKS_FLUSHRX => {}
            // EVENTS: hardware-generated; SW write-1 ignored, write-0 clears
            OFF_EVENTS_CTS if value == 0 => self.events_cts = 0,
            OFF_EVENTS_NCTS if value == 0 => self.events_ncts = 0,
            OFF_EVENTS_RXDRDY if value == 0 => self.events_rxdrdy = 0,
            OFF_EVENTS_ENDRX if value == 0 => self.events_endrx = 0,
            OFF_EVENTS_TXDRDY if value == 0 => self.events_txdrdy = 0,
            OFF_EVENTS_ENDTX if value == 0 => self.events_endtx = 0,
            OFF_EVENTS_ERROR if value == 0 => self.events_error = 0,
            OFF_EVENTS_RXTO if value == 0 => self.events_rxto = 0,
            OFF_EVENTS_RXSTARTED if value == 0 => self.events_rxstarted = 0,
            OFF_EVENTS_TXSTARTED if value == 0 => self.events_txstarted = 0,
            OFF_EVENTS_TXSTOPPED if value == 0 => self.events_txstopped = 0,
            // Interrupts
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            // ERRORSRC: write-1-clear
            OFF_ERRORSRC => self.errorsrc &= !value,
            // Enable
            OFF_ENABLE => self.enable = value & 0xF,
            // PSEL
            OFF_PSEL_RTS => self.psel_rts = value,
            OFF_PSEL_TXD => self.psel_txd = value,
            OFF_PSEL_CTS => self.psel_cts = value,
            OFF_PSEL_RXD => self.psel_rxd = value,
            // BAUDRATE
            OFF_BAUDRATE => self.baudrate = value,
            // DMA
            OFF_RXD_PTR => self.rxd_ptr = value,
            OFF_RXD_MAXCNT => self.rxd_maxcnt = value & 0xFFFF,
            OFF_RXD_AMOUNT => {} // RO, driven by DMA hardware
            OFF_TXD_PTR => self.txd_ptr = value,
            OFF_TXD_MAXCNT => self.txd_maxcnt = value & 0xFFFF,
            OFF_TXD_AMOUNT => {} // RO
            // CONFIG
            OFF_CONFIG => self.config = value & 0x1F,
            _ => {
                self.extra.insert(offset, value);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psel_defaults_to_disconnected() {
        let u = Nrf52Uarte::new();
        assert_eq!(u.read_u32(OFF_PSEL_TXD).unwrap(), 0xFFFF_FFFF);
        assert_eq!(u.read_u32(OFF_PSEL_RXD).unwrap(), 0xFFFF_FFFF);
    }

    #[test]
    fn baudrate_reset_is_115200() {
        let u = Nrf52Uarte::new();
        assert_eq!(u.read_u32(OFF_BAUDRATE).unwrap(), 0x01D7_E000);
    }

    #[test]
    fn psel_txd_roundtrips() {
        let mut u = Nrf52Uarte::new();
        u.write_u32(OFF_PSEL_TXD, 6).unwrap();
        assert_eq!(u.read_u32(OFF_PSEL_TXD).unwrap(), 6);
    }

    #[test]
    fn events_write_1_ignored() {
        let mut u = Nrf52Uarte::new();
        u.write_u32(OFF_EVENTS_TXDRDY, 1).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 0);
    }

    #[test]
    fn events_write_0_clears() {
        let mut u = Nrf52Uarte::new();
        // Simulate HW setting event (by direct field access in test)
        u.events_txdrdy = 1;
        u.write_u32(OFF_EVENTS_TXDRDY, 0).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 0);
    }
}
