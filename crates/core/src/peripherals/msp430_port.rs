// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Texas Instruments MSP430x2xx digital I/O (family user's guide SLAU144,
//! chapter 8).
//!
//! * [`Msp430Port`] (`msp430_port`): one 8-bit port. Two register layouts:
//!   * `interrupt: true` (P1, P2): `PxIN`, `PxOUT`, `PxDIR`, `PxIFG`,
//!     `PxIES`, `PxIE`, `PxSEL`, `PxREN` at offsets 0 to 7;
//!   * `interrupt: false` (P3 and up): `PxIN`, `PxOUT`, `PxDIR`, `PxSEL` at
//!     offsets 0 to 3.
//! * [`Msp430Bytes`] (`msp430_bytes`): byte registers that are storage with
//!   reset values. A descriptor uses it for the registers TI placed outside
//!   the port window (`PxSEL2`, and `PxREN` of the ports without interrupts)
//!   and for constants firmware reads (the DCO calibration bytes).
//!
//! Pad rules (SLAU144 8.2):
//!
//! * `PxDIR` 1 with `PxSEL` 0: the pad drives `PxOUT`.
//! * `PxDIR` 0 with `PxREN` 1: the pad is pulled, up for `PxOUT` 1 and down
//!   for `PxOUT` 0 (8.2.4).
//! * `PxSEL` 1: the pad belongs to a module function. This model does not
//!   know what the module drives, so the port reports no driver for it and
//!   `PxIN` reads what the outside world presents.
//! * `PxIN` reads the pad. A pad nothing drives and nothing pulls reads 0.
//! * `PxIFG` bit n is set when `PxIN` bit n moves in the direction `PxIES`
//!   selects (0: low to high, 1: high to low), whatever moved the pad: an
//!   outside level or a write to `PxOUT`, `PxDIR` or `PxREN` (8.2.7.1).
//!   Firmware can set and clear `PxIFG` bits. The port requests its line
//!   while any `PxIFG` bit with its `PxIE` bit is set; acceptance clears no
//!   flag (8.2.7).
//!
//! Limits:
//!
//! * A write to `PxIES` never sets a flag. Silicon can set one (8.2.7.2).
//! * `PxREN` of a port without interrupts sits outside the window
//!   ([`Msp430Bytes`]), so such a port has no pull.
//! * A PUC keeps `PxOUT` and `PxIES` (silicon leaves them unchanged); a
//!   power-on value of 0 is assumed.

use crate::peripherals::gpio::{GpioMode, GpioPortOffsets, GpioRouting};
use crate::peripherals::msp430::{Msp430Irq, SOURCES};
use crate::pins::{External, InputChange, PadDriver, PadWatch, PinPort, PortId, Pull};
use crate::{Peripheral, SimResult};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const OFF_IN: u64 = 0;
const OFF_OUT: u64 = 1;
const OFF_DIR: u64 = 2;
/// Layout with interrupts.
const OFF_IFG: u64 = 3;
const OFF_IES: u64 = 4;
const OFF_IE: u64 = 5;
const OFF_SEL: u64 = 6;
const OFF_REN: u64 = 7;
/// Layout without interrupts.
const OFF_SEL_BASIC: u64 = 3;

/// The interrupt request of one port: its source in [`Msp430Irq`] and the
/// line (vector `0xFFFE - 2 * line`) the source is routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRequest {
    pub source: u32,
    pub line: u32,
}

/// One MSP430x2xx digital I/O port.
#[derive(Debug)]
pub struct Msp430Port {
    /// P1/P2 layout (flag, edge select, enable and pull registers present).
    interrupt: bool,
    /// `PxSEL` after a PUC (0xC0 on a port that carries XIN/XOUT).
    sel_reset: u8,
    out: u8,
    dir: u8,
    ifg: u8,
    ies: u8,
    ie: u8,
    sel: u8,
    ren: u8,
    /// Pads the outside world holds, and the level it holds them at.
    ext_mask: u8,
    ext_level: u8,
    request: Option<PortRequest>,
    irq: Option<Arc<Msp430Irq>>,
    /// `Some` while the logic analyzer watches pads on this port in push mode.
    watch: Option<PadWatch>,
    /// Level cells kept equal to a pad (see `Peripheral::watch_pad_level`).
    cells: Vec<(u8, Arc<AtomicBool>)>,
}

