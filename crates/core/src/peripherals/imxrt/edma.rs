// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! i.MX RT1060 enhanced DMA (eDMA, DMA0 `0x400E_8000`, IMXRT1060RM §6) with
//! its request multiplexer (DMAMUX `0x400E_C000`, §5).
//!
//! 32 channels, each driven by a transfer control descriptor (TCD) at
//! `0x1000 + 32*n`. A channel runs one minor loop per service request: an
//! enabled hardware request (`ERQ` bit + DMAMUX `CHCFG.ENBL`, source routed
//! to a peripheral request line, or `A_ON` always-on) or a software start
//! (`TCD.CSR.START`). A minor loop moves `NBYTES` reading `SSIZE` units at
//! `SADDR` (+`SOFF`, `SMOD` modulo) and writing `DSIZE` units at `DADDR`
//! (+`DOFF`, `DMOD`), then applies the minor-loop offset (`CR.EMLM`), counts
//! `CITER` down and on reaching zero completes the major loop: `SLAST` /
//! `DLAST`, or a scatter/gather reload of the whole TCD from `DLAST_SGA`
//! (`ESG`), `DONE`, the major-loop interrupt (`INTMAJOR`/`INTHALF`),
//! `DREQ` auto-disable and channel linking (`ELINK`, `MAJORELINK`).
//!
//! Transfers are performed by the bus-aware tick through the system bus, so
//! a peripheral data register sees an access of the programmed width.
//! Peripheral request lines are read through
//! [`crate::Peripheral::dma_request_active`]; the DMAMUX source numbers are
//! the NXP ones (`dma_request_source_t` in the MCUXpresso device header).

use super::byte_of;
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

const CR: u32 = 0x00;
const ES: u32 = 0x04;
const ERQ: u32 = 0x0C;
const EEI: u32 = 0x14;
const INT: u32 = 0x24;
const ERR: u32 = 0x2C;
const HRS: u32 = 0x34;
const EARS: u32 = 0x44;
const TCD_BASE: u32 = 0x1000;

const CR_EMLM: u32 = 1 << 7;

const CSR_START: u16 = 1 << 0;
const CSR_INTMAJOR: u16 = 1 << 1;
const CSR_INTHALF: u16 = 1 << 2;
const CSR_DREQ: u16 = 1 << 3;
const CSR_ESG: u16 = 1 << 4;
const CSR_MAJORELINK: u16 = 1 << 5;
const CSR_ACTIVE: u16 = 1 << 6;
const CSR_DONE: u16 = 1 << 7;

/// Chip-YAML id of the request-routing register file the engine consults
/// (its base comes from the chip descriptor, not from here).
pub const DMAMUX_ID: &str = "dmamux";

/// DMAMUX source -> (peripheral id in the chip YAML, request line).
/// Source numbers: NXP `dma_request_source_t` (MCUXpresso RT1050 device
/// header `PERI_DMAMUX.h`; RT1060 uses the same numbers for these sources).
pub const DMAMUX_SOURCES: &[(u32, &str, u8)] = &[
    (1, "flexio2", 0),
    (65, "flexio2", 2),
    (2, "lpuart1", 0),
    (3, "lpuart1", 1),
    (66, "lpuart2", 0),
    (67, "lpuart2", 1),
    (4, "lpuart3", 0),
    (5, "lpuart3", 1),
    (68, "lpuart4", 0),
    (69, "lpuart4", 1),
    (6, "lpuart5", 0),
    (7, "lpuart5", 1),
    (70, "lpuart6", 0),
    (71, "lpuart6", 1),
    (8, "lpuart7", 0),
    (9, "lpuart7", 1),
    (72, "lpuart8", 0),
    (73, "lpuart8", 1),
    (17, "lpi2c1", 0),
    (81, "lpi2c2", 0),
    (18, "lpi2c3", 0),
    (82, "lpi2c4", 0),
    (19, "sai1", 1),
    (20, "sai1", 0),
    (21, "sai2", 1),
    (22, "sai2", 0),
    (83, "sai3", 1),
    (84, "sai3", 0),
    (24, "adc1", 0),
    (88, "adc2", 0),
];

