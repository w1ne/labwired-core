// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32F4 DMA controller — STREAM-based (RM0090 §10 "DMA controller").
//!
//! The F4 DMA is a different IP block from the F1/L4 channel-based DMA modeled
//! in [`crate::peripherals::dma::Dma1`]: two controllers (DMA1 / DMA2) each with
//! **8 streams**, a channel-select field per stream (`SxCR.CHSEL`), a FIFO
//! (`SxFCR`), double-buffer mode (`SxCR.DBM`), and a split interrupt-status /
//! flag-clear register pair (LISR/HISR, LIFCR/HIFCR) instead of the F1's single
//! ISR/IFCR. The register layout and flag-bit geometry share nothing with the
//! channel model, so this is a **separate model** rather than a shared transfer
//! core parameterized by register layout — forcing the two irregular MMIO maps
//! through one body would couple unrelated silicon. What IS reused is the
//! **scheduler-migration machinery** (the `scheduler_mode` predicate, the
//! `service_stream_once` single transfer body shared by both drive modes, and
//! the `take_scheduled_events` / `on_event` chain), lifted verbatim in shape
//! from `Dma1` so the two drive modes are byte-identical by construction.
//!
//! ## Register map (RM0090 §10.5, per controller)
//!
//! | Offset            | Register  | Notes                                  |
//! |-------------------|-----------|----------------------------------------|
//! | 0x00              | LISR      | interrupt status, streams 0..3 (RO)    |
//! | 0x04              | HISR      | interrupt status, streams 4..7 (RO)    |
//! | 0x08              | LIFCR     | flag clear, streams 0..3 (W1C)         |
//! | 0x0C              | HIFCR     | flag clear, streams 4..7 (W1C)         |
//! | 0x10 + 0x18·s + 0 | SxCR      | stream config                          |
//! | ...          +0x04| SxNDTR    | number of data items (16-bit)          |
//! | ...          +0x08| SxPAR     | peripheral-port address                |
//! | ...          +0x0C| SxM0AR    | memory-0 address                       |
//! | ...          +0x10| SxM1AR    | memory-1 address (double-buffer)       |
//! | ...          +0x14| SxFCR     | FIFO control (reset 0x21)              |
//!
//! Interrupt-flag geometry inside LISR/HISR (RM0090 §10.5.1/2): the four streams
//! a register covers sit at base bits `[0, 6, 16, 22]`; within a stream FEIF=+0,
//! DMEIF=+2, TEIF=+3, HTIF=+4, TCIF=+5. Streams 0..3 → LISR/LIFCR, 4..7 →
//! HISR/HIFCR.
//!
//! ## Modeled behavior (RM0090 §10.3)
//!
//! - Stream enable (`SxCR.EN` 0→1) latches the transfer pointers and initial
//!   NDTR (RM0090 §10.3.17 — SxNDTR must be written before EN).
//! - One data item per transfer step: emit a [`DmaRequest`], decrement NDTR,
//!   advance the peripheral / memory internal pointers by the PSIZE / MSIZE
//!   width when PINC / MINC are set (§10.3.10, §10.3.11). The user-visible
//!   SxPAR / SxM0AR / SxM1AR registers stay at their programmed base, exactly
//!   like the F1 model and real STM32 silicon.
//! - Direction (`SxCR.DIR`): 00 peripheral-to-memory, 01 memory-to-peripheral,
//!   10 memory-to-memory. The peripheral port (SxPAR) is always the "peripheral
//!   side"; the memory port (SxM0AR / SxM1AR) the "memory side".
//! - HTIF latches at the half-transfer crossing, TCIF at completion; the
//!   stream's own NVIC line is pended on HTIE / TCIE (each F4 DMA stream has its
//!   own interrupt vector — modeled via `explicit_irqs` / per-stream routing).
//! - Circular mode (`SxCR.CIRC`) and double-buffer mode (`SxCR.DBM`) reload
//!   NDTR and the pointers at completion; DBM additionally toggles `SxCR.CT` and
//!   swaps the active memory pointer between SxM0AR and SxM1AR (§10.3.9).
//! - **Memory-to-memory is DMA2 only** (RM0090 §10.3.3): a DIR=10 enable on
//!   DMA1 never starts a transfer.
//!
//! ## Deferred (documented, not silently diverged)
//!
//! - The FIFO (`SxFCR`) is storage only (round-trips, reset 0x21). FIFO
//!   threshold / burst pacing (`FTH`, `PBURST` / `MBURST`) is not behaviorally
//!   modeled — transfers are paced one data item per step, matching the F1
//!   model. `SxFCR.FS` (FIFO fill status) reads back the stored value rather
//!   than a live level.
//! - Stream-priority arbitration (`SxCR.PL`) is stored but does not reorder
//!   concurrent streams; each active stream advances one item per step.
//! - The transfer/FIFO error flags (TEIF/DMEIF/FEIF) are storage in LISR/HISR
//!   and clearable via LIFCR/HIFCR, but no error condition sets them (the sim
//!   bus never reports a bus/FIFO error).

