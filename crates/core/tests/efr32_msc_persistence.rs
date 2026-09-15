// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Firmware can write its own flash on efr32mg26 — the persistence path.
//!
//! Until MSC was mapped, a store to a flash address FAULTED the bus, so no
//! project on this board could remember anything across a reset. This drives
//! the controller the way a vendor flash routine does — WREN, ADDRB, WDATA,
//! poll BUSY — and then reads the word back through ordinary memory, which is
//! the only thing that proves the write reached flash rather than a register.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

const MSC: u64 = 0x4003_0000;
const MSC_WRITECTRL: u64 = MSC + 0x00C;
const MSC_WRITECMD: u64 = MSC + 0x010;
const MSC_ADDRB: u64 = MSC + 0x014;
const MSC_WDATA: u64 = MSC + 0x018;
const MSC_STATUS: u64 = MSC + 0x01C;
const WREN: u32 = 1 << 0;
const ERASEPAGE: u32 = 1 << 1;
const BUSY: u32 = 1 << 0;
const INVADDR: u32 = 1 << 2;
const LOCKED: u32 = 1 << 1;

/// The user-data page (RM Table 6.1 p.60) — where a project's settings belong.
const USERDATA: u32 = 0x0FE0_0000;

/// CMU_CLKEN1 (RM section 8.4 p.174, offset 0x068), MSC is bit 16 (p.187).
const CMU_CLKEN1: u64 = 0x4000_8068;
const CLKEN1_MSC: u32 = 1 << 16;

/// A bus with MSC clocked, which is the only state in which it answers.
///
/// ⚠️ IT ASSERTS THAT IT WORKED. An unclocked MSC reads 0 at every offset, and
/// 0 satisfies "BUSY is clear" and "no INVADDR" and every other
/// absence-shaped assertion in this file — so a test that forgot the clock
/// PASSES while proving nothing. That happened here: one test was left on the
/// bare `bus()` and its erase assertions went green against a silent
/// peripheral. Checking the clock took at the one place the clock is enabled
/// is what makes that unrepeatable.
fn clocked_bus() -> SystemBus {
    let mut b = bus();
    b.write_u32(CMU_CLKEN1, CLKEN1_MSC).unwrap();
    assert_ne!(
        b.read_u32(MSC_STATUS).unwrap_or(0),
        0,
        "MSC did not come up clocked — every assertion after this would pass on \
         a silent peripheral"
    );
    b
}

fn bus() -> SystemBus {
    let sys = repo("examples/brd2709a/agent-deck-system.yaml");
    let manifest = SystemManifest::from_file(&sys).expect("load the deck manifest");
    let chip = ChipDescriptor::from_file(sys.parent().unwrap().join(&manifest.chip))
        .expect("load efr32mg26");
    SystemBus::from_config(&chip, &manifest).expect("build the bus")
}

/// Run the machinery that carries an armed MSC command to completion.
fn settle(bus: &mut SystemBus) {
    // The bus-tick pass is what lends MSC the bus; four is generous for a
    // one-tick operation and keeps the test honest about needing ANY.
    for _ in 0..4 {
        bus.config.peripheral_tick_interval = 1;
        bus.tick_peripherals_fully_forced();
    }
}

/// ⚠️ MSC IS SILENT UNTIL CMU CLOCKS IT — and that is not a modelling choice,
/// it is what the die does. On a connected BRD2709A a cold `reset halt` read of
/// 0x40030000 over SWD FAILS OUTRIGHT; the capture that produced the reset
/// values below needed a CMU_CLKEN preamble. A twin that answered unclocked
/// would let a driver that forgot its clock enable pass here and hang on the
/// bench.
#[test]
fn msc_is_silent_until_cmu_clocks_it() {
    let b = bus();
    assert_eq!(
        b.read_u32(MSC_STATUS).unwrap_or(0),
        0,
        "an unclocked MSC must not answer with its reset values"
    );
}

#[test]
fn msc_answers_at_its_silicon_base() {
    let b = clocked_bus();
    // The whole point: this address used to fault.
    let status = b
        .read_u32(MSC_STATUS)
        .expect("MSC must answer at 0x40030000");
    assert_eq!(
        status, 0x0B00_0008,
        "the reset STATUS must be the one measured on the die (WREADY | PWRON1 | \
         PWRON0 | WDATAREADY), not the manual's 0x08000008"
    );
}

