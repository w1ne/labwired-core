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
//! **scheduler-migration machinery** (the `scheduler_mode` predicate, ONE slot
//! body shared by both drive modes, and the `take_scheduled_events` /
//! `on_event` chain), lifted in shape from `Dma1` so the two drive modes are
//! byte-identical by construction.
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
//! Interrupt-flag geometry inside LISR/HISR (RM0090 §10.5.1/2, F401 SVD): the
//! four streams a register covers sit at base bits `[0, 6, 16, 22]`; within a
//! stream FEIF=+0, DMEIF=+2, TEIF=+3, HTIF=+4, TCIF=+5. Streams 0..3 →
//! LISR/LIFCR, 4..7 → HISR/HIFCR.
//!
//! ## Arbitration (RM0090 "Arbiter")
//!
//! The controller runs ONE stream per arbitration slot (one slot per cycle).
//! Among the streams with something to do — a latched peripheral request, a
//! FIFO to fill or drain, a memory-to-memory transfer — the arbiter picks the
//! highest software priority (`SxCR.PL`, 11 very high .. 00 low), then the
//! lowest stream number. A request that loses the slot stays latched.
//!
//! ## Direct mode (`SxFCR.DMDIS` = 0, the reset state)
//!
//! Each peripheral request moves one data item straight from source to
//! destination (a [`DmaRequest`] copy). As on silicon, enabling the stream
//! forces `MSIZE` to `PSIZE`, and `MBURST` / `PBURST` / `PINCOS` to 0
//! (RM0090 §10.5.5). Memory-to-memory forces FIFO mode (`DMDIS` reads 1).
//!
//! ## FIFO mode (`DMDIS` = 1, and always for memory-to-memory)
//!
//! A 4-word (16-byte) FIFO sits between the two ports (RM0090 "FIFO"):
//!
//! * **Peripheral-to-memory** (and memory-to-memory, whose source is the
//!   peripheral port `SxPAR`): each request (each slot, for M2M) reads one
//!   `PSIZE` item — or a `PBURST` of 4/8/16 items for ONE request — into the
//!   FIFO ([`DmaDirection::Load`]; the bytes come back through
//!   [`Peripheral::dma_load_data`]). When the FIFO holds the `FTH` threshold
//!   (1/4, 1/2, 3/4 or full) the memory port drains that many bytes as
//!   `MSIZE` beats (`MBURST` bursts) ([`DmaDirection::Store`]). At the end of
//!   the transfer (NDTR = 0), and when firmware clears `EN`, what is left is
//!   flushed to memory first.
//! * **Memory-to-peripheral**: the FIFO is filled from memory as soon as the
//!   stream is enabled, and refilled whenever its level is at or below the
//!   threshold; each request writes one `PSIZE` item (or a `PBURST`) from the
//!   FIFO to the peripheral. Clearing `EN` drops what is buffered.
//! * **Packing / unpacking**: the FIFO is a byte queue, so `PSIZE` ≠ `MSIZE`
//!   packs bytes into half-words / words little-endian, as the hardware does.
//!   `SxNDTR` counts `PSIZE` items in every direction.
//! * `SxFCR.FS` reads the live level (100 empty, 101 full, 000..011 by
//!   quarters).
//!
//! ## Errors and stopping
//!
//! * **FIFO configuration error** (RM0090 "FIFO", Table 48): enabling a
//!   FIFO-mode stream whose `FTH` does not hold a whole number of `MBURST`
//!   bursts (burst bytes = beats × `MSIZE`, at most the 16-byte FIFO) sets
//!   `FEIF` and leaves the stream disabled (`EN` reads 0).
//! * **FIFO overrun / underrun** (RM0090 "Error management"): a peripheral request that
//!   finds no room for its item(s) in a P2M FIFO, or not enough data in an
//!   M2P FIFO, sets `FEIF` (the request is kept and served once the memory
//!   port catches up). Not raised for memory-to-memory.
//! * **Transfer error**: an item that faults on the bus (an unmapped address,
//!   [`Peripheral::dma_transfer_error`]) sets `TEIF` and disables the stream
//!   (`EN` cleared by hardware).
//! * `FEIE` / `TEIE` / `HTIE` / `TCIE` pend the stream's own NVIC line.
//! * **Disable** (`EN` written 0 while running, RM0090 "DMA transfer suspension"): the stream
//!   finishes at the next slot — `EN` reads 1 until then — after flushing a
//!   P2M FIFO; `TCIF` is set and `NDTR` holds what was left.
//! * Circular mode (`CIRC`) and double-buffer mode (`DBM`, which forces `CIRC`)
//!   reload NDTR and the pointers at completion; DBM toggles `CT` and swaps
//!   the active memory pointer between SxM0AR and SxM1AR (§10.3.9).
//! * **Memory-to-memory is DMA2 only** (RM0090 §10.3.3): a DIR=10 enable on
//!   DMA1 never starts a transfer.
//!
//! ## Not modelled
//!
//! - `DMEIF` (direct-mode error) is storage only: the model's request
//!   handshake (each request latched until served) never loses a direct-mode
//!   item, which is the condition the flag reports.
//! - Bus timing inside a slot: a slot moves its whole burst / drain at once.
//! - `PFCTRL` (peripheral flow control, SDIO only): stored, NDTR still counts.

use crate::{
    CycleClock, DmaDirection, DmaRequest, DmaRoute, DmaUnitTransform, Peripheral,
    PeripheralTickResult, SimResult,
};
use std::any::Any;
use std::collections::VecDeque;

const NUM_STREAMS: usize = 8;
const STREAM_BASE: u64 = 0x10;
const STREAM_STRIDE: u64 = 0x18;
/// FIFO size in bytes (four words, RM0090 "FIFO").
const FIFO_BYTES: u32 = 16;

// SxCR bit fields (RM0090 §10.5.5, F401 SVD).
const CR_EN: u32 = 1 << 0;
const CR_TEIE: u32 = 1 << 2;
const CR_HTIE: u32 = 1 << 3;
const CR_TCIE: u32 = 1 << 4;
const CR_DIR_SHIFT: u32 = 6; // DIR[1:0]
const CR_CIRC: u32 = 1 << 8;
const CR_PINC: u32 = 1 << 9;
const CR_MINC: u32 = 1 << 10;
const CR_PSIZE_SHIFT: u32 = 11; // PSIZE[1:0]
const CR_MSIZE_SHIFT: u32 = 13; // MSIZE[1:0]
const CR_PINCOS: u32 = 1 << 15;
const CR_PL_SHIFT: u32 = 16; // PL[1:0]
const CR_DBM: u32 = 1 << 18;
const CR_CT: u32 = 1 << 19;
const CR_PBURST_SHIFT: u32 = 21; // PBURST[1:0]
const CR_MBURST_SHIFT: u32 = 23; // MBURST[1:0]
                                 // SxCR writable mask: bits [27:0] are implemented (CHSEL[27:25], MBURST[24:23],
                                 // PBURST[22:21], CT[19], DBM[18], PL[17:16], PINCOS[15], MSIZE[14:13],
                                 // PSIZE[12:11], MINC[10], PINC[9], CIRC[8], DIR[7:6], PFCTRL[5], TCIE[4],
                                 // HTIE[3], TEIE[2], DMEIE[1], EN[0]). Bits [31:28] reserved, read 0.
const CR_WRITABLE_MASK: u32 = 0x0FFF_FFFF;

// SxFCR (RM0090 §10.5.10, F401 SVD): FTH[1:0], DMDIS[2], FS[5:3] (RO), FEIE[7].
const FCR_FTH_MASK: u32 = 0x3;
const FCR_DMDIS: u32 = 1 << 2;
const FCR_FS_SHIFT: u32 = 3;
const FCR_FEIE: u32 = 1 << 7;
const FCR_WRITABLE_MASK: u32 = FCR_FEIE | FCR_DMDIS | FCR_FTH_MASK;

const DIR_P2M: u32 = 0b00;
const DIR_M2P: u32 = 0b01;
const DIR_M2M: u32 = 0b10;

// Compact internal per-stream flag bits (mapped to register geometry on read).
const F_FEIF: u8 = 1 << 0;
const F_DMEIF: u8 = 1 << 1;
const F_TEIF: u8 = 1 << 2;
const F_HTIF: u8 = 1 << 3;
const F_TCIF: u8 = 1 << 4;

/// The scheduler token of the controller's arbiter event.
const ARBITER_TOKEN: u32 = 0;

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

/// Beats of a `PBURST` / `MBURST` encoding: single, INCR4, INCR8, INCR16.
fn burst_beats(code: u32) -> u32 {
    [1, 4, 8, 16][(code & 3) as usize]
}

/// `SxFCR.FS` for a FIFO holding `level` bytes (RM0090 §10.5.10).
fn fifo_status(level: u32) -> u32 {
    match level {
        0 => 0b100,
        FIFO_BYTES => 0b101,
        l => l / 4,
    }
}

