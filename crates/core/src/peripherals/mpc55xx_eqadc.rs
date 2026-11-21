// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Freescale MPC55xx enhanced queued ADC (eQADC, MPC5554/5565/5566/5567):
//! six command FIFOs (CFIFO0..5, four entries each) feed two on-chip 12-bit
//! converters (ADC0/ADC1, selected per command by `BN`), and conversion
//! results return through six result FIFOs (RFIFO0..5, four entries each)
//! selected by the command's `MESSAGE_TAG`.
//!
//! Register map (the MPC5567 header's `EQADC_tag` at 0xFFF8_0000):
//! `MCR` @0x00, `NMSFR` @0x08, `ETDFR` @0x0C, `CFPRn` @0x10 + 4n,
//! `RFPRn` @0x30 + 4n (`RFPOP` in bits [15:0]), 16-bit `CFCRn` @0x50 + 2n,
//! 16-bit `IDCRn` @0x60 + 2n, `FISRn` @0x70 + 4n, 16-bit `CFTCRn`
//! @0x90 + 2n, `CFSSR0..2` @0xA0..0xA8, `CFSR` @0xAC, `SSICR` @0xB4,
//! `SSIRDR` @0xB8, `CFn_Rw` @0x100 + 0x40n + 4w, `RFn_Rw` @0x300 + 0x40n +
//! 4w. The part is big-endian: a 16-bit register at an even offset is the
//! upper half of the word that contains it, and byte 0 of a word is its most
//! significant byte. This model uses the same lanes.
//!
//! Sources:
//! - `parai/OpenSAR` `arch/ppc/mpc55xx/drivers/mpc5567.h`: `EQADC_tag` (the
//!   offsets above and every field: `CFCR` `SSE` [10], `CFINV` [9], `MODE`
//!   [7:4]; `IDCR` `NCIE`/`TORIE`/`PIE`/`EOQIE`/`CFUIE` [15:11], `CFFE`/`CFFS`
//!   [9:8], `RFOIE` [3], `RFDE`/`RFDS` [1:0]; `FISR` `NCF` 31, `TORF` 30,
//!   `PF` 29, `EOQF` 28, `CFUF` 27, `SSS` 26, `CFFF` 25, `RFOF` 19, `RFDF`
//!   17, `CFCTR` [15:12], `TNXTPTR` [11:8], `RFCTR` [7:4], `POPNXTPTR`
//!   [3:0]; `CFSR` two status bits per CFIFO from bit 31 down) and `EQADC`
//!   at 0xFFF8_0000.
//! - The same tree's `Adc_eQADC.c` (AUTOSAR MCAL for MPC5567/5516/563xM):
//!   conversion command `EOQ` 31, `PAUSE` 30, `BN` 25, `CAL` 24,
//!   `MESSAGE_TAG` [23:20], `LST` [19:18], `TSR` 17, `FMT` 16,
//!   `CHANNEL_NUMBER` [15:8]; configuration command `BN` 25, `R/W` 24,
//!   `ADC_REG` [23:8] (write), `MESSAGE_TAG` [23:20] (read),
//!   `ADC_REG_ADDR` [7:0]; ADC registers `ADC0_CR` 1, `TSCR` 2, `TBCR` 3,
//!   `GCCR` 4, `OCCR` 5; `ADC_ENABLE` 0x8000; the register access idiom
//!   (`CFINV`, push, `MODE` = 1, `SSE`, wait `SSS` clear and `EOQF`, pop
//!   `RFPR`, clear `EOQF`); calibration on channel 44 (25 % of VRH - VRL)
//!   and channel 43 (75 %), `IDEAL_RES25` 0x1000 and `IDEAL_RES75` 0x3000,
//!   `GCC = ((IDEAL_RES75 - IDEAL_RES25) << 14) / (res75 - res25)`,
//!   `OCC = IDEAL_RES75 - ((GCC * res75) >> 14) - 2`, `GCC` 15 bits, `OCC`
//!   14 bits. The ideal results are 14-bit: the 12-bit code shifted left 2.
//! - ChibiOS `os/hal/platforms/SPC5xx/EQADC_v1/adc_lld.h` + `adc_lld.c`:
//!   the same command fields, `MODE` 0 disabled, 1 software single scan,
//!   2..6 hardware single scan, 9 software continuous scan, 10..14 hardware
//!   continuous scan; channels 40 VRH, 41 VRL, 42 50 %, 43 75 %, 44 25 %;
//!   `ADC_CR` `EN` bit 15; `RFIFO` pop is the low half of `RFPRn`
//!   (`RFIFO_POP_ADDR` = `&RFPR + 2`, 16-bit); calibration with
//!   `ADC_IDEAL_RES75_2` 12286 (= 0x3000 - 2); a CFIFO is idle when its
//!   `CFSR` bits read 0.
//!
//! Modelled: a pushed command waits in its CFIFO until the CFIFO is
//! triggered: `MODE` 1 after an `SSE` write (sets `SSS`), or `MODE` 9. Each
//! tick executes one command from the lowest-numbered triggered CFIFO that
//! holds one (CFIFO0 has the highest priority). A triggered, empty CFIFO
//! sets `CFUF`. A command with `EOQ` sets `EOQF`; with `PAUSE` it sets `PF`;
//! either ends a single scan (`SSS` clears, a new `SSE` is needed). Conversion:
//! channels 0..39 (AN0..AN39) convert the routed level, code12 =
//! mV * 4096 / VRH truncated and clamped at 4095, and the raw result is
//! code12 << 2 (14 bits); channels 40..44 return the ideal VRH (0x3FFC),
//! VRL (0), 50 % (0x2000), 75 % (0x3000) and 25 % (0x1000). With `CAL` set
//! the result is `((GCC * raw) >> 14) + OCC + 2` (OCC signed, 14 bits),
//! clamped to 0..0x3FFF, so the OpenSAR / ChibiOS calibration on these
//! ideal references gives GCC 0x4000, OCC -2 and calibrated = raw. Results
//! go to `RFIFO[MESSAGE_TAG]` (tags 0..5); a full RFIFO drops the result
//! and sets `RFOF`. A read configuration command returns the register to
//! the tagged RFIFO; a write stores it. `CFINV` empties the CFIFO. `CFFF` is
//! set while the CFIFO is not full and `RFDF` while the RFIFO is not empty.
//! `CFTCRn` counts executed commands. The IRQ level is any FISR flag
//! enabled in its `IDCR` (`CFFF`/`RFDF` only when `CFFS`/`RFDS` select the
//! interrupt).
//!
//! Not sourced, kept minimal: a command is a configuration command when
//! its bits [7:0] (the register address) are non-zero, else a conversion
//! (the two formats overlap otherwise; on the MPC5567 every ADC register
//! address is non-zero and conversion bits [7:0] are reserved); a
//! conversion on an ADC whose `CR.EN` is 0 returns no result; `FMT` 1
//! (signed) subtracts 0x2000; `TSR` appends a time stamp of 0 (the time
//! base is not modelled); reset values 0 (`GCCR`, `OCCR` included); VRH
//! (`vref_mv`, 5000 by default) is the nominal 5 V reference. Not
//! modelled: hardware triggers (modes 2..6 and 10..14 arm, never fire),
//! DMA requests (`CFFS`/`RFDS` = 1 raise nothing), external buffers (`EB`
//! commands are consumed with no result), the SSI, null messages, `TORF`,
//! `NCF`, conversion time and the `LST` sample time, external mux channels.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::RefCell;

