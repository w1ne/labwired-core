// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP LPC-style high-speed GPIO (i.MX RT500 `GPIO` @ 0x4010_0000, the same
//! block as LPC55xx/LPC54xxx).
//!
//! Source: the MCUXpresso SDK device header `MIMXRT595S_cm33.h` (`GPIO_Type`)
//! for the layout; UM11147 (i.MX RT500 reference manual) chapter "GPIO" for
//! the semantics.
//!
//! Pins are numbered `32 * port + pin`: P0_14 = 14, P1_0 = 32, P3_17 = 113.
//! The byte/word pin arrays have seven ports (`B[7][32]`, `W[7][32]`), so
//! ports 0-6 have pads here (224 pins). The port registers have eight slots
//! (`DIR[8]` ...); slot 7 is stored but drives no pad. Whether each of the
//! 32 pins of a port is bonded out on a given package is not modelled.
//!
//! Layout:
//! - `B[port][pin]` @ `0x0000 + 0x20·port + pin`: one byte per pin; reads the
//!   pin state (0/1), a write sets the output latch from bit 0.
//! - `W[port][pin]` @ `0x1000 + 0x80·port + 4·pin`: reads 0 or 0xFFFF_FFFF,
//!   a non-zero write sets the latch bit, zero clears it.
//! - Per-port words @ `base + 4·port`: `DIR` 0x2000, `MASK` 0x2080, `PIN`
//!   0x2100, `MPIN` 0x2180, `SET` 0x2200, `CLR` 0x2280, `NOT` 0x2300,
//!   `DIRSET` 0x2380, `DIRCLR` 0x2400, `DIRNOT` 0x2480, `INTENA` 0x2500,
//!   `INTENB` 0x2580, `INTPOL` 0x2600, `INTEDG` 0x2680, `INTSTATA` 0x2700,
//!   `INTSTATB` 0x2780.
//!
//! Modelled: the output latch through every view (B, W, PIN, MPIN under
//! MASK, SET, CLR, NOT), the direction through DIR/DIRSET/DIRCLR/DIRNOT, and
//! the pin state: the latch for an output pin, otherwise the level the
//! outside world presents (an undriven input reads 0). `SET` reads back the
//! latch; `PIN` reads the pin state.
//! NOT modelled: the IOPCTL pad configuration (function select, pull-up/
//! down, input buffer enable — a separate block; so a pin reads back even
//! with IBENA = 0 and an input pin has no pull), the pin interrupts (INTENA/
//! INTENB/INTPOL/INTEDG are stored, INTSTATA/B never set, no NVIC line), and
//! the secure GPIO alias (SECGPIO).

use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{Peripheral, SimResult};

/// Ports with pads (`GPIO_B_COUNT`).
pub const PAD_PORTS: usize = 7;
/// Port register slots (`GPIO_DIR_COUNT`).
const PORT_SLOTS: usize = 8;
pub const PINS: usize = PAD_PORTS * 32;

const B0: u64 = 0x0000;
const B_END: u64 = B0 + 0x20 * PAD_PORTS as u64;
const W0: u64 = 0x1000;
const W_END: u64 = W0 + 0x80 * PAD_PORTS as u64;
pub const DIR: u64 = 0x2000;
pub const MASK: u64 = 0x2080;
pub const PIN: u64 = 0x2100;
pub const MPIN: u64 = 0x2180;
pub const SET: u64 = 0x2200;
pub const CLR: u64 = 0x2280;
pub const NOT: u64 = 0x2300;
pub const DIRSET: u64 = 0x2380;
pub const DIRCLR: u64 = 0x2400;
pub const DIRNOT: u64 = 0x2480;
const INTENA: u64 = 0x2500;
const INTSTATB_END: u64 = 0x2780 + 4 * PORT_SLOTS as u64;
/// The register groups are 0x80 apart; only the first `PORT_SLOTS` words of
/// each group exist.
const GROUP_STRIDE: u64 = 0x80;

/// The LPC GPIO block (see the module docs for what is modelled).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LpcGpio {
    dir: [u32; PORT_SLOTS],
    mask: [u32; PORT_SLOTS],
    latch: [u32; PORT_SLOTS],
    /// INTENA, INTENB, INTPOL, INTEDG, INTSTATA, INTSTATB: stored only.
    int_regs: [[u32; PORT_SLOTS]; 6],
    /// Levels presented on the pads by the outside world.
    external: Vec<Option<bool>>,
}

