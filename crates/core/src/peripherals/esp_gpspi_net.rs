// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The Espressif GP-SPI (`SPI2`/FSPI) on a world `gpio_net`: a bit-level
//! master and slave between chips.
//!
//! Off a net the controller models (`esp32c3::spi`, which the ESP32-C6 also
//! uses) run a whole `SPI_CMD.USR` transaction inside the register write,
//! exchanging bytes with byte-level device models, and narrate the waveform
//! afterwards ([`crate::peripherals::esp_gpspi_wire`]). When the GPIO matrix
//! routes one of the controller's lines to a pad on a net
//! ([`PadLines::on_net`]), the wire belongs to several chips and this engine
//! runs instead. It is chip-neutral: the host model answers the
//! [`GpSpiHost`] calls from its own register file, and every offset and bit
//! below is the same on the ESP32-C3, C6 and S3 GP-SPI (esp32c3 / esp32c6 /
//! esp32s3 PAC `spi2`). The classic ESP32 SPI (HSPI / VSPI) is the older IP
//! with the same fields elsewhere; its model answers through a translated
//! view of its own registers (`esp32::spi`), so nothing here is classic.
//!
//! * **Lines.** SCK (`FSPICLK`), MOSI (`FSPID`), CS (`FSPICS0`) and MISO
//!   (`FSPIQ`), each routed through the matrix in both directions
//!   (`FUNCn_OUT_SEL_CFG` for a line the controller drives, `FUNCn_IN_SEL_CFG`
//!   for one it reads).
//! * **Master** (`SPI_SLAVE.MODE` = 0). `SPI_CMD.USR` starts a transaction at
//!   the cycle of the write: CS0 low (unless `SPI_MISC.CS0_DIS`), then the
//!   enabled phases bit by bit at the `SPI_CLOCK` rate: command
//!   (`SPI_USER2`), address (`SPI_ADDR`, `SPI_USER1.ADDR_BITLEN`), dummy
//!   cycles, then data. Full duplex (`SPI_USER.DOUTDIN`) shifts
//!   `SPI_MS_DLEN + 1` bits out of `W0..W15` while sampling MISO; half duplex
//!   sends a write-data phase (`USR_MOSI`) and then reads a read-data phase
//!   (`USR_MISO`). SCK idles at `SPI_MISC.CK_IDLE_EDGE` (CPOL); CPHA is
//!   `SPI_USER.CK_OUT_EDGE` XOR CPOL, as esp-idf and esp-hal program the four
//!   modes. MISO is sampled at the sampling edge of each bit at the pad, as
//!   the net delivered it there; the bits land in `W0..` (bit order
//!   `SPI_CTRL.RD_BIT_ORDER`), `USR` clears and `SPI_TRANS_DONE` latches
//!   after the CS hold time. CS goes high again unless
//!   `SPI_MISC.CS_KEEP_ACTIVE`.
//! * **Slave** (`SPI_SLAVE.MODE` = 1, CPU-controlled). It has no clock of its
//!   own: it reacts to the CS, SCK and MOSI levels the net delivers. CS low
//!   (`SPI_MISC.SLAVE_CS_POL` inverts) selects it: it takes the bytes to send
//!   from `W0..W15` as they are at that moment and drives MISO until CS goes
//!   high. CPOL is `CK_IDLE_EDGE`, CPHA `SPI_SLAVE.CLK_MODE_13` (esp-idf
//!   `spi_ll_slave_set_mode`). Bits sampled on MOSI are written to `W0..`
//!   when CS goes high, with their count in `SPI_SLAVE1.DATA_BITLEN`,
//!   `SPI_TRANS_DONE` latched and `USR` cleared.
//!
//! * **DMA** (`SPI_DMA_CONF.DMA_TX_ENA` / `DMA_RX_ENA`, both roles). The
//!   data phase's MOSI (master) or MISO (slave) bytes come from what the
//!   GDMA's OUT link pushed into the controller ([`GpSpiHost::take_dma_tx`]),
//!   and the bytes received go to its IN link ([`GpSpiHost::give_dma_rx`])
//!   instead of `W0..W15`; the data phase may then exceed the W buffer's
//!   64 bytes. The transaction completes (`USR` clears, `SPI_TRANS_DONE`)
//!   once the IN link holds the bytes (`crate::peripherals::esp_gdma`).
//!
//! Timed steps run at their exact cycle from a scheduler event; a slave runs
//! when the net delivers an edge. Not modelled on a net: dual/quad/octal
//! modes, segmented (`DMA_SLV_SEG_TRANS_EN`) transfers, the CS polarity bits
//! of a master, `CK_DIS`, and the timing-module delays (`DIN/DOUT_MODE`).

