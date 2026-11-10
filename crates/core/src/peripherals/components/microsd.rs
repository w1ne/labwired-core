// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! microSD SPI card with a small FAT16 demo volume.
//!
//! Handles CMD0 (GO_IDLE), CMD8 (SEND_IF_COND), CMD55/ACMD41, CMD58 (OCR),
//! CMD9 (SEND_CSD), CMD17 (READ_SINGLE_BLOCK), and CMD24 (WRITE_BLOCK). The
//! fixed 32 MiB SDHC image contains a FAT16 volume with `/README.TXT`; sector
//! writes are retained for the lifetime of the simulated card.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;
use std::collections::{HashMap, VecDeque};

const CARD_SECTORS: u32 = 65_536;
const PARTITION_START: u32 = 1;
const PARTITION_SECTORS: u32 = 32_768;
const FAT1_START: u32 = PARTITION_START + 1;
const FAT_SECTORS: u32 = 128;
const FAT2_START: u32 = FAT1_START + FAT_SECTORS;
const ROOT_START: u32 = FAT2_START + FAT_SECTORS;
const ROOT_SECTORS: u32 = 32;
const DATA_START: u32 = ROOT_START + ROOT_SECTORS;
const README_LBA: u32 = DATA_START;
const README_TEXT: &[u8] = b"LabWired SD card demo.\r\n";

