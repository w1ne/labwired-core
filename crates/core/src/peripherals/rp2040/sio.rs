// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 single-cycle IO block — SIO GPIO (datasheet §2.3.1, base
//! `0xD0000000`).
//!
//! SIO sits on the Cortex-M0+ single-cycle IO port (address `0xD0000000`),
//! *outside* the `0x40000000..0x50400000` APB/AHB peripheral window, so the
//! RP2040 atomic SET/CLR/XOR register aliases (`+0x2000` / `+0x3000` /
//! `+0x1000`) do **not** apply here. Instead SIO exposes dedicated
//! set / clear / xor registers at fixed offsets (`GPIO_OUT_SET` etc.), which
//! this model implements directly.
//!
//! Modelled behaviour: a 30-bit `GPIO_OUT` output latch and a `GPIO_OE` output
//! enable, each driven by direct / set / clear / xor registers. `GPIO_IN`
//! reads back the level a pin is *driving*: `GPIO_OUT & GPIO_OE`. With no
//! external wiring in the chip model an output pin reads back its own driven
//! level (a real, observable set-drive-readback round-trip) and an input
//! (OE=0) pin floats to 0. `CPUID` reads 0 (core 0).

use crate::{Peripheral, SimResult};
use std::cell::Cell;

// SIO register offsets (datasheet §2.3.1.7).
const CPUID: u64 = 0x000;
const GPIO_IN: u64 = 0x004;
const GPIO_HI_IN: u64 = 0x008;
const GPIO_OUT: u64 = 0x010;
const GPIO_OUT_SET: u64 = 0x014;
const GPIO_OUT_CLR: u64 = 0x018;
const GPIO_OUT_XOR: u64 = 0x01c;
const GPIO_OE: u64 = 0x020;
const GPIO_OE_SET: u64 = 0x024;
const GPIO_OE_CLR: u64 = 0x028;
const GPIO_OE_XOR: u64 = 0x02c;

// Hardware integer divider (datasheet §2.3.1.6). RP2040's Cortex-M0+ has no
// DIV instruction; the SDK's `__aeabi_uidiv`/`__aeabi_idiv` wrappers (and
// anything built on `hardware_divider`) route through this SIO-mapped
// divider instead of a software division routine. Leaving it unmodelled
// means every division silently returns 0/0, which is exactly the kind of
// wrong-answer-not-a-halt bug this simulator exists to avoid — and it's fatal
// at boot: arduino-pico's `set_sys_clock_khz` divides VCO frequencies while
// searching for PLL dividers, and a divider that always reads 0 makes every
// candidate frequency look unreachable, so it panics ("cannot be exactly
// achieved") before `LWCONF` ever prints. We compute results synchronously
// (no multi-cycle latency), so READY is always 1 in this model.
const DIV_UDIVIDEND: u64 = 0x060;
const DIV_UDIVISOR: u64 = 0x064;
const DIV_SDIVIDEND: u64 = 0x068;
const DIV_SDIVISOR: u64 = 0x06c;
const DIV_QUOTIENT: u64 = 0x070;
const DIV_REMAINDER: u64 = 0x074;
const DIV_CSR: u64 = 0x078;

// Hardware spinlocks: 32 registers, SPINLOCK0..SPINLOCK31 (datasheet §2.3.1.5).
const SPINLOCK0: u64 = 0x100;
const SPINLOCK31: u64 = 0x17c;

// The RP2040 exposes 30 GPIOs (0..29) on bank 0.
const GPIO_MASK: u32 = 0x3fff_ffff;

/// Push-mode logic capture for SIO bank-0 pads (Arduino `digitalWrite` / LED).
struct SioTap {
    tap: crate::logic_capture::LogicTap,
    /// `(pin, channel)` watch set.
    watched: Vec<(u8, u32)>,
    scratch: Vec<Option<bool>>,
}

impl std::fmt::Debug for SioTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SioTap")
            .field("watched", &self.watched)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