use crate::peripherals::esp_gpspi_wire::{LINE_CS, LINE_MISO, LINE_MOSI, LINE_SCK};
use crate::peripherals::pad_lines::{LineDrive, PadLines};

pub(crate) const REG_CMD: u64 = 0x00;
pub(crate) const REG_ADDR: u64 = 0x04;
pub(crate) const REG_CTRL: u64 = 0x08;
pub(crate) const REG_USER: u64 = 0x10;
pub(crate) const REG_USER1: u64 = 0x14;
pub(crate) const REG_USER2: u64 = 0x18;
pub(crate) const REG_MS_DLEN: u64 = 0x1C;
pub(crate) const REG_MISC: u64 = 0x20;
pub(crate) const REG_W0: u64 = 0x98;
pub(crate) const REG_SLAVE: u64 = 0xE0;
pub(crate) const REG_SLAVE1: u64 = 0xE4;

const CMD_USR: u32 = 1 << 24;
const CTRL_RD_BIT_ORDER: u32 = 1 << 25;
const CTRL_WR_BIT_ORDER: u32 = 1 << 26;
const USER_DOUTDIN: u32 = 1 << 0;
const USER_CS_HOLD: u32 = 1 << 6;
const USER_CS_SETUP: u32 = 1 << 7;
const USER_CK_OUT_EDGE: u32 = 1 << 9;
const USER_USR_MOSI: u32 = 1 << 27;
const USER_USR_MISO: u32 = 1 << 28;
const USER_USR_DUMMY: u32 = 1 << 29;
const USER_USR_ADDR: u32 = 1 << 30;
const USER_USR_COMMAND: u32 = 1 << 31;
const MISC_CS0_DIS: u32 = 1 << 0;
const MISC_SLAVE_CS_POL: u32 = 1 << 23;
const MISC_CK_IDLE_EDGE: u32 = 1 << 29;
const MISC_CS_KEEP_ACTIVE: u32 = 1 << 30;
const SLAVE_CLK_MODE_13: u32 = 1 << 2;
const SLAVE_MODE: u32 = 1 << 26;
/// `SPI_TRANS_DONE` in the `SPI_DMA_INT_*` block.
pub(crate) const INT_TRANS_DONE: u32 = 1 << 12;

/// The 64-byte `W0..W15` buffer, in bits.
const BUFFER_BITS: usize = 64 * 8;

/// Scheduler token for the net engine.
pub(crate) const NET_TOKEN: u32 = 0x5350_0000;

