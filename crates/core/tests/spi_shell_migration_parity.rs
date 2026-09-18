// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The three SPI register shells, against the hand-written models they
//! replace.**
//!
//! `components/lora_sx1278.rs`, `components/rc522.rs` and
//! `components/nrf24l01.rs` are DELETED. Between them they were 487 lines, and
//! every one of those lines was an address/data phase machine over a byte
//! array — the `register_file:` key this port adds, written three times with
//! three different masks.
//!
//! Each golden constant below is the transcript its model produced, captured by
//! running the script beside it against the model on the commit that removed
//! it. Where the descriptor deliberately differs the difference is asserted on
//! its own, with the datasheet section that decides it, so neither half can be
//! lost in a re-bless.
//!
//! ⚠️ **None of these parts has a radio, and none ever did.** What is missing —
//! the LoRa packet engine, the ISO 14443 air interface, the nRF24's TX/RX
//! FIFOs and IRQ pad — is missing in exactly the same way it was missing
//! before, and is stated in each descriptor's header rather than faked. A
//! parity test can only hold a model to what it already did.

use labwired_core::peripherals::components::declarative_spi::GenericSpiDevice;

mod common;
use common::transcript::{run_spi, script, spi_xfer, Step};

fn dev(device_type: &str) -> GenericSpiDevice {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .unwrap_or_else(|| panic!("{device_type} descriptor is not embedded"));
    GenericSpiDevice::from_yaml(yaml, "PA4").unwrap_or_else(|e| panic!("{device_type}.yaml: {e}"))
}

// ─── SX1278 / RA-02 ────────────────────────────────────────────────────────

/// The conversation the deleted `LoraSx1278` was driven through.
///
/// SX1276/77/78/79 §4.1.2: the address byte's bit 7 is **write** on this part
/// (the opposite of the ADXL345 convention), bits [6:0] are the address.
fn sx_script() -> Vec<Step<'static>> {
    script([
        spi_xfer(&[0x42, 0x00]),       // read RegVersion
        spi_xfer(&[0x01, 0x00]),       // read RegOpMode at its power-on value
        spi_xfer(&[0x81, 0x85]),       // write RegOpMode = 0x85
        spi_xfer(&[0x01, 0x00]),       // read it back
        spi_xfer(&[0xC2, 0x77]),       // write RegVersion (0x42) = 0x77
        spi_xfer(&[0x42, 0x00]),       // read RegVersion again  ⇐ DIFFERS
        spi_xfer(&[0x8D, 0x6C]),       // write 0x0D = 0x6C
        spi_xfer(&[0x0D, 0x00, 0x00]), // read 0x0D, then one byte past  ⇐ DIFFERS
        spi_xfer(&[0x7F, 0x00]),       // the top of the 128-byte map
    ])
}

/// What `components/lora_sx1278.rs` put on MISO for [`sx_script`].
const SX1278_GOLDEN: &[u8] = &[
    0x00, 0x12, 0x00, 0x09, 0x00, 0x00, 0x00, 0x85, 0x00, 0x00, 0x00, 0x77, 0x00, 0x00, 0x00, 0x6C,
    0x6C, 0x00, 0x00,
];

/// The two bytes that deliberately moved, and why. Both are at a known index of
/// [`SX1278_GOLDEN`], so this test fails if either the old value or the new one
/// stops being what it is.
const SX1278_NAMED_DIFFERENCES: &[(usize, u8, u8, &str)] = &[
    (
        11,
        0x77,
        0x12,
        "§6.4: RegVersion (0x42) is READ-ONLY and identifies the silicon. The old model \
         stored a write to it, so one stray burst made the part deny being an SX127x and \
         every RadioLib begin() after that failed.",
    ),
    (
        16,
        0x6C,
        0xFF,
        "a data byte past the end of the selected register. The old model repeated the \
         selected byte forever; the descriptor reads 0xFF (open bus) because \
         `auto_increment: false` serves exactly the word that was addressed. §4.1.2.2 says \
         silicon does neither — it auto-increments — and the walk waits for the RegFifo \
         pop that only a modelled air link could provide.",
    ),
];