/// What a stream does when it wins a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Direct mode: one item from source to destination.
    Direct,
    /// FIFO mode: the peripheral port reads an item / burst into the FIFO.
    PeriphToFifo,
    /// FIFO mode: the memory port drains the FIFO (`flush`: all of it).
    FifoToMem { flush: bool },
    /// FIFO mode: the memory port refills the FIFO.
    MemToFifo,
    /// FIFO mode: the peripheral port writes an item / burst from the FIFO.
    FifoToPeriph,
    /// Nothing left to move: firmware cleared EN (stop), or NDTR reached 0
    /// (complete).
    End,
}

#[derive(Debug, Default, serde::Serialize)]
struct DmaStream {
    cr: u32,
    ndtr: u32,
    par: u32,
    m0ar: u32,
    m1ar: u32,
    /// SxFCR writable bits (FEIE | DMDIS | FTH); FS is computed on read.
    fcr: u32,
    /// Interrupt flags, compact encoding (see `F_*`).
    flags: u8,
    /// Enabled and started (EN=1 took effect).
    running: bool,
    /// Peripheral requests latched and not yet served.
    requests: u32,
    /// Firmware cleared EN while running: the stream stops (after a P2M
    /// flush) at its next slot, EN reading 1 until then.
    disabling: bool,
    /// The FIFO's bytes, oldest first.
    fifo: VecDeque<u8>,
    /// Bytes loaded into the FIFO whose data has not come back yet.
    inflight: u32,
    /// Memory-to-peripheral FIFO mode: bytes still to fetch from memory in
    /// this pass.
    mem_left: u32,
    /// An interrupt latched outside a slot (FEIF at enable or on a request,
    /// TEIF after a faulting item), pended at the next slot.
    irq_deferred: bool,
    /// Internal transfer pointers — silicon leaves SxPAR/SxM0AR/SxM1AR readable
    /// at their programmed base while a transfer advances internal counters.
    par_ptr: u32,
    mem_ptr: u32,
    /// NDTR snapshot at enable, for the half-transfer (HTIF) crossing and for
    /// circular / double-buffer reload.
    ndtr_initial: u32,
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
    fn priority(&self) -> u32 {
        (self.cr >> CR_PL_SHIFT) & 3
    }
    fn pbeats(&self) -> u32 {
        burst_beats(self.cr >> CR_PBURST_SHIFT)
    }
    fn mbeats(&self) -> u32 {
        burst_beats(self.cr >> CR_MBURST_SHIFT)
    }
    fn fifo_mode(&self) -> bool {
        self.fcr & FCR_DMDIS != 0
    }
    /// FIFO threshold in bytes: 1/4, 1/2, 3/4 or all of the 16-byte FIFO.
    fn threshold(&self) -> u32 {
        ((self.fcr & FCR_FTH_MASK) + 1) * 4
    }
    fn level(&self) -> u32 {
        self.fifo.len() as u32
    }
    /// Room left in the FIFO, counting bytes on their way in.
    fn room(&self) -> u32 {
        FIFO_BYTES - self.level() - self.inflight
    }
    /// Peripheral-port address step: PSIZE, or 4 with PINCOS.
    fn par_step(&self) -> u32 {
        if self.cr & CR_PINCOS != 0 {
            4
        } else {
            self.psize_width()
        }
    }
    /// Bytes the next peripheral-port access moves: a PBURST (capped by
    /// what NDTR has left) of PSIZE items.
    fn periph_bytes(&self) -> u32 {
        self.pbeats().min(self.ndtr) * self.psize_width()
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
    /// RM0090 Table 48: a FIFO-mode stream with a memory burst needs a
    /// threshold that holds a whole number of bursts, and a burst that fits
    /// the FIFO.
    fn fifo_config_ok(&self) -> bool {
        if (self.cr >> CR_MBURST_SHIFT) & 3 == 0 {
            return true;
        }
        let burst = self.mbeats() * self.msize_width();
        burst <= FIFO_BYTES && self.threshold() % burst == 0
    }
    fn mem_step(&mut self, width: u32) {
        if self.cr & CR_MINC != 0 {
            self.mem_ptr = self.mem_ptr.wrapping_add(width);
        }
    }
    fn par_advance(&mut self) {
        if self.cr & CR_PINC != 0 {
            self.par_ptr = self.par_ptr.wrapping_add(self.par_step());
        }
    }
    /// The next memory-to-FIFO fetch: an MBURST while a whole one is left,
    /// else one MSIZE beat (or the shorter tail).
    fn refill_chunk(&self) -> u32 {
        let burst = self.mbeats() * self.msize_width();
        if self.mbeats() > 1 && self.mem_left >= burst {
            burst
        } else {
            self.msize_width().min(self.mem_left)
        }
    }

    /// What this stream would do if it won the slot now.
    fn next_action(&self) -> Option<Action> {
        if !self.running {
            return None;
        }
        if !self.fifo_mode() {
            return if self.disabling {
                Some(Action::End)
            } else if self.requests > 0 && self.ndtr > 0 {
                Some(Action::Direct)
            } else {
                None
            };
        }
        if self.dir() == DIR_M2P {
            if self.disabling {
                return Some(Action::End);
            }
            if self.requests > 0 && self.ndtr > 0 && self.level() >= self.periph_bytes() {
                return Some(Action::FifoToPeriph);
            }
            if self.mem_left > 0
                && self.level() + self.inflight <= self.threshold()
                && self.room() >= self.refill_chunk()
            {
                return Some(Action::MemToFifo);
            }
            return None;
        }
        // P2M / M2M: the peripheral port fills, the memory port drains.
        if self.level() >= self.threshold() {
            return Some(Action::FifoToMem { flush: false });
        }
        if (self.ndtr == 0 || self.disabling) && self.inflight == 0 {
            if self.level() > 0 {
                return Some(Action::FifoToMem { flush: true });
            }
            // A stream enabled with NDTR = 0 has nothing to complete.
            if self.disabling || self.ndtr_initial > 0 {
                return Some(Action::End);
            }
        }
        let paced = self.dir() == DIR_M2M || self.requests > 0;
        if !self.disabling && paced && self.ndtr > 0 && self.room() >= self.periph_bytes() {
            return Some(Action::PeriphToFifo);
        }
        None
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
    /// Peripheral request lines, from the chip yaml (`config: { requests: {
    /// spi1_tx: [[3, 3], [5, 3]], ... } }`): `(request, stream 0..7, channel)`.
    /// A request drives a stream only while the stream is enabled with
    /// `SxCR.CHSEL` = that channel (RM0368 Tables 27/28).
    #[serde(skip)]
    routes: Vec<(String, u32, u32)>,
    /// The stream served by the most recent slot: the issuer of the items a
    /// FIFO load answers ([`Peripheral::dma_load_data`]) and a bus error
    /// reports ([`Peripheral::dma_transfer_error`]).
    #[serde(skip)]
    last_served: Option<usize>,
    /// Scheduler mode only: the arbiter event is live in the heap.
    #[serde(skip)]
    arbiter_live: bool,
    /// Bus-published cycle clock (walk-free machinery). `Some` once the bus
    /// registration choke attaches it; `None` keeps the model on the legacy walk.
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl StreamDma {
    pub fn new() -> Self {
        let mut d = Self::default();
        for s in d.streams.iter_mut() {
            s.fcr = 0x01; // SxFCR reset 0x21: FTH=01 (FS=100 is the empty FIFO).
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

    /// Wire peripheral request `request` (`spi1_tx`, ...) to `stream` (0..7)
    /// on channel `chsel` (`SxCR.CHSEL`).
    pub fn with_request_route(mut self, request: &str, stream: u32, chsel: u32) -> Self {
        self.routes.push((request.to_string(), stream, chsel));
        self
    }

    crate::cycle_clock::scheduler_mode!();

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
                    0x14 => st.fcr | (fifo_status(st.level()) << FCR_FS_SHIFT),
                    _ => {
                        crate::census_reg!("stm32f4_dma:StreamDma", reg, "read");
                        0
                    }
                }
            }
            _ => {
                crate::census_reg!("stm32f4_dma:StreamDma", offset, "read");
                0
            }
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
                    0x14 => self.streams[s].fcr = value & FCR_WRITABLE_MASK, // FS is RO
                    _ => {
                        crate::census_reg!("stm32f4_dma:StreamDma", reg, "write");
                    }
                }
            }
            _ => {
                crate::census_reg!("stm32f4_dma:StreamDma", offset, "write");
            }
        }
    }

    fn clear_flags(&mut self, base_stream: usize, value: u32) {
        for p in 0..4 {
            let clear = reg_bits_to_flags(value, p);
            self.streams[base_stream + p].flags &= !clear;
        }
    }

    fn write_cr(&mut self, s: usize, value: u32) {
        let is_dma2 = self.is_dma2;
        let st = &mut self.streams[s];
        let old_en = st.cr & CR_EN != 0;
        st.cr = value & CR_WRITABLE_MASK;
        let new_en = st.cr & CR_EN != 0;
        if !old_en && new_en {
            Self::start(st, is_dma2);
        } else if old_en && !new_en && st.running {
            // RM0090 "DMA transfer suspension": the stream stops after the
            // current item (and,
            // in P2M FIFO mode, after flushing the FIFO to memory); EN reads
            // 1 until it has. The next slot does that.
            st.cr |= CR_EN;
            st.disabling = true;
        }
    }

    /// `SxCR.EN` 0→1 (RM0090 §10.3.17: SxNDTR is written before EN).
    fn start(st: &mut DmaStream, is_dma2: bool) {
        let dir = st.dir();
        // RM0090 §10.3.3: memory-to-memory (DIR=10) is DMA2-only; a DIR=10
        // enable on DMA1 never starts.
        if dir == DIR_M2M && !is_dma2 {
            return;
        }
        // Hardware-forced fields once EN=1 (RM0090 §10.5.5 / §10.5.10).
        if dir == DIR_M2M {
            st.fcr |= FCR_DMDIS; // M2M runs through the FIFO
        }
        if st.cr & CR_DBM != 0 {
            st.cr |= CR_CIRC;
        }
        if !st.fifo_mode() {
            // Direct mode: MSIZE = PSIZE, no bursts, no PINCOS.
            let psize = (st.cr >> CR_PSIZE_SHIFT) & 3;
            st.cr &= !((3 << CR_MSIZE_SHIFT)
                | (3 << CR_MBURST_SHIFT)
                | (3 << CR_PBURST_SHIFT)
                | CR_PINCOS);
            st.cr |= psize << CR_MSIZE_SHIFT;
        } else if (st.cr >> CR_PBURST_SHIFT) & 3 != 0 {
            st.cr &= !CR_PINCOS;
        }
        if st.fifo_mode() && !st.fifo_config_ok() {
            // RM0090 "FIFO" / Table 48: FEIF, and the stream is disabled.
            st.flags |= F_FEIF;
            st.cr &= !CR_EN;
            st.irq_deferred |= st.fcr & FCR_FEIE != 0;
            return;
        }
        st.running = true;
        st.requests = 0;
        st.disabling = false;
        st.fifo.clear();
        st.inflight = 0;
        st.par_ptr = st.par;
        st.mem_ptr = st.active_mem_base();
        st.ndtr_initial = st.ndtr;
        st.mem_left = if dir == DIR_M2P && st.fifo_mode() {
            st.ndtr * st.psize_width()
        } else {
            0
        };
    }

    /// The stream the arbiter serves this slot, with what it does: the
    /// highest `SxCR.PL` among the streams with work, the lowest stream
    /// number between equals.
    fn arbiter_winner(&self) -> Option<(usize, Action)> {
        let mut best: Option<(usize, Action)> = None;
        for (s, st) in self.streams.iter().enumerate() {
            let Some(action) = st.next_action() else {
                continue;
            };
            if best.is_none_or(|(b, _)| st.priority() > self.streams[b].priority()) {
                best = Some((s, action));
            }
        }
        best
    }

    fn has_work(&self) -> bool {
        self.streams.iter().any(|st| st.irq_deferred) || self.arbiter_winner().is_some()
    }

    /// One arbitration slot. The ONE body both drive modes run: returns the
    /// items the winner issues and the streams whose interrupt it pends.
    fn run_slot(&mut self) -> (Vec<DmaRequest>, Vec<usize>) {
        let mut pends: Vec<usize> = Vec::new();
        for (s, st) in self.streams.iter_mut().enumerate() {
            if std::mem::take(&mut st.irq_deferred) {
                pends.push(s);
            }
        }
        let mut items = Vec::new();
        self.last_served = None;
        if let Some((s, action)) = self.arbiter_winner() {
            self.last_served = Some(s);
            if self.serve(s, action, &mut items) && !pends.contains(&s) {
                pends.push(s);
            }
        }
        (items, pends)
    }

    /// Stream `s` performs `action`, appending its bus items. Returns whether
    /// its interrupt pends.
    fn serve(&mut self, s: usize, action: Action, items: &mut Vec<DmaRequest>) -> bool {
        let st = &mut self.streams[s];
        let psize = st.psize_width();
        let msize = st.msize_width();
        let mut irq = false;
        match action {
            Action::Direct => {
                // One item, PSIZE wide on both sides (MSIZE was forced).
                let (src, dst) = match st.dir() {
                    DIR_M2P => (st.mem_ptr, st.par_ptr),
                    _ => (st.par_ptr, st.mem_ptr),
                };
                st.ndtr -= 1;
                items.push(copy_item(src, dst, psize, Some(st.ndtr)));
                st.par_advance();
                st.mem_step(psize);
                st.requests = st.requests.saturating_sub(1);
                irq |= Self::half_transfer(st);
                if st.ndtr == 0 {
                    irq |= Self::complete(st);
                }
            }
            Action::PeriphToFifo => {
                // One request (one slot for M2M) moves a whole PBURST.
                let m2m = st.dir() == DIR_M2M;
                for _ in 0..st.pbeats().min(st.ndtr) {
                    st.ndtr -= 1;
                    let remaining = (!m2m).then_some(st.ndtr);
                    items.push(load_item(st.par_ptr, psize, remaining));
                    st.inflight += psize;
                    st.par_advance();
                }
                if !m2m {
                    st.requests = st.requests.saturating_sub(1);
                }
                irq |= Self::half_transfer(st);
            }
            Action::FifoToMem { flush } => {
                let mut n = if flush { st.level() } else { st.threshold() };
                while n > 0 {
                    // MSIZE beats (an MBURST is a run of them); a flush that
                    // ends short of a whole MSIZE item finishes byte-wise.
                    let w = if n >= msize { msize } else { 1 };
                    let v = pop_le(&mut st.fifo, w);
                    items.push(store_item(st.mem_ptr, w, v, None));
                    st.mem_step(w);
                    n -= w;
                }
                if st.fifo.is_empty() && st.inflight == 0 {
                    if st.disabling {
                        irq |= Self::stop(st);
                    } else if st.ndtr == 0 {
                        irq |= Self::complete(st);
                    }
                }
            }
            Action::MemToFifo => {
                // Refill: MBURSTs while a whole one fits and is left, else
                // single MSIZE beats; a tail shorter than MSIZE byte-wise.
                while st.mem_left > 0 {
                    let chunk = st.refill_chunk();
                    if st.room() < chunk {
                        break;
                    }
                    let w = if chunk >= msize { msize } else { 1 };
                    for _ in 0..chunk / w {
                        items.push(load_item(st.mem_ptr, w, None));
                        st.inflight += w;
                        st.mem_left -= w;
                        st.mem_step(w);
                    }
                }
            }
            Action::FifoToPeriph => {
                for _ in 0..st.pbeats().min(st.ndtr) {
                    let v = pop_le(&mut st.fifo, psize);
                    st.ndtr -= 1;
                    items.push(store_item(st.par_ptr, psize, v, Some(st.ndtr)));
                    st.par_advance();
                }
                st.requests = st.requests.saturating_sub(1);
                irq |= Self::half_transfer(st);
                if st.ndtr == 0 {
                    irq |= Self::complete(st);
                }
            }
            Action::End if st.disabling => irq |= Self::stop(st),
            Action::End => irq |= Self::complete(st),
        }
        irq
    }

    /// HTIF at the half-transfer crossing (latched once until cleared).
    fn half_transfer(st: &mut DmaStream) -> bool {
        if st.ndtr_initial >= 2 && st.ndtr <= st.ndtr_initial / 2 && st.flags & F_HTIF == 0 {
            st.flags |= F_HTIF;
            return st.cr & CR_HTIE != 0;
        }
        false
    }

    /// NDTR reached 0 and the FIFO is empty: TCIF, then reload (circular /
    /// double-buffer) or stop with EN cleared (RM0090 "Circular mode",
    /// "Double-buffer mode").
    fn complete(st: &mut DmaStream) -> bool {
        st.flags |= F_TCIF;
        let irq = st.cr & CR_TCIE != 0;
        if st.cr & CR_CIRC != 0 {
            if st.cr & CR_DBM != 0 {
                st.cr ^= CR_CT;
            }
            st.ndtr = st.ndtr_initial;
            st.par_ptr = st.par;
            st.mem_ptr = st.active_mem_base();
            if st.dir() == DIR_M2P && st.fifo_mode() {
                st.mem_left = st.ndtr * st.psize_width();
            }
        } else {
            st.running = false;
            st.requests = 0;
            st.cr &= !CR_EN;
        }
        irq
    }

    /// Firmware disabled the stream and it is now stopped: EN reads 0,
    /// TCIF reports the end, NDTR keeps what was left (RM0090 "DMA
    /// transfer suspension").
    fn stop(st: &mut DmaStream) -> bool {
        st.running = false;
        st.disabling = false;
        st.requests = 0;
        st.fifo.clear();
        st.mem_left = 0;
        st.cr &= !CR_EN;
        st.flags |= F_TCIF;
        st.cr & CR_TCIE != 0
    }

    fn tick_streams_once(&mut self) -> PeripheralTickResult {
        let (items, pends) = self.run_slot();
        let mut explicit_irqs: Option<Vec<u32>> = None;
        let mut irq = false;
        for s in pends {
            self.pend_stream(s, &mut explicit_irqs, &mut irq);
        }
        PeripheralTickResult {
            irq,
            // Tick-cost normalization (mirrors `Dma1`): a DMA is a bus master,
            // it does not steal CPU cycles — charge zero so the walk-on
            // reference and the scheduler path agree cycle-for-cycle.
            cycles: 0,
            dma_requests: (!items.is_empty()).then_some(items),
            explicit_irqs,
            ..Default::default()
        }
    }

    /// Route a stream's interrupt: its own NVIC vector when known, else the
    /// block's single configured line.
    fn pend_stream(&self, s: usize, explicit: &mut Option<Vec<u32>>, own: &mut bool) {
        match self.stream_irqs.get(s) {
            Some(&line) => explicit.get_or_insert_with(Vec::new).push(line),
            None => *own = true,
        }
    }
}

