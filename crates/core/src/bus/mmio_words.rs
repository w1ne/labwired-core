// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Word/halfword MMIO helpers (byte-path composition + clock-gate checks).

use super::*;

impl SystemBus {
    pub fn read_u32(&self, addr: u64) -> SimResult<u32> {
        if self.config.optimized_bus_access {
            if let Some(val) = self.ram.read_u32(addr) {
                return Ok(val);
            }
            if let Some(val) = self.flash.read_u32(addr) {
                return Ok(val);
            }
            for mem in &self.extra_mem {
                if let Some(val) = mem.read_u32(addr) {
                    return Ok(val);
                }
            }
            // Boot alias handle
            if self.flash.base_addr != 0 {
                let alias_end = self.flash.data.len() as u64;
                if addr + 3 < alias_end {
                    if let Some(val) = self.flash.read_u32(self.flash.base_addr + addr) {
                        return Ok(val);
                    }
                }
            }
        }

        if let Some(idx) = self.find_peripheral_index(addr) {
            if !self.is_peripheral_clocked(idx) {
                return Ok(0); // unclocked peripheral reads 0 (silicon gating)
            }
            let p = &self.peripherals[idx];
            return p.dev.read_u32(addr - p.base);
        }

        let b0 = self.read_u8(addr)? as u32;
        let b1 = self.read_u8(addr + 1)? as u32;
        let b2 = self.read_u8(addr + 2)? as u32;
        let b3 = self.read_u8(addr + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    /// Side-effect-free instruction-fetch of up to `max_len` **contiguous**
    /// guest code bytes starting at virtual address `vaddr`, materialised the
    /// SAME way the interpreter fetches instructions (see [`Self::read_u32`] /
    /// [`Self::read_u16`] and `RiscV::step`'s `bus.read_u32(pc)` fetch): linear
    /// RAM / flash / extra memories, and the MMU-translating flash-XIP windows
    /// (0x4200_0000 / 0x3C00_0000). Iterating one address at a time keeps the
    /// buffer contiguous in **virtual** space — exactly what the walker's
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

    pub fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        if self.config.optimized_bus_access && self.ram.write_u32(addr, value) {
            return Ok(());
        }
        if self.config.optimized_bus_access {
            for mem in &mut self.extra_mem {
                if mem.write_u32(addr, value) {
                    return Ok(());
                }
            }
        }
        // Flash is read-only via bus writes usually, but let's stick to the behavior of write_u8
        // which would likely fail or do nothing if it's flash.
        // Actually write_u8 checks flash_alias_old etc.

        if let Some(idx) = self.find_peripheral_index(addr) {
            if !self.is_peripheral_clocked(idx) {
                return Ok(()); // unclocked peripheral: write dropped (gating)
            }
            #[cfg(feature = "event-scheduler")]
            self.sync_scheduler_peripheral(idx);
            self.maybe_latch_dc(idx);
            let c3_io_mux_capture = self.begin_esp32c3_io_mux_write(idx);
            let (base, r) = {
                let p = &mut self.peripherals[idx];
                p.ticks_remaining = 0;
                let base = p.base;
                let r = p.dev.write_u32(addr - base, value);
                (base, r)
            };
            if r.is_ok() {
                self.finish_esp32c3_io_mux_write(c3_io_mux_capture);
            }
            self.maybe_arm_hcsr04(idx);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(idx);
            if r.is_ok() {
                // Keep the C3 IRQ routing cache coherent on this inherent
                // write path too (the Bus-trait accessors already do this) —
                // host/tooling writes to INTC or FROM_CPU must re-aggregate
                // exactly like CPU stores.
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

    pub fn read_u16(&self, addr: u64) -> SimResult<u16> {
        if self.config.optimized_bus_access {
            if let Some(val) = self.ram.read_u16(addr) {
                return Ok(val);
            }
            if let Some(val) = self.flash.read_u16(addr) {
                return Ok(val);
            }
            for mem in &self.extra_mem {
                if let Some(val) = mem.read_u16(addr) {
                    return Ok(val);
                }
            }
            // Boot alias handle
            if self.flash.base_addr != 0 {
                let alias_end = self.flash.data.len() as u64;
                if addr + 1 < alias_end {
                    if let Some(val) = self.flash.read_u16(self.flash.base_addr + addr) {
                        return Ok(val);
                    }
                }
            }
        }

        if let Some(idx) = self.find_peripheral_index(addr) {
            if !self.is_peripheral_clocked(idx) {
                return Ok(0); // unclocked peripheral reads 0 (silicon gating)
            }
            let p = &self.peripherals[idx];
            return p.dev.read_u16(addr - p.base);
        }

        let b0 = self.read_u8(addr)? as u16;
        let b1 = self.read_u8(addr + 1)? as u16;
        Ok(b0 | (b1 << 8))
    }

    pub fn write_u16(&mut self, addr: u64, value: u16) -> SimResult<()> {
        if self.config.optimized_bus_access && self.ram.write_u16(addr, value) {
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
            let (base, r) = {
                let p = &mut self.peripherals[idx];
                p.ticks_remaining = 0;
                let base = p.base;
                let r = p.dev.write_u16(addr - base, value);
                (base, r)
            };
            if r.is_ok() {
                self.finish_esp32c3_io_mux_write(c3_io_mux_capture);
            }
            self.maybe_arm_hcsr04(idx);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(idx);
            if r.is_ok() {
                // Cache coherence — see the write_u32 note above.
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
}
