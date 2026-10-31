// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP LPC111x / LPC11Cxx / LPC13xx GPIO port (`GPIOn` @ 0x5000_0000 +
//! 0x1_0000·n on the AHB).
//!
//! Source: NXP's CMSIS-SVD `LPC11xx_v6a.svd` (cmsis-svd-data, `GPIO0`..`GPIO3`
//! peripherals, `GPIO1..3` derivedFrom `GPIO0`) for the offsets and reset
//! values, and the NXP CMSIS device header `LPC11xx.h` (`LPC_GPIO_TypeDef`:
//! `MASKED_ACCESS[4095]` then `DATA` at 0x3FFC) for the masked-data window.
//!
//! Layout:
//! - `DATA` window 0x0000..0x3FFC: address bits \[13:2\] are a pin mask. A read
//!   returns the pad level of the masked pins (0 elsewhere); a write changes
//!   only the masked bits of the output latch. 0x3FFC (mask 0xFFF) is the
//!   plain `DATA` register.
//! - `DIR` 0x8000 (1 = output), `IS` 0x8004 (1 = level), `IBE` 0x8008 (both
//!   edges), `IEV` 0x800C (1 = rising / high), `IE` 0x8010, `RIS` 0x8014 (RO),
//!   `MIS` 0x8018 (RO, `RIS & IE`), `IC` 0x801C (WO, write 1 clears an edge).
//!   All 12 bits wide; reset 0.
//!
//! Modelled: the masked data window, the output latch, the pad level (driven
//! when `DIR` = 1, otherwise the outside world, otherwise pulled up — the
//! IOCON reset `MODE` = pull-up), edge and level detection from that pad level
//! (an output pin sees its own edges, as on silicon), and the port interrupt
//! as a LEVEL line (`MIS != 0`).
//! NOT modelled: IOCON (pin function, `MODE` pull selection, hysteresis, open
//! drain) — every pad is treated as GPIO with the reset pull-up; the
//! PIO0_0..PIO1_0 start logic wake-up interrupts (SYSCON `STARTERP0`).
//!
//! The same ARM PL061 lineage as `cc2538_pl061_gpio`, but kept separate: the
//! data mask is 12 bits wide, the control block sits at 0x8000 (not 0x400),
//! and the CC2538's AFSEL/LOCK/PMUX/power-up-interrupt registers do not exist.

use crate::pins::{External, InputChange, PadDriver, PinPort, Pull};
use crate::{Peripheral, SimResult};

/// Pins per port (PIO0..PIO2 are 12 wide; PIO3 bonds 6 on LPC111x).
pub const PINS: u8 = 12;
const PIN_MASK: u16 = (1 << PINS) - 1;

/// End of the masked `DATA` window (exclusive).
pub const DATA_END: u64 = 0x4000;
pub const DIR: u64 = 0x8000;
pub const IS: u64 = 0x8004;
pub const IBE: u64 = 0x8008;
pub const IEV: u64 = 0x800C;
pub const IE: u64 = 0x8010;
pub const RIS: u64 = 0x8014;
pub const MIS: u64 = 0x8018;
pub const IC: u64 = 0x801C;

/// One LPC11xx GPIO port (see the module docs for what is modelled).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Lpc11xxGpio {
    /// Output latch, written through the masked `DATA` window.
    out: u16,
    dir: u16,
    is: u16,
    ibe: u16,
    iev: u16,
    ie: u16,
    /// Edge-detected interrupt bits (cleared by `IC`). Level-sensitive pins
    /// are not stored here; their `RIS` bit is computed from the pad.
    edge: u16,
    /// Pad levels at the last recompute, for edge detection.
    last_level: u16,
    /// Pins the outside world drives (`ext_level` holds their levels). Two
    /// masks rather than a per-pin list: the bus reads the whole input word
    /// every step, so the pad level must be bit arithmetic.
    ext_driven: u16,
    ext_level: u16,
}

