// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 GDMA (general DMA, `DMA`, `0x6003_F000`): three channels, each an
//! IN (receive) and an OUT (transmit) direction walking linked lists of
//! descriptors in RAM (C3 TRM §2).
//!
//! ## Register map (`configs/peripherals/esp32c3/dma.yaml`, SVD-sourced)
//!
//! | Offset              | Register                                          |
//! |--------------------:|---------------------------------------------------|
//! | `0x00 + n·0x10`     | `INT_RAW_CHn` / `INT_ST` / `INT_ENA` / `INT_CLR` (+0/4/8/C) |
//! | `0x40 / 0x44 / 0x48`| `AHB_TEST` / `MISC_CONF` / `DATE`                 |
//! | `0x70 + n·0xC0`     | `IN_CONF0` .. `IN_PERI_SEL` (+0x00 .. +0x30)      |
//! | `0xD0 + n·0xC0`     | `OUT_CONF0` .. `OUT_PERI_SEL` (+0x00 .. +0x30)    |
//!
//! One interrupt word per channel, IN and OUT bits together: `IN_DONE` 0,
//! `IN_SUC_EOF` 1, `IN_ERR_EOF` 2, `OUT_DONE` 3, `OUT_EOF` 4, `IN_DSCR_ERR`
//! 5, `OUT_DSCR_ERR` 6, `IN_DSCR_EMPTY` 7, `OUT_TOTAL_EOF` 8, FIFO over/
//! underflow 9..12. `INT_RAW` is write-1-to-clear, `INT_ST` = RAW & ENA,
//! `INT_CLR` write-only. Channel `n` asserts interrupt-matrix source
//! `44 + n` (`DMA_CH0_INT_MAP` at offset 176 of `interrupt_core0.yaml`).
//!
//! `IN_LINK` / `OUT_LINK`: the descriptor address's low 20 bits
//! (`INLINK_ADDR`), the DMA rebuilding the address in internal SRAM as
//! `0x3FC0_0000 | addr` (the C3's DMA reaches internal memory through the
//! data bus, `0x3FC8_0000..`); `START` / `STOP` / `RESTART` strobes read 0;
//! `PARK` reads 1 while the direction is idle.
//!
//! ## What moves bytes
//!
//! * **GP-SPI2** (`PERI_SEL` = 0) — see [`crate::peripherals::esp_gdma`]:
//!   an OUT start pushes the list's bytes into the controller, an IN start
//!   waits for the controller's DMA transaction and writes what it received.
//! * **Memory to memory** (`IN_CONF0.MEM_TRANS_EN`): once both directions
//!   of the channel are started, the OUT list's bytes are copied into the IN
//!   list.
//!
//! Both latch `*_DONE` per descriptor, `OUT_EOF` / `OUT_TOTAL_EOF` /
//! `IN_SUC_EOF` at the end of the frame, and the descriptor errors
//! `IN_DSCR_ERR` / `OUT_DSCR_ERR` / `IN_DSCR_EMPTY` (owner check, a descriptor,
//! buffer or next link outside the internal SRAM, an IN list too short — see
//! [`crate::peripherals::esp_gdma`]).
//!
//! Any other peripheral (UHCI0, I2S, AES, SHA, ADC) is not coupled: a START
//! is accepted and the direction stays busy (`PARK` = 0) without moving a
//! byte or latching EOF — a stall firmware can see, not a pretend transfer.
//! Every other register is plain storage under its reset value.

use crate::peripherals::esp_gdma::{
    fill_in, spi2_in_delivered, spi2_in_take, spi2_out_load, walk_out,
};
use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult};

/// Interrupt-matrix source of channel 0 (`DMA_CH0`); channels 1 and 2 follow.
pub const DMA_CH0_INTR_SOURCE_ID: u32 = 44;

const NUM_CHANNELS: usize = 3;
const INT_STRIDE: u64 = 0x10;
const BLOCK_BASE: u64 = 0x70;
const BLOCK_STRIDE: u64 = 0xC0;
/// Offsets inside a channel block.
const IN_CONF0: u64 = 0x00;
const IN_CONF1: u64 = 0x04;
const IN_LINK: u64 = 0x10;
const IN_SUC_EOF_DES_ADDR: u64 = 0x18;
const IN_PERI_SEL: u64 = 0x30;
const OUT_CONF0: u64 = 0x60;
const OUT_CONF1: u64 = 0x64;
const OUT_LINK: u64 = 0x70;
const OUT_EOF_DES_ADDR: u64 = 0x78;
const OUT_PERI_SEL: u64 = 0x90;
/// One past the last register (`OUT_PERI_SEL_CH2`).
const WINDOW_END: u64 = BLOCK_BASE + 2 * BLOCK_STRIDE + OUT_PERI_SEL + 4;