#[test]
fn sx1278_register_shell_is_byte_identical_except_where_named() {
    let mut expected = SX1278_GOLDEN.to_vec();
    for (idx, was, now, _) in SX1278_NAMED_DIFFERENCES {
        assert_eq!(
            SX1278_GOLDEN[*idx], *was,
            "byte {idx} of the captured golden is no longer {was:#04X}"
        );
        expected[*idx] = *now;
    }
    let got = run_spi(&mut dev("lora-sx1278"), &sx_script());
    assert_eq!(
        got.bytes,
        expected,
        "SX1278 transcript moved.\nexpected:\n{}\ngot:\n{}",
        common::transcript::Transcript {
            bytes: expected.clone()
        }
        .render(),
        got.render()
    );
}

#[test]
fn sx1278_version_register_cannot_be_written() {
    let mut d = dev("lora-sx1278");
    let t = run_spi(
        &mut d,
        &script([spi_xfer(&[0xC2, 0x00]), spi_xfer(&[0x42, 0x00])]),
    );
    assert_eq!(
        t.bytes[3], 0x12,
        "RegVersion must still read 0x12 after a write; §6.4 makes it read-only"
    );
}

#[test]
fn sx1278_storage_addresses_hold_what_a_driver_writes() {
    let mut d = dev("lora-sx1278");
    // Every address but 0x42 is one byte of the 128-byte file.
    for addr in [0x00u8, 0x06, 0x1D, 0x39, 0x41, 0x43, 0x7F] {
        let t = run_spi(
            &mut d,
            &script([
                spi_xfer(&[0x80 | addr, addr ^ 0x5A]),
                spi_xfer(&[addr, 0x00]),
            ]),
        );
        assert_eq!(
            t.bytes[3],
            addr ^ 0x5A,
            "address {addr:#04X} did not store the byte a driver wrote"
        );
    }
}

// ─── MFRC522 / RC522 ───────────────────────────────────────────────────────

/// The conversation the deleted `Rc522` was driven through.
///
/// MFRC522 §8.1.2.3: bit 7 of the address byte is **read**, bits [6:1] are the
/// register address and bit 0 is always 0.
fn rc_script() -> Vec<Step<'static>> {
    script([
        spi_xfer(&[0x80 | (0x37 << 1), 0x00]), // read VersionReg
        spi_xfer(&[0x01 << 1, 0x0F]),          // CommandReg = SoftReset
        spi_xfer(&[0x80 | (0x01 << 1), 0x00]), // read CommandReg back: Idle
        spi_xfer(&[0x01 << 1, 0x03]),          // CommandReg = Transceive
        spi_xfer(&[0x80 | (0x01 << 1), 0x00]),
        spi_xfer(&[0x11 << 1, 0x3D]), // ModeReg = 0x3D
        spi_xfer(&[0x80 | (0x11 << 1), 0x00]),
        spi_xfer(&[0x80 | (0x0A << 1), 0x00]), // FIFOLevelReg
        spi_xfer(&[0x80 | (0x3F << 1), 0x00]), // the top of the 64-byte map
    ])
}

/// What `components/rc522.rs` put on MISO for [`rc_script`]. **Byte-identical**
/// — the descriptor reproduces it exactly, soft reset included.
const RC522_GOLDEN: &[u8] = &[
    0x00, 0x92, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x3D, 0x00, 0x00,
    0x00, 0x00,
];

#[test]
fn rc522_register_shell_is_byte_identical() {
    let got = run_spi(&mut dev("rc522"), &rc_script());
    assert_eq!(
        got.bytes,
        RC522_GOLDEN,
        "RC522 transcript moved.\ngot:\n{}",
        got.render()
    );
}

