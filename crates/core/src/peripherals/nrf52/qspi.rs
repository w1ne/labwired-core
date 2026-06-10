// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 QSPI peripheral — register-surface model.
//!
//! Source: nRF52840 PS rev 1.7 §6.4 (QSPI). Quad-SPI flash controller.
//! Models the register surface so firmware can configure pin selection,
//! READ/WRITE/ERASE source/destination/length, and address mode without
//! crashing.  No flash transactions are simulated.

use crate::{Peripheral, SimResult};

const OFF_TASKS_ACTIVATE: u64 = 0x000;
const OFF_TASKS_READSTART: u64 = 0x004;
const OFF_TASKS_WRITESTART: u64 = 0x008;
const OFF_TASKS_ERASESTART: u64 = 0x00C;
const OFF_TASKS_DEACTIVATE: u64 = 0x010;

const OFF_EVENTS_READY: u64 = 0x100;

const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;

const OFF_ENABLE: u64 = 0x500;
const OFF_READ_SRC: u64 = 0x504;
const OFF_READ_DST: u64 = 0x508;
const OFF_READ_CNT: u64 = 0x50C;
const OFF_WRITE_DST: u64 = 0x510;
const OFF_WRITE_SRC: u64 = 0x514;
const OFF_WRITE_CNT: u64 = 0x518;
const OFF_ERASE_PTR: u64 = 0x51C;
const OFF_ERASE_LEN: u64 = 0x520;

// PSEL.SCK/CSN/IO0/IO1/IO2/IO3 at 0x524..0x53C.
const OFF_PSEL_FIRST: u64 = 0x524;
const OFF_PSEL_LAST: u64 = 0x53C;

const OFF_XIPOFFSET: u64 = 0x540;
const OFF_IFCONFIG0: u64 = 0x544;
const OFF_IFCONFIG1: u64 = 0x600;
const OFF_STATUS: u64 = 0x604;
const OFF_DPMDUR: u64 = 0x614;
const OFF_ADDRCONF: u64 = 0x624;
const OFF_CINSTRCONF: u64 = 0x634;
const OFF_CINSTRDAT0: u64 = 0x638;
const OFF_CINSTRDAT1: u64 = 0x63C;
const OFF_IFTIMING: u64 = 0x648;

#[derive(Debug, Default)]
pub struct Nrf52Qspi {
    events_ready: u32,
    inten: u32,

    enable: u32,
    read_src: u32,
    read_dst: u32,
    read_cnt: u32,
    write_dst: u32,
    write_src: u32,
    write_cnt: u32,
    erase_ptr: u32,
    erase_len: u32,
    psel: [u32; 7], // PSEL.SCK/CSN/IO0/IO1/IO2/IO3 at 0x524..0x538, plus 0x53C pad
    xipoffset: u32,
    ifconfig0: u32,
    ifconfig1: u32,
    status: u32,
    dpmdur: u32,
    addrconf: u32,
    cinstrconf: u32,
    cinstrdat0: u32,
    cinstrdat1: u32,
    iftiming: u32,
}

impl Nrf52Qspi {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Qspi {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_ACTIVATE | OFF_TASKS_READSTART | OFF_TASKS_WRITESTART
            | OFF_TASKS_ERASESTART | OFF_TASKS_DEACTIVATE => 0,
            OFF_EVENTS_READY => self.events_ready,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_ENABLE => self.enable & 1,
            OFF_READ_SRC => self.read_src,
            OFF_READ_DST => self.read_dst,
            OFF_READ_CNT => self.read_cnt,
            OFF_WRITE_DST => self.write_dst,
            OFF_WRITE_SRC => self.write_src,
            OFF_WRITE_CNT => self.write_cnt,
            OFF_ERASE_PTR => self.erase_ptr,
            OFF_ERASE_LEN => self.erase_len & 0x3,
            OFF_PSEL_FIRST..=OFF_PSEL_LAST if offset.is_multiple_of(4) => {
                self.psel[((offset - OFF_PSEL_FIRST) / 4) as usize]
            }
            OFF_XIPOFFSET => self.xipoffset,
            OFF_IFCONFIG0 => self.ifconfig0,
            OFF_IFCONFIG1 => self.ifconfig1,
            OFF_STATUS => self.status,
            OFF_DPMDUR => self.dpmdur,
            OFF_ADDRCONF => self.addrconf,
            OFF_CINSTRCONF => self.cinstrconf,
            OFF_CINSTRDAT0 => self.cinstrdat0,
            OFF_CINSTRDAT1 => self.cinstrdat1,
            OFF_IFTIMING => self.iftiming,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_ACTIVATE | OFF_TASKS_READSTART | OFF_TASKS_WRITESTART
            | OFF_TASKS_ERASESTART | OFF_TASKS_DEACTIVATE => {}
            // EVENTS_READY: hardware-generated. SW write-1 ignored; write-0 clears.
            OFF_EVENTS_READY if value == 0 => self.events_ready = 0,
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            OFF_ENABLE => self.enable = value & 1,
            OFF_READ_SRC => self.read_src = value,
            OFF_READ_DST => self.read_dst = value,
            OFF_READ_CNT => self.read_cnt = value,
            OFF_WRITE_DST => self.write_dst = value,
            OFF_WRITE_SRC => self.write_src = value,
            OFF_WRITE_CNT => self.write_cnt = value,
            OFF_ERASE_PTR => self.erase_ptr = value,
            OFF_ERASE_LEN => self.erase_len = value & 0x3,
            OFF_PSEL_FIRST..=OFF_PSEL_LAST if offset.is_multiple_of(4) => {
                self.psel[((offset - OFF_PSEL_FIRST) / 4) as usize] = value;
            }
            OFF_XIPOFFSET => self.xipoffset = value,
            OFF_IFCONFIG0 => self.ifconfig0 = value,
            OFF_IFCONFIG1 => self.ifconfig1 = value,
            OFF_STATUS => {} // RO
            OFF_DPMDUR => self.dpmdur = value,
            OFF_ADDRCONF => self.addrconf = value,
            OFF_CINSTRCONF => self.cinstrconf = value,
            OFF_CINSTRDAT0 => self.cinstrdat0 = value,
            OFF_CINSTRDAT1 => self.cinstrdat1 = value,
            OFF_IFTIMING => self.iftiming = value,
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ifconfig0_round_trips() {
        let mut q = Nrf52Qspi::new();
        q.write_u32(OFF_IFCONFIG0, 0x0000_0035).unwrap();
        assert_eq!(q.read_u32(OFF_IFCONFIG0).unwrap(), 0x0000_0035);
    }

    #[test]
    fn psel_sck_round_trips() {
        let mut q = Nrf52Qspi::new();
        q.write_u32(0x524, 21).unwrap(); // PSEL.SCK
        assert_eq!(q.read_u32(0x524).unwrap(), 21);
    }
}