#[test]
fn firmware_writes_a_word_into_flash_and_reads_it_back() {
    let mut b = clocked_bus();

    // The vendor sequence: enable the controller, point it at a page, erase.
    b.write_u32(MSC_WRITECTRL, WREN).unwrap();
    b.write_u32(MSC_ADDRB, USERDATA).unwrap();
    b.write_u32(MSC_WRITECMD, ERASEPAGE).unwrap();
    settle(&mut b);
    assert_eq!(
        b.read_u32(MSC_STATUS).unwrap() & BUSY,
        0,
        "BUSY must clear once the erase lands"
    );
    assert_eq!(
        b.read_u32(USERDATA as u64).unwrap(),
        0xFFFF_FFFF,
        "an erased flash word is all ones"
    );

    // Then the write itself.
    b.write_u32(MSC_ADDRB, USERDATA).unwrap();
    b.write_u32(MSC_WDATA, 0xCAFE_F00D).unwrap();
    assert_eq!(
        b.read_u32(MSC_STATUS).unwrap() & BUSY,
        BUSY,
        "BUSY must be observable while the write is outstanding — every vendor \
         flash routine polls exactly this"
    );
    settle(&mut b);

    assert_eq!(
        b.read_u32(USERDATA as u64).unwrap(),
        0xCAFE_F00D,
        "the word must be readable through ordinary memory, which is what says \
         it reached FLASH and not just a register"
    );
}

/// ⚠️ Flash programming can only clear bits. A model that replaced the word
/// would let a driver pass here and corrupt on the bench, which is the exact
/// class of bug a twin exists to catch.
#[test]
fn a_second_write_to_an_unerased_word_ands_rather_than_replaces() {
    let mut b = clocked_bus();
    b.write_u32(MSC_WRITECTRL, WREN).unwrap();
    b.write_u32(MSC_ADDRB, USERDATA).unwrap();
    b.write_u32(MSC_WRITECMD, ERASEPAGE).unwrap();
    settle(&mut b);

    b.write_u32(MSC_ADDRB, USERDATA).unwrap();
    b.write_u32(MSC_WDATA, 0xFF00_FF00).unwrap();
    settle(&mut b);
    b.write_u32(MSC_ADDRB, USERDATA).unwrap();
    b.write_u32(MSC_WDATA, 0x0FF0_0FF0).unwrap();
    settle(&mut b);

    assert_eq!(
        b.read_u32(USERDATA as u64).unwrap(),
        0xFF00_FF00 & 0x0FF0_0FF0,
        "programming clears bits; it cannot set them without an erase"
    );
}

#[test]
fn a_write_without_wren_changes_nothing() {
    let mut b = clocked_bus();
    b.write_u32(MSC_WRITECTRL, WREN).unwrap();
    b.write_u32(MSC_ADDRB, USERDATA).unwrap();
    b.write_u32(MSC_WRITECMD, ERASEPAGE).unwrap();
    settle(&mut b);

    // Now drop WREN and try to write.
    b.write_u32(MSC_WRITECTRL, 0).unwrap();
    b.write_u32(MSC_ADDRB, USERDATA).unwrap();
    b.write_u32(MSC_WDATA, 0x1234_5678).unwrap();
    settle(&mut b);

    assert_eq!(
        b.read_u32(USERDATA as u64).unwrap(),
        0xFFFF_FFFF,
        "the erased word must be untouched"
    );
    assert_eq!(
        b.read_u32(MSC_STATUS).unwrap() & LOCKED,
        LOCKED,
        "and the controller must SAY it refused, not fail silently"
    );
}

#[test]
fn pointing_the_controller_at_ram_is_reported_as_an_invalid_address() {
    let mut b = clocked_bus();
    b.write_u32(MSC_WRITECTRL, WREN).unwrap();
    b.write_u32(MSC_ADDRB, 0x2000_0000).unwrap();
    assert_eq!(
        b.read_u32(MSC_STATUS).unwrap() & INVADDR,
        INVADDR,
        "RAM is not flash, and INVADDR is how the controller says so"
    );
}
