// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `simctl` — the simulation-control device: a door firmware can knock on to
//! **end its own run with a verdict**.
//!
//! ## Why this exists
//!
//! Every assertion LabWired could previously make about a run was *external*:
//! `uart_regex` / `uart_contains` scraped a serial line, `gpio_*` sampled a pad,
//! `register_*` / `memory_*` peeked at silicon state. All of those prove
//! "the expected bytes/levels appeared" — none of them let the firmware say
//! **"I passed"**. A test that greps a serial log is testing the log.
//!
//! Firmware writes an exit code to `EXIT` and the run ends carrying it as a
//! structured value through [`crate::machine::AdvanceStop::FirmwareExit`].
//!
//! The idea comes from MachineWare's Apache-2.0 VCML `meta::simdev`
//! (<https://github.com/machineware-gmbh/vcml>). The register map below is
//! deliberately **smaller** than theirs — see "What this device does not have".
//!
//! ## Register map (32-bit accesses)
//!
//! | Offset | Name   | Access | Meaning |
//! |--------|--------|--------|---------|
//! | `0x00` | `EXIT` | W      | End the run with this **exit code** (`0` = pass). |
//! | `0x08` | `SCLK` | R      | Simulated time, in CPU cycles (64-bit). |
//! | `0x10` | `SOUT` | W      | Append the low byte to the run's stdout stream. |
//! | `0x18` | `SERR` | W      | Append the low byte to the run's stderr stream. |
//!
//! ## What this device does not have, and why
//!
//! `simdev` also exposes `stop`, `abrt`, `hclk` and `prng`. All four were
//! written and then deliberately removed:
//!
//! - **`ABRT`** is `EXIT(1)`. Two ways to spell one outcome meant exit code 1
//!   arrived from two places and a harness could not tell them apart.
//! - **`STOP`** ended a run while making no pass/fail claim — which has no
//!   honest representation in a result that must say passed or failed. It also
//!   forced a stop reason named "exit" to describe a non-exit.
//! - **`HCLK`** (host wall-clock) let firmware read real time. The runner
//!   already knows wall-clock and simulated time and reports both, so this
//!   bought nothing the result JSON did not already have — while punching an
//!   unenforced non-determinism hole through the middle of an oracle.
//! - **`PRNG`** duplicated [`crate::peripherals::rng::Rng`]'s LFSR. A second
//!   deterministic PRNG is a second source of truth for no new capability.
//!
//! `SCLK` survives because it is deterministic and lets firmware measure its
//! own simulated intervals without a peripheral timer.
//!
//! ## Opt-in and additive
//!
//! This device only exists on a bus that declares it (`type: simctl`). A board
//! that does not declare it has no `simctl` window, and [`crate::Machine`]
//! caches `None` for its index, so the drain in the advance loop is a single
//! `Option` test. Nothing about an existing board changes.

use crate::cycle_clock::CycleClock;
use crate::{Peripheral, SimResult};
use std::any::Any;
use std::cell::Cell;

/// Address window this device answers on.
pub const WINDOW: u64 = 0x20;

// These offsets are the device's ABI. `examples/common/labwired_simctl.h` is
// GENERATED from them by `tools/gen_simctl_header.py`, so the header cannot
// drift — there is no second copy to keep in step.
/// `EXIT` — end the run with an exit code (`0` = pass).
pub const EXIT: u64 = 0x00;
/// `SCLK` — simulated time in CPU cycles (64-bit, `0x08..0x10`).
pub const SCLK: u64 = 0x08;
/// `SOUT` — append a byte to the run's stdout.
pub const SOUT: u64 = 0x10;
/// `SERR` — append a byte to the run's stderr.
pub const SERR: u64 = 0x18;

/// Every register, as `(name, offset)`. The header generator's single input.
pub const REGISTERS: &[(&str, u64)] = &[
    ("EXIT", EXIT),
    ("SCLK", SCLK),
    ("SOUT", SOUT),
    ("SERR", SERR),
];

/// The simulation-control device. See the module docs for the register map.
#[derive(Debug)]
pub struct SimCtl {
    /// The exit code written by firmware, awaiting drain by the advance loop.
    /// `Cell` because the drain happens through a shared borrow, matching
    /// [`crate::peripherals::flash::Flash::drain_pending_op`].
    pending_exit: Cell<Option<u32>>,
    /// Bus-published simulated "now", used to serve `SCLK` from a `&self` read.
    clock: CycleClock,
    /// Bytes firmware wrote to `SOUT`.
    stdout: Vec<u8>,
    /// Bytes firmware wrote to `SERR`.
    stderr: Vec<u8>,
}