impl Default for LpcGpio {
    fn default() -> Self {
        Self::new()
    }
}

impl LpcGpio {
    pub fn new() -> Self {
        Self {
            dir: [0; PORT_SLOTS],
            mask: [0; PORT_SLOTS],
            latch: [0; PORT_SLOTS],
            int_regs: [[0; PORT_SLOTS]; 6],
            external: vec![None; PINS],
        }
    }

    fn is_output(&self, pin: usize) -> bool {
        self.dir[pin / 32] >> (pin % 32) & 1 != 0
    }

    fn latch_bit(&self, pin: usize) -> bool {
        self.latch[pin / 32] >> (pin % 32) & 1 != 0
    }

    fn set_latch_bit(&mut self, pin: usize, level: bool) {
        let bit = 1 << (pin % 32);
        if level {
            self.latch[pin / 32] |= bit;
        } else {
            self.latch[pin / 32] &= !bit;
        }
    }

    fn pin_level(&self, pin: usize) -> bool {
        if self.is_output(pin) {
            self.latch_bit(pin)
        } else {
            self.external.get(pin).copied().flatten().unwrap_or(false)
        }
    }

    /// The PIN register view of `port`.
    fn port_state(&self, port: usize) -> u32 {
        if port >= PAD_PORTS {
            // No pads: an output reads its latch, an input reads 0.
            return self.latch[port] & self.dir[port];
        }
        (0..32).fold(0, |acc, bit| {
            acc | (u32::from(self.pin_level(port * 32 + bit)) << bit)
        })
    }

    /// Decode a per-port word register offset into (group base, port).
    fn port_reg(offset: u64) -> Option<(u64, usize)> {
        if !(DIR..INTSTATB_END).contains(&offset) {
            return None;
        }
        let group = DIR + (offset - DIR) / GROUP_STRIDE * GROUP_STRIDE;
        let port = ((offset - group) / 4) as usize;
        (port < PORT_SLOTS).then_some((group, port))
    }

    fn read_word(&self, offset: u64) -> u32 {
        if (W0..W_END).contains(&offset) {
            let pin = ((offset - W0) / 0x80 * 32 + (offset - W0) % 0x80 / 4) as usize;
            return if self.pin_level(pin) { u32::MAX } else { 0 };
        }
        let Some((group, port)) = Self::port_reg(offset) else {
            crate::census_reg!("lpc_gpio:LpcGpio", offset, "read");
            return 0;
        };
        match group {
            DIR => self.dir[port],
            MASK => self.mask[port],
            PIN => self.port_state(port),
            MPIN => self.port_state(port) & !self.mask[port],
            SET => self.latch[port],
            // CLR, NOT and the DIR set/clear/toggle words are write-only.
            CLR | NOT | DIRSET | DIRCLR | DIRNOT => 0,
            _ => self.int_regs[((group - INTENA) / GROUP_STRIDE) as usize][port],
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if (W0..W_END).contains(&offset) {
            let pin = ((offset - W0) / 0x80 * 32 + (offset - W0) % 0x80 / 4) as usize;
            self.set_latch_bit(pin, value != 0);
            return;
        }
        let Some((group, port)) = Self::port_reg(offset) else {
            crate::census_reg!("lpc_gpio:LpcGpio", offset, "write");
            return;
        };
        match group {
            DIR => self.dir[port] = value,
            MASK => self.mask[port] = value,
            PIN => self.latch[port] = value,
            MPIN => {
                let mask = self.mask[port];
                self.latch[port] = (self.latch[port] & mask) | (value & !mask);
            }
            SET => self.latch[port] |= value,
            CLR => self.latch[port] &= !value,
            NOT => self.latch[port] ^= value,
            DIRSET => self.dir[port] |= value,
            DIRCLR => self.dir[port] &= !value,
            DIRNOT => self.dir[port] ^= value,
            _ => self.int_regs[((group - INTENA) / GROUP_STRIDE) as usize][port] = value,
        }
    }

    /// True for the write-1-to-act words, where a narrow store must not
    /// read-modify-write the other lanes.
    fn is_action_word(offset: u64) -> bool {
        matches!(
            Self::port_reg(offset),
            Some((SET | CLR | NOT | DIRSET | DIRCLR | DIRNOT, _))
        )
    }
}

impl Peripheral for LpcGpio {
    /// A pure register bank: no `tick()`, no IRQ, DMA or event.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        if (B0..B_END).contains(&offset) {
            let pin = ((offset - B0) / 0x20 * 32 + (offset - B0) % 0x20) as usize;
            return Ok(u8::from(self.pin_level(pin)));
        }
        Ok((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if (B0..B_END).contains(&offset) {
            let pin = ((offset - B0) / 0x20 * 32 + (offset - B0) % 0x20) as usize;
            self.set_latch_bit(pin, value & 1 != 0);
            return Ok(());
        }
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let lane = u32::from(value) << shift;
        let word = if Self::is_action_word(aligned) || (W0..W_END).contains(&aligned) {
            lane
        } else {
            (self.read_word(aligned) & !(0xFF << shift)) | lane
        };
        self.write_word(aligned, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if (B0..B_END).contains(&offset) {
            let mut word = 0;
            for lane in 0..4 {
                word |= u32::from(self.read(offset + lane)?) << (lane * 8);
            }
            return Ok(word);
        }
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if (B0..B_END).contains(&offset) {
            for lane in 0..4 {
                self.write(offset + lane, (value >> (lane * 8)) as u8)?;
            }
            return Ok(());
        }
        self.write_word(offset & !3, value);
        Ok(())
    }

    /// A node reset clears the register state; the outside world stays
    /// connected (a held button is not register state).
    fn on_node_reset(&mut self) {
        let external = std::mem::take(&mut self.external);
        *self = Self::new();
        self.external = external;
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.pin_level(pin))
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.is_output(pin) && self.latch_bit(pin))
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.is_output(pin))
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
            crate::SimulationError::NotImplemented(format!("LpcGpio snapshot decode: {error}"))
        })?;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize LpcGpio")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        *self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "LpcGpio runtime snapshot decode: {error}"
            ))
        })?;
        Ok(())
    }
}