pub struct Rp2040Sio {
    gpio_out: u32,
    gpio_oe: u32,
    /// Pads bound to peripheral wires, resolved against IO_BANK0's live
    /// FUNCSEL. Empty until `SystemBus::wire_rp2040_pads` binds them.
    pad_routes: crate::peripherals::pad_routing::PadRoutes,
    /// Live pad-function state shared from IO_BANK0, so a pad re-assigned at
    /// runtime changes hands immediately.
    pad_functions: Option<std::sync::Arc<super::io_bank0::PadFunctions>>,
    /// Bit `n` set == spinlock `n` is currently claimed. `Cell` because a
    /// spinlock read is a claim (a write side-effect) on the `&self` read path.
    spinlocks_held: Cell<u32>,
    /// Logic-analyzer push tap (not snapshot state).
    tap: Option<SioTap>,
    /// Raw divider operand latches, shared between the U*/S* register views
    /// (real silicon feeds both into the same divider core).
    div_dividend: u32,
    div_divisor: u32,
    div_quotient: u32,
    div_remainder: u32,
    /// Set by the last write that kicked off a calculation; selects
    /// signed vs. unsigned interpretation of the stored operands.
    div_signed: bool,
    /// DIRTY: set on any operand write, cleared when QUOTIENT is read.
    div_dirty: bool,
}

impl Rp2040Sio {
    pub fn new() -> Self {
        Self::default()
    }

    /// Level each pin is driving onto the (unwired) pads: a pin reads back its
    /// own output when its output-enable is set, otherwise it floats to 0.
    fn gpio_in(&self) -> u32 {
        self.gpio_out & self.gpio_oe
    }

    /// The function IO_BANK0 currently selects for `pin` — the selector the
    /// shared routing seam resolves pad bindings against. `None` when IO_BANK0
    /// is not on this bus (so nothing is ever routed) or the pad is NULL.
    fn pad_function(&self, pin: u8) -> Option<u32> {
        self.pad_functions.as_ref()?.function(pin)
    }

    /// Share IO_BANK0's live pad-function state and bind a peripheral wire to
    /// the pads that can carry it. Called at bus wiring time.
    pub(crate) fn bind_pad_route(
        &mut self,
        functions: std::sync::Arc<super::io_bank0::PadFunctions>,
        cell: &std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
        pin: u8,
        function: u32,
        line: usize,
        func_name: &'static str,
    ) {
        self.pad_functions = Some(functions);
        self.pad_routes
            .bind(cell, pin, Some(function), line, func_name);
    }

