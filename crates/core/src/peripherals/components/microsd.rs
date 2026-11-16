// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! microSD card in SPI mode, backed by an in-memory block store.
//!
//! A Physical Layer Simplified Specification v2+ SDHC card (CCS = 1, block
//! addressing, 512-byte blocks) as SPI-mode drivers (SdFat, FatFs `mmc_spi`,
//! raw register code) see it:
//!
//! - Init: CMD0 → R1 0x01 (CRC checked), CMD8 → R7 echoing VHS and the check
//!   pattern (CRC checked), CMD55 + ACMD41 → 0x01 on the first poll, then
//!   0x00 once the host set HCS; CMD58 → R3 with OCR busy/CCS; CMD59 toggles
//!   CRC checking for every other command and for data blocks.
//! - Registers: CMD9 CSD (v2.0), CMD10 CID, ACMD51 SCR, ACMD13 SD status, each
//!   as a data block with token 0xFE and a real CRC16. CMD13 → R2.
//! - Data: CMD17 / CMD18 (stream until CMD12), CMD24 / CMD25 (tokens 0xFE /
//!   0xFC, stop-tran 0xFD), data response 0x05 (0x0B on a CRC error while CRC
//!   is on) followed by busy (0x00) bytes, CMD32/33/38 erase, CMD16, ACMD23.
//! - Before ACMD41 completes, any other command answers R1 with the
//!   illegal-command bit (0x05).
//!
//! A command's response starts one 0xFF filler byte after its CRC byte
//! (NCR = 1, the spec minimum in SPI mode).
//! Blocks never written read back as zeros. Written blocks are exposed as the
//! `sd_card` inspect artifact (LBA list in `meta`, contents in `bytes`) and as
//! [`MicroSd::block`], so a test or oracle can check what firmware stored.
//!
//! Not modelled: SDSC byte addressing, write protect, card-detect, CRC on
//! responses other than data blocks being checked by the host, timing (busy
//! is a fixed number of bytes), and runtime snapshots of the block store.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;
use std::collections::{BTreeMap, VecDeque};

/// Block size in bytes; SDHC fixes it.
pub const BLOCK_LEN: usize = 512;
/// Default capacity, 1 GiB.
const DEFAULT_CAPACITY_MIB: u64 = 1024;
/// Busy bytes (MISO 0x00) after a write or erase. Real cards take
/// microseconds to hundreds of milliseconds; drivers poll until 0xFF.
const BUSY_BYTES: usize = 8;

const R1_IDLE: u8 = 0x01;
const R1_ILLEGAL: u8 = 0x04;
const R1_CRC: u8 = 0x08;
const R1_PARAM: u8 = 0x40;

const TOKEN_SINGLE: u8 = 0xFE;
const TOKEN_MULTI_WRITE: u8 = 0xFC;
const TOKEN_STOP_TRAN: u8 = 0xFD;

/// CRC7 of an SD command frame (polynomial x^7 + x^3 + 1), as the 7-bit value.
pub fn crc7(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in bytes {
        for i in (0..8).rev() {
            let bit = (b >> i) & 1;
            let top = (crc >> 6) & 1;
            crc = (crc << 1) & 0x7F;
            if bit ^ top != 0 {
                crc ^= 0x09;
            }
        }
    }
    crc
}

/// CRC16-CCITT (XMODEM: polynomial 0x1021, init 0) of a data block.
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// What the card does with the next MOSI byte outside command reception.
#[derive(Debug, Clone, PartialEq)]
enum Phase {
    /// Commands only.
    Command,
    /// CMD24 / CMD25 accepted: waiting for a start token.
    WriteToken { lba: u32, multi: bool },
    /// Receiving a block and its CRC.
    WriteData { lba: u32, multi: bool, buf: Vec<u8> },
    /// CMD18: streaming blocks from `next` until CMD12.
    ReadStream { next: u32 },
}