#[derive(Debug, Clone, Copy, Default)]
struct Tcd {
    saddr: u32,
    soff: i16,
    attr: u16,
    nbytes: u32,
    slast: i32,
    daddr: u32,
    doff: i16,
    citer: u16,
    dlast_sga: i32,
    csr: u16,
    biter: u16,
}

impl Tcd {
    fn read_word(&self, off: u32) -> u32 {
        match off {
            0x00 => self.saddr,
            0x04 => (self.soff as u16 as u32) | ((self.attr as u32) << 16),
            0x08 => self.nbytes,
            0x0C => self.slast as u32,
            0x10 => self.daddr,
            0x14 => (self.doff as u16 as u32) | ((self.citer as u32) << 16),
            0x18 => self.dlast_sga as u32,
            0x1C => (self.csr as u32) | ((self.biter as u32) << 16),
            _ => 0,
        }
    }
    fn write_word(&mut self, off: u32, v: u32) {
        match off {
            0x00 => self.saddr = v,
            0x04 => {
                self.soff = v as u16 as i16;
                self.attr = (v >> 16) as u16;
            }
            0x08 => self.nbytes = v,
            0x0C => self.slast = v as i32,
            0x10 => self.daddr = v,
            0x14 => {
                self.doff = v as u16 as i16;
                self.citer = (v >> 16) as u16;
            }
            0x18 => self.dlast_sga = v as i32,
            0x1C => {
                self.csr = v as u16;
                self.biter = (v >> 16) as u16;
            }
            _ => {}
        }
    }
    fn from_bytes(b: &[u8; 32]) -> Self {
        let mut t = Tcd::default();
        for k in 0..8 {
            let w = u32::from_le_bytes([b[4 * k], b[4 * k + 1], b[4 * k + 2], b[4 * k + 3]]);
            t.write_word(4 * k as u32, w);
        }
        t
    }
    fn citer_count(&self) -> u16 {
        if self.citer & 0x8000 != 0 {
            self.citer & 0x1FF
        } else {
            self.citer & 0x7FFF
        }
    }
    fn set_citer_count(&mut self, n: u16) {
        if self.citer & 0x8000 != 0 {
            self.citer = (self.citer & !0x1FF) | (n & 0x1FF);
        } else {
            self.citer = (self.citer & 0x8000) | (n & 0x7FFF);
        }
    }
    fn biter_count(&self) -> u16 {
        if self.biter & 0x8000 != 0 {
            self.biter & 0x1FF
        } else {
            self.biter & 0x7FFF
        }
    }
}

fn size_bytes(code: u16) -> u32 {
    match code & 7 {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        5 => 32,
        _ => 4,
    }
}

/// Apply an address offset with an optional power-of-two modulo window.
fn advance(addr: u32, off: i32, modulo: u16) -> u32 {
    let next = addr.wrapping_add(off as u32);
    if modulo == 0 {
        next
    } else {
        let m = (1u32 << modulo) - 1;
        (addr & !m) | (next & m)
    }
}

#[derive(Debug)]
pub struct ImxrtEdma {
    cr: u32,
    es: u32,
    erq: u32,
    eei: u32,
    int: u32,
    err: u32,
    ears: u32,
    dchpri: [u8; 32],
    tcd: [Tcd; 32],
    /// INT bits raised since the last tick (to pend their NVIC lines).
    new_int: u32,
    /// Peripheral indices for the DMAMUX source table (resolved lazily).
    source_idx: Vec<Option<usize>>,
    resolved: bool,
    /// DMAMUX window base, resolved from the bus by id.
    dmamux_base: Option<u64>,
    /// Minor loops executed per channel (for inspection).
    minor_loops: [u64; 32],
}