    /// Every signal name bound to this port's pads, live or not — the
    /// bus-visibility reporting seam. See
    /// [`crate::peripherals::pad_routing::PadRoutes::bound_functions`] for why
    /// this is the static question and `func()` is the live one.
    pub(crate) fn bound_pad_functions(&self) -> Vec<&'static str> {
        self.pad_routes.bound_functions()
    }

    fn pad_level(&self, pin: u8) -> Option<bool> {
        if pin >= 30 {
            return None;
        }
        // A pad IO_BANK0 has handed to a peripheral is driven by that
        // peripheral's wire, not by the SIO output latch. Resolving it through
        // the shared routing seam is what makes an RP2040 bus measurable.
        if let Some(level) = self.pad_routes.level(pin, |p| self.pad_function(p)) {
            return Some(level);
        }
        let bit = 1u32 << pin;
        // Match GPIO_IN: only OE-enabled pins drive a known level.
        if self.gpio_oe & bit == 0 {
            return Some(false);
        }
        Some(self.gpio_out & bit != 0)
    }

    /// Re-register watched pads with the wires that drive them, so a pad that
    /// changes hands follows its new source.
    fn sync_pad_routes(&mut self) {
        if self.pad_routes.is_empty() {
            return;
        }
        let Some(t) = self.tap.take() else {
            return;
        };
        let functions = self.pad_functions.clone();
        self.pad_routes.sync_taps(&t.tap, &t.watched, |pin| {
            functions.as_ref().and_then(|f| f.function(pin))
        });
        self.tap = Some(t);
    }

    fn tap_snapshot(&mut self) {
        let Some(mut t) = self.tap.take() else {
            return;
        };
        for (k, &(pin, _)) in t.watched.iter().enumerate() {
            t.scratch[k] = self.pad_level(pin);
        }
        self.tap = Some(t);
    }

    fn tap_report(&mut self) {
        let Some(t) = self.tap.take() else {
            return;
        };
        for (k, &(pin, ch)) in t.watched.iter().enumerate() {
            if let Some(level) = self.pad_level(pin) {
                if t.scratch[k] != Some(level) {
                    t.tap.push(ch, level);
                }
            }
        }
        self.tap = Some(t);
        self.sync_pad_routes();
    }

    /// Recompute `DIV_QUOTIENT`/`DIV_REMAINDER` from the latched operands,
    /// interpreting them as signed or unsigned per `div_signed`. Mirrors the
    /// RP2040 divider's documented divide-by-zero behaviour (datasheet
    /// §2.3.1.6): unsigned divide-by-zero yields quotient `0xffffffff` and
    /// remainder = dividend; signed divide-by-zero yields quotient `±1`
    /// (sign of the dividend) and remainder = dividend.
    fn recompute_divider(&mut self) {
        if self.div_signed {
            let dividend = self.div_dividend as i32;
            let divisor = self.div_divisor as i32;
            if divisor == 0 {
                self.div_quotient = if dividend < 0 {
                    1i32 as u32
                } else {
                    (-1i32) as u32
                };
                self.div_remainder = dividend as u32;
            } else if dividend == i32::MIN && divisor == -1 {
                // Overflow case: matches the hardware divider's saturation.
                self.div_quotient = i32::MIN as u32;
                self.div_remainder = 0;
            } else {
                self.div_quotient = (dividend / divisor) as u32;
                self.div_remainder = (dividend % divisor) as u32;
            }
        } else {
            let dividend = self.div_dividend;
            let divisor = self.div_divisor;
            // Divide-by-zero is DEFINED on this hardware (datasheet 2.3.1.7):
            // quotient reads all-ones and the remainder is the dividend. Written
            // with checked_div so the zero case is expressed once, in the type,
            // rather than as a separate guard clippy flags as manual_checked_ops.
            match (dividend.checked_div(divisor), dividend.checked_rem(divisor)) {
                (Some(q), Some(r)) => {
                    self.div_quotient = q;
                    self.div_remainder = r;
                }
                _ => {
                    self.div_quotient = 0xffff_ffff;
                    self.div_remainder = dividend;
                }
            }
        }
        self.div_dirty = true;
    }

    /// True if `offset` names a SPINLOCKn register.
    fn is_spinlock(offset: u64) -> bool {
        (SPINLOCK0..=SPINLOCK31).contains(&offset) && offset & 0x3 == 0
    }

    /// Read (claim) SPINLOCKn (datasheet §2.3.1.5): if the lock is free, claim
    /// it atomically and return a nonzero value (bit `n`); if already held,
    /// return 0. This is the genuine try-lock semantics the pico-sdk
    /// `hw_claim_lock` / `spin_lock_blocking` loops rely on to make progress.
    //
    // FIDELITY: modeled, NOT HW-validated (2026-07-04) — SIO SPINLOCK0..31
    // try-lock/release per RP2040 datasheet §2.3.1.5. Single-core model: a free
    // lock is granted on read and released on write.
    fn claim_spinlock(&self, offset: u64) -> u32 {
        let n = (offset - SPINLOCK0) / 4;
        let bit = 1u32 << n;
        let held = self.spinlocks_held.get();
        if held & bit == 0 {
            self.spinlocks_held.set(held | bit);
            bit
        } else {
            0
        }
    }

    /// Write to SPINLOCKn releases the lock (any value; datasheet §2.3.1.5).
    fn release_spinlock(&mut self, offset: u64) {
        let n = (offset - SPINLOCK0) / 4;
        let bit = 1u32 << n;
        self.spinlocks_held.set(self.spinlocks_held.get() & !bit);
    }
}

