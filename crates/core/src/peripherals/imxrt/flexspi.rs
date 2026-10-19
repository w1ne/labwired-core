// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! i.MX RT1060 FlexSPI controller (`0x402A_8000`, IMXRT1060RM §27) with a
//! serial NOR flash on port A1.
//!
//! The NOR array IS the chip's XIP window: the bus `flash` memory mapped at
//! `0x6000_0000`. AHB reads go straight to it; IP commands (the SDK's
//! `FLEXSPI_TransferBlocking`) are executed here, in the bus-aware tick
//! (`tick_with_bus`), and read, program and erase that same array through
//! the bus. Both views therefore always agree.
//!
//! An IP command runs the LUT sequence `IPCR1.ISEQID` (16 sequences of 8
//! instructions, `LUT[0..63]`) against the device: CMD, RADDR, CADDR, MODE,
//! DUMMY, READ, WRITE and STOP instructions are interpreted in SDR and DDR
//! form on any pad count (the pad count and DDR only change the wire time).
//! Read data flows into the 128-byte IP RX FIFO (`RFDR`, released by writing
//! `INTR.IPRXWA`); program data comes from the 128-byte TX FIFO (`TFDR`,
//! pushed by writing `INTR.IPTXWE`). `STS0.SEQIDLE/ARBIDLE` and
//! `INTR.IPCMDDONE` report completion after the command's wire time.
//!
//! The device model is a generic JEDEC serial NOR (Winbond W25Q command set):
//! READ family (any opcode with an address and a READ phase), RDSR1/2/3,
//! WRSR1/2/3, WREN/WRDI, page program (256-byte pages, AND semantics), 4K/32K/
//! 64K/chip erase to 0xFF with WEL gating and WIP busy time, erase/program
//! suspend and resume (`0x75` / `0x7A`, SUS in status register 2). A page
//! program is accepted while an erase is suspended; it does not replace the
//! saved remainder. RDID (JEDEC ID). SFDP is not modelled (reads 0xFF).

use super::{byte_of, Timebase};
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::collections::VecDeque;

const MCR0: u32 = 0x000;
const INTEN: u32 = 0x010;
const INTR: u32 = 0x014;
const LUTKEY: u32 = 0x018;
const LUTCR: u32 = 0x01C;
const IPCR0: u32 = 0x0A0;
const IPCR1: u32 = 0x0A4;
const IPCMD: u32 = 0x0B0;
const IPRXFCR: u32 = 0x0B8;
const IPTXFCR: u32 = 0x0BC;
const DLLCR0: u32 = 0x0C0;
const DLLCR1: u32 = 0x0C4;
const STS0: u32 = 0x0E0;
const STS1: u32 = 0x0E4;
const STS2: u32 = 0x0E8;
const IPRXFSTS: u32 = 0x0F0;
const IPTXFSTS: u32 = 0x0F4;
const RFDR: u32 = 0x100;
const TFDR: u32 = 0x180;
const LUT: u32 = 0x200;

const INTR_IPCMDDONE: u32 = 1 << 0;
const INTR_IPRXWA: u32 = 1 << 5;
const INTR_IPTXWE: u32 = 1 << 6;
const INTR_W1C_MASK: u32 = 0xF1F;

const LUT_KEY: u32 = 0x5AF0_5AF0;
const FIFO_BYTES: usize = 128;

/// Wire time of one byte on a quad SDR bus at ~100 MHz, in core cycles at
/// 600 MHz: 2 SCK periods x 6 core cycles.
const CYCLES_PER_BYTE: u64 = 12;
/// NOR busy times (Winbond W25Q128JV datasheet, typical).
const PAGE_PROGRAM_US: u64 = 400;
const SECTOR_ERASE_US: u64 = 45_000;
const BLOCK32_ERASE_US: u64 = 120_000;
const BLOCK64_ERASE_US: u64 = 150_000;
const CHIP_ERASE_US: u64 = 5_000_000;
const DLL_LOCK_US: u64 = 5;
/// Time from Erase/Program Suspend until WIP clears (Winbond tSUS).
const SUSPEND_US: u64 = 20;

/// The NOR array as the command engine sees it.
pub trait FlashArray {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn get(&mut self, addr: usize) -> u8;
    fn set(&mut self, addr: usize, value: u8);
}

impl FlashArray for Vec<u8> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn get(&mut self, addr: usize) -> u8 {
        self[addr]
    }
    fn set(&mut self, addr: usize, value: u8) {
        self[addr] = value;
    }
}

/// The XIP window reached through the system bus.
struct BusArray<'a> {
    bus: &'a mut dyn crate::Bus,
    base: u64,
    size: usize,
}

impl FlashArray for BusArray<'_> {
    fn len(&self) -> usize {
        self.size
    }
    fn get(&mut self, addr: usize) -> u8 {
        let at = self.base + addr as u64;
        crate::with_device_write_bypass(self.bus, |bus| bus.read_u8(at)).unwrap_or(0xFF)
    }
    fn set(&mut self, addr: usize, value: u8) {
        let at = self.base + addr as u64;
        let _ = crate::with_device_write_bypass(self.bus, |bus| bus.write_u8(at, value));
    }
}

