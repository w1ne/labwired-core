// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CoreDebug window at `0xE000_EDF0`, sixteen bytes.
//!
//! `DHCSR` stores `C_DEBUGEN`, `C_HALT`, `C_STEP`, and `C_MASKINTS`.
//! The key is checked only on a word write. `DCRSR` and `DCRDR` read as
//! zero and ignore writes. `DEMCR` stores the vector-catch bits and
//! `TRCENA`. `MON_*` stays zero. The CPU shares this `Arc`.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use crate::SimResult;

const DBGKEY: u32 = 0xA05F;
const C_DEBUGEN: u32 = 1 << 0;
const C_HALT: u32 = 1 << 1;
const C_STEP: u32 = 1 << 2;
const C_MASKINTS: u32 = 1 << 3;
const S_REGRDY: u32 = 1 << 16;
const S_HALT: u32 = 1 << 17;
const S_SLEEP: u32 = 1 << 18;

pub(crate) const DEMCR_VC_CORERESET: u32 = 1 << 0;
pub(crate) const DEMCR_VC_CHKERR: u32 = 1 << 6;
pub(crate) const DEMCR_VC_STATERR: u32 = 1 << 7;
pub(crate) const DEMCR_VC_BUSERR: u32 = 1 << 8;
pub(crate) const DEMCR_VC_HARDERR: u32 = 1 << 10;
pub(crate) const DEMCR_VC_MMERR: u32 = 1 << 4;
const DEMCR_VC_NOCPERR: u32 = 1 << 5;
const DEMCR_VC_INTERR: u32 = 1 << 9;
const DEMCR_TRCENA: u32 = 1 << 24;
const DEMCR_WRITABLE: u32 = DEMCR_VC_CORERESET
    | DEMCR_VC_MMERR
    | DEMCR_VC_NOCPERR
    | DEMCR_VC_CHKERR
    | DEMCR_VC_STATERR
    | DEMCR_VC_BUSERR
    | DEMCR_VC_INTERR
    | DEMCR_VC_HARDERR
    | DEMCR_TRCENA;

const DHCSR: u64 = 0x00;
const DCRSR: u64 = 0x04;
const DCRDR: u64 = 0x08;
const DEMCR: u64 = 0x0C;

#[derive(Debug)]
pub struct DebugHaltState {
    c_debugen: AtomicBool,
    c_halt: AtomicBool,
    c_step: AtomicBool,
    c_maskints: AtomicBool,
    /// Set when a keyed write has `C_STEP` and not `C_HALT` while debug is on.
    /// The next batch retires one instruction, then sets `C_HALT`.
    step_pending: AtomicBool,
    halt_signal: AtomicBool,
    maskints_active: AtomicBool,
    demcr: AtomicU32,
    sleeping: AtomicBool,
}