impl PinPort for LpcGpio {
    fn pin_count(&self) -> u8 {
        PINS as u8
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        let pin = pin as usize;
        if pin >= PINS {
            return None;
        }
        Some(if self.is_output(pin) {
            PadDriver::drive(self.latch_bit(pin))
        } else {
            PadDriver::OFF
        })
    }

    fn external(&self, pin: u8) -> External {
        match self.external.get(pin as usize).copied().flatten() {
            Some(level) => External::Level(level),
            None => External::Released,
        }
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        let pin = pin as usize;
        if pin >= PINS {
            return None;
        }
        let before = self.pin_level(pin);
        self.external[pin] = ext.level();
        let after = self.pin_level(pin);
        Some(InputChange { before, after })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.pin_level(pin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The MIMXRT595-EVK LEDs (SDK board.h): red P0_14, green P1_0, blue
    /// P3_17.
    const RED: usize = 14;
    const GREEN: usize = 32;
    const BLUE: usize = 3 * 32 + 17;

    fn port_word(group: u64, port: usize) -> u64 {
        group + 4 * port as u64
    }

    #[test]
    fn dirset_and_set_clr_drive_the_led_pins() {
        let mut g = LpcGpio::new();
        g.write_u32(port_word(DIRSET, 0), 1 << 14).unwrap();
        g.write_u32(port_word(DIRSET, 1), 1).unwrap();
        g.write_u32(port_word(DIRSET, 3), 1 << 17).unwrap();
        assert_eq!(g.read_gpio_output(RED as u8), Some(false));
        g.write_u32(port_word(SET, 0), 1 << 14).unwrap();
        g.write_u32(port_word(SET, 3), 1 << 17).unwrap();
        assert_eq!(g.read_gpio_output(RED as u8), Some(true));
        assert_eq!(g.read_gpio_output(BLUE as u8), Some(true));
        assert_eq!(g.read_u32(port_word(PIN, 0)).unwrap(), 1 << 14);
        assert_eq!(g.read_u32(port_word(SET, 3)).unwrap(), 1 << 17);
        g.write_u32(port_word(CLR, 0), 1 << 14).unwrap();
        assert_eq!(g.read_u32(port_word(PIN, 0)).unwrap(), 0);
        g.write_u32(port_word(NOT, 1), 1).unwrap();
        assert_eq!(g.read_gpio_output(GREEN as u8), Some(true));
        g.write_u32(port_word(NOT, 1), 1).unwrap();
        assert_eq!(g.read_gpio_output(GREEN as u8), Some(false));
    }

    #[test]
    fn byte_and_word_pin_views_share_the_latch() {
        let mut g = LpcGpio::new();
        g.write_u32(port_word(DIR, 3), 1 << 17).unwrap();
        g.write(0x20 * 3 + 17, 1).unwrap();
        assert_eq!(g.read(0x20 * 3 + 17).unwrap(), 1);
        let w = W0 + 0x80 * 3 + 4 * 17;
        assert_eq!(g.read_u32(w).unwrap(), u32::MAX);
        g.write_u32(w, 0).unwrap();
        assert_eq!(g.read(0x20 * 3 + 17).unwrap(), 0);
        g.write_u32(w, 0x8000_0000).unwrap();
        assert_eq!(g.read_u32(port_word(PIN, 3)).unwrap(), 1 << 17);
    }

    #[test]
    fn mpin_writes_only_unmasked_bits() {
        let mut g = LpcGpio::new();
        g.write_u32(port_word(DIR, 0), 0xFF).unwrap();
        g.write_u32(port_word(PIN, 0), 0x0F).unwrap();
        g.write_u32(port_word(MASK, 0), !0x30).unwrap();
        g.write_u32(port_word(MPIN, 0), 0xF0).unwrap();
        assert_eq!(g.read_u32(port_word(PIN, 0)).unwrap(), 0x3F);
        assert_eq!(g.read_u32(port_word(MPIN, 0)).unwrap(), 0x30);
    }

    #[test]
    fn input_pins_read_the_outside_world() {
        let mut g = LpcGpio::new();
        // SW1 on the EVK is P0_25 (SDK board.h); an undriven input reads 0.
        assert_eq!(g.read_u32(port_word(PIN, 0)).unwrap(), 0);
        g.set_external(25, External::Level(true));
        assert_eq!(g.read_u32(port_word(PIN, 0)).unwrap(), 1 << 25);
        // Turning the pin into an output shows the latch instead.
        g.write_u32(port_word(DIRSET, 0), 1 << 25).unwrap();
        assert_eq!(g.read_u32(port_word(PIN, 0)).unwrap(), 0);
        g.write_u32(port_word(DIRCLR, 0), 1 << 25).unwrap();
        assert_eq!(g.read_u32(port_word(PIN, 0)).unwrap(), 1 << 25);
    }

    #[test]
    fn narrow_set_store_does_not_disturb_other_lanes() {
        let mut g = LpcGpio::new();
        g.write_u32(port_word(DIR, 0), u32::MAX).unwrap();
        g.write_u32(port_word(PIN, 0), 0x0000_00FF).unwrap();
        // A byte store to lane 1 of SET sets bits 8..15 only.
        g.write(port_word(SET, 0) + 1, 0x01).unwrap();
        assert_eq!(g.read_u32(port_word(PIN, 0)).unwrap(), 0x0000_01FF);
        g.write(port_word(CLR, 0), 0x01).unwrap();
        assert_eq!(g.read_u32(port_word(PIN, 0)).unwrap(), 0x0000_01FE);
    }

    #[test]
    fn port_slot_seven_is_stored_without_pads() {
        let mut g = LpcGpio::new();
        g.write_u32(port_word(DIR, 7), 0xF).unwrap();
        g.write_u32(port_word(SET, 7), 0x5).unwrap();
        assert_eq!(g.read_u32(port_word(PIN, 7)).unwrap(), 0x5);
        assert_eq!(g.pin_count() as usize, PINS);
    }

    #[test]
    fn interrupt_words_are_stored_and_reset_clears() {
        let mut g = LpcGpio::new();
        g.write_u32(port_word(INTENA, 1), 3).unwrap();
        assert_eq!(g.read_u32(port_word(INTENA, 1)).unwrap(), 3);
        g.set_external(5, External::Level(true));
        g.on_node_reset();
        assert_eq!(g.read_u32(port_word(INTENA, 1)).unwrap(), 0);
        assert_eq!(g.read_gpio_input(5), Some(true));
    }
}
