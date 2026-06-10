// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 TWIS (I2C Slave) peripheral — register-surface model.
//!
//! Source: nRF52840 Product Specification rev 1.7 §6.29 (TWIS).
//!
//! TWIS0 shares a physical base address with TWIM0/TWI0 (0x40003000).
//! TWIS1 shares a physical base address with TWIM1/TWI1 (0x40004000).
//! The peripheral operates in slave mode when ENABLE = 0x09 (value 9).
//!
//! EVENTS_* semantics: hardware-generated only. Writes of 1 are ignored;
//! only writes of 0 clear the event register — matching TIMER/RTC silicon
//! behaviour confirmed by the hw-oracle sweep.
//!
//! TASKS registers read as zero (write-only strobes on silicon).
//! ERRORSRC is W1C (write-1 clears an error flag). MATCH is read-only.

use crate::{Peripheral, SimResult};

// ── Register offsets (PS §6.29 register map) ─────────────────────────────────

const OFF_TASKS_STOP: u64 = 0x014;
const OFF_TASKS_SUSPEND: u64 = 0x01C;
const OFF_TASKS_RESUME: u64 = 0x020;
const OFF_TASKS_PREPARERX: u64 = 0x030;
const OFF_TASKS_PREPARETX: u64 = 0x034;
const OFF_EVENTS_STOPPED: u64 = 0x104;
const OFF_EVENTS_ERROR: u64 = 0x124;
const OFF_EVENTS_RXSTARTED: u64 = 0x14C;
const OFF_EVENTS_TXSTARTED: u64 = 0x150;
const OFF_EVENTS_WRITE: u64 = 0x164;
const OFF_EVENTS_READ: u64 = 0x168;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_ERRORSRC: u64 = 0x4D0;
const OFF_MATCH: u64 = 0x4D4;
const OFF_ENABLE: u64 = 0x500;
const OFF_PSEL_SCL: u64 = 0x508;
const OFF_PSEL_SDA: u64 = 0x50C;
const OFF_RXD_PTR: u64 = 0x534;
const OFF_RXD_MAXCNT: u64 = 0x538;
const OFF_RXD_AMOUNT: u64 = 0x53C;
const OFF_TXD_PTR: u64 = 0x544;
const OFF_TXD_MAXCNT: u64 = 0x548;
const OFF_TXD_AMOUNT: u64 = 0x54C;
const OFF_ADDRESS0: u64 = 0x588;
const OFF_ADDRESS1: u64 = 0x58C;
const OFF_CONFIG: u64 = 0x594;
const OFF_ORC: u64 = 0x5C0;

// ENABLE register: value 9 selects TWIS mode (PS §6.29).
const ENABLE_MASK: u32 = 0xF;

// INTEN valid bits per PS §6.29 — STOPPED(1) ERROR(9) RXSTARTED(19)
// TXSTARTED(20) WRITE(25) READ(26).
const INTEN_MASK: u32 = (1 << 1) | (1 << 9) | (1 << 19) | (1 << 20) | (1 << 25) | (1 << 26);

// ERRORSRC valid bits: OVERFLOW(0) DNACK(2) OVERREAD(3) — W1C.
const ERRORSRC_MASK: u32 = (1 << 0) | (1 << 2) | (1 << 3);

// RXD/TXD MAXCNT mask: 8-bit EasyDMA counter.
const MAXCNT_MASK: u32 = 0xFF;

// ADDRESS: 7-bit I2C slave address.
const ADDRESS_MASK: u32 = 0x7F;

// ORC: 8-bit over-read character.
const ORC_MASK: u32 = 0xFF;

// CONFIG: bit 0 = ADDRESS0 enable, bit 1 = ADDRESS1 enable.
const CONFIG_MASK: u32 = 0x3;

#[derive(Debug, Default)]
pub struct Nrf52Twis {
    events_stopped: u32,
    events_error: u32,
    events_rxstarted: u32,
    events_txstarted: u32,
    events_write: u32,
    events_read: u32,
    inten: u32,
    /// ERRORSRC: W1C bits — cleared by writing 1, set by HW only.
    errorsrc: u32,
    /// MATCH: RO — indicates which ADDRESS[i] matched the last transaction.
    match_reg: u32,
    enable: u32,
    psel_scl: u32,
    psel_sda: u32,
    rxd_ptr: u32,
    rxd_maxcnt: u32,
    rxd_amount: u32,
    txd_ptr: u32,
    txd_maxcnt: u32,
    txd_amount: u32,
    address: [u32; 2],
    config: u32,
    orc: u32,
}

impl Nrf52Twis {
    pub fn new() -> Self {
        Self {
            // PSEL regs reset to 0xFFFF_FFFF (CONNECT=1 = disconnected).
            psel_scl: 0xFFFF_FFFF,
            psel_sda: 0xFFFF_FFFF,
            ..Default::default()
        }
    }
}

