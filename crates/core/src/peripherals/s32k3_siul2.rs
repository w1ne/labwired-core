// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP S32K3 SIUL2 (System Integration Unit Lite 2): pad mux + GPIO.
//!
//! Source: the S32K388 device header `S32K388_SIUL2.h` (NXP RTD BaseNXP,
//! `SIUL2_Type` @ `IP_SIUL2_BASE` = 0x4029_0000) for the register layout and
//! the MSCR bit fields, and Zephyr `drivers/gpio/gpio_nxp_siul2.c` for the bit
//! order of the parallel registers (`PGPDO`/`PGPDI` bit `15 - k` is pin
//! `16·p + k`; the driver reverses with `__RBIT`).
//!
//! Pins are numbered as the MSCR index: PTA0 = 0, PTB0 = 32, ... PTF23 = 183.
//! The S32K388 has 237 MSCRs; indices 38, 39 and 141 are reserved (no pad).
//!
//! Layout:
//! - `MIDR1/2` @ 0x04/0x08, `MIDR3/4` @ 0x200/0x204: read 0 (the header's
//!   reset value; the factory part/revision numbers are not modelled).
//! - `DISR0` @ 0x10 (write-1-to-clear), `DIRER0`, `DIRSR0`, `IREER0`,
//!   `IFEER0`, `IFER0`, `IFMCR[32]`, `IFCPR` and the `MUXn_*_EN` words
//!   @ 0x100..0x114: stored only.
//! - `MSCR[n]` @ `0x240 + 4n`: SSS \[3:0\] (0 = GPIO), PUS \[11\], PUE \[13\],
//!   INV \[17\], IBE \[19\], OBE \[21\] and the drive/slew bits, stored.
//! - `IMCR[n]` @ `0xA40 + 4n` (470 entries): stored only.
//! - `GPDOn` @ `0x1300 + (n ^ 3)`, `GPDIn` @ `0x1500 + (n ^ 3)`: one byte per
//!   pin, bit 0, with the bytes of each word in big-endian order (the header
//!   lists `GPDO3` at 0x1300 and `GPDO0` at 0x1303).
//! - `PGPDOp` @ `0x1700 + 2·(p ^ 1)`, `PGPDIp` @ `0x1740 + 2·(p ^ 1)`: 16 pins
//!   per halfword (`PGPDO1` at 0x1700, `PGPDO0` at 0x1702; `PGPDO14` at
//!   0x171E, 0x171C is reserved).
//! - `MPGPDO[p]` @ `0x1780 + 4p` (write-only): MASK \[31:16\], MPPDO \[15:0\],
//!   both in the PGPDO bit order. The mask/data bit order is taken from PGPDO
//!   by analogy (Zephyr does not use MPGPDO); it is not confirmed on silicon.
//!
//! Modelled: MSCR/IMCR storage, the GPDO output latch through all three views
//! (GPDO bytes, PGPDO halfwords, MPGPDO masked writes), and the pad level each
//! pin presents: the latch when SSS = 0 and OBE = 1, otherwise the outside
//! world, otherwise the PUE/PUS pull; an unpulled, undriven pad reads 0.
//! GPDI/PGPDI return the pad level only where IBE = 1.
//! NOT modelled: the peripheral side of a non-GPIO SSS (such a pad is simply
//! not driven by this model), INV, open drain, drive strength/slew, the input
//! mux (IMCR routing has no effect), the external interrupts/filters (DISR0
//! never sets) and the MIDR identification values.

use crate::pins::{External, InputChange, PadDriver, PinPort, Pull};
use crate::{Peripheral, SimResult};

/// MSCR count on the S32K388 (`SIUL2_MSCR_COUNT`).
pub const PINS: usize = 237;
/// MSCR indices with no pad (`valid indices: [0-37, 40-140, 142-236]`).
const RESERVED_PINS: [usize; 3] = [38, 39, 141];
const IMCR_COUNT: usize = 470;
/// Parallel ports (`PGPDO0..14`).
const PORTS: usize = 15;

