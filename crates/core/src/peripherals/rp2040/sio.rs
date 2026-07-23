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
    /// Bit `n` set == spinlock `n` is currently claimed. `Cell` because a
    /// spinlock read is a claim (a write side-effect) on the `&self` read path.
    spinlocks_held: Cell<u32>,
    /// Logic-analyzer push tap (not snapshot state).
    tap: Option<SioTap>,
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

    fn pad_level(&self, pin: u8) -> Option<bool> {
        if pin >= 30 {
            return None;
        }
        let bit = 1u32 << pin;
        // Match GPIO_IN: only OE-enabled pins drive a known level.
        if self.gpio_oe & bit == 0 {
            return Some(false);
        }
        Some(self.gpio_out & bit != 0)
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
            _ => 0,
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if Self::is_spinlock(offset) {
            self.release_spinlock(offset);
            return Ok(());
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
            _ => {}
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

    fn install_logic_tap(
        &mut self,
        tap: &crate::logic_capture::LogicTap,
        watched: &[(u8, u32)],
    ) -> bool {
        if watched.is_empty() {
            self.tap = None;
        } else {
            self.tap = Some(SioTap {
                tap: tap.clone(),
                watched: watched.to_vec(),
                scratch: vec![None; watched.len()],
            });
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
