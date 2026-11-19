// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bounded SiFive FE310 GPIO model.
//!
//! This implements the software-controlled pad path used by HiFive1 Rev B
//! firmware: INPUT_VAL, INPUT_EN, OUTPUT_EN, OUTPUT_VAL, PULLUP_EN and
//! OUTPUT_XOR, and the four per-pin interrupt pairs (RISE/FALL/HIGH/LOW
//! `_ie` / `_ip`). The IO-function mux and drive-strength registers are
//! intentionally outside this profile.
//!
//! Interrupts follow the sifive-blocks GPIO RTL (FE310-G002 Manual v1p1
//! Ch. 17): the `_ip` bits latch whatever `_ie` holds — RISE / FALL on an
//! INPUT_VAL edge, HIGH / LOW while INPUT_VAL holds that level — and clear on
//! a write of 1, re-latching at once if the level still holds. Pin `n`'s
//! interrupt is `ip & ie` over the four pairs. It drives the PLIC source
//! `irq + n` (the entry's `irq:` is pin 0's source; FE310 `[0-31] ->
//! plic@[8-39]` in Renode `sifive-fe310.repl`), re-raised on every tick while
//! it holds, so a source still asserted after its complete pends again.

use crate::peripherals::gpio::GpioPortOffsets;
use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{Peripheral, SimResult};

const INPUT_VAL: u64 = 0x00;
const INPUT_EN: u64 = 0x04;
const OUTPUT_EN: u64 = 0x08;
const OUTPUT_VAL: u64 = 0x0C;
const PULLUP_EN: u64 = 0x10;
const RISE_IE: u64 = 0x18;
const RISE_IP: u64 = 0x1C;
const FALL_IE: u64 = 0x20;
const FALL_IP: u64 = 0x24;
const HIGH_IE: u64 = 0x28;
const HIGH_IP: u64 = 0x2C;
const LOW_IE: u64 = 0x30;
const LOW_IP: u64 = 0x34;
const OUTPUT_XOR: u64 = 0x40;

/// FE310 GPIO register bank with the per-pad input/output behavior needed by
/// the imported HiFive1 Rev B profile.
#[derive(Debug, Default)]
pub struct SifiveGpio {
    input_en: u32,
    output_en: u32,
    output_val: u32,
    pullup_en: u32,
    output_xor: u32,
    external: u32,
    external_levels: u32,
    /// `[rise, fall, high, low]` interrupt enables and pending latches.
    ie: [u32; 4],
    ip: [u32; 4],
    /// INPUT_VAL at the last sample, for edge detection.
    sampled: u32,
    /// PLIC source of pin 0 (the entry's `irq:`); `None` leaves the pin
    /// interrupts unwired.
    irq_base: Option<u32>,
}

impl SifiveGpio {
    pub fn new() -> Self {
        let mut gpio = Self::default();
        gpio.sample();
        gpio
    }

    /// Latch the interrupt-pending bits from the current INPUT_VAL. Called
    /// after every state change that can move it.
    fn sample(&mut self) {
        let value = self.input_value();
        self.ip[0] |= value & !self.sampled;
        self.ip[1] |= !value & self.sampled;
        self.ip[2] |= value;
        self.ip[3] |= !value;
        self.sampled = value;
    }

    /// Pins whose interrupt output is asserted.
    fn irq_pins(&self) -> u32 {
        (0..4).fold(0, |acc, i| acc | (self.ip[i] & self.ie[i]))
    }

    fn pin_mask(pin: u8) -> u32 {
        1u32 << pin
    }

    fn output_level(&self, pin: u8) -> bool {
        ((self.output_val ^ self.output_xor) & Self::pin_mask(pin)) != 0
    }

    fn pad_level(&self, pin: u8) -> bool {
        let bit = Self::pin_mask(pin);
        if self.output_en & bit != 0 {
            self.output_level(pin)
        } else if self.external & bit != 0 {
            self.external_levels & bit != 0
        } else {
            self.pullup_en & bit != 0
        }
    }

    fn input_value(&self) -> u32 {
        let mut value = 0;
        let mut enabled = self.input_en;
        while enabled != 0 {
            let pin = enabled.trailing_zeros() as u8;
            enabled &= enabled - 1;
            if self.pad_level(pin) {
                value |= Self::pin_mask(pin);
            }
        }
        value
    }

