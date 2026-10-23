// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! dsPIC33F/PIC24 GPIO port (TRISx / PORTx / LATx / ODCx layout).
//!
//! Register semantics pinned to the dsPIC33F FRM §30 "I/O Ports" (DS70190D):
//!
//! * a write to `PORTx` writes the port data latch (the same as a write to
//!   `LATx`); a read of `PORTx` reads the pin, a read of `LATx` the latch
//!   (§30.2.2, §30.2.3);
//! * `ODCx` set makes the pin open-drain: it drives a 0 and releases for a 1;
//! * reset: every pad is an input (TRISx all ones for the pads the package
//!   has, DS70283H Table 4-20/21/22).
//!
//! The PIC24 core owns the port registers in its data-space SFR map and
//! mirrors `TRIS`/`LAT`/`ODC` writes to this model through the chip's mirror
//! window (`PIC24_IO_MIRROR_BASE + SFR address`, see `cpu::pic24`). The
//! mirror carries two registers that are not silicon:
//!
//! * [`OFF_PDRV`] — the level a peripheral unit (the Output Compare units)
//!   drives on each pad;
//! * [`OFF_PDRV_MASK`] — the pads that unit owns (it takes the driver over
//!   from the GPIO output stage, so `LAT` no longer reaches the pad).
//!
//! Pad-level truth lives here: the core samples `PORTx` through this model
//! each step (input-capture edges, `PORTx` reads) and asks the bus when the
//! outside world changes a pad ([`PinPort::set_external`], a board button, a
//! `gpio_net`).

use crate::peripherals::gpio::{GpioMode, GpioPortOffsets, GpioRouting};
use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{Peripheral, SimResult};

/// `TRISx` — offset 0 of the window (the window base is the `TRISx` address).
pub const OFF_TRIS: u64 = 0x00;
/// `PORTx` — reads the pin.
pub const OFF_PORT: u64 = 0x02;
/// `LATx` — reads the latch.
pub const OFF_LAT: u64 = 0x04;
/// `ODCx` — open-drain enable.
pub const OFF_ODC: u64 = 0x06;
/// Peripheral-unit drive levels (Output Compare output); not a register.
pub const OFF_PDRV: u64 = 0x08;
/// Pads a peripheral unit owns; not a register.
pub const OFF_PDRV_MASK: u64 = 0x0A;
/// Bytes this model answers.
pub const WINDOW_BYTES: u64 = 0x0C;

/// One dsPIC33F/PIC24 I/O port.
#[derive(Debug)]
pub struct Pic24GpioPort {
    /// Pads this port has on the package (bits present on silicon).
    mask: u16,
    /// Highest pad number + 1, for [`PinPort::pin_count`].
    pin_count: u8,
    tris: u16,
    lat: u16,
    odc: u16,
    /// The level a peripheral unit drives per pad ([`OFF_PDRV`]).
    pdrv: u16,
    /// Pads a peripheral unit owns ([`OFF_PDRV_MASK`]).
    pdrv_mask: u16,
    /// Pads the outside world holds ([`PinPort::set_external`]).
    ext_mask: u16,
    /// The level the outside world holds each held pad at.
    ext_level: u16,
    /// `Some` while the logic analyzer watches pads on this port in push mode.
    watch: Option<crate::pins::PadWatch>,
    /// Level cells kept equal to a pad (see `Peripheral::watch_pad_level`).
    cells: Vec<(u8, std::sync::Arc<std::sync::atomic::AtomicBool>)>,
}

impl Default for Pic24GpioPort {
    fn default() -> Self {
        Self::new(0xFFFF)
    }
}

impl Pic24GpioPort {
    /// A port whose package presents `mask` pads (e.g. `0x079F` for PORTA on
    /// the 44-pin dsPIC33FJ32MC204). `TRIS` resets to all ones over the mask.
    pub fn new(mask: u16) -> Self {
        let pin_count = if mask == 0 {
            0
        } else {
            (16 - mask.leading_zeros()) as u8
        };
        Self {
            mask,
            pin_count,
            tris: mask,
            lat: 0,
            odc: 0,
            pdrv: 0,
            pdrv_mask: 0,
            ext_mask: 0,
            ext_level: 0,
            watch: None,
            cells: Vec::new(),
        }
    }

    /// The pads this port presents.
    pub fn mask(&self) -> u16 {
        self.mask
    }

    /// The output stage: `(level bits, driven bits)`.
    ///
    /// A peripheral unit beats the GPIO stage (it owns the pad); a push-pull
    /// `LAT` bit beats nothing; an open-drain `LAT` 1 is released.
    fn drive(&self) -> (u16, u16) {
        let own = !self.tris & self.mask;
        let push = own & !(self.odc & self.lat);
        let levels = (self.lat & push & !self.pdrv_mask) | (self.pdrv & self.pdrv_mask);
        let driven = (push | self.pdrv_mask) & self.mask;
        (levels, driven)
    }