impl Msp430Port {
    /// A port with the P1/P2 register layout. `request`: the interrupt
    /// source and line, `None` for a port whose vector the part does not
    /// have.
    pub fn with_interrupt(sel_reset: u8, request: Option<PortRequest>) -> Self {
        Self::build(true, sel_reset, request)
    }

    /// A port with the P3 layout: no flags, no pull register in the window.
    pub fn basic(sel_reset: u8) -> Self {
        Self::build(false, sel_reset, None)
    }

    fn build(interrupt: bool, sel_reset: u8, request: Option<PortRequest>) -> Self {
        Self {
            interrupt,
            sel_reset,
            out: 0,
            dir: 0,
            ifg: 0,
            ies: 0,
            ie: 0,
            sel: sel_reset,
            ren: 0,
            ext_mask: 0,
            ext_level: 0,
            request: request.filter(|r| r.source < SOURCES),
            irq: None,
            watch: None,
            cells: Vec::new(),
        }
    }

    /// Pads the GPIO output stage drives.
    fn own(&self) -> u8 {
        self.dir & !self.sel
    }

    /// The live pad levels (`PxIN`).
    fn pad_bits(&self) -> u8 {
        let own = self.own();
        let held = self.ext_mask & !own;
        let pulled = self.ren & !own & !held;
        (self.out & own) | (self.ext_level & held) | (self.out & pulled)
    }

    fn sync_request(&self) {
        if let (Some(irq), Some(request)) = (&self.irq, self.request) {
            irq.set_flag(request.source, self.ifg & self.ie != 0);
        }
    }

    fn sync_cells(&self) {
        let bits = self.pad_bits();
        for (pin, cell) in &self.cells {
            cell.store(bits >> pin & 1 != 0, Ordering::Relaxed);
        }
    }

    /// Run `mutate` bracketed for push capture, set the flags of the pads it
    /// moved in their selected direction, then publish the request and the
    /// pad levels.
    fn mutate_pads<R>(&mut self, mutate: impl FnOnce(&mut Self) -> R) -> R {
        crate::pins::watch_begin(self);
        let before = self.pad_bits();
        let r = mutate(self);
        let after = self.pad_bits();
        if self.interrupt {
            let rising = !before & after & !self.ies;
            let falling = before & !after & self.ies;
            self.ifg |= rising | falling;
        }
        crate::pins::watch_end(self);
        self.sync_request();
        if !self.cells.is_empty() {
            self.sync_cells();
        }
        r
    }

    fn in_range(pin: u8) -> bool {
        pin < 8
    }
}

impl PinPort for Msp430Port {
    fn pin_count(&self) -> u8 {
        8
    }

    fn port_name_prefix(&self) -> &'static str {
        "p"
    }

    /// Chip-yaml ids are `p1`, `p2`, …; port 1 is [`PortId`] 0.
    fn port_id(&self, name: &str) -> Option<PortId> {
        let n: u8 = name.to_ascii_lowercase().strip_prefix('p')?.parse().ok()?;
        (1..=16).contains(&n).then(|| PortId(n - 1))
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if !Self::in_range(pin) {
            return None;
        }
        let bit = 1u8 << pin;
        if self.sel & bit != 0 {
            // A module function owns the pad; its drive is not published.
            return None;
        }
        Some(if self.dir & bit != 0 {
            PadDriver::drive(self.out & bit != 0)
        } else if self.ren & bit != 0 {
            PadDriver::released(Pull::from_up(Some(self.out & bit != 0)))
        } else {
            PadDriver::OFF
        })
    }

    fn external(&self, pin: u8) -> External {
        if Self::in_range(pin) && self.ext_mask >> pin & 1 != 0 {
            External::Level(self.ext_level >> pin & 1 != 0)
        } else {
            External::Released
        }
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if !Self::in_range(pin) {
            return None;
        }
        let bit = 1u8 << pin;
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

    fn input(&self, pin: u8) -> Option<bool> {
        Self::in_range(pin).then(|| self.pad_bits() >> pin & 1 != 0)
    }

    fn install_watch(&mut self, watch: Option<PadWatch>) -> bool {
        self.watch = watch;
        true
    }

    fn take_watch(&mut self) -> Option<PadWatch> {
        self.watch.take()
    }

    fn put_watch(&mut self, watch: PadWatch) {
        self.watch = Some(watch);
    }
}