impl Default for ImxrtEdma {
    fn default() -> Self {
        Self::new()
    }
}

impl ImxrtEdma {
    pub fn new() -> Self {
        let mut dchpri = [0u8; 32];
        for (i, p) in dchpri.iter_mut().enumerate() {
            // Reset: each channel's priority is its own number (group 0).
            *p = (i as u8) & 0xF;
        }
        Self {
            cr: 0,
            es: 0,
            erq: 0,
            eei: 0,
            int: 0,
            err: 0,
            ears: 0,
            dchpri,
            tcd: [Tcd::default(); 32],
            new_int: 0,
            source_idx: Vec::new(),
            resolved: false,
            dmamux_base: None,
            minor_loops: [0; 32],
        }
    }

    /// Minor loops executed per channel so far.
    pub fn minor_loops(&self) -> &[u64; 32] {
        &self.minor_loops
    }

    fn any_start(&self) -> bool {
        self.tcd.iter().any(|t| t.csr & CSR_START != 0)
    }

    pub fn read_reg(&self, off: u32) -> u32 {
        match off {
            CR => self.cr,
            ES => self.es,
            ERQ => self.erq,
            EEI => self.eei,
            INT => self.int,
            ERR => self.err,
            HRS => 0,
            EARS => self.ears,
            o if (0x100..0x120).contains(&o) => {
                // DCHPRIn byte order: DCHPRI3..0 in the first word.
                let mut v = 0u32;
                for k in 0..4 {
                    let byte_off = o - 0x100 + k;
                    let ch = (byte_off & !3) + (3 - (byte_off & 3));
                    v |= (self.dchpri[ch as usize] as u32) << (8 * k);
                }
                v
            }
            o if (TCD_BASE..TCD_BASE + 32 * 32).contains(&o) => {
                let n = ((o - TCD_BASE) / 32) as usize;
                self.tcd[n].read_word((o - TCD_BASE) % 32)
            }
            _ => 0,
        }
    }

    /// A byte-granular write (the eDMA has 8-bit command registers at
    /// 0x18..0x1F that firmware stores with STRB).
    fn write_byte(&mut self, off: u32, b: u8) {
        let ch = (b & 0x1F) as u32;
        let all = b & 0x40 != 0;
        let nop = b & 0x80 != 0;
        match off {
            0x18 if !nop => self.eei &= if all { 0 } else { !(1 << ch) }, // CEEI
            0x19 if !nop => self.eei |= if all { u32::MAX } else { 1 << ch }, // SEEI
            0x1A if !nop => self.erq &= if all { 0 } else { !(1 << ch) }, // CERQ
            0x1B if !nop => self.erq |= if all { u32::MAX } else { 1 << ch }, // SERQ
            0x1C if !nop => {
                // CDNE
                for n in 0..32 {
                    if all || n == ch as usize {
                        self.tcd[n].csr &= !CSR_DONE;
                    }
                }
            }
            0x1D if !nop => {
                // SSRT
                for n in 0..32 {
                    if all || n == ch as usize {
                        self.tcd[n].csr |= CSR_START;
                    }
                }
            }
            0x1E if !nop => self.err &= if all { 0 } else { !(1 << ch) }, // CERR
            0x1F if !nop => self.int &= if all { 0 } else { !(1 << ch) }, // CINT
            o if (0x100..0x120).contains(&o) => {
                let byte_off = o - 0x100;
                let ch = (byte_off & !3) + (3 - (byte_off & 3));
                self.dchpri[ch as usize] = b;
            }
            _ => {}
        }
    }

