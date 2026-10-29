// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 EIC slice (DS60001507F 21): sixteen external interrupt
//! channels with rise/fall/both/high/low sensing, per-channel flags and IRQ
//! lines 12..27. A pad edge reaches channel n only while the pad is handed to
//! the EIC (PORT PINCFG.PMUXEN with function A) and the controller is clocked
//! and enabled. Filtering, debouncing, asynchronous edge detection timing, NMI
//! and events are not modelled; their non-default settings are refused.
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;

const PORT: u64 = 0x41008000;
const MCLK_APBAMASK: u64 = 0x40000814;
const EIC_BUS_CLOCK: u32 = 1 << 10;
const GCLK_PCHCTRL_EIC: u64 = 0x40001c80 + 4 * 4;
const FIRST_IRQ: u32 = 12;

/// (port, pin, channel) for every EXTINT signal on function A, from the
/// ATSAMD51J19A ATDF (Microchip SAMD51 DFP 3.9.276, Apache-2.0).
const PADS: [(u8, u8, u8); 50] = [
    (0, 0, 0),
    (0, 1, 1),
    (0, 2, 2),
    (0, 3, 3),
    (0, 4, 4),
    (0, 5, 5),
    (0, 6, 6),
    (0, 7, 7),
    (0, 9, 9),
    (0, 10, 10),
    (0, 11, 11),
    (0, 12, 12),
    (0, 13, 13),
    (0, 14, 14),
    (0, 15, 15),
    (0, 16, 0),
    (0, 17, 1),
    (0, 18, 2),
    (0, 19, 3),
    (0, 20, 4),
    (0, 21, 5),
    (0, 22, 6),
    (0, 23, 7),
    (0, 24, 8),
    (0, 25, 9),
    (0, 27, 11),
    (0, 30, 14),
    (0, 31, 15),
    (1, 0, 0),
    (1, 1, 1),
    (1, 2, 2),
    (1, 3, 3),
    (1, 4, 4),
    (1, 5, 5),
    (1, 6, 6),
    (1, 7, 7),
    (1, 8, 8),
    (1, 9, 9),
    (1, 10, 10),
    (1, 11, 11),
    (1, 12, 12),
    (1, 13, 13),
    (1, 14, 14),
    (1, 15, 15),
    (1, 16, 0),
    (1, 17, 1),
    (1, 22, 6),
    (1, 23, 7),
    (1, 30, 14),
    (1, 31, 15),
];

#[derive(Clone, Copy, Debug)]
struct Edge {
    port: u8,
    pin: u8,
    level: bool,
}

#[derive(Debug, Default)]
pub struct Samd51Eic {
    ctrla: u8,
    inten: u16,
    intflag: u16,
    config: [u32; 2],
    asynch: u16,
    /// Last level seen per channel, for PINSTATE and level sensing.
    state: u16,
    pending: Vec<Edge>,
    clocked: bool,
}

impl Samd51Eic {
    pub fn new() -> Self {
        Self::default()
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd51 EIC slice: {what}"
        )))
    }
    fn enabled(&self) -> bool {
        self.ctrla & 2 != 0
    }
    fn sense(&self, channel: u8) -> u32 {
        (self.config[usize::from(channel / 8)] >> (4 * u32::from(channel % 8))) & 7
    }
    fn channel(port: u8, pin: u8) -> Option<u8> {
        PADS.iter()
            .find(|(p, n, _)| *p == port && *n == pin)
            .map(|(_, _, c)| *c)
    }
    /// Apply one validated pad level to its channel.
    fn deliver(&mut self, channel: u8, level: bool) {
        let bit = 1u16 << channel;
        let before = self.state & bit != 0;
        self.state = (self.state & !bit) | if level { bit } else { 0 };
        let fired = match self.sense(channel) {
            1 => !before && level,
            2 => before && !level,
            3 => before != level,
            4 => level,
            5 => !level,
            _ => false,
        };
        if fired {
            self.intflag |= bit;
        }
    }
    fn lines(&self) -> u16 {
        if self.enabled() {
            self.intflag & self.inten
        } else {
            0
        }
    }
    fn read_reg(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            0x00 => u32::from(self.ctrla),
            0x04 => 0, // SYNCBUSY
            0x08 => 0,
            0x0c | 0x10 => u32::from(self.inten),
            0x14 => u32::from(self.intflag),
            0x18 => u32::from(self.asynch),
            0x1c => self.config[0],
            0x20 => self.config[1],
            0x30 | 0x34 => 0,
            0x38 => u32::from(self.state),
            _ => return Self::refuse("register"),
        })
    }
    fn write_reg(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let protected = matches!(offset, 0x01 | 0x08 | 0x18 | 0x1c | 0x20 | 0x30 | 0x34);
        if protected && self.enabled() {
            return Self::refuse("enable-protected register written while enabled");
        }
        match offset {
            0x00 if value & 1 != 0 => {
                *self = Self {
                    clocked: self.clocked,
                    ..Self::default()
                }
            }
            0x00 if value & !0x12 == 0 => self.ctrla = value as u8,
            0x08 | 0x30 | 0x34 if value == 0 => {}
            0x0c => self.inten &= !(value as u16),
            0x10 => self.inten |= value as u16,
            0x14 => self.intflag &= !(value as u16),
            0x18 => self.asynch = value as u16,
            // SENSE values 0..5; FILTEN (bit 3 of each nibble) is not modelled.
            0x1c | 0x20 => {
                for n in 0..8 {
                    let nibble = (value >> (4 * n)) & 0xf;
                    if nibble & 8 != 0 || nibble > 5 {
                        return Self::refuse("filtering or reserved SENSE value");
                    }
                }
                self.config[((offset - 0x1c) / 4) as usize] = value;
            }
            _ => return Self::refuse("store outside the EIC slice"),
        }
        Ok(())
    }
}