/// One decoded IP transaction (the LUT sequence flattened).
#[derive(Debug, Clone, Default)]
struct Txn {
    cmd: Option<u8>,
    addr: Option<u32>,
    read: bool,
    write: bool,
    wire_bytes: u64,
}

/// One run of identical IP commands in the log: the same sequence, command
/// byte, flash address and data size, executed `count` times in a row. A
/// status poll loop is one entry, not one per poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpCmd {
    pub seq: u8,
    pub cmd: u8,
    pub addr: u32,
    pub size: u32,
    pub count: u64,
}

#[derive(Debug)]
struct Nor {
    sr: [u8; 3],
    wel: bool,
    busy_until: u64,
    /// Cycles of busy time left when an erase or program is suspended.
    remain_cycles: u64,
    suspended: bool,
    jedec: [u8; 3],
}

#[derive(Debug)]
pub struct ImxrtFlexspi {
    regs: Vec<u32>,
    lut: [u32; 64],
    rx: VecDeque<u8>,
    /// Read data still to flow into the RX FIFO.
    rx_pending: VecDeque<u8>,
    tx: VecDeque<u8>,
    /// Program data collected for the in-flight write transaction.
    tx_collected: Vec<u8>,
    /// IP command awaiting its data (write) or its flash access.
    pending: Option<Txn>,
    /// Cycle the in-flight IP command's wire time ends.
    busy_until: u64,
    /// Set by `IPCMD.TRG`: the bus must hand us the array.
    needs_flash: bool,
    intr: u32,
    swreset_until: u64,
    dll_lock_at: [Option<u64>; 2],
    nor: Nor,
    time: Timebase,
    /// Every IP command executed, run-length coded, for inspection.
    ip_log: Vec<IpCmd>,
    /// IP commands executed: the sum of the `ip_log` counts.
    ip_commands: u64,
    xip_base: u64,
    xip_size: usize,
}

impl Default for ImxrtFlexspi {
    fn default() -> Self {
        Self::new([0xEF, 0x40, 0x18], 0x6000_0000, 16 << 20)
    }
}

/// SVD reset values of the plain registers (offset, value).
const RESETS: &[(u32, u32)] = &[
    (0x000, 0xFFFF_80C2), // MCR0
    (0x004, 0xFFFF_FFFF), // MCR1
    (0x008, 0x2000_81F7), // MCR2
    (0x00C, 0x0000_0018), // AHBCR
    (0x018, 0x5AF0_5AF0), // LUTKEY
    (0x01C, 0x0000_0002), // LUTCR
    (0x020, 0x8000_0020), // AHBRXBUF0CR0
    (0x024, 0x8001_0020),
    (0x028, 0x8002_0020),
    (0x02C, 0x8003_0020),
    (0x060, 0x0001_0000), // FLSHA1CR0
    (0x064, 0x0001_0000),
    (0x068, 0x0001_0000),
    (0x06C, 0x0001_0000),
    (0x070, 0x0000_0063), // FLSHCR1A1..B2
    (0x074, 0x0000_0063),
    (0x078, 0x0000_0063),
    (0x07C, 0x0000_0063),
    (0x0C0, 0x0000_0100), // DLLCR0
    (0x0C4, 0x0000_0100), // DLLCR1
];

impl ImxrtFlexspi {
    /// `jedec`: the manufacturer / memory type / capacity bytes RDID (0x9F)
    /// answers with.
    /// `xip_base`/`xip_size`: the AHB window the device array is mapped at.
    pub fn new(jedec: [u8; 3], xip_base: u64, xip_size: usize) -> Self {
        let mut regs = vec![0u32; 0x100 / 4];
        for &(o, v) in RESETS {
            regs[(o / 4) as usize] = v;
        }
        Self {
            regs,
            lut: [0; 64],
            rx: VecDeque::new(),
            rx_pending: VecDeque::new(),
            tx: VecDeque::new(),
            tx_collected: Vec::new(),
            pending: None,
            busy_until: 0,
            needs_flash: false,
            intr: INTR_IPTXWE,
            swreset_until: 0,
            dll_lock_at: [None, None],
            nor: Nor {
                sr: [0, 0x02, 0x60], // SR2.QE = 1 (quad enabled, factory setting on -IQ parts)
                wel: false,
                busy_until: 0,
                remain_cycles: 0,
                suspended: false,
                jedec,
            },
            time: Timebase::default(),
            ip_log: Vec::new(),
            ip_commands: 0,
            xip_base,
            xip_size: xip_size.max(1),
        }
    }

    /// Execute a pending IP command against `array` (the bus does this from
    /// `tick_with_bus`; exposed for unit tests).
    pub fn service(&mut self, array: &mut dyn FlashArray) {
        if self.needs_flash {
            self.needs_flash = false;
            let now = self.time.now();
            self.execute(array, now);
        }
    }

