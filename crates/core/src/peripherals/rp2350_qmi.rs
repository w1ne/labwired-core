// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2350 QSPI memory interface (QMI, datasheet Release 8 section 12.14,
//! base 0x400D0000) in direct serial mode, driving a serial NOR flash on CS0
//! whose array is the bus flash itself (the bytes the CPU fetches through
//! XIP at 0x10000000).
//!
//! Registers (pico-sdk rp2350 `hardware/regs/qmi.h`):
//! - 0x00 DIRECT_CSR: EN (0), BUSY (1, RO), ASSERT_CS0N (2), ASSERT_CS1N (3),
//!   AUTO_CS0N (6), AUTO_CS1N (7), TXFULL/TXEMPTY/TXLEVEL (10/11/12..14),
//!   RXEMPTY/RXFULL/RXLEVEL (16/17/18..20), CLKDIV (22..29), RXDELAY (30..31).
//!   Reset 0x0180_0000 (CLKDIV 6).
//! - 0x04 DIRECT_TX: DATA (0..15), IWIDTH (16..17), DWIDTH (18), OE (19),
//!   NOPUSH (20). A write pushes one entry onto the 4-deep TX FIFO.
//! - 0x08 DIRECT_RX: pops the 4-deep RX FIFO.
//! - 0x0C..0x1C M0_TIMING/RFMT/RCMD/WFMT/WCMD, 0x20..0x30 the M1 twins,
//!   0x34..0x50 ATRANS0..7: stored.
//!
//! Serial NOR command set (Winbond W25Q16JV datasheet section 8, the common
//! JEDEC subset): 03h read, 0Bh fast read, 05h/35h/15h read status 1/2/3,
//! 01h/31h/11h write status, 06h WREN, 04h WRDI, 50h volatile SR write
//! enable, 02h page program (256-byte page wrap), 20h 4 KiB / 52h 32 KiB /
//! D8h 64 KiB sector erase, C7h/60h chip erase, ABh release power-down,
//! B9h power-down, 66h/99h reset, FFh (continuous-read exit). Program and
//! erase need WEL, AND into / fill the array at CS deassert, and hold WIP
//! until the next bus tick commits them (no tPP/tSE durations).
//!
//! Refused (SimulationError::NotImplemented, never silent): dual/quad
//! IWIDTH, AUTO_CSn, CS1 traffic, TX while EN is clear, ATRANS values other
//! than reset, JEDEC/unique-ID reads unless the descriptor declares
//! `jedec_id`/`unique_id`, and any opcode not listed above.
//!
//! The array is the chip's flash window (descriptor `flash`): direct-mode reads, programs and
//! erases go through the same bytes the CPU fetches. The CPU's decoded-
//! instruction cache is not told about those stores, so code that erases or
//! reprograms flash it later executes from is outside this model.
//!
//! Not modelled: shift timing (each FIFO entry completes on the next bus
//! tick), memory-mapped read formats and the XIP cache (XIP reads are the
//! array, whatever M0 says), QPI/continuous-read device modes, write
//! protection bits, suspend/resume.

use crate::{Bus, Peripheral, SimResult, SimulationError};
use std::cell::RefCell;
use std::collections::VecDeque;

const DIRECT_CSR: u64 = 0x00;
const DIRECT_TX: u64 = 0x04;
const DIRECT_RX: u64 = 0x08;
const M0_TIMING: u64 = 0x0C;
const ATRANS0: u64 = 0x34;
const ATRANS7: u64 = 0x50;

const CSR_EN: u32 = 1 << 0;
const CSR_BUSY: u32 = 1 << 1;
const CSR_ASSERT_CS0N: u32 = 1 << 2;
const CSR_ASSERT_CS1N: u32 = 1 << 3;
const CSR_AUTO_CS0N: u32 = 1 << 6;
const CSR_AUTO_CS1N: u32 = 1 << 7;
const CSR_TXFULL: u32 = 1 << 10;
const CSR_TXEMPTY: u32 = 1 << 11;
const CSR_TXLEVEL_LSB: u32 = 12;
const CSR_RXEMPTY: u32 = 1 << 16;
const CSR_RXFULL: u32 = 1 << 17;
const CSR_RXLEVEL_LSB: u32 = 18;
/// EN, ASSERT_CS0N/1N, AUTO_CS0N/1N, CLKDIV, RXDELAY.
const CSR_WRITABLE: u32 = 0xFFC0_00CD;
const CSR_RESET: u32 = 0x0180_0000;

