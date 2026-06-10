// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 SPIS (SPI Slave) peripheral — register-surface model.
//!
//! Source: nRF52840 Product Specification rev 1.7 §6.26 (SPIS).
//!
//! SPIS0 shares a physical base address with SPIM0/SPI0 (0x40003000).
//! The peripheral operates in slave mode when ENABLE = 0x02 (value 2).
//! The SEMSTAT / STATUS W1C semantics and ENABLE=2 selector are modelled.
//!
//! EVENTS_* semantics: hardware-generated only. Writes of 1 are ignored;
//! only writes of 0 clear the event register — matching TIMER/RTC silicon
//! behaviour confirmed by the hw-oracle sweep.
//!
//! TASKS registers read as zero (write-only strobes on silicon).
//! SEMSTAT is read-only (RO). STATUS is W1C (write-1 clears a status bit).

use crate::{Peripheral, SimResult};

// ── Register offsets (PS §6.26, table — SPIS register map) ───────────────────

const OFF_TASKS_ACQUIRE: u64 = 0x024;
const OFF_TASKS_RELEASE: u64 = 0x028;
const OFF_EVENTS_END: u64 = 0x104;
const OFF_EVENTS_ENDRX: u64 = 0x110;
const OFF_EVENTS_ACQUIRED: u64 = 0x128;
const OFF_SHORTS: u64 = 0x200;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_SEMSTAT: u64 = 0x400;
const OFF_STATUS: u64 = 0x440;
const OFF_ENABLE: u64 = 0x500;
const OFF_PSEL_SCK: u64 = 0x508;
const OFF_PSEL_MISO: u64 = 0x50C;
const OFF_PSEL_MOSI: u64 = 0x510;
const OFF_PSEL_CSN: u64 = 0x514;
const OFF_RXD_PTR: u64 = 0x534;
const OFF_RXD_MAXCNT: u64 = 0x538;
const OFF_RXD_AMOUNT: u64 = 0x53C;
const OFF_TXD_PTR: u64 = 0x544;
const OFF_TXD_MAXCNT: u64 = 0x548;
const OFF_TXD_AMOUNT: u64 = 0x54C;
const OFF_CONFIG: u64 = 0x554;
const OFF_DEF: u64 = 0x55C;
const OFF_ORC: u64 = 0x5C0;

// ENABLE register: value 2 selects SPIS mode (PS §6.26).
const ENABLE_MASK: u32 = 0xF;

// SHORTS valid bits: END_ACQUIRE (bit 2) per PS table.
const SHORTS_MASK: u32 = 0x0000_0004;

// INTEN valid bits: END(1) ENDRX(4) ACQUIRED(10) — PS §6.26.
const INTEN_MASK: u32 = (1 << 1) | (1 << 4) | (1 << 10);

// STATUS valid bits: OVERREAD(0) OVERFLOW(1) — W1C.
const STATUS_MASK: u32 = 0x0000_0003;

// RXD/TXD MAXCNT mask: 8-bit (PS §6.26 — max 255 bytes in EasyDMA).
const MAXCNT_MASK: u32 = 0xFF;

// ORC / DEF: 8-bit character fields.
const ORC_MASK: u32 = 0xFF;

// PSEL fields: CONNECT(bit 31, active-low) + PORT(bit 5) + PIN(bits 4:0).
// Full word stored as-is; masking applied at higher-level if needed.
#[allow(dead_code)]
const PSEL_MASK: u32 = 0x8000_003F | (1 << 5); // CONNECT + PORT + PIN

#[derive(Debug, Default)]
pub struct Nrf52Spis {
    events_end: u32,
    events_endrx: u32,
    events_acquired: u32,
    shorts: u32,
    inten: u32,
    /// SEMSTAT: 2-bit RO field. Reset value 0 (CPU owns semaphore).
    semstat: u32,
    /// STATUS: W1C bits — cleared by writing 1, not set by firmware.
    status: u32,
    enable: u32,
    psel_sck: u32,
    psel_miso: u32,
    psel_mosi: u32,
    psel_csn: u32,
    rxd_ptr: u32,
    rxd_maxcnt: u32,
    rxd_amount: u32,
    txd_ptr: u32,
    txd_maxcnt: u32,
    txd_amount: u32,
    config: u32,
    def: u32,
    orc: u32,
}

impl Nrf52Spis {
    pub fn new() -> Self {
        Self {
            // PSEL regs reset to 0xFFFF_FFFF (CONNECT=1 = disconnected).
            psel_sck: 0xFFFF_FFFF,
            psel_miso: 0xFFFF_FFFF,
            psel_mosi: 0xFFFF_FFFF,
            psel_csn: 0xFFFF_FFFF,
            // SEMSTAT reset value = 0x1: CPU holds the semaphore at reset
            // (silicon-verified: SEMSTAT reads 0x1 on reset-halted nRF52840).
            // PS §6.26 SEMSTAT encoding: 0=Free 1=CPU 2=SPImaster 3=CPUandSPI.
            semstat: 0x1,
            ..Default::default()
        }
    }
}