impl Peripheral for Msp430Port {
    /// The registers move only by `read`/`write`; no tick behaviour.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match (self.interrupt, offset) {
            (_, OFF_IN) => self.pad_bits(),
            (_, OFF_OUT) => self.out,
            (_, OFF_DIR) => self.dir,
            (true, OFF_IFG) => self.ifg,
            (true, OFF_IES) => self.ies,
            (true, OFF_IE) => self.ie,
            (true, OFF_SEL) | (false, OFF_SEL_BASIC) => self.sel,
            (true, OFF_REN) => self.ren,
            _ => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let interrupt = self.interrupt;
        self.mutate_pads(|s| match (interrupt, offset) {
            // PxIN is read-only.
            (_, OFF_OUT) => s.out = value,
            (_, OFF_DIR) => s.dir = value,
            (true, OFF_IFG) => s.ifg = value,
            (true, OFF_IES) => s.ies = value,
            (true, OFF_IE) => s.ie = value,
            (true, OFF_SEL) | (false, OFF_SEL_BASIC) => s.sel = value,
            (true, OFF_REN) => s.ren = value,
            _ => {}
        });
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn attach_msp430_irq(&mut self, irq: &Arc<Msp430Irq>) {
        if let Some(request) = self.request {
            // Several flags share the line: acceptance clears none of them.
            irq.route(request.source, request.line, false);
            irq.set_enable(request.source, true);
        }
        self.irq = Some(irq.clone());
        self.sync_request();
    }

    fn on_system_reset(&mut self) {
        self.mutate_pads(|s| {
            s.dir = 0;
            s.ie = 0;
            s.sel = s.sel_reset;
            s.ren = 0;
        });
        // After the bracket: a pad the reset released sets no flag.
        self.ifg = 0;
        self.sync_request();
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        PinPort::input(self, pin)
    }

    fn read_gpio_input_word(&self) -> u32 {
        u32::from(self.pad_bits())
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (Self::in_range(pin) && self.own() >> pin & 1 != 0).then(|| self.out >> pin & 1 != 0)
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        Self::in_range(pin).then(|| self.own() >> pin & 1 != 0)
    }

