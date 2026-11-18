// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SD card in SPI mode, attached through a system manifest and backed by a
//! disk image, driven byte by byte over its `SpiDevice` surface.
//!
//! Expectations come from the SD Association "Physical Layer Simplified
//! Specification" (Part 1, version 9.10), cited by section:
//!
//! * 7.2.1 / Figure 7-2: SPI-mode initialisation, CMD0 -> CMD8 -> (CMD55 +
//!   ACMD41)* -> CMD58.
//! * 7.3.1.3 / Table 7-3: the command frame (0b01 + index, 32-bit argument,
//!   CRC7 + end bit); 7.3.2.1 R1, 7.3.2.4 R3, 7.3.2.6 R7.
//! * 4.3.13 / Table 4-18: CMD8 echoes VHS and the check pattern.
//! * 5.1 / Table 5-1: OCR bit 31 (power-up done) and bit 30 (CCS).
//! * 5.3.3 / Table 5-16: CSD version 2.0, capacity = (C_SIZE + 1) * 512 KiB.
//! * 7.2.3 / 7.3.3.2: single and multiple block read, start block token
//!   0xFE, 512 data bytes, CRC16 (4.5, polynomial x^16 + x^12 + x^5 + 1).
//! * 7.2.4 / 7.3.3.1: block write, data response token xxx0_0101 = accepted,
//!   then busy (0x00) until the card releases DO; 7.3.3.2: 0xFC start token
//!   for CMD25 and 0xFD stop-tran token.
//! * 7.2.1 note / 4.3.14: SDHC/SDXC use block addressing; CMD16 length is
//!   fixed at 512 for those cards.
//! * 7.2 "the card is selected by CS low"; CS high ends the transaction.
//!
//! The backing image is the new behaviour here: blocks come from a host file
//! (`image:` in the manifest, read relative to it) or from in-memory bytes
//! (`image_base64:` inline, or `MicroSd::from_image`), and the card's
//! capacity is derived from it.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::inspect::InspectOpts;
use labwired_core::peripherals::spi::SpiDevice;
use std::collections::HashMap;
use std::path::PathBuf;