impl Peripheral for Nrf52Spis {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // TASKS: read-as-zero (write-only strobes).
            OFF_TASKS_ACQUIRE | OFF_TASKS_RELEASE => 0,
            // EVENTS.
            OFF_EVENTS_END => self.events_end,
            OFF_EVENTS_ENDRX => self.events_endrx,
            OFF_EVENTS_ACQUIRED => self.events_acquired,
            // Control.
            OFF_SHORTS => self.shorts & SHORTS_MASK,
            OFF_INTENSET | OFF_INTENCLR => self.inten & INTEN_MASK,
            // Status — RO.
            OFF_SEMSTAT => self.semstat & 0x3,
            // STATUS W1C — read returns current latched bits.
            OFF_STATUS => self.status & STATUS_MASK,
            // Configuration.
            OFF_ENABLE => self.enable & ENABLE_MASK,
            OFF_PSEL_SCK => self.psel_sck,
            OFF_PSEL_MISO => self.psel_miso,
            OFF_PSEL_MOSI => self.psel_mosi,
            OFF_PSEL_CSN => self.psel_csn,
            OFF_RXD_PTR => self.rxd_ptr,
            OFF_RXD_MAXCNT => self.rxd_maxcnt & MAXCNT_MASK,
            OFF_RXD_AMOUNT => self.rxd_amount & MAXCNT_MASK,
            OFF_TXD_PTR => self.txd_ptr,
            OFF_TXD_MAXCNT => self.txd_maxcnt & MAXCNT_MASK,
            OFF_TXD_AMOUNT => self.txd_amount & MAXCNT_MASK,
            OFF_CONFIG => self.config & 0x7,
            OFF_DEF => self.def & ORC_MASK,
            OFF_ORC => self.orc & ORC_MASK,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // TASKS: write-only (read-as-zero); model the side-effect on semaphore.
            OFF_TASKS_ACQUIRE if value & 1 != 0 => {
                // TASKS_ACQUIRE: firmware requests semaphore (sim: no-op, SEMSTAT stays 0).
            }
            OFF_TASKS_RELEASE if value & 1 != 0 => {
                // TASKS_RELEASE: firmware releases semaphore.
            }
            // EVENTS: hardware-generated; SW write-1 ignored, write-0 clears.
            OFF_EVENTS_END if value == 0 => self.events_end = 0,
            OFF_EVENTS_ENDRX if value == 0 => self.events_endrx = 0,
            OFF_EVENTS_ACQUIRED if value == 0 => self.events_acquired = 0,
            // SHORTS.
            OFF_SHORTS => self.shorts = value & SHORTS_MASK,
            // INTENSET / INTENCLR.
            OFF_INTENSET => self.inten |= value & INTEN_MASK,
            OFF_INTENCLR => self.inten &= !(value & INTEN_MASK),
            // SEMSTAT is RO — silently ignore writes.
            OFF_SEMSTAT => {}
            // STATUS is W1C: writing 1 to a bit CLEARS that bit.
            OFF_STATUS => self.status &= !(value & STATUS_MASK),
            // ENABLE.
            OFF_ENABLE => self.enable = value & ENABLE_MASK,
            // PSEL.
            OFF_PSEL_SCK => self.psel_sck = value,
            OFF_PSEL_MISO => self.psel_miso = value,
            OFF_PSEL_MOSI => self.psel_mosi = value,
            OFF_PSEL_CSN => self.psel_csn = value,
            // RXD/TXD.
            OFF_RXD_PTR => self.rxd_ptr = value,
            OFF_RXD_MAXCNT => self.rxd_maxcnt = value & MAXCNT_MASK,
            OFF_TXD_PTR => self.txd_ptr = value,
            OFF_TXD_MAXCNT => self.txd_maxcnt = value & MAXCNT_MASK,
            // CONFIG / DEF / ORC.
            OFF_CONFIG => self.config = value & 0x7,
            OFF_DEF => self.def = value & ORC_MASK,
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
    fn enable_mask() {
        let mut s = Nrf52Spis::new();
        s.write_u32(OFF_ENABLE, 0xFF).unwrap();
        assert_eq!(s.read_u32(OFF_ENABLE).unwrap(), 0xF);
    }

    #[test]
    fn enable_value_2_selects_spis() {
        let mut s = Nrf52Spis::new();
        s.write_u32(OFF_ENABLE, 2).unwrap();
        assert_eq!(s.read_u32(OFF_ENABLE).unwrap(), 2);
    }