/// A direct-mode copy of one `width`-byte item.
fn copy_item(src: u32, dst: u32, width: u32, remaining: Option<u32>) -> DmaRequest {
    DmaRequest {
        src_addr: src.into(),
        addr: dst.into(),
        val: 0,
        direction: DmaDirection::Copy,
        transform: Some(DmaUnitTransform {
            src_width: width as u8,
            dst_width: width as u8,
            ..Default::default()
        }),
        remaining,
    }
}

/// A `width`-byte read into the FIFO.
fn load_item(src: u32, width: u32, remaining: Option<u32>) -> DmaRequest {
    DmaRequest {
        src_addr: src.into(),
        addr: src.into(),
        val: 0,
        direction: DmaDirection::Load,
        transform: Some(DmaUnitTransform {
            src_width: width as u8,
            dst_width: width as u8,
            ..Default::default()
        }),
        remaining,
    }
}

/// A `width`-byte write out of the FIFO.
fn store_item(dst: u32, width: u32, value: u32, remaining: Option<u32>) -> DmaRequest {
    DmaRequest {
        src_addr: dst.into(),
        addr: dst.into(),
        val: 0,
        direction: DmaDirection::Store(value),
        transform: Some(DmaUnitTransform {
            src_width: width as u8,
            dst_width: width as u8,
            ..Default::default()
        }),
        remaining,
    }
}

