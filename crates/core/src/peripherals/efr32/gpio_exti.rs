// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Silicon Labs EFR32 Series-2 **GPIO external interrupts** — the block behind
//! `attachInterrupt`.
//!
//! # Where this lives and why it is its own peripheral
//!
//! The EXTI registers are in the GPIO block head (`GPIO_S_BASE + 0x400`), far
//! above the four port structs at `+0x30`.. that
//! [`crate::peripherals::gpio::GpioPort`] models. Modelling them here, as a
//! window of their own, keeps that working port model untouched — the
//! alternative was folding four live board_io-bound port windows into one
//! block-wide peripheral, which is a refactor with nothing to gain.
//!
//! Before this, the whole `0x400`.. range was unmapped: a firmware that
//! configured a pin interrupt faulted the bus.
//!
//! # The mux, from the reference manual
//!
//! EFR32xG26 RM rev 1.0 §24.3.10.1 "Standard Interrupt Generation", including
//! its figure, states the rule exactly:
//!
//! ```text
//!   p = 4 * int( n / 4 )        and the mux selects PA[p+3:p] / PB[p+3:p] / …
//! ```
//!
//! So external interrupt line `n`:
//!
//! * takes its PORT from `EXTIPSEL[n]` — 2 bits, 0=PORTA … 3=PORTD;
//! * takes its PIN from `p + EXTIPINSEL[n]`, where `p = 4*(n/4)` and
//!   `EXTIPINSEL[n]` is 2 bits.
//!
//! ⚠️ That means a line canNOT watch an arbitrary pin. EXTI1 can watch pins
//! 0..3 of any port and nothing else; watching PB07 needs a line in 4..7. A
//! model that let any line watch any pin would accept firmware the silicon
//! rejects, and the mistake would only show up on the bench.
//!
//! Both fields are 4 bits wide in the register with only the low 2 used, and
//! lines 0..7 live in the `…L` register, 8..15 in `…H`.
//!
//! # Faithfully modelled
//!
//! * The port/pin mux above, including the group-of-four restriction.
//! * Edge selection: `EXTIRISE[n]` and `EXTIFALL[n]` independently, so a line
//!   armed for neither never fires and a line armed for both fires on any
//!   change.
//! * `IF[n]` latches and is write-1-to-clear; `IEN[n]` gates the interrupt.
//! * The split vector: even lines raise `GPIO_EVEN_IRQn` (40), odd lines
//!   `GPIO_ODD_IRQn` (39) — CMSIS `efr32mg26b510f3200im48.h`. Firmware
//!   installs two different handlers and a model that raised one line for both
//!   would run the wrong one.
//! * A flag latches even with `IEN[n]` clear, which is what lets firmware poll
//!   `IF` instead of taking an interrupt.
//!
//! # Idealised — present, but not physical
//!
//! * **No `GPIO_LOCK`.** The RM (§24.3.11) says writing anything but `0xA534`
//!   to `GPIO_LOCK` locks the configuration registers, EXTIPSEL among them.
//!   That register is in the same block but not this window, and the lock is
//!   not enforced here — firmware that locks the GPIO and then reconfigures a
//!   line is accepted where silicon would ignore it.
//! * **No EM4 wake-up.** `EM4WUEN`/`EM4WUPOL` decode and store; there are no
//!   energy modes to wake from.
//! * **No PRS.** A GPIO edge is a PRS producer on silicon; nothing consumes it
//!   here.
//! * **No asynchronous sense.** Edges are observed on the bus's GPIO snapshot,
//!   so an edge narrower than one snapshot interval is not seen. A real EXTI
//!   latches asynchronously and would catch it.

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// Window base: `GPIO_S_BASE + 0x400`. Offsets below are relative to THAT, not
/// to the GPIO block, so `0x00` here is `GPIO_EXTIPSELL`.
const OFF_EXTIPSELL: u64 = 0x00;
const OFF_EXTIPSELH: u64 = 0x04;
const OFF_EXTIPINSELL: u64 = 0x08;
const OFF_EXTIPINSELH: u64 = 0x0C;
const OFF_EXTIRISE: u64 = 0x10;
const OFF_EXTIFALL: u64 = 0x14;
const OFF_IF: u64 = 0x20;
const OFF_IEN: u64 = 0x24;
const OFF_EM4WUEN: u64 = 0x2C;
const OFF_EM4WUPOL: u64 = 0x30;