#[test]
fn rc522_soft_reset_returns_the_command_register_to_idle() {
    let mut d = dev("rc522");
    let t = run_spi(
        &mut d,
        &script([
            spi_xfer(&[0x01 << 1, 0x0F]),
            spi_xfer(&[0x80 | (0x01 << 1), 0x00]),
        ]),
    );
    assert_eq!(
        t.bytes[3], 0x00,
        "§10.3.1.9: a SoftReset command leaves CommandReg reading Idle. A CommandReg that \
         latched 0x0F is a reset a driver's poll can never see complete."
    );
}

#[test]
fn rc522_version_register_cannot_be_written() {
    let mut d = dev("rc522");
    let t = run_spi(
        &mut d,
        &script([
            spi_xfer(&[0x37 << 1, 0x00]),
            spi_xfer(&[0x80 | (0x37 << 1), 0x00]),
        ]),
    );
    assert_eq!(
        t.bytes[3], 0x92,
        "DELIBERATE DIFFERENCE. §9.3.4.8 makes VersionReg read-only; the old model stored \
         the write and then reported it, so a stray burst made PCD_DumpVersion() lie."
    );
}

// ─── nRF24L01+ ─────────────────────────────────────────────────────────────

/// The conversation the deleted `Nrf24l01` was driven through.
///
/// The three frames that are NOT register accesses are the point: §8.3.1
/// Table 19 makes `FLUSH_TX`, `W_TX_PAYLOAD` and `R_RX_PAYLOAD` opcodes whose
/// low five bits are not an address, and the burst read on either side of the
/// payload write proves the register file survived it.
fn nrf_script() -> Vec<Step<'static>> {
    script([
        spi_xfer(&[0x00, 0x00]), // R_REGISTER CONFIG
        spi_xfer(&[0x07, 0x00]), // R_REGISTER STATUS
        spi_xfer(&[0xFF]),       // NOP — STATUS on the command byte alone
        spi_xfer(&[0x20, 0x0E]), // W_REGISTER CONFIG = 0x0E   ⇐ DIFFERS
        spi_xfer(&[0x00, 0x00]), // read it back
        spi_xfer(&[0x25, 0x4C]), // W_REGISTER RF_CH = 0x4C    ⇐ DIFFERS
        spi_xfer(&[0x05, 0x00]), // read RF_CH
        spi_xfer(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]), // burst 0x00..0x06
        spi_xfer(&[0x27, 0x0E]), // W_REGISTER STATUS = 0x0E — write-1-to-clear
        spi_xfer(&[0x07, 0x00]), // STATUS is 0x00 now
        spi_xfer(&[0xE1]),       // FLUSH_TX
        spi_xfer(&[0xA0, 0xDE, 0xAD, 0xBE, 0xEF]), // W_TX_PAYLOAD
        spi_xfer(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]), // burst again
        spi_xfer(&[0x61, 0x00, 0x00]), // R_RX_PAYLOAD
        spi_xfer(&[0x17, 0x00]), // FIFO_STATUS
    ])
}

