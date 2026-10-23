// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! LiteX Control-and-Status (CAS) block — the LED/switch/button CSR a LiteX SoC
//! exposes to the gateware's control logic (the Arty A7 build maps it at
//! 0x60006800).
//!
//! Register map (32-bit CSRs, 4-byte strides; block size 0x14):
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x00 | Leds | R/W | bit n drives LED n (readable back, per LiteX `csr.h`) |
//! | 0x04 | Switches | R | switch levels, bit n = switch n |
//! | 0x08 | ButtonsStatus | R | button levels, bit n = button n |
//! | 0x0C | ButtonsPending | R/W1C | latched press events; write 1 to clear |
//! | 0x10 | ButtonsEnabled | R/W | per-button interrupt enable |
//!
//! Pad numbering follows the SoC wiring: LEDs 0..31 (outputs), switches 32..63
//! (inputs), buttons 64..95 (inputs). The interrupt line is
//! `any(enabled & pending)` — a level.
//!
//! Board buttons on the Arty are active-low (a press pulls the line to 0), so
//! the descriptor sets `buttons_active_low: true`; the pending latch fires on
//! the *press* edge and is cleared through the W1C register.

use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{PeripheralTickResult, SimResult};

const LEDS: usize = 32;
const SWITCHES: usize = 32;
const BUTTONS: usize = 32;
const PINS: usize = LEDS + SWITCHES + BUTTONS;

const REG_LEDS: u64 = 0x00;
const REG_SWITCHES: u64 = 0x04;
const REG_BUTTONS: u64 = 0x08;
const REG_PENDING: u64 = 0x0C;
const REG_ENABLED: u64 = 0x10;

/// LiteX Control-and-Status block.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LitexControlStatus {
    leds: u32,
    pending: u32,
    enabled: u32,
    /// Levels presented on the switch pads (bit n = switch n).
    #[serde(default)]
    switch_levels: u32,
    /// Levels presented on the button pads (bit n = button n).
    #[serde(default)]
    button_levels: u32,
    /// A press pulls the line low (the Arty's physical buttons).
    #[serde(default)]
    buttons_active_low: bool,
}

impl LitexControlStatus {
    pub fn new() -> Self {
        Self {
            leds: 0,
            pending: 0,
            enabled: 0,
            switch_levels: 0,
            button_levels: 0,
            buttons_active_low: false,
        }
    }

    pub fn with_buttons_active_low(mut self, active_low: bool) -> Self {
        self.buttons_active_low = active_low;
        self
    }

    /// Latch a press on `index` when the logical state goes low -> high.
    fn note_button(&mut self, index: usize, was_line: bool) {
        let line = self.button_levels & (1 << index) != 0;
        let was_pressed = if self.buttons_active_low {
            !was_line
        } else {
            was_line
        };
        let pressed = if self.buttons_active_low { !line } else { line };
        if pressed && !was_pressed {
            self.pending |= 1 << index;
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            REG_LEDS => self.leds,
            // The status registers combinationally reflect the live pad levels
            // (there is no separate sampled copy to go stale across a reset).
            REG_SWITCHES => self.switch_levels,
            REG_BUTTONS => self.button_levels,
            REG_PENDING => self.pending,
            REG_ENABLED => self.enabled,
            _ => {
                crate::census_reg!("litex_control_status:LitexControlStatus", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            REG_LEDS => self.leds = value,
            REG_SWITCHES | REG_BUTTONS => {} // read-only
            // W1C: only the written ones clear.
            REG_PENDING => self.pending &= !value,
            REG_ENABLED => self.enabled = value,
            _ => {
                crate::census_reg!("litex_control_status:LitexControlStatus", offset, "write");
            }
        }
    }

    /// The interrupt level: any enabled pending button.
    fn any_irq(&self) -> bool {
        self.enabled & self.pending != 0
    }
}

impl Default for LitexControlStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for LitexControlStatus {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        // W1C: a read-modify-write would feed the latched bits back and clear
        // events this byte never wrote.
        if reg == REG_PENDING {
            self.write_reg(reg, (value as u32) << (byte * 8));
            return Ok(());
        }
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }

    /// Word stores are one register write (the CSRs are 32-bit).
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset % 4 == 0 {
            self.write_reg(offset, value);
            return Ok(());
        }
        // Defensive only: every CSR is 4-byte aligned, so a misaligned word
        // store is not expected in normal operation; fall back to byte writes.
        for (i, byte) in value.to_le_bytes().iter().enumerate() {
            self.write(offset + i as u64, *byte)?;
        }
        Ok(())
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    /// Board capability: the pad level of an input pin (switch/button).
    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        self.input(pin)
    }

    /// The board_io button attach reads pins individually; the input pins live
    /// above bit 31 (switches 32..63, buttons 64..95), so the 32-bit word
    /// cannot carry them. The per-pin `read_gpio_input` path is authoritative;
    /// a renderer that falls back to this word accessor sees no input state.
    fn read_gpio_input_word(&self) -> u32 {
        0
    }