impl Peripheral for Samd51Eic {
    fn on_system_reset(&mut self) {
        *self = Self::default();
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_reg(offset & !3)?;
        Ok((word >> (8 * (offset & 3))) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            0x00 => self.write_reg(0, u32::from(value)),
            0x01 if value == 0 => Ok(()), // NMICTRL default
            _ => Self::refuse("byte store outside CTRLA"),
        }
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset, value)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn observes_gpio_edges(&self) -> bool {
        true
    }
    fn gpio_input_edge(
        &mut self,
        port: u8,
        pin: u8,
        _before: bool,
        after: bool,
        _line: Option<u8>,
    ) -> bool {
        if Self::channel(port, pin).is_some() {
            self.pending.push(Edge {
                port,
                pin,
                level: after,
            });
        }
        false
    }
    fn needs_bus_tick(&self) -> bool {
        !self.pending.is_empty() || (self.enabled() && !self.clocked)
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        // Bus clock and generic clock (or the ULP32K clock with CKSEL).
        let apb = bus
            .read_u32(MCLK_APBAMASK)
            .is_ok_and(|m| m & EIC_BUS_CLOCK != 0);
        let gclk = self.ctrla & 0x10 != 0
            || bus
                .read_u32(GCLK_PCHCTRL_EIC)
                .is_ok_and(|c| c & (1 << 6) != 0);
        self.clocked = apb && gclk;
        for edge in std::mem::take(&mut self.pending) {
            let Some(channel) = Self::channel(edge.port, edge.pin) else {
                continue;
            };
            let group = PORT + 0x80 * u64::from(edge.port);
            let pincfg = bus.read_u8(group + 0x40 + u64::from(edge.pin)).unwrap_or(0);
            let pmux = bus
                .read_u8(group + 0x30 + u64::from(edge.pin / 2))
                .unwrap_or(0xff);
            let function = if edge.pin & 1 != 0 {
                pmux >> 4
            } else {
                pmux & 0xf
            };
            if self.enabled() && self.clocked && pincfg & 1 != 0 && function == 0 {
                self.deliver(channel, edge.level);
            }
        }
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        self.lines() != 0
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        let lines = self.lines();
        for n in 0..16 {
            report(FIRST_IRQ + n, lines & (1 << n) != 0);
        }
    }
    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        let lines = self.lines();
        PeripheralTickResult {
            explicit_irqs: (lines != 0).then(|| {
                (0..16)
                    .filter(|n| lines & (1 << n) != 0)
                    .map(|n| FIRST_IRQ + n)
                    .collect()
            }),
            ..Default::default()
        }
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "samd51_eic", "ctrla": self.ctrla, "inten": self.inten,
            "intflag": self.intflag, "config": self.config, "pinstate": self.state, "clocked": self.clocked})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(sense: u32) -> Samd51Eic {
        let mut e = Samd51Eic::new();
        e.write_u32(0x1c, sense << (4 * 6)).unwrap(); // channel 6
        e.write_u32(0x10, 1 << 6).unwrap();
        e.write(0, 2).unwrap();
        e.clocked = true;
        e
    }

    #[test]
    fn rising_edge_sets_flag_and_line_until_cleared() {
        let mut e = configured(1);
        e.deliver(6, true);
        assert_eq!(e.lines(), 1 << 6);
        e.write_u32(0x14, 1 << 6).unwrap();
        assert_eq!(e.lines(), 0);
        e.deliver(6, false);
        assert_eq!(e.lines(), 0, "falling edge does not match rise sensing");
        assert_eq!(e.read_u32(0x38).unwrap() & (1 << 6), 0);
    }

    #[test]
    fn both_edges_and_level_sense() {
        let mut e = configured(3);
        e.deliver(6, true);
        e.write_u32(0x14, 1 << 6).unwrap();
        e.deliver(6, false);
        assert_eq!(e.lines(), 1 << 6);
        let mut l = configured(5);
        l.deliver(6, false);
        assert_eq!(l.lines(), 1 << 6, "low level");
    }

    #[test]
    fn configuration_is_enable_protected_and_filtering_refused() {
        let mut e = configured(1);
        assert!(e.write_u32(0x1c, 0).is_err(), "CONFIG while enabled");
        let mut d = Samd51Eic::new();
        assert!(d.write_u32(0x1c, 0x9).is_err(), "FILTEN");
        assert!(d.write_u32(0x1c, 0x6).is_err(), "reserved SENSE");
        assert!(d.write_u32(0x08, 1).is_err(), "events");
        assert_eq!(Samd51Eic::channel(0, 14), Some(14));
        assert_eq!(Samd51Eic::channel(0, 16), Some(0));
        assert_eq!(Samd51Eic::channel(0, 8), None, "PA08 is the NMI pad");
    }
}
