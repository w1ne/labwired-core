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
//! CHEAT(THUNK): every fn in this module fakes a function instead of executing
//! real code. Two kinds (see FIDELITY.md §A): THUNK-ROM (boot-ROM helpers we
//! have no binary to run — math, memcpy, cache, printf — reasonable to emulate)
//! and THUNK-LIB / BYPASS / NOP (firmware library code that IS in the ELF but we
//! skip — heap_caps, FreeRTOS, the SPI/GxEPD path — genuine fidelity debt).
//! Individual cheats below carry their own CHEAT(...) marker.
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
    /// Create a bank covering `[base, base + size)`.
    ///
    /// Prefills every 4-byte-aligned word with `BREAK 1, 14` so a CALL into an
    /// unregistered ROM entry still dispatches (via [`Self::get`] →
    /// [`nop_return_zero`]) instead of fetching zeros and raising an illegal
    /// instruction. Explicit [`Self::register`] entries overwrite the prefill
    /// with a real thunk.
    pub fn new(base: u32, size: u32) -> Self {
        let mut backing = vec![0u8; size as usize];
        let mut off = 0usize;
        while off + 3 <= backing.len() {
            backing[off..off + 3].copy_from_slice(&ROM_THUNK_BREAK_BYTES);
            off += 4;
        }
        Self {
            base,
            backing,
            thunks: HashMap::new(),
        }
    }

    /// Boot-time loader for real ROM contents (e.g. the Espressif ESP32
    /// BROM ELF). Writes `bytes` into the backing store at the absolute
    /// address `pc`, bypassing the read-only guard that `write()` enforces
    /// at runtime. Use only during machine construction; after Run begins,
    /// the bank is treated as ROM.
    ///
    /// `pc` must be within `[base, base + size)`; bytes that would extend
    /// past `size` are silently truncated and a warning is emitted.
    pub fn preload_bytes(&mut self, pc: u32, bytes: &[u8]) {
        let Some(off) = pc.checked_sub(self.base) else {
            tracing::warn!(
                "RomThunkBank::preload_bytes: pc 0x{pc:08x} below bank base 0x{:08x}",
                self.base
            );
            return;
        };
        let off = off as usize;
        let end = off.saturating_add(bytes.len()).min(self.backing.len());
        let n = end.saturating_sub(off);
        if n < bytes.len() {
            tracing::warn!(
                "RomThunkBank::preload_bytes: {} of {} bytes at 0x{pc:08x} truncated to fit bank [0x{:08x}, 0x{:08x})",
                bytes.len() - n,
                bytes.len(),
                self.base,
                self.base + self.backing.len() as u32,
            );
        }
        self.backing[off..off + n].copy_from_slice(&bytes[..n]);
    }

    /// Register `thunk` at absolute address `pc`.
    ///
    /// The bank pre-fills 3 bytes at `pc` with `ROM_THUNK_BREAK_BYTES` so
    /// that an instruction fetch from `pc` returns `BREAK 1, 14`.
    pub fn register(&mut self, pc: u32, thunk: RomThunkFn) {
        let off = pc.checked_sub(self.base).unwrap_or_else(|| {
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

    /// Look up a thunk by absolute PC. Unregistered BREAK 1,14 sites (the
    /// harness prefill) fall through to [`nop_return_zero`] so Arduino's
    /// long-tail ROM HAL surface does not fault mid-boot.
    pub fn get(&self, pc: u32) -> Option<RomThunkFn> {
        Some(self.thunks.get(&pc).copied().unwrap_or(nop_return_zero))
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
            // Thunks skip ENTRY/RETW, so they bypass the sim-level shadow
            // spill that CALL{n} performs (caller's preserved + WS-conditional
            // callee window). Pop those entries to keep stacks balanced.
            // We pop AFTER setting the return value so the pop's restore can
            // safely clobber the return-value AR slot — write_logical above
            // already saved it into the AR, but the pop will undo that for
            // slot wb_callee's a2 position. To preserve the return value,
            // re-write it after the pops.
            // Balance spill_shadow_on_call: displace is on per-slot LIFO;
            // preserve lives only on call_preserve_stack (anti-steal).
            // WB is still at the caller (thunks skip ENTRY).
            let wb_callee = cpu.regs.windowbase().wrapping_add(callinc) & 0x0F;
            for k in 0..4u8 {
                let slot = wb_callee.wrapping_add(k) & 0x0F;
                cpu.regs.pop_shadow(slot);
            }
            cpu.restore_call_preserve();
            cpu.regs.write_logical(n + 2, value);
            // ENTRY would have cleared CALLINC; thunks skip ENTRY so clear it
            // here. Leaving CALLINC sticky confuses nested CALL/interrupt paths.
            cpu.ps.set_callinc(0);
            // Caller still owes a RETW. Defer IRQs until that RETW so a just-
            // unmasked timer cannot run an ISR with the callee window still
            // open (ExitCritical after _xtos_set_intlevel).
            cpu.set_defer_irq_until_retw(true);
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

/// EXTMEM cache-state register (`EXTMEM + 0x130`). ESP-IDF's IRAM cache
/// wrappers (`Cache_Suspend_DCache`, `Cache_Freeze_{I,D}Cache_Enable`) call the
/// ROM routine and then **busy-wait** on this register until their field reads
/// the expected value. The simulator has no async cache, so these thunks drive
/// the fields directly:
///   bits[11:0]  = ICache freeze state          (1 = frozen)
///   bits[23:12] = DCache suspend/freeze state   (1 = suspended/frozen)
/// Without this the firmware spins here forever during flash bring-up, before
/// the scheduler ever starts. Read-modify-write so the two fields don't clobber
/// each other.
const EXTMEM_CACHE_STATE: u64 = 0x600C_4130;
const ICACHE_FIELD: u32 = 0x0000_0FFF; // bits[11:0]
const DCACHE_FIELD: u32 = 0x00FF_F000; // bits[23:12]

fn set_cache_field(bus: &mut dyn Bus, mask: u32, frozen: bool) -> SimResult<()> {
    let v = bus.read_u32(EXTMEM_CACHE_STATE)?;
    let one = mask & mask.wrapping_neg(); // value `1` placed in the field's LSB
    let nv = if frozen { (v & !mask) | one } else { v & !mask };
    bus.write_u32(EXTMEM_CACHE_STATE, nv)
}

/// `Cache_Suspend_ICache(): u32` — mark the ICache idle/suspended
/// (bits[11:0]=1). ESP-IDF IRAM wrappers call the ROM then busy-wait
/// `CACHE_STATE[11:0] == 1`; a plain nop leaves a previously-cleared field
/// stuck and the firmware spins forever in `spi_flash_disable_cache`.
pub fn cache_suspend_icache(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, ICACHE_FIELD, true)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Resume_ICache(prev: u32) -> u32` — restore ICache to idle/enabled.
/// Real silicon re-enables the cache and returns to state=1 (idle); we keep
/// the field at 1 so subsequent suspend/freeze polls still observe idle.
pub fn cache_resume_icache(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, ICACHE_FIELD, true)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Suspend_DCache(): u32` — mark the DCache suspended (bits[23:12]=1).
pub fn cache_suspend_dcache(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, DCACHE_FIELD, true)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Resume_DCache(prev: u32) -> u32` — clear the DCache field.
pub fn cache_resume_dcache(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, DCACHE_FIELD, false)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Disable_ICache(): u32` — ICache off; state field goes busy/cleared
/// until the next suspend/enable. IRAM doesn't poll here, but matching the
/// silicon side-effect keeps CACHE_STATE honest for later Suspend polls.
pub fn cache_disable_icache(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, ICACHE_FIELD, false)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Enable_ICache(autoload: u32) -> u32` — ICache on; idle state = 1.
pub fn cache_enable_icache(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, ICACHE_FIELD, true)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Disable_DCache(): u32` — clear DCache idle field.
pub fn cache_disable_dcache(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, DCACHE_FIELD, false)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Enable_DCache(autoload: u32) -> u32` — DCache on; idle state = 1.
pub fn cache_enable_dcache(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, DCACHE_FIELD, true)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Freeze_ICache_Enable()` — mark the ICache frozen (bits[11:0]=1).
pub fn cache_freeze_icache_enable(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, ICACHE_FIELD, true)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Freeze_ICache_Disable()` — clear the ICache freeze field.
pub fn cache_freeze_icache_disable(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, ICACHE_FIELD, false)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Freeze_DCache_Enable()` — mark the DCache frozen (bits[23:12]=1).
pub fn cache_freeze_dcache_enable(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, DCACHE_FIELD, true)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `Cache_Freeze_DCache_Disable()` — clear the DCache freeze field.
pub fn cache_freeze_dcache_disable(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    set_cache_field(bus, DCACHE_FIELD, false)?;
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `_xtos_restore_intlevel(saved_ps)` — restore PS.INTLEVEL from the value a
/// prior `_xtos_set_intlevel` returned (end of a critical section). Pairs with
/// the existing `xtos_set_intlevel` (below), which returns the prior INTLEVEL.
pub fn xtos_restore_intlevel(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let saved = cpu.regs.read_logical(n + 2);
    cpu.ps.set_intlevel((saved & 0xF) as u8);
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

// ── BROM newlib syscall shims (open/close/read/write) ────────────────────────
//
// ESP-IDF >= 5.x sets up a UART console VFS at boot: `console_open` calls the
// ROM newlib `open()` to obtain a console fd, then stdio routes through the
// ROM `write()`. On silicon each ROM shim is a thin trampoline —
//
//     int open(const char *p, int f, int m) {
//         return syscall_table_ptr_pro->_open_r(__getreent(), p, f, m);
//     }
//
// dispatching through the per-core `syscall_table_ptr` (which ESP-IDF points
// at its own `s_stub_table`, whose `_open_r` etc. are the `esp_vfs_*`
// handlers). The classic-ESP32 BROM pages backing these aren't modeled, so we
// reproduce that trampoline: read the table pointer, load the `_fn_r` slot,
// prepend the reentrancy struct, and tail-call the firmware's esp_vfs handler.
// Crucially this lets `esp_vfs_open` actually register the console fd in
// `s_fd_table` — a return-fd stub would skip registration and newlib stdio
// would then spin forever in `__swsetup_r`/`fstat` on the unregistered fd.

/// `syscall_table_ptr_pro` — ESP32 ROM-fixed DRAM word holding the active
/// PRO_CPU syscall stub table pointer (esp32.rom.ld). The sim runs on
/// PRO_CPU (core 0).
const ESP32_SYSCALL_TABLE_PTR_PRO: u64 = 0x3FFA_E024;
/// `_global_impure_ptr` — ESP32 ROM-fixed DRAM word holding `&_GLOBAL_REENT`.
/// The console VFS is brought up pre-scheduler, so the global reent is the
/// correct reentrancy struct for these calls.
const ESP32_GLOBAL_IMPURE_PTR: u64 = 0x3FFA_E0B0;
// Byte offsets of the `_fn_r` slots within the ROM `syscall_stub_table`
// (verified against the firmware's `s_stub_table` initializer).
const STUB_OFF_CLOSE_R: u32 = 0x4C;
const STUB_OFF_OPEN_R: u32 = 0x50;
const STUB_OFF_WRITE_R: u32 = 0x54;
const STUB_OFF_READ_R: u32 = 0x5C;

/// Tail-call the firmware's `esp_vfs` handler for a ROM newlib shim. Reads
/// `*syscall_table_ptr_pro`, loads the `_fn_r` pointer at `table_offset`,
/// injects the reent pointer as the new first argument (shifting the shim's
/// own args up one slot), and redirects PC to the handler — which runs in
/// the shim's call frame and `retw`s straight back to the shim's caller.
/// Returns `false` (caller should fall back) if the table isn't set up yet.
fn trampoline_syscall(cpu: &mut XtensaLx7, bus: &mut dyn Bus, table_offset: u32) -> bool {
    let tbl = match bus.read_u32(ESP32_SYSCALL_TABLE_PTR_PRO) {
        Ok(v) if v != 0 => v,
        _ => return false,
    };
    let target = match bus.read_u32(tbl as u64 + table_offset as u64) {
        Ok(v) if v != 0 => v,
        _ => return false,
    };
    let reent = bus.read_u32(ESP32_GLOBAL_IMPURE_PTR).unwrap_or(0);
    // Callee arg registers sit at logical [callinc*4 + 2 ..] of the thunk's
    // (caller's) window frame — same indexing the other ROM thunks use.
    let base = if cpu.ps.callinc() == 0 {
        2
    } else {
        cpu.ps.callinc() * 4 + 2
    };
    // Shift the shim's args (a2,a3,a4) up to (a3,a4,a5) and place the reent
    // in a2 — the `_r` reentrant calling convention. Up to three args covers
    // open/read/write (3) and close (1).
    let arg0 = cpu.regs.read_logical(base);
    let arg1 = cpu.regs.read_logical(base + 1);
    let arg2 = cpu.regs.read_logical(base + 2);
    cpu.regs.write_logical(base, reent);
    cpu.regs.write_logical(base + 1, arg0);
    cpu.regs.write_logical(base + 2, arg1);
    cpu.regs.write_logical(base + 3, arg2);
    cpu.pc = target;
    true
}

/// `open(path, flags, mode) -> int` → `esp_vfs_open(reent, …)`.
pub fn rom_open(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    if !trampoline_syscall(cpu, bus, STUB_OFF_OPEN_R) {
        RomThunkBank::return_with(cpu, u32::MAX); // -1: table not ready
    }
    Ok(())
}

/// `close(fd) -> int` → `esp_vfs_close(reent, fd)`.
pub fn rom_close(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    if !trampoline_syscall(cpu, bus, STUB_OFF_CLOSE_R) {
        RomThunkBank::return_with(cpu, 0);
    }
    Ok(())
}

/// `read(fd, buf, len) -> ssize_t` → `esp_vfs_read(reent, fd, buf, len)`.
pub fn rom_read(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    if !trampoline_syscall(cpu, bus, STUB_OFF_READ_R) {
        RomThunkBank::return_with(cpu, 0);
    }
    Ok(())
}

/// `write(fd, buf, len) -> ssize_t` → `esp_vfs_write(reent, fd, buf, len)`.
pub fn rom_write(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    if !trampoline_syscall(cpu, bus, STUB_OFF_WRITE_R) {
        let base = if cpu.ps.callinc() == 0 {
            2
        } else {
            cpu.ps.callinc() * 4 + 2
        };
        let len = cpu.regs.read_logical(base + 2);
        RomThunkBank::return_with(cpu, len);
    }
    Ok(())
}

/// `esp_rom_spiflash_unlock(): u32` — returns 0 (success).
pub fn esp_rom_spiflash_unlock(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `rom_config_instruction_cache_mode(...)` — NOP, returns 0.
pub fn rom_config_instruction_cache_mode(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `ets_set_appcpu_boot_addr(addr: u32) -> u32` — real silicon: stores
/// the address PRO_CPU wants APP_CPU to start executing at, then
/// releases APP_CPU from reset-hold. In dual-core sim configs we stash
/// the boot_addr in a thread-local that `Machine::step` reads on its
/// next tick to unhalt `cpu_secondary` with PC = boot_addr. Single-core
/// configs ignore the stash (no secondary CPU to wake), so the thunk
/// stays safe for either configuration.
pub fn ets_set_appcpu_boot_addr(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    // Argument register: a2 (CALL0), or a[N*4+2] under CALLn windowing.
    let arg_slot = if cpu.ps.callinc() == 0 {
        2
    } else {
        cpu.ps.callinc() * 4 + 2
    };
    let boot_addr = cpu.regs.read_logical(arg_slot);
    // Only release APP_CPU on a real entry-point write. ESP-IDF's
    // `esp_cpu_stall` path also calls this with addr=0 to reset the
    // shadow register before re-stalling — treat that as a no-op (we
    // don't model APP_CPU stall/resume cycles, just the initial wake).
    if boot_addr != 0 {
        APPCPU_BOOT_ADDR.with(|slot| slot.set(Some(boot_addr)));

        // Model APP_CPU bring-up. On silicon, releasing APP_CPU makes it
        // run `call_start_cpu1`, which marks the per-core startup
        // handshake flags (`s_cpu_up[1]`, `s_cpu_inited`, ...) up; PRO_CPU
        // in `start_other_core` spin-waits on those flags and `abort()`s
        // on timeout. We don't execute APP_CPU, so we model the
        // *observable effect* of its boot: set those flags here, at the
        // exact cycle PRO_CPU releases APP_CPU. This write lands after
        // `.bss` zero-init and immediately before the spin-wait, so —
        // unlike a periodic keep-alive — it cannot lose the race against
        // newer arduino-esp32 cores whose timeout is shorter than the
        // reseed interval. Empty unless a frontend resolved the flag
        // addresses from the firmware ELF.
        APPCPU_UP_FLAGS.with(|flags| {
            for &addr in flags.borrow().iter() {
                let _ = bus.write_u8(addr as u64, 0x01);
            }
        });
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// Record the firmware DRAM byte addresses of the dual-core startup
/// handshake flags so [`ets_set_appcpu_boot_addr`] can mark them up when
/// PRO_CPU releases APP_CPU. Pass the resolved `s_cpu_up`/`s_cpu_up+1`/
/// `s_cpu_inited`/… byte addresses (skip any that resolved to 0). Call
/// once per firmware after symbol resolution; replaces any prior set.
pub fn set_appcpu_up_flags(addrs: Vec<u32>) {
    APPCPU_UP_FLAGS.with(|flags| *flags.borrow_mut() = addrs);
}

/// Repin arduino-esp32's `loopTask` from APP_CPU to PRO_CPU by rewriting the
/// `xCoreID` immediate in `app_main`. arduino-esp32 pins `loopTask` to
/// `CONFIG_ARDUINO_RUNNING_CORE` (=1, APP_CPU); the sim models only PRO_CPU
/// (core 0), so a core-1 task is never scheduled and `setup()`/`loop()` never
/// run. Two firmware shapes are handled (legacy first; the demo reference
/// firmware matches it, newer IDF-5.x builds the second):
///
///  * Legacy: the 4-byte sequence `E9 01 0C 0D` → `0C 0D D9 01`.
///  * IDF 5.x: `xCoreID` is `xTaskCreateUniversal`'s 7th (stack) arg —
///    `movi.n aT, 1` whose value is stored via `s32i.n aT, a1, 0`. Recover
///    `T` from the store (`[0x09|(T<<4), 0x01]`) and zero the `movi.n aT, 1`
///    immediate (`[0x0C, 0x10|T]` → `[0x0C, T]`).
///
/// Returns `(patched_addr, which_shape)`, or `None` if neither matches
/// (firmware already targets core 0, or an unrecognized layout).
pub fn repin_loop_task(bus: &mut dyn Bus, app_main_addr: u32) -> Option<(u32, &'static str)> {
    const SCAN: u32 = 96;
    let mut w: Vec<u8> = Vec::with_capacity(SCAN as usize);
    for off in 0..SCAN {
        match bus.read_u8((app_main_addr + off) as u64) {
            Ok(b) => w.push(b),
            Err(_) => break,
        }
    }
    // Legacy pattern.
    let target = [0xE9_u8, 0x01, 0x0C, 0x0D];
    let swap = [0x0C_u8, 0x0D, 0xD9, 0x01];
    if let Some((i, _)) = w.windows(4).enumerate().find(|(_, x)| *x == target) {
        let addr = app_main_addr + i as u32;
        for (j, b) in swap.iter().enumerate() {
            let _ = bus.write_u8(addr as u64 + j as u64, *b);
        }
        return Some((addr, "legacy"));
    }
    // IDF 5.x: locate `s32i.n aT, a1, 0`, recover T, zero its movi.n source.
    for k in 0..w.len().saturating_sub(1) {
        if (w[k] & 0x0F) == 0x09 && w[k + 1] == 0x01 {
            let t = w[k] >> 4;
            let movi_hi = 0x10 | t;
            if let Some(mi) =
                (0..w.len().saturating_sub(1)).find(|&m| w[m] == 0x0C && w[m + 1] == movi_hi)
            {
                let addr = app_main_addr + mi as u32 + 1;
                let _ = bus.write_u8(addr as u64, t); // movi.n aT, 0
                return Some((addr, "idf5"));
            }
        }
    }
    None
}

/// `_xtos_set_intlevel(intlevel) -> prev` — BROM helper that sets the
/// CPU's PS.INTLEVEL field via `rsil`-equivalent. Returns the previous
/// INTLEVEL. FreeRTOS-on-Xtensa critical-section exit calls this with the
/// caller's saved INTLEVEL to restore interrupt unmasking.
///
/// We previously stubbed this as a no-op which silently kept INTLEVEL
/// pinned high — fine while no interrupts were modeled, fatal once
/// FreeRTOS started gating the FROM_CPU IPI behind a level-3 critical
/// section. Without this thunk doing anything, the IPI bit remains
/// pending forever and ipc_task never yields.
pub fn xtos_set_intlevel(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let new_level = (cpu.regs.read_logical(n + 2) & 0xF) as u8;
    let prev_level = cpu.ps.intlevel();
    cpu.ps.set_intlevel(new_level);
    RomThunkBank::return_with(cpu, prev_level as u32);
    Ok(())
}

/// `esp_rom_route_intr_matrix(cpu, src, intnum)` — write the intmatrix
/// mapping register so the source-id → CPU-internal-int routing reflects
/// what ESP-IDF programmed. ESP32-classic layout:
///   DPORT_PRO_MAC_INTR_MAP_REG = 0x3FF0_0104 (source 0 for PRO_CPU)
///   DPORT_APP_MAC_INTR_MAP_REG = 0x3FF0_0208 (source 0 for APP_CPU)
/// Each source consumes 4 bytes; the 5-bit `intnum` selects which CPU
/// internal interrupt the source's edge raises on the target CPU.
///
/// We need this to actually take effect (no longer a no-op stub) because
/// the test-level cross-core IPI bridge reads back PRO_FROM_CPU_INTR0_MAP
/// at 0x3FF0_0164 to know which INTERRUPT bit to raise on each FROM_CPU
/// trigger write.
pub fn esp_rom_route_intr_matrix(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let core = cpu.regs.read_logical(n + 2);
    let src = cpu.regs.read_logical(n + 3);
    let intnum = cpu.regs.read_logical(n + 4) & 0x1F;
    let base: u32 = if core == 0 { 0x3FF0_0104 } else { 0x3FF0_0208 };
    let addr = base.wrapping_add(src.wrapping_mul(4));
    tracing::trace!(
        "esp_rom_route_intr_matrix: cpu={} src={} intnum={} addr=0x{:08x}",
        core,
        src,
        intnum,
        addr
    );
    let _ = bus.write_u32(addr as u64, intnum);
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// ESP32-S3 `intr_matrix_set` / `esp_rom_route_intr_matrix` @ `0x4000_1b54`.
///
/// Same `(cpu_no, model_num, intr_num)` ABI as classic, but the map tables
/// live at `DR_REG_INTERRUPT_BASE` (`0x600C_2000`):
///   CORE0: `base + 4 * source`
///   CORE1: `base + 0x800 + 4 * source`
///
/// Without this, harness ROM leaves the call as a default nop → FreeRTOS
/// `esp_intr_alloc` never binds FROM_CPU / systimer sources → yield IRQs
/// never fire → only the first task runs (`ipc_task` livelock, no
/// `main_task` / `initArduino` / UART).
pub fn esp32s3_rom_route_intr_matrix(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let core = cpu.regs.read_logical(n + 2);
    let src = cpu.regs.read_logical(n + 3);
    let intnum = cpu.regs.read_logical(n + 4) & 0x1F;
    let base: u32 = if core == 0 {
        0x600C_2000
    } else {
        0x600C_2000 + 0x800
    };
    let addr = base.wrapping_add(src.wrapping_mul(4));
    tracing::trace!(
        "esp32s3_rom_route_intr_matrix: cpu={} src={} intnum={} addr=0x{:08x}",
        core,
        src,
        intnum,
        addr
    );
    let _ = bus.write_u32(addr as u64, intnum);
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// Generic NOP thunk that returns 0. Useful for ROM functions whose
/// behaviour we don't model but whose return value the caller needs to
/// pass through (e.g. cache config, frequency update, busy-wait).
// CHEAT(NOP): swallows the call and returns 0 — real: execute the function's
// actual effect. Installed at ~25 ROM/IDF addresses. See FIDELITY.md §A.
pub fn nop_return_zero(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

// RETIRED: gxepd_write_command / gxepd_write_data / spi_class_transfer (the
// GxEPD2 panel-bypass and SPI register-shim thunks) were deleted once the real
// compiled firmware was proven to paint through real SPI3 registers + real DC
// GPIO (tests/e2e_labwired_ereader.rs: 431 SPI3 transactions → refresh, no
// per-byte thunk). Do not reinstate — the data path is real now. See FIDELITY.md §A.

/// Custom thunk for `xthal_window_spill_nw` / `xthal_window_spill`.
///
/// The Xtensa HAL routine walks the AR file and for each live AR slot
/// (WS[slot]=1) writes the slot's a0..a3 to its stack save area at
/// *(slot.a1 - 16 .. slot.a1 - 4). The sim's transparent shadow-spill on
/// `CALL{n}` saves displaced slot values to a per-WB shadow stack but
/// leaves the WS bit set — so when the firmware-side spill walks WS, it
/// sees the displaced slot as live and reads CALLEE's a0..a3 / a1 from
/// the physical AR file (which the callee has likely clobbered with its
/// own locals; in particular slot.a1 is often 0, making the spill store
/// to `0 - 16 = 0xfffffff0` and trap).
///
/// This thunk does the spill semantically using the sim's knowledge:
/// for each WS=1 slot, if a shadow snapshot exists for that slot the
/// pre-displacement [a0, a1, a2, a3] is used; otherwise the physical AR
/// values. If `a1` is 0 we skip (no save area to write to — happens for
/// the top-of-stack initial frame).
///
/// Returns via plain RET.N semantics (PC ← a0), matching the function's
/// terminal `0x0d 0xf0`. We ignore PS.CALLINC because some firmware code
/// paths reach this routine via `j` (jump) rather than CALL{n}, leaving
/// CALLINC stale from an unrelated outer frame.
// CHEAT(THUNK-ROM): emulates xthal_window_spill (flush windowed regs to stack)
// in Rust — real: the ROM routine spills via ROTW + S32I and ends with
// WINDOWSTART = 1<<WB (only the current frame live). See FIDELITY.md §A.
//
// Critical for FreeRTOS solicited yield: after another task runs, physical
// ARs are clobbered. If we leave WS bits set, RETW treats outer frames as
// "live" and uses garbage physical a0..a7 (seen: a6=&xKernelLock → 1,
// WB 11→9 after vPortExitCritical). Real spill clears those WS bits so
// RETW takes WindowUnderflow and the UF handler reloads from the stack
// save area we write here.
pub fn xthal_window_spill_thunk(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    // Semantic spill via CPU helper (OF/UF save layout + WINDOWSTART=1<<WB).
    // Shared with interrupt-entry spill so FreeRTOS task switches do not need
    // this thunk to have run first. See `XtensaLx7::spill_call_preserve_to_stack`.
    cpu.spill_call_preserve_to_stack(bus);
    // Explicit yield path: drop IRQ window snapshots so a later RFE in
    // another task cannot restore this task's call_preserve.
    cpu.clear_irq_window_stack();

    // Plain RET.N: PC ← a0 (CALL0 / _nw entry).
    cpu.pc = cpu.regs.read_logical(0);
    Ok(())
}

/// Thunk for `__assert_func` / `panic_abort` / `abort` — functions that
/// real-silicon C convention treats as `noreturn`. Stubbing them as
/// nop_return_zero would silently return into the caller, which on the
/// FreeRTOS xQueue assertion paths produces a tight loop:
/// `assert → return → check failed cond → jump back to assert`.
///
/// Instead, halt the calling CPU and print the assertion arguments so the
/// operator sees what blew up.  The caller never re-runs, so the loop
/// breaks.  The OTHER CPU (if dual-core) keeps running.
// CHEAT(NOP): halts the sim on abort() instead of running the real abort path
// (which would print a backtrace via the panic handler). See FIDELITY.md §A.
pub fn abort_halt(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    use core::sync::atomic::{AtomicU32, Ordering};
    static FIRST_PRINT: AtomicU32 = AtomicU32::new(0);
    if FIRST_PRINT.fetch_add(1, Ordering::Relaxed) < 5 {
        let n = cpu.ps.callinc() * 4;
        let core_id = (cpu.sr.read(crate::cpu::xtensa_sr::PRID) >> 13) & 1;
        let a10 = cpu.regs.read_logical(n + 2);
        let a11 = cpu.regs.read_logical(n + 3);
        let a12 = cpu.regs.read_logical(n + 4);
        let a13 = cpu.regs.read_logical(n + 5);
        eprintln!(
            "[abort_halt] core={core_id} pc=0x{:08x} a0=0x{:08x} a10=0x{:08x} a11=0x{:08x} a12=0x{:08x} a13=0x{:08x}",
            cpu.pc,
            cpu.regs.read_logical(0),
            a10,
            a11,
            a12,
            a13
        );
        // Dump FreeRTOS SMP ready-list health (Arduino-ESP32 IDF layout for this ELF).
        let read32 = |addr: u32| bus.read_u32(addr as u64).unwrap_or(0xEEEE_EEEE);
        let read8 = |addr: u32| bus.read_u8(addr as u64).unwrap_or(0xEE);
        let tcb0 = read32(0x3ffc_27c8);
        let tcb1 = read32(0x3ffc_27cc);
        eprintln!(
            "[abort_halt] pxCurrentTCBs=[{tcb0:#010x},{tcb1:#010x}] xSchedulerRunning={} uxTopReadyPriority={} xYieldPending=[{},{}] uxSchedulerSuspended=[{},{}]",
            read32(0x3ffc_2540),
            read32(0x3ffc_2544),
            read32(0x3ffc_2534),
            read32(0x3ffc_2538),
            read32(0x3ffc_2518),
            read32(0x3ffc_251c)
        );
        let idle0 = read32(0x3ffc_2520);
        let idle1 = read32(0x3ffc_2524);
        eprintln!("[abort_halt] xIdleTaskHandle=[{idle0:#010x},{idle1:#010x}]");
        // ESP-IDF TCB: pxTopOfStack @0, then xStateListItem (ListItem_t = 5×u32).
        // ListItem: xItemValue, pxNext, pxPrevious, pvOwner, pxContainer.
        for (label, tcb) in [
            ("idle0", idle0),
            ("idle1", idle1),
            ("cur0", tcb0),
            ("cur1", tcb1),
        ] {
            if tcb < 0x3ff0_0000 || tcb >= 0x4000_0000 {
                continue;
            }
            let top = read32(tcb);
            let state = tcb + 4;
            let s_next = read32(state + 4);
            let s_prev = read32(state + 8);
            let s_owner = read32(state + 12);
            let s_cont = read32(state + 16);
            let event = tcb + 4 + 20;
            let e_cont = read32(event + 16);
            eprintln!(
                "[abort_halt] {label} tcb={tcb:#010x} top={top:#010x} state: next={s_next:#010x} prev={s_prev:#010x} owner={s_owner:#010x} cont={s_cont:#010x} event.cont={e_cont:#010x}"
            );
            // Dump a window of TCB words for affinity/core fields.
            let mut line = String::new();
            for w in 0..16u32 {
                line.push_str(&format!(" {:08x}", read32(tcb + w * 4)));
            }
            eprintln!("[abort_halt] {label} words:{line}");
        }
        let susp = 0x3ffc_2550u32;
        eprintln!(
            "[abort_halt] lists: ready0={:#010x} ready1={:#010x} suspended={:#010x} n={} end.next={:#010x} end.prev={:#010x} pendingReady={:#010x}",
            0x3ffc_25d4u32,
            0x3ffc_25e8u32,
            susp,
            read32(susp),
            read32(susp + 12),
            read32(susp + 16),
            0x3ffc_257cu32
        );
        // IDLE1 xCoreID at TCB+68; notify state near end of TCB
        for (label, tcb) in [("idle1", idle1), ("cur1", tcb1)] {
            if (0x3ff0_0000..0x4000_0000).contains(&tcb) {
                let core_id = read32(tcb + 68) as i32;
                let top = read32(tcb);
                eprintln!(
                    "[abort_halt] {label} xCoreID={core_id} (raw={:#x})",
                    core_id as u32
                );
                // Stack backtrace words (return addresses often have 0x400xxxxx or 0x8xxxxxxx)
                let mut line = String::new();
                for off in 0..16u32 {
                    let w = read32(top.wrapping_add(off * 4));
                    line.push_str(&format!(" {w:08x}"));
                }
                eprintln!("[abort_halt] {label} stack@top:{line}");
                // Notify state: search for non-zero in a wider TCB window
                let mut nline = String::new();
                for w in 16..32u32 {
                    nline.push_str(&format!(" {:08x}", read32(tcb + w * 4)));
                }
                eprintln!("[abort_halt] {label} tcb+64:{nline}");
            }
        }
        // Walk suspended list (max 8)
        let mut it = read32(susp + 12);
        let end = susp + 8;
        for i in 0..8 {
            if it == 0 || it == end {
                break;
            }
            let owner = read32(it + 12);
            let cont = read32(it + 16);
            let next = read32(it + 4);
            let name_ptr = owner + 52;
            let mut name = Vec::new();
            for off in 0..12u32 {
                match bus.read_u8((name_ptr + off) as u64) {
                    Ok(b) if (0x20..0x7f).contains(&b) => name.push(b),
                    _ => break,
                }
            }
            let name_s = String::from_utf8_lossy(&name);
            let top = read32(owner);
            let stack_base = read32(owner + 48);
            let core_id = read32(owner + 68) as i32;
            eprintln!(
                "[abort_halt] susp[{i}] item={it:#010x} tcb={owner:#010x} top={top:#010x} stack={stack_base:#010x} core={core_id} name≈{name_s:?}"
            );
            it = next;
        }
        // List_t on Xtensa ESP-IDF: uxNumberOfItems (u32), pxIndex (ptr), xListEnd {xItemValue, pxNext, pxPrevious}
        // = 5 words = 20 bytes per list. configMAX_PRIORITIES is typically 25.
        const LIST_STRIDE: u32 = 20;
        let ready_base = 0x3ffc_25d4u32;
        for prio in 0..25u32 {
            let base = ready_base + prio * LIST_STRIDE;
            let nitems = read32(base);
            if nitems != 0 {
                let px_index = read32(base + 4);
                let end_next = read32(base + 12); // xListEnd.pxNext
                eprintln!(
                    "[abort_halt] ready[{prio}]: n={nitems} pxIndex={px_index:#010x} end.pxNext={end_next:#010x}"
                );
            }
        }
        // Pending ready list (tasks woken while scheduler suspended)
        let pend = 0x3ffc_257cu32;
        eprintln!(
            "[abort_halt] xPendingReadyList n={} end.pxNext={:#010x}",
            read32(pend),
            read32(pend + 12)
        );
        // Try to print assert strings from flash if they look like pointers
        for (label, p) in [("file", a10), ("func", a12), ("expr", a13)] {
            if (0x3f40_0000..0x3f80_0000).contains(&p) {
                let mut buf = Vec::new();
                for off in 0..100u32 {
                    match bus.read_u8((p + off) as u64) {
                        Ok(b) if b != 0 && b < 0x7f => buf.push(b),
                        Ok(0) => break,
                        _ => break,
                    }
                }
                if let Ok(s) = std::str::from_utf8(&buf) {
                    if s.len() > 2 {
                        eprintln!("[abort_halt] {label}={s:?} line={a11}");
                    }
                }
            }
        }
        let _ = read8; // silence if unused
    }
    cpu.halted = true;
    Ok(())
}

/// `xQueueCreateMutexStatic(uint8_t ucQueueType, StaticQueue_t *pxStaticQueue)
/// -> QueueHandle_t` — on real silicon the returned handle IS the static
/// buffer (an identity cast). Returning 0 makes `esp_newlib_locks_init`'s
/// "got the static handle back" assertion fire. We echo arg 1 (the
/// caller-allocated buffer pointer).
// CHEAT(THUNK-LIB): echoes the static buffer back as the queue handle instead
// of running xQueueCreateMutexStatic — real: FreeRTOS initializes the queue
// structure. See FIDELITY.md §A.
pub fn x_queue_create_mutex_static_echo(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let static_buf = cpu.regs.read_logical(n + 3);
    RomThunkBank::return_with(cpu, static_buf);
    Ok(())
}

/// `xTaskGetCurrentTaskHandle() -> TaskHandle_t` — returns `pxCurrentTCB[core]`.
///
/// The previous nop_return_zero stub broke `vTaskDelete(NULL)`: vTaskDelete
/// looks up the current task via this getter when called with a NULL arg,
/// then passes it to prvDeleteTLS/prvDeleteTCB which assert non-NULL.
/// Arduino-ESP32's main_task self-deletes after app_main returns, tripping
/// this path right after the scheduler starts.
///
/// `pxCurrentTCB` is a per-core array at a firmware-specific address; the
/// auto-discovered symbol resolves on the Arduino-ESP32 profile. Without
/// the symbol (preset-PC profile, stripped ELF), we fall back to returning
/// 0 to preserve the previous behaviour.
// CHEAT(THUNK-LIB): returns a fabricated current-task handle — real: the
// compiled FreeRTOS scheduler tracks pxCurrentTCB. See FIDELITY.md §A.
pub fn x_task_get_current_task_handle(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let core_id = (cpu.sr.read(crate::cpu::xtensa_sr::PRID) >> 13) & 1;
    let pxcurrenttcb_addr = PX_CURRENT_TCB_ADDR.with(|s| s.get()).unwrap_or(0);
    let handle = if pxcurrenttcb_addr != 0 {
        bus.read_u32(pxcurrenttcb_addr as u64 + (core_id as u64 * 4))
            .unwrap_or(0)
    } else {
        0
    };
    RomThunkBank::return_with(cpu, handle);
    Ok(())
}

thread_local! {
    /// Set by the cli once the `pxCurrentTCB` symbol is resolved from the
    /// firmware ELF. Read by [`x_task_get_current_task_handle`].
    pub static PX_CURRENT_TCB_ADDR: std::cell::Cell<Option<u32>> =
        const { std::cell::Cell::new(None) };
}

/// Stub for queue/semaphore APIs whose real impl asserts `pxQueue != NULL`.
/// We return `pdTRUE` (1) to signal "operation succeeded" without touching
/// any queue state. Used for SPIClass / wire-library calls into
/// `xQueueSemaphoreTake` / `xQueueSemaphoreGive` on a mutex that our stubbed
/// `xQueueCreateMutex` returned NULL for. Real silicon would dereference
/// pxQueue and crash too — this fakes a recursive-mutex held-by-current
/// state, which is fine for the single-CPU sim render path.
///
/// Emits a one-time `tracing::warn!` on first call so silent activation is
/// loud in logs; this stub will hide future regressions where a take
/// *should* block (e.g. when a second consumer is added).
// CHEAT(NOP): returns pdTRUE unconditionally — real: the wrapped FreeRTOS call
// computes its result. See FIDELITY.md §A.
pub fn return_pd_true(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            target: "labwired_core::esp32s3",
            "return_pd_true stub active: xQueueSemaphoreTake / xQueueGenericSend will \
             unconditionally succeed on stubbed mutexes. Any test that depends on a \
             take blocking will silently pass."
        );
    }
    RomThunkBank::return_with(cpu, 1);
    Ok(())
}

/// `spi_t *spiStartBus(uint8_t spi_num, ...)` — Arduino-ESP32's SPI bus
/// initializer. Real impl allocates a `spi_t`, programs DPORT clock-enable
/// registers, configures the SPI peripheral. We skip the DPORT/peripheral
/// dance and hand back a tiny static `spi_t` whose only populated field is
/// `dev` (offset 0) — the SPI peripheral base address. Downstream callers
/// (`spiTransferByte`) read `spi->dev` and write the peripheral registers
/// directly, which our `spi3` peripheral catches.
///
/// `spi_num` maps to the peripheral base:
///   0 → SPI0 (flash, not modelled): return NULL
///   1 → SPI1 (flash, not modelled): return NULL
///   2 → HSPI/SPI2 (0x3FF64000)
///   3 → VSPI/SPI3 (0x3FF65000) — the default Arduino `SPI` instance
///
/// The `spi_t` blob is per-num and lives in DRAM at a reserved address.
// CHEAT(THUNK-LIB): fakes the Arduino spiStartBus() so later transfers find a
// "bus up" — real: the compiled IDF/Arduino SPI bus-init code runs against the
// SPI peripheral + GPIO matrix. See FIDELITY.md §A.
pub fn spi_start_bus_fake(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let spi_num = cpu.regs.read_logical(n + 2) & 0xFF;
    let (dev, fake_spi_t) = fake_spi_for_num(spi_num);
    if fake_spi_t == 0 {
        RomThunkBank::return_with(cpu, 0);
        return Ok(());
    }
    // Populate spi_t: dev (offset 0), lock (offset 4, NULL — our stubs
    // ignore it), num (offset 8). Remaining fields zeroed.
    bus.write_u32(fake_spi_t as u64 + SPI_T_DEV_OFFSET, dev)?;
    bus.write_u32(fake_spi_t as u64 + SPI_T_LOCK_OFFSET, 0)?;
    bus.write_u32(fake_spi_t as u64 + SPI_T_NUM_OFFSET, spi_num)?;
    RomThunkBank::return_with(cpu, fake_spi_t);
    Ok(())
}

// `spi_t` field offsets matching Arduino-ESP32's struct layout.
const SPI_T_DEV_OFFSET: u64 = 0;
const SPI_T_LOCK_OFFSET: u64 = 4;
const SPI_T_NUM_OFFSET: u64 = 8;
// SPIClass field offsets.
const SPI_CLASS_SPI_NUM_OFFSET: u64 = 0;
const SPI_CLASS_SPI_PTR_OFFSET: u64 = 4;
const SPI_CLASS_IN_TRANSACTION_OFFSET: u64 = 24;
// Reserved sim-only scratch region for fake `spi_t` blobs. Sits at the
// top of SRAM1 (0x3FFE_0000–0x4000_0000), well above Arduino-ESP32's
// initial stack (near 0x3FFE_0000, growing downward into DRAM) and any
// plausible heap allocation. DRAM proper (0x3FFA_E000–0x3FFE_0000) is
// off-limits because the firmware allocator can reach the upper pages.
const SIM_FAKE_SPI_T_SPI2: u32 = 0x3FFF_FF00;
const SIM_FAKE_SPI_T_SPI3: u32 = 0x3FFF_FF20;

fn fake_spi_for_num(spi_num: u32) -> (u32, u32) {
    match spi_num {
        2 => (0x3FF6_4000, SIM_FAKE_SPI_T_SPI2),
        3 => (0x3FF6_5000, SIM_FAKE_SPI_T_SPI3),
        _ => (0, 0),
    }
}

/// Wraps SPIClass::beginTransaction so the first call lazily initializes
/// `this->_spi` to a fake spi_t pointing at the matching SPI peripheral
/// base. The sketch never calls SPI.begin() explicitly — GxEPD2 just
/// assumes the bus is up — so without this hook spiTransferByte's
/// `if (spi == NULL) return` short-circuits every transfer and no bytes
/// reach the panel.
///
/// After ensuring _spi is non-NULL, we return pdTRUE to satisfy the
/// caller's `bnei a10, 1, retry` check (it expects a take to succeed).
// CHEAT(THUNK-LIB): fakes SPIClass::beginTransaction (populates _spi->dev so
// transfer finds SPI3) — real: the compiled Arduino code configures clock/mode
// on the SPI peripheral. See FIDELITY.md §A.
pub fn spi_class_begin_transaction(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    use crate::peripherals::esp32::spi::{REG_USER, USER_USR_MOSI_BIT};

    let n = cpu.ps.callinc() * 4;
    let this = cpu.regs.read_logical(n + 2);
    let spi_num = bus.read_u32(this as u64 + SPI_CLASS_SPI_NUM_OFFSET)? & 0xFF;
    let cur_spi = bus.read_u32(this as u64 + SPI_CLASS_SPI_PTR_OFFSET)?;
    if cur_spi == 0 {
        let (dev, fake_spi_t) = fake_spi_for_num(spi_num);
        if fake_spi_t != 0 {
            bus.write_u32(fake_spi_t as u64 + SPI_T_DEV_OFFSET, dev)?;
            bus.write_u32(fake_spi_t as u64 + SPI_T_LOCK_OFFSET, 0)?;
            bus.write_u32(fake_spi_t as u64 + SPI_T_NUM_OFFSET, spi_num)?;
            bus.write_u32(this as u64 + SPI_CLASS_SPI_PTR_OFFSET, fake_spi_t)?;
        }
    }
    // The real `spiStartBus` programs the SPI peripheral's USER register
    // to enable the MOSI-output phase. We skipped that, so the first
    // `spiTransferByte` writes data into the FIFO and sets the CMD.USR
    // start bit but the peripheral never strobes bytes out to attached
    // devices (kick_user_transaction returns early when USR_MOSI is
    // clear). Set it once here so subsequent transfers fire.
    let cur_spi = bus.read_u32(this as u64 + SPI_CLASS_SPI_PTR_OFFSET)?;
    if cur_spi != 0 {
        let dev = bus.read_u32(cur_spi as u64 + SPI_T_DEV_OFFSET)?;
        if dev != 0 {
            let cur_user = bus.read_u32(dev as u64 + REG_USER)?;
            if cur_user & USER_USR_MOSI_BIT == 0 {
                bus.write_u32(dev as u64 + REG_USER, cur_user | USER_USR_MOSI_BIT)?;
            }
        }
    }
    // Mark _inTransaction so endTransaction's take/give bookkeeping stays
    // consistent.
    bus.write_u8(this as u64 + SPI_CLASS_IN_TRANSACTION_OFFSET, 1)?;
    RomThunkBank::return_with(cpu, 1);
    Ok(())
}

/// Debug thunk for `vListInsert(List_t *pxList, ListItem_t *pxNewListItem)`.
/// Dumps the list state for the first few calls, then returns without
/// performing the insertion. Used to diagnose infinite-loop bugs in the
/// FreeRTOS scheduler list walker (typically caused by an uninitialised
/// list lacking the `xListEnd` sentinel with `xItemValue == portMAX_DELAY`).
pub fn vlist_insert_debug(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    use core::sync::atomic::{AtomicU32, Ordering};
    static CALLS: AtomicU32 = AtomicU32::new(0);
    let n = CALLS.fetch_add(1, Ordering::Relaxed);
    if n < 20 {
        // Args under CALL8 windowing: pxList in a2, pxNewListItem in a3
        // (post-rotation logical slots a2/a3 since the caller used call8).
        let callinc = cpu.ps.callinc();
        let base = if callinc == 0 { 0 } else { callinc * 4 };
        let px_list = cpu.regs.read_logical(base + 2);
        let px_item = cpu.regs.read_logical(base + 3);
        let item_value = bus.read_u32(px_item as u64).unwrap_or(0xDEAD_BEEF);
        let list_num = bus.read_u32(px_list as u64).unwrap_or(0xDEAD_BEEF); // uxNumberOfItems
        let list_idx = bus.read_u32(px_list as u64 + 4).unwrap_or(0xDEAD_BEEF); // pxIndex
        let end_val = bus.read_u32(px_list as u64 + 8).unwrap_or(0xDEAD_BEEF); // xListEnd.xItemValue
        let end_next = bus.read_u32(px_list as u64 + 12).unwrap_or(0xDEAD_BEEF);
        let end_prev = bus.read_u32(px_list as u64 + 16).unwrap_or(0xDEAD_BEEF);
        eprintln!(
            "[vListInsert #{n:>3}] pxList=0x{px_list:08x} num={list_num} idx=0x{list_idx:08x} end.value=0x{end_val:08x} end.next=0x{end_next:08x} end.prev=0x{end_prev:08x} | item=0x{px_item:08x} item.value=0x{item_value:08x}"
        );
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// Generic thunk that returns a fixed dummy pointer (0x3F40_0100, inside
/// the flash dcache window we populate with the app-image header). Used
/// for functions that must return a non-NULL "found it" pointer to avoid
/// upstream NULL-assert panics, but whose contents we don't actually use.
// CHEAT(NOP): returns a fabricated non-null pointer so the caller proceeds —
// real: the function allocates/returns a genuine structure. See FIDELITY.md §A.
pub fn nop_return_fake_ptr(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0x3F40_0100);
    Ok(())
}

/// `__getreent()` / newlib reentrancy: real Xtensa+FreeRTOS returns the
/// per-task `_reent` struct stored in task-local storage. Our single-task
/// sim has no task struct — return a pointer to a fixed DRAM region
/// (0x3FFB_F000) which is part of the SRAM2 (`dram`) peripheral we
/// always provision in `configure_xtensa_esp32`. The 4 KB region is
/// initialized to zero by `RamPeripheral::new`, which matches newlib's
/// `_REENT_INIT_ZERO` — `errno` reads as 0, all FILE* slots are null,
/// no allocator state. Adequate for sketches that don't actually use
/// stdio/errno on the panel-render path.
// CHEAT(NOP): returns a fixed DRAM address as the per-task reent struct — real:
// __getreent returns the running task's _reent. See FIDELITY.md §A.
pub fn getreent_dram_fake_ptr(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0x3FFB_F000);
    Ok(())
}

thread_local! {
    /// Set by [`set_appcpu_boot_addr`], drained by [`Machine::step`].
    /// Thread-local because the sim is single-threaded within any one
    /// `Machine`; concurrent machines on parallel threads each get their
    /// own slot. Cleared to None after Machine reads it.
    pub static APPCPU_BOOT_ADDR: core::cell::Cell<Option<u32>> = const { core::cell::Cell::new(None) };

    /// Set by the SYSTEM_CORE_1_CONTROL peripheral when the PRO_CPU clears the
    /// CORE_1_RESETING bit (the real APP_CPU-out-of-reset edge); drained by the
    /// rom-boot run loop, which then unhalts the APP_CPU at the ROM reset
    /// vector so it boots the real ROM exactly like silicon. Faithful path —
    /// no firmware-symbol hooks.
    pub static APPCPU_RESET_RELEASED: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };

    /// Firmware DRAM byte addresses of the dual-core startup handshake
    /// flags. Populated per-firmware via [`set_appcpu_up_flags`] once the
    /// symbols are resolved from the ELF; consumed by
    /// [`ets_set_appcpu_boot_addr`] to model APP_CPU bring-up. Empty for
    /// stripped/preset-PC profiles, in which case the thunk does nothing
    /// extra. Thread-local for the same per-Machine isolation reason as
    /// `APPCPU_BOOT_ADDR`.
    pub static APPCPU_UP_FLAGS: core::cell::RefCell<Vec<u32>> =
        const { core::cell::RefCell::new(Vec::new()) };
}

/// Monotonic-counter thunk for `esp_timer_impl_get_counter_reg()` and
/// similar 32-bit time-source readers. Returns an ever-increasing value
/// (steps of 1000 per call) so callers polling for timeout deadlines
/// actually make progress instead of looping forever.
// CHEAT(THUNK-LIB): returns an incrementing counter as a fake timestamp — real:
// the IDF reads a hardware timer (systimer/CCOUNT). See FIDELITY.md §A.
pub fn monotonic_counter_32(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    static MONOTONIC_TICKS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    let v = MONOTONIC_TICKS.fetch_add(1000, core::sync::atomic::Ordering::Relaxed);
    RomThunkBank::return_with(cpu, v);
    Ok(())
}

/// `esp_chip_info(esp_chip_info_t *out)` — fill the output struct with a
/// plausible-looking ESP32 chip ID, then return.
///
/// ESP-IDF's `app_main_startup` reads `out->full_revision` (byte at
/// struct+10) and panics if it's < the firmware's `min_chip_rev`. With a
/// plain `nop_return_zero` the struct stays uninitialized, that byte is
/// random stack garbage, and any sketch built against `min_chip_rev > 0`
/// crashes before reaching app code. We fill the struct with chip_model =
/// 1 (ESP32), revision = 3, cores = 2 — enough to satisfy the assert.
///
/// Args (Xtensa C ABI, pre-ENTRY caller view so the first arg is at
/// a[CALLINC*4 + 2]): out_ptr.
// CHEAT(THUNK-LIB): writes a canned esp_chip_info_t — real: reads eFuse/DPORT
// to report cores/features/revision. See FIDELITY.md §A.
pub fn esp_chip_info_stub(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let out_ptr = cpu.regs.read_logical(n + 2);
    if out_ptr != 0 {
        // Layout used by ESP-IDF v4.4 / Arduino-ESP32 2.0.x:
        //   off 0: model (4B)   = 1 (ESP32)
        //   off 4: features (4B) = 0
        //   off 8: revision (2B) = 3 (well above any min_chip_rev)
        //   off 10: cores (1B)   = 2
        // Newer ESP-IDF moves things around (full_revision at 10/12/etc.),
        // so we just splat reasonable values across the first 16 bytes —
        // any byte the firmware reads will be non-zero / >= 2.
        let bytes: [u8; 16] = [
            0x01, 0x00, 0x00, 0x00, // model = ESP32
            0x00, 0x00, 0x00, 0x00, // features
            0x03, 0x00, // revision = 3
            0x02, // cores = 2
            0x03, 0x00, 0x00, 0x00, 0x00, // full_revision + pad
        ];
        for (i, &b) in bytes.iter().enumerate() {
            let _ = bus.write_u8(out_ptr as u64 + i as u64, b);
        }
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `memcpy(dst, src, n) -> dst` — byte-wise copy via the bus.
///
/// Args (Xtensa C ABI, post-ENTRY view from caller's frame so we read
/// a[CALLINC*4 + 2..=4]):
///   a2 = dst pointer  /  a3 = src pointer  /  a4 = byte count
pub fn rom_memcpy(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let dst = cpu.regs.read_logical(n + 2);
    let src = cpu.regs.read_logical(n + 3);
    let count = cpu.regs.read_logical(n + 4);
    for i in 0..count {
        let b = bus.read_u8(src.wrapping_add(i) as u64)?;
        bus.write_u8(dst.wrapping_add(i) as u64, b)?;
    }
    RomThunkBank::return_with(cpu, dst);
    Ok(())
}

/// `memset(dst, value, n) -> dst` — byte-wise fill via the bus.
///
/// Args (Xtensa C ABI, same window-rotation rules as `rom_memcpy`):
///   a2 = dst pointer / a3 = byte value (low 8 bits) / a4 = byte count
/// Used by every esp-hal hello-world to zero its .bss before main.
pub fn rom_memset(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let dst = cpu.regs.read_logical(n + 2);
    let value = (cpu.regs.read_logical(n + 3) & 0xFF) as u8;
    let count = cpu.regs.read_logical(n + 4);
    for i in 0..count {
        bus.write_u8(dst.wrapping_add(i) as u64, value)?;
    }
    RomThunkBank::return_with(cpu, dst);
    Ok(())
}

/// `memmove(dst, src, n) -> dst` — handles overlapping copy direction.
pub fn rom_memmove(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let dst = cpu.regs.read_logical(n + 2);
    let src = cpu.regs.read_logical(n + 3);
    let count = cpu.regs.read_logical(n + 4);
    // Copy direction matters only when src/dst overlap.
    if dst > src && dst - src < count {
        // Overlap — copy backwards.
        for i in (0..count).rev() {
            let b = bus.read_u8(src.wrapping_add(i) as u64)?;
            bus.write_u8(dst.wrapping_add(i) as u64, b)?;
        }
    } else {
        for i in 0..count {
            let b = bus.read_u8(src.wrapping_add(i) as u64)?;
            bus.write_u8(dst.wrapping_add(i) as u64, b)?;
        }
    }
    RomThunkBank::return_with(cpu, dst);
    Ok(())
}

/// Helper: read a 64-bit value from `a[n+lo]:a[n+hi]` (low half:high half),
/// where `n = callinc * 4`.
fn read_u64_args(cpu: &XtensaLx7, lo: u8, hi: u8) -> u64 {
    let n = cpu.ps.callinc() * 4;
    let l = cpu.regs.read_logical(n + lo) as u64;
    let h = cpu.regs.read_logical(n + hi) as u64;
    (h << 32) | l
}

/// Helper: write a 64-bit return value to `a[n+2]:a[n+3]` and jump back.
///
/// `return_with` pops the windowed-call shadow spill AFTER setting the
/// return value — and that pop range covers logical offsets n+2..n+5
/// (the callee's a2-a5). Anything we write to n+3 BEFORE return_with
/// would be clobbered by the shadow pop. So we write n+3 AFTER
/// return_with has finished its bookkeeping; that's the only window
/// where the high half stays intact through to the caller's view.
fn return_u64(cpu: &mut XtensaLx7, v: u64) {
    let n = cpu.ps.callinc() * 4;
    RomThunkBank::return_with(cpu, v as u32);
    cpu.regs.write_logical(n + 3, (v >> 32) as u32);
}

/// `__ashldi3(u64 v, i32 count) -> u64` — left shift.
/// Args: a2:a3 = v lo:hi, a4 = count.
pub fn rom_ashldi3(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let v = read_u64_args(cpu, 2, 3);
    let n = cpu.ps.callinc() * 4;
    let count = cpu.regs.read_logical(n + 4) & 0x3F;
    return_u64(cpu, v.wrapping_shl(count));
    Ok(())
}

/// `__ashrdi3(i64 v, i32 count) -> i64` — arithmetic right shift.
pub fn rom_ashrdi3(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let v = read_u64_args(cpu, 2, 3) as i64;
    let n = cpu.ps.callinc() * 4;
    let count = cpu.regs.read_logical(n + 4) & 0x3F;
    return_u64(cpu, v.wrapping_shr(count) as u64);
    Ok(())
}

/// `__lshrdi3(u64 v, i32 count) -> u64` — logical right shift.
pub fn rom_lshrdi3(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let v = read_u64_args(cpu, 2, 3);
    let n = cpu.ps.callinc() * 4;
    let count = cpu.regs.read_logical(n + 4) & 0x3F;
    return_u64(cpu, v.wrapping_shr(count));
    Ok(())
}

/// `__divdi3(i64, i64) -> i64` — signed 64-bit divide.
pub fn rom_divdi3(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = read_u64_args(cpu, 2, 3) as i64;
    let d = read_u64_args(cpu, 4, 5) as i64;
    let q = n.checked_div(d).unwrap_or(i64::MIN);
    return_u64(cpu, q as u64);
    Ok(())
}

/// `__moddi3(i64, i64) -> i64` — signed 64-bit mod.
pub fn rom_moddi3(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = read_u64_args(cpu, 2, 3) as i64;
    let d = read_u64_args(cpu, 4, 5) as i64;
    let r = n.checked_rem(d).unwrap_or(0);
    return_u64(cpu, r as u64);
    Ok(())
}

/// `__umoddi3(u64, u64) -> u64`.
pub fn rom_umoddi3(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = read_u64_args(cpu, 2, 3);
    let d = read_u64_args(cpu, 4, 5);
    let r = n.checked_rem(d).unwrap_or(0);
    return_u64(cpu, r);
    Ok(())
}

// ── Classic ESP32 ROM MD5 (MD5Context: buf[4], bits[2], in[64]) ─────────────
//
// Partition-table load (`CONFIG_PARTITION_TABLE_MD5`) hashes every 32-byte
// entry then compares against the 0xEBEB trailer. These are real MD5, not
// firmware product-path thunks — they model BROM at 0x4005_da7c/a9c/b1c.

const MD5_CTX_BUF: u32 = 0; // 4 × u32
const MD5_CTX_BITS: u32 = 16; // 2 × u32
const MD5_CTX_IN: u32 = 24; // 64 bytes
const MD5_CTX_SIZE: u32 = 88;

fn md5_f(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | (!x & z)
}
fn md5_g(x: u32, y: u32, z: u32) -> u32 {
    (x & z) | (y & !z)
}
fn md5_h(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}
fn md5_i(x: u32, y: u32, z: u32) -> u32 {
    y ^ (x | !z)
}
fn md5_rotl(x: u32, n: u32) -> u32 {
    x.rotate_left(n)
}

fn md5_step(w: &mut [u32; 4], in_block: &[u32; 16]) {
    let (mut a, mut b, mut c, mut d) = (w[0], w[1], w[2], w[3]);
    macro_rules! round {
        ($f:ident, $a:ident, $b:ident, $c:ident, $d:ident, $k:expr, $s:expr, $t:expr) => {{
            $a = $b.wrapping_add(md5_rotl(
                $a.wrapping_add($f($b, $c, $d))
                    .wrapping_add(in_block[$k])
                    .wrapping_add($t),
                $s,
            ));
        }};
    }
    // Round 1
    round!(md5_f, a, b, c, d, 0, 7, 0xd76a_a478);
    round!(md5_f, d, a, b, c, 1, 12, 0xe8c7_b756);
    round!(md5_f, c, d, a, b, 2, 17, 0x2420_70db);
    round!(md5_f, b, c, d, a, 3, 22, 0xc1bd_ceee);
    round!(md5_f, a, b, c, d, 4, 7, 0xf57c_0faf);
    round!(md5_f, d, a, b, c, 5, 12, 0x4787_c62a);
    round!(md5_f, c, d, a, b, 6, 17, 0xa830_4613);
    round!(md5_f, b, c, d, a, 7, 22, 0xfd46_9501);
    round!(md5_f, a, b, c, d, 8, 7, 0x6980_98d8);
    round!(md5_f, d, a, b, c, 9, 12, 0x8b44_f7af);
    round!(md5_f, c, d, a, b, 10, 17, 0xffff_5bb1);
    round!(md5_f, b, c, d, a, 11, 22, 0x895c_d7be);
    round!(md5_f, a, b, c, d, 12, 7, 0x6b90_1122);
    round!(md5_f, d, a, b, c, 13, 12, 0xfd98_7193);
    round!(md5_f, c, d, a, b, 14, 17, 0xa679_438e);
    round!(md5_f, b, c, d, a, 15, 22, 0x49b4_0821);
    // Round 2
    round!(md5_g, a, b, c, d, 1, 5, 0xf61e_2562);
    round!(md5_g, d, a, b, c, 6, 9, 0xc040_b340);
    round!(md5_g, c, d, a, b, 11, 14, 0x265e_5a51);
    round!(md5_g, b, c, d, a, 0, 20, 0xe9b6_c7aa);
    round!(md5_g, a, b, c, d, 5, 5, 0xd62f_105d);
    round!(md5_g, d, a, b, c, 10, 9, 0x0244_1453);
    round!(md5_g, c, d, a, b, 15, 14, 0xd8a1_e681);
    round!(md5_g, b, c, d, a, 4, 20, 0xe7d3_fbc8);
    round!(md5_g, a, b, c, d, 9, 5, 0x21e1_cde6);
    round!(md5_g, d, a, b, c, 14, 9, 0xc337_07d6);
    round!(md5_g, c, d, a, b, 3, 14, 0xf4d5_0d87);
    round!(md5_g, b, c, d, a, 8, 20, 0x455a_14ed);
    round!(md5_g, a, b, c, d, 13, 5, 0xa9e3_e905);
    round!(md5_g, d, a, b, c, 2, 9, 0xfcef_a3f8);
    round!(md5_g, c, d, a, b, 7, 14, 0x676f_02d9);
    round!(md5_g, b, c, d, a, 12, 20, 0x8d2a_4c8a);
    // Round 3
    round!(md5_h, a, b, c, d, 5, 4, 0xfffa_3942);
    round!(md5_h, d, a, b, c, 8, 11, 0x8771_f681);
    round!(md5_h, c, d, a, b, 11, 16, 0x6d9d_6122);
    round!(md5_h, b, c, d, a, 14, 23, 0xfde5_380c);
    round!(md5_h, a, b, c, d, 1, 4, 0xa4be_ea44);
    round!(md5_h, d, a, b, c, 4, 11, 0x4bde_cfa9);
    round!(md5_h, c, d, a, b, 7, 16, 0xf6bb_4b60);
    round!(md5_h, b, c, d, a, 10, 23, 0xbebf_bc70);
    round!(md5_h, a, b, c, d, 13, 4, 0x289b_7ec6);
    round!(md5_h, d, a, b, c, 0, 11, 0xeaa1_27fa);
    round!(md5_h, c, d, a, b, 3, 16, 0xd4ef_3085);
    round!(md5_h, b, c, d, a, 6, 23, 0x0488_1d05);
    round!(md5_h, a, b, c, d, 9, 4, 0xd9d4_d039);
    round!(md5_h, d, a, b, c, 12, 11, 0xe6db_99e5);
    round!(md5_h, c, d, a, b, 15, 16, 0x1fa2_7cf8);
    round!(md5_h, b, c, d, a, 2, 23, 0xc4ac_5665);
    // Round 4
    round!(md5_i, a, b, c, d, 0, 6, 0xf429_2244);
    round!(md5_i, d, a, b, c, 7, 10, 0x432a_ff97);
    round!(md5_i, c, d, a, b, 14, 15, 0xab94_23a7);
    round!(md5_i, b, c, d, a, 5, 21, 0xfc93_a039);
    round!(md5_i, a, b, c, d, 12, 6, 0x655b_59c3);
    round!(md5_i, d, a, b, c, 3, 10, 0x8f0c_cc92);
    round!(md5_i, c, d, a, b, 10, 15, 0xffef_f47d);
    round!(md5_i, b, c, d, a, 1, 21, 0x8584_5dd1);
    round!(md5_i, a, b, c, d, 8, 6, 0x6fa8_7e4f);
    round!(md5_i, d, a, b, c, 15, 10, 0xfe2c_e6e0);
    round!(md5_i, c, d, a, b, 6, 15, 0xa301_4314);
    round!(md5_i, b, c, d, a, 13, 21, 0x4e08_11a1);
    round!(md5_i, a, b, c, d, 4, 6, 0xf753_7e82);
    round!(md5_i, d, a, b, c, 11, 10, 0xbd3a_f235);
    round!(md5_i, c, d, a, b, 2, 15, 0x2ad7_d2bb);
    round!(md5_i, b, c, d, a, 9, 21, 0xeb86_d391);

    w[0] = w[0].wrapping_add(a);
    w[1] = w[1].wrapping_add(b);
    w[2] = w[2].wrapping_add(c);
    w[3] = w[3].wrapping_add(d);
}

fn md5_read_ctx_buf(bus: &dyn Bus, ctx: u32) -> [u32; 4] {
    let mut buf = [0u32; 4];
    for i in 0..4 {
        buf[i] = bus
            .read_u32(ctx.wrapping_add(MD5_CTX_BUF + i as u32 * 4) as u64)
            .unwrap_or(0);
    }
    buf
}

fn md5_write_ctx_buf(bus: &mut dyn Bus, ctx: u32, buf: &[u32; 4]) {
    for i in 0..4 {
        let _ = bus.write_u32(ctx.wrapping_add(MD5_CTX_BUF + i as u32 * 4) as u64, buf[i]);
    }
}

fn md5_read_bits(bus: &dyn Bus, ctx: u32) -> [u32; 2] {
    [
        bus.read_u32(ctx.wrapping_add(MD5_CTX_BITS) as u64)
            .unwrap_or(0),
        bus.read_u32(ctx.wrapping_add(MD5_CTX_BITS + 4) as u64)
            .unwrap_or(0),
    ]
}

fn md5_write_bits(bus: &mut dyn Bus, ctx: u32, bits: [u32; 2]) {
    let _ = bus.write_u32(ctx.wrapping_add(MD5_CTX_BITS) as u64, bits[0]);
    let _ = bus.write_u32(ctx.wrapping_add(MD5_CTX_BITS + 4) as u64, bits[1]);
}

fn md5_transform_from_in(bus: &mut dyn Bus, ctx: u32) {
    let mut block = [0u32; 16];
    for i in 0..16 {
        block[i] = bus
            .read_u32(ctx.wrapping_add(MD5_CTX_IN + i as u32 * 4) as u64)
            .unwrap_or(0);
    }
    let mut buf = md5_read_ctx_buf(bus, ctx);
    md5_step(&mut buf, &block);
    md5_write_ctx_buf(bus, ctx, &buf);
}

/// `esp_rom_md5_init(md5_context_t *ctx)` — classic RSA MD5 init.
pub fn rom_md5_init(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let ctx = cpu.regs.read_logical(n + 2);
    // Zero whole context then set IV.
    for i in 0..MD5_CTX_SIZE {
        let _ = bus.write_u8(ctx.wrapping_add(i) as u64, 0);
    }
    md5_write_ctx_buf(
        bus,
        ctx,
        &[0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476],
    );
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `esp_rom_md5_update(ctx, buf, len)`.
pub fn rom_md5_update(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let ctx = cpu.regs.read_logical(n + 2);
    let mut data = cpu.regs.read_logical(n + 3);
    let mut len = cpu.regs.read_logical(n + 4);

    let mut bits = md5_read_bits(bus, ctx);
    let t = bits[0];
    bits[0] = t.wrapping_add(len << 3);
    if bits[0] < t {
        bits[1] = bits[1].wrapping_add(1);
    }
    bits[1] = bits[1].wrapping_add(len >> 29);
    md5_write_bits(bus, ctx, bits);

    let mut idx = ((t >> 3) & 0x3f) as u32;
    if idx != 0 {
        let mut part = 64 - idx;
        if len < part {
            part = len;
        }
        for i in 0..part {
            let b = bus.read_u8(data.wrapping_add(i) as u64).unwrap_or(0);
            let _ = bus.write_u8(ctx.wrapping_add(MD5_CTX_IN + idx + i) as u64, b);
        }
        data = data.wrapping_add(part);
        len = len.wrapping_sub(part);
        idx = idx.wrapping_add(part);
        if idx == 64 {
            md5_transform_from_in(bus, ctx);
            idx = 0;
        }
    }
    while len >= 64 {
        for i in 0..64u32 {
            let b = bus.read_u8(data.wrapping_add(i) as u64).unwrap_or(0);
            let _ = bus.write_u8(ctx.wrapping_add(MD5_CTX_IN + i) as u64, b);
        }
        md5_transform_from_in(bus, ctx);
        data = data.wrapping_add(64);
        len = len.wrapping_sub(64);
    }
    for i in 0..len {
        let b = bus.read_u8(data.wrapping_add(i) as u64).unwrap_or(0);
        let _ = bus.write_u8(ctx.wrapping_add(MD5_CTX_IN + idx + i) as u64, b);
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `esp_rom_md5_final(uint8_t digest[16], md5_context_t *ctx)`.
/// Note: classic ESP32 ROM takes (digest, ctx) — opposite of mbedtls order.
pub fn rom_md5_final(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let digest = cpu.regs.read_logical(n + 2);
    let ctx = cpu.regs.read_logical(n + 3);

    let bits = md5_read_bits(bus, ctx);
    let count = ((bits[0] >> 3) & 0x3f) as u32;
    let mut p = count;
    let _ = bus.write_u8(ctx.wrapping_add(MD5_CTX_IN + p) as u64, 0x80);
    p = p.wrapping_add(1);
    if p > 56 {
        while p < 64 {
            let _ = bus.write_u8(ctx.wrapping_add(MD5_CTX_IN + p) as u64, 0);
            p = p.wrapping_add(1);
        }
        md5_transform_from_in(bus, ctx);
        p = 0;
    }
    while p < 56 {
        let _ = bus.write_u8(ctx.wrapping_add(MD5_CTX_IN + p) as u64, 0);
        p = p.wrapping_add(1);
    }
    // Append bit count (little-endian) in last 8 bytes of in[].
    let _ = bus.write_u32(ctx.wrapping_add(MD5_CTX_IN + 56) as u64, bits[0]);
    let _ = bus.write_u32(ctx.wrapping_add(MD5_CTX_IN + 60) as u64, bits[1]);
    md5_transform_from_in(bus, ctx);

    let buf = md5_read_ctx_buf(bus, ctx);
    for i in 0..4 {
        let _ = bus.write_u32(digest.wrapping_add(i as u32 * 4) as u64, buf[i]);
    }
    // Wipe context
    for i in 0..MD5_CTX_SIZE {
        let _ = bus.write_u8(ctx.wrapping_add(i) as u64, 0);
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `esp_crc8(const uint8_t *p, uint32_t len) -> uint8_t` — ESP32 BROM CRC-8
/// used to validate the factory MAC against ESP_EFUSE_MAC_CRC. Algorithm is
/// Dallas/Maxim 1-Wire CRC-8 (polynomial 0x31, reflected 0x8C, init=0).
/// In our sim the EFUSE blob is zero-init, so CRC of [0,0,0,0,0,0] = 0 and
/// the stored CRC byte is also 0 — the validity check passes and
/// get_efuse_factory_mac returns success.
pub fn rom_esp_crc8(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let p = cpu.regs.read_logical(n + 2);
    let len = cpu.regs.read_logical(n + 3);
    let mut crc: u8 = 0;
    for i in 0..len {
        let byte = bus.read_u8(p.wrapping_add(i) as u64).unwrap_or(0);
        crc ^= byte;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0x8C;
            } else {
                crc >>= 1;
            }
        }
    }
    RomThunkBank::return_with(cpu, crc as u32);
    Ok(())
}

/// `esp_rom_crc32_le(uint32_t crc, const uint8_t *buf, uint32_t len) -> uint32_t`
/// — IEEE 802.3 CRC-32, LSB-first (poly 0xEDB88320). Used by core dump and
/// partition helpers. `crc` is the running value (typically init `0xFFFFFFFF`);
/// the ROM does **not** post-complement — callers XOR if they need it.
pub fn rom_esp_crc32_le(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let mut crc = cpu.regs.read_logical(n + 2);
    let buf = cpu.regs.read_logical(n + 3);
    let len = cpu.regs.read_logical(n + 4);
    for i in 0..len {
        let byte = bus.read_u8(buf.wrapping_add(i) as u64).unwrap_or(0);
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    RomThunkBank::return_with(cpu, crc);
    Ok(())
}

/// `__clzsi2(u32) -> i32` — leading zero count for u32.
pub fn rom_clzsi2(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let v = cpu.regs.read_logical(n + 2);
    RomThunkBank::return_with(cpu, v.leading_zeros());
    Ok(())
}

/// `__ctzsi2(u32) -> i32` — trailing zero count for u32.
pub fn rom_ctzsi2(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let v = cpu.regs.read_logical(n + 2);
    RomThunkBank::return_with(cpu, if v == 0 { 32 } else { v.trailing_zeros() });
    Ok(())
}

/// ESP-IDF `heap_caps_init(void)` — initializes the heap allocator on real
/// silicon. We model the heap via a sim-side bump allocator (see
/// `heap_caps_malloc`), so `heap_caps_init` itself is a no-op.
pub fn esp_idf_heap_caps_init(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// State for the sim-side bump allocator backing `heap_caps_malloc`. Holds
/// the next-free pointer within a DRAM region we reserve for heap use.
/// Persisted across calls via a static. Single-threaded sim — no atomic.
static mut HEAP_BUMP_PTR: u32 = 0x3FFD_0000;
const HEAP_BUMP_END: u32 = 0x3FFE_0000; // 64 KiB pool, top of SRAM2

/// ESP-IDF `heap_caps_malloc(size: size_t, caps: u32) -> void*`.
///
/// Real silicon walks `registered_heaps[]` looking for a multi_heap with
/// matching capabilities (MALLOC_CAP_INTERNAL, MALLOC_CAP_DMA, etc) and
/// calls multi_heap_malloc on the first match. We don't model multi_heap;
/// instead, we serve all allocations from a fixed bump pool in DRAM
/// regardless of caps. Returns NULL when the pool is exhausted.
///
/// Args (Xtensa C-ABI post-rotation):
///   a[n+2] = size, a[n+3] = caps  (caps ignored)
/// Return: a[n+2] = pointer (or 0 on OOM)
// CHEAT(THUNK-LIB): bump-allocates from a Rust-side arena instead of running
// the compiled heap_caps allocator — real: the IDF allocator manages the heap
// regions. (Sibling thunks: init/calloc/free/realloc.) See FIDELITY.md §A.
pub fn esp_idf_heap_caps_malloc(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let size = cpu.regs.read_logical(n + 2);
    // Align size to 16 bytes (matches multi_heap's block alignment).
    let aligned = (size + 15) & !15;
    let ptr = unsafe {
        let start = HEAP_BUMP_PTR;
        let next = start.wrapping_add(aligned);
        if next > HEAP_BUMP_END || aligned > 1 << 20 {
            0 // OOM
        } else {
            HEAP_BUMP_PTR = next;
            start
        }
    };
    RomThunkBank::return_with(cpu, ptr);
    Ok(())
}

/// `heap_caps_calloc(n, size, caps) -> void*` — malloc-and-zero.
pub fn esp_idf_heap_caps_calloc(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let nn = cpu.ps.callinc() * 4;
    let count = cpu.regs.read_logical(nn + 2);
    let size = cpu.regs.read_logical(nn + 3);
    let total = count.wrapping_mul(size);
    let aligned = (total + 15) & !15;
    let ptr = unsafe {
        let start = HEAP_BUMP_PTR;
        let next = start.wrapping_add(aligned);
        if next > HEAP_BUMP_END || aligned > 1 << 20 {
            0
        } else {
            HEAP_BUMP_PTR = next;
            for i in 0..aligned {
                bus.write_u8(start.wrapping_add(i) as u64, 0)?;
            }
            start
        }
    };
    RomThunkBank::return_with(cpu, ptr);
    Ok(())
}

/// `heap_caps_free(void*) -> void` — bump allocator can't free.
pub fn esp_idf_heap_caps_free(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `heap_caps_realloc(void*, new_size, caps) -> void*` — degrades to malloc.
pub fn esp_idf_heap_caps_realloc(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let old_ptr = cpu.regs.read_logical(n + 2);
    let new_size = cpu.regs.read_logical(n + 3);
    let aligned = (new_size + 15) & !15;
    let ptr = unsafe {
        let start = HEAP_BUMP_PTR;
        let next = start.wrapping_add(aligned);
        if next > HEAP_BUMP_END {
            0
        } else {
            HEAP_BUMP_PTR = next;
            start
        }
    };
    // The bump allocator never reuses memory, so realloc relocates: copy the
    // old buffer's bytes into the new region. Without this, anything that
    // grows a buffer (e.g. Arduino `String` concatenation — how HTTPClient
    // builds its request) silently loses the data it already wrote. We don't
    // track allocation sizes, so copy up to `new_size`; for a grow the extra
    // tail is unused-and-soon-overwritten, for a shrink it's the kept prefix.
    if ptr != 0 && old_ptr != 0 && old_ptr != ptr {
        for i in 0..new_size as u64 {
            if let Ok(b) = bus.read_u8(old_ptr as u64 + i) {
                let _ = bus.write_u8(ptr as u64 + i, b);
            }
        }
    }
    RomThunkBank::return_with(cpu, ptr);
    Ok(())
}

/// `qsort(base, nmemb, size, compar)` — sort `nmemb` elements of `size` bytes.
///
/// Arduino heap init sorts reserved memory regions via ROM `qsort`. A nop
/// leaves the array unsorted → assert `reserved[i+1].start > reserved[i].start`.
///
/// Full `compar` callback would require re-entering the guest; for the known
/// heap path (`size == 8`, pair of `u32` start/end) we sort by the first word
/// ascending. Other sizes fall back to a simple byte-wise insertion sort using
/// lexicographic compare (stable enough for identical keys).
pub fn rom_qsort(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let base = cpu.regs.read_logical(n + 2);
    let nmemb = cpu.regs.read_logical(n + 3) as usize;
    let size = cpu.regs.read_logical(n + 4) as usize;
    if nmemb <= 1 || size == 0 || base == 0 {
        RomThunkBank::return_with(cpu, 0);
        return Ok(());
    }
    // Cap to avoid runaway (heap reserved list is tiny).
    let nmemb = nmemb.min(256);
    let size = size.min(64);
    let mut items: Vec<Vec<u8>> = Vec::with_capacity(nmemb);
    for i in 0..nmemb {
        let mut item = vec![0u8; size];
        for b in 0..size {
            item[b] = bus.read_u8(base as u64 + (i * size + b) as u64)?;
        }
        items.push(item);
    }
    if size >= 4 {
        // Sort by first u32 LE (region start).
        items.sort_by_key(|it| u32::from_le_bytes([it[0], it[1], it[2], it[3]]));
    } else {
        items.sort();
    }
    for (i, item) in items.iter().enumerate() {
        for (b, &byte) in item.iter().enumerate() {
            bus.write_u8(base as u64 + (i * size + b) as u64, byte)?;
        }
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `ets_get_cpu_frequency() -> u32` — returns 240 (MHz).
pub fn rom_cpu_freq_240mhz(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 240);
    Ok(())
}

/// `esp_clk_cpu_freq() -> u32` — returns 240_000_000 (Hz).
///
/// FreeRTOS's `_frxt_tick_timer_init` uses this to compute
/// `_xt_tick_divisor = cpu_freq / configTICK_RATE_HZ`. Without it (or with
/// the stubbed esp_clk_init returning 0), the divisor is 0 and the timer
/// ISR can never advance CCOMPARE0 — every CCOUNT cycle re-fires the tick.
pub fn esp_clk_cpu_freq_240mhz(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 240_000_000);
    Ok(())
}

/// `ets_get_detected_xtal_freq() -> u32` — returns 40 (MHz).
pub fn rom_xtal_freq_40mhz(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 40);
    Ok(())
}

/// `__bswapsi2(u32) -> u32` — GCC runtime byte-swap u32.
pub fn rom_bswapsi2(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let v = cpu.regs.read_logical(n + 2);
    RomThunkBank::return_with(cpu, v.swap_bytes());
    Ok(())
}

/// `__bswapdi2(u64) -> u64` — byte-swap u64.
pub fn rom_bswapdi2(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let lo = cpu.regs.read_logical(n + 2) as u64;
    let hi = cpu.regs.read_logical(n + 3) as u64;
    let v = (hi << 32) | lo;
    let s = v.swap_bytes();
    cpu.regs.write_logical(n + 3, (s >> 32) as u32);
    RomThunkBank::return_with(cpu, s as u32);
    Ok(())
}

/// `memcmp(a, b, n) -> i32` — lexicographic byte compare.
pub fn rom_memcmp(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let a = cpu.regs.read_logical(n + 2);
    let b = cpu.regs.read_logical(n + 3);
    let count = cpu.regs.read_logical(n + 4);
    let mut ret: i32 = 0;
    for i in 0..count {
        let ax = bus.read_u8(a.wrapping_add(i) as u64)? as i32;
        let bx = bus.read_u8(b.wrapping_add(i) as u64)? as i32;
        if ax != bx {
            ret = ax - bx;
            break;
        }
    }
    RomThunkBank::return_with(cpu, ret as u32);
    Ok(())
}

/// `__udivdi3(num: u64, den: u64) -> u64` — 64-bit unsigned divide.
///
/// Args (Xtensa C ABI for 64-bit values: a2:a3 = num low:high, a4:a5 = den
/// low:high). Result returned in a2:a3 (low:high).
pub fn rom_udivdi3(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
    let num_lo = cpu.regs.read_logical(n + 2) as u64;
    let num_hi = cpu.regs.read_logical(n + 3) as u64;
    let den_lo = cpu.regs.read_logical(n + 4) as u64;
    let den_hi = cpu.regs.read_logical(n + 5) as u64;
    let num = (num_hi << 32) | num_lo;
    let den = (den_hi << 32) | den_lo;
    let q: u64 = num.checked_div(den).unwrap_or(u64::MAX);
    // Order matters: `return_with` pops the windowed-call shadow spill
    // which covers logical n+2..n+5. A write to n+3 BEFORE the pop would
    // be clobbered. Write the low half via return_with first (it re-writes
    // n+2 after the pop), then write the high half directly.
    RomThunkBank::return_with(cpu, q as u32);
    cpu.regs.write_logical(n + 3, (q >> 32) as u32);
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
        cpu.regs.write_logical(2, 41); // a2 = 41
        cpu.set_pc(0x4037_0000);

        // Step once: should fetch BREAK 1,14, dispatch to bump_a2, return.
        cpu.step(&mut bus, &[], &crate::SimulationConfig::default())
            .expect("step dispatches thunk");

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

        let res = cpu.step(&mut bus, &[], &crate::SimulationConfig::default());
        match res {
            Err(SimulationError::NotImplemented(msg)) => {
                assert!(msg.contains("ROM thunk"), "unexpected message: {msg}");
            }
            other => panic!("expected NotImplemented, got {other:?}"),
        }
    }
}
