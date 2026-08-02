// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, SimResult};
use std::any::Any;

/// STM32F1 Alternate Function I/O (AFIO)
#[derive(Debug, Default, serde::Serialize)]
pub struct Afio {
    pub evcr: u32,
    pub mapr: u32,
    pub exticr: [u32; 4], // EXTICR1, EXTICR2, EXTICR3, EXTICR4
    pub mapr2: u32,
}

impl Afio {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the GPIO port index (0=A, 1=B, etc.) for a given EXTI line (0-15)
    pub fn get_exti_mapping(&self, line: u8) -> u8 {
        if line >= 16 {
            return 0;
        }
        let reg_idx = (line / 4) as usize;
        let shift = (line % 4) * 4;
        ((self.exticr[reg_idx] >> shift) & 0xF) as u8
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.evcr,
            0x04 => self.mapr,
            0x08 => self.exticr[0],
            0x0C => self.exticr[1],
            0x10 => self.exticr[2],
            0x14 => self.exticr[3],
            0x1C => self.mapr2,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        // Writable masks silicon-confirmed on the bench F103 via the address
        // sweep (stm32f1_mmio_diff). AFIO is an F1-only peripheral (F4+ use
        // SYSCFG) and F103 is the only F1 chip, so these are exact, not gated.
        match offset {
            // EVCR: PIN[3:0]/PORT[6:4]/EVOE(7) = 0xFF.
            0x00 => self.evcr = value & 0x0000_00FF,
            // MAPR: the remap field readable on F103-medium is the low 16 bits;
            // the ADC-ETRG remap bits [20:16], the reserved [23:21]/[31:27], and
            // the write-only SWJ_CFG [26:24] all read back 0 here.
            0x04 => self.mapr = value & 0x0000_FFFF,
            // EXTICR: four 4-bit line-source fields; bit 15 (the 4th port-select
            // bit of the top field) and [31:16] read 0 → 0x7FFF.
            0x08 => self.exticr[0] = value & 0x7FFF,
            0x0C => self.exticr[1] = value & 0x7FFF,
            0x10 => self.exticr[2] = value & 0x7FFF,
            0x14 => self.exticr[3] = value & 0x7FFF,
            // MAPR2 is not implemented on F103 medium-density — reads 0.
            0x1C => self.mapr2 = 0,
            _ => {}
        }
    }
}

impl Peripheral for Afio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        let mut reg_val = self.read_reg(reg_offset);
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn needs_legacy_walk(&self) -> bool {
        // AFIO is a pure configuration register bank (MAPR/EXTICR remap bits).
        // It has no `tick()` override, so its per-cycle walk callback is the
        // default no-op: it never mutates observable state, emits an IRQ/DMA
        // request, or fires an event from the walk. Deleting the walk is
        // therefore byte-identical for every reachable firmware state, so it
        // must not pin the walk on for the rest of the bus.
        false
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::Afio;
    use crate::Peripheral;

    fn rd32(a: &Afio, off: u64) -> u32 {
        (0..4).fold(0u32, |acc, i| {
            acc | ((a.read(off + i).unwrap() as u32) << (i * 8))
        })
    }

    #[test]
    fn mapr_reserved_bits_read_zero() {
        // Implemented remap bits [20:0] stick; reserved bits [23:21]/[31:27] and
        // the write-only SWJ_CFG [26:24] read back 0. Silicon-verified on the
        // bench STM32F103 (stm32f1_exec_oracle::afio_mapr_reserved_bits...).
        let mut a = Afio::new();
        a.write_u32(0x04, 0x0820_0004).unwrap(); // reserved 27,21 + USART1_REMAP
        assert_eq!(rd32(&a, 0x04), 0x0000_0004);

        let mut b = Afio::new();
        b.write_u32(0x04, 0xFFFF_FFFF).unwrap();
        // The address sweep on the bench F103 returned 0xFFFF for the remap
        // field (bits [20:16] read 0 on this part, with SWJ_CFG held 0).
        assert_eq!(rd32(&b, 0x04), 0x0000_FFFF);
    }

    #[test]
    fn exticr_upper_half_reads_zero() {
        // EXTICR1..4 implement four 4-bit fields in [15:0]; [31:16] reserved → 0.
        // Silicon-verified on F103 (stm32f1_exec_oracle::afio_exticr1_upper...).
        let mut a = Afio::new();
        for off in [0x08u64, 0x0C, 0x10, 0x14] {
            a.write_u32(off, 0xFFFF_FFFF).unwrap();
            // Bench F103 sweep: bit 15 (top of the 4th field) also reads 0 → 0x7FFF.
            assert_eq!(rd32(&a, off), 0x0000_7FFF, "EXTICR @ 0x{off:02X}");
        }
    }
}