    /// IP commands executed so far, oldest first. Consecutive identical
    /// commands are one entry with a `count`.
    pub fn ip_log(&self) -> &[IpCmd] {
        &self.ip_log
    }

    /// IP commands executed so far (each repeat counted).
    pub fn ip_commands(&self) -> u64 {
        self.ip_commands
    }

    fn record_ip(&mut self, seq: u8, cmd: u8, addr: u32, size: u32) {
        self.ip_commands += 1;
        if let Some(last) = self.ip_log.last_mut() {
            if (last.seq, last.cmd, last.addr, last.size) == (seq, cmd, addr, size) {
                last.count += 1;
                return;
            }
        }
        self.ip_log.push(IpCmd {
            seq,
            cmd,
            addr,
            size,
            count: 1,
        });
    }

    fn reg(&self, off: u32) -> u32 {
        self.regs.get((off / 4) as usize).copied().unwrap_or(0)
    }

    fn rx_watermark(&self) -> usize {
        (((self.reg(IPRXFCR) >> 2) & 0xF) as usize + 1) * 8
    }
    fn tx_watermark(&self) -> usize {
        (((self.reg(IPTXFCR) >> 2) & 0xF) as usize + 1) * 8
    }

    fn refill_rx(&mut self) {
        while self.rx.len() < FIFO_BYTES {
            match self.rx_pending.pop_front() {
                Some(b) => self.rx.push_back(b),
                None => break,
            }
        }
    }

    fn intr_view(&self) -> u32 {
        let mut v = self.intr & !(INTR_IPRXWA | INTR_IPTXWE);
        // IPRXWA: at least a watermark of data (or the tail of the command)
        // is waiting in the RX FIFO.
        if !self.rx.is_empty()
            && (self.rx.len() >= self.rx_watermark()
                || self.rx_pending.is_empty() && !self.ip_busy())
        {
            v |= INTR_IPRXWA;
        }
        if FIFO_BYTES - self.tx.len() >= self.tx_watermark() {
            v |= INTR_IPTXWE;
        }
        if self.time.now() >= self.busy_until && self.pending.is_none() {
            v |= self.intr & INTR_IPCMDDONE;
        } else {
            v &= !INTR_IPCMDDONE;
        }
        v
    }

    fn ip_busy(&self) -> bool {
        self.pending.is_some() || self.time.now() < self.busy_until
    }