pub struct MicroSd {
    cs_pin: String,
    selected: bool,
    cmd_buf: Vec<u8>,
    out: VecDeque<u8>,
    phase: Phase,
    /// In idle state (after CMD0, until ACMD41 completes).
    idle: bool,
    /// Last command was CMD55.
    app_cmd: bool,
    /// CMD8 seen since CMD0 (a v2 host).
    if_cond: bool,
    /// ACMD41 polls answered since CMD0.
    op_cond_polls: u32,
    crc_on: bool,
    /// Erase range from CMD32 / CMD33.
    erase_start: Option<u32>,
    erase_end: Option<u32>,
    capacity_blocks: u32,
    blocks: BTreeMap<u32, Box<[u8; BLOCK_LEN]>>,
    /// Bumped on every store change, for the inspect artifact.
    generation: u64,
    blocks_written: u64,
    component_id: Option<String>,
}

impl MicroSd {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self::with_capacity_mib(cs_pin, DEFAULT_CAPACITY_MIB)
    }

    /// A card of `mib` MiB (rounded down to a 512 KiB CSD unit, at least one).
    pub fn with_capacity_mib(cs_pin: impl Into<String>, mib: u64) -> Self {
        let units = (mib * 2).clamp(1, 0x40_0000); // 512 KiB units, C_SIZE + 1
        Self {
            cs_pin: cs_pin.into(),
            selected: false,
            cmd_buf: Vec::with_capacity(6),
            out: VecDeque::new(),
            phase: Phase::Command,
            idle: true,
            app_cmd: false,
            if_cond: false,
            op_cond_polls: 0,
            crc_on: false,
            erase_start: None,
            erase_end: None,
            capacity_blocks: (units * 1024) as u32,
            blocks: BTreeMap::new(),
            generation: 0,
            blocks_written: 0,
            component_id: None,
        }
    }

    /// Capacity in 512-byte blocks.
    pub fn capacity_blocks(&self) -> u32 {
        self.capacity_blocks
    }

    /// The stored contents of `lba`, or `None` if no write ever reached it
    /// (it then reads as zeros).
    pub fn block(&self, lba: u32) -> Option<&[u8; BLOCK_LEN]> {
        self.blocks.get(&lba).map(|b| &**b)
    }

    /// Every block written so far, ascending.
    pub fn written_lbas(&self) -> Vec<u32> {
        self.blocks.keys().copied().collect()
    }

    /// Preload a block (a disk image, a test fixture).
    pub fn set_block(&mut self, lba: u32, data: &[u8; BLOCK_LEN]) {
        self.blocks.insert(lba, Box::new(*data));
        self.generation += 1;
    }

    /// True once ACMD41 has taken the card out of idle state.
    pub fn is_ready(&self) -> bool {
        !self.idle
    }

    fn r1(&self, bits: u8) -> u8 {
        bits | if self.idle { R1_IDLE } else { 0 }
    }

    fn read_block(&self, lba: u32) -> [u8; BLOCK_LEN] {
        self.blocks.get(&lba).map_or([0u8; BLOCK_LEN], |b| **b)
    }

    /// Queue a data block: one NAC filler byte, the start token, the payload
    /// and its CRC16.
    fn queue_data_block(&mut self, data: &[u8]) {
        self.out.push_back(0xFF);
        self.out.push_back(TOKEN_SINGLE);
        self.out.extend(data.iter().copied());
        let crc = crc16(data);
        self.out.push_back((crc >> 8) as u8);
        self.out.push_back(crc as u8);
    }

    fn queue_busy(&mut self) {
        self.out.extend(std::iter::repeat_n(0x00, BUSY_BYTES));
    }

    /// CSD version 2.0 (SDHC/SDXC).
    fn csd(&self) -> [u8; 16] {
        let c_size = self.capacity_blocks / 1024 - 1;
        let mut csd = [
            0x40, // CSD_STRUCTURE = 1
            0x0E, // TAAC
            0x00, // NSAC
            0x32, // TRAN_SPEED 25 MHz
            0x5B, // CCC (0x5B5) high
            0x59, // CCC low | READ_BL_LEN = 9
            0x00, // flags
            ((c_size >> 16) & 0x3F) as u8,
            (c_size >> 8) as u8,
            c_size as u8,
            0x7F, // ERASE_BLK_EN, SECTOR_SIZE
            0x80,
            0x0A, // R2W_FACTOR, WRITE_BL_LEN = 9
            0x40,
            0x00,
            0x00,
        ];
        csd[15] = (crc7(&csd[..15]) << 1) | 1;
        csd
    }

    fn cid(&self) -> [u8; 16] {
        let mut cid = [
            0x4C, b'L', b'W', b'L', b'W', b'S', b'D', b'1', 0x10, 0x00, 0x00, 0x00, 0x01, 0x01,
            0x9A, 0x00,
        ];
        cid[15] = (crc7(&cid[..15]) << 1) | 1;
        cid
    }

    fn handle_command(&mut self, frame: [u8; 6]) {
        let index = frame[0] & 0x3F;
        let arg = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
        let crc_ok = (frame[5] >> 1) == crc7(&frame[..5]);
        // A new command ends any data phase and drops unread response bytes.
        // The response follows NCR = 1 filler byte: the byte clocked right
        // after the CRC is always 0xFF (SdFat discards it unread).
        self.out.clear();
        self.out.push_back(0xFF);
        if index == 12 {
            // STOP_TRANSMISSION: a stuff byte, R1, then busy.
            self.phase = Phase::Command;
            self.app_cmd = false;
            self.out.push_back(0xFF);
            self.out.push_back(self.r1(0));
            self.queue_busy();
            return;
        }
        self.phase = Phase::Command;
        // CMD0 and CMD8 are always CRC-checked in SPI mode.
        if (self.crc_on || index == 0 || index == 8) && !crc_ok {
            self.app_cmd = false;
            self.out.push_back(self.r1(R1_CRC));
            return;
        }
        let app = std::mem::take(&mut self.app_cmd);
        if app {
            self.handle_app_command(index, arg);
            return;
        }
        // In idle state only the init commands are legal.
        let init_cmd = matches!(index, 0 | 1 | 8 | 55 | 58 | 59);
        if self.idle && !init_cmd {
            self.out.push_back(self.r1(R1_ILLEGAL));
            return;
        }
        match index {
            0 => {
                self.idle = true;
                self.if_cond = false;
                self.op_cond_polls = 0;
                self.crc_on = false;
                self.out.push_back(R1_IDLE);
            }
            1 => {
                // SEND_OP_COND (MMC init, also accepted by SD cards).
                self.op_cond(true);
            }
            8 => {
                self.if_cond = true;
                let r1 = self.r1(0);
                self.out
                    .extend([r1, 0x00, 0x00, ((arg >> 8) & 0x0F) as u8, arg as u8]);
            }
            9 => {
                self.out.push_back(self.r1(0));
                let csd = self.csd();
                self.queue_data_block(&csd);
            }
            10 => {
                self.out.push_back(self.r1(0));
                let cid = self.cid();
                self.queue_data_block(&cid);
            }
            13 => {
                let r1 = self.r1(0);
                self.out.extend([r1, 0x00]);
            }
            16 => {
                // SDHC block length is fixed at 512; CMD16 is accepted.
                self.out.push_back(self.r1(0));
            }
            17 | 18 => {
                if arg >= self.capacity_blocks {
                    self.out.push_back(self.r1(R1_PARAM));
                    return;
                }
                self.out.push_back(self.r1(0));
                let data = self.read_block(arg);
                self.queue_data_block(&data);
                if index == 18 {
                    self.phase = Phase::ReadStream { next: arg + 1 };
                }
            }
            24 | 25 => {
                if arg >= self.capacity_blocks {
                    self.out.push_back(self.r1(R1_PARAM));
                    return;
                }
                self.out.push_back(self.r1(0));
                self.phase = Phase::WriteToken {
                    lba: arg,
                    multi: index == 25,
                };
            }
            32 => {
                self.erase_start = Some(arg);
                self.out.push_back(self.r1(0));
            }
            33 => {
                self.erase_end = Some(arg);
                self.out.push_back(self.r1(0));
            }
            38 => match (self.erase_start.take(), self.erase_end.take()) {
                (Some(s), Some(e)) if s <= e => {
                    let doomed: Vec<u32> = self.blocks.range(s..=e).map(|(k, _)| *k).collect();
                    for lba in doomed {
                        self.blocks.remove(&lba);
                    }
                    self.generation += 1;
                    self.out.push_back(self.r1(0));
                    self.queue_busy();
                }
                _ => self.out.push_back(self.r1(0x10)), // erase sequence error
            },
            55 => {
                self.app_cmd = true;
                self.out.push_back(self.r1(0));
            }
            58 => {
                let ocr: u32 = if self.idle { 0x00FF_8000 } else { 0xC0FF_8000 };
                let r1 = self.r1(0);
                self.out.push_back(r1);
                self.out.extend(ocr.to_be_bytes());
            }
            59 => {
                self.crc_on = arg & 1 != 0;
                self.out.push_back(self.r1(0));
            }
            _ => self.out.push_back(self.r1(R1_ILLEGAL)),
        }
    }

    /// ACMD41 / CMD1: the first poll after CMD0 still reports idle, like a
    /// card finishing power-up; then the card is ready if the host can take
    /// it (an SDHC card stays idle for a host that sent no CMD8 or no HCS).
    fn op_cond(&mut self, hcs: bool) {
        self.op_cond_polls += 1;
        if self.op_cond_polls > 1 && self.if_cond && hcs {
            self.idle = false;
        }
        self.out.push_back(self.r1(0));
    }

    fn handle_app_command(&mut self, index: u8, arg: u32) {
        match index {
            41 => self.op_cond(arg & 0x4000_0000 != 0),
            _ if self.idle => self.out.push_back(self.r1(R1_ILLEGAL)),
            13 => {
                let r1 = self.r1(0);
                self.out.extend([r1, 0x00]);
                self.queue_data_block(&[0u8; 64]);
            }
            22 => {
                self.out.push_back(self.r1(0));
                let n = (self.blocks_written as u32).to_be_bytes();
                self.queue_data_block(&n);
            }
            23 => self.out.push_back(self.r1(0)),
            51 => {
                self.out.push_back(self.r1(0));
                // SCR: SD spec 3.0x, 1- and 4-bit bus, data erased to 0.
                self.queue_data_block(&[0x02, 0x35, 0x80, 0x03, 0, 0, 0, 0]);
            }
            _ => self.out.push_back(self.r1(R1_ILLEGAL)),
        }
    }

    /// A MOSI byte while the card waits for or receives a write block.
    /// Returns `true` when the byte belonged to the data phase.
    fn write_byte(&mut self, mosi: u8) -> bool {
        match &mut self.phase {
            Phase::WriteToken { lba, multi } => {
                let (lba, multi) = (*lba, *multi);
                match mosi {
                    TOKEN_SINGLE if !multi => {}
                    TOKEN_MULTI_WRITE if multi => {}
                    TOKEN_STOP_TRAN if multi => {
                        self.phase = Phase::Command;
                        // One byte before busy (Nbr), then busy.
                        self.out.push_back(0xFF);
                        self.queue_busy();
                        return true;
                    }
                    // Fillers, or a command (the caller handles it).
                    _ => return mosi == 0xFF,
                }
                self.phase = Phase::WriteData {
                    lba,
                    multi,
                    buf: Vec::with_capacity(BLOCK_LEN + 2),
                };
                true
            }
            Phase::WriteData { lba, multi, buf } => {
                buf.push(mosi);
                if buf.len() < BLOCK_LEN + 2 {
                    return true;
                }
                let (lba, multi) = (*lba, *multi);
                let crc = u16::from_be_bytes([buf[BLOCK_LEN], buf[BLOCK_LEN + 1]]);
                let mut data = [0u8; BLOCK_LEN];
                data.copy_from_slice(&buf[..BLOCK_LEN]);
                if self.crc_on && crc != crc16(&data) {
                    self.out.push_back(0x0B); // data rejected: CRC error
                    self.phase = Phase::Command;
                    return true;
                }
                self.blocks.insert(lba, Box::new(data));
                self.blocks_written += 1;
                self.generation += 1;
                self.out.push_back(0x05); // data accepted
                self.queue_busy();
                self.phase = if multi && lba + 1 < self.capacity_blocks {
                    Phase::WriteToken {
                        lba: lba + 1,
                        multi: true,
                    }
                } else {
                    Phase::Command
                };
                true
            }
            _ => false,
        }
    }
}