const FIFOS: usize = 6;
const DEPTH: u8 = 4;
/// AN0..AN39.
const CHANNELS: usize = 40;
const CODE_MAX: u32 = (1 << 12) - 1;
const RESULT_MAX: i32 = (1 << 14) - 1;

const MCR: u64 = 0x00;
const NMSFR: u64 = 0x08;
const ETDFR: u64 = 0x0C;
const CFPR0: u64 = 0x10;
const RFPR0: u64 = 0x30;
const CFCR0: u64 = 0x50;
const IDCR0: u64 = 0x60;
const FISR0: u64 = 0x70;
const CFTCR0: u64 = 0x90;
const CFSSR0: u64 = 0xA0;
const CFSSR2: u64 = 0xA8;
const CFSR: u64 = 0xAC;
const SSICR: u64 = 0xB4;
const CF0_R0: u64 = 0x100;
const RF0_R0: u64 = 0x300;
const FIFO_STRIDE: u64 = 0x40;

const CFCR_SSE: u16 = 1 << 10;
const CFCR_CFINV: u16 = 1 << 9;
const CFCR_MODE: u16 = 0xF << 4;
const IDCR_MASK: u16 = 0xFB0B;
const IDCR_CFFE: u16 = 1 << 9;
const IDCR_CFFS: u16 = 1 << 8;
const IDCR_RFDE: u16 = 1 << 1;
const IDCR_RFDS: u16 = 1 << 0;

const FISR_NCF: u32 = 1 << 31;
const FISR_TORF: u32 = 1 << 30;
const FISR_PF: u32 = 1 << 29;
const FISR_EOQF: u32 = 1 << 28;
const FISR_CFUF: u32 = 1 << 27;
const FISR_SSS: u32 = 1 << 26;
const FISR_CFFF: u32 = 1 << 25;
const FISR_RFOF: u32 = 1 << 19;
const FISR_RFDF: u32 = 1 << 17;
/// Flags a write of one clears.
const FISR_W1C: u32 = FISR_NCF | FISR_TORF | FISR_PF | FISR_EOQF | FISR_CFUF | FISR_RFOF;

const CMD_EOQ: u32 = 1 << 31;
const CMD_PAUSE: u32 = 1 << 30;
const CMD_EB: u32 = 1 << 26;
const CMD_BN: u32 = 1 << 25;
/// `CAL` in a conversion command, `R/W` in a configuration command.
const CMD_CAL_RW: u32 = 1 << 24;
const CMD_TSR: u32 = 1 << 17;
const CMD_FMT: u32 = 1 << 16;

const ADC_CR: u8 = 1;
const ADC_TSCR: u8 = 2;
const ADC_TBCR: u8 = 3;
const ADC_GCCR: u8 = 4;
const ADC_OCCR: u8 = 5;
const ADC_CR_EN: u16 = 1 << 15;