/// What `components/nrf24l01.rs` put on MISO for [`nrf_script`].
const NRF24_GOLDEN: &[u8] = &[
    0x0E, 0x08, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x4C, 0x0E, 0x0E, 0x00,
    0x03, 0x03, 0x00, 0x4C, 0x0F, 0x0E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x0E, 0x00, 0x03, 0x03, 0x00, 0x4C, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// The DATA bytes of a `W_REGISTER` frame are the only place the port moves.
const NRF24_NAMED_DIFFERENCES: &[(usize, u8, u8, &str)] = &[
    (
        6,
        0x0E,
        0x00,
        "the data byte of `W_REGISTER CONFIG`. §8.3.1 puts STATUS on MISO while the master \
         clocks the COMMAND word — `command_response: STATUS` reproduces exactly that — and \
         says nothing about MISO during the data word. The old model held STATUS out for the \
         whole frame; the descriptor drives 0x00. No RF24-style driver reads it: \
         write_register() returns the byte the command phase produced.",
    ),
    (
        10,
        0x0E,
        0x00,
        "the data byte of `W_REGISTER RF_CH`, the same sentence as above.",
    ),
];

#[test]
fn nrf24_register_shell_is_byte_identical_except_where_named() {
    let mut expected = NRF24_GOLDEN.to_vec();
    for (idx, was, now, _) in NRF24_NAMED_DIFFERENCES {
        assert_eq!(
            NRF24_GOLDEN[*idx], *was,
            "byte {idx} of the captured golden is no longer {was:#04X}"
        );
        expected[*idx] = *now;
    }
    let got = run_spi(&mut dev("nrf24l01"), &nrf_script());
    assert_eq!(
        got.bytes,
        expected,
        "nRF24L01+ transcript moved.\nexpected:\n{}\ngot:\n{}",
        common::transcript::Transcript {
            bytes: expected.clone()
        }
        .render(),
        got.render()
    );
}

#[test]
fn nrf24_status_rides_out_on_every_command_byte() {
    // §8.3.1. A NOP is the whole idiom: one byte out, STATUS back.
    let mut d = dev("nrf24l01");
    let t = run_spi(&mut d, &spi_xfer(&[0xFF]));
    assert_eq!(
        t.bytes,
        vec![0x0E],
        "NOP must answer the STATUS reset value"
    );
}

#[test]
fn nrf24_status_is_write_one_to_clear() {
    let mut d = dev("nrf24l01");
    let t = run_spi(
        &mut d,
        &script([
            spi_xfer(&[0x27, 0x08]), // clear only bit 3
            spi_xfer(&[0x07, 0x00]),
        ]),
    );
    assert_eq!(
        t.bytes[3], 0x06,
        "§9.1: writing a 1 clears that STATUS bit and leaves the others"
    );
}

/// The reason `op_mask` exists. Decoded by bit 5 alone — the only direction
/// vocabulary the engine had — `W_TX_PAYLOAD` (0xA0) is a WRITE to address
/// 0x00, so a 32-byte payload burst walks straight over CONFIG, EN_AA,
/// EN_RXADDR, SETUP_AW, SETUP_RETR, RF_CH and RF_SETUP.
#[test]
fn a_payload_burst_does_not_land_on_the_register_file() {
    let mut d = dev("nrf24l01");
    let before = run_spi(&mut d, &spi_xfer(&[0x00, 0, 0, 0, 0, 0, 0, 0]));
    run_spi(
        &mut d,
        &spi_xfer(&[0xA0, 0xDE, 0xAD, 0xBE, 0xEF, 0x11, 0x22, 0x33]),
    );
    let after = run_spi(&mut d, &spi_xfer(&[0x00, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(
        before.bytes, after.bytes,
        "W_TX_PAYLOAD is not a register access (§8.3.1 Table 19); it must leave the \
         register file untouched"
    );
}

/// The same rule on the read side: `R_RX_PAYLOAD` (0x61) addresses nothing, so
/// its data bytes are the command-response word rather than register 0x01.
#[test]
fn a_payload_read_serves_status_not_a_register() {
    let mut d = dev("nrf24l01");
    let t = run_spi(&mut d, &spi_xfer(&[0x61, 0x00, 0x00, 0x00]));
    assert_eq!(
        t.bytes,
        vec![0x0E, 0x0E, 0x0E, 0x0E],
        "R_RX_PAYLOAD must not read EN_AA as a payload byte"
    );
}

/// A burst of `R_REGISTER` walks the map one address at a time (§8.3.1: the
/// address auto-increments through a multi-byte access), and it stops at the
/// end of the file rather than wrapping.
#[test]
fn a_register_burst_walks_the_documented_map() {
    let mut d = dev("nrf24l01");
    let t = run_spi(&mut d, &spi_xfer(&[0x00, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(
        t.bytes,
        vec![0x0E, 0x08, 0x00, 0x03, 0x03, 0x00, 0x02, 0x0F],
        "CONFIG, EN_AA, EN_RXADDR, SETUP_AW, SETUP_RETR, RF_CH, RF_SETUP at their reset values"
    );
}