    /// Decode LUT sequence `seq` (and the following `num` sequences).
    fn decode(&self, seq: usize, num: usize, sfar: u32, datasz: u32) -> Txn {
        let mut t = Txn::default();
        'outer: for s in seq..=(seq + num).min(15) {
            for k in 0..8 {
                let word = self.lut[s * 4 + k / 2];
                let instr = if k % 2 == 0 {
                    word & 0xFFFF
                } else {
                    word >> 16
                };
                let opcode = (instr >> 10) & 0x3F;
                let operand = (instr & 0xFF) as u8;
                let pads = 1u64 << ((instr >> 8) & 0x3);
                let ddr = opcode & 0x20 != 0;
                let base_op = opcode & !0x20;
                let bits_time = |bits: u64| {
                    let per_clk = pads * if ddr { 2 } else { 1 };
                    bits.div_ceil(per_clk)
                };
                match base_op {
                    0x00 => break 'outer, // STOP
                    0x01 => {
                        if t.cmd.is_none() {
                            t.cmd = Some(operand);
                        }
                        t.wire_bytes += bits_time(8) / 8 + 1;
                    }
                    0x02 | 0x03 => {
                        // RADDR / CADDR: `operand` address bits.
                        t.addr = Some(sfar);
                        t.wire_bytes += bits_time(operand as u64) / 8 + 1;
                    }
                    0x04..=0x07 => t.wire_bytes += 1, // MODE1/2/4/8
                    0x08 => t.write = true,
                    0x09 => t.read = true,
                    0x0C | 0x0D => t.wire_bytes += (operand as u64).div_ceil(8), // DUMMY
                    0x0A | 0x0B => {}                                            // LEARN / DATSZ
                    0x1F => break 'outer,                                        // JMP_ON_CS
                    _ => {}
                }
            }
        }
        if t.read || t.write {
            t.wire_bytes += datasz as u64;
        }
        t
    }

    /// Run the pending transaction against the flash array.
    fn execute(&mut self, array: &mut dyn FlashArray, now: u64) {
        let Some(t) = self.pending.clone() else {
            return;
        };
        let datasz = (self.reg(IPCR1) & 0xFFFF) as usize;
        if t.write && self.tx_collected.len() < datasz {
            // Program data still arriving through the TX FIFO.
            let need = datasz - self.tx_collected.len();
            let take = need.min(self.tx.len());
            self.tx_collected.extend(self.tx.drain(..take));
            if self.tx_collected.len() < datasz {
                return;
            }
        }
        self.pending = None;
        let cmd = t.cmd.unwrap_or(0);
        let size = array.len().max(1);
        let addr = t.addr.unwrap_or(0) as usize % size;
        let nor_busy = now < self.nor.busy_until;
        self.record_ip(
            ((self.reg(IPCR1) >> 16) & 0xF) as u8,
            cmd,
            t.addr.unwrap_or(0),
            datasz as u32,
        );
        let mut out: Vec<u8> = Vec::new();
        match cmd {
            // Status registers.
            0x05 => {
                let sr1 = self.nor.sr[0]
                    | if nor_busy { 1 } else { 0 }
                    | if self.nor.wel { 2 } else { 0 };
                out = vec![sr1; datasz];
            }
            0x35 => {
                let mut sr2 = self.nor.sr[1];
                if self.nor.suspended {
                    sr2 |= 0x80;
                }
                out = vec![sr2; datasz];
            }
            0x15 => out = vec![self.nor.sr[2]; datasz],
            0x9F => {
                out = self
                    .nor
                    .jedec
                    .iter()
                    .copied()
                    .cycle()
                    .take(datasz)
                    .collect();
            }
            0x06 => self.nor.wel = true,
            0x04 => self.nor.wel = false,
            0x50 => {} // volatile SR write enable
            0x01 | 0x31 | 0x11 => {
                if self.nor.wel || cmd != 0x01 {
                    let idx = match cmd {
                        0x01 => 0,
                        0x31 => 1,
                        _ => 2,
                    };
                    for (k, b) in self.tx_collected.iter().enumerate() {
                        if idx + k < 3 {
                            self.nor.sr[idx + k] = *b & if idx + k == 0 { 0xFC } else { 0xFF };
                        }
                    }
                    self.nor.wel = false;
                }
            }
            0x75 => {
                // Erase/Program Suspend. WIP stays set for tSUS, then clears
                // with SUS. A suspend while idle is a no-op.
                if !self.nor.suspended && now < self.nor.busy_until {
                    self.nor.remain_cycles = self.nor.busy_until - now;
                    self.nor.suspended = true;
                    self.nor.busy_until = now + self.time.us(SUSPEND_US);
                }
            }
            0x7A => {
                if self.nor.suspended {
                    self.nor.suspended = false;
                    let left = self.nor.remain_cycles;
                    self.nor.remain_cycles = 0;
                    self.nor.busy_until = now + left;
                }
            }
            0x20 | 0x21 | 0x52 | 0xD8 | 0xDC | 0x60 | 0xC7 => {
                if self.nor.wel && !nor_busy && !self.nor.suspended {
                    let (len, us) = match cmd {
                        0x20 | 0x21 => (4096, SECTOR_ERASE_US),
                        0x52 => (32 * 1024, BLOCK32_ERASE_US),
                        0xD8 | 0xDC => (64 * 1024, BLOCK64_ERASE_US),
                        _ => (size, CHIP_ERASE_US),
                    };
                    let start = if len >= size { 0 } else { addr & !(len - 1) };
                    let end = (start + len).min(size);
                    for a in start..end {
                        array.set(a, 0xFF);
                    }
                    self.nor.busy_until = now + self.time.us(us);
                    self.nor.wel = false;
                }
            }
            0x02 | 0x12 | 0x32 | 0x34 | 0x38 | 0x3E => {
                // Legal while an erase is suspended. The program's own busy
                // time must leave `remain_cycles` and `suspended` alone.
                // Page program: wraps inside the 256-byte page.
                if self.nor.wel && !nor_busy {
                    let page = addr & !0xFF;
                    for (k, b) in self.tx_collected.iter().enumerate() {
                        let a = page + ((addr + k) & 0xFF);
                        if a < size {
                            let old = array.get(a);
                            array.set(a, old & *b);
                        }
                    }
                    self.nor.busy_until = now + self.time.us(PAGE_PROGRAM_US);
                    self.nor.wel = false;
                }
            }
            0x66 | 0x99 | 0xB9 | 0xAB | 0xB7 | 0xE9 => {}
            _ => {
                if t.read && t.addr.is_some() {
                    // READ family (0x03/0x0B/0x3B/0x6B/0xBB/0xEB/0x0C/0xEC/...).
                    out = (0..datasz).map(|k| array.get((addr + k) % size)).collect();
                } else if t.read {
                    out = vec![0xFF; datasz];
                }
            }
        }
        if t.read {
            self.rx_pending.extend(out);
            self.refill_rx();
        }
        self.tx_collected.clear();
        self.busy_until = now + t.wire_bytes * CYCLES_PER_BYTE * self.time.cpu_hz() / 600_000_000;
        self.intr |= INTR_IPCMDDONE;
    }

    pub fn read_reg(&self, off: u32) -> u32 {
        let now = self.time.now();
        match off {
            MCR0 => {
                let v = self.reg(MCR0);
                if now < self.swreset_until {
                    v | 1
                } else {
                    v & !1
                }
            }
            INTR => self.intr_view(),
            STS0 => {
                if self.ip_busy() {
                    0
                } else {
                    0x3 // SEQIDLE | ARBIDLE
                }
            }
            STS1 => 0,
            STS2 => {
                let mut v = 0x0100_0100; // reference/slave delay selections
                if self.dll_lock_at[0].is_some_and(|t| now >= t) {
                    v |= 0x3;
                }
                if self.dll_lock_at[1].is_some_and(|t| now >= t) {
                    v |= 0x3 << 16;
                }
                v
            }
            IPRXFSTS => (self.rx.len() as u32).div_ceil(8),
            IPTXFSTS => (self.tx.len() as u32).div_ceil(8),
            o if (RFDR..RFDR + 0x80).contains(&o) => {
                let k = (o - RFDR) as usize;
                let b = |i: usize| self.rx.get(k + i).copied().unwrap_or(0) as u32;
                b(0) | (b(1) << 8) | (b(2) << 16) | (b(3) << 24)
            }
            o if (TFDR..TFDR + 0x80).contains(&o) => 0,
            o if (LUT..LUT + 0x100).contains(&o) => self.lut[((o - LUT) / 4) as usize],
            o if o < 0x100 => self.reg(o),
            _ => 0,
        }
    }

    pub fn write_reg(&mut self, off: u32, value: u32, mask: u32) {
        let now = self.time.now();
        let v = value & mask;
        let old = self.reg(off);
        let merged = (old & !mask) | v;
        match off {
            MCR0 => {
                self.regs[0] = merged & !1;
                if v & 1 != 0 {
                    // Software reset: FIFOs and the sequence engine.
                    self.rx.clear();
                    self.rx_pending.clear();
                    self.tx.clear();
                    self.pending = None;
                    self.intr = INTR_IPTXWE;
                    self.swreset_until = now + 64;
                }
            }
            INTR => {
                let clr = v & INTR_W1C_MASK;
                self.intr &= !clr;
                if v & INTR_IPRXWA != 0 {
                    // Release a watermark of RX data.
                    let n = self.rx_watermark().min(self.rx.len());
                    self.rx.drain(..n);
                    self.refill_rx();
                }
                if v & INTR_IPTXWE != 0 {
                    // Push the staged TX data (it is already in `tx`).
                    self.needs_flash = self.pending.is_some();
                }
            }
            LUTCR if self.reg(LUTKEY) == LUT_KEY => {
                if v & 0x1 != 0 {
                    self.regs[(LUTCR / 4) as usize] = 0x1;
                } else if v & 0x2 != 0 {
                    self.regs[(LUTCR / 4) as usize] = 0x2;
                }
            }
            IPCMD if v & 1 != 0 && !self.ip_busy() => {
                let r1 = self.reg(IPCR1);
                let seq = ((r1 >> 16) & 0xF) as usize;
                let num = ((r1 >> 24) & 0x7) as usize;
                let sfar = self.reg(IPCR0);
                let sfar = sfar.wrapping_sub(0x6000_0000).min(sfar);
                let txn = self.decode(seq, num, sfar, r1 & 0xFFFF);
                self.pending = Some(txn);
                self.intr &= !INTR_IPCMDDONE;
                self.needs_flash = true;
            }
            IPRXFCR => {
                self.regs[(off / 4) as usize] = merged & !1;
                if v & 1 != 0 {
                    self.rx.clear();
                }
            }
            IPTXFCR => {
                self.regs[(off / 4) as usize] = merged & !1;
                if v & 1 != 0 {
                    self.tx.clear();
                }
            }
            DLLCR0 | DLLCR1 => {
                self.regs[(off / 4) as usize] = merged;
                let i = ((off - DLLCR0) / 4) as usize;
                self.dll_lock_at[i] = if merged & 1 != 0 && merged & 2 == 0 {
                    Some(now + self.time.us(DLL_LOCK_US))
                } else {
                    None
                };
            }
            o if (TFDR..TFDR + 0x80).contains(&o)
                // Word writes stage 4 bytes into the TX FIFO.
                && mask == u32::MAX && self.tx.len() + 4 <= FIFO_BYTES =>
            {
                self.tx.extend(v.to_le_bytes());
            }
            o if (LUT..LUT + 0x100).contains(&o) && self.reg(LUTCR) & 0x2 != 0 => {
                let i = ((o - LUT) / 4) as usize;
                self.lut[i] = (self.lut[i] & !mask) | v;
            }
            STS0 | STS1 | STS2 | IPRXFSTS | IPTXFSTS => {}
            o if o < 0x100 => self.regs[(o / 4) as usize] = merged,
            _ => {}
        }
        let _ = INTEN;
    }

    fn irq(&self) -> bool {
        self.intr_view() & self.reg(INTEN) & 0xF7F != 0
    }
}