/// The CFIFO status `CFSR` reports: 00 idle, 10 waiting for a trigger, 11
/// triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Idle = 0b00,
    Waiting = 0b10,
    Triggered = 0b11,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct Cfifo {
    entries: [u32; DEPTH as usize],
    /// `TNXTPTR`: the entry the next transfer takes.
    next: u8,
    count: u8,
    cfcr: u16,
    idcr: u16,
    /// Sticky `FISR` flags (`NCF`, `TORF`, `PF`, `EOQF`, `CFUF`, `SSS`,
    /// `RFOF`).
    flags: u32,
    cftcr: u16,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct Rfifo {
    entries: [u16; DEPTH as usize],
    /// `POPNXTPTR`: the entry the next pop returns.
    pop: u8,
    count: u8,
}

impl Rfifo {
    fn push(&mut self, value: u16) -> bool {
        if self.count == DEPTH {
            return false;
        }
        self.entries[usize::from((self.pop + self.count) % DEPTH)] = value;
        self.count += 1;
        true
    }

    fn take(&mut self) -> u16 {
        if self.count == 0 {
            return 0;
        }
        let value = self.entries[usize::from(self.pop)];
        self.pop = (self.pop + 1) % DEPTH;
        self.count -= 1;
        value
    }
}

/// The two on-chip converters' configuration registers.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct AdcRegs {
    cr: u16,
    tscr: u16,
    tbcr: u16,
    gccr: u16,
    occr: u16,
}

/// MPC55xx eQADC: CFIFO command queues, two 12-bit converters with gain and
/// offset calibration, RFIFO result queues.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Mpc55xxEqadc {
    mcr: u32,
    nmsfr: u32,
    etdfr: u32,
    ssicr: u32,
    cfifo: [Cfifo; FIFOS],
    /// `RefCell` because a read of `RFPRn` pops an entry.
    rfifo: RefCell<[Rfifo; FIFOS]>,
    adc: [AdcRegs; 2],
    vref_mv: u32,
    /// Routed levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: Vec<u16>,
}

impl Default for Mpc55xxEqadc {
    fn default() -> Self {
        Self::new(5000)
    }
}

impl Mpc55xxEqadc {
    /// `vref_mv`: VRH - VRL, the span the 12-bit code is a fraction of.
    pub fn new(vref_mv: u32) -> Self {
        Self {
            mcr: 0,
            nmsfr: 0,
            etdfr: 0,
            ssicr: 0,
            cfifo: [Cfifo::default(); FIFOS],
            rfifo: RefCell::new([Rfifo::default(); FIFOS]),
            adc: [AdcRegs::default(); 2],
            vref_mv: vref_mv.max(1),
            inputs_mv: vec![u16::MAX; CHANNELS],
        }
    }

    fn mode(&self, n: usize) -> u16 {
        (self.cfifo[n].cfcr & CFCR_MODE) >> 4
    }

    fn status(&self, n: usize) -> Status {
        let sss = self.cfifo[n].flags & FISR_SSS != 0;
        match self.mode(n) {
            1 if sss => Status::Triggered,
            2..=6 if sss => Status::Waiting,
            9 => Status::Triggered,
            10..=14 => Status::Waiting,
            _ => Status::Idle,
        }
    }

    fn single_scan(&self, n: usize) -> bool {
        (1..=6).contains(&self.mode(n))
    }

    /// The 14-bit raw result: the 12-bit code shifted left 2.
    fn raw(&self, channel: u8) -> u16 {
        let code = match channel {
            0..=39 => match self.inputs_mv[usize::from(channel)] {
                u16::MAX => 0,
                mv => (u32::from(mv) * 4096 / self.vref_mv).min(CODE_MAX),
            },
            40 => CODE_MAX,
            41 => 0,
            42 => 2048,
            43 => 3072,
            44 => 1024,
            _ => 0,
        };
        (code << 2) as u16
    }

    fn calibrate(&self, bn: usize, raw: u16) -> u16 {
        let gcc = u32::from(self.adc[bn].gccr & 0x7FFF);
        // OCC is a 14-bit two's complement value.
        let occ = (i32::from(self.adc[bn].occr & 0x3FFF) << 18) >> 18;
        let cal = ((gcc * u32::from(raw)) >> 14) as i32 + occ + 2;
        cal.clamp(0, RESULT_MAX) as u16
    }

    fn adc_reg(&self, bn: usize, addr: u8) -> u16 {
        let adc = &self.adc[bn];
        match addr {
            ADC_CR => adc.cr,
            ADC_TSCR => adc.tscr,
            ADC_TBCR => adc.tbcr,
            ADC_GCCR => adc.gccr,
            ADC_OCCR => adc.occr,
            _ => 0,
        }
    }

    fn set_adc_reg(&mut self, bn: usize, addr: u8, value: u16) {
        let adc = &mut self.adc[bn];
        match addr {
            ADC_CR => adc.cr = value & 0x8BFF,
            ADC_TSCR => adc.tscr = value & 0xF,
            ADC_TBCR => adc.tbcr = value,
            ADC_GCCR => adc.gccr = value & 0x7FFF,
            ADC_OCCR => adc.occr = value & 0x3FFF,
            _ => {}
        }
    }

