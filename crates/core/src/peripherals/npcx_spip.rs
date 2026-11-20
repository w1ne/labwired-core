// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nuvoton NPCX SPI Peripheral Interface (SPIP), the block Renode maps as
//! `SPI.NPCX_SPIP` at 0x400D2000 on the NPCX9.
//!
//! Sources: Renode `NPCX_SPIP.cs` (register map, transfer and RBF
//! behaviour), Renode `nuvoton_npcx9.repl` (base), Zephyr `npcx.dtsi`
//! `spip0` (NVIC 57; Renode leaves the IRQ unconnected) and Zephyr
//! `spi_npcx_spip.c` (the interrupt-driven driver's use of EIW).
//!
//! | Offset | Name | Meaning |
//! | --- | --- | --- |
//! | 0x00 | SPIP_DATA | 16-bit. A write starts a transfer; a read returns the received frame and clears RBF |
//! | 0x02 | SPIP_CTL1 | bit0 SPIEN, bit2 MOD (1 = 16-bit), bit5 EIR, bit6 EIW, bit7 SCM, bit8 SCIDL, bits9..15 SCDV |
//! | 0x04 | SPIP_STAT | bit0 BSY (always 0), bit1 RBF |
//!
//! Modelled:
//! - a `SPIP_DATA` write with SPIEN set clocks one byte (or, with MOD, the
//!   low byte then the high byte, as Renode does) through every attached
//!   device at once and sets RBF. With SPIEN clear the write is dropped;
//! - a `SPIP_DATA` read with SPIEN set returns the frame (the low byte only
//!   in 8-bit mode) and clears RBF and the frame; with SPIEN clear it reads 0
//!   and RBF stays, as Renode;
//! - SCM, SCIDL and SCDV read back but do not change the transfer;
//! - the IRQ as a LEVEL: `EIR && RBF` (Renode) or `EIW && !RBF`. The EIW
//!   term is an assumption from the Zephyr ISR, which feeds the next frame
//!   when BSY and RBF are both clear; Renode ignores EIW. The datasheet was
//!   not checked.
//!
//! Devices: every attached device takes every byte; with several devices the
//! first non-zero MISO byte wins (the [`SpiDevice`] v1 convention) and with
//! none MISO reads 0. Chip select is a GPIO on the NPCX, so devices see no
//! `cs_select` / `cs_release` from this block.
//!
//! Byte access: a byte write to offset 0 transfers that byte (high byte 0);
//! a byte write to offset 1 is dropped. A byte read of offset 0 has the same
//! effect as a 16-bit read; a byte read of offset 1 returns the high byte
//! without side effects.
//!
//! NOT modelled: BSY (the transfer ends at once), SPI modes and clock
//! polarity, the clock divider and transfer timing, the PWDWN_CTL4 clock
//! gate.

use crate::peripherals::device::SpiDevice;
use crate::{Peripheral, SimResult};
use std::cell::Cell;

const DATA: u64 = 0x00;
const CTL1: u64 = 0x02;
const STAT: u64 = 0x04;

pub const CTL1_SPIEN: u16 = 1 << 0;
pub const CTL1_MOD: u16 = 1 << 2;
pub const CTL1_EIR: u16 = 1 << 5;
pub const CTL1_EIW: u16 = 1 << 6;
/// Bits that hold state: SPIEN, MOD, EIR, EIW, SCM, SCIDL, SCDV.
const CTL1_MASK: u16 = 0xFFE5;

pub const STAT_RBF: u8 = 1 << 1;

/// NPCX SPIP (see the module docs).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct NpcxSpip {
    ctl1: u16,
    data: Cell<u16>,
    rbf: Cell<bool>,
    #[serde(skip)]
    devices: Vec<Box<dyn SpiDevice>>,
}

impl std::fmt::Debug for NpcxSpip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NpcxSpip")
            .field("ctl1", &self.ctl1)
            .field("data", &self.data.get())
            .field("rbf", &self.rbf.get())
            .field("devices", &self.devices.len())
            .finish()
    }
}

impl Default for NpcxSpip {
    fn default() -> Self {
        Self::new()
    }
}

impl NpcxSpip {
    pub fn new() -> Self {
        Self {
            ctl1: 0,
            data: Cell::new(0),
            rbf: Cell::new(false),
            devices: Vec::new(),
        }
    }