const MIDR1: u64 = 0x004;
/// Last byte of `MIDR2` (0x008).
const MIDR2_LAST: u64 = 0x00B;
const DISR0: u64 = 0x010;
const MIDR3: u64 = 0x200;
/// Last byte of `MIDR4` (0x204).
const MIDR4_LAST: u64 = 0x207;
/// End (exclusive) of the stored-only interrupt/filter/mux words.
const MISC_END: u64 = 0x240;
const MSCR0: u64 = 0x240;
const MSCR_END: u64 = MSCR0 + 4 * PINS as u64;
const IMCR0: u64 = 0xA40;
const IMCR_END: u64 = IMCR0 + 4 * IMCR_COUNT as u64;
const GPDO0: u64 = 0x1300;
/// GPDO/GPDI span whole words: 237 pins round up to 240 bytes.
const GPDO_LAST: u64 = 0x13EF;
const GPDI0: u64 = 0x1500;
const GPDI_LAST: u64 = 0x15EF;
const PGPDO0: u64 = 0x1700;
const PGPDO_LAST: u64 = 0x171F;
const PGPDI0: u64 = 0x1740;
const PGPDI_LAST: u64 = 0x175F;
const MPGPDO0: u64 = 0x1780;
const MPGPDO_END: u64 = MPGPDO0 + 4 * PORTS as u64;

pub const MSCR_SSS_MASK: u32 = 0xF;
pub const MSCR_PUS: u32 = 1 << 11;
pub const MSCR_PUE: u32 = 1 << 13;
pub const MSCR_IBE: u32 = 1 << 19;
pub const MSCR_OBE: u32 = 1 << 21;

fn has_pad(pin: usize) -> bool {
    pin < PINS && !RESERVED_PINS.contains(&pin)
}

/// The pin behind bit `bit` (0..16) of parallel port `port`.
fn parallel_pin(port: usize, bit: u32) -> usize {
    port * 16 + (15 - bit as usize)
}

/// Parallel port and byte lane for a byte offset into PGPDO/PGPDI, or `None`
/// for the reserved halfword 0x1C.
fn parallel_lane(rel: u64) -> Option<(usize, u32)> {
    let port = ((rel / 2) ^ 1) as usize;
    (port < PORTS).then_some((port, (rel % 2) as u32))
}

/// The S32K3 SIUL2 block (see the module docs for what is modelled).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S32k3Siul2 {
    /// Bytes 0x000..0x240 (interrupt/filter/mux words), stored only.
    misc: Vec<u8>,
    mscr: Vec<u32>,
    imcr: Vec<u32>,
    /// `GPDOn` bit 0, per pin.
    gpdo: Vec<bool>,
    /// Levels presented on the pads by the outside world.
    external: Vec<Option<bool>>,
}

impl S32k3Siul2 {
    pub fn new() -> Self {
        Self {
            misc: vec![0; MISC_END as usize],
            mscr: vec![0; PINS],
            imcr: vec![0; IMCR_COUNT],
            gpdo: vec![false; PINS],
            external: vec![None; PINS],
        }
    }

    fn pull(&self, pin: usize) -> Pull {
        let m = self.mscr[pin];
        match (m & MSCR_PUE != 0, m & MSCR_PUS != 0) {
            (false, _) => Pull::None,
            (true, true) => Pull::Up,
            (true, false) => Pull::Down,
        }
    }

    /// The level this block drives onto `pin`, if it drives it at all.
    fn drive(&self, pin: usize) -> Option<bool> {
        if !has_pad(pin) {
            return None;
        }
        let m = self.mscr[pin];
        (m & MSCR_SSS_MASK == 0 && m & MSCR_OBE != 0).then_some(self.gpdo[pin])
    }

    /// The level on the pad: driven, otherwise the outside world, otherwise
    /// the pull (an unpulled pad reads 0).
    fn pad_level(&self, pin: usize) -> bool {
        if let Some(level) = self.drive(pin) {
            level
        } else if let Some(level) = self.external[pin] {
            level
        } else {
            self.pull(pin) == Pull::Up
        }
    }

    fn input_bit(&self, pin: usize) -> bool {
        has_pad(pin) && self.mscr[pin] & MSCR_IBE != 0 && self.pad_level(pin)
    }

    fn set_gpdo(&mut self, pin: usize, level: bool) {
        if has_pad(pin) {
            self.gpdo[pin] = level;
        }
    }

