// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RP2040 IO_BANK0 — the pad function mux (datasheet §2.19.2).
//!
//! This block decides, per pad, which peripheral is wired to it. Without it the
//! engine had no way to answer "who drives GP4?", so a logic analyzer clipped to
//! an RP2040 I²C or SPI pin could only ever read the SIO output latch — a flat
//! line while the bus was busy. The register map existed in
//! `configs/peripherals/rp2040/io_bank0.yaml` but was never wired into the chip,
//! so firmware writes selecting a function landed nowhere.
//!
//! Layout: `GPIOn_STATUS` at `8n`, `GPIOn_CTRL` at `8n + 4`, for GP0..GP29.
//! `CTRL.FUNCSEL` is bits [4:0] and resets to 31 (NULL — pad driven by nothing).
//! The function numbers are the pico-sdk `gpio_function` enum: SPI 1, UART 2,
//! I2C 3, PWM 4, SIO 5, PIO0 6, PIO1 7, GPCK 8, USB 9.
//!
//! Only FUNCSEL is modelled behaviourally. The OVER fields (`OUTOVER`,
//! `OEOVER`, `INOVER`, `IRQOVER`) store and read back, because firmware reads
//! them, but they do not invert or force anything yet — an honest gap rather
//! than a guess, and one that only bites a firmware deliberately overriding a
//! pad.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::{Peripheral, SimResult};

/// Number of pads IO_BANK0 controls (GP0..GP29).
pub const PAD_COUNT: u8 = 30;

/// `CTRL.FUNCSEL` reset value: 31 == NULL, no peripheral connected.
const FUNCSEL_NULL: u32 = 31;

/// pico-sdk `gpio_function`. Only the ones the engine can route are named.
pub const GPIO_FUNC_SPI: u32 = 1;
pub const GPIO_FUNC_UART: u32 = 2;
pub const GPIO_FUNC_I2C: u32 = 3;

/// Live per-pad function selection, shared with whoever needs to know which
/// peripheral owns a pad (the SIO GPIO model, for pad reads).
///
/// Shared rather than copied because the answer changes at runtime: firmware
/// re-assigns a pad and every reader must see it immediately, or a re-routed
/// pin keeps reporting from its old source.
#[derive(Debug)]
pub struct PadFunctions {
    ctrl: Vec<AtomicU32>,
}

impl Default for PadFunctions {
    fn default() -> Self {
        Self::new()
    }
}

impl PadFunctions {
    pub fn new() -> Self {
        Self {
            ctrl: (0..PAD_COUNT)
                .map(|_| AtomicU32::new(FUNCSEL_NULL))
                .collect(),
        }
    }

    /// The whole `GPIOn_CTRL` word.
    fn ctrl(&self, pin: u8) -> u32 {
        self.ctrl
            .get(pin as usize)
            .map_or(FUNCSEL_NULL, |c| c.load(Ordering::Relaxed))
    }

    fn set_ctrl(&self, pin: u8, value: u32) {
        if let Some(cell) = self.ctrl.get(pin as usize) {
            cell.store(value, Ordering::Relaxed);
        }
    }

    /// The function currently selected for `pin`, or `None` when the pad is
    /// NULL (nothing connected) or out of range.
    ///
    /// This is the selector [`crate::peripherals::pad_routing::PadRoutes`]
    /// resolves pad routes against.
    pub fn function(&self, pin: u8) -> Option<u32> {
        let funcsel = self.ctrl(pin) & 0x1F;
        (funcsel != FUNCSEL_NULL).then_some(funcsel)
    }
}

/// The IO_BANK0 register block.
#[derive(Debug)]
pub struct Rp2040IoBank0 {
    pads: Arc<PadFunctions>,
}

impl Default for Rp2040IoBank0 {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040IoBank0 {
    pub fn new() -> Self {
        Self {
            pads: Arc::new(PadFunctions::new()),
        }
    }

    /// Share the live pad-function state, for the SIO GPIO model to resolve
    /// pad ownership against.
    pub fn pad_functions(&self) -> Arc<PadFunctions> {
        self.pads.clone()
    }

    /// `(pin, is_ctrl)` for a register offset inside the GPIO array.
    fn decode(offset: u64) -> Option<(u8, bool)> {
        let pin = (offset / 8) as u8;
        if pin >= PAD_COUNT {
            return None;
        }
        Some((pin, offset % 8 == 4))
    }

    fn read_u32(&self, offset: u64) -> u32 {
        match Self::decode(offset) {
            Some((pin, true)) => self.pads.ctrl(pin),
            // GPIOn_STATUS: not modelled, reads zero.
            Some((_, false)) => 0,
            None => 0,
        }
    }