    fn enabled(&self) -> bool {
        self.ctl1 & CTL1_SPIEN != 0
    }

    fn line(&self) -> bool {
        let rbf = self.rbf.get();
        (self.ctl1 & CTL1_EIR != 0 && rbf) || (self.ctl1 & CTL1_EIW != 0 && !rbf)
    }

    fn exchange(&mut self, mosi: u8) -> u8 {
        let mut miso = 0u8;
        for dev in &mut self.devices {
            let out = dev.transfer(mosi);
            if miso == 0 {
                miso = out;
            }
        }
        miso
    }

    fn write_data(&mut self, value: u16) {
        if !self.enabled() {
            return;
        }
        let mut frame = u16::from(self.exchange(value as u8));
        if self.ctl1 & CTL1_MOD != 0 {
            frame |= u16::from(self.exchange((value >> 8) as u8)) << 8;
        }
        self.data.set(frame);
        self.rbf.set(true);
    }

    fn read_data(&self) -> u16 {
        if !self.enabled() {
            return 0;
        }
        self.rbf.set(false);
        let frame = self.data.replace(0);
        if self.ctl1 & CTL1_MOD != 0 {
            frame
        } else {
            frame & 0xFF
        }
    }

    fn stat(&self) -> u8 {
        if self.rbf.get() {
            STAT_RBF
        } else {
            0
        }
    }

    fn reset_registers(&mut self) {
        self.ctl1 = 0;
        self.data.set(0);
        self.rbf.set(false);
    }
}