    fn word_lane(word: u32, offset: u64) -> u8 {
        (word >> ((offset % 4) * 8)) as u8
    }

    fn merge_lane(word: u32, offset: u64, value: u8) -> u32 {
        let shift = (offset % 4) * 8;
        (word & !(0xFF << shift)) | (u32::from(value) << shift)
    }

    fn read_byte(&self, offset: u64) -> u8 {
        match offset {
            MIDR1..=MIDR2_LAST | MIDR3..=MIDR4_LAST => 0,
            0..MISC_END => self.misc[offset as usize],
            MSCR0..MSCR_END => {
                let pin = ((offset - MSCR0) / 4) as usize;
                if has_pad(pin) {
                    Self::word_lane(self.mscr[pin], offset)
                } else {
                    0
                }
            }
            IMCR0..IMCR_END => Self::word_lane(self.imcr[((offset - IMCR0) / 4) as usize], offset),
            GPDO0..=GPDO_LAST => {
                let pin = ((offset - GPDO0) ^ 3) as usize;
                u8::from(has_pad(pin) && self.gpdo[pin])
            }
            GPDI0..=GPDI_LAST => u8::from(self.input_bit(((offset - GPDI0) ^ 3) as usize)),
            PGPDO0..=PGPDO_LAST | PGPDI0..=PGPDI_LAST => {
                let input = offset >= PGPDI0;
                let rel = offset - if input { PGPDI0 } else { PGPDO0 };
                let Some((port, lane)) = parallel_lane(rel) else {
                    return 0;
                };
                (0..8).fold(0u8, |acc, b| {
                    let pin = parallel_pin(port, lane * 8 + b);
                    let set = if input {
                        self.input_bit(pin)
                    } else {
                        has_pad(pin) && self.gpdo[pin]
                    };
                    acc | (u8::from(set) << b)
                })
            }
            // MPGPDO is write-only.
            MPGPDO0..MPGPDO_END => 0,
            _ => {
                crate::census_reg!("s32k3_siul2:S32k3Siul2", offset, "read");
                0
            }
        }
    }

    fn write_byte(&mut self, offset: u64, value: u8) {
        match offset {
            // MIDR1-4 are read-only.
            MIDR1..=MIDR2_LAST | MIDR3..=MIDR4_LAST => {}
            DISR0..=0x013 => self.misc[offset as usize] &= !value,
            0..MISC_END => self.misc[offset as usize] = value,
            MSCR0..MSCR_END => {
                let pin = ((offset - MSCR0) / 4) as usize;
                if has_pad(pin) {
                    self.mscr[pin] = Self::merge_lane(self.mscr[pin], offset, value);
                }
            }
            IMCR0..IMCR_END => {
                let i = ((offset - IMCR0) / 4) as usize;
                self.imcr[i] = Self::merge_lane(self.imcr[i], offset, value);
            }
            GPDO0..=GPDO_LAST => self.set_gpdo(((offset - GPDO0) ^ 3) as usize, value & 1 != 0),
            // GPDI/PGPDI are read-only.
            GPDI0..=GPDI_LAST | PGPDI0..=PGPDI_LAST => {}
            PGPDO0..=PGPDO_LAST => {
                if let Some((port, lane)) = parallel_lane(offset - PGPDO0) {
                    for b in 0..8 {
                        self.set_gpdo(parallel_pin(port, lane * 8 + b), value >> b & 1 != 0);
                    }
                }
            }
            // A byte store carries only its own lane of MASK/MPPDO.
            MPGPDO0..MPGPDO_END => {
                let word = u32::from(value) << ((offset % 4) * 8);
                self.write_mpgpdo(offset & !3, word);
            }
            _ => {
                crate::census_reg!("s32k3_siul2:S32k3Siul2", offset, "write");
            }
        }
    }

    fn write_mpgpdo(&mut self, offset: u64, value: u32) {
        let port = ((offset - MPGPDO0) / 4) as usize;
        for bit in 0..16 {
            if value >> (16 + bit) & 1 != 0 {
                self.set_gpdo(parallel_pin(port, bit), value >> bit & 1 != 0);
            }
        }
    }
}

