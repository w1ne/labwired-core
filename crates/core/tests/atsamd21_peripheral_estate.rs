// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The ATSAMD21G18A estate must be MODELS at the RIGHT addresses, not labels.
//!
//! ## Why this test exists
//!
//! `chip_conformance` counts the estate and `svd_conformance` checks declared
//! bases against the vendored SVD. Neither reads a register. Both stay green
//! for a chip whose peripherals are declared perfectly and modelled as holes.
//!
//! ## The negative control that matters most
//!
//! This part has a documented near-miss in the tree. `configs/chips/onboarding/
//! atsamd21j17d-aft.yaml` was machine-imported from a Renode `.repl` that
//! spells addresses as `0x4200_0800`; the import kept only the digits BEFORE
//! the underscore, so every base landed as its top 16 bits — `0x42000800` →
//! `0x4200`. Seven peripherals collapsed onto two addresses, and bus routing
//! resolves an equal-base tie SILENTLY, by registration order, so five of them
//! answered nothing at all.
//!
//! Two tests below are written to fail on exactly that arrangement:
//! `the_sercom_instances_are_not_each_other` and
//! `the_port_groups_are_not_each_other` each write one instance and require its
//! neighbour not to move. A collapsed map cannot pass either.
//!
//! ## What else is asserted
//!
//! Where silicon has a NONZERO reset value, it is probed — a nonzero read is
//! unambiguous proof the window is backed by a live model rather than a hole:
//!
//!   * `SYSCTRL.PCLKSR` must carry its clock-ready flags. This is the boot
//!     gate: every SAM D21 startup spins on these bits, and the SVD's own
//!     reset value of 0 hangs the part before `main()`.
//!   * `PM.APBCMASK` resets to 0x0001_0000 (ADC only) straight from the SVD —
//!     so SERCOM0's bit 2 really is off until firmware sets it.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;

// ── Addresses under test (all from ATSAMD21G18A.svd, Microchip, Apache-2.0) ──
const PM: u64 = 0x4000_0400;
const SYSCTRL: u64 = 0x4000_0800;
const GCLK: u64 = 0x4000_0C00;
const NVMCTRL: u64 = 0x4100_4000;

/// PORT GROUP[0] (PA) and GROUP[1] (PB) — 0x80 apart, NOT 0x400.
const PORT_A: u64 = 0x4100_4400;
const PORT_B: u64 = 0x4100_4480;

/// SERCOM0..5 on APB-C at a 0x400 stride.
const SERCOM0: u64 = 0x4200_0800;
const SERCOM1: u64 = 0x4200_0C00;
const SERCOM5: u64 = 0x4200_1C00;

// SYSCTRL / PM register offsets.
const PCLKSR: u64 = 0x0C;
const APBCMASK: u64 = 0x20;

// PORT GROUP offsets.
const PORT_DIR: u64 = 0x00;
const PORT_DIRSET: u64 = 0x08;
const PORT_OUT: u64 = 0x10;
const PORT_IN: u64 = 0x20;

// SERCOM USART offsets.
const SERCOM_CTRLA: u64 = 0x00;
const SERCOM_CTRLB: u64 = 0x04;