/// What the engine needs from the controller model that hosts it.
pub(crate) trait GpSpiHost {
    /// An architected register as stored.
    fn reg(&self, off: u64) -> u32;
    /// Store a register word (no write side effects).
    fn set_reg(&mut self, off: u64, value: u32);
    /// Latch bits in `SPI_DMA_INT_RAW`.
    fn raise(&mut self, bits: u32);
    /// CPU cycles per SCK period from `SPI_CLOCK`.
    fn bit_time(&self) -> Option<u64>;
    /// `SPI_DMA_CONF.DMA_TX_ENA`: the data phase's MOSI bytes come from the
    /// GDMA (OUT link), not from `W0..W15`. Default: no DMA.
    fn dma_tx(&self) -> bool {
        false
    }
    /// `SPI_DMA_CONF.DMA_RX_ENA`: the bytes received go to the GDMA (IN
    /// link), not into `W0..W15`.
    fn dma_rx(&self) -> bool {
        false
    }
    /// Up to `max` bytes the GDMA has pushed into the transmit FIFO, oldest
    /// first; fewer (the FIFO ran dry) shift out zeros.
    fn take_dma_tx(&mut self, _max: usize) -> Vec<u8> {
        Vec::new()
    }
    /// The transaction's received bytes for the GDMA. The host completes the
    /// transaction (`USR` clear, `TRANS_DONE`) once its IN link has taken
    /// them, so firmware that sees the transaction done finds the data in
    /// memory.
    fn give_dma_rx(&mut self, _bytes: Vec<u8>) {}
}

/// The longest data phase in bits: `SPI_MS_DATA_BITLEN` is 18 bits wide; with
/// the CPU data path the 64-byte W buffer caps it.
const DMA_MAX_BITS: usize = 1 << 18;

/// Pack wire-order bits into bytes.
fn pack_bits(bits: &[bool], lsb_first: bool) -> Vec<u8> {
    let mut out = vec![0u8; bits.len().div_ceil(8)];
    for (i, &bit) in bits.iter().enumerate() {
        let j = (i % 8) as u32;
        let pos = if lsb_first { j } else { 7 - j };
        if bit {
            out[i / 8] |= 1 << pos;
        }
    }
    out
}

/// End a transaction: the received bits go to the GDMA when DMA_RX_ENA (the
/// host completes the transaction once they are delivered), else into
/// `W0..` and the transaction completes now.
fn complete(host: &mut dyn GpSpiHost, rx: &[bool], store_empty: bool) {
    let rd_lsb = host.reg(REG_CTRL) & CTRL_RD_BIT_ORDER != 0;
    if host.dma_rx() {
        host.give_dma_rx(pack_bits(rx, rd_lsb));
        return;
    }
    if store_empty || !rx.is_empty() {
        store_bits(host, rx, rd_lsb);
    }
    host.set_reg(REG_CMD, host.reg(REG_CMD) & !CMD_USR);
    host.raise(INT_TRANS_DONE);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// CS is low; the first leading edge is due.
    Setup,
    Lead,
    Trail,
    /// The last trailing edge is out; CS goes high at the deadline.
    Hold,
}

/// One clock of a master transaction.
#[derive(Debug, Clone, Copy)]
struct Clock {
    /// The MOSI level for this bit, `None`: MOSI keeps its level.
    out: Option<bool>,
    /// MISO is sampled on this bit.
    sample: bool,
}

#[derive(Debug)]
struct MasterXfer {
    clocks: Vec<Clock>,
    rx: Vec<bool>,
    idx: usize,
    step: Step,
    half: u64,
    hold: u64,
    cpol: bool,
    cpha: bool,
    cs: bool,
    keep_cs: bool,
}

#[derive(Debug, Default)]
struct Slave {
    selected: bool,
    sck: bool,
    tx: Vec<bool>,
    rx: Vec<bool>,
}

/// The net engine's state, kept beside the controller's register file.
#[derive(Debug, Default)]
pub(crate) struct GpSpiNet {
    master: Option<MasterXfer>,
    deadline: Option<u64>,
    armed: Vec<u64>,
    slave: Slave,
    /// Slave transactions completed (CS went high after a selection).
    slave_transactions: u64,
    /// Master transactions completed.
    master_transactions: u64,
}

fn bit_of_bytes(bytes: &[u8], i: usize, lsb_first: bool) -> bool {
    let byte = bytes.get(i / 8).copied().unwrap_or(0);
    let j = (i % 8) as u32;
    let pos = if lsb_first { j } else { 7 - j };
    (byte >> pos) & 1 != 0
}

