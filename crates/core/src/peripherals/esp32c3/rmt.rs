// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Minimal ESP32-C3 RMT (WS2812 / `rgbLedWrite` path).
//!
//! Arduino-ESP32 maps `LED_BUILTIN` / pin 30 to `rgbLedWrite` → RMT TX. The
//! driver fires `TX_START` then waits for `CHn_TX_END` (INT_RAW or IRQ). We
//! model TX as instantaneous: CONF0 write with TX_START sets TX_END and
//! self-clears the WT start bit. Register layout is C3-specific (CONF0 @
//! 0x10/0x14, INT_RAW @ 0x38) — not the S3 map.

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// ETS_RMT_INTR_SOURCE on ESP32-C3 (`soc/interrupts.h` / rmt.yaml).
pub const RMT_SOURCE_C3: u32 = 28;

const TX_START_BIT: u32 = 1 << 0;
/// Write-trigger bits in TX CONF0 that self-clear (start/rst/conf_update).
const TX_CONF0_WT_MASK: u32 = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 23) | (1 << 24);

#[derive(Debug)]
pub struct Esp32c3Rmt {
    source_id: u32,
    /// CH0/CH1 TX CONF0 @ 0x10 / 0x14.
    tx_conf0: [u32; 2],
    /// CH2/CH3 RX CONF0/1 @ 0x18..0x24 (storage only).
    other: [u32; 8],
    /// Status words @ 0x28..0x34.
    status: [u32; 4],
    int_raw: u32,
    int_ena: u32,
    /// SYS_CONF and misc tail registers (sparse via generic array).
    regs: [u32; 0x40],
    /// RMTMEM shadow (optional; firmware may write symbols here).
    mem: Vec<u32>,
}

impl Esp32c3Rmt {
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            tx_conf0: [0; 2],
            other: [0; 8],
            status: [0; 4],
            int_raw: 0,
            int_ena: 0,
            regs: [0; 0x40],
            mem: vec![0; 256],
        }
    }

    pub fn new_default() -> Self {
        Self::new(RMT_SOURCE_C3)
    }

    fn int_st(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    fn write_conf0(&mut self, ch: usize, value: u32) {
        if ch >= 2 {
            return;
        }
        // Preserve non-WT bits; WT bits self-clear after the write.
        let prev = self.tx_conf0[ch];
        let non_wt = (prev & !TX_CONF0_WT_MASK) | (value & !TX_CONF0_WT_MASK);
        self.tx_conf0[ch] = non_wt;
        if value & TX_START_BIT != 0 {
            // Instant TX complete: CHn_TX_END = bit n.
            self.int_raw |= 1 << ch;
        }
    }
}

impl Peripheral for Esp32c3Rmt {
    fn needs_legacy_walk(&self) -> bool {
        true
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let off = offset as usize;
        Ok(match off {
            0x10 => self.tx_conf0[0],
            0x14 => self.tx_conf0[1],
            0x18..=0x24 if (off - 0x18) % 4 == 0 => self.other[(off - 0x18) / 4],
            0x28..=0x34 if (off - 0x28) % 4 == 0 => self.status[(off - 0x28) / 4],
            0x38 => self.int_raw,
            0x3C => self.int_st(),
            0x40 => self.int_ena,
            0x44 => 0, // INT_CLR is WT
            0x400..=0x7FC if (off - 0x400) % 4 == 0 => {
                let i = (off - 0x400) / 4;
                self.mem.get(i).copied().unwrap_or(0)
            }
            o if o < 0x100 && o % 4 == 0 => self.regs.get(o / 4).copied().unwrap_or(0),
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let off = offset as usize;
        match off {
            0x10 => self.write_conf0(0, value),
            0x14 => self.write_conf0(1, value),
            0x18..=0x24 if (off - 0x18) % 4 == 0 => {
                self.other[(off - 0x18) / 4] = value;
            }
            0x28..=0x34 if (off - 0x28) % 4 == 0 => {
                // status mostly RO; ignore
            }
            0x38 => {
                // Some firmwares write INT_RAW; treat as clear of written 1s
                // if that matches R/WTC — actually R/WTC/SS is set-by-hw.
                // Ignore direct sets; only hardware sets TX_END.
            }
            0x40 => self.int_ena = value,
            0x44 => self.int_raw &= !value,
            0x400..=0x7FC if (off - 0x400) % 4 == 0 => {
                let i = (off - 0x400) / 4;
                if i < self.mem.len() {
                    self.mem[i] = value;
                }
            }
            o if o < 0x100 && o % 4 == 0 => {
                if let Some(r) = self.regs.get_mut(o / 4) {
                    *r = value;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok(((w >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let mut w = self.read_u32(aligned)?;
        let shift = (offset & 3) * 8;
        w = (w & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, w)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.int_st() != 0 {
            PeripheralTickResult {
                explicit_irqs: Some(vec![self.source_id]),
                ..PeripheralTickResult::default()
            }
        } else {
            PeripheralTickResult::default()
        }
    }

    fn matrix_irq_sources(&self) -> Vec<u32> {
        if self.int_st() != 0 {
            vec![self.source_id]
        } else {
            Vec::new()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