const TX_IWIDTH_MASK: u32 = 3 << 16;
const TX_DWIDTH: u32 = 1 << 18;
const TX_NOPUSH: u32 = 1 << 20;

const FIFO_DEPTH: usize = 4;

/// M0/M1 TIMING, RFMT, RCMD, WFMT, WCMD reset values (qmi.h).
const MX_RESET: [u32; 5] = [
    0x4000_0004,
    0x0000_1000,
    0x0000_A003,
    0x0000_1000,
    0x0000_A002,
];
/// ATRANSn reset: SIZE = 0x400 (4 MiB window), BASE = n * 0x400.
fn atrans_reset(n: u64) -> u32 {
    (0x400 << 16) | (n as u32 * 0x400)
}

const PAGE: u32 = 256;
const SR1_WIP: u8 = 1 << 0;
const SR1_WEL: u8 = 1 << 1;

/// One shifted byte's MISO value, resolved when the bus tick runs.
#[derive(Debug, Clone, Copy)]
enum Miso {
    /// The device drives this value (status registers, IDs, idle 0xFF).
    Value(u8),
    /// The device drives the array byte at this offset.
    Array(u32),
}

#[derive(Debug, Clone, Copy)]
struct TxEntry {
    /// One or two bytes, first shifted first.
    miso: [Miso; 2],
    bytes: u8,
    push: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// CS deasserted.
    Idle,
    /// CS asserted, waiting for the opcode.
    Command,
    /// Collecting address bytes for `op`.
    Address { op: u8, got: u8 },
    /// Fast-read dummy byte.
    Dummy,
    /// Streaming array data from `addr`.
    Read,
    /// Page program data phase.
    Program,
    /// Status register write data phase (index of the next register).
    WriteStatus { reg: u8 },
    /// The device drives `bytes` repeatedly from this source.
    Out { kind: OutKind, index: u8 },
    /// Erase opcode + full address received; waits for CS deassert.
    EraseArmed,
    /// Opcode with no further data (WREN, reset, power-down...): extra
    /// bytes are clocked but ignored by the device.
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutKind {
    Status(u8),
    Jedec,
    UniqueId,
    PowerDownId,
}

#[derive(Debug, Clone)]
enum Commit {
    Program {
        page_base: u32,
        bytes: Vec<Option<u8>>,
    },
    Erase {
        base: u32,
        len: u32,
    },
}

#[derive(Debug)]
pub struct Rp2350Qmi {
    csr: u32,
    mx: [[u32; 5]; 2],
    atrans: [u32; 8],
    tx: VecDeque<TxEntry>,
    rx: RefCell<VecDeque<u16>>,
    // Serial NOR device state.
    /// Where the CPU sees the CS0 array (the chip's flash window).
    flash_base: u64,
    flash_size: u32,
    phase: Phase,
    op: u8,
    addr: u32,
    page: Vec<Option<u8>>,
    status: [u8; 3],
    commits: VecDeque<Commit>,
    /// CS0 was released while shifts were still queued.
    pending_rise: bool,
    jedec_id: Option<[u8; 3]>,
    unique_id: Option<[u8; 8]>,
    programs: u64,
    erases: u64,
}

impl Rp2350Qmi {
    /// `flash_base`/`flash_size`: the CS0 array as the CPU sees it, from the
    /// chip descriptor's `flash` window.
    pub fn new(flash_base: u64, flash_size: u32) -> Self {
        Self {
            csr: CSR_RESET,
            mx: [MX_RESET; 2],
            atrans: std::array::from_fn(|n| atrans_reset(n as u64)),
            tx: VecDeque::with_capacity(FIFO_DEPTH),
            rx: RefCell::new(VecDeque::with_capacity(FIFO_DEPTH)),
            flash_base,
            flash_size,
            phase: Phase::Idle,
            op: 0,
            addr: 0,
            page: Vec::new(),
            status: [0; 3],
            commits: VecDeque::new(),
            pending_rise: false,
            jedec_id: None,
            unique_id: None,
            programs: 0,
            erases: 0,
        }
    }

