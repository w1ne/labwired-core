// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ATmega-style 8-bit GPIO port (PINx / DDRx / PORTx layout).
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

#[derive(Debug)]
pub struct AvrGpioPort {
    pin: u8,
    ddr: u8,
    port: u8,
    /// Pads the outside world holds ([`PinPort::set_external`]); the level
    /// on each is its `pin` bit.
    ext_mask: u8,
    /// `Some` while the logic analyzer watches pads on this port in push mode
    /// ([`PinPort::install_watch`]). Not snapshot state.
    watch: Option<crate::pins::PadWatch>,
    /// Level cells kept equal to a pad (see `Peripheral::watch_pad_level`).
    cells: Vec<(u8, std::sync::Arc<std::sync::atomic::AtomicBool>)>,
}

impl Default for AvrGpioPort {
    fn default() -> Self {
        Self::new()
    }
}

impl AvrGpioPort {
    pub fn new() -> Self {
        Self {
            pin: 0,
            ddr: 0,
            port: 0,
            ext_mask: 0,
            watch: None,
            cells: Vec::new(),
        }
    }

    /// Pad level as `read_gpio_pad` reports it: PORT when DDR drives the
    /// bit, otherwise the externally held PIN bit.
    #[inline]
    fn pad_bits(&self) -> u8 {
        (self.port & self.ddr) | (self.pin & !self.ddr)
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
        8
    }

    /// `DDRx` set: an output driving `PORTx`. Clear: an input, with the
    /// internal pull-up on while its `PORTx` bit is set (ATmega328P datasheet
    /// §14.2.1). `MCUCR.PUD` lives in the CPU's IO space, which this port
    /// model does not see, so it is not applied here.
    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if pin >= 8 {
            return None;
        }
        let bit = 1u8 << pin;
        Some(if self.ddr & bit != 0 {
            PadDriver::drive(self.port & bit != 0)
        } else if self.port & bit != 0 {
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
    /// there, exactly as the wiring would keep it. Releasing keeps the last
    /// level in `PINx` (see [`level`](PinPort::level)).
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= 8 {
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
        (pin < 8).then(|| self.pad_bits() & (1u8 << pin) != 0)
    }

    /// `PORTx` on an output, `PINx` on an input. This model does not fold the
    /// pull-up into `PINx`: an input with the pull-up on and nothing outside
    /// reads the last level `PINx` held (0 from reset), where the shared rule
    /// would say 1. Kept so firmware reads exactly what it always did; the
    /// pull still shows in [`driver`](PinPort::driver), which is what a
    /// `gpio_net` counts.
    fn level(&self, pin: u8) -> Option<bool> {
        self.input(pin)
    }

    fn install_watch(&mut self, watch: Option<crate::pins::PadWatch>) -> bool {
        self.watch = watch;
        true
    }

    fn take_watch(&mut self) -> Option<crate::pins::PadWatch> {
        self.watch.take()
    }

    fn put_watch(&mut self, watch: crate::pins::PadWatch) {
        self.watch = Some(watch);
    }
}

impl Peripheral for AvrGpioPort {
    /// PIN/DDR/PORT are three bytes moved only by `read`/`write`. There is no
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
        Ok(match offset {
            OFF_PIN => {
                // Inputs float low in this minimal model; outputs read back PORT.
                (self.port & self.ddr) | (self.pin & !self.ddr)
            }
            OFF_DDR => self.ddr,
            OFF_PORT => self.port,
            _ => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            // Writing 1 to PIN toggles PORT (AVR toggle-on-write-1).
            OFF_PIN => self.mutate_pads(|s| s.port ^= value),
            OFF_DDR => self.mutate_pads(|s| s.ddr = value),
            OFF_PORT => self.mutate_pads(|s| s.port = value),
            _ => {}
        }
        Ok(())
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= 8 {
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
            output: OFF_PORT,
            input: OFF_PIN,
        })
    }

    /// `DDRx` is the direction register: a set bit makes the pad an output.
    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        (pin < 8).then(|| self.ddr & (1u8 << pin) != 0)
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        self.read_gpio_is_output(pin).map(|output| GpioRouting {
            mode: if output {
                GpioMode::Output
            } else {
                GpioMode::Input
            },
            func: None,
        })
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
        if pin >= 8 {
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

    /// Out of range is a REFUSAL, not a silent no-op: the button attach pass
    /// reads this return value to decide the contact is drivable at all.
    #[test]
    fn set_gpio_input_refuses_a_pin_outside_the_port() {
        let mut p = AvrGpioPort::new();
        assert!(p.set_gpio_input(7, true), "PB7 is the last pin of the port");
        assert!(!p.set_gpio_input(8, true));
        assert_eq!(p.read_gpio_input(8), None);
    }
}
