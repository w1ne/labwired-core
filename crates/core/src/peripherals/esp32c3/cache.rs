// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Minimal ESP32-C3 cache controller (EXTMEM, `0x600C_4000`).
//!
//! The boot ROM and 2nd-stage bootloader drive cache invalidate / writeback /
//! sync operations through a launch/done handshake: firmware sets a *launch*
//! bit, the cache controller performs the op, asserts a *done* status bit and
//! auto-clears the launch bit, and firmware busy-polls the done bit. The
//! simulator has no cache latency, so we complete the op atomically on the
//! launching write — exactly the observable contract, no fake state.
//!
//! C3 specifics (from the ROM `Cache_Invalidate_ICache_Items` routine, which
//! sets `0x28` bit0 then spins on `0x28` bit1): launch = bit0, done = bit1 at
//! offset `0x28`. This differs from the S3's EXTMEM (`launch bits[2:0]`, done
//! bit3), so it gets its own small model rather than reusing `Esp32s3Extmem`.
//! Other registers behave as plain read-back RAM until a real poll loop shows
//! one needs a specific idle value (the model is grown per spin loop, the same
//! way the S3 EXTMEM model was).

use crate::{Peripheral, SimResult};

/// Register-file size in 32-bit words (covers `0x000`..`0x400`).
const NUM_REGS: usize = 0x100;

/// `(byte_offset, done_mask)` — status/"done" bits the cache controller holds
/// asserted while idle and re-asserts the instant an op completes. Since the
/// simulator has no cache latency, every op is always "done": we force these
/// bits set on read, so the ROM/bootloader's busy-poll on completion exits
/// immediately instead of spinning forever. From the C3 EXTMEM register map
/// (soc/esp32c3/extmem_reg.h) — the ICACHE op-CTRL registers' DONE bits:
///   0x01C ICACHE_LOCK_CTRL     LOCK_DONE     = BIT(2)
///   0x028 ICACHE_SYNC_CTRL     SYNC_DONE     = BIT(1)
///   0x034 ICACHE_PRELOAD_CTRL  PRELOAD_DONE  = BIT(1)
///   0x040 ICACHE_AUTOLOAD_CTRL AUTOLOAD_DONE = BIT(3)
///   0x0B0 CACHE_STATE       ICACHE_STATE[11:0] rests at 0x1 (idle); the ROM
///                           cache enable/disable routines spin until state==1.
const DONE_BITS: [(usize, u32); 5] = [
    (0x01C, 1 << 2),
    (0x028, 1 << 1),
    (0x034, 1 << 1),
    (0x040, 1 << 3),
    (0x0B0, 1 << 0), // CACHE_STATE = idle (0x1)
];

/// `(byte_offset, request_mask, ack_mask)` — *level* handshakes where firmware
/// both freezes (set request, poll ack high) and unfreezes (clear request, poll
/// ack low) the SAME bit, so a static forced-set can't satisfy both. The ack
/// follows the request to its steady state instantly (no cache latency):
///   0x0CC ICACHE_FREEZE — request ENA = BIT(0), ack done = BIT(2).
const MIRROR_BITS: [(usize, u32, u32); 1] = [(0x0CC, 1 << 0, 1 << 2)];

#[derive(Debug)]
pub struct Esp32c3Cache {
    regs: Vec<u32>,
}

impl Default for Esp32c3Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c3Cache {
    pub fn new() -> Self {
        Self {
            regs: vec![0u32; NUM_REGS],
        }
    }
}

impl Peripheral for Esp32c3Cache {
    // Inert walk: cache ops complete atomically at the launching write (done bits forced on read); tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        let cur = self.read_u32(aligned)?;
        let merged = (cur & !(0xFFu32 << sh)) | ((value as u32) << sh);
        self.write_u32(aligned, merged)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let mut v = *self.regs.get((offset / 4) as usize).unwrap_or(&0);
        let aligned = (offset & !3) as usize;
        // Force one-shot "done" status bits set — ops complete with zero latency.
        for (off, done) in DONE_BITS {
            if aligned == off {
                v |= done;
            }
        }
        // Level handshakes: the ack bit follows the request bit to steady state.
        for (off, req, ack) in MIRROR_BITS {
            if aligned == off {
                if v & req != 0 {
                    v |= ack;
                } else {
                    v &= !ack;
                }
            }
        }
        Ok(v)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let i = (offset / 4) as usize;
        if i < self.regs.len() {
            self.regs[i] = value;
        }
        Ok(())
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
