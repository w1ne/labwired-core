// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MFRC522 / RC522 RFID reader — SPI register file plus ISO/IEC 14443-A CL1.
//!
//! Covers the register shell the common Arduino MFRC522 library probes
//! (VersionReg 0x92, soft reset) and enough of the RF side for
//! `PICC_IsNewCardPresent` / `PICC_ReadCardSerial` / `PICC_HaltA`:
//! REQA/WUPA → ATQA, anticollision CL1 + SELECT → 4-byte UID + BCC + SAK,
//! HLTA, FIFO, ComIrqReg / DivIrqReg, and CRC_A via CalcCRC. MIFARE Crypto1
//! is not modelled.
//!
//! Place or remove a card with SimInput channels `card_present` and `uid`.

use crate::peripherals::spi::SpiDevice;
use crate::sim_input::{InputChannel, SimInput, SimInputError};
use std::any::Any;

const REG_COMMAND: u8 = 0x01;
const REG_COMIRQ: u8 = 0x04;
const REG_DIVIRQ: u8 = 0x05;
const REG_ERROR: u8 = 0x06;
const REG_STATUS1: u8 = 0x07;
const REG_STATUS2: u8 = 0x08;
const REG_FIFODATA: u8 = 0x09;
const REG_FIFOLEVEL: u8 = 0x0A;
const REG_CONTROL: u8 = 0x0C;
const REG_BITFRAMING: u8 = 0x0D;
const REG_COLL: u8 = 0x0E;
const REG_MODE: u8 = 0x11;
const REG_CRCRESULT_H: u8 = 0x21;
const REG_CRCRESULT_L: u8 = 0x22;
const REG_VERSION: u8 = 0x37;

const CMD_IDLE: u8 = 0x00;
const CMD_CALCCRC: u8 = 0x03;
const CMD_TRANSCEIVE: u8 = 0x0C;
const CMD_SOFTRESET: u8 = 0x0F;

const COMIRQ_TIMER: u8 = 0x01;
const COMIRQ_IDLE: u8 = 0x04;
const COMIRQ_RX: u8 = 0x20;
const COMIRQ_TX: u8 = 0x40;
const DIVIRQ_CRC: u8 = 0x04;

const PICC_REQA: u8 = 0x26;
const PICC_WUPA: u8 = 0x52;
const PICC_HLTA: u8 = 0x50;
const PICC_SEL_CL1: u8 = 0x93;

const FIFO_CAP: usize = 64;

/// Default ATQA for a single-size (4-byte UID) Type A PICC.
const ATQA: [u8; 2] = [0x04, 0x00];
/// SAK for a MIFARE Classic 1K-class PICC (no cascade bit).
const SAK: u8 = 0x08;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Address,
    Data,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PiccState {
    Idle,
    Ready,
    Active,
    Halt,
}

/// ISO/IEC 14443-3 CRC_A (reflected poly 0x8408, preset 0x6363).
pub fn crc_a(data: &[u8]) -> [u8; 2] {
    let mut crc: u16 = 0x6363;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0x8408;
            } else {
                crc >>= 1;
            }
        }
    }
    [(crc & 0xff) as u8, (crc >> 8) as u8]
}

fn reg_from_addr(addr: u8) -> u8 {
    (addr >> 1) & 0x3F
}

fn is_read(addr: u8) -> bool {
    addr & 0x80 != 0
}

pub struct Rc522 {
    cs_pin: String,
    regs: [u8; 0x40],
    fifo: [u8; FIFO_CAP],
    fifo_len: usize,
    fifo_rpos: usize,
    phase: Phase,
    reading: bool,
    reg: u8,
    card_present: bool,
    uid: [u8; 4],
    picc: PiccState,
    component_id: Option<String>,
}