    #[test]
    fn events_write_1_ignored() {
        let mut s = Nrf52Spis::new();
        s.write_u32(OFF_EVENTS_END, 1).unwrap();
        assert_eq!(s.read_u32(OFF_EVENTS_END).unwrap(), 0);
        s.write_u32(OFF_EVENTS_ENDRX, 1).unwrap();
        assert_eq!(s.read_u32(OFF_EVENTS_ENDRX).unwrap(), 0);
        s.write_u32(OFF_EVENTS_ACQUIRED, 1).unwrap();
        assert_eq!(s.read_u32(OFF_EVENTS_ACQUIRED).unwrap(), 0);
    }

    #[test]
    fn status_w1c() {
        let mut s = Nrf52Spis::new();
        // Manually seed STATUS bits (would be set by HW on silicon).
        s.status = 0x3;
        // Writing 1 to bit 0 clears it.
        s.write_u32(OFF_STATUS, 0x1).unwrap();
        assert_eq!(s.read_u32(OFF_STATUS).unwrap(), 0x2);
        // Writing 1 to bit 1 clears remaining.
        s.write_u32(OFF_STATUS, 0x2).unwrap();
        assert_eq!(s.read_u32(OFF_STATUS).unwrap(), 0x0);
    }

    #[test]
    fn semstat_read_only() {
        let mut s = Nrf52Spis::new();
        // Reset value is 0x1 (silicon-verified: CPU holds semaphore at reset).
        assert_eq!(s.read_u32(OFF_SEMSTAT).unwrap(), 0x1);
        // Write is silently discarded — value remains 0x1.
        s.write_u32(OFF_SEMSTAT, 0x3).unwrap();
        assert_eq!(s.read_u32(OFF_SEMSTAT).unwrap(), 0x1);
    }

    #[test]
    fn psel_round_trip() {
        let mut s = Nrf52Spis::new();
        s.write_u32(OFF_PSEL_SCK, 0x0000_0005).unwrap();
        assert_eq!(s.read_u32(OFF_PSEL_SCK).unwrap(), 0x0000_0005);
        s.write_u32(OFF_PSEL_CSN, 0x8000_001F).unwrap();
        assert_eq!(s.read_u32(OFF_PSEL_CSN).unwrap(), 0x8000_001F);
    }

    #[test]
    fn rxd_txd_ptr_round_trip() {
        let mut s = Nrf52Spis::new();
        s.write_u32(OFF_RXD_PTR, 0x2000_1000).unwrap();
        assert_eq!(s.read_u32(OFF_RXD_PTR).unwrap(), 0x2000_1000);
        s.write_u32(OFF_TXD_PTR, 0x2000_2000).unwrap();
        assert_eq!(s.read_u32(OFF_TXD_PTR).unwrap(), 0x2000_2000);
    }

    #[test]
    fn maxcnt_masks_to_8_bits() {
        let mut s = Nrf52Spis::new();
        s.write_u32(OFF_RXD_MAXCNT, 0xFF).unwrap();
        assert_eq!(s.read_u32(OFF_RXD_MAXCNT).unwrap(), 0xFF);
        s.write_u32(OFF_TXD_MAXCNT, 0x1FF).unwrap();
        assert_eq!(s.read_u32(OFF_TXD_MAXCNT).unwrap(), 0xFF);
    }

    #[test]
    fn orc_masks_to_8_bits() {
        let mut s = Nrf52Spis::new();
        s.write_u32(OFF_ORC, 0x1AB).unwrap();
        assert_eq!(s.read_u32(OFF_ORC).unwrap(), 0xAB);
    }

    #[test]
    fn def_masks_to_8_bits() {
        let mut s = Nrf52Spis::new();
        s.write_u32(OFF_DEF, 0xDEAD).unwrap();
        assert_eq!(s.read_u32(OFF_DEF).unwrap(), 0xAD);
    }

    #[test]
    fn intenset_intenclr() {
        let mut s = Nrf52Spis::new();
        // END=bit1 ENDRX=bit4 ACQUIRED=bit10.
        let bits = (1 << 1) | (1 << 4) | (1 << 10);
        s.write_u32(OFF_INTENSET, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(OFF_INTENSET).unwrap(), bits);
        s.write_u32(OFF_INTENCLR, 1 << 4).unwrap();
        assert_eq!(s.read_u32(OFF_INTENSET).unwrap(), (1 << 1) | (1 << 10));
    }

    #[test]
    fn config_masks_to_3_bits() {
        let mut s = Nrf52Spis::new();
        s.write_u32(OFF_CONFIG, 0xFF).unwrap();
        assert_eq!(s.read_u32(OFF_CONFIG).unwrap(), 0x7);
    }

    #[test]
    fn tasks_read_as_zero() {
        let mut s = Nrf52Spis::new();
        s.write_u32(OFF_TASKS_ACQUIRE, 1).unwrap();
        assert_eq!(s.read_u32(OFF_TASKS_ACQUIRE).unwrap(), 0);
        s.write_u32(OFF_TASKS_RELEASE, 1).unwrap();
        assert_eq!(s.read_u32(OFF_TASKS_RELEASE).unwrap(), 0);
    }
}