    pub fn write_reg(&mut self, off: u32, value: u32, mask: u32) {
        let v = value & mask;
        match off {
            CR => self.cr = (self.cr & !mask) | v,
            ERQ => self.erq = (self.erq & !mask) | v,
            EEI => self.eei = (self.eei & !mask) | v,
            INT => self.int &= !v, // w1c
            ERR => self.err &= !v, // w1c
            EARS => self.ears = (self.ears & !mask) | v,
            0x18 | 0x1C | 0x100..=0x11C => {
                for k in 0..4 {
                    if mask & (0xFF << (8 * k)) != 0 {
                        self.write_byte(off + k, (v >> (8 * k)) as u8);
                    }
                }
            }
            o if (TCD_BASE..TCD_BASE + 32 * 32).contains(&o) => {
                let n = ((o - TCD_BASE) / 32) as usize;
                let w = (o - TCD_BASE) % 32;
                let old = self.tcd[n].read_word(w);
                self.tcd[n].write_word(w, (old & !mask) | v);
                if w == 0x1C && v & CSR_DONE as u32 != 0 {
                    // DONE is w0c from firmware's point of view; writing 1 is
                    // ignored (it is cleared by CDNE or a new start).
                }
            }
            _ => {}
        }
    }

    fn resolve_sources(&mut self, bus: &mut dyn Bus) {
        if self.resolved {
            return;
        }
        let sb = bus
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<crate::bus::SystemBus>());
        self.source_idx = DMAMUX_SOURCES
            .iter()
            .map(|(_, name, _)| {
                sb.as_ref()
                    .and_then(|b| b.find_peripheral_index_by_name(name))
            })
            .collect();
        self.dmamux_base = sb.as_ref().and_then(|b| {
            b.find_peripheral_index_by_name(DMAMUX_ID)
                .map(|i| b.peripherals[i].base)
        });
        self.resolved = true;
    }

    /// Is the DMAMUX routing an asserted request to channel `ch`?
    fn hw_request(&self, bus: &mut dyn Bus, ch: usize) -> bool {
        let Some(mux) = self.dmamux_base else {
            return false; // no DMAMUX wired: hardware requests never route
        };
        let chcfg = bus.read_u32(mux + 4 * ch as u64).unwrap_or(0);
        if chcfg & (1 << 31) == 0 {
            return false; // ENBL
        }
        if chcfg & (1 << 29) != 0 {
            return true; // A_ON
        }
        let src = chcfg & 0x7F;
        let Some(pos) = DMAMUX_SOURCES.iter().position(|(s, _, _)| *s == src) else {
            return false;
        };
        let Some(Some(idx)) = self.source_idx.get(pos) else {
            return false;
        };
        let line = DMAMUX_SOURCES[pos].2;
        let Some(sb) = bus
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<crate::bus::SystemBus>())
        else {
            return false;
        };
        sb.peripherals
            .get(*idx)
            .is_some_and(|p| p.dev.dma_request_active(line))
    }

    /// Run one minor loop of channel `ch`.
    fn minor_loop(&mut self, bus: &mut dyn Bus, ch: usize) {
        let mut t = self.tcd[ch];
        t.csr &= !CSR_START;
        t.csr |= CSR_ACTIVE;
        t.csr &= !CSR_DONE;
        let ssize = size_bytes(t.attr >> 8);
        let dsize = size_bytes(t.attr);
        let smod = (t.attr >> 11) & 0x1F;
        let dmod = (t.attr >> 3) & 0x1F;
        let (nbytes, mloff) = if self.cr & CR_EMLM != 0 && t.nbytes & 0xC000_0000 != 0 {
            let off = ((t.nbytes >> 10) & 0xF_FFFF) as i32;
            let off = (off << 12) >> 12; // sign-extend 20 bits
            (t.nbytes & 0x3FF, Some(off))
        } else if self.cr & CR_EMLM != 0 {
            (t.nbytes & 0x3FFF_FFFF, None)
        } else {
            (t.nbytes, None)
        };
        let nbytes = if nbytes == 0 {
            0x1_0000_0000u64 as usize
        } else {
            nbytes as usize
        };
        let nbytes = nbytes.min(1 << 20);
        // Gather source units, scatter destination units.
        let mut buf = Vec::with_capacity(nbytes);
        let mut s = t.saddr;
        crate::with_device_write_bypass(bus, |bus| {
            while buf.len() < nbytes {
                match ssize {
                    1 => buf.push(bus.read_u8(s as u64).unwrap_or(0)),
                    2 => buf.extend(bus.read_u16(s as u64).unwrap_or(0).to_le_bytes()),
                    _ => {
                        let mut k = 0;
                        while k < ssize {
                            buf.extend(bus.read_u32((s + k) as u64).unwrap_or(0).to_le_bytes());
                            k += 4;
                        }
                    }
                }
                s = advance(s, t.soff as i32, smod);
            }
        });
        buf.truncate(nbytes);
        let mut d = t.daddr;
        let mut k = 0usize;
        crate::with_device_write_bypass(bus, |bus| {
            while k < nbytes {
                match dsize {
                    1 => {
                        let _ = bus.write_u8(d as u64, buf[k]);
                    }
                    2 => {
                        let v = u16::from_le_bytes([buf[k], *buf.get(k + 1).unwrap_or(&0)]);
                        let _ = bus.write_u16(d as u64, v);
                    }
                    _ => {
                        let mut j = 0;
                        while j < dsize as usize {
                            let w = |i: usize| *buf.get(k + j + i).unwrap_or(&0);
                            let v = u32::from_le_bytes([w(0), w(1), w(2), w(3)]);
                            let _ = bus.write_u32((d + j as u32) as u64, v);
                            j += 4;
                        }
                    }
                }
                k += dsize as usize;
                d = advance(d, t.doff as i32, dmod);
            }
        });
        if let Some(off) = mloff {
            if t.nbytes & (1 << 31) != 0 {
                s = s.wrapping_add(off as u32);
            }
            if t.nbytes & (1 << 30) != 0 {
                d = d.wrapping_add(off as u32);
            }
        }
        t.saddr = s;
        t.daddr = d;
        self.minor_loops[ch] += 1;
        let left = t.citer_count().saturating_sub(1);
        t.set_citer_count(left);
        let mut link: Option<usize> = None;
        if left == 0 {
            // Major loop complete.
            t.csr |= CSR_DONE;
            t.csr &= !CSR_ACTIVE;
            if t.csr & CSR_INTMAJOR != 0 {
                self.int |= 1 << ch;
                self.new_int |= 1 << ch;
            }
            if t.csr & CSR_DREQ != 0 {
                self.erq &= !(1 << ch);
            }
            if t.csr & CSR_MAJORELINK != 0 {
                link = Some(((t.csr >> 8) & 0x1F) as usize);
            }
            if t.csr & CSR_ESG != 0 {
                // Scatter/gather: load the next TCD from memory.
                let mut raw = [0u8; 32];
                for (i, b) in raw.iter_mut().enumerate() {
                    *b = bus
                        .read_u8(t.dlast_sga as u32 as u64 + i as u64)
                        .unwrap_or(0);
                }
                let mut next = Tcd::from_bytes(&raw);
                next.csr &= !CSR_DONE;
                // DONE of the finished descriptor stays visible until the
                // new one starts.
                t = next;
                t.csr |= CSR_DONE;
            } else {
                t.saddr = t.saddr.wrapping_add(t.slast as u32);
                t.daddr = t.daddr.wrapping_add(t.dlast_sga as u32);
                let biter = t.biter_count();
                t.set_citer_count(biter);
            }
        } else {
            if t.csr & CSR_INTHALF != 0 && left == t.biter_count() / 2 {
                self.int |= 1 << ch;
                self.new_int |= 1 << ch;
            }
            if t.citer & 0x8000 != 0 {
                link = Some(((t.citer >> 9) & 0x1F) as usize);
            }
            t.csr &= !CSR_ACTIVE;
        }
        self.tcd[ch] = t;
        if let Some(l) = link {
            self.tcd[l].csr |= CSR_START;
        }
    }
}

