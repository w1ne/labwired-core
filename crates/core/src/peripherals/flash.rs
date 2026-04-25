// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! FLASH interface peripheral — STM32L4 layout.
//!
//! HAL-generated firmware writes to FLASH_ACR before any clock change to
//! adjust wait states (LATENCY field) for the new SYSCLK frequency.
//! Without this peripheral those writes silently drop and reads return
//! 0, which makes HAL_RCC_ClockConfig() loop forever waiting for
//! the latency change to take effect.
//!
//! Reset values verified against real NUCLEO-L476RG silicon via SWD:
//!   ACR  = 0x0000_0600  (caches enabled by the boot ROM)
//!   SR   = 0x0000_0000
//!   CR   = 0xC000_0000  (LOCK + OPTLOCK both held)
//!   OPTR = 0xFFEF_F8AA  (factory option-byte programming)

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Flash {
    acr: u32,
    pdkeyr: u32,
    keyr: u32,
    optkeyr: u32,
    sr: u32,
    cr: u32,
    eccr: u32,
    optr: u32,
    pcrop1sr: u32,
    pcrop1er: u32,
    wrp1ar: u32,
    wrp1br: u32,
    pcrop2sr: u32,
    pcrop2er: u32,
    wrp2ar: u32,
    wrp2br: u32,

    // Internal: tracks unlock sequence on KEYR (write 0x45670123 then 0xCDEF89AB).
    key_state: KeyUnlockState,
    optkey_state: KeyUnlockState,
}

#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
enum KeyUnlockState {
    #[default]
    Locked,
    HalfUnlocked,
    Unlocked,
}

const FLASH_KEY1: u32 = 0x4567_0123;
const FLASH_KEY2: u32 = 0xCDEF_89AB;
const OPTKEY1: u32 = 0x0819_2A3B;
const OPTKEY2: u32 = 0x4C5D_6E7F;

impl Flash {
    pub fn new() -> Self {
        Self {
            acr: 0x0000_0600,
            pdkeyr: 0,
            keyr: 0,
            optkeyr: 0,
            sr: 0,
            cr: 0xC000_0000,
            eccr: 0,
            optr: 0xFFEF_F8AA,
            pcrop1sr: 0,
            pcrop1er: 0,
            wrp1ar: 0,
            wrp1br: 0,
            pcrop2sr: 0,
            pcrop2er: 0,
            wrp2ar: 0,
            wrp2br: 0,
            key_state: KeyUnlockState::Locked,
            optkey_state: KeyUnlockState::Locked,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.acr,
            0x04 => self.pdkeyr,
            0x08 => self.keyr,
            0x0C => self.optkeyr,
            0x10 => self.sr,
            0x14 => self.cr,
            0x18 => self.eccr,
            0x20 => self.optr,
            0x24 => self.pcrop1sr,
            0x28 => self.pcrop1er,
            0x2C => self.wrp1ar,
            0x30 => self.wrp1br,
            0x44 => self.pcrop2sr,
            0x48 => self.pcrop2er,
            0x4C => self.wrp2ar,
            0x50 => self.wrp2br,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // ACR is writable; LATENCY (bits 2:0), PRFTEN (bit 8),
            // ICEN (9), DCEN (10), ICRST (11), DCRST (12), RUN_PD (14),
            // SLEEP_PD (15). Other bits reserved. We keep only the
            // documented writable mask so accidental writes don't pollute
            // the readback with nonsense.
            0x00 => self.acr = value & 0xC000_FF07,
            0x04 => self.pdkeyr = value,
            // KEYR / OPTKEYR are write-only; the write_volatile sequence
            // walks the lock state machine.
            0x08 => {
                self.key_state = match (self.key_state, value) {
                    (KeyUnlockState::Locked, FLASH_KEY1) => KeyUnlockState::HalfUnlocked,
                    (KeyUnlockState::HalfUnlocked, FLASH_KEY2) => {
                        // LOCK = bit 31 of CR clears.
                        self.cr &= !(1 << 31);
                        KeyUnlockState::Unlocked
                    }
                    _ => KeyUnlockState::Locked,
                };
            }
            0x0C => {
                self.optkey_state = match (self.optkey_state, value) {
                    (KeyUnlockState::Locked, OPTKEY1) => KeyUnlockState::HalfUnlocked,
                    (KeyUnlockState::HalfUnlocked, OPTKEY2) => {
                        self.cr &= !(1 << 30); // OPTLOCK
                        KeyUnlockState::Unlocked
                    }
                    _ => KeyUnlockState::Locked,
                };
            }
            // SR is rc_w1: writing 1 clears EOP / OPERR / PROGERR /
            // WRPERR / PGAERR / SIZERR / PGSERR / MISERR / FASTERR /
            // RDERR / OPTVERR. BSY (bit 16) is read-only.
            0x10 => {
                let clearable: u32 = 0x0000_FFFE;
                self.sr &= !(value & clearable);
            }
            0x14 => self.cr = value,
            0x18 => self.eccr &= !(value & 0xC000_0000), // W1C ECC error flags
            // OPTR is writable only after OPTKEYR unlock + OBL_LAUNCH.
            // For the simulator we accept the write directly.
            0x20 => {
                if matches!(self.optkey_state, KeyUnlockState::Unlocked) {
                    self.optr = value;
                }
            }
            _ => {}
        }
    }
}

impl Default for Flash { fn default() -> Self { Self::new() } }

impl crate::Peripheral for Flash {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // KEYR/OPTKEYR sequence works on full-word writes only. For
        // byte-level access (rare), update word and re-trigger.
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
