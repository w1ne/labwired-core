// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ATmega-style 8-bit GPIO port (PINx / DDRx / PORTx layout).
//!
//! Parking model so `--watch-gpio portb:5` can observe Arduino Nano
//! `LED_BUILTIN` (PB5). The AVR interpreter owns the same registers in its
//! data-space IO map and mirrors writes through `bus.write_u8`, which lands
//! here when the chip yaml maps the port window.

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
        }
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
            OFF_PIN => {
                // Writing 1 to PIN toggles PORT (AVR toggle-on-write-1).
                self.port ^= value;
            }
            OFF_DDR => self.ddr = value,
            OFF_PORT => self.port = value,
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

    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        // Driven outputs report PORT; undriven pads read as low.
        if pin >= 8 {
            return None;
        }
        let bit = 1u8 << pin;
        if self.ddr & bit != 0 {
            Some(self.port & bit != 0)
        } else {
            Some(self.pin & bit != 0)
        }
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        self.read_gpio_pad(pin)
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
}