fn w_bytes(host: &dyn GpSpiHost) -> [u8; 64] {
    let mut out = [0u8; 64];
    for (k, b) in out.iter_mut().enumerate() {
        let word = host.reg(REG_W0 + (k as u64 / 4) * 4);
        *b = (word >> ((k % 4) * 8)) as u8;
    }
    out
}

/// Pack `bits` (wire order) into `W0..` from byte 0, leaving the rest of the
/// buffer as it is. A partial last byte keeps its untouched bits.
fn store_bits(host: &mut dyn GpSpiHost, bits: &[bool], lsb_first: bool) {
    let mut bytes = w_bytes(host);
    for (i, &bit) in bits.iter().enumerate().take(BUFFER_BITS) {
        let j = (i % 8) as u32;
        let pos = if lsb_first { j } else { 7 - j };
        let b = &mut bytes[i / 8];
        if bit {
            *b |= 1 << pos;
        } else {
            *b &= !(1 << pos);
        }
    }
    for w in 0..16u64 {
        let k = (w * 4) as usize;
        let word = u32::from_le_bytes([bytes[k], bytes[k + 1], bytes[k + 2], bytes[k + 3]]);
        host.set_reg(REG_W0 + w * 4, word);
    }
}

impl GpSpiNet {
    /// Slave transactions completed (diagnostics and tests).
    pub(crate) fn slave_transactions(&self) -> u64 {
        self.slave_transactions
    }

    /// Master transactions completed (diagnostics and tests).
    pub(crate) fn master_transactions(&self) -> u64 {
        self.master_transactions
    }

    /// The cycle of the next timed step.
    pub(crate) fn deadline(&self) -> Option<u64> {
        self.deadline
    }

    fn is_slave(host: &dyn GpSpiHost) -> bool {
        host.reg(REG_SLAVE) & SLAVE_MODE != 0
    }

    fn cpol(host: &dyn GpSpiHost) -> bool {
        host.reg(REG_MISC) & MISC_CK_IDLE_EDGE != 0
    }

    /// Re-state every line's drive from the registers. Called after every
    /// register write while on a net.
    pub(crate) fn apply_drive(&mut self, host: &mut dyn GpSpiHost, pads: &PadLines) {
        if Self::is_slave(host) {
            if self.master.take().is_some() {
                self.deadline = None;
            }
            pads.set_mode(LINE_SCK, LineDrive::Input);
            pads.set_mode(LINE_MOSI, LineDrive::Input);
            pads.set_mode(LINE_CS, LineDrive::Input);
            self.slave_update_select(host, pads);
            if !self.slave.selected {
                pads.set_mode(LINE_MISO, LineDrive::Input);
            }
            return;
        }
        if self.slave.selected {
            self.slave = Slave::default();
        }
        let sck = if self.master.is_some() {
            pads.level(LINE_SCK)
        } else {
            Self::cpol(host)
        };
        pads.drive_line(LINE_SCK, LineDrive::PushPull, sck);
        pads.set_mode(LINE_MOSI, LineDrive::PushPull);
        let cs = if self.master.is_some() || host.reg(REG_MISC) & MISC_CS_KEEP_ACTIVE != 0 {
            pads.level(LINE_CS)
        } else {
            true
        };
        pads.drive_line(LINE_CS, LineDrive::PushPull, cs);
        pads.set_mode(LINE_MISO, LineDrive::Input);
    }

    // ── master ──────────────────────────────────────────────────────────