impl SimCtl {
    pub fn new() -> Self {
        Self {
            pending_exit: Cell::new(None),
            clock: CycleClock::default(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    /// Take the exit code firmware wrote, if any. Drained once per instruction
    /// boundary by the advance loop.
    pub fn drain_exit_code(&self) -> Option<u32> {
        self.pending_exit.take()
    }

    /// Bytes firmware wrote to `SOUT` — the run's stdout, distinct from UART.
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Bytes firmware wrote to `SERR` — the run's stderr, distinct from UART.
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    /// The byte of a 64-bit register at `byte` (little-endian).
    fn byte_of_u64(value: u64, byte: u64) -> u8 {
        ((value >> (8 * byte)) & 0xFF) as u8
    }
}

impl Default for SimCtl {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for SimCtl {
    /// `SCLK` is the only readable register; everything else reads `0`.
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            SCLK..=0x0F => Ok(Self::byte_of_u64(self.clock.now(), offset - SCLK)),
            _ => Ok(0),
        }
    }

    /// Byte writes are inert except on the output registers, where a byte write
    /// is exactly what it looks like. `EXIT` is 32-bit only — see
    /// [`Peripheral::write_u32`] — so a stray byte store cannot end a run.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            SOUT => self.stdout.push(value),
            SERR => self.stderr.push(value),
            _ => {
                crate::census_reg!("simctl:SimCtl", offset, "write");
            }
        }
        Ok(())
    }

    /// The authoritative write path. The bus calls this directly for 32-bit
    /// MMIO stores, so `EXIT` sees the whole word rather than firing on its
    /// first byte.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // The FIRST exit code wins: once firmware has declared an outcome,
            // a later write cannot mask it before the advance loop drains it.
            // A second write falls through to `_` and is dropped.
            EXIT if self.pending_exit.get().is_none() => {
                self.pending_exit.set(Some(value));
            }
            SOUT => self.stdout.push((value & 0xFF) as u8),
            SERR => self.stderr.push((value & 0xFF) as u8),
            _ => {
                crate::census_reg!("simctl:SimCtl", offset, "write");
            }
        }
        Ok(())
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = clock;
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_records_the_code() {
        let mut d = SimCtl::new();
        d.write_u32(EXIT, 0).unwrap();
        assert_eq!(d.drain_exit_code(), Some(0));
    }

    #[test]
    fn a_nonzero_exit_code_survives_the_whole_word() {
        // The regression this device exists to prevent: firing on the first
        // byte would report 0xEF for a write of 0xDEADBEEF.
        let mut d = SimCtl::new();
        d.write_u32(EXIT, 0xDEAD_BEEF).unwrap();
        assert_eq!(d.drain_exit_code(), Some(0xDEAD_BEEF));
    }

    #[test]
    fn the_first_exit_wins_so_failure_cannot_be_masked() {
        let mut d = SimCtl::new();
        d.write_u32(EXIT, 3).unwrap();
        d.write_u32(EXIT, 0).unwrap();
        assert_eq!(d.drain_exit_code(), Some(3));
    }

    #[test]
    fn draining_twice_yields_nothing_the_second_time() {
        let mut d = SimCtl::new();
        d.write_u32(EXIT, 7).unwrap();
        assert_eq!(d.drain_exit_code(), Some(7));
        assert_eq!(d.drain_exit_code(), None);
    }

    #[test]
    fn byte_writes_to_exit_do_not_end_a_run() {
        let mut d = SimCtl::new();
        d.write(EXIT, 0xFF).unwrap();
        assert_eq!(d.drain_exit_code(), None);
    }

    #[test]
    fn sclk_reports_the_published_simulated_cycle() {
        let mut d = SimCtl::new();
        let clock = CycleClock::default();
        d.attach_cycle_clock(clock.clone());
        clock.publish(0x0123_4567_89AB_CDEF);
        assert_eq!(d.read_u32(SCLK).unwrap(), 0x89AB_CDEF);
        assert_eq!(d.read_u32(SCLK + 4).unwrap(), 0x0123_4567);
    }

    #[test]
    fn sout_and_serr_are_separate_streams() {
        let mut d = SimCtl::new();
        for b in b"ok" {
            d.write_u32(SOUT, u32::from(*b)).unwrap();
        }
        for b in b"bad" {
            d.write_u32(SERR, u32::from(*b)).unwrap();
        }
        assert_eq!(d.stdout(), b"ok");
        assert_eq!(d.stderr(), b"bad");
    }

    #[test]
    fn every_register_fits_the_window() {
        for (name, offset) in REGISTERS {
            assert!(
                offset + 4 <= WINDOW,
                "{name} at {offset:#x} does not fit in the {WINDOW:#x}-byte window"
            );
        }
    }
}