    fn write_u32(&mut self, offset: u64, value: u32) {
        if let Some((pin, true)) = Self::decode(offset) {
            // Writable bits per the SVD: FUNCSEL [4:0] and the four OVER
            // fields. Everything else reads back zero.
            self.pads.set_ctrl(pin, value & 0x3003_331F);
        }
    }
}

impl Peripheral for Rp2040IoBank0 {
    /// Not in the per-cycle walk: this is a pure register bank. It overrides
    /// neither `tick()` nor `tick_elapsed()`, so every visit ran the default
    /// no-op and returned a default `PeripheralTickResult`. Skipping it removes
    /// dispatch, never an effect — byte-identical by construction.
    ///
    /// ⚠️ This was MISSING when the model was added, and the cost was not local
    /// to this file. `derive_walk_deletable` is `uses_scheduler() ||
    /// !needs_legacy_walk()`, and it is all-or-nothing for the whole bus: one
    /// model inheriting the conservative default `true` forces the per-cycle
    /// walk back on for EVERY RP2040 lab, including the majority that never
    /// route a pad. `rp2040_i2c_irq_delivery` caught it, but only under
    /// `--features event-scheduler`, which is not in the default test run — so
    /// it went unnoticed while the ordinary suite stayed green.
    ///
    /// Safe against the "sleeps and never wakes" trap for the same reason
    /// `GpioPort` is: there is no tick and no state-dependent condition to
    /// starve, and the bus re-arms `refresh_legacy_tick_index()` on every MMIO
    /// write if this model ever grows one.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let word = (self.read_u32(aligned) & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(Rp2040IoBank0::read_u32(self, offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        Rp2040IoBank0::write_u32(self, offset, value);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl_offset(pin: u8) -> u64 {
        u64::from(pin) * 8 + 4
    }

    #[test]
    fn every_pad_resets_to_null_not_to_function_zero() {
        // FUNCSEL 0 is XIP, a real function. A model that reset to 0 would
        // claim every pad was driven by the flash interface from power-on.
        let bank = Rp2040IoBank0::new();
        let pads = bank.pad_functions();
        for pin in 0..PAD_COUNT {
            assert_eq!(pads.function(pin), None, "GP{pin} must start unconnected");
        }
        assert_eq!(bank.read_u32(ctrl_offset(0)) & 0x1F, FUNCSEL_NULL);
    }

    #[test]
    fn selecting_a_function_is_visible_to_a_sharer_immediately() {
        let mut bank = Rp2040IoBank0::new();
        let pads = bank.pad_functions();
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(4), GPIO_FUNC_I2C);
        assert_eq!(pads.function(4), Some(GPIO_FUNC_I2C));
        assert_eq!(pads.function(5), None, "only the written pad moved");
    }

    #[test]
    fn a_pad_handed_back_to_null_stops_reporting_a_function() {
        let mut bank = Rp2040IoBank0::new();
        let pads = bank.pad_functions();
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(9), GPIO_FUNC_UART);
        assert_eq!(pads.function(9), Some(GPIO_FUNC_UART));
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(9), FUNCSEL_NULL);
        assert_eq!(pads.function(9), None);
    }

    #[test]
    fn ctrl_words_read_back_what_firmware_wrote() {
        let mut bank = Rp2040IoBank0::new();
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(2), GPIO_FUNC_SPI);
        assert_eq!(bank.read_u32(ctrl_offset(2)), GPIO_FUNC_SPI);
        // Reserved bits do not stick.
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(2), 0xFFFF_FFFF);
        assert_eq!(bank.read_u32(ctrl_offset(2)), 0x3003_331F);
    }

    #[test]
    fn status_is_not_invented() {
        // GPIOn_STATUS is real silicon state this model does not derive; it
        // must read zero rather than a plausible-looking guess.
        let bank = Rp2040IoBank0::new();
        assert_eq!(bank.read_u32(0), 0);
        assert_eq!(bank.read_u32(8), 0);
    }

    #[test]
    fn byte_writes_compose_into_the_same_word() {
        let mut bank = Rp2040IoBank0::new();
        bank.write(ctrl_offset(7), GPIO_FUNC_I2C as u8).unwrap();
        assert_eq!(bank.pad_functions().function(7), Some(GPIO_FUNC_I2C));
    }

    #[test]
    fn offsets_past_the_last_pad_are_inert() {
        let mut bank = Rp2040IoBank0::new();
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(PAD_COUNT), GPIO_FUNC_I2C);
        assert_eq!(bank.read_u32(ctrl_offset(PAD_COUNT)), 0);
    }
}
