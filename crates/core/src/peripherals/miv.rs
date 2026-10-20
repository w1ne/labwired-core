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
//! | 0x00 | Load | R/W | the value the counter is loaded from; writing it loads the counter **immediately** (while running, the count restarts at the new value) |
//! | 0x04 | Value | R | current counter (reset value all-ones on silicon) |
//! | 0x08 | Control | R/W | bit0 enable, bit1 interrupt enable, bit2 mode (0 continuous, 1 one-shot) |
//! | 0x0C | ClockPrescale | R/W | bits 3:0 select a divider of `2 << n` PCLK cycles per count (`0000` = /2 … `1001` = /1024) |
//! | 0x10 | InterruptClear | W | clears the raw interrupt latch |
//! | 0x14 | RawInterruptStatus | R | bit0: the counter reached zero |
//! | 0x18 | MaskedInterruptStatus | R | bit0: raw status AND interrupt enable |
//!
//! The counter is a **decrementing** counter (CoreTimer datasheet, v3.1): it
//! loads from `Load` and, while enabled, counts down to zero at the prescaled
//! rate. Continuous mode reloads from `Load` at zero and keeps counting;
//! one-shot mode halts at zero until the mode is deselected or a new load is
//! written. Writing `Load` loads the counter immediately but does **not**
//! enable the timer — firmware still has to set Timer Enable. The raw
//! interrupt latches at zero and is cleared only by `InterruptClear`.

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

    /// GPIO capability: the **pad** level for `pin`.
    ///
    /// The InputRegister itself masks bits whose INREG (CONFIG_X[1]) is clear
    /// (handbook: forced to 0 via a MUX), and `read_reg(GPIO_INPUT)` keeps
    /// that behaviour. This capability is what board integration asks — "did
    /// the level a button drives land on the pin?" — so a pin with an
    /// externally presented level reports it even before firmware enables
    /// INREG; otherwise it falls back to the masked register bit.
    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        if pin >= GPIO_PINS {
            return None;
        }
        match self.external[pin] {
            Some(level) => Some(level),
            None => Some(bit(self.in_mask(), pin) && self.input_level(pin)),
        }
    }

    fn read_gpio_input_word(&self) -> u32 {
        let mut word = 0u32;
        for pin in 0..GPIO_PINS {
            if self.read_gpio_input(pin as u8) == Some(true) {
                word |= 1 << pin;
            }
        }
        word
    }

    /// Firmware-visible output latch: the OutputRegister masks non-output
    /// pins to 0 while keeping the written value internally.
    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        if pin >= GPIO_PINS {
            return None;
        }
        Some(bit(self.out_mask(), pin) && bit(self.output, pin))
    }

    /// Is the pad actively driving? That is OUTBUFF (the output enable), not
    /// OUTREG: with OUTREG clear and the buffer on, the pad drives 0; with the
    /// buffer off it is released even though the output latch may be set. This
    /// matches `driver()` and the logic-capture direction it feeds.
    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        if pin >= GPIO_PINS {
            return None;
        }
        Some(bit(self.cfg[pin], 2))
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Restore the JSON snapshot the determinism/replay tools carry.
    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("MivCoreGpio snapshot decode: {error}"))
        })?;
        *self = restored;
        Ok(())
    }

    /// Binary mid-flight snapshot: configuration, directions, output levels
    /// and the external levels the pins see.
    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize MivCoreGpio")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "MivCoreGpio runtime snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
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
        // OUTBUFF (bit 2) is the output enable: while it is clear the pad is
        // released (GPIO_OE = 0, high impedance). While it is set the pad
        // drives the output data — and the data is forced to zero when OUTREG
        // (bit 0) is clear, with the written value retained in the register
        // (handbook, Output Registers section).
        Some(if bit(self.cfg[pin], 2) {
            PadDriver::drive(bit(self.out_mask(), pin) && bit(self.output, pin))
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
            // The datasheet's TimerValue reset value is all-ones (32-bit);
            // the counter itself is disabled and the interrupt is clear.
            value: u32::MAX,
            enabled: false,
            irq_enabled: false,
            one_shot: false,
            prescale: 0,
            raw_irq: false,
            phase: 0,
        }
    }

    /// Clock divider: `2 << prescale` PCLK cycles per count (2..=1024).
    fn divider(&self) -> u64 {
        2u64 << u32::from(self.prescale.min(9))
    }

    /// Advance the decrementing counter by `cycles` PCLK cycles, closed-form
    /// through the prescaler and the continuous-mode reload.
    fn advance(&mut self, cycles: u64) {
        if cycles == 0 {
            return;
        }
        let total = self.phase + cycles;
        let pulses = total / self.divider();
        self.phase = total % self.divider();
        if pulses == 0 || !self.enabled {
            return;
        }

        let load = u64::from(self.load);
        let mut value = u64::from(self.value);
        let mut remaining = pulses;

        // A halted one-shot stays put until a new Load write — or until
        // one-shot mode is deselected, in which case continuous operation
        // resumes from Load (datasheet: "halts ... until One-Shot Timer mode
        // is deselected or a new load value is written").
        if value == 0 {
            if self.one_shot || load == 0 {
                return;
            }
            value = load;
        }

        // First leg: from the current value down to zero.
        if remaining < value {
            self.value = (value - remaining) as u32;
            return;
        }
        remaining -= value;

        if self.one_shot || load == 0 {
            // Halt at zero. The enable bit is NOT cleared: the datasheet says
            // the counter halts, and a new Load write restarts it.
            self.raw_irq = true;
            self.value = 0;
            return;
        }

        // Continuous: the counter reloads from Load at the zero boundary and
        // keeps decrementing, so a period is exactly `load` pulses.
        let rest = remaining % load;
        value = if rest == 0 { load } else { load - rest };
        self.raw_irq = true;
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
                // Writing Load loads the counter immediately (and restarts a
                // running count at the new value). It does NOT enable the
                // timer: counting starts only when Timer Enable is set.
                self.load = value;
                self.value = value;
            }
            TIMER_VALUE => {} // read-only
            TIMER_CONTROL => {
                let was_one_shot = self.one_shot;
                self.enabled = value & 1 != 0;
                self.irq_enabled = value & 2 != 0;
                self.one_shot = value & 4 != 0;
                // Deselecting one-shot resumes an expired timer from Load
                // (datasheet: the counter halts "until One-Shot Timer mode is
                // deselected or a new load value is written").
                if was_one_shot
                    && !self.one_shot
                    && self.enabled
                    && self.value == 0
                    && self.load > 0
                {
                    self.value = self.load;
                }
            }
            TIMER_PRESCALE => {
                self.prescale = (value & 0xF) as u8;
                // The phase counts PCLK cycles toward the next prescaled
                // pulse. A divider change must not replay the old span as new
                // pulses: clamp it into the new counter range (highest state
                // before the pulse), so at most one catch-up pulse is
                // produced, exactly like the hardware's compare reset.
                let divider = self.divider();
                self.phase = self.phase.min(divider - 1);
            }
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

    /// The timer drives a LEVEL interrupt (CoreTimer handbook): the line
    /// tracks the latched raw status AND the enable. Declaring the level lets
    /// the bus drop a pending NVIC bit when firmware clears or masks the
    /// interrupt, instead of delivering a second handler entry.
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.raw_irq && self.irq_enabled)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Restore the JSON snapshot the determinism/replay tools carry.
    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("MivCoreTimer snapshot decode: {error}"))
        })?;
        *self = restored;
        Ok(())
    }

    /// Binary mid-flight snapshot: the live countdown, prescaler phase and
    /// interrupt flags, so a resume continues the same interval.
    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize MivCoreTimer")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "MivCoreTimer runtime snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

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
    fn gpio_outbuff_gates_the_pad_drive() {
        let mut gpio = MivCoreGpio::new();
        // OUTREG (bit 0) alone selects the output register but leaves the
        // output buffer disabled: the pad is released (GPIO_OE = 0).
        gpio.write_u32(0x00, 0b001).unwrap();
        gpio.write_u32(GPIO_OUTPUT, 1).unwrap();
        assert_eq!(
            gpio.driver(0).unwrap().out,
            crate::pins::Out::Off,
            "OUTREG without OUTBUFF must not drive"
        );
        // Enable the output buffer: the written level reaches the pad.
        gpio.write_u32(0x00, 0b101).unwrap();
        assert_eq!(
            gpio.driver(0).unwrap().out,
            crate::pins::Out::High,
            "OUTREG + OUTBUFF drives the written level"
        );
        // Clearing OUTBUFF releases a previously driven pin.
        gpio.write_u32(0x00, 0b001).unwrap();
        assert_eq!(
            gpio.driver(0).unwrap().out,
            crate::pins::Out::Off,
            "clearing OUTBUFF releases the pad"
        );

        // Clearing OUTREG while the buffer stays enabled forces the output
        // data to 0 (drive low) — it does NOT release the pad — and the
        // written level is retained for when OUTREG comes back.
        gpio.write_u32(0x00, 0b101).unwrap();
        assert_eq!(gpio.driver(0).unwrap().out, crate::pins::Out::High);
        gpio.write_u32(0x00, 0b100).unwrap();
        assert_eq!(
            gpio.driver(0).unwrap().out,
            crate::pins::Out::Low,
            "OUTREG clear forces the output data to zero, buffer still enabled"
        );
        gpio.write_u32(0x00, 0b101).unwrap();
        assert_eq!(
            gpio.driver(0).unwrap().out,
            crate::pins::Out::High,
            "the written level was retained while OUTREG was clear"
        );
    }

    #[test]
    fn gpio_output_register_masks_by_direction() {
        let mut gpio = MivCoreGpio::new();
        gpio.write_u32(0x00, 0b101).unwrap(); // pin 0 output + buffer enabled
        gpio.write_u32(0x04, 0b010).unwrap(); // pin 1 input
        gpio.write_u32(GPIO_OUTPUT, 0b11).unwrap();
        assert_eq!(
            gpio.read_u32(GPIO_OUTPUT).unwrap(),
            0b01,
            "only the output-configured pin's bit is stored/read"
        );
    }

    #[test]
    fn gpio_input_register_reports_external_levels_on_input_pins_only() {
        let mut gpio = MivCoreGpio::new();
        gpio.write_u32(0x00, 0b101).unwrap(); // pin0 output
        gpio.write_u32(0x04, 0b010).unwrap(); // pin1 input
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
    fn gpio_exposes_the_board_io_capability_hooks() {
        let mut gpio = MivCoreGpio::new();
        gpio.write_u32(0x00, 0b101).unwrap(); // pin 0 output + buffer
        gpio.write_u32(0x04, 0b010).unwrap(); // pin 1 input
        gpio.write_u32(GPIO_OUTPUT, 1).unwrap();
        gpio.set_external(1, External::Level(true));

        assert_eq!(gpio.read_gpio_input(1), Some(true));
        assert_eq!(
            gpio.read_gpio_input(0),
            Some(false),
            "an output pin's input register bit is forced to 0"
        );
        assert_eq!(gpio.read_gpio_input_word(), 0b10);
        assert_eq!(gpio.read_gpio_output(0), Some(true));
        assert_eq!(gpio.read_gpio_output(1), Some(false));
        assert_eq!(gpio.read_gpio_is_output(0), Some(true));
        assert_eq!(gpio.read_gpio_is_output(1), Some(false));
    }

    /// Direction capability, pad drive and latch read must agree for every
    /// OUTREG/OUTBUFF combination.
    #[test]
    fn gpio_direction_capability_matches_the_pad_drive() {
        for (cfg, drives, level) in [
            (0b000u32, false, crate::pins::Out::Off),
            (0b001, false, crate::pins::Out::Off), // OUTREG only: buffer off
            (0b100, true, crate::pins::Out::Low),  // buffer on, data forced 0
            (0b101, true, crate::pins::Out::High), // buffer on + OUTREG, latch 1
        ] {
            let mut gpio = MivCoreGpio::new();
            gpio.write_u32(0x00, cfg).unwrap();
            gpio.write_u32(GPIO_OUTPUT, 1).unwrap();
            assert_eq!(
                gpio.read_gpio_is_output(0),
                Some(drives),
                "cfg {cfg:#05b}: direction follows the output enable"
            );
            assert_eq!(
                gpio.driver(0).unwrap().out,
                level,
                "cfg {cfg:#05b}: pad drive"
            );
            assert_eq!(
                gpio.read_gpio_output(0),
                Some(cfg & 1 != 0),
                "cfg {cfg:#05b}: the latch read masks by OUTREG"
            );
        }
    }

    /// The board_io attach path drives the released level and proves it with
    /// `read_gpio_input`; without the capability hooks the button is silently
    /// dropped. Build a real bus from a descriptor + manifest and check it.
    #[test]
    fn gpio_button_binding_attaches_through_the_bus() {
        use labwired_config::{ChipDescriptor, SystemManifest};

        let chip_yaml = r#"
name: "miv-gpio-test"
arch: "riscv"
core: "rv32im"
cpu_hz: 1_000_000
flash: { base: 0x60000000, size: "256KB" }
ram: { base: 0x80000000, size: "64KB" }
peripherals:
  - id: "gpioInputs"
    type: "miv_coregpio"
    base_address: 0x70002000
    size: "1KB"
"#;
        let manifest_yaml = r#"
name: "miv-gpio-test"
chip: "miv-gpio-test.yaml"
board_io:
  - id: "user_switch_0"
    kind: "button"
    peripheral: "gpioInputs"
    pin: 0
    signal: "input"
    active_high: false
"#;
        let chip: ChipDescriptor = serde_yaml::from_str(chip_yaml).unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(manifest_yaml).unwrap();
        let mut bus = crate::bus::SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(
            bus.gpio_devices.len(),
            1,
            "the button binding must attach through read_gpio_input"
        );
        assert!(bus.set_peripheral_gpio_input(0, 0, true));
        assert_eq!(bus.peripherals[0].dev.read_gpio_input(0), Some(true));
    }

    #[test]
    fn gpio_snapshot_restores_direction_levels_and_inputs() {
        let mut gpio = MivCoreGpio::new();
        gpio.write_u32(0x00, 0b101).unwrap(); // pin 0 output + buffer
        gpio.write_u32(0x04, 0b010).unwrap(); // pin 1 input
        gpio.write_u32(GPIO_OUTPUT, 1).unwrap();
        gpio.set_external(1, External::Level(true));
        let json = gpio.snapshot();
        let bytes = gpio.runtime_snapshot();

        // Mutate the live model so the restore is what brings the state back.
        gpio.write_u32(0x00, 0).unwrap();
        gpio.set_external(1, External::Level(false));

        let mut from_json = MivCoreGpio::new();
        from_json.restore(json).unwrap();
        assert_eq!(from_json.read_u32(0x00).unwrap(), 0b101);
        assert_eq!(
            from_json.driver(0).unwrap().out,
            crate::pins::Out::High,
            "the restored output level drives again"
        );
        assert_eq!(from_json.input(1), Some(true), "the restored input level");

        let mut from_bytes = MivCoreGpio::new();
        from_bytes.restore_runtime_snapshot(&bytes).unwrap();
        assert_eq!(from_bytes.read_u32(0x00).unwrap(), 0b101);
        assert_eq!(from_bytes.read_u32(GPIO_OUTPUT).unwrap(), 1);
    }

    #[test]
    fn timer_snapshot_restores_the_countdown_and_flags() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 0).unwrap(); // divide by 2
        timer.write_u32(TIMER_LOAD, 100).unwrap();
        timer.write_u32(TIMER_CONTROL, 0b011).unwrap(); // enable + irq
        let _ = timer.tick_elapsed(20); // 10 pulses: 100 -> 90
        let json = timer.snapshot();
        let bytes = timer.runtime_snapshot();

        // The live model keeps counting; the restore must resume at 90.
        let _ = timer.tick_elapsed(40); // 20 more pulses: 90 -> 70
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 70);

        let mut from_json = MivCoreTimer::new();
        from_json.restore(json).unwrap();
        assert_eq!(from_json.read_u32(TIMER_VALUE).unwrap(), 90);
        let _ = from_json.tick_elapsed(2); // one pulse
        assert_eq!(from_json.read_u32(TIMER_VALUE).unwrap(), 89);
        assert_eq!(from_json.read_u32(TIMER_CONTROL).unwrap() & 3, 3, "enabled");

        let mut from_bytes = MivCoreTimer::new();
        from_bytes.restore_runtime_snapshot(&bytes).unwrap();
        let _ = from_bytes.tick_elapsed(2);
        assert_eq!(from_bytes.read_u32(TIMER_VALUE).unwrap(), 89);
    }

    #[test]
    fn timer_reset_value_is_all_ones_and_disabled() {
        let timer = MivCoreTimer::new();
        assert_eq!(
            timer.read_u32(TIMER_VALUE).unwrap(),
            0xFFFF_FFFF,
            "TimerValue resets to all-ones (datasheet)"
        );
        assert_eq!(timer.read_u32(TIMER_CONTROL).unwrap() & 1, 0, "disabled");
        assert_eq!(timer.read_u32(TIMER_PRESCALE).unwrap(), 0, "divide by 2");
        assert_eq!(timer.read_u32(TIMER_RAW_STATUS).unwrap(), 0);
    }

    #[test]
    fn timer_counts_down_and_latches_the_raw_status() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 0).unwrap(); // divide by 2
        timer.write_u32(TIMER_LOAD, 10).unwrap();
        timer.write_u32(TIMER_CONTROL, 0b011).unwrap(); // enable + irq enable
        let _ = timer.tick_elapsed(18); // 9 pulses: 10 -> 1
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 1);
        assert_eq!(timer.read_u32(TIMER_RAW_STATUS).unwrap(), 0);
        let r = timer.tick_elapsed(2); // 10th pulse reaches zero
        assert_eq!(
            timer.read_u32(TIMER_VALUE).unwrap(),
            10,
            "continuous mode reloads from Load at the zero boundary"
        );
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
        timer.write_u32(TIMER_PRESCALE, 2).unwrap(); // divide by 8
        timer.write_u32(TIMER_LOAD, 100).unwrap();
        timer.write_u32(TIMER_CONTROL, 0b001).unwrap();
        let _ = timer.tick_elapsed(7);
        assert_eq!(
            timer.read_u32(TIMER_VALUE).unwrap(),
            100,
            "7 cycles < 8: no pulse yet"
        );
        let _ = timer.tick_elapsed(1);
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 99);
    }

    #[test]
    fn timer_load_is_immediate_and_does_not_enable() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 0).unwrap();
        // One-shot selected, timer left disabled.
        timer.write_u32(TIMER_CONTROL, 0b100).unwrap();
        timer.write_u32(TIMER_LOAD, 10).unwrap();
        assert_eq!(
            timer.read_u32(TIMER_CONTROL).unwrap() & 1,
            0,
            "a Load write must not set Timer Enable"
        );
        assert_eq!(
            timer.read_u32(TIMER_VALUE).unwrap(),
            10,
            "Load writes the counter immediately"
        );
        let _ = timer.tick_elapsed(1000);
        assert_eq!(
            timer.read_u32(TIMER_VALUE).unwrap(),
            10,
            "a disabled counter does not move"
        );

        // A running counter restarts at the new value on a Load write.
        timer.write_u32(TIMER_CONTROL, 0b001).unwrap();
        let _ = timer.tick_elapsed(4); // two pulses: 10 -> 8
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 8);
        timer.write_u32(TIMER_LOAD, 50).unwrap();
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 50);
        let _ = timer.tick_elapsed(2);
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 49);
    }

    #[test]
    fn deselecting_one_shot_resumes_an_expired_timer() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 0).unwrap(); // divide by 2
        timer.write_u32(TIMER_LOAD, 3).unwrap();
        timer.write_u32(TIMER_CONTROL, 0b101).unwrap(); // one-shot + enable
        let _ = timer.tick_elapsed(8); // 4 pulses: 3 -> 0 and halt
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 0);
        timer.write_u32(TIMER_IRQ_CLEAR, 1).unwrap();

        // Select continuous without rewriting Load: the counter reloads and
        // keeps producing zero events (datasheet: deselecting one-shot
        // resumes the timer).
        timer.write_u32(TIMER_CONTROL, 0b001).unwrap();
        assert_eq!(
            timer.read_u32(TIMER_VALUE).unwrap(),
            3,
            "deselecting one-shot reloads the counter from Load"
        );
        let _ = timer.tick_elapsed(2); // one pulse
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 2);
        let _ = timer.tick_elapsed(4); // two more pulses: zero at the second
        assert_eq!(timer.read_u32(TIMER_RAW_STATUS).unwrap(), 1);
    }

    #[test]
    fn changing_the_prescaler_does_not_replay_stale_phase() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 9).unwrap(); // divide by 1024
        timer.write_u32(TIMER_LOAD, 100).unwrap();
        timer.write_u32(TIMER_CONTROL, 0b001).unwrap();
        let _ = timer.tick_elapsed(1000); // no pulse; 1000 cycles of phase
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 100);

        // Reconfigure to divide-by-2: the accumulated phase must not be
        // replayed as ~500 pulses. At most one catch-up pulse fires.
        timer.write_u32(TIMER_PRESCALE, 0).unwrap();
        let _ = timer.tick_elapsed(2);
        assert_eq!(
            timer.read_u32(TIMER_VALUE).unwrap(),
            99,
            "one catch-up pulse, not the old span replayed"
        );
    }

    /// Bus-level: programming through MMIO, ticking the bus, expiry and the
    /// level deasserting on clear all work through the real register path.
    #[test]
    fn timer_advances_and_raises_irq_through_the_bus() {
        use crate::Bus;
        use labwired_config::{ChipDescriptor, SystemManifest};

        let chip_yaml = r#"
name: "miv-timer-test"
arch: "riscv"
core: "rv32im"
cpu_hz: 1_000_000
flash: { base: 0x60000000, size: "256KB" }
ram: { base: 0x80000000, size: "64KB" }
peripherals:
  - id: "timer0"
    type: "miv_coretimer"
    base_address: 0x70003000
    size: "1KB"
"#;
        let manifest_yaml = r#"
name: "miv-timer-test"
chip: "miv-timer-test.yaml"
"#;
        let chip: ChipDescriptor = serde_yaml::from_str(chip_yaml).unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(manifest_yaml).unwrap();
        let mut bus = crate::bus::SystemBus::from_config(&chip, &manifest).unwrap();

        bus.write_u32(0x7000_300C, 0).unwrap(); // divide by 2
        bus.write_u32(0x7000_3000, 10).unwrap(); // load
        bus.write_u32(0x7000_3008, 0b011).unwrap(); // enable + irq enable
                                                    // 18 cycles -> 9 pulses; the counter is at 1.
        for _ in 0..18 {
            let _ = bus.tick_peripherals_fully();
        }
        assert_eq!(bus.read_u32(0x7000_3004).unwrap(), 1);
        // Two more cycles -> the tenth pulse reaches zero and latches.
        for _ in 0..2 {
            let _ = bus.tick_peripherals_fully();
        }
        assert_eq!(bus.read_u32(0x7000_3014).unwrap(), 1, "raw status");
        assert_eq!(bus.read_u32(0x7000_3018).unwrap(), 1, "masked status");
        assert_eq!(
            bus.peripherals[0].dev.irq_line_level(),
            Some(true),
            "the bus-visible interrupt level is asserted"
        );
        bus.write_u32(0x7000_3010, 1).unwrap(); // InterruptClear
        assert_eq!(bus.read_u32(0x7000_3014).unwrap(), 0);
        assert_eq!(bus.peripherals[0].dev.irq_line_level(), Some(false));

        // Masking also deasserts the level while raw stays set.
        bus.write_u32(0x7000_3008, 0b001).unwrap(); // clear irq enable
        assert_eq!(bus.peripherals[0].dev.irq_line_level(), Some(false));
    }

    /// A snapshot taken mid-prescaler-period and with a latched interrupt must
    /// resume the same timing and the same IRQ state.
    #[test]
    fn timer_snapshot_preserves_partial_prescale_and_a_latched_irq() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 1).unwrap(); // divide by 4
        timer.write_u32(TIMER_LOAD, 5).unwrap();
        timer.write_u32(TIMER_CONTROL, 0b011).unwrap(); // enable + irq
        let _ = timer.tick_elapsed(6); // one pulse, phase 2
        let json = timer.snapshot();
        let mut restored = MivCoreTimer::new();
        restored.restore(json).unwrap();
        assert_eq!(restored.read_u32(TIMER_VALUE).unwrap(), 4);
        let _ = restored.tick_elapsed(2); // 2 + 2 cycles -> one more pulse
        assert_eq!(
            restored.read_u32(TIMER_VALUE).unwrap(),
            3,
            "the partial prescaler phase survived the round trip"
        );

        // Latched IRQ: expire and snapshot with raw set.
        let _ = restored.tick_elapsed(100);
        assert_eq!(restored.read_u32(TIMER_RAW_STATUS).unwrap(), 1);
        let mut pending = MivCoreTimer::new();
        pending.restore(restored.snapshot()).unwrap();
        assert_eq!(pending.read_u32(TIMER_RAW_STATUS).unwrap(), 1);
        assert_eq!(pending.read_u32(TIMER_MASKED_STATUS).unwrap(), 1);
        assert_eq!(pending.irq_line_level(), Some(true));
    }

    #[test]
    fn timer_irq_is_a_level_that_follows_clear_and_mask() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 0).unwrap(); // divide by 2
        timer.write_u32(TIMER_LOAD, 3).unwrap();
        timer.write_u32(TIMER_CONTROL, 0b011).unwrap(); // enable + irq enable
        let _ = timer.tick_elapsed(6); // 3 pulses -> zero
        assert_eq!(timer.read_u32(TIMER_RAW_STATUS).unwrap(), 1);
        assert_eq!(timer.irq_line_level(), Some(true));

        // Clearing the raw status drops the line (a level, not a pulse).
        timer.write_u32(TIMER_IRQ_CLEAR, 1).unwrap();
        assert_eq!(
            timer.irq_line_level(),
            Some(false),
            "clearing the interrupt drops the level"
        );

        // Fire again, then mask: the line drops even though raw is set.
        let _ = timer.tick_elapsed(6); // another period: 3 -> 0 (reload 3)
        assert_eq!(timer.read_u32(TIMER_RAW_STATUS).unwrap(), 1);
        timer.write_u32(TIMER_CONTROL, 0b001).unwrap(); // clear irq enable
        assert_eq!(
            timer.irq_line_level(),
            Some(false),
            "masking the interrupt drops the level"
        );
    }

    #[test]
    fn timer_one_shot_halts_and_continuous_reloads() {
        let mut timer = MivCoreTimer::new();
        timer.write_u32(TIMER_PRESCALE, 0).unwrap(); // divide by 2
        timer.write_u32(TIMER_LOAD, 3).unwrap();
        timer.write_u32(TIMER_CONTROL, 0b101).unwrap(); // one-shot + enable
        let _ = timer.tick_elapsed(8); // 4 pulses: 3 -> 0 and halt
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 0);
        assert_eq!(timer.read_u32(TIMER_RAW_STATUS).unwrap(), 1);
        assert_eq!(
            timer.read_u32(TIMER_CONTROL).unwrap() & 1,
            1,
            "the datasheet halts the counter; the enable bit stays set"
        );
        let _ = timer.tick_elapsed(100);
        assert_eq!(
            timer.read_u32(TIMER_VALUE).unwrap(),
            0,
            "a halted one-shot does not move"
        );
        // A new Load write restarts it.
        timer.write_u32(TIMER_LOAD, 3).unwrap();
        let _ = timer.tick_elapsed(2);
        assert_eq!(timer.read_u32(TIMER_VALUE).unwrap(), 2);

        let mut periodic = MivCoreTimer::new();
        periodic.write_u32(TIMER_PRESCALE, 0).unwrap();
        periodic.write_u32(TIMER_LOAD, 3).unwrap();
        periodic.write_u32(TIMER_CONTROL, 0b001).unwrap(); // continuous
        let _ = periodic.tick_elapsed(8); // 4 pulses: 3 -> 0 (reload) -> 2
        assert_eq!(periodic.read_u32(TIMER_VALUE).unwrap(), 2);
        assert_eq!(periodic.read_u32(TIMER_RAW_STATUS).unwrap(), 1);
        assert_eq!(
            periodic.read_u32(TIMER_CONTROL).unwrap() & 1,
            1,
            "continuous keeps running"
        );
    }
}