impl SpiDevice for MicroSd {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.selected = true;
        self.cmd_buf.clear();
    }

    fn cs_release(&mut self) {
        self.selected = false;
        self.cmd_buf.clear();
        // Unread response bytes and busy are gone once the host lets go;
        // a write in progress keeps its phase (the host resumes it).
        self.out.clear();
    }

    fn is_selected(&self) -> bool {
        self.selected
    }

    /// The head of the response queue: `transfer` pops it before looking at
    /// MOSI, as the card's output does not depend on the byte coming in.
    fn next_miso(&self) -> Option<u8> {
        Some(self.out.front().copied().unwrap_or(0xFF))
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        let miso = self.out.pop_front().unwrap_or(0xFF);

        if !self.cmd_buf.is_empty() {
            self.cmd_buf.push(mosi);
            if self.cmd_buf.len() == 6 {
                let mut frame = [0u8; 6];
                frame.copy_from_slice(&self.cmd_buf);
                self.cmd_buf.clear();
                self.handle_command(frame);
            }
            return miso;
        }

        if self.write_byte(mosi) {
            return miso;
        }

        if mosi & 0xC0 == 0x40 {
            // Start of a command frame. During a read stream this is CMD12
            // arriving while data still flows.
            self.cmd_buf.push(mosi);
            return miso;
        }

        // A read stream refills when the host has drained a block.
        if let Phase::ReadStream { next } = self.phase {
            if self.out.is_empty() {
                if next < self.capacity_blocks {
                    let data = self.read_block(next);
                    self.queue_data_block(&data);
                    self.phase = Phase::ReadStream { next: next + 1 };
                } else {
                    self.phase = Phase::Command;
                }
            }
        }
        miso
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        let lbas = self.written_lbas();
        let bytes = opts.include_bytes.then(|| {
            let mut v = Vec::with_capacity(lbas.len() * BLOCK_LEN);
            for b in self.blocks.values() {
                v.extend_from_slice(&b[..]);
            }
            v
        });
        vec![crate::inspect::Artifact {
            kind: "sd_card".into(),
            id: id.into(),
            meta: serde_json::json!({
                "block_size": BLOCK_LEN,
                "capacity_blocks": self.capacity_blocks,
                "ready": !self.idle,
                "written_lbas": lbas,
                "blocks_written": self.blocks_written,
                "generation": self.generation,
                "bytes_layout": "written_lbas order, block_size bytes each",
            }),
            bytes,
        }]
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct MicroSdKit;
pub static MICROSD_KIT: MicroSdKit = MicroSdKit;

static MICROSD_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(&[]),
    device_type: std::borrow::Cow::Borrowed("microsd"),
    label: std::borrow::Cow::Borrowed("microSD (SPI)"),
    summary: std::borrow::Cow::Borrowed(
        "SPI-mode SDHC card with an in-memory block store (init, CSD/CID, single and multi-block read/write).",
    ),
    detail: std::borrow::Cow::Borrowed(
        "CMD0/8/55+ACMD41/58/59/9/10/13/16/17/18/12/24/25/32/33/38 with R1/R2/R3/R7 framing, \
             data tokens, data-response 0x05 and busy. Blocks never written read as zeros; \
             written blocks are exposed in the `sd_card` inspect artifact. No filesystem is \
             pre-formatted and no SDSC byte addressing.",
    ),
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: std::borrow::Cow::Borrowed(&[
        ConfigKey {
            name: std::borrow::Cow::Borrowed("cs_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("Card CS GPIO pin (e.g. \"PA4\"). Defaults to PA4."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("capacity_mib"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Card capacity in MiB. Defaults to 1024."),
        },
    ]),
    labs: std::borrow::Cow::Borrowed(&[]),
};

impl PeripheralKit for MicroSdKit {
    fn metadata(&self) -> &'static KitMetadata {
        &MICROSD_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mib = match ctx.config_i64("capacity_mib") {
            Some(m) if m > 0 => m as u64,
            Some(m) => anyhow::bail!("microsd: capacity_mib must be positive, got {m}"),
            None => DEFAULT_CAPACITY_MIB,
        };
        let mut dev = MicroSd::with_capacity_mib(cs, mib);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host as SPI-mode drivers write it: send the frame, then clock 0xFF
    /// until a byte with bit 7 clear (R1) arrives, at most 8 times (NCR).
    struct Host {
        dev: MicroSd,
    }

    impl Host {
        fn new() -> Self {
            let mut dev = MicroSd::new("PA4");
            dev.cs_select();
            Self { dev }
        }

        fn x(&mut self, b: u8) -> u8 {
            self.dev.transfer(b)
        }

        fn frame(index: u8, arg: u32) -> [u8; 6] {
            let mut f = [0x40 | index, 0, 0, 0, 0, 0];
            f[1..5].copy_from_slice(&arg.to_be_bytes());
            f[5] = (crc7(&f[..5]) << 1) | 1;
            f
        }

        /// Send a command; return R1 (0xFF if none within NCR).
        fn cmd(&mut self, index: u8, arg: u32) -> u8 {
            for b in Self::frame(index, arg) {
                let miso = self.x(b);
                assert_eq!(miso, 0xFF, "MISO idle while CMD{index} is clocked in");
            }
            for _ in 0..8 {
                let r = self.x(0xFF);
                if r & 0x80 == 0 {
                    return r;
                }
            }
            0xFF
        }

        fn acmd(&mut self, index: u8, arg: u32) -> u8 {
            self.cmd(55, 0);
            self.cmd(index, arg)
        }

        fn bytes(&mut self, n: usize) -> Vec<u8> {
            (0..n).map(|_| self.x(0xFF)).collect()
        }

        fn init(&mut self) {
            assert_eq!(self.cmd(0, 0), 0x01);
            assert_eq!(self.cmd(8, 0x1AA), 0x01);
            assert_eq!(self.bytes(4), [0x00, 0x00, 0x01, 0xAA]);
            let mut polls = 0;
            while self.acmd(41, 0x4000_0000) != 0x00 {
                polls += 1;
                assert!(polls < 10, "ACMD41 never reported ready");
            }
            assert_eq!(self.cmd(58, 0), 0x00);
            assert_eq!(self.bytes(4), [0xC0, 0xFF, 0x80, 0x00]);
        }

        fn read_data_block(&mut self, len: usize) -> Vec<u8> {
            let mut token = 0xFF;
            for _ in 0..16 {
                token = self.x(0xFF);
                if token != 0xFF {
                    break;
                }
            }
            assert_eq!(token, TOKEN_SINGLE, "data start token");
            let data = self.bytes(len);
            let crc = self.bytes(2);
            assert_eq!(
                u16::from_be_bytes([crc[0], crc[1]]),
                crc16(&data),
                "block CRC16"
            );
            data
        }

        fn write_data_block(&mut self, token: u8, data: &[u8]) -> u8 {
            self.x(0xFF);
            self.x(token);
            for &b in data {
                self.x(b);
            }
            let crc = crc16(data);
            self.x((crc >> 8) as u8);
            self.x(crc as u8);
            let resp = self.x(0xFF);
            let mut busy = 0;
            while self.x(0xFF) != 0xFF {
                busy += 1;
                assert!(busy < 1000, "card never left busy");
            }
            assert!(busy > 0, "a write must show busy");
            resp
        }
    }

    #[test]
    fn crc7_matches_the_spec_examples() {
        assert_eq!(crc7(&[0x40, 0, 0, 0, 0]), 0x4A); // CMD0 → 0x95
        assert_eq!(crc7(&[0x48, 0, 0, 0x01, 0xAA]), 0x43); // CMD8 → 0x87
        assert_eq!(crc16(&[0xFF; 512]), 0x7FA1);
    }

    #[test]
    fn cmd0_r1_arrives_one_filler_byte_after_the_crc() {
        let mut h = Host::new();
        let f = Host::frame(0, 0);
        assert_eq!(f[5], 0x95);
        for b in f {
            assert_eq!(h.x(b), 0xFF);
        }
        assert_eq!(h.x(0xFF), 0xFF, "NCR: one filler byte first");
        assert_eq!(h.x(0xFF), 0x01, "then R1 idle");
        assert_eq!(h.x(0xFF), 0xFF);
    }

    #[test]
    fn cmd0_with_a_bad_crc_is_rejected() {
        let mut h = Host::new();
        for b in [0x40, 0, 0, 0, 0, 0x01] {
            h.x(b);
        }
        assert_eq!(h.x(0xFF), 0xFF);
        assert_eq!(h.x(0xFF), R1_IDLE | R1_CRC);
    }

    #[test]
    fn cmd8_r7_carries_r1_and_echo() {
        let mut h = Host::new();
        assert_eq!(h.cmd(0, 0), 0x01);
        assert_eq!(h.cmd(8, 0x1AA), 0x01);
        assert_eq!(h.bytes(5), [0x00, 0x00, 0x01, 0xAA, 0xFF]);
    }

    #[test]
    fn acmd41_reports_idle_then_ready_and_ocr_shows_ccs() {
        let mut h = Host::new();
        h.cmd(0, 0);
        h.cmd(8, 0x1AA);
        h.bytes(4); // the R7 tail
        assert_eq!(h.cmd(58, 0), 0x01);
        assert_eq!(
            h.bytes(4),
            [0x00, 0xFF, 0x80, 0x00],
            "busy bit clear while idle"
        );
        assert_eq!(h.acmd(41, 0x4000_0000), 0x01, "first poll still idle");
        assert_eq!(h.acmd(41, 0x4000_0000), 0x00, "then ready");
        assert!(h.dev.is_ready());
        assert_eq!(h.cmd(58, 0), 0x00);
        assert_eq!(h.bytes(4), [0xC0, 0xFF, 0x80, 0x00]);
    }

    #[test]
    fn an_sdhc_card_stays_idle_for_a_host_without_hcs() {
        let mut h = Host::new();
        h.cmd(0, 0);
        h.cmd(8, 0x1AA);
        h.bytes(4); // the R7 tail
        for _ in 0..5 {
            assert_eq!(h.acmd(41, 0), 0x01);
        }
    }

    #[test]
    fn data_commands_are_illegal_before_init() {
        let mut h = Host::new();
        h.cmd(0, 0);
        assert_eq!(h.cmd(17, 0), R1_IDLE | R1_ILLEGAL);
        assert_eq!(h.cmd(24, 0), R1_IDLE | R1_ILLEGAL);
    }

    #[test]
    fn cmd24_then_cmd17_round_trips_a_block() {
        let mut h = Host::new();
        h.init();
        assert_eq!(h.cmd(16, 512), 0x00);
        let mut data = [0u8; 512];
        data[..24].copy_from_slice(b"PATHFINDER FLIGHT LOG v1");
        data[511] = 0x5A;
        assert_eq!(h.cmd(24, 100), 0x00);
        assert_eq!(h.write_data_block(TOKEN_SINGLE, &data) & 0x1F, 0x05);
        assert_eq!(h.dev.block(100), Some(&data));
        assert_eq!(h.dev.written_lbas(), vec![100]);

        assert_eq!(h.cmd(17, 100), 0x00);
        assert_eq!(h.read_data_block(512), data);
        assert_eq!(h.cmd(17, 101), 0x00);
        assert_eq!(h.read_data_block(512), [0u8; 512], "unwritten reads zero");
    }

    #[test]
    fn cmd25_writes_consecutive_blocks_until_stop_tran() {
        let mut h = Host::new();
        h.init();
        assert_eq!(h.acmd(23, 3), 0x00);
        assert_eq!(h.cmd(25, 200), 0x00);
        for i in 0..3u8 {
            let data = [i + 1; 512];
            assert_eq!(h.write_data_block(TOKEN_MULTI_WRITE, &data) & 0x1F, 0x05);
        }
        h.x(TOKEN_STOP_TRAN);
        h.x(0xFF); // Nbr
        let mut busy = 0;
        while h.x(0xFF) != 0xFF {
            busy += 1;
            assert!(busy < 1000);
        }
        assert_eq!(h.dev.written_lbas(), vec![200, 201, 202]);
        assert_eq!(h.dev.block(202), Some(&[3u8; 512]));
        assert_eq!(h.cmd(13, 0), 0x00, "card back in command phase");
    }

    #[test]
    fn cmd18_streams_blocks_until_cmd12() {
        let mut h = Host::new();
        h.init();
        for lba in 10..13u32 {
            h.dev.set_block(lba, &[lba as u8; 512]);
        }
        assert_eq!(h.cmd(18, 10), 0x00);
        for lba in 10..13u32 {
            assert_eq!(h.read_data_block(512), vec![lba as u8; 512]);
        }
        // CMD12 while the stream runs: R1 after a stuff byte, then busy.
        for b in Host::frame(12, 0) {
            h.x(b);
        }
        let mut r1 = 0xFF;
        for _ in 0..8 {
            r1 = h.x(0xFF);
            if r1 & 0x80 == 0 {
                break;
            }
        }
        assert_eq!(r1, 0x00);
        let mut busy = 0;
        while h.x(0xFF) != 0xFF {
            busy += 1;
            assert!(busy < 1000);
        }
        assert_eq!(h.cmd(13, 0), 0x00);
        assert_eq!(h.bytes(1), [0x00], "R2 second byte");
    }

    #[test]
    fn csd_reports_the_capacity() {
        let mut h = Host::new();
        h.init();
        assert_eq!(h.cmd(9, 0), 0x00);
        let csd = h.read_data_block(16);
        assert_eq!(csd[0] >> 6, 1, "CSD v2.0");
        let c_size =
            (u32::from(csd[7] & 0x3F) << 16) | (u32::from(csd[8]) << 8) | u32::from(csd[9]);
        assert_eq!((c_size + 1) * 1024, h.dev.capacity_blocks());
        assert_eq!(h.dev.capacity_blocks(), 1024 * 1024 * 2);
    }

    #[test]
    fn a_bad_data_crc_is_rejected_when_crc_is_on() {
        let mut h = Host::new();
        h.init();
        assert_eq!(h.cmd(59, 1), 0x00);
        assert_eq!(h.cmd(24, 5), 0x00);
        h.x(0xFF);
        h.x(TOKEN_SINGLE);
        for _ in 0..512 {
            h.x(0xAA);
        }
        h.x(0x00);
        h.x(0x00);
        assert_eq!(h.x(0xFF) & 0x1F, 0x0B);
        assert_eq!(h.dev.block(5), None);
    }

    #[test]
    fn erase_drops_written_blocks() {
        let mut h = Host::new();
        h.init();
        h.dev.set_block(7, &[9u8; 512]);
        h.dev.set_block(8, &[9u8; 512]);
        assert_eq!(h.cmd(32, 7), 0x00);
        assert_eq!(h.cmd(33, 7), 0x00);
        assert_eq!(h.cmd(38, 0), 0x00);
        while h.x(0xFF) != 0xFF {}
        assert_eq!(h.dev.written_lbas(), vec![8]);
    }

    #[test]
    fn written_blocks_appear_in_the_inspect_artifact() {
        let mut h = Host::new();
        h.init();
        h.cmd(24, 3);
        h.write_data_block(TOKEN_SINGLE, &[0x11; 512]);
        let opts = crate::inspect::InspectOpts {
            include_bytes: true,
            peripheral: None,
        };
        let a = h.dev.artifacts("sd0", &opts);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].kind, "sd_card");
        assert_eq!(a[0].meta["written_lbas"], serde_json::json!([3]));
        assert_eq!(a[0].bytes.as_deref(), Some(&[0x11u8; 512][..]));
    }
}
