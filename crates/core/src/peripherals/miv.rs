// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Microchip Mi-V reference-design peripherals: `CoreGPIO` and `CoreTimer`.
//!
//! These are the two blocks that kept the `miv` / `miv-board` staged
//! descriptors on the stub inventory; the Mi-V `CoreUART` was already mapped
//! to the engine's CoreUART layout. Register maps below are the blocks'
//! published layouts (the same ones Microchip's CoreGPIO/CoreTimer handbooks
//! describe), transcribed as facts.
//!
//! ## CoreGPIO (0xA4 bytes)
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x00 + 4·n | per-pin config | R/W | bit0 OUTREG (pin is an output), bit1 INREG (input), bit2 OUTBUFF (read-back bit), bit3 INTENABLE, bits5:7 INTTYPE (0 active-high, 1 active-low, 2 rising, 3 falling, 4 both), bit4 reserved |
//! | 0x80 | InterruptClear | W | clears the per-pin interrupt latches |
//! | 0x90 | InputRegister | R | levels on pins configured as inputs |
//! | 0xA0 | OutputRegister | R/W | levels driven on pins configured as outputs |
//!
//! The pin-side behaviour is exposed through the engine's [`PinPort`]: a pin
//! configured as an output drives the level in the OutputRegister; a pin
//! configured as an input reports the level the outside world presents.
//! Interrupt *generation* is not modelled yet (the INTENABLE/INTTYPE fields
//! are readable/writable register state; the interrupt-clear register is
//! absorbed) — the Mi-V promotion that consumes this model checks the
//! pending path when a board uses interrupts.
//!
//! ## CoreTimer (0x1C bytes)
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x00 | Load | R/W | the limit the counter counts up to; writing it also enables the timer in one-shot mode |
//! | 0x04 | Value | R | current counter |
//! | 0x08 | Control | R/W | bit0 enable, bit1 interrupt enable, bit2 mode (0 periodic, 1 one-shot) |
//! | 0x0C | ClockPrescale | R/W | bits 3:0 select a divider of `2 << n` clock cycles per count |
//! | 0x10 | InterruptClear | W | clears the raw interrupt latch |
//! | 0x14 | RawInterruptStatus | R | bit0: the zero/limit event occurred |
//! | 0x18 | MaskedInterruptStatus | R | bit0: raw status AND interrupt enable |
//!
//! The counter advances from the bus's elapsed cycles (closed-form through
//! the prescaler), latches the raw status at the limit, and either wraps
//! (periodic) or stops (one-shot). The IRQ level is `raw && enable` through
//! the normal `PeripheralTickResult` path.

use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{PeripheralTickResult, SimResult};

// ── CoreGPIO ────────────────────────────────────────────────────────────────

const GPIO_PINS: usize = 32;
const GPIO_CFG_BASE: u64 = 0x00;
const GPIO_IRQ_CLEAR: u64 = 0x80;
const GPIO_INPUT: u64 = 0x90;
const GPIO_OUTPUT: u64 = 0xA0;

/// Per-pin writable bits: OUTREG(0), INREG(1), OUTBUFF(2), INTENABLE(3),
/// INTTYPE(7:5). Bit 4 is reserved.
const CFG_WRITABLE_MASK: u32 = 0x0000_00EF;

fn bit(state: u32, n: usize) -> bool {
    state & (1 << n) != 0
}

/// Microchip Mi-V `CoreGPIO`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MivCoreGpio {
    /// Per-pin configuration registers (OUTREG / INREG / OUTBUFF / INTENABLE /
    /// INTTYPE).
    cfg: [u32; GPIO_PINS],
    /// OutputRegister: driven levels for output pins.
    output: u32,
    /// External levels presented to the pins, regardless of direction.
    /// `None` = released (nothing connected). Stored as `Option<bool>` so the
    /// model derives serde; `External` is the pin-layer spelling.
    external: [Option<bool>; GPIO_PINS],
}

impl MivCoreGpio {
    pub fn new() -> Self {
        Self {
            cfg: [0; GPIO_PINS],
            output: 0,
            external: [None; GPIO_PINS],
        }
    }