impl Rc522 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        let mut regs = [0u8; 0x40];
        regs[REG_VERSION as usize] = 0x92;
        regs[REG_COMMAND as usize] = CMD_IDLE;
        // CollReg power-on: ValuesAfterColl=1 (bit 7).
        regs[REG_COLL as usize] = 0x80;
        Self {
            cs_pin: cs_pin.into(),
            regs,
            fifo: [0; FIFO_CAP],
            fifo_len: 0,
            fifo_rpos: 0,
            phase: Phase::Address,
            reading: false,
            reg: 0,
            card_present: false,
            uid: [0xDE, 0xAD, 0xBE, 0xEF],
            picc: PiccState::Idle,
            component_id: None,
        }
    }

    fn fifo_level(&self) -> u8 {
        self.fifo_len as u8
    }

    fn fifo_flush(&mut self) {
        self.fifo_len = 0;
        self.fifo_rpos = 0;
        self.regs[REG_FIFOLEVEL as usize] = 0;
    }

    fn fifo_push(&mut self, b: u8) {
        if self.fifo_len >= FIFO_CAP {
            return;
        }
        let wpos = (self.fifo_rpos + self.fifo_len) % FIFO_CAP;
        self.fifo[wpos] = b;
        self.fifo_len += 1;
        self.regs[REG_FIFOLEVEL as usize] = self.fifo_level();
    }

    fn fifo_pop(&mut self) -> u8 {
        if self.fifo_len == 0 {
            return 0;
        }
        let b = self.fifo[self.fifo_rpos];
        self.fifo_rpos = (self.fifo_rpos + 1) % FIFO_CAP;
        self.fifo_len -= 1;
        self.regs[REG_FIFOLEVEL as usize] = self.fifo_level();
        b
    }

    fn fifo_snapshot(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.fifo_len);
        for i in 0..self.fifo_len {
            out.push(self.fifo[(self.fifo_rpos + i) % FIFO_CAP]);
        }
        out
    }

    fn fifo_load(&mut self, data: &[u8]) {
        self.fifo_flush();
        for &b in data {
            self.fifo_push(b);
        }
    }

    fn write_comirq(&mut self, value: u8) {
        // Bit 7 = Set1: 1 → set named bits, 0 → clear named bits.
        let set1 = value & 0x80 != 0;
        let mask = value & 0x7F;
        if set1 {
            self.regs[REG_COMIRQ as usize] |= mask;
        } else {
            self.regs[REG_COMIRQ as usize] &= !mask;
        }
    }

    fn write_divirq(&mut self, value: u8) {
        let set2 = value & 0x80 != 0;
        let mask = value & 0x7F;
        if set2 {
            self.regs[REG_DIVIRQ as usize] |= mask;
        } else {
            self.regs[REG_DIVIRQ as usize] &= !mask;
        }
    }

    fn soft_reset(&mut self) {
        self.regs[REG_COMMAND as usize] = CMD_IDLE;
        self.regs[REG_VERSION as usize] = 0x92;
        self.regs[REG_COMIRQ as usize] = 0;
        self.regs[REG_DIVIRQ as usize] = 0;
        self.regs[REG_ERROR as usize] = 0;
        self.regs[REG_CONTROL as usize] = 0;
        self.regs[REG_BITFRAMING as usize] = 0;
        self.regs[REG_COLL as usize] = 0x80;
        self.fifo_flush();
        // Soft reset does not remove a card from the field; PICC returns to Idle
        // if still present (silicon leaves the air interface cold until TxControl
        // is rewritten — for the Arduino init path Idle is correct).
        if self.card_present {
            self.picc = PiccState::Idle;
        }
    }

    fn run_calc_crc(&mut self) {
        let data = self.fifo_snapshot();
        let [lo, hi] = crc_a(&data);
        self.regs[REG_CRCRESULT_L as usize] = lo;
        self.regs[REG_CRCRESULT_H as usize] = hi;
        self.regs[REG_DIVIRQ as usize] |= DIVIRQ_CRC;
        self.regs[REG_COMMAND as usize] = CMD_IDLE;
    }

    fn respond_ok(&mut self, data: &[u8], rx_last_bits: u8) {
        self.fifo_load(data);
        self.regs[REG_CONTROL as usize] =
            (self.regs[REG_CONTROL as usize] & !0x07) | (rx_last_bits & 0x07);
        self.regs[REG_ERROR as usize] = 0;
        self.regs[REG_COMIRQ as usize] |= COMIRQ_TX | COMIRQ_RX | COMIRQ_IDLE;
        self.regs[REG_COMMAND as usize] = CMD_IDLE;
    }

    fn respond_timeout(&mut self) {
        self.fifo_flush();
        self.regs[REG_ERROR as usize] = 0;
        self.regs[REG_COMIRQ as usize] |= COMIRQ_TIMER;
        self.regs[REG_COMMAND as usize] = CMD_IDLE;
    }

    fn run_transceive(&mut self) {
        // Clear StartSend (self-clearing on silicon).
        self.regs[REG_BITFRAMING as usize] &= !0x80;
        let tx_last_bits = self.regs[REG_BITFRAMING as usize] & 0x07;
        let tx = self.fifo_snapshot();
        if tx.is_empty() {
            self.respond_timeout();
            return;
        }

        if !self.card_present {
            self.respond_timeout();
            return;
        }

        // REQA / WUPA — short 7-bit frame.
        if tx.len() == 1 && tx_last_bits == 7 {
            let cmd = tx[0] & 0x7F;
            let accept = match cmd {
                PICC_REQA => self.picc == PiccState::Idle,
                PICC_WUPA => matches!(self.picc, PiccState::Idle | PiccState::Halt),
                _ => false,
            };
            if accept {
                self.picc = PiccState::Ready;
                self.respond_ok(&ATQA, 0);
            } else {
                self.respond_timeout();
            }
            return;
        }

        // Anticollision CL1: SEL=0x93, NVB=0x20 → UID + BCC.
        if tx.len() == 2 && tx[0] == PICC_SEL_CL1 && tx[1] == 0x20 {
            if self.picc != PiccState::Ready {
                self.respond_timeout();
                return;
            }
            let bcc = self.uid[0] ^ self.uid[1] ^ self.uid[2] ^ self.uid[3];
            let mut resp = [0u8; 5];
            resp[..4].copy_from_slice(&self.uid);
            resp[4] = bcc;
            self.respond_ok(&resp, 0);
            return;
        }

        // SELECT CL1: 0x93 0x70 UID[4] BCC CRC_A[2].
        if tx.len() >= 9 && tx[0] == PICC_SEL_CL1 && tx[1] == 0x70 {
            if self.picc != PiccState::Ready {
                self.respond_timeout();
                return;
            }
            let uid_ok = tx[2..6] == self.uid;
            let bcc_ok = tx[6] == (self.uid[0] ^ self.uid[1] ^ self.uid[2] ^ self.uid[3]);
            let expect_crc = crc_a(&tx[..7]);
            let crc_ok = tx[7] == expect_crc[0] && tx[8] == expect_crc[1];
            if uid_ok && bcc_ok && crc_ok {
                self.picc = PiccState::Active;
                let sak_crc = crc_a(&[SAK]);
                self.respond_ok(&[SAK, sak_crc[0], sak_crc[1]], 0);
            } else {
                self.respond_timeout();
            }
            return;
        }

        // HLTA: 0x50 0x00 CRC_A — success is a timeout (no modulation).
        if tx.len() >= 4 && tx[0] == PICC_HLTA && tx[1] == 0x00 {
            let expect_crc = crc_a(&tx[..2]);
            if tx[2] == expect_crc[0] && tx[3] == expect_crc[1] {
                if self.picc == PiccState::Active {
                    self.picc = PiccState::Halt;
                }
            }
            self.respond_timeout();
            return;
        }

        self.respond_timeout();
    }

    fn maybe_start_transceive(&mut self) {
        let cmd = self.regs[REG_COMMAND as usize] & 0x0F;
        let start = self.regs[REG_BITFRAMING as usize] & 0x80 != 0;
        if cmd == CMD_TRANSCEIVE && start {
            self.run_transceive();
        }
    }

    fn write_reg(&mut self, reg: u8, value: u8) {
        match reg {
            REG_VERSION => {
                // Read-only (§9.3.4.8).
            }
            REG_COMMAND => {
                let cmd = value & 0x0F;
                self.regs[REG_COMMAND as usize] = value;
                match cmd {
                    CMD_SOFTRESET => self.soft_reset(),
                    CMD_CALCCRC => self.run_calc_crc(),
                    CMD_IDLE => {}
                    CMD_TRANSCEIVE => self.maybe_start_transceive(),
                    _ => {}
                }
            }
            REG_COMIRQ => self.write_comirq(value),
            REG_DIVIRQ => self.write_divirq(value),
            REG_FIFODATA => self.fifo_push(value),
            REG_FIFOLEVEL => {
                if value & 0x80 != 0 {
                    self.fifo_flush();
                }
                // Level bits are read-only; FlushBuffer is write-only.
            }
            REG_BITFRAMING => {
                self.regs[REG_BITFRAMING as usize] = value;
                self.maybe_start_transceive();
            }
            REG_CRCRESULT_H | REG_CRCRESULT_L => {
                // Read-only result registers.
            }
            REG_ERROR | REG_STATUS1 | REG_STATUS2 => {
                // Status / error are produced by the model.
            }
            _ => {
                self.regs[reg as usize] = value;
            }
        }
    }

    fn read_reg(&mut self, reg: u8) -> u8 {
        match reg {
            REG_FIFODATA => self.fifo_pop(),
            REG_FIFOLEVEL => self.fifo_level(),
            _ => self.regs[reg as usize],
        }
    }

    /// Place a card with `uid` in the field (Idle). `None` removes it.
    pub fn set_card(&mut self, uid: Option<[u8; 4]>) {
        match uid {
            Some(u) => {
                self.uid = u;
                self.card_present = true;
                self.picc = PiccState::Idle;
            }
            None => {
                self.card_present = false;
                self.picc = PiccState::Idle;
            }
        }
    }

    pub fn card_present(&self) -> bool {
        self.card_present
    }

    pub fn uid(&self) -> [u8; 4] {
        self.uid
    }
}

