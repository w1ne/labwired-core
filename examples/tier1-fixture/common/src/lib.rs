// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Shared helpers for the Tier-1 fixture firmwares.
//!
//! Every fixture under `examples/tier1-fixture/` is a standalone `no_std`
//! binary that pokes raw MMIO and reports one `TIER1 <class> PASS|FAIL` line
//! per peripheral class. The register pokes are necessarily per-chip, but the
//! volatile-access helpers, the deterministic spin, and the TIER1 reporting
//! protocol are not — they lived in 11 copies before this crate.

#![no_std]

use core::ptr::{read_volatile, write_volatile};

#[inline(always)]
pub fn rd32(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

#[inline(always)]
pub fn wr32(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// Fixed-iteration busy spin — deterministic in the simulator, which has no
/// wall clock. Never replace this with a time-based delay.
pub fn spin(iters: u32) {
    for i in 0..iters {
        core::hint::black_box(i);
    }
}

/// A polled byte-at-a-time UART console, parameterized by the register layout
/// so families with different USART maps share one implementation.
pub struct Console {
    /// Address of the status register holding the transmit-ready bit.
    status: u32,
    /// Address of the transmit data register (byte-wide write).
    tx: u32,
    /// Mask selecting the transmit-ready bit within the status word.
    ready_mask: u32,
}

impl Console {
    /// `status` and `tx` are absolute MMIO addresses; `ready_mask` selects the
    /// transmit-empty bit. Read the full status word and bit-test: a sign-bit
    /// test on a byte load compiles to `LDRSB` reg-offset, which the
    /// simulator's 16-bit Thumb decoder does not implement.
    pub const fn new(status: u32, tx: u32, ready_mask: u32) -> Self {
        Self {
            status,
            tx,
            ready_mask,
        }
    }

    pub fn putc(&self, byte: u8) {
        for _ in 0..10_000 {
            if rd32(self.status) & self.ready_mask != 0 {
                break;
            }
        }
        unsafe { write_volatile(self.tx as *mut u8, byte) };
    }

    pub fn puts(&self, s: &[u8]) {
        for &b in s {
            self.putc(b);
        }
    }

    /// Emit one line of the TIER1 protocol:
    /// `TIER1 <class> PASS` or `TIER1 <class> FAIL code=<reason>`.
    pub fn report(&self, class: &[u8], result: Result<(), &'static [u8]>) {
        self.puts(b"TIER1 ");
        self.puts(class);
        match result {
            Ok(()) => self.puts(b" PASS\n"),
            Err(code) => {
                self.puts(b" FAIL code=");
                self.puts(code);
                self.puts(b"\n");
            }
        }
    }
}

/// STM32F4-specific checks. Feature-gated because they use ARMv7-M-only
/// instructions (`cpsid`/`cpsie`) that do not assemble for the ARMv6-M
/// (Cortex-M0+) fixtures which also depend on this crate — notably
/// stm32l073. Only the F4 fixtures enable it.
#[cfg(feature = "f4")]
pub mod f4;
