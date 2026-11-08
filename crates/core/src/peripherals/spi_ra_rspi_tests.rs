//! Renesas RA RSPI (`profile: ra_rspi`) — the register sequence
//! ArduinoCore-renesas 1.6.0 `ArduinoSPI::configSpi` / `transfer` runs on the
//! RA4M1 (UNO R4) SPI0.

use super::*;
use crate::Peripheral;
use std::sync::{Arc, Mutex};

/// Answers with the complement of each byte and records what it was sent.
struct Echo(Arc<Mutex<Vec<u8>>>);

impl SpiDevice for Echo {
    fn cs_pin(&self) -> &str {
        ""
    }
    fn transfer(&mut self, byte: u8) -> u8 {
        self.0.lock().unwrap().push(byte);
        !byte
    }
}

fn controller() -> (Spi, Arc<Mutex<Vec<u8>>>) {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::RaRspi);
    let seen = Arc::new(Mutex::new(Vec::new()));
    spi.push_device(Box::new(Echo(seen.clone())));
    (spi, seen)
}

/// `configSpi` at MSBFIRST / mode 0, 8-bit: SPCR=0, SPCMD0 SPB=7, SPDCR2
/// BYSW=1 (`~bit_order`), SPSR cleared, MSTR, then SPE.
fn configured() -> (Spi, Arc<Mutex<Vec<u8>>>) {
    let (mut spi, seen) = controller();
    spi.write(RSPI_SPCR, 0).unwrap();
    spi.write_u16(RSPI_SPCMD0, 0x0700).unwrap();
    spi.write(RSPI_SPDCR2, RSPI_SPDCR2_BYSW).unwrap();
    let _ = spi.read(RSPI_SPSR).unwrap();
    spi.write(RSPI_SPSR, 0).unwrap();
    spi.write(RSPI_SPCR, RSPI_SPCR_MSTR).unwrap();
    spi.write(RSPI_SPCR, RSPI_SPCR_MSTR | RSPI_SPCR_SPE)
        .unwrap();
    (spi, seen)
}

fn spsr(spi: &Spi) -> u8 {
    spi.read(RSPI_SPSR).unwrap()
}

#[test]
fn reset_values_match_the_manual() {
    let (spi, _) = controller();
    assert_eq!(spsr(&spi), 0x20, "SPSR resets to SPTEF only");
    assert_eq!(spi.read(RSPI_SPBR).unwrap(), 0xFF);
    assert_eq!(spi.read_u16(RSPI_SPCMD0).unwrap(), 0x070D);
    assert_eq!(spi.read(RSPI_SPCR).unwrap(), 0);
}

/// The Arduino byte path: SPDR_BY store, spin on SPRF, SPDR_BY load.
#[test]
fn byte_transfer_sets_sprf_and_the_read_clears_it() {
    let (mut spi, seen) = configured();
    assert_eq!(spsr(&spi) & RSPI_SPSR_SPRF, 0);
    spi.write(RSPI_SPDR, 0x5A).unwrap();
    assert_ne!(spsr(&spi) & RSPI_SPSR_SPRF, 0);
    assert_ne!(spsr(&spi) & RSPI_SPSR_SPTEF, 0);
    assert_eq!(spi.read(RSPI_SPDR).unwrap(), !0x5Au8);
    assert_eq!(spsr(&spi) & RSPI_SPSR_SPRF, 0, "SPDR read takes the buffer");
    assert_eq!(*seen.lock().unwrap(), vec![0x5A]);
}

/// A unit left disabled (or in slave mode) clocks nothing: SPRF never rises
/// and the Arduino spin hangs, as on silicon.
#[test]
fn no_frame_without_spe_or_mstr() {
    let (mut spi, seen) = controller();
    spi.write(RSPI_SPDR, 0x11).unwrap();
    spi.write(RSPI_SPCR, RSPI_SPCR_SPE).unwrap(); // slave
    spi.write(RSPI_SPDR, 0x22).unwrap();
    assert_eq!(spsr(&spi) & RSPI_SPSR_SPRF, 0);
    assert!(seen.lock().unwrap().is_empty());
}