    /// Tags 0..5 select RFIFO0..5; other tags (null messages) are dropped.
    fn deliver(&mut self, tag: u32, value: u16) {
        let tag = tag as usize;
        if tag >= FIFOS {
            return;
        }
        if !self.rfifo.get_mut()[tag].push(value) {
            self.cfifo[tag].flags |= FISR_RFOF;
        }
    }

    fn execute(&mut self, n: usize, cmd: u32) {
        let bn = usize::from(cmd & CMD_BN != 0);
        let tag = (cmd >> 20) & 0xF;
        if cmd & CMD_EB == 0 {
            let addr = (cmd & 0xFF) as u8;
            if addr != 0 {
                if cmd & CMD_CAL_RW != 0 {
                    let value = self.adc_reg(bn, addr);
                    self.deliver(tag, value);
                } else {
                    self.set_adc_reg(bn, addr, (cmd >> 8) as u16);
                }
            } else if self.adc[bn].cr & ADC_CR_EN != 0 {
                let raw = self.raw((cmd >> 8) as u8);
                let result = if cmd & CMD_CAL_RW != 0 {
                    self.calibrate(bn, raw)
                } else {
                    raw
                };
                let result = if cmd & CMD_FMT != 0 {
                    (i32::from(result) - 0x2000) as i16 as u16
                } else {
                    result
                };
                self.deliver(tag, result);
                if cmd & CMD_TSR != 0 {
                    self.deliver(tag, 0);
                }
            }
        }
        let fifo = &mut self.cfifo[n];
        fifo.cftcr = (fifo.cftcr + 1) & 0x7FF;
        if cmd & (CMD_EOQ | CMD_PAUSE) != 0 {
            fifo.flags |= if cmd & CMD_EOQ != 0 {
                FISR_EOQF
            } else {
                FISR_PF
            };
            if self.single_scan(n) {
                self.cfifo[n].flags &= !FISR_SSS;
            }
        }
    }

    fn fisr(&self, n: usize) -> u32 {
        let c = &self.cfifo[n];
        let r = self.rfifo.borrow()[n];
        let mut value = c.flags;
        if c.count < DEPTH {
            value |= FISR_CFFF;
        }
        if r.count > 0 {
            value |= FISR_RFDF;
        }
        value
            | u32::from(c.count) << 12
            | u32::from(c.next) << 8
            | u32::from(r.count) << 4
            | u32::from(r.pop)
    }

    fn cfsr(&self) -> u32 {
        (0..FIFOS).fold(0, |acc, n| {
            acc | (self.status(n) as u32) << (30 - 2 * n as u32)
        })
    }

    /// 16-bit registers (`CFCR`, `IDCR`, `CFTCR`) by their own offset.
    fn reg16_index(offset: u64) -> Option<(u64, usize)> {
        for base in [CFCR0, IDCR0, CFTCR0] {
            if (base..base + 2 * FIFOS as u64).contains(&offset) {
                return Some((base, ((offset - base) / 2) as usize));
            }
        }
        None
    }

    fn read16(&self, base: u64, n: usize) -> u16 {
        match base {
            // SSE and CFINV are write-only.
            CFCR0 => self.cfifo[n].cfcr,
            IDCR0 => self.cfifo[n].idcr,
            _ => self.cfifo[n].cftcr,
        }
    }

    fn write16(&mut self, base: u64, n: usize, value: u16) {
        match base {
            CFCR0 => {
                let fifo = &mut self.cfifo[n];
                fifo.cfcr = value & CFCR_MODE;
                if fifo.cfcr == 0 {
                    fifo.flags &= !FISR_SSS;
                }
                if value & CFCR_CFINV != 0 {
                    fifo.count = 0;
                    fifo.next = 0;
                }
                if value & CFCR_SSE != 0 && self.single_scan(n) {
                    self.cfifo[n].flags |= FISR_SSS;
                }
            }
            IDCR0 => self.cfifo[n].idcr = value & IDCR_MASK,
            _ => self.cfifo[n].cftcr = value & 0x7FF,
        }
    }