const BLOCK: usize = 512;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// RFC 4648 base64, written out here so the test does not share an encoder
/// with the code under test.
fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        s.push(A[(n >> 18) as usize & 63] as char);
        s.push(A[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

/// A recognisable image: block `n` is filled with `n as u8 ^ i as u8`, and
/// carries its LBA in its first four bytes.
fn image(blocks: usize) -> Vec<u8> {
    let mut v = vec![0u8; blocks * BLOCK];
    for (n, blk) in v.chunks_mut(BLOCK).enumerate() {
        for (i, b) in blk.iter_mut().enumerate() {
            *b = (n as u8) ^ (i as u8);
        }
        blk[..4].copy_from_slice(&(n as u32).to_be_bytes());
    }
    v
}

fn bus_with(manifest_devices: Vec<ExternalDevice>) -> SystemBus {
    let chip_path = repo("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip");
    let manifest = SystemManifest {
        max_peripheral_tick_interval: None,
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "sd-rig".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: manifest_devices,
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        parts: Default::default(),
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

fn sd_device(config: &[(&str, serde_yaml::Value)]) -> ExternalDevice {
    let mut cfg = HashMap::new();
    cfg.insert("cs_pin".to_string(), "PA4".into());
    for (k, v) in config {
        cfg.insert(k.to_string(), v.clone());
    }
    ExternalDevice {
        id: "sd0".to_string(),
        r#type: "microsd".to_string(),
        connection: "spi1".to_string(),
        channel: None,
        route: Default::default(),
        config: cfg,
    }
}

fn rig_with_image(img: &[u8]) -> SystemBus {
    bus_with(vec![sd_device(&[("image_base64", b64(img).into())])])
}

/// CRC7 (x^7 + x^3 + 1) of a command frame, 7.2.2.
fn crc7(bytes: &[u8]) -> u8 {
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

/// CRC16-CCITT (x^16 + x^12 + x^5 + 1, init 0), 4.5.
fn crc16(bytes: &[u8]) -> u16 {
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

/// The host side of the SPI bus, as SD drivers write it.
struct Host<'a> {
    dev: &'a mut Box<dyn SpiDevice>,
}

impl<'a> Host<'a> {
    fn on(bus: &'a mut SystemBus) -> Self {
        let idx = bus.find_peripheral_index_by_name("spi1").expect("spi1");
        let dev = bus.peripherals[idx]
            .dev
            .spi_attached_devices_mut()
            .expect("spi1 hosts SPI devices")
            .first_mut()
            .expect("card attached");
        dev.cs_select();
        Host { dev }
    }

    fn x(&mut self, b: u8) -> u8 {
        self.dev.transfer(b)
    }

    /// Table 7-3 frame; R1 is the first byte with bit 7 clear within NCR (8).
    fn cmd(&mut self, index: u8, arg: u32) -> u8 {
        let mut f = [0x40 | index, 0, 0, 0, 0, 0];
        f[1..5].copy_from_slice(&arg.to_be_bytes());
        f[5] = (crc7(&f[..5]) << 1) | 1;
        for b in f {
            self.x(b);
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

    /// Figure 7-2, the v2.00+ / SDHC branch.
    fn init(&mut self) {
        assert_eq!(self.cmd(0, 0), 0x01, "CMD0 -> R1 idle");
        assert_eq!(self.cmd(8, 0x1AA), 0x01, "CMD8 -> R7");
        assert_eq!(
            self.bytes(4),
            [0x00, 0x00, 0x01, 0xAA],
            "VHS + check pattern echoed"
        );
        let mut polls = 0;
        while self.acmd(41, 0x4000_0000) != 0x00 {
            polls += 1;
            assert!(polls < 100, "ACMD41 never left idle");
        }
        assert_eq!(self.cmd(58, 0), 0x00, "CMD58 -> R3");
        let ocr = self.bytes(4);
        assert_eq!(ocr[0] & 0xC0, 0xC0, "OCR busy (31) and CCS (30) set: SDHC");
    }

    /// Wait for the 0xFE start token (NAC), then read `len` bytes + CRC16.
    fn data_block(&mut self, len: usize) -> Vec<u8> {
        let mut token = 0xFF;
        for _ in 0..64 {
            token = self.x(0xFF);
            if token != 0xFF {
                break;
            }
        }
        assert_eq!(token, 0xFE, "start block token");
        let data = self.bytes(len);
        let crc = u16::from_be_bytes([self.x(0xFF), self.x(0xFF)]);
        assert_eq!(crc, crc16(&data), "data CRC16");
        data
    }

    /// Send a data block behind `token`; return the data response token.
    fn write_block(&mut self, token: u8, data: &[u8]) -> u8 {
        self.x(0xFF);
        self.x(token);
        for &b in data {
            self.x(b);
        }
        let crc = crc16(data);
        self.x((crc >> 8) as u8);
        self.x(crc as u8);
        let resp = self.x(0xFF);
        // Busy: DO held low until the card is done.
        let mut n = 0;
        while self.x(0xFF) == 0x00 {
            n += 1;
            assert!(n < 10_000, "card never left busy");
        }
        resp
    }
}

/// 7.2.3: CMD17 with a block address (SDHC) returns that block of the image
/// behind a 0xFE token and a valid CRC16.
#[test]
fn cmd17_reads_blocks_from_the_backing_image() {
    let img = image(8);
    let mut bus = rig_with_image(&img);
    let mut h = Host::on(&mut bus);
    h.init();
    for lba in [0u32, 1, 5, 7] {
        assert_eq!(h.cmd(17, lba), 0x00, "R1 ready for CMD17 lba {lba}");
        let got = h.data_block(BLOCK);
        let at = lba as usize * BLOCK;
        assert_eq!(got, img[at..at + BLOCK], "block {lba} from the image");
    }
}

/// 7.2.3: CMD18 streams consecutive blocks until CMD12 (STOP_TRANSMISSION).
#[test]
fn cmd18_streams_consecutive_image_blocks_until_cmd12() {
    let img = image(8);
    let mut bus = rig_with_image(&img);
    let mut h = Host::on(&mut bus);
    h.init();
    assert_eq!(h.cmd(18, 2), 0x00);
    for lba in 2..5usize {
        assert_eq!(
            h.data_block(BLOCK),
            img[lba * BLOCK..(lba + 1) * BLOCK],
            "streamed block {lba}"
        );
    }
    assert_eq!(h.cmd(12, 0), 0x00, "CMD12 R1");
    // After busy the card answers commands again.
    while h.x(0xFF) == 0x00 {}
    assert_eq!(h.cmd(13, 0), 0x00, "card back in transfer state");
}

/// 5.3.3: CSD v2.0 capacity is (C_SIZE + 1) * 512 KiB, so a 2 MiB image is
/// C_SIZE = 3; an image that is not a whole number of units rounds up.
#[test]
fn csd_capacity_follows_the_image_size() {
    let img = image(2 * 2048); // 2 MiB
    let mut bus = rig_with_image(&img);
    let mut h = Host::on(&mut bus);
    h.init();
    assert_eq!(h.cmd(9, 0), 0x00, "CMD9 R1");
    let csd = h.data_block(16);
    assert_eq!(csd[0] >> 6, 1, "CSD_STRUCTURE = 1 (version 2.0)");
    let c_size = (u32::from(csd[7] & 0x3F) << 16) | (u32::from(csd[8]) << 8) | u32::from(csd[9]);
    assert_eq!(c_size, 3, "(3 + 1) * 512 KiB = 2 MiB");

    // An image that is not a whole number of 512 KiB units is padded up.
    let mut bus = rig_with_image(&image(3));
    let mut h = Host::on(&mut bus);
    h.init();
    h.cmd(9, 0);
    let csd = h.data_block(16);
    let c_size = (u32::from(csd[7] & 0x3F) << 16) | (u32::from(csd[8]) << 8) | u32::from(csd[9]);
    assert_eq!(c_size, 0, "three blocks fit the smallest unit");
    // Beyond the image but inside the unit reads as erased (zeros).
    assert_eq!(h.cmd(17, 100), 0x00);
    assert_eq!(h.data_block(BLOCK), vec![0u8; BLOCK]);
    // Beyond capacity is a parameter error (R1 bit 6, 7.3.2.1).
    assert_eq!(h.cmd(17, 1024), 0x40);
}

/// 7.2.4 + 7.3.3.1: CMD24 writes one block (data response 0bxxx0_0101), the
/// written block reads back and its neighbours still come from the image.
#[test]
fn cmd24_overwrites_one_image_block() {
    let img = image(4);
    let mut bus = rig_with_image(&img);
    {
        let mut h = Host::on(&mut bus);
        h.init();
        assert_eq!(h.cmd(24, 1), 0x00);
        let new = [0xA5u8; BLOCK];
        assert_eq!(h.write_block(0xFE, &new) & 0x1F, 0x05, "data accepted");
        assert_eq!(h.cmd(17, 1), 0x00);
        assert_eq!(h.data_block(BLOCK), new.to_vec());
        for lba in [0usize, 2] {
            assert_eq!(h.cmd(17, lba as u32), 0x00);
            assert_eq!(h.data_block(BLOCK), img[lba * BLOCK..(lba + 1) * BLOCK]);
        }
    }
    // The inspect artifact lists only the block that differs from the image.
    let art = bus
        .inspect_devices(None, &InspectOpts::default())
        .into_iter()
        .find(|d| d.id == "sd0")
        .expect("sd0 inspect")
        .artifacts
        .into_iter()
        .find(|a| a.kind == "sd_card")
        .expect("sd_card artifact");
    assert_eq!(art.meta["written_lbas"], serde_json::json!([1]));
    assert_eq!(art.meta["image_blocks"], 4);
}

/// 7.3.3.2: CMD25 takes blocks behind 0xFC tokens and ends with 0xFD.
#[test]
fn cmd25_writes_consecutive_blocks_over_the_image() {
    let img = image(6);
    let mut bus = rig_with_image(&img);
    let mut h = Host::on(&mut bus);
    h.init();
    assert_eq!(h.cmd(25, 2), 0x00);
    for v in [0x11u8, 0x22] {
        assert_eq!(h.write_block(0xFC, &[v; BLOCK]) & 0x1F, 0x05);
    }
    h.x(0xFD); // stop tran
    while h.x(0xFF) == 0x00 {}
    for (lba, want) in [(2u32, vec![0x11u8; BLOCK]), (3, vec![0x22; BLOCK])] {
        assert_eq!(h.cmd(17, lba), 0x00);
        assert_eq!(h.data_block(BLOCK), want);
    }
    assert_eq!(h.cmd(17, 4), 0x00);
    assert_eq!(
        h.data_block(BLOCK),
        img[4 * BLOCK..5 * BLOCK],
        "block 4 untouched"
    );
}

/// 4.3.14 / 7.2.1: on an SDHC card CMD16 is accepted and the block length
/// stays 512; addresses are block numbers, not bytes.
#[test]
fn sdhc_uses_block_addressing_and_fixed_block_length() {
    let img = image(4);
    let mut bus = rig_with_image(&img);
    let mut h = Host::on(&mut bus);
    h.init();
    assert_eq!(h.cmd(16, 512), 0x00, "CMD16 accepted");
    assert_eq!(h.cmd(17, 3), 0x00);
    assert_eq!(
        h.data_block(BLOCK),
        img[3 * BLOCK..4 * BLOCK],
        "arg 3 is LBA 3, not byte 3"
    );
}

/// 7.2: CS high ends the transaction; a command cut by CS is discarded and the
/// card answers the next full command normally.
#[test]
fn cs_release_discards_a_partial_command() {
    let mut bus = rig_with_image(&image(2));
    let mut h = Host::on(&mut bus);
    h.init();
    // First three bytes of CMD17, then CS goes high.
    for b in [0x51, 0x00, 0x00] {
        h.x(b);
    }
    h.dev.cs_release();
    h.dev.cs_select();
    assert_eq!(h.cmd(13, 0), 0x00, "fresh frame decodes as CMD13");
    assert_eq!(h.x(0xFF), 0x00, "R2 second byte");
}

/// The image comes from a host file named in the system manifest, relative to
/// the manifest's directory.
#[test]
fn image_from_a_host_file_named_in_the_manifest() {
    let dir = std::env::temp_dir().join(format!("labwired-sd-image-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let img = image(4);
    std::fs::write(dir.join("card.img"), &img).unwrap();
    let chip = repo("configs/chips/stm32f103.yaml");
    let yaml = format!(
        "name: sd-file\nchip: \"{}\"\nexternal_devices:\n  - id: sd0\n    type: microsd\n    \
         connection: spi1\n    config:\n      cs_pin: PA4\n      image: card.img\nboard_io: []\n",
        chip.display()
    );
    let path = dir.join("system.yaml");
    std::fs::write(&path, yaml).unwrap();

    let manifest = SystemManifest::from_file(&path).expect("manifest loads");
    let chip = ChipDescriptor::from_file(&chip).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let mut h = Host::on(&mut bus);
    h.init();
    assert_eq!(h.cmd(17, 2), 0x00);
    assert_eq!(h.data_block(BLOCK), img[2 * BLOCK..3 * BLOCK]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A missing image file is a load error naming the file, not an empty card.
#[test]
fn missing_image_file_is_an_error() {
    let dir = std::env::temp_dir().join(format!("labwired-sd-missing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("system.yaml");
    std::fs::write(
        &path,
        "name: sd-missing\nchip: x.yaml\nexternal_devices:\n  - id: sd0\n    type: microsd\n    \
         connection: spi1\n    config:\n      image: nope.img\nboard_io: []\n",
    )
    .unwrap();
    let err = SystemManifest::from_file(&path).expect_err("missing image must fail");
    assert!(format!("{err:#}").contains("nope.img"), "{err:#}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An explicit capacity smaller than the image cannot hold it.
#[test]
fn capacity_smaller_than_the_image_is_rejected() {
    let chip_path = repo("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_yaml(&format!(
        "name: sd-small\nchip: \"{}\"\nexternal_devices: []\nboard_io: []\n",
        chip_path.display()
    ))
    .unwrap();
    manifest.external_devices.push(sd_device(&[
        ("image_base64", b64(&image(2 * 2048)).into()),
        ("capacity_mib", 1.into()),
    ]));
    assert!(SystemBus::from_config(&chip, &manifest).is_err());
}

/// In-memory bytes through the Rust API.
#[test]
fn from_image_serves_in_memory_bytes() {
    use labwired_core::peripherals::components::microsd::MicroSd;
    let img = image(3);
    let mut card: Box<dyn SpiDevice> = Box::new(MicroSd::from_image("PA4", img.clone()).unwrap());
    let mut h = Host { dev: &mut card };
    h.dev.cs_select();
    h.init();
    assert_eq!(h.cmd(17, 1), 0x00);
    assert_eq!(h.data_block(BLOCK), img[BLOCK..2 * BLOCK]);
}

/// 4.3.5 erase (CMD32/CMD33/CMD38): erased blocks read as the SCR's
/// DATA_STAT_AFTER_ERASE value (0 for this card), image blocks included.
#[test]
fn cmd38_erase_zeroes_image_blocks() {
    let img = image(4);
    let mut bus = rig_with_image(&img);
    let mut h = Host::on(&mut bus);
    h.init();
    assert_eq!(h.cmd(32, 1), 0x00);
    assert_eq!(h.cmd(33, 2), 0x00);
    assert_eq!(h.cmd(38, 0), 0x00);
    while h.x(0xFF) == 0x00 {}
    for lba in [1u32, 2] {
        assert_eq!(h.cmd(17, lba), 0x00);
        assert_eq!(h.data_block(BLOCK), vec![0u8; BLOCK], "lba {lba} erased");
    }
    assert_eq!(h.cmd(17, 3), 0x00);
    assert_eq!(h.data_block(BLOCK), img[3 * BLOCK..4 * BLOCK], "lba 3 kept");
}