/// `GPIO_EVEN_IRQn` and `GPIO_ODD_IRQn` — CMSIS `IRQn_Type`.
const IRQ_GPIO_ODD: u32 = 39;
const IRQ_GPIO_EVEN: u32 = 40;

/// External interrupt lines on this part.
const EXTI_LINES: u8 = 16;

/// Both `EXTIPSEL` and `EXTIPINSEL` pack one 4-bit field per line, of which
/// the low 2 bits are used.
const FIELD_BITS: u32 = 4;
const FIELD_MASK: u32 = 0x3;

/// EFR32 Series-2 GPIO external interrupts.
#[derive(Debug)]
pub struct Efr32s2GpioExti {
    extipsel: [u32; 2],
    extipinsel: [u32; 2],
    extirise: u32,
    extifall: u32,
    iflag: u32,
    ien: u32,
    em4wuen: u32,
    em4wupol: u32,
    /// Last level seen per (port, pin), so an edge is a CHANGE and not a
    /// level. Index: `port * 16 + pin`.
    last_level: [Option<bool>; 64],
}

impl Default for Efr32s2GpioExti {
    fn default() -> Self {
        Self::new()
    }
}

impl Efr32s2GpioExti {
    pub fn new() -> Self {
        Self {
            extipsel: [0; 2],
            extipinsel: [0; 2],
            extirise: 0,
            extifall: 0,
            iflag: 0,
            ien: 0,
            em4wuen: 0,
            em4wupol: 0,
            last_level: [None; 64],
        }
    }

    /// The `(port, pin)` line `n` watches, per RM §24.3.10.1.
    fn watched_pad(&self, line: u8) -> (u8, u8) {
        let half = (line / 8) as usize;
        let shift = ((line % 8) as u32) * FIELD_BITS;
        let port = ((self.extipsel[half] >> shift) & FIELD_MASK) as u8;
        let within = ((self.extipinsel[half] >> shift) & FIELD_MASK) as u8;
        // p = 4 * int(n / 4): the line's group of four fixes the high bits of
        // the pin, and EXTIPINSEL only chooses within that group.
        let pin = 4 * (line / 4) + within;
        (port, pin)
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OFF_EXTIPSELL => self.extipsel[0],
            OFF_EXTIPSELH => self.extipsel[1],
            OFF_EXTIPINSELL => self.extipinsel[0],
            OFF_EXTIPINSELH => self.extipinsel[1],
            OFF_EXTIRISE => self.extirise,
            OFF_EXTIFALL => self.extifall,
            OFF_IF => self.iflag,
            OFF_IEN => self.ien,
            OFF_EM4WUEN => self.em4wuen,
            OFF_EM4WUPOL => self.em4wupol,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            OFF_EXTIPSELL => self.extipsel[0] = value,
            OFF_EXTIPSELH => self.extipsel[1] = value,
            OFF_EXTIPINSELL => self.extipinsel[0] = value,
            OFF_EXTIPINSELH => self.extipinsel[1] = value,
            OFF_EXTIRISE => self.extirise = value,
            OFF_EXTIFALL => self.extifall = value,
            // Write-1-to-clear, the Series-2 convention. `GPIO_IF_CLR` (the
            // `+0x2000` alias) reaches the same path through the bus's alias
            // fold, which is how `GPIO_IntClear` actually spells it.
            OFF_IF => self.iflag &= !value,
            OFF_IEN => self.ien = value,
            OFF_EM4WUEN => self.em4wuen = value,
            OFF_EM4WUPOL => self.em4wupol = value,
            _ => {}
        }
    }

    /// Which NVIC line a flagged EXTI line raises.
    fn irq_for(line: u8) -> u32 {
        if line % 2 == 0 {
            IRQ_GPIO_EVEN
        } else {
            IRQ_GPIO_ODD
        }
    }
}

