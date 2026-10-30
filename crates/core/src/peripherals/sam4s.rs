// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Microchip SAM4S peripherals: the PIO controller and the PMC.
//!
//! The SAM4S (Cortex-M4) uses the older Atmel PIO block — parallel
//! PER/PDR/PSR, OER/ODR/OSR, SODR/CODR/ODSR and a pin-status read — not the
//! SAM D21 `PORT` block the `gpio` model implements (`SamPort`). The register
//! map and bit meanings are from the SAM4S datasheet (§31 "Parallel I/O
//! Controller", §28 "Power Management Controller").
//!
//! Modelled: pin enable, output enable, the output set/clear latch with its
//! ODSR read-back, PSR/PDSR pin levels, the input glitch filter registers
//! (stored) and the interrupt mask/status storage. Not modelled: the write
//! protection, the peripheral A-D mux side effects (`ABCDSR1/2` are stored), the
//! multi-drive/open-drain/PIO clock outputs, the debounce/schmitt controls and
//! the programmable clock (everything reads back what was written, or zero).
//! The PMC models the master-clock-ready status the boot path polls plus the
//! system and peripheral clock-enable/status banks.
//!
//! The SAM E70 (Cortex-M7) keeps the same PIO and PMC register layouts at
//! different bases (SAM E70 datasheet DS60001527, PIO and PMC chapters), so the
//! `same70q21b` chip reuses both models.

use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{Peripheral, SimResult};

const PINS: usize = 32;

// ── PIO register map (SAM4S datasheet §31.7) ────────────────────────────────
const PIO_PER: u64 = 0x0000;
const PIO_PDR: u64 = 0x0004;
const PIO_PSR: u64 = 0x0008;
const PIO_OER: u64 = 0x0010;
const PIO_ODR: u64 = 0x0014;
const PIO_OSR: u64 = 0x0018;
const PIO_IFER: u64 = 0x0020;
const PIO_IFDR: u64 = 0x0024;
const PIO_IFSR: u64 = 0x0028;
const PIO_SODR: u64 = 0x0030;
const PIO_CODR: u64 = 0x0034;
const PIO_ODSR: u64 = 0x0038;
const PIO_PDSR: u64 = 0x003C;
const PIO_IER: u64 = 0x0040;
const PIO_IDR: u64 = 0x0044;
const PIO_IMR: u64 = 0x0048;
const PIO_ISR: u64 = 0x004C;
const PIO_MDER: u64 = 0x0050;
const PIO_MDDR: u64 = 0x0054;
const PIO_MDSR: u64 = 0x0058;
const PIO_PUER: u64 = 0x0060;
const PIO_PUDR: u64 = 0x0064;
const PIO_PUSR: u64 = 0x0068;
/// PIO_ABCDSR1/PIO_ABCDSR2: the two peripheral-select bits per pin (A = 00,
/// B = 01, C = 10, D = 11). The SAM4S and SAM E70 both have four functions.
const PIO_ABSR: u64 = 0x0070;
const PIO_ABSR2: u64 = 0x0074;

fn bit(word: u32, index: usize) -> bool {
    word & (1 << index) != 0
}

/// SAM4S Parallel I/O controller (one instance per PIOA/PIOB/PIOC).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Sam4sPio {
    /// PIO_PER: pins claimed by the PIO controller.
    enabled: u32,
    /// PIO_OER: pins driven as outputs.
    output_enable: u32,
    /// PIO_ODSR: the output data latch.
    output: u32,
    /// PIO_IFER: input glitch filters enabled.
    input_filter: u32,
    /// PIO_IER: interrupt mask.
    irq_enable: u32,
    /// PIO_ISR: latched edge status (W1C via a read).
    irq_status: u32,
    /// PIO_ABCDSR1: peripheral-select bit 0.
    ab_select: u32,
    /// PIO_ABCDSR2: peripheral-select bit 1.
    #[serde(default)]
    ab_select2: u32,
    /// PIO_PUER: input pull-ups enabled.
    pull_up: u32,
    /// Levels presented on the pads by the outside world (`None` = nothing
    /// connected; the pad reads 0 in that case, as a floating input does).
    #[serde(default)]
    external: [Option<bool>; PINS],
}

impl Sam4sPio {
    pub fn new() -> Self {
        Self {
            enabled: 0,
            output_enable: 0,
            output: 0,
            input_filter: 0,
            irq_enable: 0,
            irq_status: 0,
            ab_select: 0,
            ab_select2: 0,
            pull_up: 0,
            external: [None; PINS],
        }
    }

