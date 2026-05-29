// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 NVMC (Non-Volatile Memory Controller).
//!
//! Source: nRF52840 PS rev 1.7 §6.14 (NVMC). Controls writes to flash
//! and UICR. We model just enough that firmware polling READY before
//! flash ops doesn't deadlock: READY always reads 1 (idle/ready),
//! CONFIG and ERASE* are accepted but no actual flash erase is
//! performed.

use crate::{Peripheral, SimResult};

const OFF_READY: u64 = 0x400;
const OFF_READYNEXT: u64 = 0x408;
const OFF_CONFIG: u64 = 0x504;
const OFF_ERASEPAGE: u64 = 0x508;
const OFF_ERASEALL: u64 = 0x50C;
const OFF_ERASEPAGEPARTIAL: u64 = 0x510;
const OFF_ERASEPAGEPARTIALCFG: u64 = 0x514;
const OFF_ERASEUICR: u64 = 0x514;
const OFF_ICACHECNF: u64 = 0x540;
const OFF_IHIT: u64 = 0x548;
const OFF_IMISS: u64 = 0x54C;

#[derive(Debug, Default)]
pub struct Nrf52Nvmc {
    config: u32,
    erasepagepartialcfg: u32,
    icachecnf: u32,
    ihit: u32,
    imiss: u32,
}

impl Nrf52Nvmc {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Nvmc {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // Always ready — we don't simulate flash latency.
            OFF_READY => 1,
            OFF_READYNEXT => 1,
            OFF_CONFIG => self.config & 0x3,
            OFF_ERASEPAGE | OFF_ERASEALL | OFF_ERASEPAGEPARTIAL => 0,
            OFF_ERASEPAGEPARTIALCFG => self.erasepagepartialcfg & 0x3F,
            OFF_ICACHECNF => self.icachecnf & 0x101,
            OFF_IHIT => self.ihit,
            OFF_IMISS => self.imiss,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_CONFIG => self.config = value & 0x3,
            OFF_ERASEPAGE | OFF_ERASEALL | OFF_ERASEPAGEPARTIAL | OFF_ERASEUICR => {
                // Erase requested — we accept but don't actually clear
                // simulated flash. Firmware that subsequently reads back
                // erased data may see stale content; acceptable for
                // boot-path probing.
            }
            OFF_ERASEPAGEPARTIALCFG => self.erasepagepartialcfg = value & 0x3F,
            OFF_ICACHECNF => self.icachecnf = value & 0x101,
            OFF_IHIT => self.ihit = 0,
            OFF_IMISS => self.imiss = 0,
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_always_one() {
        let n = Nrf52Nvmc::new();
        assert_eq!(n.read_u32(OFF_READY).unwrap(), 1);
        assert_eq!(n.read_u32(OFF_READYNEXT).unwrap(), 1);
    }

    #[test]
    fn config_masks_to_2_bits() {
        let mut n = Nrf52Nvmc::new();
        n.write_u32(OFF_CONFIG, 0xFF).unwrap();
        assert_eq!(n.read_u32(OFF_CONFIG).unwrap(), 0x3);
    }
}
