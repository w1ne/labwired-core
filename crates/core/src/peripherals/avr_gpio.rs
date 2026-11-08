// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! 8-bit AVR GPIO port: classic ATmega PINx / DDRx / PORTx, AVRxt VPORT
//! DIR / OUT / IN register ordering, or the full megaAVR 0-series PORT block.
//!
//! The megaAVR PORT layout (ATmega4808/4809 data sheet DS40002173 §16.5,
//! offsets from the Microchip ATDF): DIR 0x00, DIRSET/CLR/TGL 0x01..0x03,
//! OUT 0x04, OUTSET/CLR/TGL 0x05..0x07, IN 0x08 (writing 1 toggles OUT),
//! INTFLAGS 0x09, PORTCTRL 0x0A and PINnCTRL 0x10..0x17. PINnCTRL.PULLUPEN
//! is the pull-up; ISC = INPUT_DISABLE turns the digital input buffer off, so
//! IN reads 0 there. Pin-change / edge interrupts (ISC 1..3, 5) and INVEN are
//! not modelled: a PINnCTRL write that asks for them is refused. The CPU
//! aliases VPORTn onto DIR / OUT / IN / INTFLAGS of this block.
//!
//! A megaAVR pad can also carry a timer waveform output: with the pad an
//! output (DIR set) and the timer driving its line (TCA0 CTRLB.CMPnEN), the
//! waveform overrides OUT (DS40002173 §20.3.3.4), and IN, the logic analyzer
//! and push capture read the waveform. The bus binds the routes
//! (`SystemBus::wire_megaavr_tca_output_pads`), gated on PORTMUX.TCAROUTEA.
//!
//! Parking model so `--watch-gpio portb:5` can observe Arduino Nano
//! `LED_BUILTIN` (PB5). The AVR interpreter owns the same registers in its
//! data-space IO map and mirrors writes through `bus.write_u8`, which lands
//! here when the chip yaml maps the port window.

use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::pins::{External, InputChange, PadDriver, PinPort, Pull};
use crate::{Peripheral, SimResult};

/// Offsets relative to the port base (PINB @ 0x23 ⇒ base 0x23).
const OFF_PIN: u64 = 0;
const OFF_DDR: u64 = 1;
const OFF_PORT: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvrGpioRegisterLayout {
    #[default]
    Atmega,
    AvrxtVport,
    MegaAvrPort,
}

/// megaAVR PORT register offsets.
const MP_DIR: u64 = 0x00;
const MP_OUT: u64 = 0x04;
const MP_IN: u64 = 0x08;
const MP_INTFLAGS: u64 = 0x09;
const MP_PORTCTRL: u64 = 0x0A;
const MP_PINCTRL: u64 = 0x10;
const PINCTRL_PULLUPEN: u8 = 1 << 3;
const PINCTRL_INVEN: u8 = 1 << 7;
const PINCTRL_ISC_MASK: u8 = 0x07;
const PINCTRL_ISC_INTDISABLE: u8 = 0;
const PINCTRL_ISC_INPUT_DISABLE: u8 = 4;

#[derive(Debug)]
pub struct AvrGpioPort {
    layout: AvrGpioRegisterLayout,
    pin_count: u8,
    pin: u8,
    ddr: u8,
    port: u8,
    /// Pads the outside world holds ([`PinPort::set_external`]); the level
    /// on each is its `pin` bit.
    ext_mask: u8,
    /// `MCUCR.PUD` as the CPU last handed it over
    /// ([`PinPort::set_pull_ups_disabled`]): set, no pad has a pull-up. Not
    /// a register of this port.
    pud: bool,
    /// `Some` while the logic analyzer watches pads on this port in push mode
    /// ([`PinPort::install_watch`]). Not snapshot state.
    watch: Option<crate::pins::PadWatch>,
    /// Level cells kept equal to a pad (see `Peripheral::watch_pad_level`).
    cells: Vec<(u8, std::sync::Arc<std::sync::atomic::AtomicBool>)>,
    /// megaAVR `PINnCTRL` and `PORTCTRL` (unused by the other layouts).
    pinctrl: [u8; 8],
    portctrl: u8,
    /// Timer waveform outputs bound to these pads (megaAVR layout only).
    pad_routes: crate::peripherals::pad_routing::PadRoutes,
}