impl Peripheral for Rp2040Sio {
    /// GPIO latch + spinlocks are pure MMIO — `tick()` is the default no-op.
    /// Dropping SIO from the walk is byte-identical (logic taps fire on write).
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if Self::is_spinlock(offset) {
            return Ok(self.claim_spinlock(offset));
        }
        let val = match offset {
            CPUID => 0, // single core context: always core 0
            GPIO_IN => self.gpio_in(),
            GPIO_HI_IN => 0, // QSPI bank pins — not modelled
            GPIO_OUT | GPIO_OUT_SET | GPIO_OUT_CLR | GPIO_OUT_XOR => self.gpio_out,
            GPIO_OE | GPIO_OE_SET | GPIO_OE_CLR | GPIO_OE_XOR => self.gpio_oe,
            DIV_UDIVIDEND | DIV_SDIVIDEND => self.div_dividend,
            DIV_UDIVISOR | DIV_SDIVISOR => self.div_divisor,
            // Real hardware clears DIRTY when QUOTIENT is read; we leave it
            // latched once set. `hardware_divider`'s save/restore helpers use
            // DIRTY only to decide whether a nested division needs to save
            // and restore the divider state around a reentrant call — an
            // always-1 DIRTY just means that save/restore path is always
            // taken, which is still numerically correct, only slightly more
            // conservative than silicon.
            DIV_QUOTIENT => self.div_quotient,
            DIV_REMAINDER => self.div_remainder,
            DIV_CSR => {
                let ready = 1u32; // synchronous model: always settled.
                let dirty = if self.div_dirty { 1u32 << 1 } else { 0 };
                ready | dirty
            }
            _ => {
                crate::census_reg!("rp2040.sio:Rp2040Sio", offset, "read");
                0
            }
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if Self::is_spinlock(offset) {
            self.release_spinlock(offset);
            return Ok(());
        }
        match offset {
            DIV_UDIVIDEND => {
                self.div_dividend = value;
                self.div_signed = false;
                self.recompute_divider();
                return Ok(());
            }
            DIV_UDIVISOR => {
                self.div_divisor = value;
                self.div_signed = false;
                self.recompute_divider();
                return Ok(());
            }
            DIV_SDIVIDEND => {
                self.div_dividend = value;
                self.div_signed = true;
                self.recompute_divider();
                return Ok(());
            }
            DIV_SDIVISOR => {
                self.div_divisor = value;
                self.div_signed = true;
                self.recompute_divider();
                return Ok(());
            }
            DIV_QUOTIENT => {
                self.div_quotient = value;
                self.div_dirty = true;
                return Ok(());
            }
            DIV_REMAINDER => {
                self.div_remainder = value;
                self.div_dirty = true;
                return Ok(());
            }
            _ => {
                crate::census_reg!("rp2040.sio:Rp2040Sio", offset, "write");
            }
        }
        let v = value & GPIO_MASK;
        let mut_out = matches!(
            offset,
            GPIO_OUT
                | GPIO_OUT_SET
                | GPIO_OUT_CLR
                | GPIO_OUT_XOR
                | GPIO_OE
                | GPIO_OE_SET
                | GPIO_OE_CLR
                | GPIO_OE_XOR
        );
        if mut_out {
            self.tap_snapshot();
        }
        match offset {
            GPIO_OUT => self.gpio_out = v,
            GPIO_OUT_SET => self.gpio_out |= v,
            GPIO_OUT_CLR => self.gpio_out &= !v,
            GPIO_OUT_XOR => self.gpio_out ^= v,
            GPIO_OE => self.gpio_oe = v,
            GPIO_OE_SET => self.gpio_oe |= v,
            GPIO_OE_CLR => self.gpio_oe &= !v,
            GPIO_OE_XOR => self.gpio_oe ^= v,
            _ => {
                crate::census_reg!("rp2040.sio:Rp2040Sio", offset, "write");
            }
        }
        if mut_out {
            self.tap_report();
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        // Don't route a spinlock RMW through the claiming read path.
        if Self::is_spinlock(aligned) {
            return self.write_u32(aligned, value as u32);
        }
        let shift = (offset & 0x3) * 8;
        let cur = match aligned {
            GPIO_OUT | GPIO_OUT_SET | GPIO_OUT_CLR | GPIO_OUT_XOR => self.gpio_out,
            GPIO_OE | GPIO_OE_SET | GPIO_OE_CLR | GPIO_OE_XOR => self.gpio_oe,
            _ => self.read_u32(aligned)?,
        };
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }

    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        self.pad_level(pin)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn install_logic_tap(
        &mut self,
        tap: &crate::logic_capture::LogicTap,
        watched: &[(u8, u32)],
    ) -> bool {
        if watched.is_empty() {
            self.tap = None;
            self.pad_routes.clear_taps();
        } else {
            self.tap = Some(SioTap {
                tap: tap.clone(),
                watched: watched.to_vec(),
                scratch: vec![None; watched.len()],
            });
            // Routed pads are driven by their peripheral's wire, so the wire
            // reports their transitions at the cycles they occurred.
            self.pad_routes.invalidate_registrations();
            self.sync_pad_routes();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN25: u32 = 1 << 25;

    #[test]
    fn cpuid_reads_zero() {
        assert_eq!(Rp2040Sio::new().read_u32(CPUID).unwrap(), 0);
    }

    #[test]
    fn set_drive_readback_roundtrip() {
        let mut sio = Rp2040Sio::new();
        // Output disabled → driven level not visible on GPIO_IN.
        sio.write_u32(GPIO_OUT_SET, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, 0);
        // Enable output → pin reads back its driven high level.
        sio.write_u32(GPIO_OE_SET, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, PIN25);
        assert_eq!(sio.read_u32(GPIO_OUT).unwrap() & PIN25, PIN25);
        // Clear the output → reads back low.
        sio.write_u32(GPIO_OUT_CLR, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, 0);
    }

    #[test]
    fn logic_tap_sees_led_pin25_toggle() {
        use crate::logic_capture::LogicTap;
        use crate::Peripheral;
        let mut sio = Rp2040Sio::new();
        let tap = LogicTap::new();
        assert!(sio.install_logic_tap(&tap, &[(25, 0)]));
        // Arm push mode so ingest is live (machine does this on logic_watch).
        tap.set_armed(true);
        sio.write_u32(GPIO_OE_SET, PIN25).unwrap();
        sio.write_u32(GPIO_OUT_SET, PIN25).unwrap();
        sio.write_u32(GPIO_OUT_CLR, PIN25).unwrap();
        let events = tap.take_events();
        assert!(
            events.len() >= 2,
            "expected LED toggle edges, got {:?}",
            events
        );
    }

    #[test]
    fn xor_toggles_output() {
        let mut sio = Rp2040Sio::new();
        sio.write_u32(GPIO_OE_SET, PIN25).unwrap();
        sio.write_u32(GPIO_OUT_XOR, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, PIN25);
        sio.write_u32(GPIO_OUT_XOR, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, 0);
    }

    #[test]
    fn spinlock_try_lock_and_release() {
        let mut sio = Rp2040Sio::new();
        // First read of a free lock claims it and returns a nonzero value.
        let claimed = sio.read_u32(SPINLOCK0).unwrap();
        assert_ne!(claimed, 0, "free lock is granted on read");
        // While held, a second read returns 0 (would spin on real HW).
        assert_eq!(sio.read_u32(SPINLOCK0).unwrap(), 0, "held lock reads 0");
        // Writing releases it; it can then be claimed again.
        sio.write_u32(SPINLOCK0, 1).unwrap();
        assert_ne!(
            sio.read_u32(SPINLOCK0).unwrap(),
            0,
            "released lock reclaims"
        );
    }

    #[test]
    fn spinlocks_are_independent() {
        // read_u32 claims through a `Cell`, so no `mut` binding is needed.
        let sio = Rp2040Sio::new();
        assert_ne!(sio.read_u32(SPINLOCK0).unwrap(), 0);
        // A different lock is unaffected by claiming lock 0.
        let l31 = sio.read_u32(SPINLOCK31).unwrap();
        assert_ne!(l31, 0, "lock 31 independent of lock 0");
        assert_eq!(l31 & (l31 - 1), 0, "grant value is a single bit (1<<n)");
    }
}