    /// The device's JEDEC ID (manufacturer, memory type, capacity), answered
    /// to 9Fh. Without it 9Fh is refused rather than invented.
    pub fn with_jedec_id(mut self, id: [u8; 3]) -> Self {
        self.jedec_id = Some(id);
        self
    }

    /// The device's 64-bit unique ID, answered to 4Bh after 4 dummy bytes.
    pub fn with_unique_id(mut self, id: [u8; 8]) -> Self {
        self.unique_id = Some(id);
        self
    }

    fn unsupported(what: impl std::fmt::Display) -> SimulationError {
        SimulationError::NotImplemented(format!("RP2350 QMI: {what}"))
    }

    fn cs0_asserted(&self) -> bool {
        self.csr & CSR_ASSERT_CS0N != 0
    }

    fn busy(&self) -> bool {
        !self.tx.is_empty()
    }

    fn csr_value(&self) -> u32 {
        let rx = self.rx.borrow().len() as u32;
        let tx = self.tx.len() as u32;
        let mut v = self.csr & CSR_WRITABLE;
        if self.busy() {
            v |= CSR_BUSY;
        }
        if tx == 0 {
            v |= CSR_TXEMPTY;
        }
        if tx as usize >= FIFO_DEPTH {
            v |= CSR_TXFULL;
        }
        v |= (tx.min(7)) << CSR_TXLEVEL_LSB;
        if rx == 0 {
            v |= CSR_RXEMPTY;
        }
        if rx as usize >= FIFO_DEPTH {
            v |= CSR_RXFULL;
        }
        v |= (rx.min(7)) << CSR_RXLEVEL_LSB;
        v
    }

    fn write_csr(&mut self, value: u32) -> SimResult<()> {
        if value & (CSR_AUTO_CS0N | CSR_AUTO_CS1N) != 0 {
            return Err(Self::unsupported(
                "DIRECT_CSR.AUTO_CSnN (hardware chip-select timing) is not modelled; \
                 drive ASSERT_CS0N explicitly",
            ));
        }
        if value & CSR_ASSERT_CS1N != 0 {
            return Err(Self::unsupported("CS1 has no device attached"));
        }
        let was = self.cs0_asserted();
        self.csr = value & CSR_WRITABLE;
        let now = self.cs0_asserted();
        if was && !now {
            // Deassert takes effect after the queued shifts complete; the
            // bus tick drains them first (see `tick_with_bus`), so the
            // device sees CS rise only once `tx` is empty.
            if self.tx.is_empty() {
                self.cs_rise();
            } else {
                self.pending_rise = true;
            }
        } else if !was && now {
            self.pending_rise = false;
            self.phase = Phase::Command;
        }
        Ok(())
    }