impl Default for AvrGpioPort {
    fn default() -> Self {
        Self::new()
    }
}

impl AvrGpioPort {
    pub fn new() -> Self {
        Self::with_layout(AvrGpioRegisterLayout::Atmega, 8)
    }

    /// Create an AVRxt VPORT block. VPORT exposes DIR, OUT and IN in that
    /// order; writes to IN are ignored (unlike classic AVR PINx toggle-on-1).
    pub fn new_avrxt_vport(pin_count: u8) -> Self {
        Self::with_layout(AvrGpioRegisterLayout::AvrxtVport, pin_count)
    }

    /// Create a megaAVR 0-series PORT block (`PORTx`, 0x20 bytes).
    pub fn new_megaavr_port(pin_count: u8) -> Self {
        Self::with_layout(AvrGpioRegisterLayout::MegaAvrPort, pin_count)
    }

    /// Create a classic AVR GPIO block with fewer than eight bonded pins.
    pub fn new_with_pin_count(pin_count: u8) -> Self {
        Self::with_layout(AvrGpioRegisterLayout::Atmega, pin_count)
    }

    fn with_layout(layout: AvrGpioRegisterLayout, pin_count: u8) -> Self {
        assert!((1..=8).contains(&pin_count), "AVR GPIO width must be 1..=8");
        Self {
            layout,
            pin_count,
            pin: 0,
            ddr: 0,
            port: 0,
            ext_mask: 0,
            pud: false,
            watch: None,
            cells: Vec::new(),
            pinctrl: [0; 8],
            portctrl: 0,
            pad_routes: crate::peripherals::pad_routing::PadRoutes::new(),
        }
    }

    /// The route selector of every pad: a pad can carry a peripheral line
    /// only while it is an output.
    #[inline]
    fn route_selector(ddr: u8) -> impl Fn(u8) -> Option<u32> {
        move |pin| (pin < 8 && ddr & (1u8 << pin) != 0).then_some(0)
    }

    /// The output pads a timer line drives now, and the levels it puts there.
    #[inline]
    fn routed(&self) -> (u8, u8) {
        if self.pad_routes.is_empty() {
            return (0, 0);
        }
        let (mut mask, mut level) = (0u8, 0u8);
        let select = Self::route_selector(self.ddr);
        for pin in 0..self.pin_count {
            if let Some(l) = self.pad_routes.level(pin, &select) {
                mask |= 1 << pin;
                if l {
                    level |= 1 << pin;
                }
            }
        }
        (mask, level)
    }

    /// Re-register watched pads with the lines that drive them now.
    fn sync_pad_line_taps(&mut self) {
        if self.pad_routes.is_empty() {
            return;
        }
        let Some(watch) = self.watch.take() else {
            return;
        };
        self.pad_routes
            .sync_taps(watch.tap(), watch.pairs(), Self::route_selector(self.ddr));
        self.watch = Some(watch);
    }

    /// Bits of the bonded pads.
    #[inline]
    fn bonded(&self) -> u8 {
        if self.pin_count >= 8 {
            0xFF
        } else {
            (1u8 << self.pin_count) - 1
        }
    }

    /// megaAVR pads whose digital input buffer is off (ISC = INPUT_DISABLE).
    fn input_disabled(&self) -> u8 {
        if self.layout != AvrGpioRegisterLayout::MegaAvrPort {
            return 0;
        }
        (0..8).fold(0u8, |acc, i| {
            if self.pinctrl[i] & PINCTRL_ISC_MASK == PINCTRL_ISC_INPUT_DISABLE {
                acc | (1 << i)
            } else {
                acc
            }
        })
    }