fn demo_sector(lba: u32) -> [u8; 512] {
    let mut sector = [0u8; 512];
    match lba {
        0 => {
            // One FAT16 partition starting at LBA 1.
            let entry = &mut sector[446..462];
            entry[1..4].copy_from_slice(&[0x00, 0x02, 0x00]);
            entry[4] = 0x06;
            entry[5..8].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
            entry[8..12].copy_from_slice(&PARTITION_START.to_le_bytes());
            entry[12..16].copy_from_slice(&PARTITION_SECTORS.to_le_bytes());
            sector[510..512].copy_from_slice(&[0x55, 0xAA]);
        }
        PARTITION_START => {
            // FAT16 BPB: 512-byte sectors, one sector per cluster, two FATs.
            sector[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
            sector[3..11].copy_from_slice(b"MSDOS5.0");
            sector[11..13].copy_from_slice(&512u16.to_le_bytes());
            sector[13] = 1;
            sector[14..16].copy_from_slice(&1u16.to_le_bytes());
            sector[16] = 2;
            sector[17..19].copy_from_slice(&512u16.to_le_bytes());
            sector[19..21].copy_from_slice(&(PARTITION_SECTORS as u16).to_le_bytes());
            sector[21] = 0xF8;
            sector[22..24].copy_from_slice(&(FAT_SECTORS as u16).to_le_bytes());
            sector[24..26].copy_from_slice(&32u16.to_le_bytes());
            sector[26..28].copy_from_slice(&64u16.to_le_bytes());
            sector[28..32].copy_from_slice(&PARTITION_START.to_le_bytes());
            sector[36] = 0x80;
            sector[38] = 0x29;
            sector[39..43].copy_from_slice(&0x4C57_5244u32.to_le_bytes());
            sector[43..54].copy_from_slice(b"LABWIRED   ");
            sector[54..62].copy_from_slice(b"FAT16   ");
            sector[510..512].copy_from_slice(&[0x55, 0xAA]);
        }
        FAT1_START | FAT2_START => {
            // Reserved FAT entries plus README.TXT in cluster 2 (end of chain).
            sector[0..6].copy_from_slice(&[0xF8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        }
        ROOT_START => {
            sector[0..11].copy_from_slice(b"README  TXT");
            sector[11] = 0x20; // archive file
            sector[26..28].copy_from_slice(&2u16.to_le_bytes());
            sector[28..32].copy_from_slice(&(README_TEXT.len() as u32).to_le_bytes());
        }
        README_LBA => sector[..README_TEXT.len()].copy_from_slice(README_TEXT),
        _ => {}
    }
    sector
}

fn demo_csd() -> [u8; 16] {
    // CSD v2: 32 MiB capacity, enough for the fixed FAT16 demo volume.
    let mut csd = [0u8; 16];
    csd[0] = 0x40;
    csd[5] = 0x09; // 512-byte read block length
    csd[9] = 0x3F; // C_SIZE = 63 => (63 + 1) * 512 KiB
    csd
}

fn sd_crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
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

fn queue_data_block(out: &mut VecDeque<u8>, data: &[u8]) {
    out.push_back(0xFE); // data token
    out.extend(data.iter().copied());
    let crc = sd_crc16(data);
    out.push_back((crc >> 8) as u8);
    out.push_back(crc as u8);
}

enum WriteState {
    AwaitToken {
        lba: u32,
    },
    Data {
        lba: u32,
        bytes: Vec<u8>,
    },
    Crc {
        lba: u32,
        bytes: Vec<u8>,
        remaining: u8,
    },
}

pub struct MicroSd {
    cs_pin: String,
    cmd_buf: Vec<u8>,
    out: VecDeque<u8>,
    idle: bool,
    initialized: bool,
    component_id: Option<String>,
    written_sectors: HashMap<u32, [u8; 512]>,
    write_state: Option<WriteState>,
}

impl MicroSd {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            cmd_buf: Vec::new(),
            out: VecDeque::new(),
            idle: true,
            initialized: false,
            component_id: None,
            written_sectors: HashMap::new(),
            write_state: None,
        }
    }

    fn r1(&self, bits: u8) -> u8 {
        bits
    }

    fn handle_command(&mut self) {
        if self.cmd_buf.len() < 6 {
            return;
        }
        let index = self.cmd_buf[0] & 0x3F;
        let arg = u32::from_be_bytes([
            self.cmd_buf[1],
            self.cmd_buf[2],
            self.cmd_buf[3],
            self.cmd_buf[4],
        ]);
        self.cmd_buf.clear();

        match index {
            0 => {
                // CMD0 GO_IDLE_STATE
                self.idle = true;
                self.initialized = false;
                self.write_state = None;
                self.out.push_back(self.r1(0x01)); // idle
            }
            8 => {
                // CMD8 SEND_IF_COND — R7: R1 + echo arg low 32
                self.out.push_back(self.r1(0x01));
                self.out.push_back(0x00);
                self.out.push_back(0x00);
                self.out.push_back(0x01);
                self.out.push_back(0xAA);
            }
            9 => {
                // CMD9 SEND_CSD — R1 followed by a 16-byte CSD data block.
                self.out
                    .push_back(self.r1(if self.idle { 0x01 } else { 0x00 }));
                queue_data_block(&mut self.out, &demo_csd());
            }
            55 => {
                // APP_CMD
                self.out
                    .push_back(self.r1(if self.idle { 0x01 } else { 0x00 }));
            }
            41 => {
                // ACMD41 — leave idle
                self.idle = false;
                self.initialized = true;
                self.out.push_back(self.r1(0x00));
            }
            42 => {
                // ACMD42 SET_CLR_CARD_DETECT — clear the pull-up on DAT3.
                self.out
                    .push_back(self.r1(if self.idle { 0x01 } else { 0x00 }));
            }
            13 => {
                // CMD13 SEND_STATUS — R2 (R1 plus a clear status byte).
                self.out
                    .push_back(self.r1(if self.idle { 0x01 } else { 0x00 }));
                self.out.push_back(0x00);
            }
            58 => {
                // CMD58 READ_OCR — R3
                self.out
                    .push_back(self.r1(if self.idle { 0x01 } else { 0x00 }));
                self.out.push_back(0xC0); // CCS + power up
                self.out.push_back(0xFF);
                self.out.push_back(0x80);
                self.out.push_back(0x00);
            }
            17 => {
                // CMD17 READ_SINGLE_BLOCK
                if self.idle {
                    self.out.push_back(self.r1(0x01));
                } else if arg >= CARD_SECTORS {
                    self.out.push_back(self.r1(0x20)); // address error
                } else {
                    self.out.push_back(self.r1(0x00));
                    let sector = self
                        .written_sectors
                        .get(&arg)
                        .copied()
                        .unwrap_or_else(|| demo_sector(arg));
                    queue_data_block(&mut self.out, &sector);
                }
            }
            24 => {
                // CMD24 WRITE_BLOCK; the host follows with a data token,
                // 512-byte sector and two CRC bytes. SDHC addresses are LBAs.
                if self.idle {
                    self.out.push_back(self.r1(0x01));
                } else if arg >= CARD_SECTORS {
                    self.out.push_back(self.r1(0x20)); // address error
                } else {
                    self.out.push_back(self.r1(0x00));
                    self.write_state = Some(WriteState::AwaitToken { lba: arg });
                }
            }
            _ => {
                self.out
                    .push_back(self.r1(if self.idle { 0x01 } else { 0x04 })); // illegal
            }
        }
    }
}

impl SpiDevice for MicroSd {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.cmd_buf.clear();
        self.out.clear();
    }

    fn cs_release(&mut self) {
        self.cmd_buf.clear();
        self.write_state = None;
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        if let Some(b) = self.out.pop_front() {
            // Still accept MOSI into command if host sends while draining
            if (mosi & 0xC0) == 0x40 && self.cmd_buf.is_empty() {
                self.cmd_buf.push(mosi);
            } else if !self.cmd_buf.is_empty() && self.cmd_buf.len() < 6 {
                self.cmd_buf.push(mosi);
                if self.cmd_buf.len() == 6 {
                    self.handle_command();
                }
            }
            return b;
        }

        if let Some(state) = self.write_state.take() {
            match state {
                WriteState::AwaitToken { lba } if mosi == 0xFE => {
                    self.write_state = Some(WriteState::Data {
                        lba,
                        bytes: Vec::with_capacity(512),
                    });
                }
                WriteState::AwaitToken { lba } if mosi == 0xFF => {
                    self.write_state = Some(WriteState::AwaitToken { lba });
                }
                WriteState::AwaitToken { .. } => {
                    // 0x0D is the SD data-error token. Unsupported write
                    // tokens do not alter the fixed demo volume.
                    self.out.push_back(0x0D);
                }
                WriteState::Data { lba, mut bytes } => {
                    bytes.push(mosi);
                    if bytes.len() == 512 {
                        self.write_state = Some(WriteState::Crc {
                            lba,
                            bytes,
                            remaining: 2,
                        });
                    } else {
                        self.write_state = Some(WriteState::Data { lba, bytes });
                    }
                }
                WriteState::Crc {
                    lba,
                    bytes,
                    remaining,
                } if remaining > 1 => {
                    self.write_state = Some(WriteState::Crc {
                        lba,
                        bytes,
                        remaining: remaining - 1,
                    });
                }
                WriteState::Crc { lba, bytes, .. } => {
                    if let Ok(sector) = <Vec<u8> as TryInto<[u8; 512]>>::try_into(bytes) {
                        self.written_sectors.insert(lba, sector);
                        // Accepted data block, brief busy interval, then
                        // ready. The two supplied CRC bytes are ignored while
                        // card CRC checking is disabled in SPI mode.
                        self.out.extend([0x05, 0x00, 0xFF]);
                    } else {
                        self.out.push_back(0x0D);
                    }
                }
            }
            return 0xFF;
        }

        // Idle MISO is 0xFF until a command starts
        if (mosi & 0xC0) == 0x40 || !self.cmd_buf.is_empty() {
            self.cmd_buf.push(mosi);
            if self.cmd_buf.len() == 6 {
                self.handle_command();
                // The host may send the packet with a bulk SPI write that
                // discards MISO. SD cards present R1 on the following clock.
                return 0xFF;
            }
            return 0xFF;
        }
        0xFF
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
        "SDHC card image with a FAT16 demo volume and README.TXT; single-block writes are retained in simulation.",
    ),
    detail: std::borrow::Cow::Borrowed(
        "CMD0/8/9/13/17/24/55/41/42/58 SDHC profile with a fixed 32 MiB FAT16 \
             volume containing /README.TXT. Single-block writes persist for the \
             lifetime of this simulated card; the image resets between runs.",
    ),
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: std::borrow::Cow::Borrowed(&[ConfigKey {
        name: std::borrow::Cow::Borrowed("cs_pin"),
        ty: ConfigType::Str,
        doc: std::borrow::Cow::Borrowed("Card CS GPIO pin (e.g. \"PA4\"). Defaults to PA4."),
    }]),
    labs: std::borrow::Cow::Borrowed(&[]),
};

impl PeripheralKit for MicroSdKit {
    fn metadata(&self) -> &'static KitMetadata {
        &MICROSD_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mut dev = MicroSd::new(cs);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd0_returns_idle() {
        let mut dev = MicroSd::new("PA4");
        // CMD0 + arg0 + crc 0x95
        let cmd = [0x40, 0, 0, 0, 0, 0x95];
        dev.cs_select();
        let mut last = 0xFF;
        for &b in &cmd {
            last = dev.transfer(b);
        }
        // response may appear on last CRC or next FF
        if last == 0xFF {
            last = dev.transfer(0xFF);
        }
        assert_eq!(last, 0x01);
    }
}
