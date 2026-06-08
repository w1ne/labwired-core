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
        match offset {
            0x00 => self.evcr = value,
            // MAPR implements remap bits [20:0]; bits [23:21] and [31:27] are
            // reserved and the write-only SWJ_CFG field [26:24] reads back
            // undefined. Silicon returns 0 for the reserved bits, so mask the
            // stored value to the readable remap field. Silicon-verified on the
            // bench STM32F103 (stm32f1_exec_oracle::afio_mapr_reserved_bits...).
            0x04 => self.mapr = value & 0x001F_FFFF,
            // Each EXTICR holds four 4-bit line-source fields in bits [15:0];
            // bits [31:16] are reserved and read 0 (RM0008 §9.4.3). Silicon-
            // verified on the bench F103 (afio_exticr1_upper_half_reads_zero).
            0x08 => self.exticr[0] = value & 0xFFFF,
            0x0C => self.exticr[1] = value & 0xFFFF,
            0x10 => self.exticr[2] = value & 0xFFFF,
            0x14 => self.exticr[3] = value & 0xFFFF,
            0x1C => self.mapr2 = value,
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
        assert_eq!(rd32(&b, 0x04), 0x001F_FFFF); // only the 21 remap bits
    }

    #[test]
    fn exticr_upper_half_reads_zero() {
        // EXTICR1..4 implement four 4-bit fields in [15:0]; [31:16] reserved → 0.
        // Silicon-verified on F103 (stm32f1_exec_oracle::afio_exticr1_upper...).
        let mut a = Afio::new();
        for off in [0x08u64, 0x0C, 0x10, 0x14] {
            a.write_u32(off, 0xFFFF_FFFF).unwrap();
            assert_eq!(rd32(&a, off), 0x0000_FFFF, "EXTICR @ 0x{off:02X}");
        }
    }
}