impl Peripheral for Efr32s2GpioExti {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        let merged = (self.read_word(reg) & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    /// Pure register state between edges; the work happens in
    /// `observe_gpio_change` and `tick_elapsed`, and neither needs a per-cycle
    /// visit of its own.
    fn needs_legacy_walk(&self) -> bool {
        true
    }

    /// The bus hands every GPIO transition here. This is the whole model: an
    /// edge on a pad a line watches, with the matching polarity armed, latches
    /// that line's flag.
    fn observe_gpio_change(&mut self, changes: &[(u8, u8, u8)]) -> bool {
        let mut latched = false;
        for &(port, pin, level) in changes {
            let idx = (port as usize) * 16 + (pin as usize);
            let new = level != 0;
            let prev = self.last_level.get(idx).copied().flatten();
            if let Some(slot) = self.last_level.get_mut(idx) {
                *slot = Some(new);
            }
            // With no previous level there is no edge — the first observation
            // establishes the baseline. Otherwise a pad that merely STARTS
            // high would look like a rising edge at boot.
            let Some(prev) = prev else { continue };
            if prev == new {
                continue;
            }
            let rising = new;

            for line in 0..EXTI_LINES {
                let (sel_port, sel_pin) = self.watched_pad(line);
                if sel_port != port || sel_pin != pin {
                    continue;
                }
                let armed = if rising {
                    self.extirise & (1 << line) != 0
                } else {
                    self.extifall & (1 << line) != 0
                };
                if !armed {
                    continue;
                }
                self.iflag |= 1 << line;
                latched = true;
            }
        }
        latched
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        let mut result = PeripheralTickResult::default();
        let active = self.iflag & self.ien & 0xFFFF;
        if active == 0 {
            return result;
        }
        // Even and odd lines are two different vectors, and a firmware installs
        // two different handlers. Raise exactly the ones that have a flag.
        let mut irqs = Vec::new();
        for line in 0..EXTI_LINES {
            if active & (1 << line) == 0 {
                continue;
            }
            let irq = Self::irq_for(line);
            if !irqs.contains(&irq) {
                irqs.push(irq);
            }
        }
        result.explicit_irqs = Some(irqs);
        result
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point line `n` at `(port, pin)`. Panics if the pin is not in the line's
    /// group of four — which is the silicon rule, and a test that got it wrong
    /// should say so rather than quietly watch a different pad.
    fn select(exti: &mut Efr32s2GpioExti, line: u8, port: u8, pin: u8) {
        assert_eq!(
            pin / 4,
            line / 4,
            "EXTI{line} cannot watch pin {pin}: p = 4*int(n/4) restricts it to \
             pins {}..{}",
            4 * (line / 4),
            4 * (line / 4) + 3
        );
        let half = (line / 8) as usize;
        let shift = ((line % 8) as u32) * FIELD_BITS;
        let off_psel = if half == 0 {
            OFF_EXTIPSELL
        } else {
            OFF_EXTIPSELH
        };
        let off_pinsel = if half == 0 {
            OFF_EXTIPINSELL
        } else {
            OFF_EXTIPINSELH
        };
        let psel = exti.read_word(off_psel) | ((port as u32) << shift);
        exti.write_word(off_psel, psel);
        let pinsel = exti.read_word(off_pinsel) | (((pin % 4) as u32) << shift);
        exti.write_word(off_pinsel, pinsel);
    }

    /// Establish the baseline level, then drive the edge.
    fn edge(exti: &mut Efr32s2GpioExti, port: u8, pin: u8, from: u8, to: u8) {
        exti.observe_gpio_change(&[(port, pin, from)]);
        exti.observe_gpio_change(&[(port, pin, to)]);
    }

    #[test]
    fn a_rising_edge_on_a_watched_pad_latches_its_line() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0); // EXTI0 watches PB00 (BTN0 on BRD2709A)
        exti.write_word(OFF_EXTIRISE, 1 << 0);

        edge(&mut exti, 1, 0, 0, 1);
        assert_eq!(exti.read_word(OFF_IF) & 1, 1);
    }