    fn gpio_port_offsets(&self) -> Option<GpioPortOffsets> {
        Some(GpioPortOffsets {
            output: OFF_OUT,
            input: OFF_IN,
        })
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        if !Self::in_range(pin) {
            return None;
        }
        let bit = 1u8 << pin;
        Some(GpioRouting {
            mode: if self.sel & bit != 0 {
                GpioMode::Af
            } else if self.dir & bit != 0 {
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

    fn watch_pad_level(&mut self, pin: u8, cell: Arc<AtomicBool>) -> bool {
        if !Self::in_range(pin) {
            return false;
        }
        cell.store(self.pad_bits() >> pin & 1 != 0, Ordering::Relaxed);
        self.cells.push((pin, cell));
        true
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "out": self.out,
            "dir": self.dir,
            "ifg": self.ifg,
            "ies": self.ies,
            "ie": self.ie,
            "sel": self.sel,
            "ren": self.ren,
        })
    }
}

/// Byte registers that are storage with reset values.
#[derive(Debug)]
pub struct Msp430Bytes {
    reset: Vec<u8>,
    regs: Vec<u8>,
}

impl Msp430Bytes {
    pub fn new(reset: Vec<u8>) -> Self {
        Self {
            regs: reset.clone(),
            reset,
        }
    }
}

impl Peripheral for Msp430Bytes {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.regs.get(offset as usize).copied().unwrap_or(0))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if let Some(reg) = self.regs.get_mut(offset as usize) {
            *reg = value;
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn on_system_reset(&mut self) {
        self.regs.clone_from(&self.reset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST: PortRequest = PortRequest {
        source: 24,
        line: 13,
    };

    fn p1(irq: &Arc<Msp430Irq>) -> Msp430Port {
        let mut p = Msp430Port::with_interrupt(0, Some(REQUEST));
        p.attach_msp430_irq(irq);
        p
    }

    #[test]
    fn reset_is_all_inputs_reading_low() {
        let p = Msp430Port::with_interrupt(0xC0, None);
        assert_eq!(p.read(OFF_DIR).unwrap(), 0);
        assert_eq!(p.read(OFF_IN).unwrap(), 0, "a floating pad reads 0");
        assert_eq!(p.read(OFF_SEL).unwrap(), 0xC0, "XIN/XOUT after a PUC");
        assert_eq!(p.driver(0), Some(PadDriver::OFF));
        assert_eq!(p.driver(7), None, "a module pad has no published driver");
    }

    #[test]
    fn an_output_pad_drives_pxout_and_reads_it_back() {
        let mut p = Msp430Port::with_interrupt(0, None);
        p.write(OFF_DIR, 0x01).unwrap();
        assert_eq!(p.driver(0), Some(PadDriver::drive(false)));
        p.write(OFF_OUT, 0x41).unwrap();
        assert_eq!(p.driver(0), Some(PadDriver::drive(true)));
        assert_eq!(
            p.read(OFF_IN).unwrap(),
            0x01,
            "P1.6 is an input: not driven"
        );
        assert_eq!(p.read_gpio_output(0), Some(true));
        assert_eq!(p.read_gpio_output(6), None);
        assert_eq!(p.level(0), Some(true));
    }

    #[test]
    fn pxren_pulls_an_input_in_the_direction_of_pxout() {
        let mut p = Msp430Port::with_interrupt(0, None);
        p.write(OFF_REN, 0x08).unwrap();
        assert_eq!(p.read(OFF_IN).unwrap() & 0x08, 0, "pull-down");
        p.write(OFF_OUT, 0x08).unwrap();
        assert_eq!(p.read(OFF_IN).unwrap() & 0x08, 0x08, "pull-up");
        assert_eq!(p.driver(3), Some(PadDriver::released(Pull::Up)));
        // A button to ground beats the pull.
        let change = p.set_external(3, External::Level(false)).unwrap();
        assert!(change.before && !change.after);
        assert_eq!(p.read(OFF_IN).unwrap() & 0x08, 0);
        p.set_external(3, External::Released).unwrap();
        assert_eq!(p.read(OFF_IN).unwrap() & 0x08, 0x08);
    }

    #[test]
    fn an_edge_in_the_selected_direction_sets_the_flag_and_requests_the_line() {
        let irq = Msp430Irq::new();
        let mut p = p1(&irq);
        // LaunchPad S2: P1.3, pull-up, falling edge.
        p.write(OFF_REN, 0x08).unwrap();
        p.write(OFF_OUT, 0x08).unwrap();
        p.write(OFF_IES, 0x08).unwrap();
        p.write(OFF_IFG, 0).unwrap();
        p.write(OFF_IE, 0x08).unwrap();
        assert_eq!(irq.pending(), 0);
        p.set_external(3, External::Level(false)).unwrap();
        assert_eq!(p.read(OFF_IFG).unwrap(), 0x08);
        assert_eq!(irq.pending(), 1 << REQUEST.line);
        // Acceptance clears nothing: the handler clears the flag.
        irq.accept(REQUEST.line);
        assert_eq!(irq.pending(), 1 << REQUEST.line);
        p.write(OFF_IFG, 0).unwrap();
        assert_eq!(irq.pending(), 0);
        // The release is a rising edge: not selected.
        p.set_external(3, External::Released).unwrap();
        assert_eq!(p.read(OFF_IFG).unwrap(), 0);
    }

    #[test]
    fn a_flag_without_its_enable_requests_nothing_and_software_can_set_one() {
        let irq = Msp430Irq::new();
        let mut p = p1(&irq);
        p.set_external(0, External::Level(true)).unwrap();
        assert_eq!(p.read(OFF_IFG).unwrap(), 0x01, "rising edge, PxIES 0");
        assert_eq!(irq.pending(), 0, "PxIE is clear");
        p.write(OFF_IE, 0x02).unwrap();
        assert_eq!(irq.pending(), 0, "the enabled pad has no flag");
        p.write(OFF_IFG, 0x03).unwrap();
        assert_eq!(irq.pending(), 1 << REQUEST.line, "software-set flag");
    }

    #[test]
    fn a_write_to_pxout_that_moves_the_pad_sets_the_flag() {
        let irq = Msp430Irq::new();
        let mut p = p1(&irq);
        p.write(OFF_DIR, 0x01).unwrap();
        p.write(OFF_OUT, 0x01).unwrap();
        assert_eq!(p.read(OFF_IFG).unwrap(), 0x01);
    }

    #[test]
    fn a_puc_keeps_pxout_and_clears_the_rest() {
        let irq = Msp430Irq::new();
        let mut p = Msp430Port::with_interrupt(0xC0, Some(REQUEST));
        p.attach_msp430_irq(&irq);
        p.write(OFF_SEL, 0).unwrap();
        p.write(OFF_DIR, 0xFF).unwrap();
        p.write(OFF_OUT, 0x55).unwrap();
        p.write(OFF_IE, 0xFF).unwrap();
        assert_ne!(irq.pending(), 0);
        p.on_system_reset();
        assert_eq!(p.read(OFF_OUT).unwrap(), 0x55);
        assert_eq!(p.read(OFF_DIR).unwrap(), 0);
        assert_eq!(p.read(OFF_SEL).unwrap(), 0xC0);
        assert_eq!(p.read(OFF_IFG).unwrap(), 0);
        assert_eq!(p.read(OFF_IE).unwrap(), 0);
        assert_eq!(irq.pending(), 0);
    }

    #[test]
    fn the_basic_layout_has_pxsel_at_offset_3_and_no_flags() {
        let mut p = Msp430Port::basic(0);
        p.write(OFF_SEL_BASIC, 0x30).unwrap();
        assert_eq!(p.read(OFF_SEL_BASIC).unwrap(), 0x30);
        assert_eq!(p.gpio_routing(4).unwrap().mode, GpioMode::Af);
        p.write(OFF_DIR, 0x01).unwrap();
        p.write(OFF_OUT, 0x01).unwrap();
        assert_eq!(p.read(OFF_IN).unwrap(), 0x01);
        assert_eq!(p.read(4).unwrap(), 0, "no register behind offset 4");
    }

    #[test]
    fn port_ids_count_from_p1() {
        let p = Msp430Port::basic(0);
        assert_eq!(p.port_id("p1"), Some(PortId(0)));
        assert_eq!(p.port_id("P3"), Some(PortId(2)));
        assert_eq!(p.port_id("gpioa"), None);
    }

    #[test]
    fn bytes_are_storage_with_reset_values() {
        let mut b = Msp430Bytes::new(vec![0x60, 0x87]);
        assert_eq!(b.read(1).unwrap(), 0x87);
        b.write(1, 0x12).unwrap();
        assert_eq!(b.read(1).unwrap(), 0x12);
        b.on_system_reset();
        assert_eq!(b.read(1).unwrap(), 0x87);
        assert_eq!(b.read(2).unwrap(), 0, "outside the window");
    }
}