impl Lpc11xxGpio {
    pub fn new() -> Self {
        let mut s = Self::default();
        s.last_level = s.levels();
        s
    }

    /// Pad levels: the latch where `DIR` = 1, otherwise the outside world,
    /// otherwise the IOCON reset pull-up.
    fn levels(&self) -> u16 {
        let undriven = (self.ext_level & self.ext_driven) | !self.ext_driven;
        ((self.dir & self.out) | (!self.dir & undriven)) & PIN_MASK
    }

    fn pad_level(&self, pin: u8) -> bool {
        self.levels() & (1 << pin) != 0
    }

    /// Re-sample the pads and latch any edges the sense configuration
    /// selects. Called after every write that can move a pad.
    fn sample(&mut self) {
        let now = self.levels();
        let rose = now & !self.last_level;
        let fell = !now & self.last_level;
        let edge_pins = !self.is & PIN_MASK;
        let both = edge_pins & self.ibe;
        let single = edge_pins & !self.ibe;
        self.edge |=
            (both & (rose | fell)) | (single & self.iev & rose) | (single & !self.iev & fell);
        self.last_level = now;
    }

    /// Raw interrupt status: latched edges plus the live level detectors.
    fn ris(&self) -> u16 {
        let lvl = self.levels();
        let level_hits = self.is & !(lvl ^ self.iev) & PIN_MASK;
        (self.edge & !self.is) | level_hits
    }

    fn mis(&self) -> u16 {
        self.ris() & self.ie
    }

    fn reg(&self, word: u64) -> u16 {
        match word {
            DIR => self.dir,
            IS => self.is,
            IBE => self.ibe,
            IEV => self.iev,
            IE => self.ie,
            RIS => self.ris(),
            MIS => self.mis(),
            _ => 0,
        }
    }

    fn read_byte(&self, offset: u64) -> u8 {
        let word = offset & !3;
        let shift = (offset % 4) * 8;
        let value = if offset < DATA_END {
            let mask = ((word >> 2) as u16) & PIN_MASK;
            self.levels() & mask
        } else {
            self.reg(word)
        };
        (u32::from(value) >> shift) as u8
    }

    fn write_byte(&mut self, offset: u64, value: u8) {
        let word = offset & !3;
        let shift = (offset % 4) * 8;
        if shift >= 16 {
            return;
        }
        let lane: u16 = 0xFF << shift;
        let data = u16::from(value) << shift;
        let merge = |reg: &mut u16| *reg = ((*reg & !lane) | data) & PIN_MASK;
        if offset < DATA_END {
            let mask = ((word >> 2) as u16) & PIN_MASK & lane;
            self.out = (self.out & !mask) | (data & mask);
        } else {
            match word {
                DIR => merge(&mut self.dir),
                IS => merge(&mut self.is),
                IBE => merge(&mut self.ibe),
                IEV => merge(&mut self.iev),
                IE => merge(&mut self.ie),
                IC => self.edge &= !(data & PIN_MASK),
                _ => return,
            }
        }
        self.sample();
    }
}

impl Peripheral for Lpc11xxGpio {
    /// A register bank: no `tick()`; the interrupt is a level recomputed on
    /// access.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.read_byte(offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_byte(offset, value);
        Ok(())
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.mis() != 0)
    }

    /// A node reset clears the register state; the outside world stays
    /// connected (a held button is not register state).
    fn on_node_reset(&mut self) {
        let (driven, level) = (self.ext_driven, self.ext_level);
        *self = Self::new();
        self.ext_driven = driven;
        self.ext_level = level;
        self.last_level = self.levels();
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        (pin < PINS).then(|| self.pad_level(pin))
    }

    /// One computed word: the bus polls this every step.
    fn read_gpio_input_word(&self) -> u32 {
        u32::from(self.levels())
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (pin < PINS).then(|| self.dir & self.out & (1 << pin) != 0)
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        (pin < PINS).then(|| self.dir & (1 << pin) != 0)
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        *self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Lpc11xxGpio snapshot decode: {error}"))
        })?;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize Lpc11xxGpio")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        *self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "Lpc11xxGpio runtime snapshot decode: {error}"
            ))
        })?;
        Ok(())
    }
}