impl Peripheral for ImxrtEdma {
    /// Walked only to deliver completion interrupts raised by the bus tick.
    fn legacy_tick_active(&self) -> bool {
        self.new_int != 0
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(byte_of(self.read_reg(offset as u32 & !3), offset))
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset as u32;
        if (0x18..0x20).contains(&off) || (0x100..0x120).contains(&off) {
            self.write_byte(off, value);
            return Ok(());
        }
        let shift = (offset & 3) * 8;
        self.write_reg(off & !3, (value as u32) << shift, 0xFF << shift);
        Ok(())
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        let off = offset as u32;
        if (0x18..0x20).contains(&off) {
            self.write_byte(off, value as u8);
            self.write_byte(off + 1, (value >> 8) as u8);
            return Ok(());
        }
        let shift = (offset & 3) * 8;
        self.write_reg(off & !3, (value as u32) << shift, 0xFFFF << shift);
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset as u32))
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset as u32, value, u32::MAX);
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        let fired = std::mem::take(&mut self.new_int);
        if fired == 0 {
            return PeripheralTickResult::default();
        }
        // Channel n and n+16 share NVIC line n (DMA0_DMA16 .. DMA15_DMA31).
        let mut lines: Vec<u32> = (0..32)
            .filter(|ch| fired & (1 << ch) != 0)
            .map(|ch| ch % 16)
            .collect();
        lines.dedup();
        PeripheralTickResult {
            explicit_irqs: Some(lines),
            ..Default::default()
        }
    }
    fn needs_bus_tick(&self) -> bool {
        self.erq != 0 || self.any_start()
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.resolve_sources(bus);
        // Fixed priority: highest channel number first (reset DCHPRI).
        for ch in (0..32).rev() {
            let start = self.tcd[ch].csr & CSR_START != 0;
            let hw = self.erq & (1 << ch) != 0 && self.hw_request(bus, ch);
            if start || hw {
                self.minor_loop(bus, ch);
                // An explicit start runs its whole major loop back to back
                // (a software-triggered memory-to-memory transfer).
                let mut guard = 0u32;
                while start
                    && self.tcd[ch].csr & CSR_DONE == 0
                    && self.tcd[ch].citer_count() > 0
                    && guard < 65_536
                {
                    self.minor_loop(bus, ch);
                    guard += 1;
                }
            }
        }
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "imxrt_edma",
            "erq": self.erq,
            "int": self.int,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcd_word_layout_round_trips() {
        let mut e = ImxrtEdma::new();
        e.write_reg(TCD_BASE + 0x04, 0x0202_0004, u32::MAX); // ATTR 32/32, SOFF 4
        e.write_reg(TCD_BASE + 0x14, 0x0010_0004, u32::MAX); // CITER 16, DOFF 4
        e.write_reg(TCD_BASE + 0x1C, 0x0010_0002, u32::MAX); // BITER 16, INTMAJOR
        assert_eq!(e.tcd[0].soff, 4);
        assert_eq!(e.tcd[0].attr, 0x0202);
        assert_eq!(e.tcd[0].citer_count(), 16);
        assert_eq!(e.read_reg(TCD_BASE + 0x1C), 0x0010_0002);
    }

    #[test]
    fn serq_cerq_byte_commands() {
        let mut e = ImxrtEdma::new();
        e.write(0x1B, 5).unwrap(); // SERQ 5
        e.write(0x1B, 0).unwrap();
        assert_eq!(e.read_reg(ERQ), 0b10_0001);
        e.write(0x1A, 0).unwrap(); // CERQ 0
        assert_eq!(e.read_reg(ERQ), 0b10_0000);
        e.write(0x1A, 0x40).unwrap(); // CERQ all
        assert_eq!(e.read_reg(ERQ), 0);
    }
}