impl Default for S32k3Siul2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for S32k3Siul2 {
    /// A pure register bank: no `tick()`, no IRQ, DMA or event from the walk
    /// (pad levels are recomputed on access).
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

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if (MPGPDO0..MPGPDO_END).contains(&offset) {
            self.write_mpgpdo(offset & !3, value);
        } else {
            for lane in 0..4 {
                self.write_byte(offset + lane, (value >> (lane * 8)) as u8);
            }
        }
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
        has_pad(pin).then(|| self.pad_level(pin))
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        has_pad(pin).then(|| self.drive(pin) == Some(true))
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        has_pad(pin).then(|| self.drive(pin).is_some())
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
            crate::SimulationError::NotImplemented(format!("S32k3Siul2 snapshot decode: {error}"))
        })?;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize S32k3Siul2")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        *self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "S32k3Siul2 runtime snapshot decode: {error}"
            ))
        })?;
        Ok(())
    }
}

impl PinPort for S32k3Siul2 {
    fn pin_count(&self) -> u8 {
        PINS as u8
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        let pin = pin as usize;
        if !has_pad(pin) {
            return None;
        }
        Some(match self.drive(pin) {
            Some(level) => PadDriver::drive(level),
            None => match self.pull(pin) {
                Pull::None => PadDriver::OFF,
                pull => PadDriver::released(pull),
            },
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
        if !has_pad(pin) {
            return None;
        }
        let before = self.pad_level(pin);
        self.external[pin] = ext.level();
        let after = self.pad_level(pin);
        Some(InputChange { before, after })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        has_pad(pin).then(|| self.pad_level(pin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PTF23 (port F = pins 160..191): the S32K388-EVB green LED.
    const PTF23: usize = 183;
    const GPIO_OUT: u32 = MSCR_OBE | MSCR_IBE;

    fn mscr(pin: usize) -> u64 {
        MSCR0 + 4 * pin as u64
    }

    fn gpdo(pin: usize) -> u64 {
        GPDO0 + (pin as u64 ^ 3)
    }

    fn gpdi(pin: usize) -> u64 {
        GPDI0 + (pin as u64 ^ 3)
    }

    /// PGPDO byte-swapped halfword address for parallel port `p`.
    fn pgpdo(p: u64) -> u64 {
        PGPDO0 + 2 * (p ^ 1)
    }

    fn pgpdi(p: u64) -> u64 {
        PGPDI0 + 2 * (p ^ 1)
    }

    #[test]
    fn mscr_stores_and_reserved_indices_read_zero() {
        let mut s = S32k3Siul2::new();
        s.write_u32(mscr(PTF23), GPIO_OUT | MSCR_PUE).unwrap();
        assert_eq!(s.read_u32(mscr(PTF23)).unwrap(), GPIO_OUT | MSCR_PUE);
        for pin in RESERVED_PINS {
            s.write_u32(mscr(pin), GPIO_OUT).unwrap();
            assert_eq!(s.read_u32(mscr(pin)).unwrap(), 0, "MSCR{pin} is reserved");
        }
        // The last MSCR (236) is addressable; IMCR is stored.
        s.write_u32(mscr(236), 0x5).unwrap();
        assert_eq!(s.read_u32(mscr(236)).unwrap(), 0x5);
        s.write_u32(IMCR0 + 4 * 469, 0x3).unwrap();
        assert_eq!(s.read_u32(IMCR0 + 4 * 469).unwrap(), 0x3);
    }

    #[test]
    fn gpdo_byte_drives_the_pad_only_as_a_gpio_output() {
        let mut s = S32k3Siul2::new();
        s.write(gpdo(PTF23), 1).unwrap();
        assert_eq!(s.read_gpio_is_output(PTF23 as u8), Some(false), "OBE=0");
        assert_eq!(s.read(gpdi(PTF23)).unwrap(), 0, "IBE=0 reads 0");
        s.write_u32(mscr(PTF23), GPIO_OUT).unwrap();
        assert_eq!(s.read_gpio_output(PTF23 as u8), Some(true));
        assert_eq!(s.read(gpdi(PTF23)).unwrap(), 1);
        // GPDO byte order within a word is big-endian: pin 183 = 0x13B4.
        assert_eq!(gpdo(PTF23), 0x13B4);
        assert_eq!(s.read_u32(0x13B4).unwrap(), 0x0000_0001);
        // A non-GPIO SSS releases the pad.
        s.write_u32(mscr(PTF23), GPIO_OUT | 0x2).unwrap();
        assert_eq!(s.read_gpio_is_output(PTF23 as u8), Some(false));
    }

    #[test]
    fn pgpdo_is_bit_reversed_and_halfword_swapped() {
        let mut s = S32k3Siul2::new();
        s.write_u32(mscr(PTF23), GPIO_OUT).unwrap();
        // PTF23 = port 11 (pins 176..191), k = 7 -> bit 15 - 7 = 8.
        s.write_u16(pgpdo(11), 1 << 8).unwrap();
        assert_eq!(pgpdo(11), 0x1714);
        assert_eq!(s.read(gpdo(PTF23)).unwrap(), 1);
        assert_eq!(s.read_u16(pgpdo(11)).unwrap(), 1 << 8);
        assert_eq!(s.read_u16(pgpdi(11)).unwrap(), 1 << 8);
        // Pin 0 (PTA0) is bit 15 of PGPDO0, which lives at 0x1702.
        s.write_u16(0x1702, 0x8000).unwrap();
        assert!(s.gpdo[0]);
        // The reserved halfword 0x171C reads 0 and ignores writes.
        s.write_u16(0x171C, 0xFFFF).unwrap();
        assert_eq!(s.read_u16(0x171C).unwrap(), 0);
        assert_eq!(s.read_u16(pgpdo(14)).unwrap(), 0);
    }

    #[test]
    fn mpgpdo_writes_only_the_masked_pins() {
        let mut s = S32k3Siul2::new();
        // Port 11: set PTF17 (k=1, bit 14) and PTF23 (k=7, bit 8).
        s.write_u16(pgpdo(11), (1 << 14) | (1 << 8)).unwrap();
        // Mask only bit 8, data 0: clears PTF23, leaves PTF17.
        s.write_u32(MPGPDO0 + 4 * 11, 1 << (16 + 8)).unwrap();
        assert!(s.gpdo[177]);
        assert!(!s.gpdo[PTF23]);
        assert_eq!(s.read_u32(MPGPDO0 + 4 * 11).unwrap(), 0, "write-only");
    }

    #[test]
    fn pull_and_outside_world_set_the_input_level() {
        let mut s = S32k3Siul2::new();
        let pta1 = 1;
        s.write_u32(mscr(pta1), MSCR_IBE).unwrap();
        assert_eq!(s.read(gpdi(pta1)).unwrap(), 0, "unpulled reads 0");
        s.write_u32(mscr(pta1), MSCR_IBE | MSCR_PUE | MSCR_PUS)
            .unwrap();
        assert_eq!(s.read(gpdi(pta1)).unwrap(), 1, "pull-up");
        s.set_external(pta1 as u8, External::Level(false));
        assert_eq!(s.read(gpdi(pta1)).unwrap(), 0, "held low");
        // PGPDI0 (0x1742) bit 15 - 1 = 14 is PTA1.
        s.set_external(pta1 as u8, External::Level(true));
        assert_eq!(s.read_u16(pgpdi(0)).unwrap(), 1 << 14);
    }

    #[test]
    fn disr0_is_write_one_to_clear_and_midr_is_read_only() {
        let mut s = S32k3Siul2::new();
        s.misc[DISR0 as usize] = 0xFF;
        s.write_u32(DISR0, 0x0F).unwrap();
        assert_eq!(s.read_u32(DISR0).unwrap(), 0xF0);
        s.write_u32(MIDR1, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(MIDR1).unwrap(), 0);
        s.write_u32(0x18, 0x1234).unwrap();
        assert_eq!(s.read_u32(0x18).unwrap(), 0x1234, "DIRER0 stored");
    }

    #[test]
    fn node_reset_keeps_the_outside_world() {
        let mut s = S32k3Siul2::new();
        s.write_u32(mscr(5), MSCR_IBE).unwrap();
        s.set_external(5, External::Level(true));
        s.on_node_reset();
        assert_eq!(s.read_u32(mscr(5)).unwrap(), 0);
        assert_eq!(s.input(5), Some(true));
    }
}
