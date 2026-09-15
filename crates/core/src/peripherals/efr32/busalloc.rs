// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `GPIO_ABUSALLOC` / `GPIO_BBUSALLOC` / `GPIO_CDBUSALLOC` — the analog-bus
//! allocation registers that connect a pad to the IADC on Series 2.
//!
//! # Why this exists
//!
//! On Series 2 a pad reaches the ADC only through one of the analog buses, and
//! a bus is connected to NOTHING out of reset (`_GPIO_CDBUSALLOC_CDEVEN0_DEFAULT`
//! is 0, the same value as TRISTATE). So a silicon-correct `analogRead` writes
//! one of these three registers before it starts a conversion — the
//! silabs-arduino core does exactly that — and this window was not mapped at
//! all. The store bus-faulted, escalated to HardFault, and every sketch that
//! called `analogRead` on the BRD2709A twin parked in `Default_Handler` with no
//! output: the Arduino matrix's `L5_adc` cell, the browser, the hosted runs.
//! Silicon takes the same store without comment.
//!
//! This is the analog twin of the route trap `gpio_route.rs` and
//! `usart_route.rs` were written for, on the same block, one window further
//! down: the peripheral is enabled, and it is wired to nothing until the GPIO
//! block says otherwise.
//!
//! # Where the numbers come from
//!
//! `GPIO_TypeDef` in `efr32mg26_gpio.h` (simplicity_sdk `sisdk-2025.6`), walked
//! member by member the way the TIMERROUTE/USARTROUTE offsets were: `ABUSALLOC`
//! at block `+0x320`, `BBUSALLOC` at `+0x324`, `CDBUSALLOC` at `+0x328`. Ports
//! A and B have a register each; C and D SHARE `CDBUSALLOC`.
//!
//! Field layout, from the same header (`_GPIO_ABUSALLOC_AEVEN0_SHIFT` = 0,
//! `_GPIO_ABUSALLOC_AODD0_SHIFT` = 16, four bits each; `AEVEN1`/`AODD1` at 8
//! and 24 are the second bus, which nothing here drives): even-numbered pins
//! take the `xEVEN0` nibble, odd-numbered pins the `xODD0` nibble. The
//! `ADC0` selector is 1 in all three registers.
//!
//! # What is modelled
//!
//! The three words: stored, and read back verbatim, so a driver that
//! read-modify-writes its own allocation keeps every bit. [`Self::adc0_owns`]
//! answers "does the IADC currently see this pad?" from those bits.
//!
//! ⚠️ NOT YET ENFORCED: the IADC model still converts whatever level stands on
//! the selected pad without asking this block, so a firmware that never
//! allocates a bus — which reads 0 forever on silicon — still reads the pad on
//! the twin. `firmware-mg26-adc` and `firmware-mg26-deck` are two such
//! firmwares. Gating the IADC on [`Self::adc0_owns`] is the next step, the
//! same way `RouteGate` closed the USART/I2C route stubs, and it will turn
//! those two lanes red until they allocate — which is the correct outcome.

use crate::SimResult;

/// Words in the window: A, B, CD.
pub const BUSALLOC_WORDS: usize = 3;
/// `xODD0` starts at bit 16 — ⚠️ 16, not 8 (8 is `xEVEN1`, the second bus).
const ODD_SHIFT: u32 = 16;
/// Each selector field is four bits wide.
const FIELD_MASK: u32 = 0xF;
/// `_GPIO_ABUSALLOC_AEVEN0_ADC0` — the same value in all three registers.
pub const SEL_ADC0: u32 = 0x1;

/// The GPIO block's analog-bus allocation registers.
#[derive(Debug, Default, serde::Serialize)]
pub struct Efr32s2BusAlloc {
    /// `[0]` = ABUSALLOC, `[1]` = BBUSALLOC, `[2]` = CDBUSALLOC.
    regs: [u32; BUSALLOC_WORDS],
}

impl Efr32s2BusAlloc {
    pub fn new() -> Self {
        Self::default()
    }

    /// The register word that owns `port` (0..=3 for A..D), or `None` past D.
    fn word_for_port(port: u8) -> Option<usize> {
        match port {
            0 => Some(0),
            1 => Some(1),
            2 | 3 => Some(2),
            _ => None,
        }
    }

    /// True when the `EVEN0`/`ODD0` selector for `(port, pin)` names ADC0 —
    /// i.e. the IADC's input mux is connected to that pad right now.
    pub fn adc0_owns(&self, port: u8, pin: u8) -> bool {
        let Some(w) = Self::word_for_port(port) else {
            return false;
        };
        let shift = if pin & 1 == 1 { ODD_SHIFT } else { 0 };
        (self.regs[w] >> shift) & FIELD_MASK == SEL_ADC0
    }