    /// The live pad level (`PORTx`): driven bits from the chip's own stages,
    /// everything else what the outside world holds.
    fn pad_bits(&self) -> u16 {
        let (levels, driven) = self.drive();
        let ext = self.ext_level & self.ext_mask;
        ((levels & driven) | (ext & !driven)) & self.mask
    }

    /// Publish the pad level into every watching cell.
    fn sync_cells(&self) {
        let bits = self.pad_bits();
        for (pin, cell) in &self.cells {
            cell.store(
                bits & (1u16 << pin) != 0,
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

    /// Read a 16-bit register by its (even) window offset.
    fn reg(&self, off: u64) -> u16 {
        match off & !1 {
            OFF_TRIS => self.tris,
            OFF_PORT => self.pad_bits(),
            OFF_LAT => self.lat,
            OFF_ODC => self.odc,
            OFF_PDRV => self.pdrv,
            OFF_PDRV_MASK => self.pdrv_mask,
            _ => 0,
        }
    }

    /// One byte of a write: the mirror writes registers byte-wise
    /// (`Bus::write_u16` is two `write_u8` calls, low byte first).
    fn write_reg_byte(&mut self, off: u64, value: u8) {
        let reg = off & !1;
        let shift = (off & 1) * 8;
        let merged = |cur: u16| (cur & !(0xFF << shift)) | (u16::from(value) << shift);
        match reg {
            OFF_TRIS => self.tris = merged(self.tris),
            // Writing PORTx writes the port data latch (DS70190D §30.2.2).
            OFF_PORT | OFF_LAT => self.lat = merged(self.lat),
            OFF_ODC => self.odc = merged(self.odc),
            OFF_PDRV => self.pdrv = merged(self.pdrv),
            OFF_PDRV_MASK => self.pdrv_mask = merged(self.pdrv_mask),
            _ => {}
        }
    }
}

impl PinPort for Pic24GpioPort {
    fn pin_count(&self) -> u8 {
        self.pin_count
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if pin >= 16 || self.mask & (1u16 << pin) == 0 {
            return None;
        }
        let bit = 1u16 << pin;
        Some(if self.pdrv_mask & bit != 0 {
            PadDriver::drive(self.pdrv & bit != 0)
        } else if self.tris & bit == 0 {
            if self.odc & bit != 0 && self.lat & bit != 0 {
                // Open-drain holding a 1: released.
                PadDriver::OFF
            } else {
                PadDriver::drive(self.lat & bit != 0)
            }
        } else {
            PadDriver::OFF
        })
    }

    fn external(&self, pin: u8) -> External {
        let bit = 1u16 << (pin & 15);
        if pin < 16 && self.ext_mask & bit != 0 {
            External::Level(self.ext_level & bit != 0)
        } else {
            External::Released
        }
    }

    /// A level lands in the pad and in the `PORTx` read. The bit is held
    /// regardless of `TRIS` (the wiring keeps the level while the pad is
    /// reconfigured), the same seam the AVR port exposes.
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= 16 || self.mask & (1u16 << pin) == 0 {
            return None;
        }
        let bit = 1u16 << pin;
        let before = self.pad_bits() & bit != 0;
        self.mutate_pads(|s| match ext {
            External::Level(level) => {
                s.ext_mask |= bit;
                if level {
                    s.ext_level |= bit;
                } else {
                    s.ext_level &= !bit;
                }
            }
            External::Released => s.ext_mask &= !bit,
        });
        Some(InputChange {
            before,
            after: self.pad_bits() & bit != 0,
        })
    }

    /// The firmware-visible input level of `pin` (`PORTx`).
    fn input(&self, pin: u8) -> Option<bool> {
        // `then_some` evaluates its argument eagerly, so the shift must only
        // happen inside the guarded branch (pin >= 16 would overflow).
        if pin < 16 && self.mask & (1u16 << pin) != 0 {
            Some(self.pad_bits() & (1u16 << pin) != 0)
        } else {
            None
        }
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

impl Peripheral for Pic24GpioPort {
    /// TRIS/PORT/LAT/ODC move only by `read`/`write`; no tick behaviour.
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
        let reg = self.reg(offset);
        Ok(if offset & 1 == 0 {
            (reg & 0xFF) as u8
        } else {
            (reg >> 8) as u8
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.mutate_pads(|s| s.write_reg_byte(offset, value));
        Ok(())
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= 16 || self.mask & (1u16 << pin) == 0 {
            return None;
        }
        let bit = 1u16 << pin;
        if self.pdrv_mask & bit != 0 {
            return Some(self.pdrv & bit != 0);
        }
        (self.tris & bit == 0).then_some(self.lat & bit != 0)
    }

    fn gpio_port_offsets(&self) -> Option<GpioPortOffsets> {
        Some(GpioPortOffsets {
            output: OFF_LAT,
            input: OFF_PORT,
        })
    }

    /// `TRISx` is the direction register: a set bit makes the pad an input.
    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        (pin < 16 && self.mask & (1u16 << pin) != 0).then_some(self.tris & (1u16 << pin) == 0)
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
        if pin >= 16 || self.mask & (1u16 << pin) == 0 {
            return false;
        }
        cell.store(
            self.pad_bits() & (1u16 << pin) != 0,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.cells.push((pin, cell));
        true
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "tris": self.tris,
            "lat": self.lat,
            "odc": self.odc,
            "pdrv": self.pdrv,
            "pdrv_mask": self.pdrv_mask,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_is_all_inputs() {
        let p = Pic24GpioPort::new(0x03FF);
        assert_eq!(p.tris, 0x03FF, "TRISC resets 0x03FF on this package");
        assert_eq!(p.driver(0), Some(PadDriver::OFF));
        assert_eq!(p.read_gpio_is_output(0), Some(false));
        assert_eq!(
            p.read_gpio_is_output(10),
            None,
            "RC10 is not on the package"
        );
        assert_eq!(p.mask(), 0x03FF);
        assert_eq!(p.pin_count(), 10);
    }

    #[test]
    fn push_pull_output_drives_the_latch() {
        let mut p = Pic24GpioPort::new(0xFFFF);
        p.write(OFF_TRIS, 0x00).unwrap(); // all outputs
        assert_eq!(p.driver(3), Some(PadDriver::drive(false)));
        p.write(OFF_LAT, 1 << 3).unwrap();
        assert_eq!(p.driver(3), Some(PadDriver::drive(true)));
        assert_eq!(p.read_gpio_pad(3), Some(true));
    }

    #[test]
    fn port_write_writes_the_latch_like_lat() {
        let mut p = Pic24GpioPort::new(0xFFFF);
        p.write(OFF_TRIS, 0x00).unwrap();
        p.write(OFF_PORT, 1 << 4).unwrap();
        assert_eq!(p.read(OFF_LAT).unwrap(), 1 << 4, "PORT write reached LAT");
    }

    #[test]
    fn open_drain_one_is_released() {
        let mut p = Pic24GpioPort::new(0xFFFF);
        p.write(OFF_TRIS, 0x00).unwrap();
        p.write(OFF_ODC, 1 << 2).unwrap();
        p.write(OFF_LAT, 1 << 2).unwrap();
        assert_eq!(p.driver(2), Some(PadDriver::OFF), "open-drain 1 released");
        p.write(OFF_LAT, 0).unwrap();
        assert_eq!(p.driver(2), Some(PadDriver::drive(false)));
    }

    #[test]
    fn external_level_reads_back_on_an_input() {
        let mut p = Pic24GpioPort::new(0x079F);
        assert_eq!(p.input(1), Some(false));
        let change = p.set_external(1, External::Level(true)).unwrap();
        assert!(change.changed());
        assert_eq!(p.input(1), Some(true));
        assert_eq!(p.read(OFF_PORT).unwrap() & (1 << 1), 1 << 1);
        // Releasing hands the pad back to floating (keeps the last level).
        p.set_external(1, External::Released).unwrap();
        assert_eq!(p.external(1), External::Released);
        // RA5 is not on the package: refused, not silently dropped.
        assert!(p.set_external(5, External::Level(true)).is_none());
        assert_eq!(p.input(5), None);
    }

    #[test]
    fn peripheral_drive_beats_the_lat() {
        let mut p = Pic24GpioPort::new(0xFFFF);
        p.write(OFF_TRIS, 0x00).unwrap();
        p.write(OFF_LAT, 0).unwrap();
        p.write(OFF_PDRV_MASK, 1 << 0).unwrap();
        p.write(OFF_PDRV, 1 << 0).unwrap();
        assert_eq!(p.driver(0), Some(PadDriver::drive(true)));
        assert_eq!(p.read(OFF_PORT).unwrap() & 1, 1, "PORT shows the unit");
        // Releasing the unit hands the pad back to the GPIO stage.
        p.write(OFF_PDRV_MASK, 0).unwrap();
        assert_eq!(p.driver(0), Some(PadDriver::drive(false)));
    }
}
