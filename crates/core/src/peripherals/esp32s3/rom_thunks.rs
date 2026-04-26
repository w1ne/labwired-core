// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ROM thunk dispatch for ESP32-S3.
//!
//! The ESP32-S3 has ~384 KiB of mask ROM at 0x4000_0000 holding the BROM
//! reset handler and a library of utility functions (`ets_printf`, cache
//! maintenance, flash access, …).  Real firmware calls a small subset of
//! these.  Rather than emulate the whole BROM, we register Rust thunks at
//! the addresses the firmware calls.
//!
//! ## Dispatch mechanism
//!
//! When the simulator constructs a `RomThunkBank`, it pre-fills the bank's
//! backing memory with the byte sequence `BREAK 1, 14` (encoded
//! `[0xE0, 0x41, 0x00]`) at every registered address.  When the CPU fetches
//! from that address it gets BREAK back.  The CPU's BREAK exec arm
//! recognises `imm_s == 1 && imm_t == 14` as a thunk dispatch, looks up
//! the current PC in the bank, and calls the registered Rust function.
//! The function is responsible for setting `PC = a0` to return.
//!
//! The level-imm pair `1, 14` is reserved for ROM thunks; `1, 15` is
//! reserved for the oracle harness BREAK.  Other BREAK values fall through
//! to the existing `SimulationError::BreakpointHit` raise.

use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, Peripheral, SimResult, SimulationError};
use std::collections::HashMap;

/// A ROM thunk function: invoked when the CPU executes the registered
/// `BREAK 1, 14` at a known address.  Must set `cpu.pc = a0` to return —
/// use `RomThunkBank::return_with(cpu, retval)` for the standard case.
pub type RomThunkFn = fn(&mut XtensaLx7, &mut dyn Bus) -> SimResult<()>;

/// `BREAK 1, 14` encoded as 3 LE bytes.
///
/// Encoding (ST0 format, op0=0, op1=0, op2=0, r=4, s=imm_s=1, t=imm_t=14):
///   st0(r=4, s=1, t=14) = (4<<12)|(1<<8)|(14<<4) = 0x4000 | 0x0100 | 0x00E0
///                       = 0x41E0
///   3-byte LE: 0xE0, 0x41, 0x00
pub const ROM_THUNK_BREAK_BYTES: [u8; 3] = [0xE0, 0x41, 0x00];

/// `imm_s` value reserved for ROM thunk dispatch in the BREAK exec arm.
pub const ROM_THUNK_IMM_S: u8 = 1;
/// `imm_t` value reserved for ROM thunk dispatch.
pub const ROM_THUNK_IMM_T: u8 = 14;

pub struct RomThunkBank {
    base: u32,
    backing: Vec<u8>,
    thunks: HashMap<u32, RomThunkFn>,
}

impl RomThunkBank {
    /// Create an empty bank covering `[base, base + size)`.
    pub fn new(base: u32, size: u32) -> Self {
        Self {
            base,
            backing: vec![0u8; size as usize],
            thunks: HashMap::new(),
        }
    }

    /// Register `thunk` at absolute address `pc`.
    ///
    /// The bank pre-fills 3 bytes at `pc` with `ROM_THUNK_BREAK_BYTES` so
    /// that an instruction fetch from `pc` returns `BREAK 1, 14`.
    pub fn register(&mut self, pc: u32, thunk: RomThunkFn) {
        let off = pc
            .checked_sub(self.base)
            .unwrap_or_else(|| {
                panic!(
                    "RomThunkBank::register: pc 0x{pc:08x} below bank base 0x{:08x}",
                    self.base
                )
            }) as usize;
        assert!(
            off + 3 <= self.backing.len(),
            "RomThunkBank::register: pc 0x{pc:08x} outside bank [0x{:08x}, 0x{:08x})",
            self.base,
            self.base as u64 + self.backing.len() as u64,
        );
        self.backing[off..off + 3].copy_from_slice(&ROM_THUNK_BREAK_BYTES);
        let prev = self.thunks.insert(pc, thunk);
        debug_assert!(
            prev.is_none(),
            "RomThunkBank::register: duplicate thunk at 0x{pc:08x}"
        );
    }

    /// Look up a thunk by absolute PC.  Returns `None` if no thunk is
    /// registered (the BREAK exec arm raises `NotImplemented` in that case).
    pub fn get(&self, pc: u32) -> Option<RomThunkFn> {
        self.thunks.get(&pc).copied()
    }