impl Peripheral for Nrf52Twis {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // TASKS: read-as-zero.
            OFF_TASKS_STOP | OFF_TASKS_SUSPEND | OFF_TASKS_RESUME | OFF_TASKS_PREPARERX
            | OFF_TASKS_PREPARETX => 0,
            // EVENTS.
            OFF_EVENTS_STOPPED => self.events_stopped,
            OFF_EVENTS_ERROR => self.events_error,
            OFF_EVENTS_RXSTARTED => self.events_rxstarted,
            OFF_EVENTS_TXSTARTED => self.events_txstarted,
            OFF_EVENTS_WRITE => self.events_write,
            OFF_EVENTS_READ => self.events_read,
            // INTEN (all three aliases return the same mask).
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten & INTEN_MASK,
            // ERRORSRC: W1C — read returns latched bits.
            OFF_ERRORSRC => self.errorsrc & ERRORSRC_MASK,
            // MATCH: RO.
            OFF_MATCH => self.match_reg & 0x1,
            // Configuration.
            OFF_ENABLE => self.enable & ENABLE_MASK,
            OFF_PSEL_SCL => self.psel_scl,
            OFF_PSEL_SDA => self.psel_sda,
            OFF_RXD_PTR => self.rxd_ptr,
            OFF_RXD_MAXCNT => self.rxd_maxcnt & MAXCNT_MASK,
            OFF_RXD_AMOUNT => self.rxd_amount & MAXCNT_MASK,
            OFF_TXD_PTR => self.txd_ptr,
            OFF_TXD_MAXCNT => self.txd_maxcnt & MAXCNT_MASK,
            OFF_TXD_AMOUNT => self.txd_amount & MAXCNT_MASK,
            OFF_ADDRESS0 => self.address[0] & ADDRESS_MASK,
            OFF_ADDRESS1 => self.address[1] & ADDRESS_MASK,
            OFF_CONFIG => self.config & CONFIG_MASK,
            OFF_ORC => self.orc & ORC_MASK,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // TASKS: write-only strobes.
            OFF_TASKS_STOP | OFF_TASKS_SUSPEND | OFF_TASKS_RESUME | OFF_TASKS_PREPARERX
            | OFF_TASKS_PREPARETX => {}
            // EVENTS: hardware-generated; SW write-1 ignored, write-0 clears.
            OFF_EVENTS_STOPPED if value == 0 => self.events_stopped = 0,
            OFF_EVENTS_ERROR if value == 0 => self.events_error = 0,
            OFF_EVENTS_RXSTARTED if value == 0 => self.events_rxstarted = 0,
            OFF_EVENTS_TXSTARTED if value == 0 => self.events_txstarted = 0,
            OFF_EVENTS_WRITE if value == 0 => self.events_write = 0,
            OFF_EVENTS_READ if value == 0 => self.events_read = 0,
            // INTEN / INTENSET / INTENCLR.
            OFF_INTEN => self.inten = value & INTEN_MASK,
            OFF_INTENSET => self.inten |= value & INTEN_MASK,
            OFF_INTENCLR => self.inten &= !(value & INTEN_MASK),
            // ERRORSRC W1C: writing 1 to a bit CLEARS it.
            OFF_ERRORSRC => self.errorsrc &= !(value & ERRORSRC_MASK),
            // MATCH is RO.
            OFF_MATCH => {}
            // Configuration.
            OFF_ENABLE => self.enable = value & ENABLE_MASK,
            OFF_PSEL_SCL => self.psel_scl = value,
            OFF_PSEL_SDA => self.psel_sda = value,
            OFF_RXD_PTR => self.rxd_ptr = value,
            OFF_RXD_MAXCNT => self.rxd_maxcnt = value & MAXCNT_MASK,
            OFF_TXD_PTR => self.txd_ptr = value,
            OFF_TXD_MAXCNT => self.txd_maxcnt = value & MAXCNT_MASK,
            OFF_ADDRESS0 => self.address[0] = value & ADDRESS_MASK,
            OFF_ADDRESS1 => self.address[1] = value & ADDRESS_MASK,
            OFF_CONFIG => self.config = value & CONFIG_MASK,
            OFF_ORC => self.orc = value & ORC_MASK,
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_value_9_selects_twis() {
        let mut t = Nrf52Twis::new();
        t.write_u32(OFF_ENABLE, 9).unwrap();
        assert_eq!(t.read_u32(OFF_ENABLE).unwrap(), 9);
    }

    #[test]
    fn enable_mask() {
        let mut t = Nrf52Twis::new();
        t.write_u32(OFF_ENABLE, 0xFF).unwrap();
        assert_eq!(t.read_u32(OFF_ENABLE).unwrap(), 0xF);
    }

    #[test]
    fn events_write_1_ignored() {
        let mut t = Nrf52Twis::new();
        for &off in &[
            OFF_EVENTS_STOPPED,
            OFF_EVENTS_ERROR,
            OFF_EVENTS_RXSTARTED,
            OFF_EVENTS_TXSTARTED,
            OFF_EVENTS_WRITE,
            OFF_EVENTS_READ,
        ] {
            t.write_u32(off, 1).unwrap();
            assert_eq!(
                t.read_u32(off).unwrap(),
                0,
                "write-1 at offset 0x{:03X} must be ignored",
                off
            );
        }
    }

