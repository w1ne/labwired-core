// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Side-effect-free instruction-fetch helpers for the RISC-V JIT.
//!
//! The word/halfword MMIO accessors (`read_u32`/`read_u16`/`write_u32`/
//! `write_u16`) live solely in the [`crate::Bus`] trait impl in
//! `bus/accessors.rs` — that is the single source of truth the CPU runs and the
//! fidelity gates cover. Direct `SystemBus` callers reach them through the
//! trait (`use crate::Bus`), so there is no second, drift-prone inherent copy.

use super::*;

impl SystemBus {
    /// Side-effect-free instruction-fetch of up to `max_len` **contiguous**
    /// guest code bytes starting at virtual address `vaddr`, materialised the
    /// SAME way the interpreter fetches instructions (see
    /// [`<Self as crate::Bus>::read_u32`] / [`<Self as crate::Bus>::read_u16`]
    /// and `RiscV::step`'s `bus.read_u32(pc)` fetch): linear RAM / flash / extra
    /// memories, and the MMU-translating flash-XIP windows (0x4200_0000 /
    /// 0x3C00_0000). Iterating one address at a time keeps the buffer contiguous
    /// in **virtual** space — exactly what the walker's
    /// [`CodeView`](crate::cpu::jit_framework::CodeView) indexes — even when the
    /// XIP MMU maps consecutive virtual pages to discontiguous flash pages.
    ///
    /// Stops at the first address that is not fetchable code memory (an
    /// unmapped XIP page, or an MMIO peripheral — which is deliberately **never
    /// read here**, so no FIFO / clear-on-read side effect can fire). The
    /// returned buffer is therefore a byte-exact prefix of what the CPU would
    /// fetch from `vaddr` onward.
    ///
    /// Used by the RISC-V JIT to compile hot blocks through the same XIP/MMU
    /// mapping the interpreter fetches through: reading `bus.flash.data`
    /// directly bypasses the ESP32-C3 XIP MMU and yields the wrong bytes
    /// (typically zeros → a spurious 1024-instruction runaway block).
    pub fn read_code_slice(&self, vaddr: u64, max_len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(max_len);
        for i in 0..max_len as u64 {
            match self.read_code_byte(vaddr.wrapping_add(i)) {
                Some(b) => out.push(b),
                None => break,
            }
        }
        out
    }

    /// One code byte at `addr`, or `None` if `addr` is not side-effect-free
    /// code memory. See [`Self::read_code_slice`].
    fn read_code_byte(&self, addr: u64) -> Option<u8> {
        // Linear code/data memories are side-effect free; check them in the
        // same precedence `read_u32`'s optimized path uses (ranges are
        // disjoint, so precedence only picks the one backing store).
        if let Some(b) = self.ram.read_u8(addr) {
            return Some(b);
        }
        if let Some(b) = self.flash.read_u8(addr) {
            return Some(b);
        }
        for mem in &self.extra_mem {
            if let Some(b) = mem.read_u8(addr) {
                return Some(b);
            }
        }
        // Flash-XIP window: read-only, MMU-translated exactly as the CPU's
        // instruction fetch routes it. Restricting to the XIP peripheral keeps
        // this fetch side-effect free — no MMIO is ever touched.
        if let Some(idx) = self.find_peripheral_index(addr) {
            let p = &self.peripherals[idx];
            if p.dev
                .as_any()
                .and_then(|a| {
                    a.downcast_ref::<crate::peripherals::esp32s3::flash_xip::FlashXipPeripheral>()
                })
                .is_some()
            {
                return p.dev.read(addr - p.base).ok();
            }
        }
        None
    }

    /// Side-effect-free entry-word peek in linear flash for verifying cached
    /// blocks backed by synthetic base-zero flash.
    pub(crate) fn read_flash_entry_word_impl(&self, pc: u64) -> Option<u32> {
        let off = pc.checked_sub(self.flash.base_addr)? as usize;
        if off + 4 <= self.flash.data.len() {
            Some(u32::from_le_bytes([
                self.flash.data[off],
                self.flash.data[off + 1],
                self.flash.data[off + 2],
                self.flash.data[off + 3],
            ]))
        } else if off + 2 <= self.flash.data.len() {
            Some(u32::from(u16::from_le_bytes([
                self.flash.data[off],
                self.flash.data[off + 1],
            ])))
        } else {
            None
        }
    }

    /// Side-effect-free code-window slice for the decoded basic-block cache:
    /// returns `(bytes, writable_code, verify_entry)` matching `refill_fetch_window`'s
    /// code regions (flash, extra IRAM/ROM memories, and Flash-XIP windows; plain RAM
    /// is deliberately excluded so host-patched RAM tests stay exact).
    pub(crate) fn read_block_cache_code_impl(
        &self,
        pc: u64,
        max_bytes: usize,
    ) -> Option<(Vec<u8>, bool, bool)> {
        if self.find_peripheral_index(pc).is_some() {
            // Reuses `read_code_slice` which permits only FlashXipPeripheral
            // and never touches side-effectful MMIO peripherals.
            let slice = self.read_code_slice(pc, max_bytes);
            return (!slice.is_empty()).then_some((slice, false, false));
        }
        if let Some(off) = pc.checked_sub(self.flash.base_addr) {
            let off = off as usize;
            if off + 2 <= self.flash.data.len() {
                let len = (self.flash.data.len() - off).min(max_bytes);
                let verify = self.flash.base_addr == 0;
                return Some((self.flash.data[off..off + len].to_vec(), false, verify));
            }
        }
        for mem in &self.extra_mem {
            if let Some(off) = pc.checked_sub(mem.base_addr) {
                let off = off as usize;
                if off + 2 <= mem.data.len() {
                    let len = (mem.data.len() - off).min(max_bytes);
                    return Some((mem.data[off..off + len].to_vec(), true, false));
                }
            }
        }
        None
    }
}