    fn megaavr_write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let v = value & self.bonded();
        match offset {
            MP_DIR => self.mutate_pads(|s| s.ddr = v),
            0x01 => self.mutate_pads(|s| s.ddr |= v),
            0x02 => self.mutate_pads(|s| s.ddr &= !v),
            0x03 => self.mutate_pads(|s| s.ddr ^= v),
            MP_OUT => self.mutate_pads(|s| s.port = v),
            0x05 => self.mutate_pads(|s| s.port |= v),
            0x06 => self.mutate_pads(|s| s.port &= !v),
            0x07 | MP_IN => self.mutate_pads(|s| s.port ^= v),
            // No pin interrupt is ever raised, so there is no flag to clear.
            MP_INTFLAGS => {}
            MP_PORTCTRL => self.portctrl = value & 0x01,
            o if (MP_PINCTRL..MP_PINCTRL + 8).contains(&o) => {
                let isc = value & PINCTRL_ISC_MASK;
                if value & PINCTRL_INVEN != 0
                    || !matches!(isc, PINCTRL_ISC_INTDISABLE | PINCTRL_ISC_INPUT_DISABLE)
                {
                    return Err(crate::SimulationError::NotImplemented(format!(
                        "megaAVR PIN{}CTRL {value:#04x}: pin interrupts and INVEN are \
                         not modelled",
                        o - MP_PINCTRL
                    )));
                }
                let i = (o - MP_PINCTRL) as usize;
                self.mutate_pads(|s| s.pinctrl[i] = value & (PINCTRL_PULLUPEN | PINCTRL_ISC_MASK));
            }
            _ => {}
        }
        Ok(())
    }

    fn megaavr_read(&self, offset: u64) -> u8 {
        match offset {
            MP_DIR..=0x03 => self.ddr,
            MP_OUT..=0x07 => self.port,
            MP_IN => self.pad_bits() & !self.input_disabled(),
            MP_PORTCTRL => self.portctrl,
            o if (MP_PINCTRL..MP_PINCTRL + 8).contains(&o) => {
                self.pinctrl[(o - MP_PINCTRL) as usize]
            }
            _ => 0,
        }
    }

    #[inline]
    fn input_register_offset(&self) -> u64 {
        match self.layout {
            AvrGpioRegisterLayout::Atmega => OFF_PIN,
            AvrGpioRegisterLayout::AvrxtVport => OFF_PORT,
            AvrGpioRegisterLayout::MegaAvrPort => MP_IN,
        }
    }

    #[inline]
    fn output_register_offset(&self) -> u64 {
        match self.layout {
            AvrGpioRegisterLayout::Atmega => OFF_PORT,
            AvrGpioRegisterLayout::AvrxtVport => OFF_DDR,
            AvrGpioRegisterLayout::MegaAvrPort => MP_OUT,
        }
    }

    /// Inputs with the pull-up on: `PORTx` set on a `DDRx`-clear pad, unless
    /// `MCUCR.PUD` disables every pull-up (ATmega328P datasheet §14.2.1,
    /// table 14-1).
    #[inline]
    fn pull_ups(&self) -> u8 {
        if self.pud {
            0
        } else if self.layout == AvrGpioRegisterLayout::MegaAvrPort {
            let pullen = (0..8).fold(0u8, |acc, i| {
                if self.pinctrl[i] & PINCTRL_PULLUPEN != 0 {
                    acc | (1 << i)
                } else {
                    acc
                }
            });
            pullen & !self.ddr
        } else {
            self.port & !self.ddr
        }
    }

    /// `PINx` as silicon presents it: PORT where DDR drives the pad; the
    /// pull-up's 1 on an input nothing outside holds; otherwise the level the
    /// outside last presented (a floating input keeps it).
    #[inline]
    fn pad_bits(&self) -> u8 {
        let from_pull = self.pull_ups() & !self.ext_mask;
        let bits = (self.port & self.ddr) | (self.pin & !self.ddr & !from_pull) | from_pull;
        let (mask, level) = self.routed();
        (bits & !mask) | (level & mask)
    }

    /// Publish the pad level into every watching cell.
    fn sync_cells(&self) {
        let bits = self.pad_bits();
        for (pin, cell) in &self.cells {
            cell.store(
                bits & (1u8 << pin) != 0,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }

    /// Run `mutate` bracketed for push capture, then publish the pad levels
    /// to the watching cells.
    #[inline]
    fn mutate_pads<R>(&mut self, mutate: impl FnOnce(&mut Self) -> R) -> R {
        crate::pins::watch_begin(self);
        let r = mutate(self);
        crate::pins::watch_end(self);
        if !self.cells.is_empty() {
            self.sync_cells();
        }
        r
    }
}

impl PinPort for AvrGpioPort {
    fn pin_count(&self) -> u8 {
        self.pin_count
    }

    /// `DDRx` set: an output driving `PORTx`. Clear: an input, with the
    /// internal pull-up on while its `PORTx` bit is set and `MCUCR.PUD` is
    /// clear (ATmega328P datasheet §14.2.1). The CPU owns `MCUCR` and hands
    /// PUD over through [`PinPort::set_pull_ups_disabled`].
    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if pin >= self.pin_count {
            return None;
        }
        let bit = 1u8 << pin;
        let (mask, level) = self.routed();
        Some(if mask & bit != 0 {
            PadDriver::drive(level & bit != 0)
        } else if self.ddr & bit != 0 {
            PadDriver::drive(self.port & bit != 0)
        } else if self.pull_ups() & bit != 0 {
            PadDriver::released(Pull::Up)
        } else {
            PadDriver::OFF
        })
    }

    fn external(&self, pin: u8) -> External {
        let bit = 1u8 << (pin & 7);
        if pin < 8 && self.ext_mask & bit != 0 {
            External::Level(self.pin & bit != 0)
        } else {
            External::Released
        }
    }

    /// A level lands in `PINx`, the input latch on this family — the register
    /// `digitalRead` reads and the one the outside world moves. Writing it
    /// through the MMIO `write` path instead would toggle PORT (AVR's
    /// write-1-to-PIN toggle), which moves the OUTPUT latch, so the external
    /// world needs this seam of its own.
    ///
    /// The bit is held regardless of DDR: firmware that reconfigures the pin
    /// as an output and later releases it must find the contact's level still
    /// there, exactly as the wiring would keep it. Releasing hands the pad
    /// back to its pull-up, or leaves a floating input at the last level.
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= self.pin_count {
            return None;
        }
        let bit = 1u8 << pin;
        let before = self.pad_bits() & bit != 0;
        self.mutate_pads(|s| match ext {
            External::Level(level) => {
                s.ext_mask |= bit;
                if level {
                    s.pin |= bit;
                } else {
                    s.pin &= !bit;
                }
            }
            External::Released => s.ext_mask &= !bit,
        });
        Some(InputChange {
            before,
            after: self.pad_bits() & bit != 0,
        })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        (pin < self.pin_count).then(|| self.pad_bits() & (1u8 << pin) != 0)
    }

    fn set_pull_ups_disabled(&mut self, disabled: bool) -> bool {
        if self.pud != disabled {
            self.pud = disabled;
            if !self.cells.is_empty() {
                self.sync_cells();
            }
        }
        true
    }

    fn install_watch(&mut self, watch: Option<crate::pins::PadWatch>) -> bool {
        match watch {
            None => {
                self.watch = None;
                self.pad_routes.clear_taps();
            }
            Some(watch) => {
                self.watch = Some(watch);
                // Seeded stale so the sync installs the current routing.
                self.pad_routes.invalidate_registrations();
                self.sync_pad_line_taps();
            }
        }
        true
    }

    fn routes_changed(&mut self) {
        self.sync_pad_line_taps();
    }

    fn take_watch(&mut self) -> Option<crate::pins::PadWatch> {
        self.watch.take()
    }

    fn put_watch(&mut self, watch: crate::pins::PadWatch) {
        self.watch = Some(watch);
    }
}

