// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 NVMC (Non-Volatile Memory Controller).
//!
//! Source: nRF52840 PS rev 1.7 §6.14 (NVMC). Controls writes to flash
//! and UICR.
//!
//! Fidelity contract:
//!   * READY/READYNEXT always read 1 (no erase/program latency modelled).
//!   * CONFIG.Wen gates flash programming: with Wen clear, stores to the
//!     flash region are dropped (silicon ignores them); with Wen set, a
//!     store commits as `existing & new` (flash bits only flip 1→0). The
//!     gating itself lives on the bus write path (`bus/accessors.rs`),
//!     consulted through the cached `nrf52_nvmc_idx`.
//!   * Erase is REAL but applied at the instruction boundary, not inside the
//!     register write: ERASEPAGE/ERASEALL/ERASEUICR latch a pending op here,
//!     and `machine/boundary.rs` drains it — blanking the 4 KiB page (or the
//!     whole flash region, or resetting the UICR model) between
//!     instructions, so neither the CPU nor a peripheral observes a
//!     half-erased page. Erase ops require CONFIG.Een, as on silicon;
//!     without it they are ignored.

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

const CONFIG_WEN: u32 = 1;
const CONFIG_EEN: u32 = 2;

/// A latched erase request, drained by the machine at the next instruction
/// boundary (`machine/boundary.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nrf52NvmcOp {
    /// Blank the 4 KiB page containing this address.
    ErasePage(u64),
    /// Blank the entire flash region.
    EraseAll,
    /// Reset the UICR model to its erased state.
    EraseUicr,
}

#[derive(Debug, Default)]
pub struct Nrf52Nvmc {
    config: u32,
    erasepagepartialcfg: u32,
    icachecnf: u32,
    ihit: u32,
    imiss: u32,
    pending: Option<Nrf52NvmcOp>,
}

impl Nrf52Nvmc {
    pub fn new() -> Self {
        Self::default()
    }

    /// CONFIG.Wen: flash program writes are permitted (the bus write path
    /// asks this on every flash-region store).
    pub fn write_enabled(&self) -> bool {
        self.config & CONFIG_WEN != 0
    }

    /// Take the latched erase op, if any (machine boundary drains it).
    pub fn take_pending_op(&mut self) -> Option<Nrf52NvmcOp> {
        self.pending.take()
    }
}

impl Peripheral for Nrf52Nvmc {
    /// Walk-independent for every firmware state: this model overrides neither
    /// `tick()` nor `tick_elapsed()` with time-driven work that the walk must
    /// deliver. Observable effects land on MMIO writes and/or the separate
    /// `tick_with_bus` path (`bus_tick_indices`), which still runs when the
    /// legacy walk is deleted. Marking `needs_legacy_walk = false` therefore
    /// drops only empty dispatch from the per-cycle walk — byte-identical.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
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
            _ => {
                crate::census_reg!("nrf52.nvmc:Nrf52Nvmc", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_CONFIG => self.config = value & 0x3,
            // Erase requests latch only with CONFIG.Een set (silicon ignores
            // them otherwise). The boundary drain performs the blanking.
            OFF_ERASEPAGE if self.config & CONFIG_EEN != 0 => {
                self.pending = Some(Nrf52NvmcOp::ErasePage(value as u64));
            }
            OFF_ERASEALL if self.config & CONFIG_EEN != 0 && value & 1 != 0 => {
                self.pending = Some(Nrf52NvmcOp::EraseAll);
            }
            OFF_ERASEUICR if self.config & CONFIG_EEN != 0 && value & 1 != 0 => {
                self.pending = Some(Nrf52NvmcOp::EraseUicr);
            }
            OFF_ERASEPAGE | OFF_ERASEALL | OFF_ERASEUICR => {
                // Erase requested without Een: ignored, as on silicon.
            }
            OFF_ERASEPAGEPARTIAL => {}
            #[allow(unreachable_patterns)]
            OFF_ERASEPAGEPARTIALCFG => self.erasepagepartialcfg = value & 0x3F,
            OFF_ICACHECNF => self.icachecnf = value & 0x101,
            OFF_IHIT => self.ihit = 0,
            OFF_IMISS => self.imiss = 0,
            _ => {
                crate::census_reg!("nrf52.nvmc:Nrf52Nvmc", offset, "write");
            }
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

    #[test]
    fn write_enabled_tracks_wen_bit() {
        let mut n = Nrf52Nvmc::new();
        assert!(!n.write_enabled());
        n.write_u32(OFF_CONFIG, CONFIG_WEN).unwrap();
        assert!(n.write_enabled());
        n.write_u32(OFF_CONFIG, 0).unwrap();
        assert!(!n.write_enabled());
    }

    #[test]
    fn erase_latches_only_with_een() {
        let mut n = Nrf52Nvmc::new();
        n.write_u32(OFF_ERASEPAGE, 0x70000).unwrap();
        assert_eq!(n.take_pending_op(), None, "no Een ⇒ erase ignored");

        n.write_u32(OFF_CONFIG, CONFIG_EEN).unwrap();
        n.write_u32(OFF_ERASEPAGE, 0x70000).unwrap();
        assert_eq!(n.take_pending_op(), Some(Nrf52NvmcOp::ErasePage(0x70000)));
        assert_eq!(n.take_pending_op(), None, "op is consumed once");

        n.write_u32(OFF_ERASEALL, 1).unwrap();
        assert_eq!(n.take_pending_op(), Some(Nrf52NvmcOp::EraseAll));

        n.write_u32(OFF_ERASEUICR, 1).unwrap();
        assert_eq!(n.take_pending_op(), Some(Nrf52NvmcOp::EraseUicr));
    }
}