impl ImxrtFlexspi {
    /// Recompute the cached interrupt line (see `Timebase::level`).
    fn refresh_irq(&self) {
        self.time.set_level(self.irq());
    }

    fn tick_inner(&mut self, cycles: u64) -> PeripheralTickResult {
        self.time.advance(cycles);
        let now = self.time.now();
        let until = (now < self.busy_until).then_some(self.busy_until);
        super::wake_hint(now, until)
    }
}

impl Peripheral for ImxrtFlexspi {
    /// Walked only while timed work is in flight or the interrupt line is
    /// asserted (so its deassert is reconciled); MMIO re-arms it.
    fn legacy_tick_active(&self) -> bool {
        (self.time.now() < self.busy_until || self.pending.is_some()) || self.time.level()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let v = byte_of(self.read_reg(offset as u32 & !3), offset);
        self.refresh_irq();
        Ok(v)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        self.write_reg(offset as u32 & !3, (value as u32) << shift, 0xFF << shift);
        self.refresh_irq();
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = self.read_reg(offset as u32 & !3);
        self.refresh_irq();
        Ok(v)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset as u32 & !3, value, u32::MAX);
        self.refresh_irq();
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        let r = self.tick_inner(cycles);
        self.refresh_irq();
        r
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.time.level())
    }
    fn needs_bus_tick(&self) -> bool {
        self.needs_flash
    }
    fn tick_with_bus(&mut self, bus: &mut dyn crate::Bus) {
        if self.needs_flash {
            self.needs_flash = false;
            let now = self.time.now();
            let mut array = BusArray {
                bus,
                base: self.xip_base,
                size: self.xip_size,
            };
            self.execute(&mut array, now);
        }
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.time.attach_clock(clock);
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.time.attach_cpu_hz(hz);
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    /// `ip`: one entry per run of identical IP commands,
    /// `seq 1 cmd 0x9f addr 0x00000000 size 3`, with ` x{count}` after it when
    /// the command ran more than once in a row.
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        use crate::peripheral_log::{LogEntry, PeripheralLog};
        let entries = self
            .ip_log
            .iter()
            .map(|c| {
                LogEntry::new(
                    format!(
                        "seq {} cmd {:#04x} addr {:#010x} size {}",
                        c.seq, c.cmd, c.addr, c.size
                    ),
                    c.count,
                )
            })
            .collect();
        vec![PeripheralLog::from_entries("ip", entries)]
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "imxrt_flexspi",
            "ip_commands": self.ip_commands,
            "busy": self.ip_busy(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CycleClock;

    /// LUT instruction pair helper: (opcode, pads, operand).
    fn lut(a: (u32, u32, u32), b: (u32, u32, u32)) -> u32 {
        let one = |(op, pads, operand): (u32, u32, u32)| (op << 10) | (pads << 8) | operand;
        one(a) | (one(b) << 16)
    }

    fn setup() -> (ImxrtFlexspi, CycleClock, Vec<u8>) {
        let mut f = ImxrtFlexspi::default();
        let c = CycleClock::default();
        f.attach_cycle_clock(c.clone());
        let mut array = vec![0xFFu8; 0x10000];
        array[0x1000..0x1008].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        // seq 0: quad read 0xEB: CMD_SDR 0xEB, RADDR_SDR 4 pads 24 bits,
        // DUMMY 6, READ_SDR 4 pads.
        f.write_reg(LUT, lut((0x01, 0, 0xEB), (0x02, 2, 24)), u32::MAX);
        f.write_reg(LUT + 4, lut((0x0C, 2, 6), (0x09, 2, 4)), u32::MAX);
        // seq 1: WREN; seq 2: RDSR; seq 3: sector erase; seq 4: page program
        f.write_reg(LUT + 16, lut((0x01, 0, 0x06), (0, 0, 0)), u32::MAX);
        f.write_reg(LUT + 32, lut((0x01, 0, 0x05), (0x09, 0, 4)), u32::MAX);
        f.write_reg(LUT + 48, lut((0x01, 0, 0x20), (0x02, 0, 24)), u32::MAX);
        f.write_reg(LUT + 64, lut((0x01, 0, 0x02), (0x02, 0, 24)), u32::MAX);
        f.write_reg(LUT + 68, lut((0x08, 0, 4), (0, 0, 0)), u32::MAX);
        (f, c, array)
    }

    fn ip(f: &mut ImxrtFlexspi, array: &mut Vec<u8>, seq: u32, addr: u32, size: u32) {
        f.write_reg(IPCR0, addr, u32::MAX);
        f.write_reg(IPCR1, (seq << 16) | size, u32::MAX);
        f.write_reg(IPCMD, 1, u32::MAX);
        f.service(array);
    }

    #[test]
    fn ip_read_fills_the_rx_fifo_and_completes_after_wire_time() {
        let (mut f, c, mut array) = setup();
        ip(&mut f, &mut array, 0, 0x1000, 8);
        assert_eq!(f.read_reg(STS0) & 1, 0, "sequence busy");
        c.publish(1_000_000);
        assert_eq!(f.read_reg(STS0) & 3, 3);
        assert_ne!(f.read_reg(INTR) & INTR_IPCMDDONE, 0);
        assert_ne!(f.read_reg(INTR) & INTR_IPRXWA, 0);
        assert_eq!(f.read_reg(RFDR), 0x0403_0201);
        assert_eq!(f.read_reg(RFDR + 4), 0x0807_0605);
        f.write_reg(INTR, INTR_IPRXWA, u32::MAX);
        assert_eq!(f.read_reg(IPRXFSTS), 0);
    }

    #[test]
    fn erase_needs_wel_and_reports_wip() {
        let (mut f, c, mut array) = setup();
        ip(&mut f, &mut array, 3, 0x1004, 0);
        assert_eq!(array[0x1000], 1, "no WEL: erase ignored");
        c.publish(10_000);
        ip(&mut f, &mut array, 1, 0, 0); // WREN
        c.publish(20_000);
        ip(&mut f, &mut array, 3, 0x1004, 0);
        assert!(array[0x1000..0x2000].iter().all(|&b| b == 0xFF));
        c.publish(30_000);
        ip(&mut f, &mut array, 2, 0, 1); // RDSR
        assert_eq!(f.read_reg(RFDR) & 1, 1, "WIP while erasing");
        f.write_reg(INTR, INTR_IPRXWA, u32::MAX);
        c.publish(f.time.us(SECTOR_ERASE_US) + 100_000);
        ip(&mut f, &mut array, 2, 0, 1);
        c.publish(f.time.us(SECTOR_ERASE_US) + 200_000);
        assert_eq!(f.read_reg(RFDR) & 3, 0);
    }

    #[test]
    fn erase_suspend_clears_wip_until_resume() {
        let (mut f, c, mut array) = setup();
        // seq 5: suspend 0x75; seq 6: resume 0x7A; seq 7: RDSR2 0x35.
        f.write_reg(LUT + 80, lut((0x01, 0, 0x75), (0, 0, 0)), u32::MAX);
        f.write_reg(LUT + 96, lut((0x01, 0, 0x7A), (0, 0, 0)), u32::MAX);
        f.write_reg(LUT + 112, lut((0x01, 0, 0x35), (0x09, 0, 1)), u32::MAX);
        ip_done(&mut f, &c, &mut array, 1, 0, 0);
        ip_done(&mut f, &c, &mut array, 3, 0x1000, 0);
        ip_done(&mut f, &c, &mut array, 5, 0, 0);
        c.publish(c.now() + f.time.us(SUSPEND_US) + 1_000);
        assert_eq!(
            status(&mut f, &c, &mut array, 2) & 1,
            0,
            "WIP clear while suspended"
        );
        assert_ne!(status(&mut f, &c, &mut array, 7) & 0x80, 0, "SUS set");
        ip_done(&mut f, &c, &mut array, 6, 0, 0);
        assert_eq!(
            status(&mut f, &c, &mut array, 2) & 1,
            1,
            "WIP set again after resume"
        );
        c.publish(c.now() + f.time.us(SECTOR_ERASE_US));
        assert_eq!(
            status(&mut f, &c, &mut array, 2) & 1,
            0,
            "erase finishes after the remaining busy time"
        );
    }

    #[test]
    fn page_program_works_while_erase_is_suspended() {
        let (mut f, c, mut array) = setup();
        f.write_reg(LUT + 80, lut((0x01, 0, 0x75), (0, 0, 0)), u32::MAX);
        f.write_reg(LUT + 96, lut((0x01, 0, 0x7A), (0, 0, 0)), u32::MAX);
        f.write_reg(LUT + 112, lut((0x01, 0, 0x35), (0x09, 0, 1)), u32::MAX);
        ip_done(&mut f, &c, &mut array, 1, 0, 0);
        ip_done(&mut f, &c, &mut array, 3, 0x1000, 0);
        ip_done(&mut f, &c, &mut array, 5, 0, 0);
        c.publish(c.now() + f.time.us(SUSPEND_US) + 1_000);
        ip_done(&mut f, &c, &mut array, 1, 0, 0);
        f.write_reg(IPCR0, 0x2000, u32::MAX);
        f.write_reg(IPCR1, (4 << 16) | 1, u32::MAX);
        f.write_reg(IPCMD, 1, u32::MAX);
        f.service(&mut array);
        f.write_reg(TFDR, 0xA5, u32::MAX);
        f.write_reg(INTR, INTR_IPTXWE, u32::MAX);
        f.service(&mut array);
        assert_eq!(array[0x2000], 0xA5, "program while suspended");
        c.publish(c.now() + f.time.us(PAGE_PROGRAM_US) + 1_000);
        assert_ne!(
            status(&mut f, &c, &mut array, 7) & 0x80,
            0,
            "SUS stays set after the program"
        );
        assert_eq!(
            status(&mut f, &c, &mut array, 2) & 1,
            0,
            "WIP clear after the program"
        );
        ip_done(&mut f, &c, &mut array, 6, 0, 0);
        assert_eq!(status(&mut f, &c, &mut array, 2) & 1, 1, "erase resumes");
    }

    /// One status read. `ip_done` drains the RX FIFO, so a status check has
    /// to capture `RFDR` before that drain.
    fn status(f: &mut ImxrtFlexspi, c: &CycleClock, array: &mut Vec<u8>, seq: u32) -> u32 {
        ip(f, array, seq, 0, 1);
        c.publish(c.now() + 10_000);
        let value = f.read_reg(RFDR);
        f.write_reg(INTR, INTR_IPRXWA, u32::MAX);
        value
    }

    #[test]
    fn ip_log_lines_name_sequence_command_address_and_size() {
        let (mut f, _c, mut array) = setup();
        assert_eq!(f.logs()[0].name, "ip");
        assert!(f.logs()[0].entries.is_empty());
        ip(&mut f, &mut array, 0, 0x1000, 8);
        assert_eq!(
            f.logs()[0].lines(),
            ["seq 0 cmd 0xeb addr 0x00001000 size 8"]
        );
    }

    /// Run one IP command to its end: let its wire time pass, drop its read
    /// data. The next command then finds the controller idle.
    fn ip_done(
        f: &mut ImxrtFlexspi,
        c: &CycleClock,
        array: &mut Vec<u8>,
        seq: u32,
        addr: u32,
        size: u32,
    ) {
        ip(f, array, seq, addr, size);
        c.publish(c.now() + 10_000);
        f.write_reg(INTR, INTR_IPRXWA, u32::MAX);
    }

    /// A status poll loop, as a flash driver runs it while an erase is busy.
    fn poll_loop(f: &mut ImxrtFlexspi, c: &CycleClock, array: &mut Vec<u8>, polls: usize) {
        ip_done(f, c, array, 1, 0, 0); // WREN
        ip_done(f, c, array, 3, 0x1000, 0); // sector erase
        for _ in 0..polls {
            ip_done(f, c, array, 2, 0, 1); // RDSR
        }
        ip_done(f, c, array, 1, 0, 0); // WREN
        ip_done(f, c, array, 3, 0x2000, 0); // next sector
    }

    #[test]
    fn identical_consecutive_commands_are_one_entry() {
        let (mut f, c, mut array) = setup();
        let polls = 100_000;
        poll_loop(&mut f, &c, &mut array, polls);
        let before = polls + 4; // one entry per command without run-length
        let after = f.ip_log().len();
        eprintln!("ip log entries for {polls} polls: {before} before, {after} after");
        assert_eq!(after, 5);
        assert_eq!(f.ip_commands(), before as u64);
        assert_eq!(f.snapshot()["ip_commands"], before as u64);
        let poll = f.ip_log()[2];
        assert_eq!(
            (poll.seq, poll.cmd, poll.size, poll.count),
            (2, 0x05, 1, polls as u64)
        );
        let log = &f.logs()[0];
        assert_eq!(
            log.lines()[2],
            format!("seq 2 cmd 0x05 addr 0x00000000 size 1 x{polls}")
        );
        assert_eq!(log.count_matching("cmd 0x05 "), polls as u64);
        assert_eq!(
            log.count_matching("cmd 0x06 "),
            2,
            "separate runs stay apart"
        );
        assert_eq!(log.count_matching("cmd 0x9f "), 0, "negative control");
    }

    #[test]
    fn different_commands_are_not_merged() {
        let (mut f, c, mut array) = setup();
        ip_done(&mut f, &c, &mut array, 1, 0, 0); // WREN
        ip_done(&mut f, &c, &mut array, 3, 0x1000, 0); // erase 0x1000
        ip_done(&mut f, &c, &mut array, 1, 0, 0); // WREN
        ip_done(&mut f, &c, &mut array, 3, 0x2000, 0); // other address
        ip_done(&mut f, &c, &mut array, 2, 0, 1); // RDSR, size 1
        ip_done(&mut f, &c, &mut array, 2, 0, 2); // RDSR, other size
        let log = f.ip_log();
        assert_eq!(log.len(), 6);
        assert!(log.iter().all(|c| c.count == 1));
        let lines = f.logs()[0].lines();
        assert!(lines.iter().all(|l| !l.contains(" x")), "{lines:?}");
    }

    #[test]
    fn page_program_takes_data_from_the_tx_fifo() {
        let (mut f, c, mut array) = setup();
        ip(&mut f, &mut array, 1, 0, 0); // WREN
        c.publish(10_000);
        f.write_reg(IPCR0, 0x2000, u32::MAX);
        f.write_reg(IPCR1, (4 << 16) | 8, u32::MAX);
        f.write_reg(IPCMD, 1, u32::MAX);
        f.service(&mut array);
        assert_eq!(array[0x2000], 0xFF, "no data yet");
        f.write_reg(TFDR, 0x4433_2211, u32::MAX);
        f.write_reg(TFDR + 4, 0x8877_6655, u32::MAX);
        f.write_reg(INTR, INTR_IPTXWE, u32::MAX);
        f.service(&mut array);
        assert_eq!(
            &array[0x2000..0x2008],
            &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]
        );
    }
}