    /// Feed one MOSI byte to the device; returns what it drives on MISO.
    fn device_byte(&mut self, mosi: u8) -> SimResult<Miso> {
        let busy = self.status[0] & SR1_WIP != 0;
        Ok(match self.phase {
            Phase::Idle => {
                return Err(Self::unsupported(
                    "DIRECT_TX shifted with CS0 deasserted (no device is selected)",
                ))
            }
            Phase::Command => {
                self.op = mosi;
                // While a program/erase is in progress only status reads are
                // accepted (W25Q16JV 8.1: "all instructions except Read
                // Status Register will be ignored").
                if busy && !matches!(mosi, 0x05 | 0x35 | 0x15) {
                    self.phase = Phase::Done;
                    return Ok(Miso::Value(0xFF));
                }
                self.phase = match mosi {
                    0x03 | 0x0B | 0x02 | 0x20 | 0x52 | 0xD8 => Phase::Address { op: mosi, got: 0 },
                    0x05 => Phase::Out {
                        kind: OutKind::Status(0),
                        index: 0,
                    },
                    0x35 => Phase::Out {
                        kind: OutKind::Status(1),
                        index: 0,
                    },
                    0x15 => Phase::Out {
                        kind: OutKind::Status(2),
                        index: 0,
                    },
                    0x01 => Phase::WriteStatus { reg: 0 },
                    0x31 => Phase::WriteStatus { reg: 1 },
                    0x11 => Phase::WriteStatus { reg: 2 },
                    0x06 => {
                        self.status[0] |= SR1_WEL;
                        Phase::Done
                    }
                    0x04 => {
                        self.status[0] &= !SR1_WEL;
                        Phase::Done
                    }
                    0x50 | 0x66 | 0x99 | 0xB9 | 0xFF => Phase::Done,
                    0xC7 | 0x60 => Phase::EraseArmed,
                    0xAB => Phase::Out {
                        kind: OutKind::PowerDownId,
                        index: 0,
                    },
                    0x9F => match self.jedec_id {
                        Some(_) => Phase::Out {
                            kind: OutKind::Jedec,
                            index: 0,
                        },
                        None => return Err(Self::unsupported(
                            "JEDEC ID (9Fh) requested but the flash part declares no `jedec_id`",
                        )),
                    },
                    0x4B => match self.unique_id {
                        Some(_) => Phase::Out {
                            kind: OutKind::UniqueId,
                            index: 0,
                        },
                        None => return Err(Self::unsupported(
                            "unique ID (4Bh) requested but the flash part declares no `unique_id`",
                        )),
                    },
                    other => {
                        return Err(Self::unsupported(format!(
                            "serial NOR opcode {other:#04x} is not modelled"
                        )))
                    }
                };
                Miso::Value(0xFF)
            }
            Phase::Address { op, got } => {
                self.addr = if got == 0 { 0 } else { self.addr };
                self.addr = (self.addr << 8) | u32::from(mosi);
                let got = got + 1;
                self.phase = if got < 3 {
                    Phase::Address { op, got }
                } else {
                    self.addr &= 0x00FF_FFFF;
                    match op {
                        0x03 => Phase::Read,
                        0x0B => Phase::Dummy,
                        0x02 => {
                            self.page = vec![None; PAGE as usize];
                            Phase::Program
                        }
                        _ => Phase::EraseArmed,
                    }
                };
                Miso::Value(0xFF)
            }
            Phase::Dummy => {
                self.phase = Phase::Read;
                Miso::Value(0xFF)
            }
            Phase::Read => {
                let at = self.addr % self.flash_size;
                self.addr = self.addr.wrapping_add(1);
                Miso::Array(at)
            }
            Phase::Program => {
                let col = (self.addr % PAGE) as usize;
                // Page wrap (W25Q16JV 8.2.13): bytes past the page end wrap
                // to its start; the last value written to a column wins.
                self.page[col] = Some(mosi);
                self.addr = (self.addr & !(PAGE - 1)) | ((self.addr + 1) % PAGE);
                Miso::Value(0xFF)
            }
            Phase::WriteStatus { reg } => {
                if self.status[0] & SR1_WEL != 0 && reg < 3 {
                    let keep = if reg == 0 { SR1_WIP | SR1_WEL } else { 0 };
                    self.status[reg as usize] = (self.status[reg as usize] & keep) | (mosi & !keep);
                }
                self.phase = Phase::WriteStatus { reg: reg + 1 };
                Miso::Value(0xFF)
            }
            Phase::Out { kind, index } => {
                self.phase = Phase::Out {
                    kind,
                    index: index.wrapping_add(1),
                };
                Miso::Value(match kind {
                    OutKind::Status(n) => self.status[n as usize],
                    OutKind::Jedec => self.jedec_id.map_or(0xFF, |id| id[(index % 3) as usize]),
                    // 4Bh: four dummy bytes, then the 64-bit ID.
                    OutKind::UniqueId => match index {
                        0..=3 => 0xFF,
                        i if i < 12 => self.unique_id.map_or(0xFF, |id| id[(i - 4) as usize]),
                        _ => 0xFF,
                    },
                    // ABh: three dummy bytes, then the device ID (the low
                    // byte of the JEDEC capacity code minus 1 on Winbond);
                    // unknown without a declared ID.
                    OutKind::PowerDownId => match (index, self.jedec_id) {
                        (0..=2, _) => 0xFF,
                        (_, Some(id)) => id[2].wrapping_sub(1),
                        (_, None) => {
                            return Err(Self::unsupported(
                                "device ID (ABh data phase) requested but the flash part declares no `jedec_id`",
                            ))
                        }
                    },
                })
            }
            Phase::EraseArmed | Phase::Done => Miso::Value(0xFF),
        })
    }