    fn read_register(&self, offset: u64) -> u32 {
        match offset {
            INPUT_VAL => self.input_value(),
            INPUT_EN => self.input_en,
            OUTPUT_EN => self.output_en,
            OUTPUT_VAL => self.output_val,
            PULLUP_EN => self.pullup_en,
            RISE_IE => self.ie[0],
            RISE_IP => self.ip[0],
            FALL_IE => self.ie[1],
            FALL_IP => self.ip[1],
            HIGH_IE => self.ie[2],
            HIGH_IP => self.ip[2],
            LOW_IE => self.ie[3],
            LOW_IP => self.ip[3],
            OUTPUT_XOR => self.output_xor,
            // This profile does not claim the FE310 IO-function or drive
            // registers. Unimplemented/reserved words read as zero.
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u32) {
        match offset {
            // INPUT_VAL is a sampled pad value and is read-only.
            INPUT_EN => self.input_en = value,
            OUTPUT_EN => self.output_en = value,
            OUTPUT_VAL => self.output_val = value,
            PULLUP_EN => self.pullup_en = value,
            RISE_IE => self.ie[0] = value,
            RISE_IP => self.ip[0] &= !value,
            FALL_IE => self.ie[1] = value,
            FALL_IP => self.ip[1] &= !value,
            HIGH_IE => self.ie[2] = value,
            HIGH_IP => self.ip[2] &= !value,
            LOW_IE => self.ie[3] = value,
            LOW_IP => self.ip[3] &= !value,
            OUTPUT_XOR => self.output_xor = value,
            _ => {}
        }
        self.sample();
    }

    fn read_input_pin(&self, pin: u8) -> Option<bool> {
        (pin < 32 && self.input_en & Self::pin_mask(pin) != 0)
            .then(|| self.input_value() & Self::pin_mask(pin) != 0)
    }

    fn merge_byte(&mut self, offset: u64, value: u8) {
        let word_offset = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        let mask = 0xFFu32 << shift;
        // The `_ip` words are write-1-to-clear: a byte store clears only its
        // own byte's bits.
        let old = if matches!(word_offset, RISE_IP | FALL_IP | HIGH_IP | LOW_IP) {
            0
        } else {
            self.read_register(word_offset)
        };
        self.write_register(word_offset, (old & !mask) | (u32::from(value) << shift));
    }
}

impl Peripheral for SifiveGpio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let value = self.read_register(offset & !3);
        let shift = ((offset & 3) * 8) as u32;
        Ok(((value >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.merge_byte(offset, value);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_register(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_register(offset & !3, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let value = self.read_register(offset & !3);
        let shift = ((offset & 3) * 8) as u32;
        Some(((value >> shift) & 0xFF) as u8)
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        let (Some(base), pins) = (self.irq_base, self.irq_pins()) else {
            return crate::PeripheralTickResult::default();
        };
        if pins == 0 {
            return crate::PeripheralTickResult::default();
        }
        let mut irqs = Vec::with_capacity(pins.count_ones() as usize);
        let mut rest = pins;
        while rest != 0 {
            irqs.push(base + rest.trailing_zeros());
            rest &= rest - 1;
        }
        crate::PeripheralTickResult {
            explicit_irqs: Some(irqs),
            ..Default::default()
        }
    }

    fn attach_irq_line(&mut self, irq: Option<u32>) {
        self.irq_base = irq;
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        self.read_input_pin(pin)
    }

    fn read_gpio_input_word(&self) -> u32 {
        self.input_value()
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (pin < 32).then(|| self.output_val & Self::pin_mask(pin) != 0)
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        (pin < 32).then(|| self.output_en & Self::pin_mask(pin) != 0)
    }

    fn gpio_port_offsets(&self) -> Option<GpioPortOffsets> {
        Some(GpioPortOffsets {
            output: OUTPUT_VAL,
            input: INPUT_VAL,
        })
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }
}

impl PinPort for SifiveGpio {
    fn pin_count(&self) -> u8 {
        32
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if pin >= 32 {
            return None;
        }
        let bit = Self::pin_mask(pin);
        Some(if self.output_en & bit != 0 {
            PadDriver::drive(self.output_level(pin))
        } else if self.pullup_en & bit != 0 {
            PadDriver::released(crate::pins::Pull::Up)
        } else {
            PadDriver::OFF
        })
    }

    fn external(&self, pin: u8) -> External {
        if pin >= 32 || self.external & Self::pin_mask(pin) == 0 {
            External::Released
        } else {
            External::Level(self.external_levels & Self::pin_mask(pin) != 0)
        }
    }

    fn set_external(&mut self, pin: u8, external: External) -> Option<InputChange> {
        if pin >= 32 {
            return None;
        }
        let bit = Self::pin_mask(pin);
        let was_enabled = self.input_en & bit != 0;
        let before = self.read_input_pin(pin).unwrap_or(false);
        match external {
            External::Released => {
                self.external &= !bit;
                self.external_levels &= !bit;
            }
            External::Level(level) => {
                self.external |= bit;
                if level {
                    self.external_levels |= bit;
                } else {
                    self.external_levels &= !bit;
                }
            }
        }
        self.sample();
        was_enabled.then(|| InputChange {
            before,
            after: self.read_input_pin(pin).unwrap_or(before),
        })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        self.read_input_pin(pin)
    }
}

#[cfg(test)]
mod tests {
    use super::SifiveGpio;
    use crate::pins::{External, PadDriver, PinPort};
    use crate::Peripheral;

    const LED: u32 = 1 << 19;

    #[test]
    fn software_output_and_output_xor_drive_the_pad() {
        let mut gpio = SifiveGpio::new();
        gpio.write_u32(0x08, LED).unwrap();
        gpio.write_u32(0x0C, LED).unwrap();
        assert_eq!(gpio.read_u32(0x0C).unwrap(), LED);
        assert_eq!(gpio.read_gpio_is_output(19), Some(true));
        assert_eq!(gpio.driver(19), Some(PadDriver::drive(true)));

        gpio.write_u32(0x40, LED).unwrap();
        assert_eq!(gpio.read_u32(0x0C).unwrap(), LED);
        assert_eq!(gpio.driver(19), Some(PadDriver::drive(false)));
    }

    #[test]
    fn input_enable_gates_the_sampled_external_level() {
        let mut gpio = SifiveGpio::new();
        assert_eq!(gpio.read_u32(0).unwrap(), 0);
        assert!(gpio.set_external(7, External::Level(true)).is_none());
        gpio.write_u32(0x04, 1 << 7).unwrap();
        assert_eq!(gpio.read_u32(0).unwrap(), 1 << 7);
        assert_eq!(gpio.read_gpio_input(7), Some(true));
        assert!(!gpio.set_external(7, External::Level(false)).unwrap().after);
        assert_eq!(gpio.read_u32(0).unwrap(), 0);
    }

    #[test]
    fn edge_and_level_latches_drive_plic_source_per_pin() {
        let mut gpio = SifiveGpio::new();
        gpio.attach_irq_line(Some(8));
        // Input disabled: INPUT_VAL reads 0, so only LOW latches.
        assert_eq!(gpio.read_u32(0x34).unwrap(), u32::MAX);
        assert_eq!(gpio.read_u32(0x1C).unwrap(), 0);
        gpio.write_u32(0x34, u32::MAX).unwrap();
        assert_eq!(gpio.read_u32(0x34).unwrap(), u32::MAX, "LOW holds");

        const PIN: u32 = 1 << 2;
        gpio.write_u32(0x04, PIN).unwrap();
        gpio.write_u32(0x08, PIN).unwrap();
        gpio.write_u32(0x18, PIN).unwrap();
        assert!(gpio.tick().explicit_irqs.is_none());

        gpio.write_u32(0x0C, PIN).unwrap();
        assert_eq!(gpio.read_u32(0x1C).unwrap(), PIN);
        assert_eq!(gpio.read_u32(0x24).unwrap(), 0);
        assert_eq!(gpio.read_u32(0x2C).unwrap() & PIN, PIN);
        assert_eq!(gpio.tick().explicit_irqs, Some(vec![10]));

        // RISE clears on a write of 1; HIGH re-latches while the pin is high.
        gpio.write_u32(0x1C, PIN).unwrap();
        gpio.write_u32(0x2C, PIN).unwrap();
        assert_eq!(gpio.read_u32(0x1C).unwrap(), 0);
        assert_eq!(gpio.read_u32(0x2C).unwrap() & PIN, PIN);
        assert!(gpio.tick().explicit_irqs.is_none());

        // FALL on the high-to-low edge, routed to the same source.
        gpio.write_u32(0x20, PIN).unwrap();
        gpio.write_u32(0x0C, 0).unwrap();
        assert_eq!(gpio.read_u32(0x24).unwrap(), PIN);
        assert_eq!(gpio.tick().explicit_irqs, Some(vec![10]));
    }

    #[test]
    fn external_edge_on_pin_31_maps_to_source_39_and_byte_clear_is_w1c() {
        let mut gpio = SifiveGpio::new();
        gpio.attach_irq_line(Some(8));
        const PIN: u32 = 1 << 31;
        gpio.write_u32(0x04, PIN | 1).unwrap();
        gpio.write_u32(0x18, PIN | 1).unwrap();
        gpio.set_external(31, External::Level(true));
        gpio.set_external(0, External::Level(true));
        assert_eq!(gpio.read_u32(0x1C).unwrap(), PIN | 1);
        assert_eq!(gpio.tick().explicit_irqs, Some(vec![8, 39]));
        // A byte store to the top byte clears pin 31 only.
        gpio.write(0x1F, 0x80).unwrap();
        assert_eq!(gpio.read_u32(0x1C).unwrap(), 1);
    }

    #[test]
    fn unwired_gpio_raises_nothing() {
        let mut gpio = SifiveGpio::new();
        gpio.write_u32(0x30, u32::MAX).unwrap();
        assert!(gpio.tick().explicit_irqs.is_none());
    }
}
