// SPDX-License-Identifier: MIT

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, cpu::cortex_m::CortexM, memory::ProgramImage, Arch, Bus, Machine,
};
use std::path::PathBuf;

fn root(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

// Exercise the exact board through the same GPIO registers firmware accesses.
// This is peripheral qualification, not a claim that Arcade firmware/display runs.
const BUTTONS: [(&str, u8); 8] = [
    ("left", 0),
    ("up", 1),
    ("down", 2),
    ("right", 3),
    ("select", 4),
    ("start", 5),
    ("a", 6),
    ("b", 7),
];

fn button_bus() -> SystemBus {
    let chip = ChipDescriptor::from_file(root("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    let manifest = SystemManifest::from_file(root("configs/systems/pybadge.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    drive(&mut bus, "PB31", false);
    drive(&mut bus, "PB0", false);
    bus
}

fn drive(bus: &mut SystemBus, pin: &str, high: bool) {
    let (addr, bit) = SystemBus::resolve_pin_odr_pub(bus, pin).unwrap();
    let word = bus.read_u32(addr).unwrap();
    let next = if high {
        word | (1 << bit)
    } else {
        word & !(1 << bit)
    };
    bus.write_u32(addr, next).unwrap();
}

fn data(bus: &SystemBus) -> bool {
    let (addr, bit) = SystemBus::resolve_pin_idr_pub(bus, "PB30").unwrap();
    (bus.read_u32(addr).unwrap() >> bit) & 1 != 0
}

fn press(bus: &mut SystemBus, key: &str, value: f64) {
    bus.set_input(Some("buttons"), key, value).unwrap();
}

fn read_snapshot(bus: &mut SystemBus) -> u8 {
    let mut word = 0_u8;
    for _ in 0..8 {
        word = (word << 1) | u8::from(data(bus));
        drive(bus, "PB31", true);
        drive(bus, "PB31", false);
    }
    word
}

fn latch(bus: &mut SystemBus) {
    drive(bus, "PB31", false);
    drive(bus, "PB0", false);
    drive(bus, "PB0", true);
}

#[test]
fn pybadge_buttons_all_combinations_are_active_low_and_msb_first() {
    let mut bus = button_bus();
    for mask in 0_u16..=255 {
        for (key, bit) in BUTTONS {
            press(&mut bus, key, f64::from(((mask >> bit) & 1) as u8));
        }
        latch(&mut bus);
        let before = bus.current_cycle;
        assert_eq!(
            read_snapshot(&mut bus),
            !(mask as u8),
            "pressed mask {mask:02x}"
        );
        assert_eq!(
            bus.current_cycle, before,
            "every GPIO edge works without ticks"
        );
        assert!(
            !data(&bus),
            "grounded SER fills with zero after eight clocks"
        );
        for _ in 0..3 {
            drive(&mut bus, "PB31", true);
            drive(&mut bus, "PB31", false);
            assert!(!data(&bus));
        }
    }
}

#[test]
fn pybadge_low_latch_tracks_inputs_and_ignores_clocks() {
    let mut bus = button_bus();
    assert!(data(&bus), "released B has its pull-up");
    press(&mut bus, "b", 1.0);
    // Host input rules apply immediately; their pad drives settle on the next
    // device service, here the real peripheral tick (not a direct pin injection).
    bus.tick_peripherals();
    assert!(!data(&bus));
    press(&mut bus, "b", 0.0);
    bus.tick_peripherals();
    for _ in 0..10 {
        drive(&mut bus, "PB31", true);
        assert!(data(&bus), "low SH/LD overrides rising clock");
        drive(&mut bus, "PB31", false);
        assert!(data(&bus));
    }
}

#[test]
fn pybadge_high_latch_freezes_inputs_until_reloaded() {
    let mut bus = button_bus();
    press(&mut bus, "left", 1.0);
    press(&mut bus, "a", 1.0);
    latch(&mut bus);
    // Change contacts in the middle of the frame; both word halves must still
    // come from the old snapshot. Tick cannot accidentally reload a high latch.
    let mut word = 0_u8;
    for _ in 0..4 {
        word = (word << 1) | u8::from(data(&bus));
        drive(&mut bus, "PB31", true);
        drive(&mut bus, "PB31", false);
    }
    press(&mut bus, "left", 0.0);
    press(&mut bus, "a", 0.0);
    press(&mut bus, "b", 1.0);
    bus.tick_peripherals();
    for _ in 0..4 {
        word = (word << 1) | u8::from(data(&bus));
        drive(&mut bus, "PB31", true);
        drive(&mut bus, "PB31", false);
    }
    assert_eq!(word, 0xbe);
    latch(&mut bus);
    assert_eq!(read_snapshot(&mut bus), 0x7f);
}

#[test]
fn pybadge_only_rising_clock_shifts_and_guest_cannot_write_data_input() {
    let mut bus = button_bus();
    press(&mut bus, "b", 1.0);
    latch(&mut bus);
    assert!(!data(&bus));
    drive(&mut bus, "PB31", false);
    drive(&mut bus, "PB0", true);
    drive(&mut bus, "PB1", true);
    assert!(!data(&bus), "same-level and unrelated writes do not shift");
    let (addr, _) = SystemBus::resolve_pin_idr_pub(&bus, "PB30").unwrap();
    bus.write_u32(addr, u32::MAX).unwrap();
    assert!(!data(&bus), "PORT IN is read-only to the guest");
    drive(&mut bus, "PB31", true);
    assert!(data(&bus), "first rising clock moves A to QH");
    drive(&mut bus, "PB31", true);
    drive(&mut bus, "PB31", false);
    assert!(data(&bus), "held-high and falling clock do not shift");
}

#[test]
fn pybadge_button_input_ranges_and_threshold_are_explicit() {
    let mut bus = button_bus();
    for value in [-1.0, 2.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            bus.set_input(Some("buttons"), "b", value).is_err(),
            "accepted invalid button value {value}"
        );
    }
    assert!(bus.set_input(Some("buttons"), "missing", 1.0).is_err());
    latch(&mut bus);
    assert_eq!(
        read_snapshot(&mut bus),
        0xff,
        "rejected inputs changed nothing"
    );
    for (value, expected) in [(0.499, 0xff), (0.5, 0x7f), (1.0, 0x7f)] {
        press(&mut bus, "b", value);
        latch(&mut bus);
        assert_eq!(read_snapshot(&mut bus), expected);
    }
}

#[test]
fn pybadge_system_builds_with_exact_onboard_wiring() {
    let chip = ChipDescriptor::from_file(root("configs/chips/atsamd51-pybadge.yaml"))
        .expect("SAMD51 chip descriptor");
    let manifest = SystemManifest::from_file(root("configs/systems/pybadge.yaml"))
        .expect("PyBadge system manifest");
    let bus = SystemBus::from_config(&chip, &manifest).expect("PyBadge bus builds");

    assert_eq!(chip.reset_vector_offset, 0x4000);

    for name in ["porta", "portb", "sercom1"] {
        assert!(
            bus.peripherals.iter().any(|entry| entry.name == name),
            "missing {name}"
        );
    }
    assert_eq!(
        chip.pins.get("PA15").map(|p| (p.gpio.as_str(), p.bit)),
        Some(("porta", 15))
    );
    assert_eq!(
        chip.pins.get("PB30").map(|p| (p.gpio.as_str(), p.bit)),
        Some(("portb", 30))
    );

    // MakeCode Arcade UF2 application payloads omit the resident bootloader
    // and begin with their vector table at 0x4000. Prove that this is a bootable
    // contract, not merely a loader address accepted by the YAML parser.
    let mut machine = Machine::new(CortexM::new(), bus);
    let mut image = ProgramImage::new(0x4101, Arch::Arm);
    let mut application = vec![0_u8; 0x104];
    application[0..4].copy_from_slice(&0x2000_4000_u32.to_le_bytes());
    application[4..8].copy_from_slice(&0x0000_4101_u32.to_le_bytes());
    application[0x100..0x102].copy_from_slice(&0xe7fe_u16.to_le_bytes());
    image.add_segment(0x4000, application);
    machine
        .load_firmware(&image)
        .expect("load PyBadge application at 0x4000");
    assert_eq!(machine.cpu.sp, 0x2000_4000);
    assert_eq!(machine.cpu.pc, 0x4100);
}