    /// CS0 rising edge: program/erase instructions execute here.
    fn cs_rise(&mut self) {
        let wel = self.status[0] & SR1_WEL != 0;
        match (self.phase, self.op) {
            (Phase::Program, 0x02) if wel => {
                let page = std::mem::take(&mut self.page);
                self.commits.push_back(Commit::Program {
                    page_base: self.addr & !(PAGE - 1),
                    bytes: page,
                });
                self.status[0] |= SR1_WIP;
            }
            (Phase::EraseArmed, op) if wel => {
                let len = match op {
                    0x20 => 4096,
                    0x52 => 32 * 1024,
                    0xD8 => 64 * 1024,
                    _ => self.flash_size,
                };
                let base = if len == self.flash_size {
                    0
                } else {
                    (self.addr % self.flash_size) & !(len - 1)
                };
                self.commits.push_back(Commit::Erase { base, len });
                self.status[0] |= SR1_WIP;
            }
            // WRSR with WEL completes immediately here (no tW modelled) and
            // clears WEL like any write instruction.
            (Phase::WriteStatus { reg }, _) if wel && reg > 0 => {
                self.status[0] &= !SR1_WEL;
            }
            _ => {}
        }
        self.phase = Phase::Idle;
    }

    fn push_tx(&mut self, value: u32) -> SimResult<()> {
        if self.csr & CSR_EN == 0 {
            return Err(Self::unsupported(
                "DIRECT_TX written while DIRECT_CSR.EN is clear",
            ));
        }
        if value & TX_IWIDTH_MASK != 0 {
            return Err(Self::unsupported(
                "dual/quad DIRECT_TX.IWIDTH is not modelled (single-SPI only)",
            ));
        }
        if self.tx.len() >= FIFO_DEPTH {
            // Datasheet 12.14.6: writing a full TX FIFO is a programming
            // error; the hardware drops the entry. Make it visible.
            return Err(Self::unsupported(
                "DIRECT_TX written while the TX FIFO is full",
            ));
        }
        let (bytes, wire) = if value & TX_DWIDTH != 0 {
            (2u8, [(value >> 8) as u8, value as u8])
        } else {
            (1u8, [value as u8, 0])
        };
        let mut miso = [Miso::Value(0xFF); 2];
        for i in 0..bytes as usize {
            miso[i] = self.device_byte(wire[i])?;
        }
        self.tx.push_back(TxEntry {
            miso,
            bytes,
            push: value & TX_NOPUSH == 0,
        });
        Ok(())
    }