    /// `SPI_CMD.USR` written with the controller a master: start the
    /// transaction now. Ignored while one is on the wire.
    pub(crate) fn launch(&mut self, host: &mut dyn GpSpiHost, now: u64, pads: &PadLines) {
        if self.master.is_some() || Self::is_slave(host) {
            return;
        }
        let bit_time = host.bit_time().unwrap_or(2).max(2);
        let half = bit_time / 2;
        let user = host.reg(REG_USER);
        let user1 = host.reg(REG_USER1);
        let ctrl = host.reg(REG_CTRL);
        let misc = host.reg(REG_MISC);
        let wr_lsb = ctrl & CTRL_WR_BIT_ORDER != 0;
        let cpol = misc & MISC_CK_IDLE_EDGE != 0;
        let cpha = (user & USER_CK_OUT_EDGE != 0) != cpol;
        let mut clocks = Vec::new();
        if user & USER_USR_COMMAND != 0 {
            let user2 = host.reg(REG_USER2);
            let n = ((user2 >> 28) & 0xF) as usize + 1;
            let value = user2 & 0xFFFF;
            for i in 0..n {
                let pos = if wr_lsb { i } else { n - 1 - i };
                clocks.push(Clock {
                    out: Some((value >> pos) & 1 != 0),
                    sample: false,
                });
            }
        }
        if user & USER_USR_ADDR != 0 {
            let n = ((user1 >> 27) & 0x1F) as usize + 1;
            let addr = host.reg(REG_ADDR);
            for i in 0..n {
                // The address goes out from bit 31 down (esp-hal shifts it
                // into the top of `SPI_ADDR`).
                clocks.push(Clock {
                    out: Some((addr >> (31 - i)) & 1 != 0),
                    sample: false,
                });
            }
        }
        if user & USER_USR_DUMMY != 0 {
            let n = (user1 & 0xFF) as usize + 1;
            clocks.extend(std::iter::repeat_n(
                Clock {
                    out: None,
                    sample: false,
                },
                n,
            ));
        }
        let dma_tx = host.dma_tx();
        let cap = if dma_tx || host.dma_rx() {
            DMA_MAX_BITS
        } else {
            BUFFER_BITS
        };
        let n = ((host.reg(REG_MS_DLEN) & 0x3_FFFF) as usize + 1).min(cap);
        let tx: Vec<u8> = if dma_tx {
            host.take_dma_tx(n.div_ceil(8))
        } else {
            w_bytes(host).to_vec()
        };
        let mosi = user & USER_USR_MOSI != 0;
        let miso = user & USER_USR_MISO != 0;
        if user & USER_DOUTDIN != 0 {
            for i in 0..n {
                clocks.push(Clock {
                    out: mosi.then(|| bit_of_bytes(&tx, i, wr_lsb)),
                    sample: miso,
                });
            }
        } else {
            if mosi {
                for i in 0..n {
                    clocks.push(Clock {
                        out: Some(bit_of_bytes(&tx, i, wr_lsb)),
                        sample: false,
                    });
                }
            }
            if miso {
                clocks.extend(std::iter::repeat_n(
                    Clock {
                        out: None,
                        sample: true,
                    },
                    n,
                ));
            }
        }
        let setup = if user & USER_CS_SETUP != 0 {
            (u64::from((user1 >> 17) & 0x1F) + 1) * bit_time
        } else {
            half
        };
        let hold = if user & USER_CS_HOLD != 0 {
            half + u64::from((user1 >> 22) & 0x1F) * bit_time
        } else {
            half
        };
        let cs = misc & MISC_CS0_DIS == 0;
        pads.drive_line(LINE_SCK, LineDrive::PushPull, cpol);
        if cs {
            pads.drive_line(LINE_CS, LineDrive::PushPull, false);
        }
        if !cpha {
            if let Some(Some(level)) = clocks.first().map(|c| c.out) {
                pads.drive_line(LINE_MOSI, LineDrive::PushPull, level);
            }
        }
        if clocks.is_empty() {
            // Nothing enabled: CS frames an empty transaction.
            clocks.push(Clock {
                out: None,
                sample: false,
            });
        }
        self.master = Some(MasterXfer {
            clocks,
            rx: Vec::new(),
            idx: 0,
            step: Step::Setup,
            half: half.max(1),
            hold: hold.max(1),
            cpol,
            cpha,
            cs,
            keep_cs: misc & MISC_CS_KEEP_ACTIVE != 0,
        });
        self.deadline = Some(now + setup.max(1));
    }

