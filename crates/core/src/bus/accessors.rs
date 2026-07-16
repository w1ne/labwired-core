// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The `Bus` trait impl for `SystemBus`: byte/half/word memory accessors with
//! address routing + bit-band translation. Split out of `bus/mod.rs`.

use super::*;
use crate::{SimResult, SimulationError};
use std::sync::atomic::Ordering;

impl SystemBus {
    /// Side-effect-free byte read used by the universal inspect `peek`.
    ///
    /// Mirrors `read_u8`'s routing (RAM, flash, extra windows, flash boot
    /// alias, then peripherals) but reads peripherals via
    /// [`crate::Peripheral::peek`] so read-to-clear registers are never
    /// perturbed. Returns `None` for any address that no memory region or
    /// peripheral window covers — the caller renders that as an explicit
    /// unmapped marker rather than a silent zero.
    pub fn peek_byte(&self, addr: u64) -> Option<u8> {
        if let Some(val) = self.ram.read_u8(addr) {
            return Some(val);
        }
        if let Some(val) = self.flash.read_u8(addr) {
            return Some(val);
        }
        for mem in &self.extra_mem {
            if let Some(val) = mem.read_u8(addr) {
                return Some(val);
            }
        }
        // Cortex-M boot alias: 0x0 mirrors flash start on many STM32 parts.
        if self.flash.base_addr != 0 && addr < self.flash.data.len() as u64 {
            if let Some(val) = self.flash.read_u8(self.flash.base_addr + addr) {
                return Some(val);
            }
        }
        if let Some(idx) = self.find_peripheral_index(addr) {
            let p = &self.peripherals[idx];
            return p.dev.peek(addr - p.base);
        }
        None
    }
}

impl crate::Bus for SystemBus {
    fn logic_tap(&self) -> Option<crate::logic_capture::LogicTap> {
        Some(self.logic_tap.clone())
    }

    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        // RAM is always first (hot path, never overlaps a peripheral window).
        if let Some(val) = self.ram.read_u8(addr) {
            return Ok(val);
        }
        // Cortex-M boot alias: 0x0000_0000 mirrors flash start on many STM32
        // parts so reset-vector fetch works with flash at 0x0800_0000.
        let flash_alias = |s: &Self| -> Option<u8> {
            if s.flash.base_addr != 0 {
                let alias_end = s.flash.data.len() as u64;
                if addr < alias_end {
                    return s.flash.read_u8(s.flash.base_addr + addr);
                }
            }
            None
        };
        if self.config.optimized_bus_access {
            // Fast path: flash/extra_mem before peripherals.
            if let Some(val) = self.flash.read_u8(addr) {
                return Ok(val);
            }
            for mem in &self.extra_mem {
                if let Some(val) = mem.read_u8(addr) {
                    return Ok(val);
                }
            }
            if let Some(val) = flash_alias(self) {
                return Ok(val);
            }
            if let Some(idx) = self.find_peripheral_index(addr) {
                if !self.is_peripheral_clocked(idx) {
                    return Ok(0); // unclocked peripheral reads 0 (silicon gating)
                }
                let p = &self.peripherals[idx];
                return p.dev.read(addr - p.base);
            }
        } else {
            // Peripherals first so an MMU-translating FlashXip window overrides a
            // plain flash/extra_mem region claiming the same XIP address; flash/
            // extra_mem remain the fallback for addresses no peripheral covers.
            if let Some(idx) = self.find_peripheral_index(addr) {
                if !self.is_peripheral_clocked(idx) {
                    return Ok(0); // unclocked peripheral reads 0 (silicon gating)
                }
                let p = &self.peripherals[idx];
                return p.dev.read(addr - p.base);
            }
            if let Some(val) = self.flash.read_u8(addr) {
                return Ok(val);
            }
            for mem in &self.extra_mem {
                if let Some(val) = mem.read_u8(addr) {
                    return Ok(val);
                }
            }
            if let Some(val) = flash_alias(self) {
                return Ok(val);
            }
        }

