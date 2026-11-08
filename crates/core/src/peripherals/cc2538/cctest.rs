// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CC2538 test control (`cc2538_cctest`, instance CC_TESTCTRL) at
//! 0x4401_0000 (SWRU319C §23.16.4, Table 23-11 "CCTEST Register Summary").
//!
//! | Offset | Name | Field | Reset |
//! | --- | --- | --- | --- |
//! | 0x00 | CCTEST_IO | SC \[0\]: pad output strength | 0 |
//! | 0x14..=0x30 | CCTEST_OBSSEL0..7 | EN \[7\], SEL \[6:0\]: RF observation output on PC0..PC7 | 0x00 |
//! | 0x34 | CCTEST_TR0 | bit 2 reserved RW, ADCTM \[1\], bit 0 reserved RW | 0 |
//! | 0x50 | CCTEST_USBCTRL | USB_STB \[0\]: USB PHY stand-by override | 0 |
//!
//! Bits above each field are RO reserved and read 0. Offsets not in
//! Table 23-11 read 0 and drop writes.
//!
//! Modelled: storage and read-back at the documented reset values and field
//! widths. NOT modelled: the effects. SC does not change pad drive strength,
//! OBSSELx.EN does not put an RF Core observation signal on PCx, ADCTM does
//! not gate the SOC_ADC temperature channel (the `cc2538_soc_adc` model always
//! converts it), and USB_STB has no PHY to act on.

use crate::{Peripheral, SimResult};

const IO: u64 = 0x00;
const OBSSEL_BASE: u64 = 0x14;
const OBSSELS: usize = 8;
const TR0: u64 = 0x34;
const USBCTRL: u64 = 0x50;

const IO_MASK: u32 = 0x1;
const OBSSEL_MASK: u32 = 0xFF;
const TR0_MASK: u32 = 0x7;
const USBCTRL_MASK: u32 = 0x1;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Cc2538CcTest {
    io: u32,
    obssel: [u32; OBSSELS],
    tr0: u32,
    usbctrl: u32,
}

impl Cc2538CcTest {
    pub fn new() -> Self {
        Self::default()
    }

    /// The register a word offset names and its field mask, if Table 23-11
    /// defines one.
    fn slot(&mut self, offset: u64) -> Option<(&mut u32, u32)> {
        match offset {
            IO => Some((&mut self.io, IO_MASK)),
            o if (OBSSEL_BASE..TR0).contains(&o) => Some((
                &mut self.obssel[((o - OBSSEL_BASE) / 4) as usize],
                OBSSEL_MASK,
            )),
            TR0 => Some((&mut self.tr0, TR0_MASK)),
            USBCTRL => Some((&mut self.usbctrl, USBCTRL_MASK)),
            _ => None,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            IO => self.io,
            o if (OBSSEL_BASE..TR0).contains(&o) => self.obssel[((o - OBSSEL_BASE) / 4) as usize],
            TR0 => self.tr0,
            USBCTRL => self.usbctrl,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if let Some((reg, mask)) = self.slot(offset) {
            *reg = value & mask;
        }
    }
}

impl Peripheral for Cc2538CcTest {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let lane = (offset & 3) * 8;
        Ok((self.read_reg(offset & !3) >> lane) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    /// Every field sits in byte 0 of its register, so a byte store to lane 0
    /// is the whole write; the upper lanes are reserved.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_reg(offset, value as u32);
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn on_system_reset(&mut self) {
        *self = Self::new();
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    /// Register storage only: no `tick()`/`tick_elapsed()` override and no
    /// IRQ, DMA or event output, so (like the stub it replaced) it can neither
    /// block walk deletion nor needs a per-cycle visit. Leaving the defaults on
    /// kept the walk alive and starved idle fast-forward.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        *self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Cc2538CcTest snapshot decode: {error}"))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_23_11_registers_reset_to_zero_and_keep_field_bits() {
        let mut cc = Cc2538CcTest::new();
        for offset in [0x00, 0x14, 0x30, 0x34, 0x50] {
            assert_eq!(cc.read_u32(offset).unwrap(), 0, "reset @ {offset:#x}");
            cc.write_u32(offset, 0xFFFF_FFFF).unwrap();
        }
        assert_eq!(cc.read_u32(0x00).unwrap(), 0x1); // IO.SC
        assert_eq!(cc.read_u32(0x14).unwrap(), 0xFF); // OBSSEL0 EN|SEL
        assert_eq!(cc.read_u32(0x30).unwrap(), 0xFF); // OBSSEL7
        assert_eq!(cc.read_u32(0x34).unwrap(), 0x7); // TR0
        assert_eq!(cc.read_u32(0x50).unwrap(), 0x1); // USBCTRL.USB_STB

        // Neighbouring OBSSELs are untouched.
        assert_eq!(cc.read_u32(0x18).unwrap(), 0);
    }

    #[test]
    fn gaps_read_zero_and_drop_writes() {
        let mut cc = Cc2538CcTest::new();
        for offset in [0x04, 0x10, 0x38, 0x4C, 0x54, 0xFC] {
            cc.write_u32(offset, 0xFFFF_FFFF).unwrap();
            assert_eq!(cc.read_u32(offset).unwrap(), 0, "gap @ {offset:#x}");
        }
    }

    #[test]
    fn byte_access_hits_lane_zero_only() {
        let mut cc = Cc2538CcTest::new();
        cc.write(0x1C, 0x82).unwrap();
        cc.write(0x1D, 0xFF).unwrap();
        assert_eq!(cc.read_u32(0x1C).unwrap(), 0x82);
        assert_eq!(cc.read(0x1C).unwrap(), 0x82);
        assert_eq!(cc.read(0x1D).unwrap(), 0);
    }

    #[test]
    fn reset_and_snapshot_round_trip() {
        let mut cc = Cc2538CcTest::new();
        cc.write_u32(0x34, 0x2).unwrap();
        cc.write_u32(0x2C, 0x81).unwrap();
        let saved = cc.snapshot();
        cc.on_system_reset();
        assert_eq!(cc.read_u32(0x34).unwrap(), 0);
        assert_eq!(cc.read_u32(0x2C).unwrap(), 0);
        cc.restore(saved).unwrap();
        assert_eq!(cc.read_u32(0x34).unwrap(), 0x2);
        assert_eq!(cc.read_u32(0x2C).unwrap(), 0x81);
    }

    #[test]
    fn stays_out_of_the_legacy_walk() {
        // Contiki-NG hello-world needs idle fast-forward to reach its second
        // etimer inside the cc2538dk CI step budget.
        let cc = Cc2538CcTest::new();
        assert!(!cc.needs_legacy_walk());
        assert!(!cc.legacy_tick_active());
    }
}