    /// The level on `pin`: the output latch when the pin is driven, otherwise
    /// what the outside world presents.
    fn pad_level(&self, pin: usize) -> bool {
        if bit(self.output_enable, pin) {
            bit(self.output, pin)
        } else if let Some(level) = self.external[pin] {
            level
        } else {
            // An unconnected input follows its pull resistor; the SAM4S
            // resets with the pull-ups off, so it floats low.
            bit(self.pull_up, pin)
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            PIO_PER | PIO_PSR => self.enabled,
            PIO_OER | PIO_OSR => self.output_enable,
            PIO_IFER | PIO_IFSR => self.input_filter,
            PIO_ODSR => self.output,
            // PSR/PDSR are the live pin levels; the SAM4S reports the pads,
            // not the latch, so an input follows the outside world.
            PIO_PDSR => (0..PINS)
                .filter(|&p| self.pad_level(p))
                .fold(0u32, |word, p| word | (1 << p)),
            PIO_IER | PIO_IMR => self.irq_enable,
            PIO_ISR => self.irq_status,
            PIO_MDSR => 0,
            // PUSR reads 1 where no pull-up is enabled (datasheet §31.7.20).
            PIO_PUSR => !self.pull_up,
            PIO_ABSR => self.ab_select,
            PIO_ABSR2 => self.ab_select2,
            _ => {
                crate::census_reg!("sam4s:Sam4sPio", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            PIO_PER => self.enabled |= value,
            PIO_PDR => self.enabled &= !value,
            PIO_OER => {
                self.output_enable |= value;
                self.note_all_edges(value);
            }
            PIO_ODR => self.output_enable &= !value,
            PIO_IFER => self.input_filter |= value,
            PIO_IFDR => self.input_filter &= !value,
            PIO_SODR => {
                self.output |= value;
                self.note_all_edges(value);
            }
            PIO_CODR => {
                self.output &= !value;
                self.note_all_edges(value);
            }
            PIO_IER => self.irq_enable |= value,
            PIO_IDR => self.irq_enable &= !value,
            // Reading ISR clears it (rc_r); a write is ignored on silicon.
            PIO_ISR => {}
            PIO_MDER | PIO_MDDR => {}
            PIO_PUER => self.pull_up |= value,
            PIO_PUDR => self.pull_up &= !value,
            PIO_ABSR => self.ab_select = value,
            PIO_ABSR2 => self.ab_select2 = value,
            _ => {
                crate::census_reg!("sam4s:Sam4sPio", offset, "write");
            }
        }
    }

    /// Latch an edge on the pins in `mask` whose level changed. The model has
    /// no interrupt consumers in the smoke, but ISR/IMR are the registers a
    /// driver reads, so keep them coherent.
    fn note_all_edges(&mut self, mask: u32) {
        for pin in 0..PINS {
            if mask & (1 << pin) != 0 && self.pad_level(pin) {
                self.irq_status |= 1 << pin;
            }
        }
    }

    fn input_level(&self, pin: usize) -> bool {
        self.pad_level(pin)
    }
}

impl Default for Sam4sPio {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Sam4sPio {
    // Inert walk: a pure register bank with no `tick()`; pin edges are
    // surfaced by the bus GPIO-diff pass. Without these the bus walked every
    // instance on every cycle and the batched ARM loop never engaged.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mask: u32 = 0xFF << (byte * 8);
        let mut v = self.read_reg(reg);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    /// A node reset clears the chip-side configuration and latches. The
    /// outside world stays connected: a held button is not register state.
    fn on_node_reset(&mut self) {
        let external = self.external;
        *self = Self::new();
        self.external = external;
    }

    /// PIO_PSR reads the pin levels; use PDSR's semantics for the byte path.
    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.input_level(pin))
    }

    fn read_gpio_input_word(&self) -> u32 {
        (0..PINS)
            .filter(|&p| self.input_level(p))
            .fold(0u32, |word, p| word | (1 << p))
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| bit(self.output_enable, pin) && bit(self.output, pin))
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| bit(self.output_enable, pin))
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
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Sam4sPio snapshot decode: {error}"))
        })?;
        *self = restored;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize Sam4sPio")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "Sam4sPio runtime snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }
}