    /// Return from a ROM thunk by jumping to the saved PC and writing the
    /// 32-bit `value` into the C-ABI return register (a2 of the callee).
    ///
    /// This handles BOTH the CALL0 (no window) and CALL4/8/12 (windowed)
    /// conventions. For CALL0, the return PC is plain a0 of the caller.
    /// For CALL{4,8,12}, the return PC is in a[CALLINC*4] of the caller
    /// with bits[31:30] = CALLINC encoded — the CALLEE's RETW would normally
    /// rotate WB by -CALLINC and mask the encoded bits. ROM thunks don't
    /// execute ENTRY/RETW; we model the equivalent here:
    ///
    /// * CALLINC = 0 (CALL0): pc = a0; a2 = value (unrotated).
    /// * CALLINC = N (CALL{4,8,12}): pc = a[N*4] & 0x3FFF_FFFF;
    ///   value goes into a[N*4 + 2] (the post-rotation a2).
    ///   PS.CALLINC stays where the caller put it — the caller's RFE/RFI
    ///   path doesn't depend on it, and the next CALL clears it.
    pub fn return_with(cpu: &mut XtensaLx7, value: u32) {
        let callinc = cpu.ps.callinc();
        if callinc == 0 {
            // CALL0: return PC in a0, return value in a2 (no rotation).
            cpu.regs.write_logical(2, value);
            cpu.pc = cpu.regs.read_logical(0);
        } else {
            // CALL{4,8,12}: return PC encoded in a[N*4]; return value in a[N*4 + 2].
            // Reconstruct full PC the way RETW does: low 30 from saved a[N*4],
            // high 2 from the thunk's own PC (which is on the same 1 GiB segment
            // as the caller, since CALLn can't cross 1 GiB regions).
            let n = callinc * 4;
            let raw = cpu.regs.read_logical(n);
            cpu.regs.write_logical(n + 2, value);
            cpu.pc = (raw & 0x3FFF_FFFF) | (cpu.pc & 0xC000_0000);
        }
    }
}

impl std::fmt::Debug for RomThunkBank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RomThunkBank(base=0x{:08x}, size={}, {} thunks)",
            self.base,
            self.backing.len(),
            self.thunks.len(),
        )
    }
}