impl Peripheral for AvrGpioPort {
    /// The selected three-byte GPIO register block moves only by `read`/`write`. There is no
    /// `tick`/`tick_elapsed` override, so the walk calls the trait default,
    /// which returns `PeripheralTickResult::default()` — no IRQ, no DMA
    /// request, no mmio-write, no fired event, for every reachable state.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let (direction, output, input) = match self.layout {
            AvrGpioRegisterLayout::Atmega => (OFF_DDR, OFF_PORT, OFF_PIN),
            AvrGpioRegisterLayout::AvrxtVport => (OFF_PIN, OFF_DDR, OFF_PORT),
            AvrGpioRegisterLayout::MegaAvrPort => return Ok(self.megaavr_read(offset)),
        };
        Ok(match offset {
            // Outputs read back their latch; inputs read the outside/pull-up
            // level. The field ordering is selected by the register layout.
            a if a == input => self.pad_bits(),
            a if a == direction => self.ddr,
            a if a == output => self.port,
            _ => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match self.layout {
            AvrGpioRegisterLayout::Atmega => match offset {
                // Writing 1 to PIN toggles PORT (AVR toggle-on-write-1).
                OFF_PIN => self.mutate_pads(|s| s.port ^= value),
                OFF_DDR => self.mutate_pads(|s| s.ddr = value),
                OFF_PORT => self.mutate_pads(|s| s.port = value),
                _ => {}
            },
            AvrGpioRegisterLayout::AvrxtVport => match offset {
                OFF_PIN => self.mutate_pads(|s| s.ddr = value),
                OFF_DDR => self.mutate_pads(|s| s.port = value),
                // VPORT.IN is read-only; it does not have PINx toggle
                // semantics.
                _ => {}
            },
            AvrGpioRegisterLayout::MegaAvrPort => return self.megaavr_write(offset, value),
        }
        Ok(())
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= self.pin_count {
            return None;
        }
        let bit = 1u8 << pin;
        if self.ddr & bit == 0 {
            return None;
        }
        Some(self.port & bit != 0)
    }

    fn gpio_port_offsets(&self) -> Option<crate::peripherals::gpio::GpioPortOffsets> {
        Some(crate::peripherals::gpio::GpioPortOffsets {
            output: self.output_register_offset(),
            input: self.input_register_offset(),
        })
    }

    /// `DDRx` is the direction register: a set bit makes the pad an output.
    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        (pin < self.pin_count).then(|| self.ddr & (1u8 << pin) != 0)
    }

    /// A DDR-clear pad drives nothing, whatever `PORTx` holds.
    fn gpio_input_is_released(&self) -> bool {
        true
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        self.read_gpio_is_output(pin).map(|output| GpioRouting {
            mode: if output {
                GpioMode::Output
            } else {
                GpioMode::Input
            },
            func: self
                .pad_routes
                .func(pin, Self::route_selector(self.ddr))
                .map(String::from),
        })
    }

    /// Bind pad `pin` to TCA0 waveform output `line`: live while the pad is
    /// an output, the timer drives the line, and `gate` (PORTMUX.TCAROUTEA)
    /// routes TCA0 to this port. megaAVR PORT layout only.
    fn bind_remapped_timer_output_pad(
        &mut self,
        cell: &std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
        pin: u8,
        line: usize,
        func: &'static str,
        gate: crate::peripherals::pad_routing::RemapGate,
    ) -> bool {
        if self.layout != AvrGpioRegisterLayout::MegaAvrPort || pin >= self.pin_count {
            return false;
        }
        self.pad_routes
            .bind_when_driving_gated(cell, pin, None, line, func, Some(gate));
        true
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        PinPort::input(self, pin)
    }

    fn watch_pad_level(
        &mut self,
        pin: u8,
        cell: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> bool {
        if pin >= self.pin_count {
            return false;
        }
        cell.store(
            self.pad_bits() & (1u8 << pin) != 0,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.cells.push((pin, cell));
        true
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "pin": self.pin,
            "ddr": self.ddr,
            "port": self.port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn led_pin_tracks_port_when_ddr_out() {
        let mut p = AvrGpioPort::new();
        p.write(OFF_DDR, 1 << 5).unwrap();
        assert_eq!(p.read_gpio_pad(5), Some(false));
        p.write(OFF_PORT, 1 << 5).unwrap();
        assert_eq!(p.read_gpio_pad(5), Some(true));
        p.write(OFF_PORT, 0).unwrap();
        assert_eq!(p.read_gpio_pad(5), Some(false));
    }

    /// A `board_io` button drives its pin through `set_gpio_input`, and
    /// `attach_board_io_buttons` proves the level landed by reading it straight
    /// back. Without both halves the button is dropped as undrivable — which is
    /// what this port did before it had a `set_gpio_input` of its own.
    #[test]
    fn externally_driven_level_lands_in_pin_and_reads_back() {
        let mut p = AvrGpioPort::new();
        // PB2 left as an input (DDR bit clear): undriven it reads low.
        assert_eq!(p.read_gpio_input(2), Some(false));

        assert!(p.set_gpio_input(2, true), "PB2 must be drivable");
        assert_eq!(p.read(OFF_PIN).unwrap() & (1 << 2), 1 << 2, "PINB bit set");
        assert_eq!(p.read_gpio_input(2), Some(true));
        assert_eq!(p.read_gpio_pad(2), Some(true));

        // Releasing an active-low contact takes the pin back high→low here.
        assert!(p.set_gpio_input(2, false));
        assert_eq!(p.read(OFF_PIN).unwrap() & (1 << 2), 0);
        assert_eq!(p.read_gpio_input(2), Some(false));
    }

    /// A pin the firmware drives reads back its own PORT latch, so an external
    /// level on the same pad does not fake a set-then-confirm round-trip.
    #[test]
    fn output_pin_still_reads_its_own_port_latch() {
        let mut p = AvrGpioPort::new();
        p.set_gpio_input(5, true);
        p.write(OFF_DDR, 1 << 5).unwrap(); // PB5 becomes an output, driving low
        assert_eq!(p.read_gpio_pad(5), Some(false));
        assert_eq!(p.read(OFF_PIN).unwrap() & (1 << 5), 0);
        // Releasing the driver hands the pad back to the outside world.
        p.write(OFF_DDR, 0).unwrap();
        assert_eq!(p.read_gpio_pad(5), Some(true));
    }

    /// A circuit that also reports the level on a pad the firmware drives (the
    /// touch lab routes `in_pd4` for its send pin) writes PIN on an output.
    /// That must change nothing the firmware or a model reads for the pad:
    /// direction, latch and pad level all stay the driver's.
    #[test]
    fn an_input_level_on_an_output_pad_is_harmless() {
        let mut p = AvrGpioPort::new();
        p.write(OFF_DDR, 1 << 4).unwrap();
        assert!(p.set_gpio_input(4, true), "accepted, and held for later");
        assert_eq!(p.read_gpio_is_output(4), Some(true));
        assert_eq!(p.read_gpio_output(4), Some(false));
        assert_eq!(p.read_gpio_pad(4), Some(false));
        assert_eq!(p.read(OFF_PIN).unwrap() & (1 << 4), 0);
        assert_eq!(p.read(OFF_PORT).unwrap(), 0, "the latch is untouched");
    }

    /// Direction comes from DDR alone. PORT is the pull-up enable on an input,
    /// so a high latch on an input pad must still read as "not an output".
    #[test]
    fn direction_is_the_ddr_bit() {
        let mut p = AvrGpioPort::new();
        p.write(OFF_PORT, 0xFF).unwrap();
        assert_eq!(
            p.read_gpio_is_output(4),
            Some(false),
            "reset: every pad is an input"
        );
        p.write(OFF_DDR, 1 << 4).unwrap();
        assert_eq!(p.read_gpio_is_output(4), Some(true));
        assert_eq!(p.read_gpio_is_output(2), Some(false));
        p.write(OFF_DDR, 0).unwrap();
        assert_eq!(p.read_gpio_is_output(4), Some(false));
        assert_eq!(p.read_gpio_is_output(8), None, "an 8-bit port has no pad 8");
    }

    #[test]
    fn routing_and_snapshot_expose_avr_direction_and_pull_latch() {
        let mut p = AvrGpioPort::new();
        p.write(OFF_PORT, 1 << 2).unwrap(); // input pull-up on PB2
        assert_eq!(p.gpio_routing(2).unwrap().mode, GpioMode::Input);
        assert_eq!(
            p.snapshot(),
            serde_json::json!({ "pin": 0, "ddr": 0, "port": 4 })
        );

        p.write(OFF_DDR, 1 << 2).unwrap();
        assert_eq!(p.gpio_routing(2).unwrap().mode, GpioMode::Output);
        assert_eq!(p.gpio_routing(8), None);
    }

    /// `INPUT_PULLUP` with nothing attached reads HIGH: the pull-up drives
    /// the pad. A contact to ground wins over it, and letting go hands the
    /// pad back to it.
    #[test]
    fn a_pulled_up_input_reads_high_until_something_holds_it_low() {
        let mut p = AvrGpioPort::new();
        assert_eq!(p.read(OFF_PIN).unwrap(), 0, "reset: floating inputs read 0");
        p.write(OFF_PORT, 1 << 2).unwrap(); // pull-up on PB2
        assert_eq!(p.read(OFF_PIN).unwrap(), 1 << 2);
        assert_eq!(p.read_gpio_input(2), Some(true));
        assert_eq!(p.read_gpio_pad(2), Some(true));
        let change = p.set_external(2, External::Level(false)).unwrap();
        assert_eq!(
            change,
            InputChange {
                before: true,
                after: false
            }
        );
        let change = p.set_external(2, External::Released).unwrap();
        assert_eq!(
            change,
            InputChange {
                before: false,
                after: true
            }
        );
        // Turning the pull-up off leaves the pad floating at its last level.
        p.write(OFF_PORT, 0).unwrap();
        assert_eq!(p.read(OFF_PIN).unwrap() & (1 << 2), 0);
    }

    /// `MCUCR.PUD` disables every pull-up: the driver loses it (so a net no
    /// longer counts it) and PINx reads the floating level.
    #[test]
    fn pud_disables_every_pull_up() {
        let mut p = AvrGpioPort::new();
        p.write(OFF_PORT, 0xF0).unwrap();
        p.write(OFF_DDR, 0x80).unwrap(); // PB7 an output driving 1
        assert_eq!(p.read(OFF_PIN).unwrap(), 0xF0);
        assert_eq!(p.driver(4).unwrap().pull, Pull::Up);
        assert!(p.set_pull_ups_disabled(true));
        assert_eq!(p.driver(4).unwrap().pull, Pull::None);
        assert_eq!(p.read(OFF_PIN).unwrap(), 0x80, "only the output stays high");
        assert_eq!(
            p.driver(7),
            Some(PadDriver::drive(true)),
            "outputs keep driving"
        );
        assert!(p.set_pull_ups_disabled(false));
        assert_eq!(p.read(OFF_PIN).unwrap(), 0xF0);
    }

    /// Out of range is a REFUSAL, not a silent no-op: the button attach pass
    /// reads this return value to decide the contact is drivable at all.
    #[test]
    fn set_gpio_input_refuses_a_pin_outside_the_port() {
        let mut p = AvrGpioPort::new();
        assert!(p.set_gpio_input(7, true), "PB7 is the last pin of the port");
        assert!(!p.set_gpio_input(8, true));
        assert_eq!(p.read_gpio_input(8), None);
    }

    #[test]
    fn avrxt_vport_uses_dir_out_in_register_order_and_read_only_in() {
        let mut p = AvrGpioPort::new_avrxt_vport(4);

        p.write(0, 0b0001).unwrap(); // DIR
        p.write(1, 0b0001).unwrap(); // OUT
        assert_eq!(p.read(0).unwrap(), 0b0001);
        assert_eq!(p.read(1).unwrap(), 0b0001);
        assert_eq!(p.read(2).unwrap(), 0b0001);
        assert_eq!(p.read_gpio_output(0), Some(true));
        assert_eq!(p.gpio_port_offsets().unwrap().output, 1);
        assert_eq!(p.gpio_port_offsets().unwrap().input, 2);

        // VPORT.IN is read-only; writing it must not toggle the output latch.
        p.write(2, 0b0001).unwrap();
        assert_eq!(p.read(1).unwrap(), 0b0001);
        assert_eq!(p.read_gpio_output(0), Some(true));
    }

    #[test]
    fn avrxt_vport_refuses_unbonded_bits_from_pin_services() {
        let mut p = AvrGpioPort::new_avrxt_vport(4);
        assert_eq!(PinPort::pin_count(&p), 4);
        assert!(p.set_gpio_input(3, true));
        assert!(!p.set_gpio_input(4, true));
        assert_eq!(p.read_gpio_input(4), None);
        assert_eq!(p.read_gpio_is_output(4), None);
    }

    /// megaAVR PORT: SET/CLR/TGL strobes, IN-write toggle, PINnCTRL pull-up.
    #[test]
    fn megaavr_port_strobes_and_pullups() {
        let mut p = AvrGpioPort::new_megaavr_port(8);
        p.write(0x01, 1 << 2).unwrap(); // DIRSET PE2
        assert_eq!(p.read(0x00).unwrap(), 1 << 2);
        p.write(0x05, 1 << 2).unwrap(); // OUTSET
        assert_eq!(p.read_gpio_output(2), Some(true));
        p.write(0x07, 1 << 2).unwrap(); // OUTTGL
        assert_eq!(p.read_gpio_output(2), Some(false));
        p.write(0x08, 1 << 2).unwrap(); // writing 1 to IN toggles OUT
        assert_eq!(p.read(0x04).unwrap(), 1 << 2);
        p.write(0x06, 1 << 2).unwrap(); // OUTCLR
        assert_eq!(p.read(0x08).unwrap() & (1 << 2), 0);
        // PIN3CTRL.PULLUPEN: an input with nothing attached reads high.
        assert_eq!(p.read(0x08).unwrap() & (1 << 3), 0);
        p.write(0x13, PINCTRL_PULLUPEN).unwrap();
        assert_eq!(p.read(0x08).unwrap() & (1 << 3), 1 << 3);
        // INPUT_DISABLE turns the input buffer off.
        p.write(0x13, PINCTRL_PULLUPEN | PINCTRL_ISC_INPUT_DISABLE)
            .unwrap();
        assert_eq!(p.read(0x08).unwrap() & (1 << 3), 0);
    }

    #[test]
    fn megaavr_port_refuses_pin_interrupts() {
        let mut p = AvrGpioPort::new_megaavr_port(8);
        assert!(
            p.write(0x10, 0x03).is_err(),
            "ISC = FALLING is not modelled"
        );
        assert!(p.write(0x10, 0x80).is_err(), "INVEN is not modelled");
        assert_eq!(p.read(0x10).unwrap(), 0);
    }

    #[test]
    fn megaavr_port_masks_unbonded_pads() {
        let mut p = AvrGpioPort::new_megaavr_port(4);
        p.write(0x00, 0xFF).unwrap();
        assert_eq!(p.read(0x00).unwrap(), 0x0F);
    }
}
