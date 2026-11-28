// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF5340 GPIOTE task output updates the selected GPIO input latch.
//!
//! Source: Nordic `nrf5340.svd` (vendored at
//! `tests/fixtures/real_world/nrf5340.svd`), `GPIOTE0_S` at 0x5000D000.
//! The implementation reuses the nRF52 GPIOTE task/event register model and
//! resolves GPIO0/GPIO1 addresses from this chip descriptor.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

const GPIOTE_TASKS_SET0: u64 = 0x030;
const GPIOTE_CONFIG0: u64 = 0x510;
// P0_S starts at OUT (the model's family offset +0x500). The SVD's IN
// register is at P0_S +0x10, not +0x510.
const GPIO_IN: u64 = 0x010;
const PIN: u32 = 5;

fn root(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn nrf5340_bus_with_gpiote_type(gpiote_type: &str) -> (ChipDescriptor, SystemBus) {
    let path = root("configs/chips/nrf5340.yaml");
    let mut chip = ChipDescriptor::from_file(&path).expect("load nrf5340 chip");
    chip.peripherals
        .iter_mut()
        .find(|peripheral| peripheral.id == "gpiote0")
        .expect("nrf5340 GPIOTE0 descriptor")
        .r#type = gpiote_type.to_string();
    let manifest = SystemManifest {
        max_peripheral_tick_interval: None,
        parts: Vec::new(),
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "nrf5340-gpiote".to_string(),
        chip: path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let bus = SystemBus::from_config(&chip, &manifest).expect("assemble nrf5340 bus");
    (chip, bus)
}

fn nrf5340_bus() -> (ChipDescriptor, SystemBus) {
    nrf5340_bus_with_gpiote_type("nrf52_gpiote")
}

fn peripheral_base(chip: &ChipDescriptor, id: &str) -> u64 {
    chip.peripherals
        .iter()
        .find(|peripheral| peripheral.id == id)
        .unwrap_or_else(|| panic!("nrf5340 descriptor has no {id}"))
        .base_address
}

#[test]
fn gpiote_task_set_updates_selected_gpio0_input_latch() {
    let svd = std::fs::read_to_string(root("tests/fixtures/real_world/nrf5340.svd"))
        .expect("read Nordic nrf5340 SVD");
    assert!(svd.contains("<name>GPIOTE0_S</name>"));
    assert!(svd.contains("<baseAddress>0x5000D000</baseAddress>"));
    let interrupt_name = "<name>GPIOTE0</name>";
    let interrupt_at =
        svd.find(interrupt_name).expect("GPIOTE0 interrupt in SVD") + interrupt_name.len();
    let interrupt_value = &svd[interrupt_at..];
    let value_start = interrupt_value
        .find("<value>")
        .expect("GPIOTE0 SVD interrupt value")
        + "<value>".len();
    let value_end = interrupt_value[value_start..]
        .find("</value>")
        .expect("GPIOTE0 SVD interrupt value close")
        + value_start;
    assert_eq!(
        interrupt_value[value_start..value_end].trim(),
        "13",
        "GPIOTE0 interrupt number from Nordic SVD"
    );

    let (chip, mut bus) = nrf5340_bus();
    let gpiote = peripheral_base(&chip, "gpiote0");
    let gpio0 = peripheral_base(&chip, "gpio0");
    assert_eq!(
        chip.peripherals
            .iter()
            .find(|peripheral| peripheral.id == "gpiote0")
            .and_then(|peripheral| peripheral.irq),
        Some(13),
        "chip descriptor GPIOTE0 IRQ"
    );
    assert_eq!(gpiote, 0x5000_D000, "GPIOTE0 secure SVD base");
    assert_eq!(gpio0, 0x5084_2500, "GPIO0 secure SVD base");

    // MODE=Task, PSEL=PIN, PORT=0, POLARITY=None. TASKS_SET0 must set the
    // GPIO0 per-pin input latch high; it is visible at the SVD GPIO0.IN
    // address. This does not assert an external electrical pad route.
    bus.write_u32(gpiote + GPIOTE_CONFIG0, 3 | (PIN << 8))
        .expect("configure task-mode channel");
    bus.write_u32(gpiote + GPIOTE_TASKS_SET0, 1)
        .expect("trigger task-mode set");
    for _ in 0..4 {
        bus.tick_peripherals_fully_forced();
    }

    let input = bus.read_u32(gpio0 + GPIO_IN).expect("read GPIO0.IN");
    assert_eq!(
        input & (1 << PIN),
        1 << PIN,
        "GPIOTE0 TASKS_SET0 must latch P0.{PIN} high for GPIO0.IN"
    );
}

#[test]
fn legacy_fuzzy_gpio_type_does_not_latch_a_gpiote_task() {
    let (chip, mut bus) = nrf5340_bus_with_gpiote_type("nrf5340_gpiote_stub");
    let gpiote = peripheral_base(&chip, "gpiote0");
    let gpio0 = peripheral_base(&chip, "gpio0");

    bus.write_u32(gpiote + GPIOTE_CONFIG0, 3 | (PIN << 8))
        .expect("configure task-mode channel");
    bus.write_u32(gpiote + GPIOTE_TASKS_SET0, 1)
        .expect("trigger task-mode set");
    for _ in 0..4 {
        bus.tick_peripherals_fully_forced();
    }

    let input = bus.read_u32(gpio0 + GPIO_IN).expect("read GPIO0.IN");
    assert_eq!(
        input & (1 << PIN),
        0,
        "the legacy `_stub` name resolves as generic GPIO and cannot execute GPIOTE tasks"
    );
}
