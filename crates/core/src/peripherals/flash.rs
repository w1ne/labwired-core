// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! FLASH interface peripheral — layout-selectable per chip family.
//!
//! ARCHITECTURAL SEPARATION: each family's register layout lives behind a
//! `FlashRegisterLayout` variant. F1 and L4 reset values + register offsets
//! differ; both are reachable through the same `Flash` struct but every
//! F1-specific code path is gated by `matches!(self.layout, Stm32F1)` so
//! adding/changing F1 cannot regress L4 behaviour and vice-versa.
//!
//! HAL-generated firmware writes to FLASH_ACR before any clock change to
//! adjust wait states (LATENCY field) for the new SYSCLK frequency.
//! Without this peripheral those writes silently drop and reads return
//! 0, which makes HAL_RCC_ClockConfig() loop forever waiting for
//! the latency change to take effect.
//!
//! Reset values verified against real NUCLEO-L476RG silicon via SWD:
//!   L4 ACR  = 0x0000_0600  (caches enabled by the boot ROM)
//!   L4 SR   = 0x0000_0000
//!   L4 CR   = 0xC000_0000  (LOCK + OPTLOCK both held)
//!   L4 OPTR = 0xFFEF_F8AA  (factory option-byte programming)
//!
//! Reset values verified against real STM32F103C8 silicon via SWD
//! (Blue Pill, ST-LINK V2J43, 2026-06-04):
//!   F1 ACR  = 0x0000_0030  (PRFTBE=1, PRFTBS=1; LATENCY=0)
//!   F1 SR   = 0x0000_0000
//!   F1 CR   = 0x0000_0080  (LOCK=1 — bit 7 on F1, not 31 like L4)
//!   F1 OBR  = 0x03FF_FFFC  (factory option bytes)

use crate::SimResult;
use std::str::FromStr;

/// Register layout / reset-value profile for the FLASH interface.
/// Adding a variant must NOT touch the read/write branches of existing
/// variants — keep family-specific behaviour isolated.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlashRegisterLayout {
    /// Default. Verified on NUCLEO-L476RG (silicon, 2026-05).
    #[default]
    Stm32L4,
    /// STM32F1 medium-density (RM0008). Verified on STM32F103C8 Blue Pill
    /// (silicon, 2026-06-04). ACR has PRFTBE→PRFTBS read-back mirror;
    /// register file is shorter (KEYR@0x04, SR@0x0C, CR@0x10, OBR@0x1C,
    /// WRPR@0x20). No ECCR / OPTR / PCROP / WRP1..2.
    Stm32F1,
}

impl FromStr for FlashRegisterLayout {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stm32l4" | "l4" => Ok(Self::Stm32L4),
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            _ => Err(format!(
                "unsupported FLASH register layout '{}'; supported: stm32l4, stm32f1",
                value
            )),
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Flash {
    layout: FlashRegisterLayout,
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
    /// Backwards-compat constructor: defaults to STM32L4 layout so existing
    /// callers (and the bus dispatch fallback) are unchanged.
    pub fn new() -> Self {
        Self::new_with_layout(FlashRegisterLayout::Stm32L4)
    }

    pub fn new_with_layout(layout: FlashRegisterLayout) -> Self {
        // Per-layout reset values — verified against real silicon per family.
        // Touching the F1 branch must not change L4 reset values.
        let (acr_reset, cr_reset, optr_reset) = match layout {
            FlashRegisterLayout::Stm32L4 => (0x0000_0600u32, 0xC000_0000u32, 0xFFEF_F8AAu32),
            FlashRegisterLayout::Stm32F1 => (0x0000_0030u32, 0x0000_0080u32, 0x0000_0000u32),
        };
        Self {
            layout,
            acr: acr_reset,
            pdkeyr: 0,
            keyr: 0,
            optkeyr: 0,
            sr: 0,
            cr: cr_reset,
            eccr: 0,
            optr: optr_reset,
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

    // (legacy `new()` body replaced; kept as the no-op below for the
    // never-reached default path — Rust requires all fields, so this
    // disambiguates from new_with_layout.)
    #[allow(dead_code)]
    fn _new_l4_legacy() -> Self {
        Self {
            layout: FlashRegisterLayout::Stm32L4,
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
        // ─── F1 layout (isolated; no fall-through to L4) ────────────────
        if matches!(self.layout, FlashRegisterLayout::Stm32F1) {
            return match offset {
                // ACR: PRFTBE (bit 4) reads back as PRFTBS (bit 5). HAL
                // sets PRFTBE then polls PRFTBS — without the mirror the
                // poll never satisfies and Arduino startup loops forever.
                0x00 => {
                    let prftbe = (self.acr >> 4) & 1;
                    (self.acr & !(1 << 5)) | (prftbe << 5)
                }
                0x04 => 0,         // KEYR write-only
                0x08 => 0,         // OPTKEYR write-only
                0x0C => self.sr,   // SR; BSY (bit 0) is RO, we never set busy.
                0x10 => self.cr,   // CR; LOCK at bit 7 (NOT bit 31 like L4).
                0x14 => 0,         // AR write-only
                0x1C => 0x03FF_FFFC, // OBR — verified on STM32F103C8 silicon.
                0x20 => 0xFFFF_FFFF, // WRPR — no write protect.
                _ => 0,
            };
        }
        // ─── L4 layout (untouched) ──────────────────────────────────────
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
        // ─── F1 layout (isolated; no fall-through to L4) ────────────────
        if matches!(self.layout, FlashRegisterLayout::Stm32F1) {
            match offset {
                // ACR writable: LATENCY[2:0], HLFCYA(3), PRFTBE(4). PRFTBS RO.
                0x00 => self.acr = value & 0x0000_001F,
                // KEYR — unlocks CR.LOCK (bit 7 on F1).
                0x04 => {
                    self.key_state = match (self.key_state, value) {
                        (KeyUnlockState::Locked, FLASH_KEY1) => KeyUnlockState::HalfUnlocked,
                        (KeyUnlockState::HalfUnlocked, FLASH_KEY2) => {
                            self.cr &= !(1 << 7);
                            KeyUnlockState::Unlocked
                        }
                        _ => KeyUnlockState::Locked,
                    };
                }
                // OPTKEYR — unlocks OPTWRE (bit 9 of F1 CR).
                0x08 => {
                    self.optkey_state = match (self.optkey_state, value) {
                        (KeyUnlockState::Locked, OPTKEY1) => KeyUnlockState::HalfUnlocked,
                        (KeyUnlockState::HalfUnlocked, OPTKEY2) => {
                            self.cr &= !(1 << 9);
                            KeyUnlockState::Unlocked
                        }
                        _ => KeyUnlockState::Locked,
                    };
                }
                // SR: EOP / PGERR / WRPRTERR are rc_w1. BSY (bit 0) RO.
                0x0C => self.sr &= !(value & 0x0000_0034),
                0x10 => self.cr = value,
                0x14 => {} // AR — accept; sim doesn't program flash pages
                _ => {}
            }
            return;
        }
        // ─── L4 layout (untouched) ──────────────────────────────────────
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

impl Default for Flash {
    fn default() -> Self {
        Self::new()
    }
}

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