    /// The word at an aligned offset, without side effects (an `RFPRn` read
    /// shows the head entry without popping it).
    fn peek_word(&self, offset: u64) -> u32 {
        if let Some((base, n)) = Self::reg16_index(offset) {
            let hi = u32::from(self.read16(base, n));
            let lo = Self::reg16_index(offset + 2).map_or(0, |(b, m)| u32::from(self.read16(b, m)));
            return hi << 16 | lo;
        }
        let index = |base: u64| ((offset - base) / 4) as usize;
        match offset {
            MCR => self.mcr,
            NMSFR => self.nmsfr,
            ETDFR => self.etdfr,
            o if (RFPR0..RFPR0 + 4 * FIFOS as u64).contains(&o) => {
                let r = self.rfifo.borrow()[index(RFPR0)];
                if r.count == 0 {
                    0
                } else {
                    u32::from(r.entries[usize::from(r.pop)])
                }
            }
            o if (FISR0..FISR0 + 4 * FIFOS as u64).contains(&o) => self.fisr(index(FISR0)),
            o if (CFSSR0..=CFSSR2).contains(&o) => self.cfsr() & 0xFFF0_0000,
            CFSR => self.cfsr(),
            SSICR => self.ssicr,
            o if (CF0_R0..CF0_R0 + FIFO_STRIDE * FIFOS as u64).contains(&o) => {
                let rel = o - CF0_R0;
                let (n, w) = ((rel / FIFO_STRIDE) as usize, rel % FIFO_STRIDE / 4);
                if w < u64::from(DEPTH) {
                    self.cfifo[n].entries[w as usize]
                } else {
                    0
                }
            }
            o if (RF0_R0..RF0_R0 + FIFO_STRIDE * FIFOS as u64).contains(&o) => {
                let rel = o - RF0_R0;
                let (n, w) = ((rel / FIFO_STRIDE) as usize, rel % FIFO_STRIDE / 4);
                if w < u64::from(DEPTH) {
                    u32::from(self.rfifo.borrow()[n].entries[w as usize])
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    fn pop(&self, offset: u64) -> Option<u16> {
        if (RFPR0..RFPR0 + 4 * FIFOS as u64).contains(&offset) {
            let n = ((offset - RFPR0) / 4) as usize;
            Some(self.rfifo.borrow_mut()[n].take())
        } else {
            None
        }
    }

    fn push(&mut self, n: usize, cmd: u32) {
        let fifo = &mut self.cfifo[n];
        if fifo.count == DEPTH {
            return;
        }
        fifo.entries[usize::from((fifo.next + fifo.count) % DEPTH)] = cmd;
        fifo.count += 1;
    }

    /// A 32-bit register write at an aligned offset.
    fn write_word(&mut self, offset: u64, value: u32) {
        if Self::reg16_index(offset).is_some() {
            self.write_half(offset, (value >> 16) as u16);
            self.write_half(offset + 2, value as u16);
            return;
        }
        let index = |base: u64| ((offset - base) / 4) as usize;
        match offset {
            MCR => self.mcr = value & 0x1B,
            NMSFR => self.nmsfr = value & 0x03FF_FFFF,
            ETDFR => self.etdfr = value & 0xF,
            o if (CFPR0..CFPR0 + 4 * FIFOS as u64).contains(&o) => self.push(index(CFPR0), value),
            o if (FISR0..FISR0 + 4 * FIFOS as u64).contains(&o) => {
                self.cfifo[index(FISR0)].flags &= !(value & FISR_W1C);
            }
            SSICR => self.ssicr = value & 0x70F,
            _ => {}
        }
    }

    /// A 16-bit write at an even offset.
    fn write_half(&mut self, offset: u64, value: u16) {
        if let Some((base, n)) = Self::reg16_index(offset) {
            self.write16(base, n, value);
            return;
        }
        let aligned = offset & !3;
        let shift = if offset & 2 == 0 { 16 } else { 0 };
        self.write_lanes(aligned, u32::from(value) << shift, 0xFFFF << shift);
    }

    /// A partial write of a 32-bit register: `lanes` are the bytes written.
    fn write_lanes(&mut self, aligned: u64, value: u32, lanes: u32) {
        match aligned {
            // A push needs the whole command.
            o if (CFPR0..CFPR0 + 4 * FIFOS as u64).contains(&o) => {}
            // Write-one-to-clear: only the written ones clear.
            o if (FISR0..FISR0 + 4 * FIFOS as u64).contains(&o) => {
                self.write_word(aligned, value & lanes)
            }
            _ => {
                let merged = (self.peek_word(aligned) & !lanes) | (value & lanes);
                self.write_word(aligned, merged);
            }
        }
    }

    fn irq_level(&self) -> bool {
        (0..FIFOS).any(|n| {
            let c = &self.cfifo[n];
            let fisr = self.fisr(n);
            let idcr = u32::from(c.idcr);
            // NCIE..CFUIE [15:11] line up with NCF..CFUF [31:27].
            let flags = (fisr >> 16) & idcr & 0xF800 != 0;
            let rfof = fisr & FISR_RFOF != 0 && c.idcr & (1 << 3) != 0;
            let cfff = fisr & FISR_CFFF != 0 && c.idcr & IDCR_CFFE != 0 && c.idcr & IDCR_CFFS == 0;
            let rfdf = fisr & FISR_RFDF != 0 && c.idcr & IDCR_RFDE != 0 && c.idcr & IDCR_RFDS == 0;
            flags || rfof || cfff || rfdf
        })
    }
}

impl Peripheral for Mpc55xxEqadc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if usize::from(channel) >= CHANNELS {
            return false;
        }
        self.inputs_mv[usize::from(channel)] = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        if usize::from(channel) >= CHANNELS {
            return false;
        }
        self.inputs_mv[usize::from(channel)] = u16::MAX;
        true
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts <= 0.0 {
            return false;
        }
        self.vref_mv = ((volts * 1000.0).round() as u32).max(1);
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_level())
    }

    fn on_system_reset(&mut self) {
        let (inputs, vref) = (std::mem::take(&mut self.inputs_mv), self.vref_mv);
        *self = Self::new(vref);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.peek_word(offset & !3) >> (24 - (offset & 3) * 8)) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if let Some((base, n)) = Self::reg16_index(offset & !1) {
            let shift = if offset & 1 == 0 { 8 } else { 0 };
            let current = self.read16(base, n);
            let next = (current & !(0xFF << shift)) | (u16::from(value) << shift);
            self.write16(base, n, next);
            return Ok(());
        }
        let shift = 24 - (offset & 3) * 8;
        self.write_lanes(offset & !3, u32::from(value) << shift, 0xFF << shift);
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        // `RFIFO_POP_ADDR`: the 16-bit pop is the low half of `RFPRn`.
        if offset & 2 != 0 {
            if let Some(value) = self.pop(offset & !3) {
                return Ok(value);
            }
        }
        let word = self.peek_word(offset & !3);
        Ok(if offset & 2 == 0 {
            (word >> 16) as u16
        } else {
            word as u16
        })
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write_half(offset & !1, value);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if let Some(value) = self.pop(offset) {
            return Ok(u32::from(value));
        }
        Ok(self.peek_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        for n in 0..FIFOS {
            if self.status(n) != Status::Triggered {
                continue;
            }
            let fifo = &mut self.cfifo[n];
            if fifo.count == 0 {
                fifo.flags |= FISR_CFUF;
                continue;
            }
            let cmd = fifo.entries[usize::from(fifo.next)];
            fifo.next = (fifo.next + 1) % DEPTH;
            fifo.count -= 1;
            self.execute(n, cmd);
            break;
        }
        PeripheralTickResult {
            irq: self.irq_level(),
            ..PeripheralTickResult::default()
        }
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

    const VREF: u32 = 5000;

    fn eqadc_with(inputs: &[(u8, u16)]) -> Mpc55xxEqadc {
        let mut adc = Mpc55xxEqadc::new(VREF);
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv));
        }
        adc
    }

    fn conv(bn: u32, tag: u32, channel: u32) -> u32 {
        (bn << 25) | (tag << 20) | (3 << 18) | (channel << 8)
    }

    fn write_cmd(bn: u32, reg: u32, value: u32) -> u32 {
        (bn << 25) | (value << 8) | reg
    }

    fn read_cmd(bn: u32, tag: u32, reg: u32) -> u32 {
        (bn << 25) | CMD_CAL_RW | (tag << 20) | reg
    }

    fn fisr(adc: &Mpc55xxEqadc, n: u64) -> u32 {
        adc.read_u32(FISR0 + 4 * n).unwrap()
    }

    /// The OpenSAR `Adc_WriteEQADCRegister` / `Adc_ReadEQADCRegister`
    /// pattern on CFIFO0: CFINV, push, MODE 1 + SSE, wait for SSS to clear
    /// and EOQF, pop, clear EOQF.
    fn run_queue(adc: &mut Mpc55xxEqadc, cmds: &[u32]) {
        adc.write_u16(CFCR0, CFCR_CFINV).unwrap();
        for (i, &cmd) in cmds.iter().enumerate() {
            let eoq = if i + 1 == cmds.len() { CMD_EOQ } else { 0 };
            adc.write_u32(CFPR0, cmd | eoq).unwrap();
        }
        adc.write_u16(CFCR0, 1 << 4).unwrap();
        adc.write_u16(CFCR0, (1 << 4) | CFCR_SSE).unwrap();
        for _ in 0..16 {
            if fisr(adc, 0) & FISR_SSS == 0 {
                break;
            }
            adc.tick();
        }
        let f = fisr(adc, 0);
        assert_eq!(f & (FISR_SSS | FISR_EOQF), FISR_EOQF, "queue ended: {f:#x}");
        adc.write_u32(FISR0, FISR_EOQF).unwrap();
    }

    fn enable_both(adc: &mut Mpc55xxEqadc) {
        run_queue(
            adc,
            &[
                write_cmd(0, 1, 0x8000 | 0x09),
                write_cmd(1, 1, 0x8000 | 0x09),
            ],
        );
    }

    fn code(mv: u32) -> u32 {
        (mv * 4096 / VREF).min(4095)
    }

    #[test]
    fn reset_state_layout_and_lanes() {
        let mut adc = Mpc55xxEqadc::new(VREF);
        for n in 0..6 {
            // CFFF: not full; nothing else.
            assert_eq!(fisr(&adc, n), FISR_CFFF);
        }
        assert_eq!(adc.read_u32(CFSR).unwrap(), 0);
        // CFCR1 is the low half of the word at 0x50 (big-endian lanes).
        adc.write_u16(CFCR0 + 2, 9 << 4).unwrap();
        assert_eq!(adc.read_u16(CFCR0 + 2).unwrap(), 0x90);
        assert_eq!(adc.read_u32(CFCR0).unwrap(), 0x0000_0090);
        assert_eq!(adc.read(CFCR0 + 3).unwrap(), 0x90);
        // CFIFO1 is now triggered (continuous software scan): CFSR bits 29:28.
        assert_eq!(adc.read_u32(CFSR).unwrap(), 0b11 << 28);
        // SSE and CFINV read 0; IDCR keeps only defined bits.
        adc.write_u16(CFCR0, 0xFFFF).unwrap();
        assert_eq!(adc.read_u16(CFCR0).unwrap(), 0x00F0);
        adc.write_u16(IDCR0, 0xFFFF).unwrap();
        assert_eq!(adc.read_u16(IDCR0).unwrap(), IDCR_MASK);
    }

    #[test]
    fn disabled_cfifo_holds_commands_and_cfinv_flushes() {
        let mut adc = eqadc_with(&[(5, 1000)]);
        adc.write_u32(CFPR0, conv(0, 0, 5) | CMD_EOQ).unwrap();
        for _ in 0..4 {
            adc.tick();
        }
        let f = fisr(&adc, 0);
        assert_eq!((f >> 12) & 0xF, 1, "CFCTR: the command waits");
        assert_eq!((f >> 4) & 0xF, 0, "RFCTR: nothing converted");
        assert_eq!(adc.read_u32(CF0_R0).unwrap(), conv(0, 0, 5) | CMD_EOQ);
        adc.write_u16(CFCR0, CFCR_CFINV).unwrap();
        assert_eq!((fisr(&adc, 0) >> 12) & 0xF, 0, "CFINV empties CFIFO0");
        // Four entries, then CFFF drops and a fifth push is lost.
        for _ in 0..5 {
            adc.write_u32(CFPR0, 0).unwrap();
        }
        let f = fisr(&adc, 0);
        assert_eq!(((f >> 12) & 0xF, f & FISR_CFFF), (4, 0));
    }

    #[test]
    fn conversion_needs_an_enabled_adc_then_reads_fourteen_bits() {
        let mut adc = eqadc_with(&[(5, 1234), (39, 4999)]);
        // ADC0_CR.EN = 0 after reset: the command runs, no result.
        run_queue(&mut adc, &[conv(0, 0, 5)]);
        assert_eq!((fisr(&adc, 0) >> 4) & 0xF, 0, "no result while EN = 0");
        enable_both(&mut adc);
        run_queue(&mut adc, &[conv(0, 0, 5), conv(1, 0, 39), conv(0, 0, 6)]);
        assert_eq!((fisr(&adc, 0) >> 4) & 0xF, 3);
        assert_ne!(fisr(&adc, 0) & FISR_RFDF, 0);
        assert_eq!(adc.read_u32(RFPR0).unwrap(), code(1234) << 2);
        // The ChibiOS 16-bit pop at RFPR + 2.
        assert_eq!(u32::from(adc.read_u16(RFPR0 + 2).unwrap()), code(4999) << 2);
        assert_eq!(adc.read_u32(RFPR0).unwrap(), 0, "undriven AN6 reads 0");
        assert_eq!(fisr(&adc, 0) & (FISR_RFDF | 0xFF), 3, "empty, POPNXTPTR 3");
        // Above VRH the code clamps at 4095.
        adc.set_adc_channel_input(5, 6000);
        run_queue(&mut adc, &[conv(0, 0, 5)]);
        assert_eq!(adc.read_u32(RFPR0).unwrap(), 4095 << 2);
    }

    #[test]
    fn message_tag_routes_results_and_overflow_sets_rfof() {
        let mut adc = eqadc_with(&[(1, 2500)]);
        enable_both(&mut adc);
        // Tag 3 lands in RFIFO3; tag 8 (null message) lands nowhere.
        run_queue(&mut adc, &[conv(0, 3, 1), conv(0, 8, 1)]);
        assert_eq!((fisr(&adc, 3) >> 4) & 0xF, 1);
        assert_eq!(adc.read_u32(RFPR0 + 12).unwrap(), code(2500) << 2);
        // Five results into a 4-deep RFIFO2: the fifth drops, RFOF sets.
        run_queue(&mut adc, &[conv(0, 2, 1); 4]);
        assert_eq!(fisr(&adc, 2) & FISR_RFOF, 0);
        run_queue(&mut adc, &[conv(0, 2, 1)]);
        assert_ne!(fisr(&adc, 2) & FISR_RFOF, 0);
        adc.write_u32(FISR0 + 8, FISR_RFOF).unwrap();
        assert_eq!(fisr(&adc, 2) & FISR_RFOF, 0, "RFOF is write-one-to-clear");
    }

    #[test]
    fn opensar_calibration_on_the_ideal_references() {
        let mut adc = eqadc_with(&[(7, 3300)]);
        enable_both(&mut adc);
        run_queue(
            &mut adc,
            &[
                conv(0, 0, 44),
                conv(0, 0, 43),
                conv(0, 0, 40),
                conv(0, 0, 41),
            ],
        );
        let res: Vec<u32> = (0..4).map(|_| adc.read_u32(RFPR0).unwrap()).collect();
        assert_eq!(res, [0x1000, 0x3000, 0x3FFC, 0]);
        let (r25, r75) = (res[0], res[1]);
        let gcc = ((0x3000 - 0x1000) << 14) / (r75 - r25);
        let occ = (0x3000_i32 - ((gcc * r75) >> 14) as i32 - 2) as u32 & 0x3FFF;
        assert_eq!((gcc, occ), (0x4000, 0x3FFE));
        run_queue(&mut adc, &[write_cmd(0, 4, gcc), write_cmd(0, 5, occ)]);
        run_queue(&mut adc, &[read_cmd(0, 0, 4), read_cmd(0, 0, 5)]);
        assert_eq!(adc.read_u32(RFPR0).unwrap(), gcc);
        assert_eq!(adc.read_u32(RFPR0).unwrap(), occ);
        // Ideal calibration: CAL = raw.
        run_queue(&mut adc, &[conv(0, 0, 7) | CMD_CAL_RW, conv(0, 0, 7)]);
        let cal = adc.read_u32(RFPR0).unwrap();
        let raw = adc.read_u32(RFPR0).unwrap();
        assert_eq!((cal, raw), (code(3300) << 2, code(3300) << 2));
        // A 9/8 gain and +8 offset: (raw * 9 / 8) + 8 + 2.
        run_queue(&mut adc, &[write_cmd(0, 4, 0x4800), write_cmd(0, 5, 8)]);
        run_queue(&mut adc, &[conv(0, 0, 7) | CMD_CAL_RW]);
        assert_eq!(adc.read_u32(RFPR0).unwrap(), ((raw * 9) >> 3) + 10);
        // ADC1 keeps its own (reset) calibration.
        run_queue(&mut adc, &[read_cmd(1, 0, 4)]);
        assert_eq!(adc.read_u32(RFPR0).unwrap(), 0);
    }

    #[test]
    fn signed_format_and_time_stamp() {
        let mut adc = eqadc_with(&[(2, 1250)]);
        enable_both(&mut adc);
        run_queue(
            &mut adc,
            &[conv(0, 0, 2) | CMD_FMT, conv(0, 0, 42) | CMD_TSR],
        );
        assert_eq!(
            adc.read_u32(RFPR0).unwrap(),
            u32::from(((code(1250) << 2) as i32 - 0x2000) as i16 as u16)
        );
        assert_eq!(adc.read_u32(RFPR0).unwrap(), 0x2000);
        assert_eq!(adc.read_u32(RFPR0).unwrap(), 0, "the time stamp entry");
    }

    #[test]
    fn continuous_scan_pause_underflow_and_irq() {
        let mut adc = eqadc_with(&[(3, 2000)]);
        enable_both(&mut adc);
        adc.write_u16(IDCR0 + 4, 0x1000 | 0x0800).unwrap(); // EOQIE | CFUIE on CFIFO2
        adc.write_u32(CFPR0 + 8, conv(0, 2, 3) | CMD_PAUSE).unwrap();
        adc.write_u32(CFPR0 + 8, conv(0, 2, 3) | CMD_EOQ).unwrap();
        adc.write_u16(CFCR0 + 4, 9 << 4).unwrap();
        assert_eq!(adc.read_u32(CFSR).unwrap() >> 26 & 0b11, 0b11);
        assert!(!adc.tick().irq, "PAUSE: PF, not enabled");
        assert_ne!(fisr(&adc, 2) & FISR_PF, 0);
        assert!(adc.tick().irq, "EOQ with EOQIE");
        assert_eq!((fisr(&adc, 2) >> 4) & 0xF, 2);
        assert_eq!(adc.read_u16(CFTCR0 + 4).unwrap(), 2, "two transfers");
        adc.write_u32(FISR0 + 8, FISR_EOQF).unwrap();
        // Still triggered, now empty: CFUF.
        assert!(adc.tick().irq, "CFUF with CFUIE");
        assert_ne!(fisr(&adc, 2) & FISR_CFUF, 0);
        adc.write_u16(CFCR0 + 4, 0).unwrap();
        adc.write_u32(FISR0 + 8, FISR_W1C).unwrap();
        assert!(!adc.tick().irq);
        // Hardware-trigger modes wait and never fire.
        adc.write_u32(CFPR0 + 8, conv(0, 2, 3) | CMD_EOQ).unwrap();
        adc.write_u16(CFCR0 + 4, 13 << 4).unwrap();
        adc.tick();
        assert_eq!(adc.read_u32(CFSR).unwrap() >> 26 & 0b11, 0b10);
        assert_eq!((fisr(&adc, 2) >> 12) & 0xF, 1, "command still queued");
    }

    #[test]
    fn single_scan_priority_and_reset_keeps_inputs() {
        let mut adc = eqadc_with(&[(0, 100), (1, 200)]);
        enable_both(&mut adc);
        // CFIFO4 and CFIFO1 both triggered: CFIFO1 goes first.
        adc.write_u32(CFPR0 + 16, conv(0, 4, 0) | CMD_EOQ).unwrap();
        adc.write_u32(CFPR0 + 4, conv(0, 1, 1) | CMD_EOQ).unwrap();
        for n in [4u64, 1] {
            adc.write_u16(CFCR0 + 2 * n, (1 << 4) | CFCR_SSE).unwrap();
        }
        adc.tick();
        assert_eq!(
            ((fisr(&adc, 1) >> 4) & 0xF, (fisr(&adc, 4) >> 4) & 0xF),
            (1, 0)
        );
        assert_eq!(fisr(&adc, 1) & FISR_SSS, 0, "EOQ ends the single scan");
        adc.tick();
        assert_eq!((fisr(&adc, 4) >> 4) & 0xF, 1);
        adc.on_system_reset();
        assert_eq!(adc.adc[0].cr, 0);
        assert_eq!(adc.inputs_mv[1], 200);
    }

    #[test]
    fn routed_reference_rescales() {
        let mut adc = eqadc_with(&[(9, 1650)]);
        assert!(adc.set_adc_reference_voltage_v(3.3));
        enable_both(&mut adc);
        run_queue(&mut adc, &[conv(0, 0, 9)]);
        assert_eq!(adc.read_u32(RFPR0).unwrap(), 2048 << 2);
    }
}