impl Peripheral for RomThunkBank {
    fn read(&self, offset: u64) -> SimResult<u8> {
        self.backing
            .get(offset as usize)
            .copied()
            .ok_or(SimulationError::MemoryViolation(self.base as u64 + offset))
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // ROM is read-only; silently drop writes (real silicon ignores them).
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

// ── Default thunk set ────────────────────────────────────────────────────────
//
// These are the thunk functions esp-hal hello-world is expected to call.
// The actual addresses are filled in during Task 11 by disassembling the
// built firmware and reading ESP-IDF's `rom/esp32s3.rom.ld`.
// The implementations here are NOPs or zero-returns where appropriate.

/// `Cache_Suspend_DCache(): u32` — returns 0 (cache wasn't suspended).
pub fn cache_suspend_dcache(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Resume_DCache(prev: u32) -> u32` — returns 0.
pub fn cache_resume_dcache(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `esp_rom_spiflash_unlock(): u32` — returns 0 (success).
pub fn esp_rom_spiflash_unlock(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `rom_config_instruction_cache_mode(...)` — NOP, returns 0.
pub fn rom_config_instruction_cache_mode(
    cpu: &mut XtensaLx7,
    _bus: &mut dyn Bus,
) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `ets_set_appcpu_boot_addr(addr: u32) -> u32` — NOP, returns 0
/// (cpu1 is not modelled in Plan 2).
pub fn ets_set_appcpu_boot_addr(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `rtc_get_reset_reason(cpu_idx: u32) -> u32` — returns 1 (POWERON_RESET).
///
/// ESP32-S3 enum values per ESP-IDF rtc_cntl.h:
///   1  POWERON_RESET — chip just powered on (the value we report).
///   3  SW_RESET, 12  SW_CPU_RESET, etc.
/// esp-hal's reset cause code branches on this — POWERON_RESET is the
/// most "first boot" / least surprising value.
pub fn rtc_get_reset_reason(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 1);
    Ok(())
}

/// `ets_printf(fmt: *const u8, ...)` — minimal printf expansion.
///
/// Reads fmt string from `a2`, expands `%s/%d/%i/%u/%x/%p/%c/%%` consuming
/// args from a3..a7, writes to host log via `tracing::info!`.
pub fn ets_printf(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let fmt_addr = cpu.regs.read_logical(2);
    let mut fmt = String::new();
    for i in 0..256u32 {
        let b = match bus.read_u8(fmt_addr.wrapping_add(i) as u64) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    target: "esp32s3::rom::ets_printf",
                    "fault reading fmt at 0x{:08x}: {}",
                    fmt_addr.wrapping_add(i), e
                );
                0
            }
        };
        if b == 0 {
            break;
        }
        fmt.push(b as char);
    }

    let args = [
        cpu.regs.read_logical(3),
        cpu.regs.read_logical(4),
        cpu.regs.read_logical(5),
        cpu.regs.read_logical(6),
        cpu.regs.read_logical(7),
    ];
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    let mut argi = 0usize;
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => {
                let addr = args[argi.min(4)];
                argi += 1;
                for i in 0..256u32 {
                    let b = match bus.read_u8(addr.wrapping_add(i) as u64) {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::warn!(
                                target: "esp32s3::rom::ets_printf",
                                "fault reading %s arg at 0x{:08x}: {}",
                                addr.wrapping_add(i), e
                            );
                            0
                        }
                    };
                    if b == 0 {
                        break;
                    }
                    out.push(b as char);
                }
            }
            Some('d') | Some('i') => {
                out.push_str(&format!("{}", args[argi.min(4)] as i32));
                argi += 1;
            }
            Some('u') => {
                out.push_str(&format!("{}", args[argi.min(4)]));
                argi += 1;
            }
            Some('x') => {
                out.push_str(&format!("{:x}", args[argi.min(4)]));
                argi += 1;
            }
            Some('p') => {
                out.push_str(&format!("0x{:08x}", args[argi.min(4)]));
                argi += 1;
            }
            Some('c') => {
                out.push((args[argi.min(4)] as u8) as char);
                argi += 1;
            }
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    tracing::info!(target: "esp32s3::rom::ets_printf", "{}", out);
    RomThunkBank::return_with(cpu, out.len() as u32);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::cpu::xtensa_lx7::XtensaLx7;
    use crate::Cpu;

    #[test]
    fn registered_thunk_address_holds_break_bytes() {
        let mut bank = RomThunkBank::new(0x4000_0000, 0x10_0000);
        bank.register(0x4000_1234, cache_suspend_dcache);
        // Reach into backing via the Peripheral read API.
        let off = 0x1234u64;
        assert_eq!(bank.read(off).unwrap(), ROM_THUNK_BREAK_BYTES[0]);
        assert_eq!(bank.read(off + 1).unwrap(), ROM_THUNK_BREAK_BYTES[1]);
        assert_eq!(bank.read(off + 2).unwrap(), ROM_THUNK_BREAK_BYTES[2]);
    }

    #[test]
    fn unregistered_thunk_returns_none() {
        let bank = RomThunkBank::new(0x4000_0000, 0x10_0000);
        assert!(bank.get(0x4000_1234).is_none());
    }

    #[test]
    fn registered_thunk_is_retrievable() {
        let mut bank = RomThunkBank::new(0x4000_0000, 0x10_0000);
        bank.register(0x4000_2000, cache_suspend_dcache);
        assert!(bank.get(0x4000_2000).is_some());
    }

    #[test]
    fn return_with_sets_a2_and_pc() {
        let mut bus = SystemBus::new();
        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();
        cpu.regs.write_logical(0, 0x4037_0010); // a0 = return address
        cpu.set_pc(0x4000_0000);
        RomThunkBank::return_with(&mut cpu, 0xCAFE_BABE);
        assert_eq!(cpu.regs.read_logical(2), 0xCAFE_BABE);
        assert_eq!(cpu.get_pc(), 0x4037_0010);
    }

    #[test]
    fn break_1_14_dispatches_to_thunk_via_bus() {
        let mut bus = SystemBus::new();
        let mut bank = RomThunkBank::new(0x4037_0000, 0x100);
        // Register a thunk that bumps a2 by 1 to prove it ran.
        fn bump_a2(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
            let v = cpu.regs.read_logical(2);
            RomThunkBank::return_with(cpu, v + 1);
            Ok(())
        }
        bank.register(0x4037_0000, bump_a2);
        bus.add_peripheral("rom", 0x4037_0000, 0x100, None, Box::new(bank));

        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();
        cpu.regs.write_logical(0, 0x4037_0080); // a0 = return address
        cpu.regs.write_logical(2, 41);          // a2 = 41
        cpu.set_pc(0x4037_0000);

        // Step once: should fetch BREAK 1,14, dispatch to bump_a2, return.
        cpu.step(&mut bus, &[]).expect("step dispatches thunk");

        assert_eq!(cpu.regs.read_logical(2), 42);
        assert_eq!(cpu.get_pc(), 0x4037_0080);
    }

    #[test]
    fn break_1_14_unregistered_raises_not_implemented() {
        // Plant raw BREAK 1,14 bytes in a Ram peripheral with NO thunk
        // registered, so the dispatch path's "no thunk found" branch fires.
        struct OneShotRam(std::cell::RefCell<Vec<u8>>);
        impl std::fmt::Debug for OneShotRam {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "OneShotRam")
            }
        }
        impl Peripheral for OneShotRam {
            fn read(&self, off: u64) -> SimResult<u8> {
                Ok(*self.0.borrow().get(off as usize).unwrap_or(&0))
            }
            fn write(&mut self, _off: u64, _v: u8) -> SimResult<()> {
                Ok(())
            }
        }

        let mut bytes = vec![0u8; 0x100];
        bytes[0..3].copy_from_slice(&ROM_THUNK_BREAK_BYTES);

        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "ram",
            0x4037_0000,
            0x100,
            None,
            Box::new(OneShotRam(std::cell::RefCell::new(bytes))),
        );

        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();
        cpu.set_pc(0x4037_0000);

        let res = cpu.step(&mut bus, &[]);
        match res {
            Err(SimulationError::NotImplemented(msg)) => {
                assert!(msg.contains("ROM thunk"), "unexpected message: {msg}");
            }
            other => panic!("expected NotImplemented, got {other:?}"),
        }
    }
}