    fn out_mask(&self) -> u32 {
        self.cfg
            .iter()
            .enumerate()
            .fold(0u32, |m, (i, c)| if bit(*c, 0) { m | (1 << i) } else { m })
    }

    fn in_mask(&self) -> u32 {
        self.cfg
            .iter()
            .enumerate()
            .fold(0u32, |m, (i, c)| if bit(*c, 1) { m | (1 << i) } else { m })
    }

    /// The level the outside world presents on `pin`.
    fn external_level(&self, pin: usize) -> bool {
        self.external[pin].unwrap_or(false)
    }

    /// Firmware-visible input level of `pin` (InputRegister bit).
    fn input_level(&self, pin: usize) -> bool {
        self.external_level(pin)
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            GPIO_IRQ_CLEAR => 0, // write-only
            GPIO_INPUT => {
                let mut v = 0u32;
                for pin in 0..GPIO_PINS {
                    if bit(self.in_mask(), pin) && self.input_level(pin) {
                        v |= 1 << pin;
                    }
                }
                v
            }
            GPIO_OUTPUT => self.output & self.out_mask(),
            _ if (GPIO_CFG_BASE..GPIO_IRQ_CLEAR).contains(&offset) && offset % 4 == 0 => {
                let index = (offset / 4) as usize;
                if index < GPIO_PINS {
                    self.cfg[index]
                } else {
                    crate::census_reg!("miv:CoreGPIO", offset, "read");
                    0
                }
            }
            _ => {
                crate::census_reg!("miv:CoreGPIO", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // InterruptClear: no latched interrupts are modelled yet; the write
            // is absorbed so firmware's clear-before-enable sequence is silent.
            GPIO_IRQ_CLEAR => {}
            GPIO_INPUT => {} // read-only
            GPIO_OUTPUT => {
                let mask = self.out_mask();
                self.output = (self.output & !mask) | (value & mask);
            }
            _ if (GPIO_CFG_BASE..GPIO_IRQ_CLEAR).contains(&offset) && offset % 4 == 0 => {
                let index = (offset / 4) as usize;
                if index < GPIO_PINS {
                    self.cfg[index] = value & CFG_WRITABLE_MASK;
                } else {
                    crate::census_reg!("miv:CoreGPIO", offset, "write");
                }
            }
            _ => {
                crate::census_reg!("miv:CoreGPIO", offset, "write");
            }
        }
    }
}

impl Default for MivCoreGpio {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for MivCoreGpio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
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
}

impl PinPort for MivCoreGpio {
    fn pin_count(&self) -> u8 {
        GPIO_PINS as u8
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        let pin = pin as usize;
        if pin >= GPIO_PINS {
            return None;
        }
        Some(if bit(self.out_mask(), pin) {
            PadDriver::drive(bit(self.output, pin))
        } else {
            PadDriver::OFF
        })
    }

    fn external(&self, pin: u8) -> External {
        let pin = pin as usize;
        if pin >= GPIO_PINS {
            return External::Released;
        }
        match self.external[pin] {
            None => External::Released,
            Some(level) => External::Level(level),
        }
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        let pin = pin as usize;
        if pin >= GPIO_PINS {
            return None;
        }
        let before = self.input_level(pin);
        self.external[pin] = ext.level();
        let after = self.input_level(pin);
        Some(InputChange { before, after })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        if pin >= GPIO_PINS || !bit(self.in_mask(), pin) {
            return None;
        }
        Some(self.input_level(pin))
    }
}

// ── CoreTimer ───────────────────────────────────────────────────────────────

const TIMER_LOAD: u64 = 0x00;
const TIMER_VALUE: u64 = 0x04;
const TIMER_CONTROL: u64 = 0x08;
const TIMER_PRESCALE: u64 = 0x0C;
const TIMER_IRQ_CLEAR: u64 = 0x10;
const TIMER_RAW_STATUS: u64 = 0x14;
const TIMER_MASKED_STATUS: u64 = 0x18;

/// Microchip Mi-V `CoreTimer`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MivCoreTimer {
    load: u32,
    value: u32,
    enabled: bool,
    irq_enabled: bool,
    one_shot: bool,
    prescale: u8,
    raw_irq: bool,
    /// Clock cycles accumulated toward the next prescaled count.
    phase: u64,
}