    fn master_step(&mut self, host: &mut dyn GpSpiHost, now: u64, pads: &PadLines) {
        let Some(mut x) = self.master.take() else {
            self.deadline = None;
            return;
        };
        let sample = |x: &mut MasterXfer| {
            if x.clocks[x.idx].sample {
                x.rx.push(pads.wire_level(LINE_MISO));
            }
        };
        let present = |x: &MasterXfer, idx: usize| {
            if let Some(Some(level)) = x.clocks.get(idx).map(|c| c.out) {
                pads.drive_line(LINE_MOSI, LineDrive::PushPull, level);
            }
        };
        match x.step {
            Step::Setup | Step::Lead => {
                pads.drive_line(LINE_SCK, LineDrive::PushPull, !x.cpol);
                if x.cpha {
                    present(&x, x.idx);
                } else {
                    sample(&mut x);
                }
                x.step = Step::Trail;
                self.deadline = Some(now + x.half);
            }
            Step::Trail => {
                pads.drive_line(LINE_SCK, LineDrive::PushPull, x.cpol);
                if x.cpha {
                    sample(&mut x);
                } else {
                    present(&x, x.idx + 1);
                }
                x.idx += 1;
                if x.idx < x.clocks.len() {
                    x.step = Step::Lead;
                    self.deadline = Some(now + x.half);
                } else {
                    x.step = Step::Hold;
                    self.deadline = Some(now + x.hold);
                }
            }
            Step::Hold => {
                if x.cs && !x.keep_cs {
                    pads.drive_line(LINE_CS, LineDrive::PushPull, true);
                }
                complete(host, &x.rx, false);
                self.master_transactions += 1;
                self.deadline = None;
                return;
            }
        }
        self.master = Some(x);
    }

    /// Run every timed step due by `now`, each at its own cycle.
    pub(crate) fn step(&mut self, host: &mut dyn GpSpiHost, now: u64, pads: &PadLines) {
        while let Some(due) = self.deadline {
            if due > now {
                break;
            }
            self.deadline = None;
            pads.set_stamp(Some(due));
            self.master_step(host, due, pads);
        }
        pads.set_stamp(None);
    }

    // ── slave ───────────────────────────────────────────────────────────

    fn slave_cpha(host: &dyn GpSpiHost) -> bool {
        host.reg(REG_SLAVE) & SLAVE_CLK_MODE_13 != 0
    }

    fn slave_present(&self, host: &dyn GpSpiHost, pads: &PadLines) {
        if !self.slave.selected {
            return;
        }
        let idx = self.slave.rx.len();
        let level = self.slave.tx.get(idx).copied().unwrap_or(false);
        let _ = host;
        pads.drive_line(LINE_MISO, LineDrive::PushPull, level);
    }

    fn slave_update_select(&mut self, host: &mut dyn GpSpiHost, pads: &PadLines) {
        let pol = host.reg(REG_MISC) & MISC_SLAVE_CS_POL != 0;
        let selected = Self::is_slave(host) && pads.input(LINE_CS).map(|l| l != pol) == Some(false);
        if selected == self.slave.selected {
            return;
        }
        if selected {
            let wr_lsb = host.reg(REG_CTRL) & CTRL_WR_BIT_ORDER != 0;
            // DMA: everything the GDMA has queued; else the W buffer.
            let tx: Vec<u8> = if host.dma_tx() {
                host.take_dma_tx(DMA_MAX_BITS / 8)
            } else {
                w_bytes(host).to_vec()
            };
            self.slave = Slave {
                selected: true,
                sck: pads.input(LINE_SCK).unwrap_or(Self::cpol(host)),
                tx: (0..tx.len() * 8)
                    .map(|i| bit_of_bytes(&tx, i, wr_lsb))
                    .collect(),
                rx: Vec::new(),
            };
            pads.set_mode(LINE_MISO, LineDrive::PushPull);
            if !Self::slave_cpha(host) {
                self.slave_present(host, pads);
            }
        } else {
            self.slave.selected = false;
            pads.set_mode(LINE_MISO, LineDrive::Input);
        }
    }