const INT_IN_DONE: u32 = 1 << 0;
const INT_IN_SUC_EOF: u32 = 1 << 1;
const INT_OUT_DONE: u32 = 1 << 3;
const INT_OUT_EOF: u32 = 1 << 4;
const INT_IN_DSCR_ERR: u32 = 1 << 5;
const INT_OUT_DSCR_ERR: u32 = 1 << 6;
const INT_IN_DSCR_EMPTY: u32 = 1 << 7;
const INT_OUT_TOTAL_EOF: u32 = 1 << 8;
const INT_MASK: u32 = 0x1FFF;

const LINK_ADDR_MASK: u32 = 0x000F_FFFF;
const IN_LINK_AUTO_RET: u32 = 1 << 20;
const IN_LINK_STOP: u32 = 1 << 21;
const IN_LINK_START: u32 = 1 << 22;
const IN_LINK_RESTART: u32 = 1 << 23;
const IN_LINK_PARK: u32 = 1 << 24;
const OUT_LINK_STOP: u32 = 1 << 20;
const OUT_LINK_START: u32 = 1 << 21;
const OUT_LINK_RESTART: u32 = 1 << 22;
const OUT_LINK_PARK: u32 = 1 << 23;

const CONF0_RST: u32 = 1 << 0;
const MEM_TRANS_EN: u32 = 1 << 4;
const OUT_AUTO_WRBACK: u32 = 1 << 2;
const CHECK_OWNER: u32 = 1 << 12;
const PERI_SEL_SPI2: u32 = 0;
/// Descriptor addresses are rebuilt in internal SRAM.
const DESC_PREFIX: u32 = 0x3FC0_0000;
/// The internal SRAM the DMA reaches, on the data bus: the chip yaml's `ram`
/// window (0x3FC8_0000, 400 KB). A descriptor, buffer or next link outside it
/// is a descriptor error.
const DMA_RAM: std::ops::Range<u32> = 0x3FC8_0000..0x3FC8_0000 + 400 * 1024;