    /// Board capability: the LED latch for an output pin.
    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < LEDS).then_some(self.leds & (1 << pin) != 0)
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        ((pin as usize) < PINS).then_some((pin as usize) < LEDS)
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.any_irq())
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        PeripheralTickResult {
            irq: self.any_irq(),
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "LitexControlStatus snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize LitexControlStatus")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "LitexControlStatus runtime snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }

    /// A node reset returns the block to its reset state; the switch/button
    /// levels are the world's connections and survive, and `buttons_active_low`
    /// is a board-wiring fact (a reset does not rewire the physical buttons).
    fn on_node_reset(&mut self) {
        let switch_levels = self.switch_levels;
        let button_levels = self.button_levels;
        let buttons_active_low = self.buttons_active_low;
        *self = Self::new();
        self.buttons_active_low = buttons_active_low;
        self.switch_levels = switch_levels;
        self.button_levels = button_levels;
    }
}

impl PinPort for LitexControlStatus {
    fn pin_count(&self) -> u8 {
        PINS as u8
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        let pin = pin as usize;
        if pin >= LEDS {
            return None; // switches and buttons do not drive
        }
        Some(PadDriver::drive(self.leds & (1 << pin) != 0))
    }

    fn external(&self, pin: u8) -> External {
        let pin = pin as usize;
        if (SWITCHES..LEDS + SWITCHES).contains(&pin) {
            let bit = pin - LEDS;
            External::Level(self.switch_levels & (1 << bit) != 0)
        } else if (LEDS + SWITCHES..PINS).contains(&pin) {
            let bit = pin - (LEDS + SWITCHES);
            External::Level(self.button_levels & (1 << bit) != 0)
        } else {
            External::Released
        }
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        let pin = pin as usize;
        let level = ext.level();
        if (SWITCHES..LEDS + SWITCHES).contains(&pin) {
            let bit = pin - LEDS;
            let before = self.switch_levels & (1 << bit) != 0;
            if let Some(level) = level {
                self.switch_levels = (self.switch_levels & !(1 << bit)) | ((level as u32) << bit);
            }
            Some(InputChange {
                before,
                after: self.switch_levels & (1 << bit) != 0,
            })
        } else if (LEDS + SWITCHES..PINS).contains(&pin) {
            let bit = pin - (LEDS + SWITCHES);
            let before = self.button_levels & (1 << bit) != 0;
            if let Some(level) = level {
                self.button_levels = (self.button_levels & !(1 << bit)) | ((level as u32) << bit);
                self.note_button(bit, before);
            }
            Some(InputChange {
                before,
                after: self.button_levels & (1 << bit) != 0,
            })
        } else {
            None // LEDs do not accept input
        }
    }