    /// CS went high on a selected slave: the transaction is complete.
    fn slave_finish(&mut self, host: &mut dyn GpSpiHost) {
        let rx = std::mem::take(&mut self.slave.rx);
        let slave1 = host.reg(REG_SLAVE1);
        host.set_reg(
            REG_SLAVE1,
            (slave1 & !0x3_FFFF) | (rx.len() as u32 & 0x3_FFFF),
        );
        complete(host, &rx, true);
        self.slave_transactions += 1;
    }

    /// The net delivered a new level on one of this controller's lines.
    /// Returns `true` when the controller's state changed in a way its host
    /// must act on (an interrupt bit latched).
    pub(crate) fn on_edge(
        &mut self,
        host: &mut dyn GpSpiHost,
        line: usize,
        level: bool,
        pads: &PadLines,
    ) -> bool {
        if !Self::is_slave(host) {
            return false;
        }
        match line {
            LINE_CS => {
                let was = self.slave.selected;
                self.slave_update_select(host, pads);
                if was && !self.slave.selected {
                    self.slave_finish(host);
                    return true;
                }
            }
            LINE_SCK if self.slave.selected && level != self.slave.sck => {
                self.slave.sck = level;
                let leading = level != Self::cpol(host);
                if leading != Self::slave_cpha(host) {
                    // Sampling edge.
                    let cap = if host.dma_rx() {
                        DMA_MAX_BITS
                    } else {
                        BUFFER_BITS
                    };
                    if self.slave.rx.len() < cap {
                        self.slave.rx.push(pads.wire_level(LINE_MOSI));
                    }
                } else {
                    // Shift edge: the next bit goes out.
                    self.slave_present(host, pads);
                }
            }
            _ => {}
        }
        false
    }

    // ── scheduler plumbing ──────────────────────────────────────────────

    /// The engine's next timed step as a scheduler delay from `now`.
    pub(crate) fn take_event(&mut self, now: u64) -> Option<(u64, u32)> {
        let due = self.deadline?.max(now + 1);
        if self.armed.contains(&due) {
            return None;
        }
        self.armed.push(due);
        Some((due - now - 1, NET_TOKEN))
    }

