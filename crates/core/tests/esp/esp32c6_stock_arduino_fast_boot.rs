//! Stock Arduino-ESP32 3.x on the ESP32-C6 through the shared builder and a
//! world node; see `tests/fixtures/esp32c6-stock-arduino-gpio2.md`.
//!
//! `labwired test` was the only runtime that mapped the app through the SPI0
//! MMU, and the only one that could find the mask ROM (in-tree file, relative
//! to the working directory). These tests pass the ROM as the `rom` blob, the
//! route a process with no core checkout has.
use crate::common;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::machine::AdvanceRequest;
use labwired_core::system::builder::{
    build_machine, BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};
use labwired_core::system::node::{build_node_with_options, NodeBuildOptions, NodeFirmware};

/// GPIO_OUT_REG (GPIO base 0x6009_1000 + 0x04).
const GPIO_OUT: u64 = 0x6009_1004;
const GPIO2: u32 = 1 << 2;
/// The sketch drives GPIO2 high near cycle 554,514.
const BUDGET: u64 = 2_000_000;
const BATCH: u64 = 50_000;

fn chip() -> ChipDescriptor {
    ChipDescriptor::from_file(common::root("configs/chips/esp32c6.yaml")).unwrap()
}

fn manifest() -> SystemManifest {
    SystemManifest::from_file(common::root("configs/systems/esp32c6-devkitc.yaml")).unwrap()
}

fn firmware() -> Vec<u8> {
    common::committed("tests/fixtures/esp32c6-stock-arduino-gpio2.elf")
}

fn rom_blob(rom: Vec<u8>) -> BlobMap {
    let mut blobs = BlobMap::new();
    blobs.insert("rom".into(), rom);
    blobs
}

fn mask_rom() -> Vec<u8> {
    common::committed("crates/core/roms/esp32c6/esp32c6_rom.bin")
}

/// Cycle count at which GPIO2 read high, or `None` when the run faulted or
/// the budget ran out first.
fn builder_gpio2_high(blobs: &BlobMap) -> Option<u64> {
    let (chip, system, firmware) = (chip(), manifest(), firmware());
    let mut built = build_machine(BuildRequest {
        chip: &chip,
        system: &system,
        firmware: FirmwareSource::Elf(&firmware),
        boot: BootMode::FastBoot,
        blobs,
        options: BuildOptions::default(),
    })
    .unwrap();
    let mut cycles = 0;
    while cycles < BUDGET {
        if built
            .machine
            .advance(AdvanceRequest::run(Some(BATCH)))
            .is_err()
        {
            return None;
        }
        cycles += BATCH;
        if built.machine.bus_read_u32(GPIO_OUT).unwrap() & GPIO2 != 0 {
            return Some(cycles);
        }
    }
    None
}

#[test]
fn builder_boots_stock_arduino_c6_with_rom_blob() {
    let at = builder_gpio2_high(&rom_blob(mask_rom()));
    assert!(at.is_some(), "GPIO2 stayed low for {BUDGET} cycles");
}

/// Negative control: the same build with a zero ROM window never reaches
/// `setup()`, so the pass above is the ROM blob and not the sketch.
#[test]
fn builder_stock_arduino_c6_needs_the_rom() {
    let at = builder_gpio2_high(&rom_blob(vec![0; 16]));
    assert_eq!(at, None, "GPIO2 went high with a zero ROM window");
}

#[test]
fn world_node_boots_stock_arduino_c6_with_rom_blob() {
    let (chip, system) = (chip(), manifest());
    let blobs = rom_blob(mask_rom());
    let mut machine = build_node_with_options(
        "c6",
        &chip,
        &system,
        NodeFirmware::Elf(firmware()),
        &NodeBuildOptions {
            blobs: Some(&blobs),
            ..Default::default()
        },
    )
    .unwrap();
    let mut cycles = 0;
    let mut high = false;
    while cycles < BUDGET && !high {
        machine.advance_to_cycle(cycles + BATCH).unwrap();
        cycles += BATCH;
        high = u32::from(machine.read_u8(GPIO_OUT).unwrap()) & GPIO2 != 0;
    }
    assert!(high, "GPIO2 stayed low for {BUDGET} cycles");
}