    fn read_word(&self, offset: u64) -> u32 {
        let w = (offset / 4) as usize;
        self.regs.get(w).copied().unwrap_or(0)
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let w = (offset / 4) as usize;
        if let Some(slot) = self.regs.get_mut(w) {
            *slot = value;
        }
    }
}

impl crate::Peripheral for Efr32s2BusAlloc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        let merged = (self.read_word(reg) & !(0xFFu32 << shift)) | (u32::from(value) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// `as_any_mut`, NOT `as_any` — the default is `None`, and the IADC gate
    /// that will one day take this block's state through `downcast_mut` must
    /// not compile green while wiring nothing (see `gpio_route.rs`).
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    const OFF_ABUSALLOC: u64 = 0x00;
    const OFF_BBUSALLOC: u64 = 0x04;
    const OFF_CDBUSALLOC: u64 = 0x08;

    /// The exact sequence `analogRead(PD2)` performs on BRD2709A: a
    /// read-modify-write of `CDBUSALLOC`'s `CDEVEN0` field. It must land, read
    /// back, and answer "ADC0 owns PD02".
    #[test]
    fn the_arduino_core_allocation_reads_back_and_connects_the_pad() {
        let mut b = Efr32s2BusAlloc::new();
        assert!(
            !b.adc0_owns(3, 2),
            "out of reset the bus is connected to nothing"
        );
        let v = b.read_u32(OFF_CDBUSALLOC).unwrap();
        b.write_u32(OFF_CDBUSALLOC, (v & !FIELD_MASK) | SEL_ADC0)
            .unwrap();
        assert_eq!(b.read_u32(OFF_CDBUSALLOC).unwrap(), SEL_ADC0);
        assert!(b.adc0_owns(3, 2));
        assert!(
            b.adc0_owns(2, 4),
            "C and D share CDBUSALLOC — an even pin on C too"
        );
        assert!(!b.adc0_owns(3, 3), "the odd nibble is still tristate");
    }

    /// ⚠️ `xODD0` is at bit 16, not 8. A shift of 8 would allocate the SECOND
    /// bus's even half and leave every odd pad unconnected.
    #[test]
    fn odd_pins_take_the_field_at_shift_sixteen() {
        let mut b = Efr32s2BusAlloc::new();
        b.write_u32(OFF_ABUSALLOC, SEL_ADC0 << 16).unwrap();
        assert!(b.adc0_owns(0, 5), "PA05 is odd");
        assert!(!b.adc0_owns(0, 4), "PA04 is even and still tristate");
        b.write_u32(OFF_ABUSALLOC, SEL_ADC0 << 8).unwrap();
        assert!(!b.adc0_owns(0, 5), "bit 8 is AEVEN1, not AODD0");
    }

    /// Each port reads its own register — PB's allocation says nothing about
    /// PA.
    #[test]
    fn ports_a_and_b_have_their_own_registers() {
        let mut b = Efr32s2BusAlloc::new();
        b.write_u32(OFF_BBUSALLOC, SEL_ADC0).unwrap();
        assert!(b.adc0_owns(1, 0));
        assert!(!b.adc0_owns(0, 0));
        assert!(!b.adc0_owns(2, 0));
    }

    /// A selector other than ADC0 (the header defines ACMP/VDAC/… values too)
    /// is stored but does not connect the IADC.
    #[test]
    fn a_selector_that_is_not_adc0_does_not_connect_the_iadc() {
        let mut b = Efr32s2BusAlloc::new();
        b.write_u32(OFF_ABUSALLOC, 0x3).unwrap();
        assert_eq!(b.read_u32(OFF_ABUSALLOC).unwrap(), 0x3);
        assert!(!b.adc0_owns(0, 0));
    }

    /// Every word reads back what firmware wrote, byte by byte, so a driver's
    /// read-modify-write keeps the bits it did not touch.
    #[test]
    fn every_word_reads_back_what_was_written() {
        let mut b = Efr32s2BusAlloc::new();
        for w in 0..BUSALLOC_WORDS as u64 {
            b.write_u32(w * 4, 0x0301_0201).unwrap();
            assert_eq!(b.read_u32(w * 4).unwrap(), 0x0301_0201, "word {w}");
            assert_eq!(b.read(w * 4 + 2).unwrap(), 0x01, "byte 2 of word {w}");
        }
    }

    /// The window is three words. Past it is not a register and must not
    /// alias onto one, and a port past D owns nothing.
    #[test]
    fn past_the_window_decodes_to_nothing() {
        let mut b = Efr32s2BusAlloc::new();
        b.write_u32(0x0C, 0xFFFF_FFFF).unwrap();
        assert_eq!(b.read_u32(0x0C).unwrap(), 0);
        assert_eq!(b.read_u32(OFF_ABUSALLOC).unwrap(), 0);
        assert!(!b.adc0_owns(4, 0));
    }
}
