// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! AT24C256: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/at24c256.rs` is reproduced verbatim below as
//! [`legacy`] — the wire behaviour only, with the kit wrapper stripped — and
//! both models are driven through the SAME I²C script. Where the transcripts
//! must be identical they are asserted equal byte for byte; the one place they
//! must differ is asserted as a difference, by name, so neither direction can
//! be changed in silence.
//!
//! This part is the reason Tier 1 grew `pointer_width` and `write_page`: an
//! EEPROM has no named registers, its address is TWO bytes, and a sequential
//! write wraps inside a page. None of that could be said in YAML before.
//!
//! Held identical:
//!   * the two-byte address pointer (high byte first) and where it lands;
//!   * the erased state — every cell reads 0xFF before anything is written;
//!   * byte write, random read, sequential read, and the pointer's rollover
//!     into the 256-byte modelled window.
//!
//! Deliberately DIFFERENT:
//!   * `write_page: 64`. The hand model let a sequential write run straight
//!     through the array; real silicon wraps it to the start of the same
//!     64-byte page. A driver whose record straddles a page boundary therefore
//!     passed against the model and corrupts its own data on hardware.
//!
//! Scripts are driven through Phase A's shared harness
//! (`tests/common/transcript.rs`), the same one the byte-parity ratchet uses —
//! one vocabulary for "drive a device and collect what it put on the wire".

mod common;