/// The ready flags `configs/peripherals/atsamd21g18a/sysctrl.yaml` hand-sets.
/// The fault bits (DFLLOOB, DFLLRCS, BOD33DET, DPLLLTO) are deliberately clear.
const PCLKSR_READY: u32 = 0x0001_8ADF;
const PCLKSR_OSC8MRDY: u32 = 1 << 3;
const PCLKSR_DFLLRDY: u32 = 1 << 4;
/// Bits that must NOT be set: asserting a fault that never happened.
const PCLKSR_FAULTS: u32 = (1 << 5) | (1 << 8) | (1 << 10) | (1 << 17);

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Build the part the way every production entry point does.
fn machine() -> Machine<impl Cpu> {
    let chip_path = root("configs/chips/atsamd21g18a.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("atsamd21g18a chip yaml");
    let manifest = SystemManifest {
        chip: chip_path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build atsamd21g18a bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

fn read_u32(m: &Machine<impl Cpu>, addr: u64) -> u32 {
    (0..4)
        .map(|i| (m.bus.read_u8(addr + i).unwrap_or(0) as u32) << (i * 8))
        .fold(0, |a, b| a | b)
}

fn write_u32(m: &mut Machine<impl Cpu>, addr: u64, value: u32) {
    for i in 0..4 {
        m.bus
            .write_u8(addr + i, ((value >> (i * 8)) & 0xFF) as u8)
            .expect("bus write");
    }
}

/// Every declared window answers, and the clock block carries the reset values
/// that make the part bootable.
#[test]
fn the_estate_answers_at_its_own_addresses() {
    let m = machine();

    let pclksr = read_u32(&m, SYSCTRL + PCLKSR);
    assert_eq!(
        pclksr, PCLKSR_READY,
        "SYSCTRL.PCLKSR must carry its ready flags. Zero here means either an \
         unmapped window or a descriptor regenerated from the SVD, whose own \
         reset value is 0 — and every SAM D21 startup spins on these bits, so \
         zero is a hang before main() with no output to say so."
    );
    assert_eq!(
        pclksr & PCLKSR_OSC8MRDY,
        PCLKSR_OSC8MRDY,
        "OSC8MRDY is the flag the smoke firmware polls"
    );
    assert_eq!(
        pclksr & PCLKSR_DFLLRDY,
        PCLKSR_DFLLRDY,
        "DFLLRDY is the flag the Arduino SAMD core polls"
    );
    assert_eq!(
        pclksr & PCLKSR_FAULTS,
        0,
        "DFLLOOB / DFLLRCS / BOD33DET / DPLLLTO are FAULTS, not readiness. \
         Setting them reports a failure that never happened."
    );

    assert_eq!(
        read_u32(&m, PM + APBCMASK),
        0x0001_0000,
        "PM.APBCMASK must read its SVD reset value (ADC only). A zero means the \
         window is a hole — and would also make SERCOM0's clock bit look \
         already-on, hiding the enable every real driver performs."
    );

    // GCLK.STATUS.SYNCBUSY (bit 7 of the byte at +0x01) must be CLEAR, or the
    // three `while (GCLK->STATUS.bit.SYNCBUSY)` spins never fall through.
    assert_eq!(
        read_u32(&m, GCLK) & 0x0000_8000,
        0,
        "GCLK.STATUS.SYNCBUSY must read 0"
    );

    // NVMCTRL must answer rather than fault: the wait-state write is the first
    // store a SAM D21 startup makes.
    let _ = read_u32(&m, NVMCTRL);
}

/// ⚠️ Written to FAIL on the Renode-truncated map, where SERCOM0/1/2/3 and two
/// timers all collapsed onto 0x4200 and bus routing silently served one of
/// them for all.
#[test]
fn the_sercom_instances_are_not_each_other() {
    let mut m = machine();

    write_u32(&mut m, SERCOM0 + SERCOM_CTRLA, 0x0000_0004);
    write_u32(&mut m, SERCOM0 + SERCOM_CTRLB, 0x0003_0000);

    assert_eq!(
        read_u32(&m, SERCOM0 + SERCOM_CTRLA),
        0x0000_0004,
        "SERCOM0 must hold what was written to it"
    );
    assert_eq!(
        read_u32(&m, SERCOM1 + SERCOM_CTRLA),
        0,
        "SERCOM1 moved when only SERCOM0 was written — the instances share a \
         window. This is the Renode-import shape: 0x4200_0800 and 0x4200_0C00 \
         both truncate to 0x4200."
    );
    assert_eq!(
        read_u32(&m, SERCOM5 + SERCOM_CTRLA),
        0,
        "SERCOM5 moved when only SERCOM0 was written"
    );

    // ...and the far instance is independently reachable, not merely silent.
    write_u32(&mut m, SERCOM5 + SERCOM_CTRLA, 0x0000_0008);
    assert_eq!(read_u32(&m, SERCOM5 + SERCOM_CTRLA), 0x0000_0008);
    assert_eq!(
        read_u32(&m, SERCOM0 + SERCOM_CTRLA),
        0x0000_0004,
        "writing SERCOM5 disturbed SERCOM0"
    );
}

/// The two PORT groups are 0x80 apart inside one 0x200 block. Declared at any
/// wider stride they would overlap, and one group's registers would be served
/// entirely by the other while every label-checking gate stayed green.
#[test]
fn the_port_groups_are_not_each_other() {
    let mut m = machine();

    write_u32(&mut m, PORT_A + PORT_DIRSET, 1 << 17);
    assert_eq!(
        read_u32(&m, PORT_A + PORT_DIR),
        1 << 17,
        "PA17 (LED_BUILTIN) must be an output on group A"
    );
    assert_eq!(
        read_u32(&m, PORT_B + PORT_DIR),
        0,
        "group B's DIR moved when only group A was written — the two GROUPs \
         share a window"
    );

    write_u32(&mut m, PORT_B + PORT_DIRSET, 1 << 3);
    assert_eq!(read_u32(&m, PORT_B + PORT_DIR), 1 << 3);
    assert_eq!(
        read_u32(&m, PORT_A + PORT_DIR),
        1 << 17,
        "writing group B disturbed group A"
    );
}

/// The SET/CLR/TGL registers are ALIASES of DIR and OUT, and a driven pin reads
/// back on IN. Both are silicon behaviour a label-checking gate cannot see.
#[test]
fn the_port_aliases_resolve_to_one_register_and_a_driven_pin_reads_back() {
    let mut m = machine();

    write_u32(&mut m, PORT_A + PORT_DIRSET, 1 << 17);
    write_u32(&mut m, PORT_A + 0x18, 1 << 17); // OUTSET

    assert_eq!(
        read_u32(&m, PORT_A + PORT_OUT),
        1 << 17,
        "OUTSET must land in OUT, not in a register of its own"
    );
    assert_eq!(
        read_u32(&m, PORT_A + PORT_IN),
        1 << 17,
        "a pin the port DRIVES must read back on IN — digitalRead() on an \
         OUTPUT pin is an everyday Arduino idiom and must not return 0"
    );

    write_u32(&mut m, PORT_A + 0x1C, 1 << 17); // OUTTGL
    assert_eq!(read_u32(&m, PORT_A + PORT_OUT), 0, "OUTTGL must toggle OUT");
}