    fn commit(&mut self, bus: &mut dyn Bus) {
        while let Some(c) = self.commits.pop_front() {
            match c {
                Commit::Program { page_base, bytes } => {
                    for (i, b) in bytes.iter().enumerate() {
                        if let Some(b) = b {
                            let at = self.flash_base + u64::from(page_base) + i as u64;
                            // NOR program only clears bits.
                            let old = bus.read_u8(at).unwrap_or(0xFF);
                            let _ = bus.write_u8(at, old & b);
                        }
                    }
                    self.programs += 1;
                }
                Commit::Erase { base, len } => {
                    for w in (0..u64::from(len)).step_by(4) {
                        let _ = bus.write_u32(self.flash_base + u64::from(base) + w, 0xFFFF_FFFF);
                    }
                    self.erases += 1;
                }
            }
        }
        self.status[0] &= !(SR1_WIP | SR1_WEL);
    }
}

impl Peripheral for Rp2350Qmi {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            DIRECT_CSR => self.csr_value(),
            DIRECT_TX => 0,
            DIRECT_RX => u32::from(self.rx.borrow_mut().pop_front().unwrap_or(0)),
            o @ M0_TIMING..=0x30 if o & 3 == 0 => {
                let i = ((o - M0_TIMING) / 4) as usize;
                self.mx[i / 5][i % 5]
            }
            o @ ATRANS0..=ATRANS7 if o & 3 == 0 => self.atrans[((o - ATRANS0) / 4) as usize],
            other => {
                return Err(Self::unsupported(format!(
                    "read of unmapped offset {other:#x}"
                )))
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            DIRECT_CSR => self.write_csr(value),
            DIRECT_TX => self.push_tx(value),
            DIRECT_RX => Ok(()),
            o @ M0_TIMING..=0x30 if o & 3 == 0 => {
                let i = ((o - M0_TIMING) / 4) as usize;
                self.mx[i / 5][i % 5] = value;
                Ok(())
            }
            o @ ATRANS0..=ATRANS7 if o & 3 == 0 => {
                let n = (o - ATRANS0) / 4;
                if value != atrans_reset(n) {
                    return Err(Self::unsupported(format!(
                        "ATRANS{n} = {value:#010x}: address translation other than the reset \
                         identity map is not modelled"
                    )));
                }
                self.atrans[n as usize] = value;
                Ok(())
            }
            other => Err(Self::unsupported(format!(
                "write of unmapped offset {other:#x}"
            ))),
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset & !3 == DIRECT_RX {
            return Err(Self::unsupported("sub-word DIRECT_RX read"));
        }
        let word = self.read_u32(offset & !3)?;
        Ok((word >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        if matches!(aligned, DIRECT_TX | DIRECT_RX | DIRECT_CSR) {
            return Err(Self::unsupported(format!(
                "sub-word write to {aligned:#x} (direct-mode registers are word-accessed)"
            )));
        }
        let shift = (offset & 3) * 8;
        let cur = self.read_u32(aligned)?;
        self.write_u32(
            aligned,
            (cur & !(0xFF << shift)) | (u32::from(value) << shift),
        )
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let aligned = offset & !3;
        let word = match aligned {
            DIRECT_RX => u32::from(self.rx.borrow().front().copied().unwrap_or(0)),
            _ => self.read_u32(aligned).ok()?,
        };
        Some((word >> ((offset & 3) * 8)) as u8)
    }

    fn needs_bus_tick(&self) -> bool {
        !self.tx.is_empty() || !self.commits.is_empty() || self.pending_rise
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        // Shift every queued entry the RX FIFO has room for (a full RX FIFO
        // stalls the interface unless the entry is NOPUSH).
        while let Some(entry) = self.tx.front().copied() {
            if entry.push && self.rx.borrow().len() >= FIFO_DEPTH {
                break;
            }
            self.tx.pop_front();
            let mut value: u16 = 0;
            for miso in &entry.miso[..entry.bytes as usize] {
                let byte = match *miso {
                    Miso::Value(v) => v,
                    Miso::Array(at) => bus.read_u8(self.flash_base + u64::from(at)).unwrap_or(0xFF),
                };
                value = (value << 8) | u16::from(byte);
            }
            if entry.push {
                self.rx.borrow_mut().push_back(value);
            }
        }
        if self.pending_rise && self.tx.is_empty() {
            self.pending_rise = false;
            self.cs_rise();
        }
        if !self.commits.is_empty() {
            self.commit(bus);
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "direct_csr": self.csr_value(),
            "m0": self.mx[0],
            "status": self.status,
            "programs": self.programs,
            "erases": self.erases,
            "jedec_id": self.jedec_id,
            "scope": "single-SPI direct mode + serial NOR on CS0; XIP reads are the array (format/cache/timing not modelled)",
        })
    }
}