    #[test]
    fn errorsrc_w1c() {
        let mut t = Nrf52Twis::new();
        // Seed ERRORSRC (would be HW-set on silicon).
        t.errorsrc = 0x7; // OVERFLOW|DNACK|OVERREAD — all 3 valid bits
                          // Clear OVERFLOW (bit 0) and OVERREAD (bit 3).
        t.write_u32(OFF_ERRORSRC, 0x9).unwrap();
        assert_eq!(t.read_u32(OFF_ERRORSRC).unwrap(), 0x4); // only DNACK remains
    }

    #[test]
    fn match_read_only() {
        let mut t = Nrf52Twis::new();
        t.write_u32(OFF_MATCH, 0xFF).unwrap();
        assert_eq!(t.read_u32(OFF_MATCH).unwrap(), 0);
    }

    #[test]
    fn address_masks_to_7_bits() {
        let mut t = Nrf52Twis::new();
        t.write_u32(OFF_ADDRESS0, 0xFF).unwrap();
        assert_eq!(t.read_u32(OFF_ADDRESS0).unwrap(), 0x7F);
        t.write_u32(OFF_ADDRESS1, 0x68).unwrap();
        assert_eq!(t.read_u32(OFF_ADDRESS1).unwrap(), 0x68);
    }

    #[test]
    fn psel_round_trip() {
        let mut t = Nrf52Twis::new();
        t.write_u32(OFF_PSEL_SCL, 0x0000_000B).unwrap();
        assert_eq!(t.read_u32(OFF_PSEL_SCL).unwrap(), 0x0000_000B);
        t.write_u32(OFF_PSEL_SDA, 0x8000_000C).unwrap();
        assert_eq!(t.read_u32(OFF_PSEL_SDA).unwrap(), 0x8000_000C);
    }

    #[test]
    fn rxd_txd_ptr_round_trip() {
        let mut t = Nrf52Twis::new();
        t.write_u32(OFF_RXD_PTR, 0x2000_0100).unwrap();
        assert_eq!(t.read_u32(OFF_RXD_PTR).unwrap(), 0x2000_0100);
        t.write_u32(OFF_TXD_PTR, 0x2000_0200).unwrap();
        assert_eq!(t.read_u32(OFF_TXD_PTR).unwrap(), 0x2000_0200);
    }

    #[test]
    fn maxcnt_masks_to_8_bits() {
        let mut t = Nrf52Twis::new();
        t.write_u32(OFF_RXD_MAXCNT, 0x1FF).unwrap();
        assert_eq!(t.read_u32(OFF_RXD_MAXCNT).unwrap(), 0xFF);
        t.write_u32(OFF_TXD_MAXCNT, 0x1FF).unwrap();
        assert_eq!(t.read_u32(OFF_TXD_MAXCNT).unwrap(), 0xFF);
    }

    #[test]
    fn orc_masks_to_8_bits() {
        let mut t = Nrf52Twis::new();
        t.write_u32(OFF_ORC, 0x1AB).unwrap();
        assert_eq!(t.read_u32(OFF_ORC).unwrap(), 0xAB);
    }

    #[test]
    fn config_masks_to_2_bits() {
        let mut t = Nrf52Twis::new();
        t.write_u32(OFF_CONFIG, 0xFF).unwrap();
        assert_eq!(t.read_u32(OFF_CONFIG).unwrap(), 0x3);
    }

    #[test]
    fn intenset_intenclr() {
        let mut t = Nrf52Twis::new();
        let bits = (1 << 1) | (1 << 9) | (1 << 19) | (1 << 20) | (1 << 25) | (1 << 26);
        t.write_u32(OFF_INTENSET, 0xFFFF_FFFF).unwrap();
        assert_eq!(t.read_u32(OFF_INTENSET).unwrap(), bits);
        t.write_u32(OFF_INTENCLR, 1 << 9).unwrap();
        assert_eq!(t.read_u32(OFF_INTENSET).unwrap(), bits & !(1 << 9));
    }

    #[test]
    fn tasks_read_as_zero() {
        let mut t = Nrf52Twis::new();
        for &off in &[
            OFF_TASKS_STOP,
            OFF_TASKS_SUSPEND,
            OFF_TASKS_RESUME,
            OFF_TASKS_PREPARERX,
            OFF_TASKS_PREPARETX,
        ] {
            t.write_u32(off, 1).unwrap();
            assert_eq!(
                t.read_u32(off).unwrap(),
                0,
                "TASK at 0x{:03X} must read zero",
                off
            );
        }
    }

    #[test]
    fn inten_direct_write() {
        let mut t = Nrf52Twis::new();
        let bits = (1 << 1) | (1 << 9);
        t.write_u32(OFF_INTEN, bits).unwrap();
        assert_eq!(t.read_u32(OFF_INTEN).unwrap(), bits);
        // Writing 0 clears all.
        t.write_u32(OFF_INTEN, 0).unwrap();
        assert_eq!(t.read_u32(OFF_INTEN).unwrap(), 0);
    }
}
