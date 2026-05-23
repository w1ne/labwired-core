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
            // Thunks skip ENTRY/RETW, so they bypass the sim-level shadow
            // spill that CALL{n} performs (caller's preserved + WS-conditional
            // callee window). Pop those entries to keep stacks balanced.
            // We pop AFTER setting the return value so the pop's restore can
            // safely clobber the return-value AR slot — write_logical above
            // already saved it into the AR, but the pop will undo that for
            // slot wb_callee's a2 position. To preserve the return value,
            // re-write it after the pops.
            let wb_caller = cpu.regs.windowbase();
            let wb_callee = wb_caller.wrapping_add(callinc) & 0x0F;
            for k in 0..4u8 {
                let slot = wb_callee.wrapping_add(k) & 0x0F;
                cpu.regs.pop_shadow(slot);
            }
            for k in 0..callinc {
                let slot = wb_caller.wrapping_add(k) & 0x0F;
                cpu.regs.pop_shadow(slot);
            }
            cpu.regs.write_logical(n + 2, value);
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
pub fn ets_set_appcpu_boot_addr(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
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
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
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

/// Generic NOP thunk that returns 0. Useful for ROM functions whose
/// behaviour we don't model but whose return value the caller needs to
/// pass through (e.g. cache config, frequency update, busy-wait).
pub fn nop_return_zero(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
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
}

/// Monotonic-counter thunk for `esp_timer_impl_get_counter_reg()` and
/// similar 32-bit time-source readers. Returns an ever-increasing value
/// (steps of 1000 per call) so callers polling for timeout deadlines
/// actually make progress instead of looping forever.
pub fn monotonic_counter_32(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    static MONOTONIC_TICKS: core::sync::atomic::AtomicU32 =
        core::sync::atomic::AtomicU32::new(0);
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
            0x03, 0x00,             // revision = 3
            0x02,                   // cores = 2
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
pub fn esp_idf_heap_caps_realloc(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let n = cpu.ps.callinc() * 4;
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
    RomThunkBank::return_with(cpu, ptr);
    Ok(())
}

/// `ets_get_cpu_frequency() -> u32` — returns 240 (MHz).
pub fn rom_cpu_freq_240mhz(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 240);
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