impl PinPort for Lpc11xxGpio {
    fn pin_count(&self) -> u8 {
        PINS
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if pin >= PINS {
            return None;
        }
        Some(if self.dir & (1 << pin) != 0 {
            PadDriver::drive(self.out & (1 << pin) != 0)
        } else {
            PadDriver::released(Pull::Up)
        })
    }

    fn external(&self, pin: u8) -> External {
        let bit = 1u16 << pin.min(15);
        if pin < PINS && self.ext_driven & bit != 0 {
            External::Level(self.ext_level & bit != 0)
        } else {
            External::Released
        }
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= PINS {
            return None;
        }
        let before = self.pad_level(pin);
        let bit = 1u16 << pin;
        match ext.level() {
            Some(level) => {
                self.ext_driven |= bit;
                self.ext_level = (self.ext_level & !bit) | if level { bit } else { 0 };
            }
            None => {
                self.ext_driven &= !bit;
                self.ext_level &= !bit;
            }
        }
        self.sample();
        let after = self.pad_level(pin);
        Some(InputChange { before, after })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        (pin < PINS).then(|| self.pad_level(pin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w32(g: &mut Lpc11xxGpio, off: u64, v: u32) {
        g.write_u32(off, v).unwrap();
    }

    fn r32(g: &Lpc11xxGpio, off: u64) -> u32 {
        g.read_u32(off).unwrap()
    }

    /// Masked address for the pins in `mask`.
    fn data(mask: u16) -> u64 {
        u64::from(mask) << 2
    }

    #[test]
    fn reset_reads_pulled_up_inputs_and_zero_config() {
        let g = Lpc11xxGpio::new();
        assert_eq!(r32(&g, 0x3FFC), 0xFFF);
        for off in [DIR, IS, IBE, IEV, IE, RIS, MIS] {
            assert_eq!(r32(&g, off), 0, "offset {off:#x}");
        }
        assert_eq!(g.irq_line_level(), Some(false));
    }

    #[test]
    fn masked_write_changes_only_the_addressed_pins() {
        let mut g = Lpc11xxGpio::new();
        w32(&mut g, DIR, 0xFFF);
        w32(&mut g, 0x3FFC, 0);
        // Mask = PIO7 only; the value sets every bit.
        w32(&mut g, data(1 << 7), 0xFFF);
        assert_eq!(r32(&g, 0x3FFC), 1 << 7);
        // A read through a mask returns only the masked pins.
        assert_eq!(r32(&g, data(1 << 7)), 1 << 7);
        assert_eq!(r32(&g, data(1 << 6)), 0);
        // Mask 0 (offset 0) writes nothing.
        w32(&mut g, 0, 0);
        assert_eq!(r32(&g, 0x3FFC), 1 << 7);
        assert_eq!(g.read_gpio_output(7), Some(true));
        assert_eq!(g.read_gpio_output(6), Some(false));
    }

    #[test]
    fn byte_write_to_the_high_lane_uses_its_part_of_the_mask() {
        let mut g = Lpc11xxGpio::new();
        w32(&mut g, DIR, 0xFFF);
        w32(&mut g, 0x3FFC, 0);
        // strb to byte 1 of the full-mask word drives PIO8..11.
        g.write(0x3FFD, 0x0F).unwrap();
        assert_eq!(r32(&g, 0x3FFC), 0xF00);
    }

    #[test]
    fn inputs_follow_the_outside_world() {
        let mut g = Lpc11xxGpio::new();
        g.set_external(3, External::Level(false));
        assert_eq!(r32(&g, 0x3FFC), 0xFFF & !(1 << 3));
        // The latch does not reach an input pad.
        w32(&mut g, 0x3FFC, 0xFFF);
        assert_eq!(g.input(3), Some(false));
    }

    #[test]
    fn rising_edge_latches_until_ic_and_drives_the_line_through_ie() {
        let mut g = Lpc11xxGpio::new();
        w32(&mut g, DIR, 1 << 7);
        w32(&mut g, data(1 << 7), 0);
        w32(&mut g, IEV, 1 << 7); // rising
        w32(&mut g, data(1 << 7), 1 << 7);
        assert_eq!(r32(&g, RIS), 1 << 7);
        assert_eq!(r32(&g, MIS), 0);
        assert_eq!(g.irq_line_level(), Some(false));
        w32(&mut g, IE, 1 << 7);
        assert_eq!(r32(&g, MIS), 1 << 7);
        assert_eq!(g.irq_line_level(), Some(true));
        // A falling edge does not clear a rising-edge detector.
        w32(&mut g, data(1 << 7), 0);
        assert_eq!(r32(&g, RIS), 1 << 7);
        w32(&mut g, IC, 1 << 7);
        assert_eq!(r32(&g, RIS), 0);
        assert_eq!(g.irq_line_level(), Some(false));
    }

    #[test]
    fn falling_and_both_edge_sense() {
        let mut g = Lpc11xxGpio::new();
        // IEV = 0: falling edge on an input pulled low from outside.
        g.set_external(2, External::Level(false));
        assert_eq!(r32(&g, RIS), 1 << 2);
        w32(&mut g, IC, 1 << 2);
        // IBE: the rising edge back also latches.
        w32(&mut g, IBE, 1 << 2);
        g.set_external(2, External::Level(true));
        assert_eq!(r32(&g, RIS), 1 << 2);
    }

    #[test]
    fn level_sense_follows_the_pad_and_ignores_ic() {
        let mut g = Lpc11xxGpio::new();
        w32(&mut g, IS, 1 << 4);
        w32(&mut g, IE, 1 << 4);
        // IEV = 0 → active low; pin idles high.
        assert_eq!(g.irq_line_level(), Some(false));
        g.set_external(4, External::Level(false));
        assert_eq!(r32(&g, MIS), 1 << 4);
        w32(&mut g, IC, 1 << 4);
        assert_eq!(r32(&g, MIS), 1 << 4, "IC does not clear a level detector");
        g.set_external(4, External::Level(true));
        assert_eq!(g.irq_line_level(), Some(false));
    }

    #[test]
    fn ris_and_mis_are_read_only() {
        let mut g = Lpc11xxGpio::new();
        w32(&mut g, RIS, 0xFFF);
        w32(&mut g, MIS, 0xFFF);
        assert_eq!(r32(&g, RIS), 0);
        assert_eq!(r32(&g, MIS), 0);
    }

    #[test]
    fn input_word_matches_the_per_pin_levels() {
        let mut g = Lpc11xxGpio::new();
        w32(&mut g, DIR, 0x0F0);
        w32(&mut g, 0x3FFC, 0x0A0);
        g.set_external(1, External::Level(false));
        g.set_external(5, External::Level(false)); // output pin: latch wins
        let per_pin = (0..PINS).fold(0u32, |w, p| w | (u32::from(g.input(p).unwrap()) << p));
        assert_eq!(g.read_gpio_input_word(), per_pin);
        // Inputs 0-3/8-11 pulled up except PIO1 (0xF0D); outputs 5 and 7 high.
        assert_eq!(per_pin, 0xFAD);
        assert_eq!(g.external(1), External::Level(false));
        g.set_external(1, External::Released);
        assert_eq!(g.external(1), External::Released);
        assert_eq!(g.input(1), Some(true));
    }

    #[test]
    fn node_reset_keeps_the_outside_world() {
        let mut g = Lpc11xxGpio::new();
        g.set_external(0, External::Level(false));
        w32(&mut g, DIR, 0xFFF);
        g.on_node_reset();
        assert_eq!(r32(&g, DIR), 0);
        assert_eq!(g.input(0), Some(false));
        assert_eq!(r32(&g, RIS), 0, "reset does not invent an edge");
    }
}