/// Pop `width` FIFO bytes as a little-endian value (missing bytes read 0).
fn pop_le(fifo: &mut VecDeque<u8>, width: u32) -> u32 {
    (0..width).fold(0, |v, k| {
        v | (u32::from(fifo.pop_front().unwrap_or(0)) << (8 * k))
    })
}

impl Peripheral for StreamDma {
    /// A word store is ONE register write: split into bytes, a CCR value
    /// with EN in byte 0 and (say) MEM2MEM in byte 1 would enable the
    /// channel before its mode bits are written.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset & 2) * 8;
        let v = (self.read_reg(reg) & !(0xFFFF << shift)) | (u32::from(value) << shift);
        self.write_reg(reg, v);
        Ok(())
    }

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

    fn dma_route_request(&self, request: &str, out: &mut Vec<DmaRoute>) {
        for (name, stream, chsel) in &self.routes {
            let Some(st) = self.streams.get(*stream as usize) else {
                continue;
            };
            if name == request && st.cr & CR_EN != 0 && (st.cr >> 25) & 0x7 == *chsel {
                out.push(DmaRoute {
                    controller: None,
                    channel: stream + 1,
                });
            }
        }
    }

    fn dma_request(&mut self, request_id: u32) {
        // request_id maps 1..8 to stream 0..7.
        let s = request_id.saturating_sub(1) as usize;
        let Some(st) = self.streams.get_mut(s) else {
            return;
        };
        if st.cr & CR_EN == 0 || !st.running || st.disabling {
            return;
        }
        st.requests = st.requests.saturating_add(1);
        // RM0090 "Error management": a FIFO overrun (P2M: no room for the item) or
        // underrun (M2P: not enough data for it) sets FEIF. The request is
        // kept: it is served once the memory port catches up.
        let short = match st.dir() {
            DIR_P2M if st.fifo_mode() => st.room() < st.periph_bytes(),
            DIR_M2P if st.fifo_mode() => st.level() + st.inflight < st.periph_bytes(),
            _ => false,
        };
        if short && st.flags & F_FEIF == 0 {
            st.flags |= F_FEIF;
            st.irq_deferred |= st.fcr & FCR_FEIE != 0;
        }
    }

    fn dma_load_data(&mut self, item: &DmaRequest, value: u32) {
        let Some(s) = self.last_served else {
            return;
        };
        let st = &mut self.streams[s];
        let width = u32::from(item.transform.map_or(1, |t| t.src_width));
        st.inflight = st.inflight.saturating_sub(width);
        if !st.running {
            return; // stopped meanwhile (a transfer error): the data is dropped
        }
        for k in 0..width {
            st.fifo.push_back((value >> (8 * k)) as u8);
        }
    }

    fn dma_transfer_error(&mut self, _item: &DmaRequest) {
        let Some(s) = self.last_served else {
            return;
        };
        let st = &mut self.streams[s];
        if !st.running && st.flags & F_TEIF != 0 {
            return; // a later item of the same faulting slot
        }
        // TEIF, and the stream is disabled by hardware (RM0090 "Error
        // management").
        st.flags |= F_TEIF;
        st.running = false;
        st.disabling = false;
        st.requests = 0;
        st.fifo.clear();
        st.inflight = 0;
        st.mem_left = 0;
        st.cr &= !CR_EN;
        st.irq_deferred |= st.cr & CR_TEIE != 0;
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
        // Every readable register mutates only at a slot (or a write); nothing
        // to replay on the write path.
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        if self.has_work() && !self.arbiter_live {
            self.arbiter_live = true;
            return vec![(0, ARBITER_TOKEN)];
        }
        Vec::new()
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let _ = sched;
        if !self.scheduler_mode() || event_token != ARBITER_TOKEN {
            return crate::sched::EventResult::default();
        }
        let res = self.tick_streams_once();
        // Another slot next cycle while a stream still has work. Work that
        // appears later (a FIFO load's data, a request) re-arms the chain.
        let reschedule = self.has_work();
        self.arbiter_live = reschedule;
        crate::sched::EventResult {
            raise_own_irq: res.irq,
            explicit_irqs: res.explicit_irqs.unwrap_or_default(),
            reschedule_delay: reschedule.then_some(1),
            dma_requests: res.dma_requests.unwrap_or_default(),
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
    use std::collections::BTreeMap;

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
    fn m1ar(s: u64) -> u64 {
        cr(s) + 0x10
    }
    fn fcr(s: u64) -> u64 {
        cr(s) + 0x14
    }

    const M2M: u32 = DIR_M2M << CR_DIR_SHIFT;
    const M2P: u32 = DIR_M2P << CR_DIR_SHIFT;
    fn psize(w: u32) -> u32 {
        (w.trailing_zeros()) << CR_PSIZE_SHIFT
    }
    fn msize(w: u32) -> u32 {
        (w.trailing_zeros()) << CR_MSIZE_SHIFT
    }
    fn mburst(beats: u32) -> u32 {
        [1, 4, 8, 16].iter().position(|&b| b == beats).unwrap() as u32 * (1 << CR_MBURST_SHIFT)
    }
    fn pburst(beats: u32) -> u32 {
        [1, 4, 8, 16].iter().position(|&b| b == beats).unwrap() as u32 * (1 << CR_PBURST_SHIFT)
    }
    fn pl(p: u32) -> u32 {
        p << CR_PL_SHIFT
    }

    /// The model's bus: RAM as a byte map, a receive data register that pops
    /// a queue, a transmit data register that logs, and an unmapped hole.
    #[derive(Default)]
    struct Mem {
        ram: BTreeMap<u64, u8>,
        rx: VecDeque<u8>,
        /// `(value, width)` of each transmit-register write.
        tx: Vec<(u32, u8)>,
        /// `(address, width)` of each memory write, in order.
        writes: Vec<(u64, u8)>,
    }
    const RX_DR: u64 = 0x4001_300C;
    const TX_DR: u64 = 0x4001_3010;
    const HOLE: u64 = 0x5000_0000;

    impl Mem {
        fn read(&mut self, addr: u64, w: u8) -> Option<u32> {
            if (HOLE..HOLE + 0x1000).contains(&addr) {
                return None;
            }
            if addr == RX_DR {
                let mut v = 0;
                for k in 0..w {
                    v |= u32::from(self.rx.pop_front().unwrap_or(0)) << (8 * k);
                }
                return Some(v);
            }
            Some((0..u64::from(w)).fold(0, |v, k| {
                v | (u32::from(*self.ram.get(&(addr + k)).unwrap_or(&0)) << (8 * k))
            }))
        }
        fn write(&mut self, addr: u64, w: u8, v: u32) -> bool {
            if (HOLE..HOLE + 0x1000).contains(&addr) {
                return false;
            }
            if addr == TX_DR {
                self.tx.push((v, w));
                return true;
            }
            self.writes.push((addr, w));
            for k in 0..u64::from(w) {
                self.ram.insert(addr + k, (v >> (8 * k)) as u8);
            }
            true
        }
        fn fill(&mut self, addr: u64, bytes: &[u8]) {
            for (i, b) in bytes.iter().enumerate() {
                self.ram.insert(addr + i as u64, *b);
            }
        }
        fn get(&self, addr: u64, n: usize) -> Vec<u8> {
            (0..n as u64)
                .map(|k| *self.ram.get(&(addr + k)).unwrap_or(&0))
                .collect()
        }
    }

    /// Run one slot's items the way the bus does, answering loads and
    /// reporting faults to the DMA.
    fn exec(dma: &mut StreamDma, mem: &mut Mem, items: &[DmaRequest]) {
        for r in items {
            let t = r.transform.unwrap_or_default();
            match r.direction {
                DmaDirection::Copy => {
                    let ok = match mem.read(r.src_addr, t.src_width) {
                        Some(v) => mem.write(r.addr, t.dst_width, v),
                        None => false,
                    };
                    if !ok {
                        dma.dma_transfer_error(r);
                    }
                }
                DmaDirection::Load => match mem.read(r.src_addr, t.src_width) {
                    Some(v) => dma.dma_load_data(r, v),
                    None => dma.dma_transfer_error(r),
                },
                DmaDirection::Store(v) => {
                    if !mem.write(r.addr, t.dst_width, v) {
                        dma.dma_transfer_error(r);
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    fn step(dma: &mut StreamDma, mem: &mut Mem) -> PeripheralTickResult {
        let res = dma.tick();
        let items = res.dma_requests.clone().unwrap_or_default();
        exec(dma, mem, &items);
        res
    }

    fn steps(dma: &mut StreamDma, mem: &mut Mem, n: usize) {
        for _ in 0..n {
            step(dma, mem);
        }
    }

    fn flag(dma: &StreamDma, s: usize, f: u8) -> bool {
        let reg = if s < 4 { 0x00 } else { 0x04 };
        dma.read_reg(reg) & flags_to_reg_bits(f, s % 4) != 0
    }

    fn fs(dma: &StreamDma, s: u64) -> u32 {
        (dma.read_reg(fcr(s)) >> FCR_FS_SHIFT) & 7
    }

    fn routes(d: &StreamDma, request: &str) -> Vec<u32> {
        let mut out = Vec::new();
        d.dma_route_request(request, &mut out);
        out.iter().map(|r| r.channel).collect()
    }

    /// A request reaches a stream only while it is enabled on the request's
    /// channel (SxCR.CHSEL), and a peripheral stream moves nothing until a
    /// request comes: one copy item per request, carrying the NDTR left.
    #[test]
    fn requests_route_by_chsel_and_pace_a_peripheral_stream() {
        let mut d = StreamDma::new()
            .as_dma2()
            .with_request_route("spi1_tx", 3, 3)
            .with_request_route("spi1_tx", 5, 3);
        assert!(routes(&d, "spi1_tx").is_empty(), "no stream enabled");
        d.write_reg(ndtr(3), 2);
        d.write_reg(par(3), 0x4001_300C);
        d.write_reg(m0ar(3), 0x2000_0000);
        // EN | DIR = memory-to-peripheral | MINC, CHSEL = 2: the wrong channel.
        d.write_reg(cr(3), CR_EN | M2P | CR_MINC | (2 << 25));
        assert!(routes(&d, "spi1_tx").is_empty(), "CHSEL selects channel 2");
        d.write_reg(cr(3), 0);
        d.tick(); // the disable completes
        d.write_reg(cr(3), CR_EN | M2P | CR_MINC | (3 << 25));
        assert_eq!(routes(&d, "spi1_tx"), vec![4], "stream 3 is request id 4");
        assert!(d.tick().dma_requests.is_none(), "waits for its request");
        d.dma_request(4);
        let r = d.tick().dma_requests.unwrap();
        assert_eq!(
            (r[0].src_addr, r[0].addr, r[0].direction, r[0].remaining),
            (0x2000_0000, 0x4001_300C, DmaDirection::Copy, Some(1))
        );
        assert!(d.tick().dma_requests.is_none(), "one item per request");
        d.dma_request(4);
        let r = d.tick().dma_requests.unwrap();
        assert_eq!((r[0].src_addr, r[0].remaining), (0x2000_0001, Some(0)));
        assert_eq!(d.read_reg(cr(3)) & CR_EN, 0, "EN clears at the end");
    }

    #[test]
    fn fcr_resets_to_0x21_and_fs_is_read_only() {
        let mut dma = StreamDma::new();
        for s in 0..NUM_STREAMS as u64 {
            assert_eq!(dma.read_reg(fcr(s)), 0x21, "stream {s} SxFCR reset");
        }
        dma.write_reg(fcr(2), 0xFFFF_FFFF);
        // FEIE | FS = 100 (empty) | DMDIS | FTH = 11.
        assert_eq!(dma.read_reg(fcr(2)), 0xA7);
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

    /// RM0090 §10.5.5: in direct mode, enabling the stream forces MSIZE to
    /// PSIZE and the bursts (and PINCOS) to 0; memory-to-memory forces DMDIS.
    #[test]
    fn enabling_forces_direct_mode_fields_and_m2m_fifo_mode() {
        let mut dma = StreamDma::new().as_dma2();
        dma.write_reg(ndtr(1), 4);
        let v = CR_EN | psize(2) | msize(4) | mburst(4) | pburst(4) | CR_PINCOS | CR_PINC;
        dma.write_reg(cr(1), v);
        let got = dma.read_reg(cr(1));
        assert_eq!(got & (3 << CR_MSIZE_SHIFT), msize(2), "MSIZE = PSIZE");
        assert_eq!(got & ((3 << CR_MBURST_SHIFT) | (3 << CR_PBURST_SHIFT)), 0);
        assert_eq!(got & CR_PINCOS, 0);
        dma.write_reg(ndtr(2), 4);
        dma.write_reg(cr(2), CR_EN | M2M);
        assert_ne!(dma.read_reg(fcr(2)) & FCR_DMDIS, 0, "M2M forces DMDIS");
    }

    #[test]
    fn m2m_is_dma2_only() {
        // DIR=10 (mem-to-mem) on a DMA1 instance must not start.
        let mut mem = Mem::default();
        mem.fill(0x2000_0000, &[1, 2, 3, 4]);
        let mut dma1 = StreamDma::new();
        dma1.write_reg(ndtr(0), 4);
        dma1.write_reg(par(0), 0x2000_0000);
        dma1.write_reg(m0ar(0), 0x2000_0100);
        dma1.write_reg(cr(0), CR_EN | M2M | CR_PINC | CR_MINC);
        steps(&mut dma1, &mut mem, 12);
        // Never started: NDTR unchanged, no TCIF.
        assert_eq!(dma1.read_reg(ndtr(0)), 4, "DMA1 M2M must not start");
        assert!(!flag(&dma1, 0, F_TCIF));

        // Same program on DMA2 completes.
        let mut dma2 = StreamDma::new().as_dma2();
        dma2.write_reg(ndtr(0), 4);
        dma2.write_reg(par(0), 0x2000_0000);
        dma2.write_reg(m0ar(0), 0x2000_0100);
        dma2.write_reg(cr(0), CR_EN | M2M | CR_PINC | CR_MINC);
        steps(&mut dma2, &mut mem, 12);
        assert_eq!(dma2.read_reg(ndtr(0)), 0, "DMA2 M2M drains");
        assert!(flag(&dma2, 0, F_TCIF));
        assert_eq!(mem.get(0x2000_0100, 4), vec![1, 2, 3, 4]);
    }

    /// Memory-to-memory runs through the FIFO: byte items in at the
    /// peripheral port, word bursts (MBURST INCR4, FTH full) out at the memory
    /// port, byte-exact, NDTR counting PSIZE items.
    #[test]
    fn m2m_packs_bytes_into_word_bursts_at_the_threshold() {
        let mut mem = Mem::default();
        let src: Vec<u8> = (0..32).map(|i| 0xA0 ^ i).collect();
        mem.fill(0x2000_0000, &src);
        let mut dma = StreamDma::new().as_dma2();
        dma.write_reg(ndtr(0), 32);
        dma.write_reg(par(0), 0x2000_0000);
        dma.write_reg(m0ar(0), 0x2000_0100);
        dma.write_reg(fcr(0), FCR_DMDIS | 3); // FTH = full
        let v = CR_EN | M2M | CR_PINC | CR_MINC | psize(1) | msize(4) | mburst(4) | CR_TCIE;
        dma.write_reg(cr(0), v);
        assert_eq!(dma.read_reg(cr(0)) & CR_EN, CR_EN);
        steps(&mut dma, &mut mem, 15);
        assert!(mem.writes.is_empty(), "nothing reaches memory below FTH");
        assert_eq!(fs(&dma, 0), 0b011, "3/4 <= level < full");
        step(&mut dma, &mut mem); // the 16th byte
        assert_eq!(fs(&dma, 0), 0b101, "full");
        assert_eq!(dma.read_reg(ndtr(0)), 16);
        step(&mut dma, &mut mem); // one INCR4 burst of words
        assert_eq!(
            mem.writes,
            (0..4).map(|k| (0x2000_0100 + 4 * k, 4)).collect::<Vec<_>>()
        );
        assert_eq!(fs(&dma, 0), 0b100, "empty");
        let mut irq = false;
        for _ in 0..20 {
            irq |= step(&mut dma, &mut mem).irq;
        }
        assert!(irq, "TCIE pends the block's line (no per-stream vectors)");
        assert_eq!(mem.get(0x2000_0100, 32), src);
        assert!(flag(&dma, 0, F_TCIF));
        assert_eq!(dma.read_reg(cr(0)) & CR_EN, 0);
        assert!(mem.writes.iter().all(|&(_, w)| w == 4), "word beats only");
    }

    /// P2M FIFO mode: requests fill the FIFO from the peripheral (PSIZE byte)
    /// and the memory port writes half-words only when FTH (1/2) is reached.
    #[test]
    fn p2m_fills_to_the_threshold_then_drains_half_words() {
        let mut mem = Mem::default();
        mem.rx.extend(1..=12u8);
        let mut dma = StreamDma::new();
        dma.write_reg(ndtr(2), 12);
        dma.write_reg(par(2), RX_DR as u32);
        dma.write_reg(m0ar(2), 0x2000_0200);
        dma.write_reg(fcr(2), FCR_DMDIS | 1); // FTH = 1/2: 8 bytes
        dma.write_reg(cr(2), CR_EN | CR_MINC | psize(1) | msize(2));
        for k in 1..=7 {
            dma.dma_request(3);
            step(&mut dma, &mut mem);
            assert_eq!(dma.read_reg(ndtr(2)), 12 - k);
        }
        assert!(mem.writes.is_empty(), "7 bytes buffered");
        assert_eq!(fs(&dma, 2), 0b001, "1/4 <= level < 1/2");
        dma.dma_request(3);
        step(&mut dma, &mut mem); // the 8th byte
        step(&mut dma, &mut mem); // the drain
        assert_eq!(
            mem.writes,
            (0..4).map(|k| (0x2000_0200 + 2 * k, 2)).collect::<Vec<_>>()
        );
        assert_eq!(mem.get(0x2000_0200, 8), (1..=8).collect::<Vec<u8>>());
        for _ in 0..4 {
            dma.dma_request(3);
            step(&mut dma, &mut mem);
        }
        // NDTR = 0 with 4 bytes below FTH: the end flushes them.
        step(&mut dma, &mut mem);
        assert_eq!(mem.get(0x2000_0200, 12), (1..=12).collect::<Vec<u8>>());
        assert!(flag(&dma, 2, F_TCIF));
        assert_eq!(dma.read_reg(cr(2)) & CR_EN, 0);
    }

    /// PBURST INCR4: ONE request moves four PSIZE items (NDTR -4).
    #[test]
    fn a_peripheral_burst_moves_four_items_per_request() {
        let mut mem = Mem::default();
        mem.rx
            .extend([0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        let mut dma = StreamDma::new();
        dma.write_reg(ndtr(0), 8);
        dma.write_reg(par(0), RX_DR as u32);
        dma.write_reg(m0ar(0), 0x2000_0300);
        dma.write_reg(fcr(0), FCR_DMDIS); // FTH = 1/4: one word
        dma.write_reg(cr(0), CR_EN | CR_MINC | psize(1) | msize(4) | pburst(4));
        dma.dma_request(1);
        let items = step(&mut dma, &mut mem).dma_requests.unwrap();
        assert_eq!(items.len(), 4, "one request, a 4-beat burst");
        assert_eq!(
            items.iter().map(|r| r.remaining).collect::<Vec<_>>(),
            vec![Some(7), Some(6), Some(5), Some(4)]
        );
        assert_eq!(dma.read_reg(ndtr(0)), 4);
        step(&mut dma, &mut mem); // drain one word
        assert_eq!(mem.writes, vec![(0x2000_0300, 4)]);
        assert!(dma.tick().dma_requests.is_none(), "waits for a request");
        dma.dma_request(1);
        steps(&mut dma, &mut mem, 3);
        assert_eq!(
            mem.get(0x2000_0300, 8),
            vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]
        );
        assert!(flag(&dma, 0, F_TCIF));
    }

    /// M2P FIFO mode: the FIFO is prefetched from memory at enable (one word
    /// burst), and each request unpacks one byte to the peripheral, LSB first.
    #[test]
    fn m2p_prefetches_word_bursts_and_unpacks_bytes() {
        let mut mem = Mem::default();
        let src: Vec<u8> = (0..20).map(|i| 0x30 + i).collect();
        mem.fill(0x2000_0400, &src);
        let mut dma = StreamDma::new();
        dma.write_reg(ndtr(5), 20);
        dma.write_reg(par(5), TX_DR as u32);
        dma.write_reg(m0ar(5), 0x2000_0400);
        dma.write_reg(fcr(5), FCR_DMDIS | 3); // FTH = full
        dma.write_reg(
            cr(5),
            CR_EN | M2P | CR_MINC | psize(1) | msize(4) | mburst(4),
        );
        let items = step(&mut dma, &mut mem).dma_requests.unwrap();
        assert_eq!(items.len(), 4, "one INCR4 word burst fills the FIFO");
        assert!(items.iter().all(|r| r.direction == DmaDirection::Load));
        assert_eq!(fs(&dma, 5), 0b101, "full before any request");
        for _ in 0..20 {
            dma.dma_request(6);
            steps(&mut dma, &mut mem, 3);
        }
        assert_eq!(
            mem.tx,
            src.iter().map(|&b| (u32::from(b), 1)).collect::<Vec<_>>()
        );
        assert!(flag(&dma, 5, F_TCIF));
        assert_eq!(dma.read_reg(cr(5)) & CR_EN, 0);
        assert!(!flag(&dma, 5, F_FEIF), "never ran dry");
    }

    /// RM0090 Table 48: the FIFO threshold must hold a whole number of
    /// memory bursts. A forbidden combination sets FEIF at enable and the
    /// stream stays disabled.
    #[test]
    fn forbidden_fth_mburst_msize_combinations_raise_feif() {
        // (MSIZE, MBURST beats) -> FTH quarters (1..4) that are forbidden.
        let forbidden: [((u32, u32), &[u32]); 9] = [
            ((1, 4), &[]),
            ((1, 8), &[1, 3]),
            ((1, 16), &[1, 2, 3]),
            ((2, 4), &[1, 3]),
            ((2, 8), &[1, 2, 3]),
            ((2, 16), &[1, 2, 3, 4]),
            ((4, 4), &[1, 2, 3]),
            ((4, 8), &[1, 2, 3, 4]),
            ((4, 16), &[1, 2, 3, 4]),
        ];
        for ((ms, beats), bad) in forbidden {
            for q in 1..=4u32 {
                let mut dma = StreamDma::new().with_stream_irqs(vec![11; 8]);
                dma.write_reg(ndtr(1), 16);
                dma.write_reg(fcr(1), FCR_FEIE | FCR_DMDIS | (q - 1));
                dma.write_reg(cr(1), CR_EN | psize(1) | msize(ms) | mburst(beats));
                let is_bad = bad.contains(&q);
                assert_eq!(
                    flag(&dma, 1, F_FEIF),
                    is_bad,
                    "MSIZE {ms} INCR{beats} FTH {q}/4"
                );
                assert_eq!(dma.read_reg(cr(1)) & CR_EN == 0, is_bad);
                let res = dma.tick();
                assert_eq!(res.explicit_irqs.is_some(), is_bad, "FEIE pends");
            }
        }
        // Single memory beats: any threshold.
        let mut dma = StreamDma::new();
        dma.write_reg(fcr(1), FCR_DMDIS);
        dma.write_reg(cr(1), CR_EN | msize(4));
        assert!(!flag(&dma, 1, F_FEIF));
    }

    /// FEIF on a FIFO overrun (P2M, FIFO full when a request comes) and
    /// underrun (M2P, nothing buffered when a request comes); the request is
    /// still served.
    #[test]
    fn fifo_overrun_and_underrun_set_feif() {
        let mut mem = Mem::default();
        mem.rx.extend(0..17u8);
        let mut dma = StreamDma::new();
        dma.write_reg(ndtr(0), 17);
        dma.write_reg(par(0), RX_DR as u32);
        dma.write_reg(m0ar(0), 0x2000_0000);
        dma.write_reg(fcr(0), FCR_DMDIS | 3);
        dma.write_reg(cr(0), CR_EN | CR_MINC);
        for _ in 0..16 {
            dma.dma_request(1);
            step(&mut dma, &mut mem);
        }
        assert!(!flag(&dma, 0, F_FEIF));
        dma.dma_request(1); // the FIFO is full: overrun
        assert!(flag(&dma, 0, F_FEIF));
        steps(&mut dma, &mut mem, 4);
        assert_eq!(mem.get(0x2000_0000, 17), (0..17).collect::<Vec<u8>>());

        let mut dma = StreamDma::new();
        dma.write_reg(ndtr(4), 4);
        dma.write_reg(par(4), TX_DR as u32);
        dma.write_reg(m0ar(4), 0x2000_0000);
        dma.write_reg(fcr(4), FCR_DMDIS | 3);
        dma.write_reg(cr(4), CR_EN | M2P | CR_MINC);
        dma.dma_request(5); // before the prefetch: underrun
        assert!(flag(&dma, 4, F_FEIF));
        steps(&mut dma, &mut mem, 3);
        assert_eq!(mem.tx.len(), 1, "served once data arrived");
    }

    /// An item that faults on the bus: TEIF, EN cleared by hardware, TEIE
    /// pends the stream's line on the next slot.
    #[test]
    fn a_bus_error_sets_teif_and_disables_the_stream() {
        let mut mem = Mem::default();
        let mut dma = StreamDma::new()
            .as_dma2()
            .with_stream_irqs(vec![56, 57, 58, 59, 60, 68, 69, 70]);
        dma.write_reg(ndtr(6), 8);
        dma.write_reg(par(6), HOLE as u32);
        dma.write_reg(m0ar(6), 0x2000_0000);
        dma.write_reg(cr(6), CR_EN | M2M | CR_PINC | CR_MINC | CR_TEIE);
        step(&mut dma, &mut mem);
        assert!(flag(&dma, 6, F_TEIF));
        assert_eq!(dma.read_reg(cr(6)) & CR_EN, 0, "disabled by hardware");
        assert_eq!(
            dma.tick().explicit_irqs.as_deref(),
            Some(&[69u32][..]),
            "TEIE pends DMA2_Stream6"
        );
        steps(&mut dma, &mut mem, 4);
        assert!(mem.writes.is_empty());
        // Direct mode, a faulting destination.
        let mut dma = StreamDma::new();
        dma.write_reg(ndtr(1), 2);
        dma.write_reg(par(1), RX_DR as u32);
        dma.write_reg(m0ar(1), HOLE as u32);
        dma.write_reg(cr(1), CR_EN | CR_MINC);
        dma.dma_request(2);
        step(&mut dma, &mut mem);
        assert!(flag(&dma, 1, F_TEIF));
        assert_eq!(dma.read_reg(cr(1)) & CR_EN, 0);
    }

    /// RM0090 "DMA transfer suspension": clearing EN mid-transfer stops the stream at the
    /// next slot (EN reads 1 until then), flushing a P2M FIFO to memory
    /// first; TCIF reports the end and NDTR keeps what was left.
    #[test]
    fn disabling_flushes_the_p2m_fifo_and_sets_tcif() {
        let mut mem = Mem::default();
        mem.rx.extend([9, 8, 7, 6, 5]);
        let mut dma = StreamDma::new();
        dma.write_reg(ndtr(3), 10);
        dma.write_reg(par(3), RX_DR as u32);
        dma.write_reg(m0ar(3), 0x2000_0000);
        dma.write_reg(fcr(3), FCR_DMDIS | 3);
        dma.write_reg(cr(3), CR_EN | CR_MINC | msize(4));
        for _ in 0..5 {
            dma.dma_request(4);
            step(&mut dma, &mut mem);
        }
        assert!(mem.writes.is_empty());
        dma.write_reg(cr(3), dma.read_reg(cr(3)) & !CR_EN);
        assert_eq!(
            dma.read_reg(cr(3)) & CR_EN,
            CR_EN,
            "EN reads 1 until stopped"
        );
        step(&mut dma, &mut mem);
        assert_eq!(dma.read_reg(cr(3)) & CR_EN, 0);
        assert_eq!(mem.get(0x2000_0000, 5), vec![9, 8, 7, 6, 5]);
        assert_eq!(mem.writes, vec![(0x2000_0000, 4), (0x2000_0004, 1)]);
        assert!(flag(&dma, 3, F_TCIF));
        assert_eq!(dma.read_reg(ndtr(3)), 5);
    }

    /// RM0090 "Arbiter": two memory-to-memory streams started together — the
    /// higher SxCR.PL runs first, then (equal PL) the lower stream number.
    #[test]
    fn the_arbiter_serves_by_pl_then_stream_number() {
        fn run(pl0: u32, pl1: u32) -> Vec<usize> {
            let mut mem = Mem::default();
            let mut dma = StreamDma::new().as_dma2();
            for (s, p) in [(0u64, pl0), (1, pl1)] {
                dma.write_reg(ndtr(s), 8);
                dma.write_reg(par(s), 0x2000_0000 + 0x100 * s as u32);
                dma.write_reg(m0ar(s), 0x2000_1000 + 0x100 * s as u32);
                dma.write_reg(fcr(s), FCR_DMDIS | 1);
                dma.write_reg(cr(s), CR_EN | M2M | CR_PINC | CR_MINC | pl(p));
            }
            let mut order = Vec::new();
            for _ in 0..40 {
                step(&mut dma, &mut mem);
                for s in 0..2 {
                    if flag(&dma, s, F_TCIF) && !order.contains(&s) {
                        order.push(s);
                    }
                }
            }
            order
        }
        assert_eq!(run(0, 3), vec![1, 0], "very high PL first");
        assert_eq!(run(2, 2), vec![0, 1], "equal PL: lower stream first");
        assert_eq!(run(1, 0), vec![0, 1]);
    }

    /// Circular P2M with peripheral bursts: NDTR reloads after the FIFO is
    /// flushed, and the second pass overwrites the buffer from its start.
    #[test]
    fn circular_mode_with_bursts_reloads_after_the_flush() {
        let mut mem = Mem::default();
        mem.rx.extend(1..=16u8);
        let mut dma = StreamDma::new();
        dma.write_reg(ndtr(0), 8);
        dma.write_reg(par(0), RX_DR as u32);
        dma.write_reg(m0ar(0), 0x2000_0000);
        dma.write_reg(fcr(0), FCR_DMDIS | 1);
        dma.write_reg(
            cr(0),
            CR_EN | CR_MINC | CR_CIRC | psize(1) | msize(4) | pburst(4),
        );
        for _ in 0..2 {
            dma.dma_request(1);
            steps(&mut dma, &mut mem, 2);
        }
        step(&mut dma, &mut mem);
        assert_eq!(mem.get(0x2000_0000, 8), (1..=8).collect::<Vec<u8>>());
        assert!(flag(&dma, 0, F_TCIF));
        assert_eq!(dma.read_reg(ndtr(0)), 8, "reloaded");
        assert_eq!(dma.read_reg(cr(0)) & CR_EN, CR_EN, "still running");
        dma.write_reg(0x08, flags_to_reg_bits(F_TCIF | F_HTIF, 0));
        for _ in 0..2 {
            dma.dma_request(1);
            steps(&mut dma, &mut mem, 2);
        }
        step(&mut dma, &mut mem);
        assert_eq!(mem.get(0x2000_0000, 8), (9..=16).collect::<Vec<u8>>());
        assert!(flag(&dma, 0, F_TCIF));
    }

    /// Double-buffer mode (direct, M2P): CT toggles at each completion and
    /// the memory port switches between M0AR and M1AR; DBM forces CIRC.
    #[test]
    fn double_buffer_mode_switches_buffers() {
        let mut mem = Mem::default();
        mem.fill(0x2000_0000, &[1, 2]);
        mem.fill(0x2000_0100, &[3, 4]);
        let mut dma = StreamDma::new();
        dma.write_reg(ndtr(7), 2);
        dma.write_reg(par(7), TX_DR as u32);
        dma.write_reg(m0ar(7), 0x2000_0000);
        dma.write_reg(m1ar(7), 0x2000_0100);
        dma.write_reg(cr(7), CR_EN | M2P | CR_MINC | CR_DBM);
        assert_ne!(dma.read_reg(cr(7)) & CR_CIRC, 0, "DBM forces CIRC");
        for _ in 0..6 {
            dma.dma_request(8);
            step(&mut dma, &mut mem);
        }
        assert_eq!(
            mem.tx.iter().map(|&(v, _)| v).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 1, 2]
        );
        assert_ne!(dma.read_reg(cr(7)) & CR_CT, 0, "on buffer 1 after 3 passes");
    }

    #[test]
    fn tcif_pends_stream_irq_and_lifcr_clears() {
        let mut mem = Mem::default();
        let mut dma = StreamDma::new()
            .as_dma2()
            .with_stream_irqs(vec![56, 57, 58, 59, 60, 68, 69, 70]);
        dma.write_reg(ndtr(5), 2);
        dma.write_reg(par(5), 0x2000_0000);
        dma.write_reg(m0ar(5), 0x2000_0100);
        dma.write_reg(cr(5), CR_EN | M2M | CR_MINC | CR_TCIE);
        let mut lines = Vec::new();
        for _ in 0..4 {
            let r = step(&mut dma, &mut mem);
            assert!(!r.irq);
            lines.extend(r.explicit_irqs.unwrap_or_default());
        }
        // Stream 5 pends its own NVIC vector (68), not the block's own line.
        assert_eq!(lines, vec![68]);
        // Stream 5 is in HISR (position 1). TCIF set there.
        assert_ne!(dma.read_reg(0x04) & flags_to_reg_bits(F_TCIF, 1), 0);
        // HIFCR (0x0C) write-1-clears it.
        dma.write_reg(0x0C, flags_to_reg_bits(F_TCIF, 1));
        assert_eq!(dma.read_reg(0x04) & flags_to_reg_bits(F_TCIF, 1), 0);
    }

    #[cfg(feature = "event-scheduler")]
    mod scheduler_mode {
        use super::*;
        use crate::CycleClock;

        struct SchedHarness {
            dma: StreamDma,
            mem: Mem,
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
                    mem: seeded_mem(),
                    clock,
                    bus: crate::bus::SystemBus::new(),
                    events: Vec::new(),
                    now: 0,
                    pends: Vec::new(),
                    reqs: Vec::new(),
                }
            }
            fn harvest(&mut self) {
                for (delay, token) in self.dma.take_scheduled_events() {
                    self.events.push((self.now + 1 + delay, token));
                }
            }
            fn write(&mut self, offset: u64, value: u32) {
                self.dma.sync_to(self.now);
                self.dma.write_reg(offset, value);
                self.harvest();
            }
            fn request(&mut self, id: u32) {
                self.dma.dma_request(id);
                self.harvest();
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
                    if let Some(delay) = res.reschedule_delay {
                        self.events.push((self.now + delay, token));
                    }
                    // The machine executes the items right after `on_event`,
                    // then harvests whatever they armed.
                    exec(&mut self.dma, &mut self.mem, &res.dma_requests);
                    self.harvest();
                    for r in res.dma_requests {
                        self.reqs.push((self.now, r));
                    }
                }
            }
        }

        fn seeded_mem() -> Mem {
            let mut mem = Mem::default();
            mem.fill(0x2000_0000, &(0..64).collect::<Vec<u8>>());
            mem.rx.extend(0..64u8);
            mem
        }

        #[derive(Clone, Copy)]
        enum Op {
            Write(u64, u32),
            Request(u32),
        }

        /// Replay a script against (a) the legacy walk and (b) the event path;
        /// compare full register snapshot every cycle, the emitted request
        /// stream, the per-cycle IRQ pend set, and the memory written.
        fn assert_walk_identical(is_dma2: bool, script: &[(u64, Op)], cycles: u64, what: &str) {
            let mut walk = StreamDma::new();
            if is_dma2 {
                walk = walk.as_dma2();
            }
            walk = walk.with_stream_irqs(vec![11, 12, 13, 14, 15, 16, 17, 47]);
            let mut walk_mem = seeded_mem();
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
                let res = step(&mut walk, &mut walk_mem);
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
            assert_eq!(walk_mem.ram, sched.mem.ram, "{what}: memory diverged");
        }

        #[test]
        fn clock_attach_flips_scheduler_and_tick_is_inert() {
            let mut dma = StreamDma::new().as_dma2();
            dma.attach_cycle_clock(CycleClock::default());
            assert!(dma.uses_scheduler());
            assert!(!dma.needs_legacy_walk());
            dma.write_reg(ndtr(0), 4);
            dma.write_reg(cr(0), CR_EN | M2M);
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
                (1, Op::Write(cr(0), CR_EN | M2M | CR_PINC | CR_MINC)),
                (30, Op::Write(0x08, 0xFFFF_FFFF)), // LIFCR clear-all mid-idle
            ];
            assert_walk_identical(true, &script, 50, "m2m 8-item self-paced");
        }

        #[test]
        fn m2m_htie_tcie_burst_irq_walk_identity() {
            let script = [
                (1u64, Op::Write(ndtr(0), 32)),
                (1, Op::Write(par(0), 0x2000_0000)),
                (1, Op::Write(m0ar(0), 0x2000_0100)),
                (1, Op::Write(fcr(0), FCR_DMDIS | 3)),
                (
                    1,
                    Op::Write(
                        cr(0),
                        CR_EN
                            | M2M
                            | CR_PINC
                            | CR_MINC
                            | CR_HTIE
                            | CR_TCIE
                            | psize(1)
                            | msize(4)
                            | mburst(4),
                    ),
                ),
            ];
            assert_walk_identical(true, &script, 60, "m2m HTIE+TCIE burst");
        }

        #[test]
        fn peripheral_paced_request_driven_walk_identity() {
            let script = [
                (1u64, Op::Write(ndtr(2), 5)),
                (1, Op::Write(par(2), 0x4000_0000)),
                (1, Op::Write(m0ar(2), 0x2000_0100)),
                // P2M (DIR=00) | MINC | TCIE, direct mode.
                (1, Op::Write(cr(2), CR_EN | CR_MINC | CR_TCIE)),
                (6, Op::Request(3)),
                (10, Op::Request(3)),
                (14, Op::Request(3)),
                (18, Op::Request(3)),
            ];
            assert_walk_identical(false, &script, 30, "peripheral-paced request-driven");
        }

        #[test]
        fn fifo_p2m_and_m2p_with_priorities_walk_identity() {
            let script = [
                (1u64, Op::Write(ndtr(1), 8)),
                (1, Op::Write(par(1), RX_DR as u32)),
                (1, Op::Write(m0ar(1), 0x2000_0200)),
                (1, Op::Write(fcr(1), FCR_DMDIS | FCR_FEIE | 1)),
                (1, Op::Write(cr(1), CR_EN | CR_MINC | CR_TCIE | msize(2))),
                (1, Op::Write(ndtr(6), 6)),
                (1, Op::Write(par(6), TX_DR as u32)),
                (1, Op::Write(m0ar(6), 0x2000_0000)),
                (1, Op::Write(fcr(6), FCR_DMDIS | 3)),
                (
                    1,
                    Op::Write(cr(6), CR_EN | M2P | CR_MINC | CR_TCIE | msize(4) | pl(3)),
                ),
                (3, Op::Request(2)),
                (3, Op::Request(7)),
                (4, Op::Request(2)),
                (5, Op::Request(7)),
                (6, Op::Request(2)),
                (6, Op::Request(2)),
                (7, Op::Request(7)),
                (8, Op::Request(7)),
                (9, Op::Request(7)),
                (9, Op::Request(2)),
                (12, Op::Request(2)),
                (12, Op::Request(2)),
                (13, Op::Request(2)),
                (14, Op::Write(cr(6), 0)), // disable mid-way
            ];
            assert_walk_identical(false, &script, 40, "FIFO P2M + M2P");
        }

        #[test]
        fn transfer_error_walk_identity() {
            let script = [
                (1u64, Op::Write(ndtr(3), 4)),
                (1, Op::Write(par(3), HOLE as u32)),
                (1, Op::Write(m0ar(3), 0x2000_0000)),
                (1, Op::Write(cr(3), CR_EN | M2M | CR_PINC | CR_TEIE)),
            ];
            assert_walk_identical(true, &script, 10, "transfer error");
        }
    }
}
