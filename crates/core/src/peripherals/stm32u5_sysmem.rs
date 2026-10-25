// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32U5 system-memory window (RM0456 §76, `stm32u575xx.h`):
//! `FLASH_OTP_BASE` 0x0BFA_0000, package data 0x0BFA_0500, unique device ID
//! 0x0BFA_0700 and the factory-programmed flash size at 0x0BFA_07A0.
//!
//! The model is deliberately small: OTP and identity words are a plain
//! read/write backing store (the same contract as Renode's `ArrayMemory` in
//! the 60North U5 platform, which the bare-metal core firmware writes and
//! reads back), and only the flash-size word is seeded. The flash-size value
//! is part of the part's identity, not firmware-writable state, so writes to
//! it are ignored.
//!
//! This is a *system-memory* model, not a peripheral register file: firmware
//! reads `FLASHSIZE_BASE` as a `const` array on silicon and gets the package
//! sizing from factory test. Without it the read bus-faults (core-smoke does
//! exactly this at `main.c:97`).

use crate::SimResult;

const WINDOW_SIZE: u64 = 0x800;
const FLASH_SIZE_OFFSET: u64 = 0x7A0;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Stm32u5Sysmem {
    data: Vec<u8>,
    /// Copy of the flash-size word, kept out of `data` so writes cannot move it.
    flash_size_kb: u16,
}

impl Stm32u5Sysmem {
    /// `flash_size_kb` is the factory `FLASHSIZE` value (2048 for the 2 MiB
    /// STM32U575xI, 512 for the STM32U545).
    pub fn new(flash_size_kb: u16) -> Self {
        let mut data = vec![0u8; WINDOW_SIZE as usize];
        data[FLASH_SIZE_OFFSET as usize..FLASH_SIZE_OFFSET as usize + 2]
            .copy_from_slice(&flash_size_kb.to_le_bytes());
        Self {
            data,
            flash_size_kb,
        }
    }

    fn is_flash_size_word(&self, offset: u64) -> bool {
        (FLASH_SIZE_OFFSET..FLASH_SIZE_OFFSET + 2).contains(&offset)
    }
}

impl crate::Peripheral for Stm32u5Sysmem {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self
            .data
            .get(offset as usize)
            .copied()
            .unwrap_or(if self.is_flash_size_word(offset) {
                (self.flash_size_kb >> ((offset - FLASH_SIZE_OFFSET) * 8)) as u8
            } else {
                0
            }))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if !self.is_flash_size_word(offset) {
            if let Some(slot) = self.data.get_mut(offset as usize) {
                *slot = value;
            }
        }
        Ok(())
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn flash_size_word_reports_the_configured_part() {
        for kb in [2048u16, 512, 1024] {
            let dev = Stm32u5Sysmem::new(kb);
            assert_eq!(dev.read(0x7A0).unwrap(), (kb & 0xFF) as u8);
            assert_eq!(dev.read(0x7A1).unwrap(), (kb >> 8) as u8);
            assert_eq!(dev.read(0x7A2).unwrap(), 0);
            assert_eq!(dev.read(0x7A3).unwrap(), 0);
        }
    }

    #[test]
    fn otp_words_round_trip_and_flash_size_stays_read_only() {
        let mut dev = Stm32u5Sysmem::new(2048);
        dev.write(0x10, 0x5A).unwrap();
        assert_eq!(dev.read(0x10).unwrap(), 0x5A);
        dev.write(0x7A0, 0x00).unwrap();
        assert_eq!(dev.read(0x7A0).unwrap(), 2048u16 as u8);
    }

    #[test]
    fn writes_outside_the_window_are_dropped() {
        let mut dev = Stm32u5Sysmem::new(2048);
        assert_eq!(dev.read(WINDOW_SIZE).unwrap(), 0);
        dev.write(WINDOW_SIZE, 0xFF).unwrap();
        assert_eq!(dev.read(WINDOW_SIZE).unwrap(), 0);
    }
}