    /// An armed event fired at `now`: run the due steps and say when the
    /// next one is.
    pub(crate) fn on_event(
        &mut self,
        host: &mut dyn GpSpiHost,
        now: u64,
        pads: &PadLines,
    ) -> Option<u64> {
        self.armed.retain(|&t| t > now);
        self.step(host, now, pads);
        let due = self.deadline?.max(now + 1);
        if self.armed.contains(&due) {
            return None;
        }
        self.armed.push(due);
        Some(due - now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::esp_gpspi_wire::SPI_NET_LINES;

    struct Regs {
        r: [u32; 64],
        int_raw: u32,
        bit_time: u64,
    }

    impl GpSpiHost for Regs {
        fn reg(&self, off: u64) -> u32 {
            self.r[(off / 4) as usize]
        }
        fn set_reg(&mut self, off: u64, value: u32) {
            self.r[(off / 4) as usize] = value;
        }
        fn raise(&mut self, bits: u32) {
            self.int_raw |= bits;
        }
        fn bit_time(&self) -> Option<u64> {
            Some(self.bit_time)
        }
    }

    fn regs() -> Regs {
        Regs {
            r: [0; 64],
            int_raw: 0,
            bit_time: 8,
        }
    }

    fn pads() -> PadLines {
        let p = PadLines::new(SPI_NET_LINES, &[false, false, true, false]);
        for l in 0..4 {
            p.mark_on_net(l);
        }
        p
    }

    /// A master in mode 0 against a scripted slave answering `answer` on
    /// MISO, presenting each bit on the trailing edge.
    #[test]
    fn a_master_clocks_the_w_buffer_out_and_samples_miso_into_it() {
        let mut h = regs();
        h.set_reg(REG_USER, USER_DOUTDIN | USER_USR_MOSI | USER_USR_MISO);
        h.set_reg(REG_MS_DLEN, 15);
        h.set_reg(REG_W0, 0x3CA5);
        h.set_reg(REG_CMD, CMD_USR);
        let p = pads();
        let mut net = GpSpiNet::default();
        net.apply_drive(&mut h, &p);
        assert!(p.drives(LINE_SCK) && p.drives(LINE_CS) && !p.drives(LINE_MISO));
        net.launch(&mut h, 0, &p);
        assert!(!p.level(LINE_CS), "CS low for the transaction");
        let answer: u16 = 0x8281;
        let mut mosi = Vec::new();
        let mut now = 0;
        let mut miso_bit = 0usize;
        // Bit 0 of the answer is on MISO before the first edge.
        p.set_input(LINE_MISO, (answer >> 7) & 1 != 0);
        while let Some(due) = net.deadline() {
            now = due;
            let sck_before = p.level(LINE_SCK);
            net.step(&mut h, now, &p);
            if !sck_before && p.level(LINE_SCK) {
                mosi.push(p.level(LINE_MOSI));
            }
            if sck_before && !p.level(LINE_SCK) && miso_bit + 1 < 16 {
                miso_bit += 1;
                let byte = (answer >> (8 * (miso_bit / 8))) as u8;
                p.set_input(LINE_MISO, (byte >> (7 - miso_bit % 8)) & 1 != 0);
            }
        }
        assert!(now > 0);
        let sent: u16 = mosi
            .chunks(8)
            .enumerate()
            .map(|(k, c)| u16::from(c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b))) << (8 * k))
            .sum();
        assert_eq!(sent, 0x3CA5, "W0 went out byte 0 first, MSB first");
        assert_eq!(h.reg(REG_W0) & 0xFFFF, u32::from(answer));
        assert_eq!(h.reg(REG_CMD) & CMD_USR, 0);
        assert_eq!(h.int_raw, INT_TRANS_DONE);
        assert!(p.level(LINE_CS), "CS released");
    }

    #[test]
    fn a_slave_answers_from_the_w_buffer_and_stores_what_it_sampled() {
        let mut h = regs();
        h.set_reg(REG_SLAVE, SLAVE_MODE);
        h.set_reg(REG_W0, 0x81);
        let p = pads();
        let mut net = GpSpiNet::default();
        net.apply_drive(&mut h, &p);
        assert!(!p.drives(LINE_MISO));
        p.set_input(LINE_SCK, false);
        p.set_input(LINE_CS, false);
        net.on_edge(&mut h, LINE_CS, false, &p);
        assert!(p.drives(LINE_MISO));
        let mut got = 0u8;
        for i in (0..8).rev() {
            got = (got << 1) | u8::from(p.level(LINE_MISO));
            p.set_input(LINE_MOSI, (0xA5u8 >> i) & 1 != 0);
            p.set_input(LINE_SCK, true);
            net.on_edge(&mut h, LINE_SCK, true, &p);
            p.set_input(LINE_SCK, false);
            net.on_edge(&mut h, LINE_SCK, false, &p);
        }
        assert_eq!(got, 0x81);
        p.set_input(LINE_CS, true);
        assert!(net.on_edge(&mut h, LINE_CS, true, &p));
        assert!(!p.drives(LINE_MISO));
        assert_eq!(h.reg(REG_W0) & 0xFF, 0xA5);
        assert_eq!(h.reg(REG_SLAVE1) & 0x3_FFFF, 8);
        assert_eq!(h.int_raw, INT_TRANS_DONE);
        assert_eq!(net.slave_transactions(), 1);
    }
}