impl SpiDevice for Rc522 {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.phase = Phase::Address;
    }

    fn cs_release(&mut self) {
        self.phase = Phase::Address;
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        match self.phase {
            Phase::Address => {
                self.reg = reg_from_addr(mosi);
                self.reading = is_read(mosi);
                self.phase = Phase::Data;
                0x00
            }
            Phase::Data => {
                if self.reading {
                    self.read_reg(self.reg)
                } else {
                    self.write_reg(self.reg, mosi);
                    0x00
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

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

pub const INPUT_CHANNELS: &[InputChannel] = &[
    InputChannel {
        key: std::borrow::Cow::Borrowed("card_present"),
        label: std::borrow::Cow::Borrowed("Card present"),
        unit: std::borrow::Cow::Borrowed("on/off"),
        min: 0.0,
        max: 1.0,
        default: Some(0.0),
    },
    InputChannel {
        key: std::borrow::Cow::Borrowed("uid"),
        label: std::borrow::Cow::Borrowed("Card UID"),
        unit: std::borrow::Cow::Borrowed("u32"),
        min: 0.0,
        max: 4_294_967_295.0,
        default: Some(0xDEAD_BEEFu32 as f64),
    },
];

impl SimInput for Rc522 {
    fn input_channels(&self) -> &[InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        let ch = self.require_channel(key, value)?;
        match ch.key.as_ref() {
            "card_present" => {
                if value >= 0.5 {
                    let uid = self.uid;
                    self.set_card(Some(uid));
                } else {
                    self.set_card(None);
                }
            }
            "uid" => {
                let bits = value as u32;
                self.uid = bits.to_be_bytes();
                if self.card_present {
                    self.picc = PiccState::Idle;
                }
            }
            _ => unreachable!("require_channel only returns declared channels"),
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Rc522Kit;
pub static RC522_KIT: Rc522Kit = Rc522Kit;

static RC522_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(INPUT_CHANNELS),
    device_type: std::borrow::Cow::Borrowed("rc522"),
    label: std::borrow::Cow::Borrowed("RC522 RFID"),
    summary: std::borrow::Cow::Borrowed(
        "MFRC522 SPI RFID reader with ISO/IEC 14443-A cascade-level-1 air interface.",
    ),
    detail: std::borrow::Cow::Borrowed(
        "VersionReg 0x37 returns 0x92. Soft reset, FIFO, ComIrqReg/DivIrqReg, and CRC_A \
         (CalcCRC) follow the MFRC522 datasheet. REQA/WUPA return ATQA; anticollision CL1 \
         and SELECT return a 4-byte UID + BCC + SAK; HLTA succeeds as a timeout. Drive \
         card_present and uid to place or remove a card. MIFARE Crypto1 is not modelled.",
    ),
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: std::borrow::Cow::Borrowed(&[ConfigKey {
        name: std::borrow::Cow::Borrowed("cs_pin"),
        ty: ConfigType::Str,
        doc: std::borrow::Cow::Borrowed(
            "SPI CS GPIO pin name (module SDA/NSS). Defaults to PA4.",
        ),
    }]),
    labs: std::borrow::Cow::Borrowed(&[]),
};

impl PeripheralKit for Rc522Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &RC522_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mut dev = Rc522::new(cs);
        dev.set_component_id(ctx.device_id().to_string());
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "rc522_tests.rs"]
mod tests;