impl PinPort for Sam4sPio {
    fn pin_count(&self) -> u8 {
        PINS as u8
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        let pin = pin as usize;
        if pin >= PINS {
            return None;
        }
        Some(if bit(self.output_enable, pin) {
            PadDriver::drive(bit(self.output, pin))
        } else if bit(self.pull_up, pin) {
            PadDriver::released(crate::pins::Pull::Up)
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
        let before = self.input_level(pin);
        self.external[pin] = ext.level();
        let after = self.input_level(pin);
        if after != before {
            self.irq_status |= 1 << pin;
        }
        Some(InputChange { before, after })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.input_level(pin))
    }
}

// ── PMC (SAM4S datasheet §28.7) ─────────────────────────────────────────────
const PMC_SCER: u64 = 0x0000;
const PMC_SCDR: u64 = 0x0004;
const PMC_SCSR: u64 = 0x0008;
const PMC_PCER0: u64 = 0x0010;
const PMC_PCDR0: u64 = 0x0014;
const PMC_PCSR0: u64 = 0x0018;
// Bank 1 sits after the write-protection registers on both the SAM4S and the
// SAM E70 (0x40-0x48 are PMC_PCK0-2).
const PMC_PCER1: u64 = 0x0100;
const PMC_PCDR1: u64 = 0x0104;
const PMC_PCSR1: u64 = 0x0108;
const PMC_MCKR: u64 = 0x0030;
const PMC_SR: u64 = 0x0068;

/// PMC_SR bits the boot path waits for (datasheet §28.7.12): MOSCXTS (the
/// selected main clock is stable) and MCKRDY (the master clock is ready).
const SR_MOSCXTS: u32 = 1 << 0;
const SR_MCKRDY: u32 = 1 << 3;

/// SAM4S Power Management Controller.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Sam4sPmc {
    /// PMC_SCSR: system clock-enable status (USB, PCKx).
    #[serde(default)]
    scsr: u32,
    /// PMC_PCSR0: peripheral clock-enable bank 0.
    pcsr0: u32,
    /// PMC_PCSR1: peripheral clock-enable bank 1.
    pcsr1: u32,
    /// PMC_MCKR: master-clock / prescaler selection.
    mckr: u32,
}

impl Sam4sPmc {
    pub fn new() -> Self {
        Self {
            scsr: 0,
            pcsr0: 0,
            pcsr1: 0,
            // Reset selection: the RC oscillator divided by 1 (CSS = 0,
            // PRES = 0). The status reads ready because the RC is running.
            mckr: 0,
        }
    }
}

impl Default for Sam4sPmc {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Sam4sPmc {
    // Inert walk: a pure register bank with no `tick()`; pin edges are
    // surfaced by the bus GPIO-diff pass. Without these the bus walked every
    // instance on every cycle and the batched ARM loop never engaged.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok((self.read_u32(reg)? >> (byte * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mask: u32 = 0xFF << (byte * 8);
        let mut v = self.read_u32(reg)?;
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_u32(reg, v)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            PMC_SCSR => self.scsr,
            PMC_PCSR0 => self.pcsr0,
            PMC_PCSR1 => self.pcsr1,
            PMC_MCKR => self.mckr,
            // The RC oscillator is the reset clock and is stable, so the
            // status the boot path polls is ready from reset.
            PMC_SR => SR_MOSCXTS | SR_MCKRDY,
            _ => {
                crate::census_reg!("sam4s:Sam4sPmc", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            PMC_SCER => self.scsr |= value,
            PMC_SCDR => self.scsr &= !value,
            PMC_PCER0 => self.pcsr0 |= value,
            PMC_PCDR0 => self.pcsr0 &= !value,
            PMC_PCER1 => self.pcsr1 |= value,
            PMC_PCDR1 => self.pcsr1 &= !value,
            PMC_MCKR => self.mckr = value & 0x1F,
            _ => {
                crate::census_reg!("sam4s:Sam4sPmc", offset, "write");
            }
        }
        Ok(())
    }