    fn input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        if (SWITCHES..LEDS + SWITCHES).contains(&pin) {
            Some(self.switch_levels & (1 << (pin - LEDS)) != 0)
        } else if (LEDS + SWITCHES..PINS).contains(&pin) {
            Some(self.button_levels & (1 << (pin - LEDS - SWITCHES)) != 0)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn reset_values_and_sizes_match_the_gateware() {
        let cas = LitexControlStatus::new();
        for reg in [
            REG_LEDS,
            REG_SWITCHES,
            REG_BUTTONS,
            REG_PENDING,
            REG_ENABLED,
        ] {
            assert_eq!(cas.read_u32(reg).unwrap(), 0, "register {reg:#04x}");
        }
        assert_eq!(cas.irq_line_level(), Some(false));
    }

    #[test]
    fn leds_drive_and_read_back() {
        let mut cas = LitexControlStatus::new();
        cas.write_u32(REG_LEDS, 0b1010).unwrap();
        assert_eq!(cas.read_u32(REG_LEDS).unwrap(), 0b1010);
        assert_eq!(cas.driver(1).unwrap().out.level(), Some(true));
        assert_eq!(cas.driver(0).unwrap().out.level(), Some(false));
        cas.write_u32(REG_LEDS, 0).unwrap();
        assert_eq!(cas.driver(1).unwrap().out.level(), Some(false));
    }

    #[test]
    fn switches_and_buttons_report_the_pad_levels() {
        let mut cas = LitexControlStatus::new();
        cas.set_external(LEDS as u8 + 2, External::Level(true)); // switch 2
        assert_eq!(cas.read_u32(REG_SWITCHES).unwrap(), 0b100);
        cas.set_external((LEDS + SWITCHES) as u8 + 1, External::Level(true)); // button 1 high
        assert_eq!(cas.read_u32(REG_BUTTONS).unwrap(), 0b10);
        assert_eq!(cas.input(LEDS as u8 + 2), Some(true));
        assert_eq!(cas.input(0), None, "an LED is not an input");
    }

    #[test]
    fn node_reset_preserves_the_wiring_and_the_status_readback() {
        let mut cas = LitexControlStatus::new().with_buttons_active_low(true);
        cas.write_u32(REG_ENABLED, 0b1).unwrap();
        // A held switch and a released (high, active-low) button.
        cas.set_external(LEDS as u8, External::Level(true)); // switch 0
        cas.set_external((LEDS + SWITCHES) as u8, External::Level(true)); // button 0 released
        assert_eq!(cas.read_u32(REG_SWITCHES).unwrap(), 0b1);

        cas.on_node_reset();

        // The status CSRs still reflect the held pad levels (not zero), and the
        // registered interrupts are cleared.
        assert_eq!(cas.read_u32(REG_SWITCHES).unwrap(), 0b1);
        assert_eq!(cas.read_u32(REG_BUTTONS).unwrap(), 0b1);
        assert_eq!(cas.read_u32(REG_PENDING).unwrap(), 0);
        assert_eq!(cas.read_u32(REG_ENABLED).unwrap(), 0);

        // The active-low wiring survives the reset: a press still latches.
        cas.write_u32(REG_ENABLED, 0b1).unwrap();
        cas.set_external((LEDS + SWITCHES) as u8, External::Level(false));
        assert_eq!(cas.read_u32(REG_PENDING).unwrap(), 0b1);
    }

    #[test]
    fn snapshot_and_runtime_snapshot_round_trip_the_wiring() {
        let mut cas = LitexControlStatus::new().with_buttons_active_low(true);
        cas.write_u32(REG_LEDS, 0b101).unwrap();
        cas.set_external(LEDS as u8, External::Level(true)); // switch 0 held
        cas.set_external((LEDS + SWITCHES) as u8, External::Level(true)); // button 0 released

        let json = cas.snapshot();
        let mut from_json = LitexControlStatus::new();
        from_json.restore(json).unwrap();
        assert_eq!(from_json.read_u32(REG_LEDS).unwrap(), 0b101);
        assert_eq!(from_json.read_u32(REG_SWITCHES).unwrap(), 0b1);
        assert!(from_json.buttons_active_low);

        let bytes = cas.runtime_snapshot();
        let mut from_bytes = LitexControlStatus::new();
        from_bytes.restore_runtime_snapshot(&bytes).unwrap();
        assert_eq!(from_bytes.read_u32(REG_LEDS).unwrap(), 0b101);
        assert_eq!(from_bytes.read_u32(REG_BUTTONS).unwrap(), 0b1);
        assert!(from_bytes.buttons_active_low);
    }

    #[test]
    fn active_low_buttons_latch_pending_on_the_press_edge_and_clear_w1c() {
        let mut cas = LitexControlStatus::new().with_buttons_active_low(true);
        cas.write_u32(REG_ENABLED, 0b1).unwrap();
        // Released = high; pressing pulls the line low.
        cas.set_external((LEDS + SWITCHES) as u8, External::Level(true));
        assert_eq!(cas.pending, 0, "release does not latch");
        cas.set_external((LEDS + SWITCHES) as u8, External::Level(false));
        assert_eq!(cas.pending, 1, "press latches");
        assert_eq!(cas.irq_line_level(), Some(true));
        cas.write_u32(REG_PENDING, 1).unwrap();
        assert_eq!(cas.pending, 0);
        assert_eq!(cas.irq_line_level(), Some(false));
    }

    #[test]
    fn pending_byte_stores_clear_only_their_own_byte() {
        let mut cas = LitexControlStatus::new().with_buttons_active_low(true);
        for pin in [0u8, 8] {
            cas.set_external((LEDS + SWITCHES) as u8 + pin, External::Level(true));
            cas.set_external((LEDS + SWITCHES) as u8 + pin, External::Level(false));
        }
        assert_eq!(cas.read_u32(REG_PENDING).unwrap(), 0b1_0000_0001);
        cas.write(REG_PENDING, 0x01).unwrap();
        assert_eq!(cas.read_u32(REG_PENDING).unwrap(), 0b1_0000_0000);
    }

    #[test]
    fn enabled_gates_the_interrupt_line() {
        let mut cas = LitexControlStatus::new().with_buttons_active_low(true);
        cas.set_external((LEDS + SWITCHES) as u8, External::Level(true));
        cas.set_external((LEDS + SWITCHES) as u8, External::Level(false));
        assert_eq!(cas.pending, 1);
        assert_eq!(cas.irq_line_level(), Some(false), "disabled by default");
        cas.write_u32(REG_ENABLED, 1).unwrap();
        assert_eq!(cas.irq_line_level(), Some(true));
        cas.write_u32(REG_ENABLED, 0).unwrap();
        assert_eq!(cas.irq_line_level(), Some(false));
    }
}
