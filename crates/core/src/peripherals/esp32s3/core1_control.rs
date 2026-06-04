// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 `SYSTEM_CORE_1_CONTROL_0/1` registers (`0x600C_0000` / `+0x4`).
//!
//! The PRO_CPU releases the APP_CPU (core 1) from reset through these registers
//! — exactly as `cpu_utility_ll_enable_clock_and_reset_app_cpu()` does:
//!
//! ```text
//!   SET   CORE_1_CONTROL_0.CLKGATE_EN  (bit 1)
//!   CLR   CORE_1_CONTROL_0.RUNSTALL    (bit 0)
//!   SET   CORE_1_CONTROL_0.RESETING    (bit 2)
//!   CLR   CORE_1_CONTROL_0.RESETING    (bit 2)   <-- APP_CPU comes out of reset
//! ```
//!
//! The faithful model watches `CONTROL_0` for the `RESETING` 1→0 edge and
//! raises [`APPCPU_RESET_RELEASED`]; the rom-boot run loop then unhalts the
//! APP_CPU at the ROM reset vector so it boots the real ROM like silicon.
//! No firmware-symbol hooks — works for any image.

use crate::peripherals::esp32s3::rom_thunks::APPCPU_RESET_RELEASED;
use crate::{Peripheral, SimResult};

const CONTROL_0: u64 = 0x0;
const RESETING: u32 = 1 << 2; // SYSTEM_CONTROL_CORE_1_RESETING

#[derive(Debug)]
pub struct Esp32s3Core1Control {
    /// CONTROL_0 (idx 0) and CONTROL_1 (idx 1).
    regs: [u32; 2],
}

impl Default for Esp32s3Core1Control {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32s3Core1Control {
    pub fn new() -> Self {
        // Reset default per the TRM: CORE_1_RESETING = 1 (core 1 held in reset).
        Self {
            regs: [RESETING, 0],
        }
    }

    fn idx(offset: u64) -> Option<usize> {
        match offset & !3 {
            CONTROL_0 => Some(0),
            0x4 => Some(1),
            _ => None,
        }
    }
}

impl Peripheral for Esp32s3Core1Control {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = Self::idx(offset).map(|i| self.regs[i]).unwrap_or(0);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if let Some(i) = Self::idx(offset) {
            let byte_off = (offset & 3) * 8;
            let mut w = self.regs[i];
            w = (w & !(0xFFu32 << byte_off)) | ((value as u32) << byte_off);
            self.write_u32(offset & !3, w)?;
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(Self::idx(offset).map(|i| self.regs[i]).unwrap_or(0))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if let Some(i) = Self::idx(offset) {
            let prev = self.regs[i];
            self.regs[i] = value;
            if std::env::var("LABWIRED_CCDBG").is_ok() {
                eprintln!("core1_control: write CONTROL_{i} prev=0x{prev:08x} val=0x{value:08x}");
            }
            // CONTROL_0: RESETING 1→0 == APP_CPU released from reset.
            if i == 0 && (prev & RESETING) != 0 && (value & RESETING) == 0 {
                APPCPU_RESET_RELEASED.with(|s| s.set(true));
            }
        }
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reseting_falling_edge_releases_appcpu() {
        APPCPU_RESET_RELEASED.with(|s| s.set(false));
        let mut c = Esp32s3Core1Control::new();
        // Mirror the ll sequence: clkgate, set RESETING, then clear it.
        c.write_u32(CONTROL_0, RESETING | (1 << 1)).unwrap(); // CLKGATE + RESETING
        assert!(!APPCPU_RESET_RELEASED.with(|s| s.get()), "not yet");
        c.write_u32(CONTROL_0, 1 << 1).unwrap(); // clear RESETING
        assert!(APPCPU_RESET_RELEASED.with(|s| s.get()), "released on 1->0");
    }

    #[test]
    fn no_release_without_falling_edge() {
        APPCPU_RESET_RELEASED.with(|s| s.set(false));
        let mut c = Esp32s3Core1Control::new();
        // Writes that never clear an already-set RESETING don't release.
        c.write_u32(CONTROL_0, RESETING).unwrap();
        assert!(!APPCPU_RESET_RELEASED.with(|s| s.get()));
    }
}
