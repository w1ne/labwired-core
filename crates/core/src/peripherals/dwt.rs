// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::atomic::{AtomicU32, Ordering};

const DWT_CTRL: u64 = 0x00;
const DWT_CYCCNT: u64 = 0x04;

#[derive(Debug)]
pub struct Dwt {
    ctrl: AtomicU32,
    cyccnt: AtomicU32,
}

impl Dwt {
    pub fn new() -> Self {
        Self {
            ctrl: AtomicU32::new(0),
            cyccnt: AtomicU32::new(0),
        }
    }
}

impl Default for Dwt {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Dwt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let val = match offset & !3 {
            DWT_CTRL => self.ctrl.load(Ordering::SeqCst),
            DWT_CYCCNT => self.cyccnt.load(Ordering::SeqCst),
            _ => 0,
        };

        let byte_offset = (offset & 3) as u32;
        Ok(((val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // We only support word-aligned writes for simplicity in this MVP,
        // or we need a read-modify-write buffer.
        // For DWT, usually 32-bit access is used.
        // But `write` takes u8.
        // To properly support byte writes we need either internal state or
        // we can implement `write_u32` if the trait supported it directly/optimally,
        // but `Peripheral` trait is byte-oriented.
        //
        // However, `Bus` breaks down u32 writes into 4 u8 writes.
        // We can use a simplified approach: just update the byte in the atomic.
        // But Atomically updating a single byte in a U32 is tricky without CAS loop.
        //
        // Simplification: Load, modify, store.

        let aligned_offset = offset & !3;
        let byte_shift = (offset & 3) * 8;

        let (atom, _is_cyccnt) = match aligned_offset {
            DWT_CTRL => (&self.ctrl, false),
            DWT_CYCCNT => (&self.cyccnt, true),
            _ => return Ok(()),
        };

        let mut current = atom.load(Ordering::SeqCst);
        let mask = 0xFF << byte_shift;
        current &= !mask;
        current |= (value as u32) << byte_shift;
        atom.store(current, Ordering::SeqCst);

        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Check CYCCNTENA bit (bit 0) in CTRL
        let ctrl = self.ctrl.load(Ordering::Relaxed);
        if (ctrl & 1) != 0 {
            self.cyccnt.fetch_add(1, Ordering::Relaxed);
        }

        PeripheralTickResult {
            irq: false,
            cycles: 0,
            dma_requests: Vec::new(),
            explicit_irqs: Vec::new(),
        }
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let val = match offset & !3 {
            DWT_CTRL => self.ctrl.load(Ordering::Relaxed),
            DWT_CYCCNT => self.cyccnt.load(Ordering::Relaxed),
            _ => return None,
        };
        let byte_offset = (offset & 3) as u32;
        Some(((val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "ctrl": self.ctrl.load(Ordering::Relaxed),
            "cyccnt": self.cyccnt.load(Ordering::Relaxed),
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Some(obj) = state.as_object() {
            if let Some(ctrl) = obj.get("ctrl").and_then(|v| v.as_u64()) {
                self.ctrl.store(ctrl as u32, Ordering::Relaxed);
            }
            if let Some(cyccnt) = obj.get("cyccnt").and_then(|v| v.as_u64()) {
                self.cyccnt.store(cyccnt as u32, Ordering::Relaxed);
            }
        }
        Ok(())
    }
}