    #[test]
    fn a_line_armed_for_rising_ignores_a_falling_edge() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0);
        exti.write_word(OFF_EXTIRISE, 1 << 0);

        edge(&mut exti, 1, 0, 1, 0);
        assert_eq!(exti.read_word(OFF_IF) & 1, 0);
    }

    /// A button is active-low on this board, so the press is a FALLING edge —
    /// the one `attachInterrupt(..., FALLING)` arms.
    #[test]
    fn a_falling_edge_latches_when_extifall_is_armed() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0);
        exti.write_word(OFF_EXTIFALL, 1 << 0);

        edge(&mut exti, 1, 0, 1, 0);
        assert_eq!(exti.read_word(OFF_IF) & 1, 1);
    }

    #[test]
    fn a_line_armed_for_neither_edge_never_fires() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0);
        edge(&mut exti, 1, 0, 0, 1);
        edge(&mut exti, 1, 0, 1, 0);
        assert_eq!(exti.read_word(OFF_IF), 0);
    }

    /// The port select has to matter: PB00 and PA00 are different pads.
    #[test]
    fn an_edge_on_another_port_does_not_fire_the_line() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0); // PORTB
        exti.write_word(OFF_EXTIRISE, 1 << 0);

        edge(&mut exti, 0, 0, 0, 1); // PA00
        assert_eq!(exti.read_word(OFF_IF) & 1, 0);
    }

    /// The group-of-four rule, which is the part a model is most likely to get
    /// wrong: EXTI4 selects within pins 4..7, so `EXTIPINSEL4 = 1` is PIN5.
    #[test]
    fn the_pin_is_the_group_base_plus_extipinsel() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 4, 2, 5); // EXTI4, PORTC, pin 5 = group base 4 + 1
        assert_eq!(exti.watched_pad(4), (2, 5));
        exti.write_word(OFF_EXTIRISE, 1 << 4);

        edge(&mut exti, 2, 5, 0, 1);
        assert_eq!(exti.read_word(OFF_IF) & (1 << 4), 1 << 4);

        // ...and pin 1, which shares the EXTIPINSEL value but not the group,
        // must not fire it.
        exti.write_word(OFF_IF, 0xFFFF);
        edge(&mut exti, 2, 1, 0, 1);
        assert_eq!(exti.read_word(OFF_IF), 0);
    }

    #[test]
    fn lines_eight_and_up_use_the_high_registers() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 9, 3, 9); // EXTI9, PORTD, pin 9 = group base 8 + 1
        assert_eq!(exti.read_word(OFF_EXTIPSELL), 0, "L is for lines 0..7");
        assert_ne!(exti.read_word(OFF_EXTIPSELH), 0);
        exti.write_word(OFF_EXTIRISE, 1 << 9);

        edge(&mut exti, 3, 9, 0, 1);
        assert_eq!(exti.read_word(OFF_IF) & (1 << 9), 1 << 9);
    }

    #[test]
    fn the_flag_latches_without_ien_and_clears_on_write_one() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0);
        exti.write_word(OFF_EXTIRISE, 1 << 0);

        edge(&mut exti, 1, 0, 0, 1);
        assert_eq!(exti.read_word(OFF_IF) & 1, 1, "polling works without IEN");
        assert!(
            exti.tick_elapsed(1).explicit_irqs.is_none(),
            "IEN clear must not raise an interrupt"
        );

        exti.write_word(OFF_IF, 1);
        assert_eq!(exti.read_word(OFF_IF), 0);
    }

    /// Even and odd lines are two different vectors. A model that raised one
    /// for both runs the wrong handler.
    #[test]
    fn even_and_odd_lines_raise_their_own_vectors() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0); // even
        select(&mut exti, 1, 1, 1); // odd
        exti.write_word(OFF_EXTIRISE, (1 << 0) | (1 << 1));
        exti.write_word(OFF_IEN, 1 << 0);

        edge(&mut exti, 1, 0, 0, 1);
        assert_eq!(
            exti.tick_elapsed(1).explicit_irqs,
            Some(vec![IRQ_GPIO_EVEN]),
            "EXTI0 is even"
        );

        exti.write_word(OFF_IF, 0xFFFF);
        exti.write_word(OFF_IEN, 1 << 1);
        edge(&mut exti, 1, 1, 0, 1);
        assert_eq!(
            exti.tick_elapsed(1).explicit_irqs,
            Some(vec![IRQ_GPIO_ODD]),
            "EXTI1 is odd"
        );
    }

    /// A pad that is already high when first observed is not a rising edge.
    /// Without a baseline every pull-up would fire its line at boot.
    #[test]
    fn the_first_observation_establishes_a_baseline_and_is_not_an_edge() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0);
        exti.write_word(OFF_EXTIRISE, 1 << 0);

        exti.observe_gpio_change(&[(1, 0, 1)]);
        assert_eq!(exti.read_word(OFF_IF), 0);
    }
}
