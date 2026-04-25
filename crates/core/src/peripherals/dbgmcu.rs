// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

use crate::SimResult;

/// DBGMCU — debug-MCU peripheral. STM32 firmware reads
/// `DBGMCU_IDCODE` at base + 0x00 to identify the chip family/revision
/// (e.g., HAL `LL_DBGMCU_GetDeviceID`, vendor self-test code, the
/// CMSIS-DAP layer used by some bootloaders). Without this peripheral
/// firmware that probes IDCODE either bus-faults or reads zeros.
///
/// Registers (per ARM Cortex-M debug architecture + STM32 RM):
///   0x00 IDCODE  R/O  bits[31:16]=REV_ID, bits[11:0]=DEV_ID
///   0x04 CR      R/W  freeze-on-halt control bits (modelled as a latch)
///   0x08 APB1_FZ R/W  peripheral freeze masks (latched, no semantics)
///   0x0C APB2_FZ R/W  same
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Dbgmcu {
    /// IDCODE value to report. Set per-chip via the YAML config; defaults
    /// to 0 which would surface as a wrong probe result, so configs SHOULD
    /// set this.
    pub idcode: u32,
    pub cr: u32,
    pub apb1_fz: u32,
    pub apb2_fz: u32,
}

impl Dbgmcu {
    pub fn new(idcode: u32) -> Self {
        Self { idcode, cr: 0, apb1_fz: 0, apb2_fz: 0 }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.idcode,
            0x04 => self.cr,
            0x08 => self.apb1_fz,
            0x0C => self.apb2_fz,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // IDCODE is read-only on real hardware.
            0x00 => {}
            0x04 => self.cr = value,
            0x08 => self.apb1_fz = value,
            0x0C => self.apb2_fz = value,
            _ => {}
        }
    }
}

impl crate::Peripheral for Dbgmcu {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask = 0xFF << (byte * 8);
        v &= !mask;
        v |= (value as u32) << (byte * 8);
        self.write_reg(reg, v);
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