/// SPE 0 -> 1 raises SPTEF even after firmware cleared SPSR to 0.
#[test]
fn enabling_raises_sptef() {
    let (mut spi, _) = controller();
    spi.write(RSPI_SPSR, 0).unwrap();
    assert_eq!(spsr(&spi), 0);
    spi.write(RSPI_SPCR, RSPI_SPCR_MSTR | RSPI_SPCR_SPE)
        .unwrap();
    assert_eq!(spsr(&spi) & RSPI_SPSR_SPTEF, RSPI_SPSR_SPTEF);
}

/// One 32-bit SPDR store is ONE frame of four bytes. With BYSW the lowest
/// memory byte goes out first (the `transfer(buf, n)` word path).
#[test]
fn word_frame_with_byte_swap_is_one_frame_in_memory_order() {
    let (mut spi, seen) = configured();
    spi.write_u16(RSPI_SPCMD0, 0x0200).unwrap(); // SPB = 32 bits
    spi.write_u32(RSPI_SPDR, 0x4433_2211).unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![0x11, 0x22, 0x33, 0x44]);
    assert_eq!(spi.read_u32(RSPI_SPDR).unwrap(), !0x4433_2211u32);
}

#[test]
fn word_frame_without_byte_swap_is_msb_first() {
    let (mut spi, seen) = configured();
    spi.write(RSPI_SPDCR2, 0).unwrap();
    spi.write_u16(RSPI_SPCMD0, 0x0200).unwrap();
    spi.write_u32(RSPI_SPDR, 0x4433_2211).unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![0x44, 0x33, 0x22, 0x11]);
}

/// LSBF: the wire carries the frame bit-reversed, so an MSB-first device
/// sees the reversed byte, and its answer is reversed back.
#[test]
fn lsb_first_reverses_the_wire_bits() {
    let (mut spi, seen) = configured();
    spi.write_u16(RSPI_SPCMD0, 0x0700 | RSPI_SPCMD_LSBF)
        .unwrap();
    spi.write(RSPI_SPDR, 0x01).unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![0x80]);
    assert_eq!(spi.read(RSPI_SPDR).unwrap(), !0x01u8);
}

/// TXMD (transmit-only): the frame is clocked, SPRF never set.
#[test]
fn transmit_only_never_sets_sprf() {
    let (mut spi, seen) = configured();
    spi.write(RSPI_SPCR, RSPI_SPCR_MSTR | RSPI_SPCR_SPE | RSPI_SPCR_TXMD)
        .unwrap();
    spi.write(RSPI_SPDR, 0x33).unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![0x33]);
    assert_eq!(spsr(&spi) & RSPI_SPSR_SPRF, 0);
}

/// A second frame before SPDR is read: OVRF, and the buffer keeps the first.
#[test]
fn unread_frame_overruns_and_keeps_the_old_data() {
    let (mut spi, _) = configured();
    spi.write(RSPI_SPDR, 0x01).unwrap();
    spi.write(RSPI_SPDR, 0x02).unwrap();
    assert_ne!(spsr(&spi) & RSPI_SPSR_OVRF, 0);
    assert_eq!(spi.read(RSPI_SPDR).unwrap(), !0x01u8);
    // Write-0-to-clear; writing 1 leaves a clear flag clear.
    spi.write(RSPI_SPSR, !RSPI_SPSR_OVRF).unwrap();
    assert_eq!(spsr(&spi) & RSPI_SPSR_OVRF, 0);
    spi.write(RSPI_SPSR, 0xFF).unwrap();
    assert_eq!(spsr(&spi) & RSPI_SPSR_OVRF, 0);
}

#[test]
fn profile_names_parse() {
    for name in ["ra_rspi", "rspi", "renesas_rspi", "ra4m1"] {
        assert_eq!(
            name.parse::<SpiRegisterLayout>().unwrap(),
            SpiRegisterLayout::RaRspi
        );
    }
}