use common::transcript::{run_i2c, script, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;

const ADDR: u8 = 0x50;

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/at24c256.rs` at `origin/main`,
/// wire behaviour only. This is the ONLY place the old behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    const MEM_SIZE: usize = 256;

    pub struct At24c256 {
        address: u8,
        mem: [u8; MEM_SIZE],
        ptr: u16,
        addr_bytes: u8,
    }

    impl At24c256 {
        pub fn new(address: u8) -> Self {
            Self {
                address,
                mem: [0xFF; MEM_SIZE],
                ptr: 0,
                addr_bytes: 0,
            }
        }
    }

    impl I2cDevice for At24c256 {
        fn address(&self) -> u8 {
            self.address
        }

        fn start(&mut self) {
            self.addr_bytes = 0;
        }

        fn write(&mut self, data: u8) {
            match self.addr_bytes {
                0 => {
                    self.ptr = (data as u16) << 8;
                    self.addr_bytes = 1;
                }
                1 => {
                    self.ptr = (self.ptr & 0xFF00) | data as u16;
                    self.addr_bytes = 2;
                }
                _ => {
                    let idx = (self.ptr as usize) % MEM_SIZE;
                    self.mem[idx] = data;
                    self.ptr = self.ptr.wrapping_add(1);
                }
            }
        }

        fn read(&mut self) -> u8 {
            let idx = (self.ptr as usize) % MEM_SIZE;
            let v = self.mem[idx];
            self.ptr = self.ptr.wrapping_add(1);
            v
        }
    }
}

// ─── the script vocabulary ─────────────────────────────────────────────────

/// Byte or page write: START, address (two bytes, high first), data, STOP.
fn write_at(addr: u16, data: &[u8]) -> Vec<Step<'static>> {
    let mut steps = vec![
        Step::Start,
        Step::Write((addr >> 8) as u8),
        Step::Write(addr as u8),
    ];
    steps.extend(data.iter().map(|&b| Step::Write(b)));
    steps.push(Step::Stop);
    steps
}

/// Random read: START, address, repeated START, `n` bytes, STOP.
fn read_at(addr: u16, n: usize) -> Vec<Step<'static>> {
    vec![
        Step::Start,
        Step::Write((addr >> 8) as u8),
        Step::Write(addr as u8),
        Step::Start,
        Step::Read(n),
        Step::Stop,
    ]
}

/// Current-address (sequential) read continuing from wherever the pointer is.
fn read_more(n: usize) -> Vec<Step<'static>> {
    vec![Step::Start, Step::Read(n), Step::Stop]
}

fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("at24c256")
        .expect("at24c256 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("at24c256.yaml does not build")
}

/// Run one script through both models and return `(old, new)`.
fn both(script: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::At24c256::new(ADDR);
    let mut new = declarative();
    (
        run_i2c(&mut old, script).bytes,
        run_i2c(&mut new, script).bytes,
    )
}

/// Assert byte-for-byte parity, and that the script actually produced bytes —
/// two empty transcripts are equal and prove nothing.
fn assert_parity(name: &str, script: &[Step<'_>]) -> Vec<u8> {
    let (old, new) = both(script);
    assert!(!old.is_empty(), "{name}: the script read no bytes at all");
    assert_eq!(old, new, "{name}: the YAML model changed the transcript");
    new
}

// ─── held identical ────────────────────────────────────────────────────────

#[test]
fn an_erased_part_reads_all_ones() {
    // `mem: [0xFF; 256]` in the old model; `fill: 0xFF` in the descriptor.
    // Without the new `fill` key the array would power up all-zero, which is a
    // value a blank EEPROM never reads.
    let bytes = assert_parity("erased", &read_at(0x0000, 8));
    assert_eq!(bytes, vec![0xFF; 8]);
}

#[test]
fn a_byte_write_then_a_random_read_round_trips() {
    // The old model's own unit test, as a two-model comparison.
    let bytes = assert_parity(
        "byte write",
        &script([write_at(0x0010, &[0xAB]), read_at(0x0010, 1)]),
    );
    assert_eq!(bytes, vec![0xAB]);
}

#[test]
fn the_pointer_takes_two_address_bytes_high_first() {
    // THE case `pointer_width: 2` exists for. A one-byte pointer would read the
    // low address byte as DATA, so this write would land at 0x0034 and the read
    // would answer 0xFF.
    let bytes = assert_parity(
        "16-bit pointer",
        &script([write_at(0x0034, &[0x5A]), read_at(0x0034, 1)]),
    );
    assert_eq!(bytes, vec![0x5A]);
    let (_, new) = both(&script([write_at(0x0034, &[0x5A]), read_at(0x0012, 1)]));
    assert_eq!(new, vec![0xFF], "0x0012 must be untouched");
}

#[test]
fn a_sequential_read_walks_the_pointer() {
    let bytes = assert_parity(
        "sequential read",
        &script([
            write_at(0x0020, &[1, 2, 3, 4]),
            read_at(0x0020, 2),
            read_more(2),
        ]),
    );
    assert_eq!(bytes, vec![1, 2, 3, 4]);
}

#[test]
fn the_modelled_window_wraps_every_256_addresses() {
    // The 256-byte window is an approximation carried over unchanged: 0x0110 is
    // the same cell as 0x0010. Asserted so widening it is a deliberate act.
    let bytes = assert_parity(
        "window wrap",
        &script([write_at(0x0110, &[0x7E]), read_at(0x0010, 1)]),
    );
    assert_eq!(bytes, vec![0x7E]);
}

#[test]
fn a_write_that_stays_inside_one_page_is_unchanged() {
    // 0x30..0x37 is wholly inside page 0 (0x00..0x3F), so `write_page` cannot
    // show up here — the negative control for the page-wrap test below.
    let bytes = assert_parity(
        "intra-page write",
        &script([
            write_at(0x0030, &[0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7]),
            read_at(0x0030, 8),
        ]),
    );
    assert_eq!(bytes, vec![0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7]);
}

// ─── deliberately different ────────────────────────────────────────────────

#[test]
fn a_write_across_a_page_boundary_wraps_inside_the_page() {
    // THE deliberate change. Six bytes from 0x3E cross the 64-byte page
    // boundary at 0x40. Silicon wraps the last four to 0x00..0x03; the hand
    // model wrote them to 0x40..0x43, so a driver that straddles a page passed
    // against the model and corrupts its own record on hardware.
    let steps = script([
        write_at(0x003E, &[0xD0, 0xD1, 0xD2, 0xD3, 0xD4, 0xD5]),
        read_at(0x003E, 6), // 0x3E, 0x3F, then 0x40..0x43
        read_at(0x0000, 4), // the start of the page the write wrapped into
    ]);
    let (old, new) = both(&steps);

    assert_eq!(
        old,
        vec![
            0xD0, 0xD1, 0xD2, 0xD3, 0xD4, 0xD5, // the old model ran straight on
            0xFF, 0xFF, 0xFF, 0xFF,
        ],
        "the model this replaces must be recorded as it was"
    );
    assert_eq!(
        new,
        vec![
            0xD0, 0xD1, 0xFF, 0xFF, 0xFF, 0xFF, // 0x40.. untouched
            0xD2, 0xD3, 0xD4, 0xD5, // wrapped to the start of page 0
        ],
        "a page write must wrap to the start of its own page"
    );
    assert_ne!(old, new, "this difference is the point of write_page");
}
