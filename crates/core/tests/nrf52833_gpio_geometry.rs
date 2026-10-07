// SPDX-License-Identifier: MIT
//! Raw silicon register addresses must reach independent P0/P1 ports.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, Bus};
use labwired_core::{
    cpu::CortexM, inspect::InspectOpts, memory::ProgramImage, system::cortex_m::configure_cortex_m,
    Arch, Machine,
};
use std::path::PathBuf;

fn bus() -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join("configs/systems/microbit-v2.yaml");
    let mut manifest = SystemManifest::from_file(&path).unwrap();
    let chip_path = path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    manifest.chip = chip_path.to_str().unwrap().into();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

#[test]
fn silicon_gpio_windows_keep_all_pin_configurations_independent() {
    let mut bus = bus();
    let schema = bus.debug_schemas.get("gpio1").expect("P1 debug schema");
    assert_eq!(schema.iter().find(|r| r.name == "OUT").unwrap().offset, 4);
    assert_eq!(
        schema.iter().find(|r| r.name == "PIN_CNF5").unwrap().offset,
        0x214
    );
    for pin in 0..32 {
        bus.write_u32(0x5000_0700 + pin * 4, 0xC).unwrap();
    }
    for pin in 0..10 {
        bus.write_u32(0x5000_0A00 + pin * 4, 0x1).unwrap();
    }
    for pin in 0..32 {
        assert_eq!(bus.read_u32(0x5000_0700 + pin * 4).unwrap(), 0xC);
    }
    for pin in 0..10 {
        assert_eq!(bus.read_u32(0x5000_0A00 + pin * 4).unwrap(), 0x1);
    }
    bus.write_u32(0x5000_0518, 1 << 21).unwrap();
    bus.write_u32(0x5000_0508, 1 << 21).unwrap();
    bus.write_u32(0x5000_0808, 1 << 5).unwrap();
    assert_eq!(bus.read_u32(0x5000_0504).unwrap(), 1 << 21);
    assert_eq!(bus.read_u32(0x5000_0804).unwrap(), 1 << 5);
    assert_eq!(bus.read_u32(0x5000_0814).unwrap(), 0x3FF);
    bus.write_u32(0x5000_081C, 1 << 5).unwrap();
    assert_eq!(bus.read_u32(0x5000_0814).unwrap(), 0x3DF);
    assert_eq!(bus.read_u32(0x5000_0514).unwrap(), 1 << 21);
    bus.write_u32(0x5000_080C, 1 << 5).unwrap();
    assert_eq!(bus.read_u32(0x5000_0804).unwrap(), 0);
}

#[test]
fn compact_p1_keeps_nrf52_peripheral_pad_routing() {
    let mut bus = bus();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine: Machine<CortexM> = Machine::new(cpu, bus);
    let mut image = ProgramImage::new(0x101, Arch::Arm);
    let mut flash = vec![0u8; 0x104];
    flash[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
    flash[4..8].copy_from_slice(&0x101u32.to_le_bytes());
    flash[0x100..0x102].copy_from_slice(&0xE7FEu16.to_le_bytes());
    image.add_segment(0, flash);
    machine.load_firmware(&image).unwrap();
    machine.bus.write_u32(0x5000_0A14, 1).unwrap();
    machine.bus.write_u32(0x5000_080C, 1 << 5).unwrap();
    let idx = machine.bus.find_peripheral_index_by_name("gpio1").unwrap();
    assert_eq!(
        machine.bus.peripherals[idx].dev.read_gpio_pad(5),
        Some(false)
    );
    // GPIOTE Task mode owns COL4=P1.05, initially high, independent of GPIO OUT.
    machine
        .bus
        .write_u32(
            0x4000_6510,
            3 | (5 << 8) | (1 << 13) | (3 << 16) | (1 << 20),
        )
        .unwrap();
    for _ in 0..8 {
        machine.step().unwrap();
    }
    assert_eq!(
        machine.bus.peripherals[idx].dev.read_gpio_pad(5),
        Some(true)
    );
    machine.bus.write_u32(0x4000_6000, 1).unwrap();
    // Step the machine so the event scheduler drains GPIOTE's queued IN writes.
    for _ in 0..8 {
        machine.step().unwrap();
    }
    assert_eq!(
        machine.bus.peripherals[idx].dev.read_gpio_pad(5),
        Some(false)
    );
    machine.bus.write_u32(0x4000_6000, 1).unwrap();
    for _ in 0..8 {
        machine.step().unwrap();
    }
    machine.bus.write_u32(0x5000_0A14, 0).unwrap();
    machine.bus.write_u32(0x4000_6510, 0).unwrap();
    for _ in 0..8 {
        machine.step().unwrap();
    }
    assert_ne!(
        machine.bus.read_u32(0x5000_0810).unwrap() & (1 << 5),
        0,
        "the compact port also receives the GPIOTE per-pin IN latch"
    );
}

#[test]
fn silicon_p1_output_drives_the_fourth_matrix_column() {
    let mut bus = bus();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine: Machine<CortexM> = Machine::new(cpu, bus);
    let mut image = ProgramImage::new(0x101, Arch::Arm);
    let mut flash = vec![0u8; 0x104];
    flash[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
    flash[4..8].copy_from_slice(&0x101u32.to_le_bytes());
    flash[0x100..0x102].copy_from_slice(&0xE7FEu16.to_le_bytes());
    image.add_segment(0, flash);
    machine.load_firmware(&image).unwrap();
    let columns = (1 << 28) | (1 << 11) | (1 << 31) | (1 << 30);
    machine
        .bus
        .write_u32(0x5000_0518, columns | (1 << 21))
        .unwrap();
    machine
        .bus
        .write_u32(0x5000_0508, columns | (1 << 21))
        .unwrap();
    // Actual P1.05 PIN_CNF and OUT drive COL4 low; all other columns are high.
    machine.bus.write_u32(0x5000_0A14, 1).unwrap();
    machine.bus.write_u32(0x5000_080C, 1 << 5).unwrap();
    for _ in 0..2_000_000 {
        machine.step().unwrap();
    }
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    let frame = machine
        .bus
        .display_artifact("led_matrix", &opts)
        .unwrap()
        .bytes
        .unwrap();
    assert!(frame[3] > 0, "P1.05 sinks visible pixel (3,0)");
    assert!(frame.iter().enumerate().all(|(i, b)| i == 3 || *b == 0));
    machine.bus.write_u32(0x5000_0808, 1 << 5).unwrap();
    for _ in 0..4_000_000 {
        machine.step().unwrap();
    }
    let frame = machine
        .bus
        .display_artifact("led_matrix", &opts)
        .unwrap()
        .bytes
        .unwrap();
    assert!(
        frame.iter().all(|b| *b == 0),
        "raising P1.05 extinguishes COL4"
    );
}