impl Peripheral for NpcxSpip {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match offset {
            DATA => self.read_data() as u8,
            0x01 => (self.data.get() >> 8) as u8,
            CTL1 => self.ctl1 as u8,
            0x03 => (self.ctl1 >> 8) as u8,
            STAT => self.stat(),
            _ => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            DATA => self.write_data(u16::from(value)),
            CTL1 => self.ctl1 = (self.ctl1 & 0xFF00) | (u16::from(value) & CTL1_MASK),
            0x03 => self.ctl1 = (self.ctl1 & 0x00FF) | ((u16::from(value) << 8) & CTL1_MASK),
            _ => {}
        }
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(match offset {
            DATA => self.read_data(),
            CTL1 => self.ctl1,
            STAT => u16::from(self.stat()),
            _ => 0,
        })
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        match offset {
            DATA => self.write_data(value),
            CTL1 => self.ctl1 = value & CTL1_MASK,
            _ => {}
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let lo = u32::from(self.read_u16(offset)?);
        let hi = u32::from(self.read_u16(offset + 2)?);
        Ok(lo | (hi << 16))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_u16(offset, value as u16)?;
        self.write_u16(offset + 2, (value >> 16) as u16)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(match offset {
            DATA => self.data.get() as u8,
            0x01 => (self.data.get() >> 8) as u8,
            CTL1 => self.ctl1 as u8,
            0x03 => (self.ctl1 >> 8) as u8,
            STAT => self.stat(),
            _ => 0,
        })
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn spi_attached_devices(&self) -> Option<&Vec<Box<dyn SpiDevice>>> {
        Some(&self.devices)
    }

    fn spi_attached_devices_mut(&mut self) -> Option<&mut Vec<Box<dyn SpiDevice>>> {
        Some(&mut self.devices)
    }

    fn on_system_reset(&mut self) {
        self.reset_registers();
    }

    fn on_node_reset(&mut self) {
        self.reset_registers();
    }

    /// A `SPIP_DATA` read clears RBF, which moves the EIR / EIW level.
    fn reads_can_deassert_irq(&self) -> bool {
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.line())
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> crate::PeripheralTickResult {
        crate::PeripheralTickResult {
            irq: self.line(),
            ..Default::default()
        }
    }

    /// The line changes only through this block's own MMIO, after which the
    /// bus re-checks this predicate, so an idle controller leaves the walk.
    fn legacy_tick_active(&self) -> bool {
        self.line()
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("NpcxSpip snapshot decode: {error}"))
        })?;
        let devices = std::mem::take(&mut self.devices);
        *self = restored;
        self.devices = devices;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Returns each MOSI byte plus one and logs it.
    struct Echo {
        log: Arc<Mutex<Vec<u8>>>,
    }

    impl SpiDevice for Echo {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.log.lock().unwrap().push(mosi);
            mosi.wrapping_add(1)
        }
        fn cs_pin(&self) -> &str {
            ""
        }
    }

    fn with_echo() -> (NpcxSpip, Arc<Mutex<Vec<u8>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut spip = NpcxSpip::new();
        spip.devices.push(Box::new(Echo { log: log.clone() }));
        (spip, log)
    }

    #[test]
    fn disabled_block_drops_writes_and_reads_zero() {
        let (mut spip, log) = with_echo();
        spip.write_u16(DATA, 0x5A).unwrap();
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(spip.read_u16(STAT).unwrap(), 0);
        assert_eq!(spip.read_u16(DATA).unwrap(), 0);
    }

    #[test]
    fn eight_bit_transfer_sets_and_read_clears_rbf() {
        let (mut spip, log) = with_echo();
        spip.write_u16(CTL1, CTL1_SPIEN).unwrap();
        spip.write_u16(DATA, 0x125A).unwrap();
        assert_eq!(*log.lock().unwrap(), vec![0x5A]);
        assert_eq!(spip.read_u16(STAT).unwrap(), u16::from(STAT_RBF));
        assert_eq!(spip.read_u16(DATA).unwrap(), 0x5B);
        assert_eq!(spip.read_u16(STAT).unwrap(), 0);
        assert_eq!(spip.read_u16(DATA).unwrap(), 0);
    }

    #[test]
    fn sixteen_bit_mode_sends_low_byte_first() {
        let (mut spip, log) = with_echo();
        spip.write_u16(CTL1, CTL1_SPIEN | CTL1_MOD).unwrap();
        spip.write_u16(DATA, 0x1234).unwrap();
        assert_eq!(*log.lock().unwrap(), vec![0x34, 0x12]);
        assert_eq!(spip.read_u16(DATA).unwrap(), 0x1335);
    }

    #[test]
    fn no_device_reads_zero_frame() {
        let mut spip = NpcxSpip::new();
        spip.write_u16(CTL1, CTL1_SPIEN).unwrap();
        spip.write(DATA, 0xA5).unwrap();
        assert_eq!(spip.read(STAT).unwrap(), STAT_RBF);
        assert_eq!(spip.read(DATA).unwrap(), 0);
        assert_eq!(spip.read(STAT).unwrap(), 0);
    }

    #[test]
    fn ctl1_reads_back_and_masks_reserved_bits() {
        let mut spip = NpcxSpip::new();
        spip.write_u16(CTL1, 0xFFFF).unwrap();
        assert_eq!(spip.read_u16(CTL1).unwrap(), 0xFFE5);
        spip.write(0x03, 0x12).unwrap();
        assert_eq!(spip.read_u16(CTL1).unwrap(), 0x12E5);
    }

    #[test]
    fn irq_follows_eir_and_eiw() {
        let (mut spip, _) = with_echo();
        spip.write_u16(CTL1, CTL1_SPIEN | CTL1_EIR).unwrap();
        assert_eq!(spip.irq_line_level(), Some(false));
        spip.write_u16(DATA, 1).unwrap();
        assert_eq!(spip.irq_line_level(), Some(true));
        spip.read_u16(DATA).unwrap();
        assert_eq!(spip.irq_line_level(), Some(false));
        spip.write_u16(CTL1, CTL1_SPIEN | CTL1_EIR | CTL1_EIW)
            .unwrap();
        assert_eq!(spip.irq_line_level(), Some(true));
        spip.write_u16(CTL1, CTL1_SPIEN | CTL1_EIW).unwrap();
        spip.write_u16(DATA, 1).unwrap();
        assert_eq!(spip.irq_line_level(), Some(false));
    }

    #[test]
    fn system_reset_clears_state_and_keeps_devices() {
        let (mut spip, _) = with_echo();
        spip.write_u16(CTL1, CTL1_SPIEN | CTL1_EIR).unwrap();
        spip.write_u16(DATA, 1).unwrap();
        spip.on_system_reset();
        assert_eq!(spip.read_u16(CTL1).unwrap(), 0);
        assert_eq!(spip.read_u16(STAT).unwrap(), 0);
        assert_eq!(spip.devices.len(), 1);
    }
}