impl MivCoreTimer {
    pub fn new() -> Self {
        Self {
            load: 0,
            value: 0,
            enabled: false,
            irq_enabled: false,
            one_shot: false,
            prescale: 0,
            raw_irq: false,
            phase: 0,
        }
    }

    /// Clock divider: `2 << prescale` cycles per count (2..=1024).
    fn divider(&self) -> u64 {
        2u64 << u32::from(self.prescale.min(9))
    }

    /// Advance the counter by `cycles` clock cycles, closed-form through the
    /// prescaler.
    fn advance(&mut self, cycles: u64) {
        if !self.enabled || self.load == 0 {
            return;
        }
        let divider = self.divider();
        let total = self.phase + cycles;
        let counts = total / divider;
        self.phase = total % divider;
        if counts == 0 {
            return;
        }
        let limit = u64::from(self.load);
        let mut value = u64::from(self.value);
        let mut remaining = counts;
        while remaining > 0 {
            if value >= limit {
                // The limit event: latch the raw status and either wrap or
                // stop (one-shot).
                self.raw_irq = true;
                if self.one_shot {
                    self.enabled = false;
                    self.value = 0;
                    self.phase = 0;
                    return;
                }
                value = 0;
            }
            let step = remaining.min(limit - value);
            value += step;
            remaining -= step;
        }
        if value >= limit {
            self.raw_irq = true;
            if self.one_shot {
                self.enabled = false;
                self.value = 0;
                self.phase = 0;
                return;
            }
            value = 0;
        }
        self.value = value as u32;
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            TIMER_LOAD => self.load,
            TIMER_VALUE => self.value,
            TIMER_CONTROL => {
                (self.enabled as u32)
                    | ((self.irq_enabled as u32) << 1)
                    | ((self.one_shot as u32) << 2)
            }
            TIMER_PRESCALE => u32::from(self.prescale),
            TIMER_IRQ_CLEAR => 0, // write-only
            TIMER_RAW_STATUS => self.raw_irq as u32,
            TIMER_MASKED_STATUS => (self.raw_irq && self.irq_enabled) as u32,
            _ => {
                crate::census_reg!("miv:CoreTimer", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            TIMER_LOAD => {
                self.load = value;
                // The block enables itself when a one-shot duration is loaded.
                if self.one_shot {
                    self.enabled = true;
                }
            }
            TIMER_VALUE => {} // read-only
            TIMER_CONTROL => {
                self.enabled = value & 1 != 0;
                self.irq_enabled = value & 2 != 0;
                self.one_shot = value & 4 != 0;
            }
            TIMER_PRESCALE => self.prescale = (value & 0xF) as u8,
            TIMER_IRQ_CLEAR => self.raw_irq = false,
            TIMER_RAW_STATUS | TIMER_MASKED_STATUS => {} // read-only
            _ => {
                crate::census_reg!("miv:CoreTimer", offset, "write");
            }
        }
    }
}

impl Default for MivCoreTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for MivCoreTimer {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }

    fn needs_legacy_walk(&self) -> bool {
        true
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.advance(cycles);
        PeripheralTickResult {
            irq: self.raw_irq && self.irq_enabled,
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn gpio_direction_and_output_register_drive_the_pads() {
        let mut gpio = MivCoreGpio::new();
        // Pin 0 as output, pin 1 as input.
        gpio.write_u32(0x00, 1).unwrap();
        gpio.write_u32(0x04, 2).unwrap();
        gpio.write_u32(GPIO_OUTPUT, 0b11).unwrap();
        // Only the output pin's level is stored and driven.
        assert_eq!(gpio.read_u32(GPIO_OUTPUT).unwrap(), 0b01);
        assert_eq!(
            gpio.driver(0).unwrap().out,
            crate::pins::Out::High,
            "output pin drives the written level"
        );
        assert_eq!(
            gpio.driver(1).unwrap().out,
            crate::pins::Out::Off,
            "input pin does not drive"
        );
    }

    #[test]
    fn gpio_input_register_reports_external_levels_on_input_pins_only() {
        let mut gpio = MivCoreGpio::new();
        gpio.write_u32(0x00, 1).unwrap(); // pin0 output
        gpio.write_u32(0x04, 2).unwrap(); // pin1 input
        gpio.set_external(0, External::Level(true));
        gpio.set_external(1, External::Level(true));
        assert_eq!(
            gpio.read_u32(GPIO_INPUT).unwrap(),
            0b10,
            "only the input pin appears in the input register"
        );
        assert_eq!(gpio.input(1), Some(true));
        assert_eq!(gpio.input(0), None, "an output pin is not an input");
    }

    #[test]
    fn gpio_config_writable_bits_match_the_block() {
        let mut gpio = MivCoreGpio::new();
        gpio.write_u32(0x00, 0xFFFF_FFFF).unwrap();
        assert_eq!(
            gpio.read_u32(0x00).unwrap(),
            CFG_WRITABLE_MASK,
            "reserved bit 4 and the upper bits do not store"
        );
    }

    #[test]
    fn timer_counts_up_and_latches_the_raw_status() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 0).unwrap(); // divider 2
        timer.write_u32(TIMER_CONTROL, 0b011).unwrap(); // enable + irq enable
        timer.write_u32(TIMER_LOAD, 10).unwrap();
        let _ = timer.tick_elapsed(18); // 9 counts, one short of 10
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 9);
        assert_eq!(timer.read_u32(TIMER_RAW_STATUS).unwrap(), 0);
        let r = timer.tick_elapsed(2); // 10th count reaches the limit
        assert_eq!(timer.read_u32(TIMER_RAW_STATUS).unwrap(), 1);
        assert_eq!(timer.read_u32(TIMER_MASKED_STATUS).unwrap(), 1);
        assert!(r.irq, "raw status + interrupt enable raises the IRQ");
        timer.write_u32(TIMER_IRQ_CLEAR, 1).unwrap();
        assert_eq!(timer.read_u32(TIMER_RAW_STATUS).unwrap(), 0);
        assert!(!timer.tick_elapsed(2).irq, "cleared status drops the IRQ");
    }

    #[test]
    fn timer_prescaler_divides_the_clock() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 2).unwrap(); // divider 8
        timer.write_u32(TIMER_CONTROL, 0b001).unwrap();
        timer.write_u32(TIMER_LOAD, 100).unwrap();
        let _ = timer.tick_elapsed(7);
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 0, "7 cycles < 8");
        let _ = timer.tick_elapsed(1);
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 1);
    }

    #[test]
    fn timer_one_shot_stops_at_the_limit_and_periodic_wraps() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 0).unwrap(); // divider 2
        timer.write_u32(TIMER_CONTROL, 0b101).unwrap(); // one-shot + enable
        timer.write_u32(TIMER_LOAD, 3).unwrap();
        let _ = timer.tick_elapsed(8); // 4 counts: reaches the limit and stops
        assert_eq!(timer.read_u32(TIMER_CONTROL).unwrap() & 1, 0, "stopped");
        assert_eq!(timer.read_u32(TIMER_RAW_STATUS).unwrap(), 1);

        let mut periodic = MivCoreTimer::new();
        periodic.write_u32(TIMER_PRESCALE, 0).unwrap();
        periodic.write_u32(TIMER_CONTROL, 0b001).unwrap();
        periodic.write_u32(TIMER_LOAD, 3).unwrap();
        let _ = periodic.tick_elapsed(8); // 4 counts: limit at 3, wraps, one past it
        assert_eq!(periodic.read_u32(TIMER_CONTROL).unwrap() & 1, 1, "running");
        assert_eq!(periodic.read_u32(TIMER_VALUE).unwrap(), 1);
        assert_eq!(periodic.read_u32(TIMER_RAW_STATUS).unwrap(), 1);
    }
}