        if std::env::var("LABWIRED_TRACE_VIOLATIONS").is_ok() {
            eprintln!("BUS_VIOLATION read_u8 addr=0x{:08X}", addr);
        }
        crate::fidelity::record_unmapped(addr, "read");
        Err(SimulationError::MemoryViolation(addr))
    }

    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        let flash_alias_old = if self.flash.base_addr != 0 && addr < self.flash.data.len() as u64 {
            self.flash.read_u8(self.flash.base_addr + addr)
        } else {
            None
        };

        // Avoid calling `read_u8` here since peripheral reads may carry side effects.
        let old_value = self
            .ram
            .read_u8(addr)
            .or_else(|| self.flash.read_u8(addr))
            .or(flash_alias_old)
            .or_else(|| self.extra_mem.iter().find_map(|m| m.read_u8(addr)))
            .or_else(|| {
                self.find_peripheral_index(addr).and_then(|idx| {
                    let p = &self.peripherals[idx];
                    p.dev.peek(addr - p.base)
                })
            })
            .unwrap_or(0);

        // OPT-IN H5 program write-buffer fidelity gate. When a FLASH peripheral
        // has the gate enabled, a flash-region write feeds the silicon-true
        // quad-word write-buffer state machine instead of committing directly:
        // bytes accumulate into a 16-byte buffer (WBNE), commit only on a full
        // aligned quad-word as the bitwise AND of new & existing (flash flips
        // only 1→0, setting EOP), and a misaligned/inconsistent quad-word
        // raises INCERR alone with no commit. The peripheral owns the NSSR
        // status; the bus owns the flash backing memory, so the AND-commit is
        // done here. `None` (gate off) ⇒ this block is skipped and the write
        // commits as before — byte-identical to prior behaviour.
        if let Some(flash_idx) = self.flash_error_flags_idx {
            // Resolve the flash-region offset this write targets, if any. The
            // backing buffer is addressed at `flash.base_addr`; the boot alias
            // (addr < buffer len) mirrors the same offset.
            let region_off = if self.flash.read_u8(addr).is_some() {
                Some(addr - self.flash.base_addr)
            } else if self.flash.base_addr != 0 && addr < self.flash.data.len() as u64 {
                Some(addr) // boot-alias write: offset is addr itself
            } else {
                None
            };
            if let Some(off) = region_off {
                use crate::peripherals::flash::H5ProgAction;
                let action = self.peripherals[flash_idx]
                    .dev
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<crate::peripherals::flash::Flash>())
                    .map(|f| f.h5_program_byte(off, value));
                match action {
                    // Quad-word complete: read the 16 existing bytes, AND each
                    // with the buffered value, write back. This is the only
                    // store on the gate-on path.
                    Some(H5ProgAction::Commit { base, bytes }) => {
                        for (i, &nb) in bytes.iter().enumerate() {
                            let qoff = base + i as u64;
                            let existing = self
                                .flash
                                .read_u8(self.flash.base_addr + qoff)
                                .unwrap_or(0xFF);
                            self.flash
                                .write_u8(self.flash.base_addr + qoff, existing & nb);
                        }
                        for observer in &self.observers {
                            observer.on_memory_write(addr, old_value, value);
                        }
                        return Ok(());
                    }
                    // Buffered / inconsistent / not-programming: nothing stored
                    // (NSSR status already updated by the state machine).
                    Some(_) => return Ok(()),
                    // Downcast failed (should not happen): fall through.
                    None => {}
                }
            }
        }

        let flash_alias_write = self.flash.base_addr != 0
            && addr < self.flash.data.len() as u64
            && self.flash.write_u8(self.flash.base_addr + addr, value);

        let res = if self.ram.write_u8(addr, value)
            || self.flash.write_u8(addr, value)
            || flash_alias_write
            || self.extra_mem.iter_mut().any(|m| m.write_u8(addr, value))
        {
            Ok(())
        } else {
            // Dynamic Peripherals
            if let Some(idx) = self.find_peripheral_index(addr) {
                if !self.is_peripheral_clocked(idx) {
                    // Unclocked peripheral: the write is dropped on real silicon
                    // (the bus access never reaches the gated block), so status
                    // bits never change and the firmware visibly stalls.
                    return Ok(());
                }
                #[cfg(feature = "event-scheduler")]
                self.sync_scheduler_peripheral(idx);
                self.maybe_latch_dc(idx);
                let c3_io_mux_capture = self.begin_esp32c3_io_mux_write(idx);
                let r = {
                    let p = &mut self.peripherals[idx];
                    p.dev.write(addr - p.base, value)
                };
                if r.is_ok() {
                    self.finish_esp32c3_io_mux_write(c3_io_mux_capture);
                }
                self.maybe_arm_hcsr04(idx);
                self.maybe_clock_tm1637(idx);
                #[cfg(feature = "event-scheduler")]
                self.collect_scheduled_events(idx);
                r
            } else {
                if std::env::var("LABWIRED_TRACE_VIOLATIONS").is_ok() {
                    eprintln!(
                        "BUS_VIOLATION write_u8 addr=0x{:08X} val=0x{:02X}",
                        addr, value
                    );
                }
                crate::fidelity::record_unmapped(addr, "write");
                Err(SimulationError::MemoryViolation(addr))
            }
        };

        if res.is_ok() {
            // Wake up the peripheral
            if let Some(idx) = self.find_peripheral_index(addr) {
                let base = self.peripherals[idx].base;
                self.sync_esp32c3_irq_cache_write(idx, addr - base);
                self.peripherals[idx].ticks_remaining = 0;
                self.refresh_legacy_tick_index(idx);
                self.refresh_bus_tick_index(idx);
            }

            // Trigger observers
            for observer in &self.observers {
                observer.on_memory_write(addr, old_value, value);
            }
        }

        res
    }

    fn read_u16(&self, addr: u64) -> SimResult<u16> {
        if let Some(val) = self.ram.read_u16(addr) {
            return Ok(val);
        }
        // See read_u32: with optimized_bus_access off, peripherals (FlashXip)
        // win over the plain flash region at the same XIP address. extra_mem
        // always gets a word path (IRAM etc. never conflict with XIP windows).
        let flash_and_alias = |s: &Self| -> Option<u16> {
            if let Some(val) = s.flash.read_u16(addr) {
                return Some(val);
            }
            if s.flash.base_addr != 0 && addr + 1 < s.flash.data.len() as u64 {
                return s.flash.read_u16(s.flash.base_addr + addr);
            }
            None
        };
        let extra_mem_half = |s: &Self| -> Option<u16> {
            for mem in &s.extra_mem {
                if let Some(val) = mem.read_u16(addr) {
                    return Some(val);
                }
            }
            None
        };
        if self.config.optimized_bus_access {
            if let Some(val) = flash_and_alias(self) {
                return Ok(val);
            }
            if let Some(val) = extra_mem_half(self) {
                return Ok(val);
            }
            if let Some(idx) = self.find_peripheral_index(addr) {
                if !self.is_peripheral_clocked(idx) {
                    return Ok(0);
                }
                return self.peripherals[idx]
                    .dev
                    .read_u16(addr - self.peripherals[idx].base);
            }
        } else {
            if let Some(idx) = self.find_peripheral_index(addr) {
                if !self.is_peripheral_clocked(idx) {
                    return Ok(0);
                }
                return self.peripherals[idx]
                    .dev
                    .read_u16(addr - self.peripherals[idx].base);
            }
            if let Some(val) = extra_mem_half(self) {
                return Ok(val);
            }
            if let Some(val) = flash_and_alias(self) {
                return Ok(val);
            }
        }
        let b0 = self.read_u8(addr)? as u16;
        let b1 = self.read_u8(addr + 1)? as u16;
        Ok(b0 | (b1 << 8))
    }

    fn read_u32(&self, addr: u64) -> SimResult<u32> {
        // Debug (env-gated): trace the driver's reads of a freshly-injected RX
        // buffer, to RE the rx-control header format the RX callback parses.
        if crate::peripherals::esp32c3::wifi_mac::rxbuf_trace_enabled() {
            let base = crate::peripherals::esp32c3::wifi_mac::RX_DBG_BUF
                .load(std::sync::atomic::Ordering::Relaxed) as u64;
            // Trace from 0x100 BEFORE the buffer (to catch the descriptor-list
            // reads, e.g. the descriptor at buf-0x7c) through the buffer.
            if base != 0 && (base.saturating_sub(0x100)..base + 512).contains(&addr) {
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                if N.fetch_add(1, Ordering::Relaxed) < 200 {
                    let v = self.ram.read_u32(addr).unwrap_or(0);
                    eprintln!("[rxrd] +{:#05x} ({addr:#010x}) = {v:#010x}", addr - base);
                }
            }
        }
        // Cortex-M bit-band alias: return 0 or 1 based on the physical bit.
        if self.bit_band_enabled {
            if let Some((phys_byte, bit)) = Self::bit_band_translate(addr) {
                let byte_val = self.read_u8(phys_byte)?;
                return Ok(((byte_val >> bit) & 1) as u32);
            }
        }
        // RP2040 atomic register aliases: every alias of a register reads back
        // the aligned base register (the op only affects writes).
        if self.atomic_register_aliases {
            if let Some((base, _)) = self.atomic_alias_redirect(addr) {
                return self.read_u32(base);
            }
        }

        if let Some(val) = self.ram.read_u32(addr) {
            return Ok(val);
        }
        // Flash region + boot alias, and peripherals. With optimized_bus_access
        // off (C3 rom-boot) peripherals win over the plain flash region so an
        // MMU-translating FlashXip window overrides the zero-filled flash at the
        // same XIP address — otherwise a 4-byte instruction fetch at an XIP
        // address reads 0, misdecodes as 2-byte and the PC drifts (mirrors the
        // read_u8 fix). Linear extra_mem (IRAM/ROM/RTC) never conflicts with
        // those XIP windows, so it always gets a word-sized fast path — the
        // previous fall-through did 4× find_peripheral via read_u8.
        let flash_and_alias = |s: &Self| -> Option<u32> {
            if let Some(val) = s.flash.read_u32(addr) {
                return Some(val);
            }
            if s.flash.base_addr != 0 && addr + 3 < s.flash.data.len() as u64 {
                return s.flash.read_u32(s.flash.base_addr + addr);
            }
            None
        };
        let extra_mem_word = |s: &Self| -> Option<u32> {
            for mem in &s.extra_mem {
                if let Some(val) = mem.read_u32(addr) {
                    return Some(val);
                }
            }
            None
        };
        if self.config.optimized_bus_access {
            if let Some(val) = flash_and_alias(self) {
                return Ok(val);
            }
            if let Some(val) = extra_mem_word(self) {
                return Ok(val);
            }
            if let Some(idx) = self.find_peripheral_index(addr) {
                if !self.is_peripheral_clocked(idx) {
                    return Ok(0);
                }
                return self.peripherals[idx]
                    .dev
                    .read_u32(addr - self.peripherals[idx].base);
            }
        } else {
            if let Some(idx) = self.find_peripheral_index(addr) {
                if !self.is_peripheral_clocked(idx) {
                    return Ok(0);
                }
                return self.peripherals[idx]
                    .dev
                    .read_u32(addr - self.peripherals[idx].base);
            }
            // IRAM / ROM / RTC after peripherals so XIP FlashXip still wins on
            // 0x4200_0000 / 0x3C00_0000 over zero-filled extra_mem twins.
            if let Some(val) = extra_mem_word(self) {
                return Ok(val);
            }
            if let Some(val) = flash_and_alias(self) {
                return Ok(val);
            }
        }
        let b0 = self.read_u8(addr)? as u32;
        let b1 = self.read_u8(addr + 1)? as u32;
        let b2 = self.read_u8(addr + 2)? as u32;
        let b3 = self.read_u8(addr + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    fn write_u16(&mut self, addr: u64, value: u16) -> SimResult<()> {
        let mut wrote = self.ram.write_u16(addr, value) || self.flash.write_u16(addr, value);
        if !wrote && self.flash.base_addr != 0 && addr + 1 < self.flash.data.len() as u64 {
            wrote = self.flash.write_u16(self.flash.base_addr + addr, value);
        }
        if wrote {
            return Ok(());
        }
        if let Some(idx) = self.find_peripheral_index(addr) {
            if !self.is_peripheral_clocked(idx) {
                return Ok(()); // unclocked peripheral: write dropped (gating)
            }
            #[cfg(feature = "event-scheduler")]
            self.sync_scheduler_peripheral(idx);
            self.maybe_latch_dc(idx);
            let c3_io_mux_capture = self.begin_esp32c3_io_mux_write(idx);
            let r = {
                let p = &mut self.peripherals[idx];
                p.ticks_remaining = 0;
                p.dev.write_u16(addr - p.base, value)
            };
            if r.is_ok() {
                self.finish_esp32c3_io_mux_write(c3_io_mux_capture);
            }
            self.maybe_arm_hcsr04(idx);
            self.maybe_clock_tm1637(idx);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(idx);
            if r.is_ok() {
                let base = self.peripherals[idx].base;
                self.sync_esp32c3_irq_cache_write(idx, addr - base);
                self.refresh_legacy_tick_index(idx);
                self.refresh_bus_tick_index(idx);
            }
            return r;
        }
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }

    fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        // RP2040 atomic register aliases: a write to a +0x1000/0x2000/0x3000
        // alias of a peripheral register is a read-modify-write (XOR/SET/CLR)
        // on the aligned base register. The base access recurses into the
        // normal path (its alias bits are clear), so there is no further alias.
        if self.atomic_register_aliases {
            if let Some((base, op)) = self.atomic_alias_redirect(addr) {
                let cur = self.read_u32(base)?;
                let new = match op {
                    crate::bus::AtomicAliasOp::Xor => cur ^ value,
                    crate::bus::AtomicAliasOp::Set => cur | value,
                    crate::bus::AtomicAliasOp::Clr => cur & !value,
                };
                return self.write_u32(base, new);
            }
        }
        // Debug: trace WiFi MAC-window writes (env-gated) to RE the TX path.
        if (0x6003_3000..0x6003_6000).contains(&addr) && std::env::var("LABWIRED_MAC_TRACE").is_ok()
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            if N.fetch_add(1, Ordering::Relaxed) < 600 {
                eprintln!("[macw] {addr:#010x} <= {value:#010x}");
            }
        }
        // Cortex-M bit-band alias translation (peripheral: 0x42000000-0x43FFFFFF,
        // SRAM: 0x22000000-0x23FFFFFF).  Each alias word maps to one bit of the
        // physical address.  Writing 1 sets the bit; writing 0 clears it.
        if self.bit_band_enabled {
            if let Some((phys_byte, bit)) = Self::bit_band_translate(addr) {
                let old = self.read_u8(phys_byte)?;
                let new_byte = if value & 1 != 0 {
                    old | (1 << bit)
                } else {
                    old & !(1 << bit)
                };
                return self.write_u8(phys_byte, new_byte);
            }
        }

        let mut wrote = self.ram.write_u32(addr, value) || self.flash.write_u32(addr, value);
        if !wrote && self.flash.base_addr != 0 && addr + 3 < self.flash.data.len() as u64 {
            wrote = self.flash.write_u32(self.flash.base_addr + addr, value);
        }
        if wrote {
            return Ok(());
        }
        if let Some(idx) = self.find_peripheral_index(addr) {
            if !self.is_peripheral_clocked(idx) {
                return Ok(()); // unclocked peripheral: write dropped (gating)
            }
            #[cfg(feature = "event-scheduler")]
            self.sync_scheduler_peripheral(idx);
            self.maybe_latch_dc(idx);
            let c3_io_mux_capture = self.begin_esp32c3_io_mux_write(idx);
            let r = {
                let p = &mut self.peripherals[idx];
                p.ticks_remaining = 0;
                p.dev.write_u32(addr - p.base, value)
            };
            if r.is_ok() {
                self.finish_esp32c3_io_mux_write(c3_io_mux_capture);
            }
            self.maybe_arm_hcsr04(idx);
            self.maybe_clock_tm1637(idx);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(idx);
            if r.is_ok() {
                let base = self.peripherals[idx].base;
                self.sync_esp32c3_irq_cache_write(idx, addr - base);
                self.refresh_legacy_tick_index(idx);
                self.refresh_bus_tick_index(idx);
            }
            return r;
        }
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write_u8(addr + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write_u8(addr + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    /// Fast-path fetch slice for the CPU instruction-fetch cache
    /// (#119 Phase 1.2). Returns `Some((base, end, slice))` when `pc`
    /// lands inside a `RamPeripheral` we can serve directly; falls
    /// through to `None` (slow path) for any other peripheral kind or
    /// unmapped addresses.
    ///
    /// The returned slice borrows the peripheral's backing buffer for
    /// the duration of the call. The CPU stashes a raw pointer derived
    /// from it; the `RamPeripheral` INVARIANT (no resize) keeps that
    /// pointer valid until the peripheral is dropped, but the CPU MUST
    /// invalidate the cache on any bus write into the cached range
    /// and on snapshot restore. Reads from non-RAM peripherals (e.g.
    /// `RomThunkBank`, GPIO, declarative peripherals) keep going
    /// through the slow path so side effects fire as before.
    fn fetch_slice(&self, pc: u64) -> Option<(u64, u64, &[u8])> {
        let idx = self.find_peripheral_index(pc)?;
        let entry = self.peripherals.get(idx)?;
        let any = entry.dev.as_any()?;
        let ram = any.downcast_ref::<crate::system::xtensa::RamPeripheral>()?;
        let (ptr, len) = ram.backing_ptr_len();
        // SAFETY: `RamPeripheral`'s backing `Vec` is fixed-size at
        // construction (see struct-level INVARIANT in
        // `system::xtensa::RamPeripheral`). The `&self` borrow on the
        // bus keeps the peripheral entry alive for the duration of
        // this borrow. We're only producing a read-only `&[u8]` from
        // a `*const u8`; no concurrent `borrow_mut` is in flight
        // because reads don't mutate.
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        Some((entry.base, entry.base.saturating_add(entry.size), slice))
    }

    fn execute_dma(&mut self, requests: &[crate::DmaRequest]) -> SimResult<()> {
        for req in requests {
            match req.direction {
                crate::DmaDirection::Read => {
                    let _ = self.read_u8(req.addr)?;
                }
                crate::DmaDirection::Write => {
                    self.write_u8(req.addr, req.val)?;
                }
                crate::DmaDirection::Copy => {
                    if let Some(t) = req.transform {
                        self.dma_copy_unit(req.src_addr, req.addr, t)?;
                    } else {
                        let val = self.read_u8(req.src_addr)?;
                        self.write_u8(req.addr, val)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn config(&self) -> &crate::SimulationConfig {
        &self.config
    }

    fn external_irq_lines(&self) -> u32 {
        self.riscv_irq_lines
    }

    #[cfg(feature = "event-scheduler")]
    fn has_pending_schedule(&self) -> bool {
        !self.pending_schedule.is_empty()
    }

    fn earliest_pending_deadline(&self) -> Option<u64> {
        self.pending_schedule.iter().map(|(_, deadline, _)| *deadline).min()
    }

    #[cfg(feature = "event-scheduler")]
    fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    #[cfg(feature = "event-scheduler")]
    fn publish_cycle(&mut self, cycle: u64) {
        self.set_current_cycle(cycle);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn tick_peripherals(&mut self) -> Vec<u32> {
        let (interrupts, _costs) = self.tick_peripherals_fully();
        interrupts
    }

    fn clear_nvic_pending(&mut self, exception_num: u32) {
        if exception_num >= 16 {
            if let Some(nvic) = &self.nvic {
                let irq = exception_num - 16;
                let idx = (irq / 32) as usize;
                let bit = irq % 32;
                if idx < 8 {
                    nvic.ispr[idx].fetch_and(!(1 << bit), Ordering::SeqCst);
                }
            }
        }
    }

    fn is_nvic_irq_pending(&self, exception_num: u32) -> bool {
        if exception_num < 16 {
            return true; // Non-NVIC exceptions (SysTick, PendSV, etc.) are always live.
        }
        if let Some(nvic) = &self.nvic {
            let irq = exception_num - 16;
            let idx = (irq / 32) as usize;
            let bit = irq % 32;
            if idx < 8 {
                return (nvic.ispr[idx].load(Ordering::SeqCst) & (1 << bit)) != 0;
            }
        }
        true // No NVIC — assume pending (safe conservative default).
    }

    fn get_rom_thunk(
        &self,
        pc: u32,
    ) -> Option<crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkFn> {
        SystemBus::get_rom_thunk(self, pc)
    }

    fn route_irq_source_to_cpu_irq(&self, source_id: u32) -> Option<u8> {
        SystemBus::route_irq_source_to_cpu_irq(self, source_id)
    }

    fn pending_cpu_irqs(&self, core_id: u8) -> u32 {
        // Two cross-core delivery paths coexist after the dual-core merge:
        //   * ESP32-S3 (intmatrix registered): the aggregator routes every
        //     asserting source — including the FROM_CPU IPI sources
        //     79→core0 / 80→core1 — into this per-core array.
        //   * ESP32-classic (no intmatrix, DPORT instead): the array stays
        //     empty and cross-core FROM_CPU IPIs come from the DPORT matrix.
        // Each path contributes 0 on the other chip, so OR-ing is safe and
        // keeps both dual-core models working.
        self.pending_cpu_irqs[(core_id & 1) as usize] | self.dport_cross_core_pending(core_id)
    }

    fn clear_cpu_irq_pending(&mut self, core_id: u8, slot: u8) {
        self.pending_cpu_irqs[(core_id & 1) as usize] &= !(1u32 << slot);
    }
}
