// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! What a source-level stepper needs from a machine, without the `C: Cpu`
//! generic.
//!
//! The stepping algorithm itself lives in `labwired_loader::source_step`,
//! next to the DWARF it reads (the loader depends on this crate, so the
//! algorithm cannot live here). This trait is the machine half: the core a
//! debugger follows, its registers, the engine's breakpoint set, and the one
//! authoritative advance path. [`Machine`] implements it directly; a host
//! that journals its calls (the browser simulator) can implement it over its
//! own wrappers.

use crate::machine::{AdvanceReport, AdvanceRequest};
use crate::{Bus, Cpu, Machine, SimResult};

pub trait SourceStepTarget {
    /// The core a debugger shows and steps: 0, or 1 once a breakpoint stopped
    /// the second core of a dual-core chip (`Machine::breakpoint_core`).
    fn debug_core(&self) -> u8;
    /// Point the debugger at `core`. Ignored for core 1 on a single-core
    /// machine.
    fn set_debug_core(&mut self, core: u8);
    /// The CPU of [`Self::debug_core`].
    fn debug_cpu(&self) -> &dyn Cpu;
    /// One byte through the bus (code fetches for call detection).
    fn read_code_u8(&self, addr: u32) -> Option<u8>;
    /// The engine breakpoint set, sorted.
    fn engine_breakpoints(&self) -> Vec<u32>;
    /// Replace the engine breakpoint set (Thumb bit ignored) and forget the
    /// last stop.
    fn set_engine_breakpoints(&mut self, addresses: &[u32]);
    /// The breakpoint the last advance stopped [`Self::debug_core`] on.
    fn stopped_breakpoint(&self) -> Option<u32>;
    /// Advance through the machine's authoritative execution path.
    fn advance_for_debug(&mut self, request: AdvanceRequest) -> SimResult<AdvanceReport>;
}

impl<C: Cpu + 'static> SourceStepTarget for Machine<C> {
    fn debug_core(&self) -> u8 {
        self.breakpoint_core
    }

    fn set_debug_core(&mut self, core: u8) {
        if core == 0 || self.cpu_secondary.is_some() {
            self.breakpoint_core = core;
        }
    }

    fn debug_cpu(&self) -> &dyn Cpu {
        match (self.breakpoint_core, self.cpu_secondary.as_ref()) {
            (1, Some(cpu)) => cpu,
            _ => &self.cpu,
        }
    }

    fn read_code_u8(&self, addr: u32) -> Option<u8> {
        self.bus.read_u8(u64::from(addr)).ok()
    }

    fn engine_breakpoints(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.breakpoints.iter().copied().collect();
        out.sort_unstable();
        out
    }

    fn set_engine_breakpoints(&mut self, addresses: &[u32]) {
        self.breakpoints = addresses.iter().map(|a| a & !1).collect();
        self.last_breakpoint = None;
        self.last_breakpoint_secondary = None;
    }

    fn stopped_breakpoint(&self) -> Option<u32> {
        match self.breakpoint_core {
            1 => self.last_breakpoint_secondary,
            _ => self.last_breakpoint,
        }
    }

    fn advance_for_debug(&mut self, request: AdvanceRequest) -> SimResult<AdvanceReport> {
        Machine::advance(self, request)
    }
}