    /// A node reset returns the PMC to its reset selections; the peripheral
    /// clock enables are register state and clear with it.
    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Sam4sPmc snapshot decode: {error}"))
        })?;
        *self = restored;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize Sam4sPmc")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "Sam4sPmc runtime snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pio_enables_outputs_and_sets_clears_the_latch() {
        let mut pio = Sam4sPio::new();
        pio.write_u32(PIO_OER, 1 << 10).unwrap();
        pio.write_u32(PIO_SODR, 1 << 10).unwrap();
        assert_eq!(pio.read_u32(PIO_ODSR).unwrap(), 1 << 10);
        assert_eq!(pio.read_u32(PIO_PDSR).unwrap(), 1 << 10);
        assert_eq!(pio.driver(10).unwrap().out.level(), Some(true));
        pio.write_u32(PIO_CODR, 1 << 10).unwrap();
        assert_eq!(pio.read_u32(PIO_ODSR).unwrap(), 0);
        assert_eq!(pio.driver(10).unwrap().out.level(), Some(false));
    }

    #[test]
    fn pio_inputs_follow_the_pad_and_survive_a_node_reset() {
        let mut pio = Sam4sPio::new();
        pio.write_u32(PIO_PER, 1 << 5).unwrap();
        // Releasing an active-low button presents a high level on the pad.
        pio.set_external(5, External::Level(true));
        assert_eq!(pio.read_u32(PIO_PSR).unwrap() & (1 << 5), 1 << 5);
        assert_eq!(pio.input(5), Some(true));

        pio.on_node_reset();
        // The pad connection survives; the enable registers clear.
        assert_eq!(pio.read_u32(PIO_PER).unwrap(), 0);
        assert_eq!(pio.input(5), Some(true));
    }

    #[test]
    fn pio_pull_up_makes_an_unconnected_input_read_high() {
        let mut pio = Sam4sPio::new();
        pio.write_u32(PIO_PER, 1 << 5).unwrap();
        pio.write_u32(PIO_PUER, 1 << 5).unwrap();
        assert_eq!(pio.input(5), Some(true));
        assert_eq!(pio.read_u32(PIO_PUSR).unwrap() & (1 << 5), 0);
        // A driven external level still wins over the pull.
        pio.set_external(5, External::Level(false));
        assert_eq!(pio.input(5), Some(false));
    }

    #[test]
    fn pio_output_latch_clears_on_a_node_reset_but_keeps_the_connection() {
        let mut pio = Sam4sPio::new();
        pio.write_u32(PIO_PER, 1 << 10).unwrap();
        pio.write_u32(PIO_OER, 1 << 10).unwrap();
        pio.write_u32(PIO_SODR, 1 << 10).unwrap();
        pio.on_node_reset();
        assert_eq!(pio.read_u32(PIO_ODSR).unwrap(), 0);
        assert_eq!(pio.read_u32(PIO_OER).unwrap(), 0);
    }

    #[test]
    fn pmc_enables_peripheral_clocks_and_reports_the_master_clock_ready() {
        let mut pmc = Sam4sPmc::new();
        pmc.write_u32(PMC_PCER0, (1 << 8) | (1 << 11) | (1 << 13))
            .unwrap();
        assert_eq!(
            pmc.read_u32(PMC_PCSR0).unwrap(),
            (1 << 8) | (1 << 11) | (1 << 13)
        );
        assert_eq!(
            pmc.read_u32(PMC_SR).unwrap() & (SR_MOSCXTS | SR_MCKRDY),
            SR_MOSCXTS | SR_MCKRDY
        );
        pmc.write_u32(PMC_PCDR0, 1 << 8).unwrap();
        assert_eq!(pmc.read_u32(PMC_PCSR0).unwrap(), (1 << 11) | (1 << 13));
    }

    #[test]
    fn pmc_bank1_is_at_0x100_and_system_clocks_stay_out_of_bank0() {
        let mut pmc = Sam4sPmc::new();
        // SAM E70 UART2 is peripheral id 44: PCER1 bit 12.
        pmc.write_u32(0x100, 1 << 12).unwrap();
        assert_eq!(pmc.read_u32(0x108).unwrap(), 1 << 12);
        pmc.write_u32(0x104, 1 << 12).unwrap();
        assert_eq!(pmc.read_u32(0x108).unwrap(), 0);
        // 0x40 is PMC_PCK0, not a peripheral-clock bank.
        pmc.write_u32(0x40, 1 << 12).unwrap();
        assert_eq!(pmc.read_u32(0x108).unwrap(), 0);
        // SCER enables a system clock (PCK0 = bit 8), not peripheral id 8.
        pmc.write_u32(PMC_SCER, 1 << 8).unwrap();
        assert_eq!(pmc.read_u32(PMC_SCSR).unwrap(), 1 << 8);
        assert_eq!(pmc.read_u32(PMC_PCSR0).unwrap(), 0);
        pmc.write_u32(PMC_SCDR, 1 << 8).unwrap();
        assert_eq!(pmc.read_u32(PMC_SCSR).unwrap(), 0);
    }

    #[test]
    fn pio_stores_both_peripheral_select_banks() {
        let mut pio = Sam4sPio::new();
        // Peripheral D on PB4 (SAM E70 USART1 TXD1): ABCDSR1 = ABCDSR2 = 1.
        pio.write_u32(PIO_ABSR, 1 << 4).unwrap();
        pio.write_u32(PIO_ABSR2, 1 << 4).unwrap();
        assert_eq!(pio.read_u32(PIO_ABSR).unwrap(), 1 << 4);
        assert_eq!(pio.read_u32(PIO_ABSR2).unwrap(), 1 << 4);
    }
}