impl DebugHaltState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            c_debugen: AtomicBool::new(false),
            c_halt: AtomicBool::new(false),
            c_step: AtomicBool::new(false),
            c_maskints: AtomicBool::new(false),
            step_pending: AtomicBool::new(false),
            halt_signal: AtomicBool::new(false),
            maskints_active: AtomicBool::new(false),
            demcr: AtomicU32::new(0),
            sleeping: AtomicBool::new(false),
        })
    }

    pub fn read_dhcsr(&self) -> u32 {
        let mut value = S_REGRDY;
        if self.c_debugen.load(Ordering::Relaxed) {
            value |= C_DEBUGEN;
        }
        if self.c_halt.load(Ordering::Relaxed) {
            value |= C_HALT;
        }
        if self.c_step.load(Ordering::Relaxed) {
            value |= C_STEP;
        }
        if self.c_maskints.load(Ordering::Relaxed) {
            value |= C_MASKINTS;
        }
        if self.halt_signal.load(Ordering::Relaxed) {
            value |= S_HALT;
        }
        if self.sleeping.load(Ordering::Relaxed) {
            value |= S_SLEEP;
        }
        value
    }

    pub fn read_demcr(&self) -> u32 {
        self.demcr.load(Ordering::Relaxed)
    }

    pub fn write_dhcsr(&self, value: u32) {
        if (value >> 16) != DBGKEY {
            return;
        }
        let debugen = (value & C_DEBUGEN) != 0;
        self.c_debugen.store(debugen, Ordering::Relaxed);
        if debugen {
            let halt = (value & C_HALT) != 0;
            let step = (value & C_STEP) != 0;
            self.c_halt.store(halt, Ordering::Relaxed);
            self.c_step.store(step, Ordering::Relaxed);
            self.c_maskints
                .store((value & C_MASKINTS) != 0, Ordering::Relaxed);
            // Halt wins over step on the same write. The step is the write
            // that leaves C_HALT clear while C_STEP is set.
            self.step_pending.store(step && !halt, Ordering::Relaxed);
        } else {
            self.step_pending.store(false, Ordering::Relaxed);
        }
        self.sync_signals();
    }

    pub fn write_demcr(&self, value: u32) {
        self.demcr.store(value & DEMCR_WRITABLE, Ordering::Relaxed);
    }

    pub fn halted(&self) -> bool {
        self.halt_signal.load(Ordering::Relaxed)
    }

    pub(crate) fn step_pending(&self) -> bool {
        self.step_pending.load(Ordering::Relaxed)
    }

    pub(crate) fn maskints_active(&self) -> bool {
        self.maskints_active.load(Ordering::Relaxed)
    }

    pub(crate) fn c_debugen(&self) -> bool {
        self.c_debugen.load(Ordering::Relaxed)
    }

    pub(crate) fn demcr(&self) -> u32 {
        self.demcr.load(Ordering::Relaxed)
    }

    pub(crate) fn set_sleeping(&self, sleeping: bool) {
        self.sleeping.store(sleeping, Ordering::Relaxed);
    }

    /// One stepped instruction retired, or a vector catch fired.
    pub(crate) fn halt_for_debug_event(&self) {
        self.step_pending.store(false, Ordering::Relaxed);
        if self.c_debugen.load(Ordering::Relaxed) {
            self.c_halt.store(true, Ordering::Relaxed);
        }
        self.sync_signals();
    }

    /// Drop the armed step before the instruction runs. A DHCSR write inside
    /// that instruction can arm another step; `halt_for_debug_event` then
    /// sees the new token and leaves it alone.
    pub(crate) fn begin_stepped_instruction(&self) {
        self.step_pending.store(false, Ordering::Relaxed);
        self.sync_signals();
    }

    pub fn clear(&self) {
        self.c_debugen.store(false, Ordering::Relaxed);
        self.c_halt.store(false, Ordering::Relaxed);
        self.c_step.store(false, Ordering::Relaxed);
        self.c_maskints.store(false, Ordering::Relaxed);
        self.step_pending.store(false, Ordering::Relaxed);
        self.demcr.store(0, Ordering::Relaxed);
        self.sleeping.store(false, Ordering::Relaxed);
        self.sync_signals();
    }

    /// A CPU reset drops halt and step. `C_DEBUGEN` and `DEMCR` stay, and
    /// `VC_CORERESET` halts again on this same reset.
    pub(crate) fn on_cpu_reset(&self) {
        let catch = self.c_debugen.load(Ordering::Relaxed)
            && (self.demcr.load(Ordering::Relaxed) & DEMCR_VC_CORERESET) != 0;
        self.c_halt.store(catch, Ordering::Relaxed);
        self.c_step.store(false, Ordering::Relaxed);
        self.step_pending.store(false, Ordering::Relaxed);
        self.sleeping.store(false, Ordering::Relaxed);
        self.sync_signals();
    }

    fn sync_signals(&self) {
        let debugen = self.c_debugen.load(Ordering::Relaxed);
        let halt = self.c_halt.load(Ordering::Relaxed);
        let mask = self.c_maskints.load(Ordering::Relaxed);
        let step_pending = self.step_pending.load(Ordering::Relaxed);
        self.maskints_active
            .store(debugen && mask, Ordering::Relaxed);
        self.halt_signal
            .store(debugen && halt && !step_pending, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub struct ScsDebug {
    pub state: Arc<DebugHaltState>,
}

impl ScsDebug {
    pub fn new(state: Arc<DebugHaltState>) -> Self {
        Self { state }
    }

    fn read_word(&self, aligned: u64) -> u32 {
        match aligned {
            DHCSR => self.state.read_dhcsr(),
            DCRSR | DCRDR => 0,
            DEMCR => self.state.read_demcr(),
            _ => 0,
        }
    }
}

impl crate::Peripheral for ScsDebug {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        let lane = (offset & 3) as u32;
        Ok(((word >> (lane * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let lane = (offset & 3) * 8;
        if aligned == DEMCR {
            let current = self.state.read_demcr();
            let mask = 0xFFu32 << lane;
            let next = (current & !mask) | (u32::from(value) << lane);
            self.state.write_demcr(next);
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            DHCSR => self.state.write_dhcsr(value),
            DEMCR => self.state.write_demcr(value),
            _ => {}
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn wrong_key_does_not_change_dhcsr() {
        let s = DebugHaltState::new();
        s.write_dhcsr(0x0000_0003);
        assert_eq!(s.read_dhcsr(), 0x0001_0000);
    }

    #[test]
    fn debug_key_and_halt_sets_s_halt() {
        let s = DebugHaltState::new();
        s.write_dhcsr(0xA05F_0003);
        assert_eq!(s.read_dhcsr(), 0x0001_0003 | (1 << 17));
        assert!(s.halted());
    }

    #[test]
    fn c_step_arms_one_instruction_and_does_not_halt() {
        let s = DebugHaltState::new();
        s.write_dhcsr(0xA05F_0005);
        let word = s.read_dhcsr();
        assert_ne!(word & (1 << 2), 0, "C_STEP stored, word {word:#x}");
        assert_eq!(word & (1 << 17), 0, "S_HALT clear, word {word:#x}");
        assert!(s.step_pending());
        assert!(!s.halted());
        s.halt_for_debug_event();
        assert!(!s.step_pending());
        assert!(s.halted());
    }

    #[test]
    fn c_maskints_and_demcr_stick_only_while_debug_is_enabled() {
        let s = DebugHaltState::new();
        s.write_dhcsr(0xA05F_0009);
        assert!(s.maskints_active());
        assert_ne!(s.read_dhcsr() & (1 << 3), 0);
        s.write_demcr(DEMCR_VC_BUSERR | (1 << 16));
        assert_eq!(s.read_demcr(), DEMCR_VC_BUSERR);
        s.write_dhcsr(0xA05F_0000);
        assert!(!s.maskints_active());
        assert_ne!(s.read_dhcsr() & (1 << 3), 0, "C_MASKINTS sticks");
    }

    #[test]
    fn scs_keyed_word_write_sets_s_halt_and_byte_lanes() {
        let mut dev = ScsDebug::new(DebugHaltState::new());
        dev.write_u32(0, 0xA05F_0003).unwrap();
        let word = dev.read_u32(0).unwrap();
        assert_ne!(word & (1 << 17), 0, "S_HALT set, word {word:#x}");
        assert_eq!(word, 0x0003_0003);
        for lane in 0..4u64 {
            let byte = ((word >> (lane * 8)) & 0xFF) as u8;
            assert_eq!(dev.read(lane).unwrap(), byte, "lane {lane}");
            assert_eq!(dev.peek(lane), Some(byte), "peek lane {lane}");
        }
    }

    #[test]
    fn byte_write_does_not_change_dhcsr() {
        let mut dev = ScsDebug::new(DebugHaltState::new());
        dev.write(0, 0x03).unwrap();
        assert_eq!(dev.read_u32(0).unwrap(), 0x0001_0000);

        dev.write_u32(0, 0xA05F_0003).unwrap();
        let keyed = dev.read_u32(0).unwrap();
        dev.write(0, 0x00).unwrap();
        assert_eq!(dev.read_u32(0).unwrap(), keyed);
    }

    #[test]
    fn reads_past_dhcsr_are_zero_until_demcr() {
        let mut dev = ScsDebug::new(DebugHaltState::new());
        dev.write_u32(0, 0xA05F_0003).unwrap();
        assert_eq!(dev.read(4).unwrap(), 0);
        assert_eq!(dev.peek(4), Some(0));
        assert_eq!(dev.read_u32(4).unwrap(), 0);
        assert_eq!(dev.read_u32(8).unwrap(), 0);
        dev.write_u32(0x0C, DEMCR_VC_CORERESET).unwrap();
        assert_eq!(dev.read_u32(0x0C).unwrap(), DEMCR_VC_CORERESET);
    }

    #[test]
    fn c_halt_without_debugen_does_not_set_s_halt() {
        let mut dev = ScsDebug::new(DebugHaltState::new());
        dev.write_u32(0, 0xA05F_0002).unwrap();
        let word = dev.read_u32(0).unwrap();
        assert_eq!(
            word & 0x3,
            0,
            "C_HALT ignored while C_DEBUGEN is 0, word {word:#x}"
        );
        assert_eq!(word & (1 << 17), 0, "S_HALT clear, word {word:#x}");
        assert!(!dev.state.halted());
    }

    #[test]
    fn cpu_reset_clears_halt_unless_vector_catch_is_set() {
        let s = DebugHaltState::new();
        s.write_dhcsr(0xA05F_0003);
        s.on_cpu_reset();
        assert!(!s.halted());
        assert_ne!(s.read_dhcsr() & 1, 0, "C_DEBUGEN survives the reset");

        s.write_demcr(DEMCR_VC_CORERESET);
        s.on_cpu_reset();
        assert!(s.halted());
    }
}
