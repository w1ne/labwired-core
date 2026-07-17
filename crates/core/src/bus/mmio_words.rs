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
        // RAM is always the highest-priority linear backing store and never
        // aliases an XIP window.
        if let Some(b) = self.ram.read_u8(addr) {
            return Some(b);
        }

        // A side-effect-free read of the Flash-XIP peripheral: MMU-translated
        // exactly as the CPU's instruction fetch routes it. This MUST be
        // tried before the plain flash region, mirroring the precedence
        // `read_u32`/`read_u8` use when `optimized_bus_access` is off (the C3
        // rom-boot config): the XIP window and a zero-filled linear-flash twin
        // occupy the SAME virtual address (0x4200_0000), and only the XIP read
        // MMU-translates to the real flash page. Checking `self.flash` first
        // returned `Some(0)` for the whole C3 app image, so the JIT compiled
        // (or here, walked) from all-zero bytes — `0x0000` decodes to an
        // illegal `c.addi4spn`, i.e. `Unknown` — and every hot XIP block was
        // silently kept on the interpreter. Restricting to the FlashXip
        // peripheral keeps this fetch side-effect free (no MMIO is touched).
        let xip_byte = |s: &Self| -> Option<u8> {
            let idx = s.find_peripheral_index(addr)?;
            let p = &s.peripherals[idx];
            p.dev
                .as_any()
                .and_then(|a| {
                    a.downcast_ref::<crate::peripherals::esp32s3::flash_xip::FlashXipPeripheral>()
                })
                .is_some()
                .then(|| p.dev.read(addr - p.base).ok())
                .flatten()
        };
        let flash_byte = |s: &Self| -> Option<u8> { s.flash.read_u8(addr) };
        let extra_byte =
            |s: &Self| -> Option<u8> { s.extra_mem.iter().find_map(|mem| mem.read_u8(addr)) };

        // Follow `read_u32`'s two precedence orders exactly so the walked bytes
        // are byte-identical to what the interpreter fetches.
        if self.config.optimized_bus_access {
            flash_byte(self)
                .or_else(|| extra_byte(self))
                .or_else(|| xip_byte(self))
        } else {
            // IRAM/ROM/RTC (extra_mem) after the XIP peripheral so FlashXip
            // still wins on 0x4200_0000 over zero-filled extra_mem twins.
            xip_byte(self)
                .or_else(|| extra_byte(self))
                .or_else(|| flash_byte(self))
        }
    }
}