/// Reset value of the architected register at `off`, or `None` for a hole.
fn reset_of(off: u64) -> Option<u32> {
    match off {
        0x00..=0x2C => Some(0),
        0x40 | 0x44 => Some(0),
        0x48 => Some(0x0200_8250), // DATE
        o if (BLOCK_BASE..WINDOW_END).contains(&o) => {
            match (o - BLOCK_BASE) % BLOCK_STRIDE {
                0x08 => Some(0x0780_0003), // INFIFO_STATUS
                0x0C => Some(0x0000_0800), // IN_POP
                IN_LINK => Some(0x0110_0000),
                IN_PERI_SEL | OUT_PERI_SEL => Some(0x3F),
                OUT_CONF0 => Some(0x0000_0008),
                0x68 => Some(0x0780_0002), // OUTFIFO_STATUS
                OUT_LINK => Some(0x0080_0000),
                r if r <= 0x30 || (0x60..=0x90).contains(&r) => Some(0),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A started direction waiting to move bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Busy {
    #[default]
    Idle,
    /// Started; the bus-aware pass serves it.
    Started,
}

#[derive(Debug, Default)]
pub struct Esp32c3Gdma {
    regs: Vec<u32>,
    int_raw: [u32; NUM_CHANNELS],
    in_busy: [Busy; NUM_CHANNELS],
    out_busy: [Busy; NUM_CHANNELS],
    source_base: u32,
    /// The internal SRAM the DMA reaches ([`DMA_RAM`]; tests widen it).
    dma_ram: std::ops::Range<u32>,
    clock: Option<CycleClock>,
}

impl Esp32c3Gdma {
    /// A GDMA whose channel 0 asserts interrupt-matrix source `source_base`.
    pub fn new(source_base: u32) -> Self {
        let mut regs = vec![0u32; (WINDOW_END / 4) as usize];
        for (w, r) in regs.iter_mut().enumerate() {
            *r = reset_of(w as u64 * 4).unwrap_or(0);
        }
        Self {
            regs,
            source_base,
            dma_ram: DMA_RAM,
            ..Default::default()
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn block(ch: usize, reg: u64) -> u64 {
        BLOCK_BASE + ch as u64 * BLOCK_STRIDE + reg
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs.get((off / 4) as usize).copied().unwrap_or(0)
    }

    fn set(&mut self, off: u64, v: u32) {
        if let Some(r) = self.regs.get_mut((off / 4) as usize) {
            *r = v;
        }
    }

    fn int_st(&self, ch: usize) -> u32 {
        self.int_raw[ch] & self.reg(ch as u64 * INT_STRIDE + 8)
    }

    fn desc_addr(&self, ch: usize, link: u64) -> u32 {
        DESC_PREFIX | (self.reg(Self::block(ch, link)) & LINK_ADDR_MASK)
    }

    fn mem2mem(&self, ch: usize) -> bool {
        self.reg(Self::block(ch, IN_CONF0)) & MEM_TRANS_EN != 0
    }

    fn peri(&self, ch: usize, sel: u64) -> u32 {
        self.reg(Self::block(ch, sel)) & 0x3F
    }

    fn write_reg(&mut self, off: u64, value: u32) {
        if off < 0x30 {
            let ch = (off / INT_STRIDE) as usize;
            match off % INT_STRIDE {
                // INT_RAW: write 1 to clear; INT_CLR: write-only clear.
                0x0 | 0xC => self.int_raw[ch] &= !(value & INT_MASK),
                0x4 => {} // INT_ST is read-only
                _ => self.set(off, value & INT_MASK),
            }
            return;
        }
        if reset_of(off).is_none() {
            return;
        }
        if (BLOCK_BASE..WINDOW_END).contains(&off) {
            let ch = ((off - BLOCK_BASE) / BLOCK_STRIDE) as usize;
            match (off - BLOCK_BASE) % BLOCK_STRIDE {
                IN_LINK => {
                    self.set(off, value & (LINK_ADDR_MASK | IN_LINK_AUTO_RET));
                    if value & IN_LINK_STOP != 0 {
                        self.in_busy[ch] = Busy::Idle;
                    }
                    if value & (IN_LINK_START | IN_LINK_RESTART) != 0 {
                        self.in_busy[ch] = Busy::Started;
                    }
                    return;
                }
                OUT_LINK => {
                    self.set(off, value & LINK_ADDR_MASK);
                    if value & OUT_LINK_STOP != 0 {
                        self.out_busy[ch] = Busy::Idle;
                    }
                    if value & (OUT_LINK_START | OUT_LINK_RESTART) != 0 {
                        self.out_busy[ch] = Busy::Started;
                    }
                    return;
                }
                IN_CONF0 if value & CONF0_RST != 0 => self.in_busy[ch] = Busy::Idle,
                OUT_CONF0 if value & CONF0_RST != 0 => self.out_busy[ch] = Busy::Idle,
                IN_PERI_SEL | OUT_PERI_SEL => {
                    self.set(off, value & 0x3F);
                    return;
                }
                _ => {}
            }
        }
        self.set(off, value);
    }

    fn read_reg(&self, off: u64) -> u32 {
        if off < 0x30 {
            let ch = (off / INT_STRIDE) as usize;
            return match off % INT_STRIDE {
                0x0 => self.int_raw[ch],
                0x4 => self.int_st(ch),
                0xC => 0,
                _ => self.reg(off),
            };
        }
        if reset_of(off).is_none() {
            return 0;
        }
        if (BLOCK_BASE..WINDOW_END).contains(&off) {
            let ch = ((off - BLOCK_BASE) / BLOCK_STRIDE) as usize;
            match (off - BLOCK_BASE) % BLOCK_STRIDE {
                IN_LINK if self.in_busy[ch] == Busy::Idle => {
                    return self.reg(off) | IN_LINK_PARK;
                }
                OUT_LINK if self.out_busy[ch] == Busy::Idle => {
                    return self.reg(off) | OUT_LINK_PARK;
                }
                _ => {}
            }
        }
        self.reg(off)
    }

    /// The bus-aware pass: serve every started direction that can move now.
    fn pump(&mut self, bus: &mut dyn Bus) {
        for ch in 0..NUM_CHANNELS {
            if self.mem2mem(ch) {
                if self.out_busy[ch] == Busy::Started && self.in_busy[ch] == Busy::Started {
                    let out = self.walk_out(bus, ch);
                    self.fill_in(bus, ch, &out);
                }
                continue;
            }
            if self.out_busy[ch] == Busy::Started && self.peri(ch, OUT_PERI_SEL) == PERI_SEL_SPI2 {
                let out = self.walk_out(bus, ch);
                spi2_out_load(bus, &out);
            }
            if self.in_busy[ch] == Busy::Started && self.peri(ch, IN_PERI_SEL) == PERI_SEL_SPI2 {
                if let Some(rx) = spi2_in_take(bus) {
                    self.fill_in(bus, ch, &rx);
                    spi2_in_delivered(bus);
                }
            }
        }
    }

    fn walk_out(&mut self, bus: &mut dyn Bus, ch: usize) -> Vec<u8> {
        let conf0 = self.reg(Self::block(ch, OUT_CONF0));
        let conf1 = self.reg(Self::block(ch, OUT_CONF1));
        let w = walk_out(
            bus,
            self.desc_addr(ch, OUT_LINK),
            conf1 & CHECK_OWNER != 0,
            conf0 & OUT_AUTO_WRBACK != 0,
            &self.dma_ram,
        );
        self.set(Self::block(ch, OUT_EOF_DES_ADDR), w.last_desc);
        if w.descs > 0 {
            self.int_raw[ch] |= INT_OUT_DONE;
        }
        if w.eof {
            self.int_raw[ch] |= INT_OUT_EOF | INT_OUT_TOTAL_EOF;
        }
        if w.dscr_err {
            self.int_raw[ch] |= INT_OUT_DSCR_ERR;
        }
        self.out_busy[ch] = Busy::Idle;
        w.bytes
    }

    fn fill_in(&mut self, bus: &mut dyn Bus, ch: usize, data: &[u8]) {
        let conf1 = self.reg(Self::block(ch, IN_CONF1));
        let f = fill_in(
            bus,
            self.desc_addr(ch, IN_LINK),
            data,
            conf1 & CHECK_OWNER != 0,
            &self.dma_ram,
        );
        self.set(Self::block(ch, IN_SUC_EOF_DES_ADDR), f.last_desc);
        if f.descs > 0 {
            self.int_raw[ch] |= INT_IN_DONE;
        }
        if f.suc_eof {
            self.int_raw[ch] |= INT_IN_SUC_EOF;
        }
        if f.dscr_err {
            self.int_raw[ch] |= INT_IN_DSCR_ERR;
        }
        if f.dscr_empty {
            self.int_raw[ch] |= INT_IN_DSCR_EMPTY;
        }
        self.in_busy[ch] = Busy::Idle;
    }

    /// A direction the bus-aware pass can serve.
    fn has_work(&self) -> bool {
        (0..NUM_CHANNELS).any(|ch| {
            if self.mem2mem(ch) {
                return self.out_busy[ch] == Busy::Started && self.in_busy[ch] == Busy::Started;
            }
            (self.out_busy[ch] == Busy::Started && self.peri(ch, OUT_PERI_SEL) == PERI_SEL_SPI2)
                || (self.in_busy[ch] == Busy::Started
                    && self.peri(ch, IN_PERI_SEL) == PERI_SEL_SPI2)
        })
    }

    fn sources(&self, out: &mut Vec<u32>) {
        for ch in 0..NUM_CHANNELS {
            if self.int_st(ch) != 0 {
                out.push(self.source_base + ch as u32);
            }
        }
    }
}

impl Peripheral for Esp32c3Gdma {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_reg(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset & 3) * 8;
        let byte = u32::from(value) << shift;
        // INT_RAW / INT_CLR are write-1-to-clear: the byte alone. Everything
        // else merges into the stored word (a LINK register stores no
        // strobe bits, so a byte write starts only what that byte says).
        let word = if reg < 0x30 && matches!(reg % INT_STRIDE, 0x0 | 0xC) {
            byte
        } else {
            (self.reg(reg) & !(0xFF << shift)) | byte
        };
        self.write_reg(reg, word);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn needs_bus_tick(&self) -> bool {
        self.has_work()
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.pump(bus);
    }

    /// Legacy walk: re-assert the level sources while an enabled interrupt
    /// bit is set (the bus routes them through the interrupt matrix).
    fn tick(&mut self) -> PeripheralTickResult {
        let mut explicit = Vec::new();
        self.sources(&mut explicit);
        PeripheralTickResult {
            explicit_irqs: (!explicit.is_empty()).then_some(explicit),
            ..Default::default()
        }
    }

    fn legacy_tick_active(&self) -> bool {
        (0..NUM_CHANNELS).any(|ch| self.int_st(ch) != 0)
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    /// Under the scheduler the level is exported through
    /// [`Self::matrix_irq_sources_into`]; there is no free-running state.
    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        self.sources(out);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::peripherals::esp_gdma::{DESC_OWNER, DESC_SUC_EOF};

    #[test]
    fn reset_values_match_the_svd() {
        let g = Esp32c3Gdma::new(44);
        assert_eq!(g.read_reg(0x48), 0x0200_8250, "DATE");
        assert_eq!(g.read_reg(0x80), 0x0110_0000, "IN_LINK_CH0: AUTO_RET, PARK");
        assert_eq!(g.read_reg(0xE0), 0x0080_0000, "OUT_LINK_CH0: PARK");
        assert_eq!(g.read_reg(0xA0), 0x3F, "IN_PERI_SEL_CH0 unbound");
        assert_eq!(g.read_reg(0x100), 0x3F, "OUT_PERI_SEL_CH0");
        assert_eq!(g.read_reg(0xD0), 0x8, "OUT_CONF0: OUT_EOF_MODE");
        assert_eq!(g.read_reg(0x78), 0x0780_0003, "INFIFO_STATUS");
        assert_eq!(g.read_reg(0x1A0), 0x0080_0000, "OUT_LINK_CH1");
    }

    #[test]
    fn interrupt_words_are_w1c_with_st_masked_by_ena() {
        let mut g = Esp32c3Gdma::new(44);
        g.int_raw[1] = INT_IN_SUC_EOF | INT_OUT_EOF;
        g.write_u32(0x18, INT_OUT_EOF).unwrap(); // INT_ENA_CH1
        assert_eq!(g.read_reg(0x14), INT_OUT_EOF, "INT_ST = RAW & ENA");
        let mut src = Vec::new();
        g.matrix_irq_sources_into(&mut src);
        assert_eq!(src, vec![45]);
        g.write_u32(0x1C, INT_OUT_EOF).unwrap(); // INT_CLR_CH1
        assert_eq!(g.read_reg(0x10), INT_IN_SUC_EOF);
        g.write_u32(0x10, INT_IN_SUC_EOF).unwrap(); // INT_RAW: W1C
        assert_eq!(g.read_reg(0x10), 0);
        assert_eq!(g.read_reg(0x1C), 0, "INT_CLR reads 0");
    }

    /// Memory to memory: the walkers copy an OUT list into an IN list, and
    /// the register side pairs the two directions of a MEM_TRANS_EN channel.
    #[test]
    fn mem_trans_copies_the_out_list_into_the_in_list() {
        let mut bus = SystemBus::new();
        let mut g = Esp32c3Gdma::new(44);
        let ram = 0x2000_0000u32;
        let sram = ram..ram + 0x1000;
        for (i, b) in [0x11u8, 0x22, 0x33].iter().enumerate() {
            bus.write_u8(u64::from(ram + 0x200) + i as u64, *b).unwrap();
        }
        // The walkers take full addresses; drive them directly.
        let out = walk_out(&mut bus, 0, false, false, &sram);
        assert!(out.bytes.is_empty(), "a null list moves nothing");
        bus.write_u32(u64::from(ram), DESC_OWNER | DESC_SUC_EOF | (3 << 12) | 3)
            .unwrap();
        bus.write_u32(u64::from(ram) + 4, ram + 0x200).unwrap();
        bus.write_u32(u64::from(ram) + 8, 0).unwrap();
        bus.write_u32(u64::from(ram) + 0x10, DESC_OWNER | 8)
            .unwrap();
        bus.write_u32(u64::from(ram) + 0x14, ram + 0x300).unwrap();
        bus.write_u32(u64::from(ram) + 0x18, 0).unwrap();
        let bytes = walk_out(&mut bus, ram, true, false, &sram).bytes;
        fill_in(&mut bus, ram + 0x10, &bytes, true, &sram);
        let got: Vec<u8> = (0..3)
            .map(|i| bus.read_u8(u64::from(ram + 0x300) + i).unwrap())
            .collect();
        assert_eq!(got, vec![0x11, 0x22, 0x33]);
        // The register side: START parks / un-parks, MEM_TRANS_EN pairs them.
        g.write_u32(0x70, MEM_TRANS_EN).unwrap(); // IN_CONF0_CH0
        g.write_u32(0xE0, OUT_LINK_START).unwrap();
        assert_eq!(g.read_reg(0xE0) & OUT_LINK_PARK, 0, "busy");
        assert!(!g.has_work(), "mem2mem waits for both directions");
        g.write_u32(0x80, IN_LINK_START | 0x10).unwrap();
        assert!(g.has_work());
        g.write_u32(0x80, IN_LINK_STOP).unwrap();
        assert_ne!(g.read_reg(0x80) & IN_LINK_PARK, 0, "STOP parks");
    }

    /// Descriptor errors land in INT_RAW: a CPU-owned OUT descriptor under
    /// OUT_CHECK_OWNER is OUT_DSCR_ERR (no EOF); an IN list shorter than the
    /// data is IN_DSCR_EMPTY (no SUC_EOF); a buffer outside the SRAM the DMA
    /// reaches is IN_DSCR_ERR.
    #[test]
    fn descriptor_errors_raise_their_int_raw_bits() {
        let ram = 0x2000_0000u32;
        let sram = ram..ram + 0x1000;
        // The walk the controller runs for a MEM_TRANS_EN channel with
        // OUT_CHECK_OWNER set: OUT list at `ram`, IN list at `ram + 0x10`.
        let run = |out_dw0: u32, in_dw0: u32, in_buf: u32| {
            let mut bus = SystemBus::new();
            for (at, dw0, buf) in [(0x00, out_dw0, ram + 0x200), (0x10, in_dw0, in_buf)] {
                bus.write_u32(u64::from(ram + at), dw0).unwrap();
                bus.write_u32(u64::from(ram + at) + 4, buf).unwrap();
                bus.write_u32(u64::from(ram + at) + 8, 0).unwrap();
            }
            let w = walk_out(&mut bus, ram, true, false, &sram);
            let f = fill_in(&mut bus, ram + 0x10, &w.bytes, false, &sram);
            (w, f)
        };
        // CHECK_OWNER on a CPU-owned OUT descriptor.
        let (w, _) = run(DESC_SUC_EOF | (4 << 12) | 4, DESC_OWNER | 8, ram + 0x300);
        assert!(w.dscr_err && !w.eof);
        // IN list too short for 4 bytes.
        let (w, f) = run(
            DESC_OWNER | DESC_SUC_EOF | (4 << 12) | 4,
            DESC_OWNER | 2,
            ram + 0x300,
        );
        assert!(w.eof && f.dscr_empty && !f.suc_eof);
        // IN buffer outside the DMA's SRAM.
        let (_, f) = run(
            DESC_OWNER | DESC_SUC_EOF | (4 << 12) | 4,
            DESC_OWNER | 8,
            0x6000_0000,
        );
        assert!(f.dscr_err);

        // And the controller maps them onto INT_RAW.
        let mut bus = SystemBus::new();
        let mut g = Esp32c3Gdma::new(44);
        g.dma_ram = DESC_PREFIX..DESC_PREFIX + 0x10_0000;
        g.write_u32(0x70, MEM_TRANS_EN).unwrap();
        g.write_u32(0xD4, CHECK_OWNER).unwrap();
        g.write_u32(0xE0, OUT_LINK_START | 0x100).unwrap();
        g.write_u32(0x80, IN_LINK_START | 0x200).unwrap();
        assert!(g.has_work());
        g.pump(&mut bus); // descriptors at 0x3FC0_0100: unmapped here
        let raw = g.read_reg(0x00);
        assert_ne!(raw & INT_OUT_DSCR_ERR, 0, "OUT_DSCR_ERR: {raw:#x}");
        assert_eq!(raw & (INT_OUT_EOF | INT_OUT_TOTAL_EOF | INT_IN_SUC_EOF), 0);
        assert_ne!(raw & INT_IN_DSCR_ERR, 0, "IN_DSCR_ERR");
    }
}