use crate::{CycleClock, DmaDirection, DmaRequest, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

const NUM_STREAMS: usize = 8;
const STREAM_BASE: u64 = 0x10;
const STREAM_STRIDE: u64 = 0x18;

// SxCR bit fields (RM0090 §10.5.5).
const CR_EN: u32 = 1 << 0;
const CR_HTIE: u32 = 1 << 3;
const CR_TCIE: u32 = 1 << 4;
const CR_DIR_SHIFT: u32 = 6; // DIR[1:0]
const CR_CIRC: u32 = 1 << 8;
const CR_PINC: u32 = 1 << 9;
const CR_MINC: u32 = 1 << 10;
const CR_PSIZE_SHIFT: u32 = 11; // PSIZE[1:0]
const CR_MSIZE_SHIFT: u32 = 13; // MSIZE[1:0]
const CR_DBM: u32 = 1 << 18;
const CR_CT: u32 = 1 << 19;
// SxCR writable mask: bits [27:0] are implemented (CHSEL[27:25], MBURST[24:23],
// PBURST[22:21], CT[19], DBM[18], PL[17:16], PINCOS[15], MSIZE[14:13],
// PSIZE[12:11], MINC[10], PINC[9], CIRC[8], DIR[7:6], PFCTRL[5], TCIE[4],
// HTIE[3], TEIE[2], DMEIE[1], EN[0]). Bits [31:28] reserved, read 0.
const CR_WRITABLE_MASK: u32 = 0x0FFF_FFFF;

// Compact internal per-stream flag bits (mapped to register geometry on read).
const F_FEIF: u8 = 1 << 0;
const F_DMEIF: u8 = 1 << 1;
const F_TEIF: u8 = 1 << 2;
const F_HTIF: u8 = 1 << 3;
const F_TCIF: u8 = 1 << 4;

/// Base bit of stream position `p` (0..3) inside a LISR/HISR register.
const STREAM_FLAG_BASE: [u32; 4] = [0, 6, 16, 22];

/// Map a compact `flags` byte to its LISR/HISR register bits for stream
/// position `p` (0..3 within the register).
fn flags_to_reg_bits(flags: u8, p: usize) -> u32 {
    let base = STREAM_FLAG_BASE[p];
    let mut v = 0u32;
    if flags & F_FEIF != 0 {
        v |= 1 << base;
    }
    if flags & F_DMEIF != 0 {
        v |= 1 << (base + 2);
    }
    if flags & F_TEIF != 0 {
        v |= 1 << (base + 3);
    }
    if flags & F_HTIF != 0 {
        v |= 1 << (base + 4);
    }
    if flags & F_TCIF != 0 {
        v |= 1 << (base + 5);
    }
    v
}

/// Which compact flag bits a LIFCR/HIFCR write clears for stream position `p`.
fn reg_bits_to_flags(value: u32, p: usize) -> u8 {
    let base = STREAM_FLAG_BASE[p];
    let mut f = 0u8;
    if value & (1 << base) != 0 {
        f |= F_FEIF;
    }
    if value & (1 << (base + 2)) != 0 {
        f |= F_DMEIF;
    }
    if value & (1 << (base + 3)) != 0 {
        f |= F_TEIF;
    }
    if value & (1 << (base + 4)) != 0 {
        f |= F_HTIF;
    }
    if value & (1 << (base + 5)) != 0 {
        f |= F_TCIF;
    }
    f
}

/// Data-item width in bytes from a 2-bit PSIZE/MSIZE encoding (00=1,01=2,10=4;
/// 11 reserved, clamped to 4).
fn size_width(size2: u32) -> u32 {
    1 << (size2 & 0x3).min(2)
}

#[derive(Debug, Default, serde::Serialize)]
struct DmaStream {
    cr: u32,
    ndtr: u32,
    par: u32,
    m0ar: u32,
    m1ar: u32,
    /// SxFCR — FIFO control. Reset value 0x21 (FTH=01, FS=100); storage only.
    fcr: u32,
    /// Interrupt flags, compact encoding (see `F_*`).
    flags: u8,
    active: bool,
    /// Internal transfer pointers — silicon leaves SxPAR/SxM0AR/SxM1AR readable
    /// at their programmed base while a transfer advances internal counters.
    par_ptr: u32,
    mem_ptr: u32,
    /// NDTR snapshot at enable, for the half-transfer (HTIF) crossing and for
    /// circular / double-buffer reload.
    ndtr_initial: u32,
    /// Scheduler mode only: a transfer event is live in the heap for this
    /// stream. Mirrors `Dma1::DmaChannel::chain_live`.
    #[serde(skip)]
    chain_live: bool,
}

impl DmaStream {
    fn dir(&self) -> u32 {
        (self.cr >> CR_DIR_SHIFT) & 0x3
    }
    fn psize_width(&self) -> u32 {
        size_width(self.cr >> CR_PSIZE_SHIFT)
    }
    fn msize_width(&self) -> u32 {
        size_width(self.cr >> CR_MSIZE_SHIFT)
    }
    /// Base address of the currently-active memory buffer (M0AR, or M1AR when
    /// double-buffer's CT bit selects buffer 1).
    fn active_mem_base(&self) -> u32 {
        if self.cr & CR_CT != 0 {
            self.m1ar
        } else {
            self.m0ar
        }
    }
}

/// STM32F4 DMA controller — 8 streams. `is_dma2` gates the memory-to-memory
/// mode (RM0090 §10.3.3: M2M is DMA2-only).
#[derive(Debug, Default, serde::Serialize)]
pub struct StreamDma {
    streams: [DmaStream; NUM_STREAMS],
    /// True for the DMA2 instance — enables memory-to-memory transfers.
    is_dma2: bool,
    /// Per-stream NVIC vector numbers (RM0090 / stm32f407xx.h). Stream `s` pends
    /// `stream_irqs[s]`. Empty → fall back to the block's single configured line.
    stream_irqs: Vec<u32>,
    /// Bus-published cycle clock (walk-free machinery). `Some` once the bus
    /// registration choke attaches it; `None` keeps the model on the legacy walk.
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl StreamDma {
    pub fn new() -> Self {
        let mut d = Self::default();
        for s in d.streams.iter_mut() {
            s.fcr = 0x21; // SxFCR reset value (RM0090 §10.5.10).
        }
        d
    }

    /// DMA2 instance — enables memory-to-memory mode.
    pub fn as_dma2(mut self) -> Self {
        self.is_dma2 = true;
        self
    }

    /// Set the per-stream NVIC vector numbers (stream `s` → `irqs[s]`).
    pub fn with_stream_irqs(mut self, irqs: Vec<u32>) -> Self {
        self.stream_irqs = irqs;
        self
    }

    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy walk path. Mirrors `Dma1::force_legacy_walk`.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.isr(0), // LISR: streams 0..3
            0x04 => self.isr(4), // HISR: streams 4..7
            0x08 | 0x0C => 0,    // LIFCR / HIFCR are write-1-to-clear, read 0
            _ if offset >= STREAM_BASE => {
                let s = ((offset - STREAM_BASE) / STREAM_STRIDE) as usize;
                let reg = (offset - STREAM_BASE) % STREAM_STRIDE;
                if s >= NUM_STREAMS {
                    return 0;
                }
                let st = &self.streams[s];
                match reg {
                    0x00 => st.cr,
                    0x04 => st.ndtr,
                    0x08 => st.par,
                    0x0C => st.m0ar,
                    0x10 => st.m1ar,
                    0x14 => st.fcr,
                    _ => 0,
                }
            }
            _ => 0,
        }
    }

    /// Assemble the LISR (base_stream=0) or HISR (base_stream=4) value from the
    /// four covered streams' compact flags.
    fn isr(&self, base_stream: usize) -> u32 {
        let mut v = 0u32;
        for p in 0..4 {
            v |= flags_to_reg_bits(self.streams[base_stream + p].flags, p);
        }
        v
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 | 0x04 => {}                   // LISR / HISR read-only
            0x08 => self.clear_flags(0, value), // LIFCR: streams 0..3
            0x0C => self.clear_flags(4, value), // HIFCR: streams 4..7
            _ if offset >= STREAM_BASE => {
                let s = ((offset - STREAM_BASE) / STREAM_STRIDE) as usize;
                let reg = (offset - STREAM_BASE) % STREAM_STRIDE;
                if s >= NUM_STREAMS {
                    return;
                }
                match reg {
                    0x00 => self.write_cr(s, value),
                    0x04 => self.streams[s].ndtr = value & 0xFFFF,
                    0x08 => self.streams[s].par = value,
                    0x0C => self.streams[s].m0ar = value,
                    0x10 => self.streams[s].m1ar = value,
                    0x14 => self.streams[s].fcr = value & 0xBF, // FEIE|DMDIS|FTH; FS is RO
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn clear_flags(&mut self, base_stream: usize, value: u32) {
        for p in 0..4 {
            let clear = reg_bits_to_flags(value, p);
            self.streams[base_stream + p].flags &= !clear;
        }
    }

    fn write_cr(&mut self, s: usize, value: u32) {
        let st = &mut self.streams[s];
        let old_en = st.cr & CR_EN != 0;
        st.cr = value & CR_WRITABLE_MASK;
        let new_en = st.cr & CR_EN != 0;
        if !old_en && new_en {
            let dir = st.dir();
            // RM0090 §10.3.3: memory-to-memory (DIR=10) is DMA2-only; a DIR=10
            // enable on DMA1 never starts.
            if dir == 0b10 && !self.is_dma2 {
                return;
            }
            st.active = true;
            st.par_ptr = st.par;
            st.mem_ptr = st.active_mem_base();
            st.ndtr_initial = st.ndtr;
        } else if old_en && !new_en {
            // Disabling clears EN; silicon completes the current item then
            // stops. We mirror `Dma1`: `active` is sticky so an in-flight
            // self-paced transfer runs to completion (the event chain reads
            // live `active`). Peripheral-paced streams go inert after one item
            // regardless.
        }
    }

    /// Transfer one data item on stream `s` if it is active with NDTR>0.
    /// Returns the emitted [`DmaRequest`] and whether this item pends the
    /// stream's NVIC line (HTIE/TCIE). The single body both drive modes call.
    fn service_stream_once(&mut self, s: usize) -> (Option<DmaRequest>, bool) {
        let st = &mut self.streams[s];
        if !(st.active && st.ndtr > 0) {
            return (None, false);
        }
        let dir = st.dir();
        let mem2mem = dir == 0b10;

        // SxPAR is always the peripheral side, SxM0AR/M1AR the memory side.
        let (src, dst, direction) = match dir {
            0b01 => (st.mem_ptr, st.par_ptr, DmaDirection::Write), // memory → peripheral
            0b10 => (st.par_ptr, st.mem_ptr, DmaDirection::Copy),  // memory → memory
            _ => (st.par_ptr, st.mem_ptr, DmaDirection::Read),     // peripheral → memory
        };
        let request = DmaRequest {
            src_addr: src as u64,
            addr: dst as u64,
            val: 0,
            direction,
            transform: None,
        };

        st.ndtr -= 1;
        if st.cr & CR_PINC != 0 {
            st.par_ptr = st.par_ptr.wrapping_add(st.psize_width());
        }
        if st.cr & CR_MINC != 0 {
            st.mem_ptr = st.mem_ptr.wrapping_add(st.msize_width());
        }

        let ndtr_after = st.ndtr;
        let ndtr_initial = st.ndtr_initial;
        let htie = st.cr & CR_HTIE != 0;
        let tcie = st.cr & CR_TCIE != 0;
        let circ = st.cr & CR_CIRC != 0;
        let dbm = st.cr & CR_DBM != 0;
        let mut irq = false;

        // HTIF at the half-transfer crossing (latched once until cleared).
        if ndtr_initial >= 2 && ndtr_after <= ndtr_initial / 2 && st.flags & F_HTIF == 0 {
            st.flags |= F_HTIF;
            if htie {
                irq = true;
            }
        }

        if ndtr_after == 0 {
            st.flags |= F_TCIF;
            if tcie {
                irq = true;
            }
            if dbm {
                // Double-buffer (RM0090 §10.3.9): toggle CT, swap the active
                // memory buffer, reload NDTR and the pointers, keep running.
                st.cr ^= CR_CT;
                st.ndtr = ndtr_initial;
                st.par_ptr = st.par;
                st.mem_ptr = st.active_mem_base();
                st.active = mem2mem; // request-paced streams re-arm on request
            } else if circ {
                // Circular (RM0090 §10.3.8): reload NDTR and pointers, continue.
                st.ndtr = ndtr_initial;
                st.par_ptr = st.par;
                st.mem_ptr = st.active_mem_base();
                st.active = mem2mem;
            } else {
                st.active = false;
            }
        } else if !mem2mem {
            // Peripheral-request paced: one item per request, then inert.
            st.active = false;
        }

        (Some(request), irq)
    }

    fn tick_streams_once(&mut self) -> PeripheralTickResult {
        let mut dma_requests: Option<Vec<DmaRequest>> = None;
        let mut explicit_irqs: Option<Vec<u32>> = None;
        let mut irq = false;
        for s in 0..NUM_STREAMS {
            let (req, pend) = self.service_stream_once(s);
            if let Some(r) = req {
                dma_requests.get_or_insert_with(Vec::new).push(r);
            }
            if pend {
                self.pend_stream(s, &mut explicit_irqs, &mut irq);
            }
        }
        PeripheralTickResult {
            irq,
            // Tick-cost normalization (mirrors `Dma1`): a DMA is a bus master,
            // it does not steal CPU cycles — charge zero so the walk-on
            // reference and the scheduler path agree cycle-for-cycle.
            cycles: 0,
            dma_requests,
            explicit_irqs,
            ..Default::default()
        }
    }

    /// Route a stream's completion/half IRQ: its own NVIC vector when known,
    /// else the block's single configured line.
    fn pend_stream(&self, s: usize, explicit: &mut Option<Vec<u32>>, own: &mut bool) {
        match self.stream_irqs.get(s) {
            Some(&line) => explicit.get_or_insert_with(Vec::new).push(line),
            None => *own = true,
        }
    }
}

impl Peripheral for StreamDma {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);
        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn dma_request(&mut self, request_id: u32) {
        // request_id maps 1..8 to stream 0..7.
        let s = request_id.saturating_sub(1) as usize;
        if s < NUM_STREAMS && self.streams[s].cr & CR_EN != 0 {
            self.streams[s].active = true;
        }
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.tick_streams_once()
    }

    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        self.tick_streams_once()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn sync_to(&mut self, _now_cycle: u64) {
        // Every readable register mutates only at a transfer event; nothing to
        // replay on the write path.
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        let mut events = Vec::new();
        for s in 0..NUM_STREAMS {
            let st = &mut self.streams[s];
            if st.active && st.ndtr > 0 && !st.chain_live {
                st.chain_live = true;
                events.push((0u64, s as u32));
            }
        }
        events
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let _ = sched;
        let s = event_token as usize;
        if !self.scheduler_mode() || s >= NUM_STREAMS {
            return crate::sched::EventResult::default();
        }
        let (req, pend) = self.service_stream_once(s);
        let mut explicit = None;
        let mut own = false;
        if pend {
            self.pend_stream(s, &mut explicit, &mut own);
        }
        let st = &mut self.streams[s];
        let reschedule = st.active && st.ndtr > 0;
        st.chain_live = reschedule;
        crate::sched::EventResult {
            raise_own_irq: own,
            explicit_irqs: explicit.unwrap_or_default(),
            reschedule_delay: reschedule.then_some(1),
            dma_requests: req.into_iter().collect(),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Absolute stream-register offsets for stream `s`.
    fn cr(s: u64) -> u64 {
        STREAM_BASE + s * STREAM_STRIDE
    }
    fn ndtr(s: u64) -> u64 {
        cr(s) + 0x04
    }
    fn par(s: u64) -> u64 {
        cr(s) + 0x08
    }
    fn m0ar(s: u64) -> u64 {
        cr(s) + 0x0C
    }

    #[test]
    fn fcr_resets_to_0x21() {
        let dma = StreamDma::new();
        for s in 0..NUM_STREAMS as u64 {
            assert_eq!(dma.read_reg(cr(s) + 0x14), 0x21, "stream {s} SxFCR reset");
        }
    }

    #[test]
    fn stream_regs_round_trip_and_are_independent() {
        let mut dma = StreamDma::new();
        dma.write_reg(par(3), 0x4000_5400);
        dma.write_reg(m0ar(3), 0x2000_0100);
        dma.write_reg(ndtr(3), 0x1_2345); // only low 16 bits kept
        assert_eq!(dma.read_reg(par(3)), 0x4000_5400);
        assert_eq!(dma.read_reg(m0ar(3)), 0x2000_0100);
        assert_eq!(dma.read_reg(ndtr(3)), 0x2345);
        // Neighbors untouched.
        assert_eq!(dma.read_reg(par(2)), 0);
        assert_eq!(dma.read_reg(par(4)), 0);
    }

    #[test]
    fn m2m_is_dma2_only() {
        // DIR=10 (mem-to-mem) on a DMA1 instance must not start.
        let mut dma1 = StreamDma::new();
        dma1.write_reg(ndtr(0), 4);
        dma1.write_reg(par(0), 0x2000_0000);
        dma1.write_reg(m0ar(0), 0x2000_0100);
        dma1.write_reg(cr(0), CR_EN | (0b10 << CR_DIR_SHIFT) | CR_PINC | CR_MINC);
        for _ in 0..8 {
            dma1.tick();
        }
        // Never started: NDTR unchanged, no TCIF.
        assert_eq!(dma1.read_reg(ndtr(0)), 4, "DMA1 M2M must not start");
        assert_eq!(dma1.read_reg(0x00) & flags_to_reg_bits(F_TCIF, 0), 0);

        // Same program on DMA2 completes.
        let mut dma2 = StreamDma::new().as_dma2();
        dma2.write_reg(ndtr(0), 4);
        dma2.write_reg(par(0), 0x2000_0000);
        dma2.write_reg(m0ar(0), 0x2000_0100);
        dma2.write_reg(cr(0), CR_EN | (0b10 << CR_DIR_SHIFT) | CR_PINC | CR_MINC);
        for _ in 0..4 {
            dma2.tick();
        }
        assert_eq!(dma2.read_reg(ndtr(0)), 0, "DMA2 M2M drains");
        assert_ne!(
            dma2.read_reg(0x00) & flags_to_reg_bits(F_TCIF, 0),
            0,
            "TCIF set"
        );
    }

    #[test]
    fn m2m_transfer_requests_ndtr_increment_and_tcif() {
        let mut dma = StreamDma::new().as_dma2();
        dma.write_reg(ndtr(0), 4);
        dma.write_reg(par(0), 0x2000_0000); // source (peripheral port)
        dma.write_reg(m0ar(0), 0x2000_0100); // dest (memory port)
                                             // EN|DIR=M2M|PINC|MINC|TCIE, byte width.
        dma.write_reg(
            cr(0),
            CR_EN | (0b10 << CR_DIR_SHIFT) | CR_PINC | CR_MINC | CR_TCIE,
        );

        // First item: Copy from PAR to M0AR, addresses advance by 1 (byte).
        let r0 = dma.tick();
        let reqs = r0.dma_requests.unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].direction, DmaDirection::Copy);
        assert_eq!(reqs[0].src_addr, 0x2000_0000);
        assert_eq!(reqs[0].addr, 0x2000_0100);
        assert_eq!(dma.read_reg(ndtr(0)), 3, "NDTR decremented");

        let r1 = dma.tick();
        assert_eq!(
            r1.dma_requests.unwrap()[0].src_addr,
            0x2000_0001,
            "PINC advanced"
        );

        for _ in 0..2 {
            dma.tick();
        }
        assert_eq!(dma.read_reg(ndtr(0)), 0);
        // TCIF (stream 0 → LISR) latched.
        assert_ne!(dma.read_reg(0x00) & flags_to_reg_bits(F_TCIF, 0), 0);
        // User-visible SxPAR/SxM0AR stay at programmed base.
        assert_eq!(dma.read_reg(par(0)), 0x2000_0000);
        assert_eq!(dma.read_reg(m0ar(0)), 0x2000_0100);
    }

    #[test]
    fn tcif_pends_stream_irq_and_lifcr_clears() {
        let mut dma = StreamDma::new()
            .as_dma2()
            .with_stream_irqs(vec![56, 57, 58, 59, 60, 68, 69, 70]);
        dma.write_reg(ndtr(5), 2);
        dma.write_reg(par(5), 0x2000_0000);
        dma.write_reg(m0ar(5), 0x2000_0100);
        dma.write_reg(cr(5), CR_EN | (0b10 << CR_DIR_SHIFT) | CR_MINC | CR_TCIE);
        dma.tick();
        let r = dma.tick(); // completes
                            // Stream 5 pends its own NVIC vector (68), not the block's own line.
        assert!(!r.irq);
        assert_eq!(r.explicit_irqs.as_deref(), Some(&[68u32][..]));
        // Stream 5 is in HISR (position 1). TCIF set there.
        assert_ne!(dma.read_reg(0x04) & flags_to_reg_bits(F_TCIF, 1), 0);
        // HIFCR (0x0C) write-1-clears it.
        dma.write_reg(0x0C, flags_to_reg_bits(F_TCIF, 1));
        assert_eq!(dma.read_reg(0x04) & flags_to_reg_bits(F_TCIF, 1), 0);
    }

    #[test]
    fn circular_mode_reloads_ndtr() {
        let mut dma = StreamDma::new().as_dma2();
        dma.write_reg(ndtr(0), 3);
        dma.write_reg(par(0), 0x2000_0000);
        dma.write_reg(m0ar(0), 0x2000_0100);
        // M2M + CIRC — self-paced, reloads on completion.
        dma.write_reg(cr(0), CR_EN | (0b10 << CR_DIR_SHIFT) | CR_MINC | CR_CIRC);
        for _ in 0..3 {
            dma.tick();
        }
        // Reloaded, still running.
        assert_eq!(dma.read_reg(ndtr(0)), 3, "NDTR reloaded in circular mode");
        dma.tick();
        assert_eq!(dma.read_reg(ndtr(0)), 2);
    }

    #[cfg(feature = "event-scheduler")]
    mod scheduler_mode {
        use super::*;
        use crate::CycleClock;

        struct SchedHarness {
            dma: StreamDma,
            clock: CycleClock,
            bus: crate::bus::SystemBus,
            events: Vec<(u64, u32)>,
            now: u64,
            pends: Vec<(u64, Vec<u32>)>,
            reqs: Vec<(u64, DmaRequest)>,
        }

        impl SchedHarness {
            fn new(is_dma2: bool) -> Self {
                let clock = CycleClock::default();
                let mut dma = StreamDma::new();
                if is_dma2 {
                    dma = dma.as_dma2();
                }
                dma = dma.with_stream_irqs(vec![11, 12, 13, 14, 15, 16, 17, 47]);
                dma.attach_cycle_clock(clock.clone());
                Self {
                    dma,
                    clock,
                    bus: crate::bus::SystemBus::new(),
                    events: Vec::new(),
                    now: 0,
                    pends: Vec::new(),
                    reqs: Vec::new(),
                }
            }
            fn write(&mut self, offset: u64, value: u32) {
                self.dma.sync_to(self.now);
                self.dma.write_reg(offset, value);
                for (delay, token) in self.dma.take_scheduled_events() {
                    self.events.push((self.now + 1 + delay, token));
                }
            }
            fn request(&mut self, id: u32) {
                self.dma.dma_request(id);
                for (delay, token) in self.dma.take_scheduled_events() {
                    self.events.push((self.now + 1 + delay, token));
                }
            }
            fn step(&mut self) {
                self.now += 1;
                self.clock.publish(self.now);
                let due: Vec<(u64, u32)> = self
                    .events
                    .iter()
                    .copied()
                    .filter(|(d, _)| *d <= self.now)
                    .collect();
                self.events.retain(|(d, _)| *d > self.now);
                let mut sched = crate::sched::EventScheduler::new();
                sched.advance_to(self.now);
                for (_, token) in due {
                    let res = self.dma.on_event(token, &mut sched, &mut self.bus);
                    let mut lines = res.explicit_irqs.clone();
                    if res.raise_own_irq {
                        lines.push(u32::MAX); // sentinel: own-line pend
                    }
                    if !lines.is_empty() {
                        self.pends.push((self.now, lines));
                    }
                    for r in res.dma_requests {
                        self.reqs.push((self.now, r));
                    }
                    if let Some(delay) = res.reschedule_delay {
                        self.events.push((self.now + delay, token));
                    }
                }
            }
        }

        #[derive(Clone, Copy)]
        enum Op {
            Write(u64, u32),
            Request(u32),
        }

        /// Replay a script against (a) the legacy walk and (b) the event path;
        /// compare full register snapshot every cycle, the emitted request
        /// stream, and the per-cycle IRQ pend set.
        fn assert_walk_identical(is_dma2: bool, script: &[(u64, Op)], cycles: u64, what: &str) {
            let mut walk = StreamDma::new();
            if is_dma2 {
                walk = walk.as_dma2();
            }
            walk = walk.with_stream_irqs(vec![11, 12, 13, 14, 15, 16, 17, 47]);
            let mut sched = SchedHarness::new(is_dma2);

            let mut walk_pends: Vec<(u64, Vec<u32>)> = Vec::new();
            let mut walk_reqs: Vec<(u64, DmaRequest)> = Vec::new();

            for c in 1..=cycles {
                for (sc, op) in script {
                    if *sc == c {
                        match *op {
                            Op::Write(off, val) => {
                                walk.write_reg(off, val);
                                sched.now = c - 1;
                                sched.write(off, val);
                            }
                            Op::Request(id) => {
                                walk.dma_request(id);
                                sched.now = c - 1;
                                sched.request(id);
                            }
                        }
                    }
                }
                let res = walk.tick();
                let mut lines = res.explicit_irqs.clone().unwrap_or_default();
                if res.irq {
                    lines.push(u32::MAX);
                }
                if !lines.is_empty() {
                    walk_pends.push((c, lines));
                }
                for r in res.dma_requests.unwrap_or_default() {
                    walk_reqs.push((c, r));
                }
                sched.now = c - 1;
                sched.step();

                assert_eq!(
                    walk.snapshot(),
                    sched.dma.snapshot(),
                    "{what}: register state diverged at cycle {c}"
                );
            }
            assert_eq!(walk_pends, sched.pends, "{what}: IRQ pend cycles diverged");
            assert_eq!(walk_reqs, sched.reqs, "{what}: request stream diverged");
        }

        #[test]
        fn clock_attach_flips_scheduler_and_tick_is_inert() {
            let mut dma = StreamDma::new().as_dma2();
            dma.attach_cycle_clock(CycleClock::default());
            assert!(dma.uses_scheduler());
            assert!(!dma.needs_legacy_walk());
            dma.write_reg(ndtr(0), 4);
            dma.write_reg(cr(0), CR_EN | (0b10 << CR_DIR_SHIFT));
            assert!(
                dma.tick().dma_requests.is_none(),
                "tick inert in scheduler mode"
            );
            assert_eq!(dma.read_reg(ndtr(0)), 4, "no item transferred by tick");
        }

        #[test]
        fn m2m_self_paced_walk_identity() {
            let script = [
                (1u64, Op::Write(ndtr(0), 8)),
                (1, Op::Write(par(0), 0x2000_0000)),
                (1, Op::Write(m0ar(0), 0x2000_0100)),
                (
                    1,
                    Op::Write(cr(0), CR_EN | (0b10 << CR_DIR_SHIFT) | CR_PINC | CR_MINC),
                ),
                (20, Op::Write(0x08, 0xFFFF_FFFF)), // LIFCR clear-all mid-idle
            ];
            assert_walk_identical(true, &script, 40, "m2m 8-item self-paced");
        }

        #[test]
        fn m2m_htie_tcie_irq_walk_identity() {
            let script = [
                (1u64, Op::Write(ndtr(0), 10)),
                (1, Op::Write(par(0), 0x2000_0000)),
                (1, Op::Write(m0ar(0), 0x2000_0100)),
                (
                    1,
                    Op::Write(
                        cr(0),
                        CR_EN | (0b10 << CR_DIR_SHIFT) | CR_PINC | CR_MINC | CR_HTIE | CR_TCIE,
                    ),
                ),
            ];
            assert_walk_identical(true, &script, 30, "m2m HTIE+TCIE stream IRQ");
        }

        #[test]
        fn peripheral_paced_request_driven_walk_identity() {
            let script = [
                (1u64, Op::Write(ndtr(2), 5)),
                (1, Op::Write(par(2), 0x4000_0000)),
                (1, Op::Write(m0ar(2), 0x2000_0100)),
                // P2M (DIR=00) | MINC | TCIE.
                (1, Op::Write(cr(2), CR_EN | CR_MINC | CR_TCIE)),
                (6, Op::Request(3)),
                (10, Op::Request(3)),
                (14, Op::Request(3)),
                (18, Op::Request(3)),
            ];
            assert_walk_identical(false, &script, 30, "peripheral-paced request-driven");
        }

        #[test]
        fn two_concurrent_streams_walk_identity() {
            let script = [
                (1u64, Op::Write(ndtr(0), 5)),
                (1, Op::Write(par(0), 0x2000_0000)),
                (1, Op::Write(m0ar(0), 0x2000_0100)),
                (1, Op::Write(cr(0), CR_EN | (0b10 << CR_DIR_SHIFT))),
                (1, Op::Write(ndtr(7), 4)),
                (1, Op::Write(par(7), 0x2000_0200)),
                (1, Op::Write(m0ar(7), 0x2000_0300)),
                (1, Op::Write(cr(7), CR_EN | (0b10 << CR_DIR_SHIFT))),
            ];
            assert_walk_identical(true, &script, 20, "two concurrent M2M streams");
        }
    }
}
