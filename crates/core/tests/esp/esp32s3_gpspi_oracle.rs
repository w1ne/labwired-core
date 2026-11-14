// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Executing oracle for the ESP32-S3 GP-SPI2/3 controller (`esp32s3/gpspi.rs`,
//! `Esp32s3Spi`) CPU / data-buffer transfer path.
//!
//! The S3 had no SPI executing coverage in the ratchet-discoverable `tests/`
//! tree. This drives the controller the way an esp-hal CPU-mode `SpiBus`
//! transfer does — stage the MOSI payload into the W0.. data buffer, program
//! the transfer length in `MS_DLEN` (bits − 1), then set `CMD.usr` — and
//! asserts the transfer engine actually EXECUTES: `usr` auto-clears, the
//! `TRANS_DONE` interrupt latches, and the MISO half of the data buffer is
//! filled for exactly the programmed byte count (0xFF with no device on the
//! bus, matching real fast-read-with-no-slave behaviour). A length-decode or
//! completion-latch regression fails here.
//!
//! The full-duplex MOSI-capture / MISO-injection path (attached `SpiDevice` +
//! DMA pump) is exercised by the in-module `gpspi.rs` tests; the device-attach
//! entry point is crate-private, so this integration oracle drives the
//! externally reachable CPU path.
//!
//! Named `esp32s3_*` so the board-coverage ratchet discovers it.

use labwired_core::peripherals::esp32s3::gpspi::Esp32s3Spi;
use labwired_core::Peripheral;

const CMD: u64 = 0x00;
const MS_DLEN: u64 = 0x1C;
const DMA_INT_RAW: u64 = 0x3C;
const W0: u64 = 0x98;

const USR: u32 = 1 << 24;
const TRANS_DONE: u32 = 1 << 12;

const SPI2_SOURCE: u32 = 21;

/// An 8-byte CPU-mode transfer executes to completion: USR self-clears,
/// TRANS_DONE latches, and the MISO region is filled for all 8 bytes.
#[test]
fn gpspi_cpu_transfer_executes_and_latches_done() {
    let mut s = Esp32s3Spi::new(SPI2_SOURCE);

    // Stage MOSI bytes DE AD BE EF | 11 22 33 44 into W0/W1.
    s.write_u32(W0, 0xEFBE_ADDE).unwrap();
    s.write_u32(W0 + 4, 0x4433_2211).unwrap();

    // 8-byte (64-bit) transfer: MS_DLEN holds bits − 1.
    s.write_u32(MS_DLEN, 64 - 1).unwrap();

    // No completion latched before the transfer is kicked.
    assert_eq!(s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE, 0);

    // Kick.
    s.write_u32(CMD, USR).unwrap();

    // CPU-path completion is synchronous.
    assert_eq!(
        s.read_u32(CMD).unwrap() & USR,
        0,
        "USR must auto-clear once the transfer completes"
    );
    assert_ne!(
        s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
        0,
        "TRANS_DONE must latch after the transfer"
    );

    // With no device on the bus the MISO half of the buffer reads back 0xFF for
    // exactly the two words (8 bytes) that were clocked.
    assert_eq!(
        s.read_u32(W0).unwrap(),
        0xFFFF_FFFF,
        "clocked bytes read back as 0xFF MISO"
    );
    assert_eq!(s.read_u32(W0 + 4).unwrap(), 0xFFFF_FFFF);
}

/// The programmed transfer length bounds how many bytes the engine clocks: a
/// 3-byte transfer fills only the first three MISO bytes, leaving the 4th
/// buffer byte untouched.
#[test]
fn gpspi_transfer_length_bounds_clocked_bytes() {
    let mut s = Esp32s3Spi::new(SPI2_SOURCE);

    // Fill the word so an over-run into the 4th byte is visible.
    s.write_u32(W0, 0xFFFF_FFFF).unwrap();
    s.write_u32(MS_DLEN, 24 - 1).unwrap(); // 3 bytes
    s.write_u32(CMD, USR).unwrap();

    assert_ne!(s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE, 0);

    // Only the 3 clocked bytes read back as 0xFF MISO; the 4th byte is NOT
    // clocked (the engine does not touch it as a MISO byte), so the low 3 bytes
    // are 0xFF and the top byte is not.
    let w0 = s.read_u32(W0).unwrap();
    assert_eq!(
        w0 & 0x00FF_FFFF,
        0x00FF_FFFF,
        "the 3 clocked bytes read back as 0xFF MISO"
    );
    assert_ne!(
        (w0 >> 24) & 0xFF,
        0xFF,
        "the 4th byte must not be clocked by a 3-byte transfer"
    );
}
