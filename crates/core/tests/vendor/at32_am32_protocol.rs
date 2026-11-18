// SPDX-License-Identifier: MIT
//! Exact native flash image: boot, DMA capture and DShot checksum acceptance.
//! No RAM injection, forced arming, motor plant or flight qualification.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, cpu::CortexM, system::cortex_m::configure_cortex_m, Bus, Cpu, Machine,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const APP_SHA: &str = "2dc65fec5924f31548cc3041690b9b5a1645466832a09e46d012c33b14f8452d";
const COMPLETE_SHA: &str = "c6ae7b29e1fe35dd56070435aa271b18ec9cba77ec5329f092b3296776664e2a";
fn scalar(machine: &Machine<CortexM>, elf: &[u8], name: &str) -> u32 {
    let address = u64::from(labwired_loader::resolve_symbol_in_elf(elf, name).expect(name));
    match name {
        "armed"
        | "sensor_configuration_error"
        | "inputSet"
        | "dshot"
        | "servoPwm"
        | "dshot_telemetry"
        | "high_pin_count"
        | "ic_timer_prescaler"
        | "output_timer_prescaler"
        | "out_put"
        | "EDT_ARMED" => u32::from(machine.bus.read_u8(address).unwrap()),
        "dshot_goodcounts"
        | "dshot_badcounts"
        | "dshot_frametime"
        | "newinput"
        | "adjusted_input"
        | "smallestnumber"
        | "zero_input_count"
        | "armed_timeout_count"
        | "dshot_frametime_high"
        | "dshot_frametime_low" => u32::from(machine.bus.read_u16(address).unwrap()),
        _ => machine.bus.read_u32(address).unwrap(),
    }
}
fn state(machine: &Machine<CortexM>, elf: &[u8]) -> Value {
    let mut values = serde_json::Map::new();
    for name in [
        "system_core_clock",
        "armed",
        "sensor_configuration_error",
        "inputSet",
        "dshot",
        "servoPwm",
        "dshot_telemetry",
        "high_pin_count",
        "ic_timer_prescaler",
        "output_timer_prescaler",
        "out_put",
        "EDT_ARMED",
        "dshot_goodcounts",
        "dshot_badcounts",
        "dshot_frametime",
        "newinput",
        "adjusted_input",
        "smallestnumber",
        "average_signal_pulse",
        "zero_input_count",
        "armed_timeout_count",
        "dshot_frametime_high",
        "dshot_frametime_low",
    ] {
        values.insert(name.into(), json!(scalar(machine, elf, name)));
    }
    values.insert("cycles".into(), json!(machine.total_cycles));
    values.insert("pc".into(), json!(machine.cpu.get_pc()));
    Value::Object(values)
}
fn advance(machine: &mut Machine<CortexM>, cycles: u64) {
    let target = machine.total_cycles + cycles;
    while machine.total_cycles < target {
        machine.step().unwrap();
    }
}
fn frame(machine: &mut Machine<CortexM>, gpio: usize, value: u16, corrupt_crc: bool) {
    assert!(value < 2048);
    let payload = value << 1;
    let checksum = (payload ^ (payload >> 4) ^ (payload >> 8)) & 15;
    let bits = (payload << 4) | (checksum ^ u16::from(corrupt_crc));
    for bit in (0..16).rev() {
        let high_cycles = if bits & (1 << bit) == 0 { 75 } else { 150 };
        assert!(machine.bus.set_peripheral_gpio_input(gpio, 4, true));
        advance(machine, high_cycles);
        assert!(machine.bus.set_peripheral_gpio_input(gpio, 4, false));
        advance(machine, 200 - high_cycles);
    }
    // Complete 1 kHz frames, including the exact 26 2/3 us DShot600 packet.
    advance(machine, 120_000 - 16 * 200);
}

#[test]
#[ignore = "requires immutable native flash package and explicit evidence output"]
fn native_complete_image_decodes_dshot600_and_rejects_bad_checksum() {
    let artifacts = PathBuf::from(std::env::var("AT32_NATIVE_ARTIFACTS").unwrap());
    let report = PathBuf::from(std::env::var("AT32_NATIVE_PROTOCOL_REPORT").unwrap());
    let app = std::fs::read(artifacts.join("AM32_OPENESC_30_F421_2.21.elf")).unwrap();
    let complete = artifacts.join("OPENESC_30_F421_complete.elf");
    let bytes = std::fs::read(&complete).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&app)), APP_SHA);
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), COMPLETE_SHA);
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/at32f421g8u7.yaml"
    ))
    .unwrap();
    let system: SystemManifest = serde_yaml::from_str("name: openesc30-native\nchip: at32f421g8u7\ncpu_hz: 120000000\nmax_peripheral_tick_interval: 1\nexternal_devices: []\n").unwrap();
    let mut bus = SystemBus::from_config(&chip, &system).unwrap();
    bus.config.peripheral_tick_interval = 1;
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&labwired_loader::load_elf(&complete).unwrap())
        .unwrap();
    let gpio = machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
    assert!(machine.bus.set_peripheral_gpio_input(gpio, 4, false));
    advance(&mut machine, 90_000_000);
    let mut stages = vec![json!({"stage":"native-boot","state":state(&machine,&app)})];
    for index in 0..40 {
        frame(&mut machine, gpio, 0, false);
        if [0, 1, 7, 15, 31, 39].contains(&index) {
            stages.push(json!({"stage":"zero-frame","index":index,"state":state(&machine,&app)}));
        }
    }
    let before_bad = state(&machine, &app);
    frame(&mut machine, gpio, 100, true);
    let after_bad = state(&machine, &app);
    frame(&mut machine, gpio, 100, false);
    let after_valid = state(&machine, &app);
    let receipt = json!({"scope":"Exact complete native flash image; conventional non-inverted DShot600 protocol acquisition and checksum behavior before arming. No bidirectional telemetry, motor commutation or five-node proof.","applicationElfSha256":APP_SHA,"completeElfSha256":COMPLETE_SHA,"stages":stages,"beforeCorruptFrame":before_bad,"afterCorruptFrame":after_bad,"afterValidFrame":after_valid});
    if let Some(parent) = report.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&report, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    eprintln!("{}", receipt);
    assert_eq!(before_bad["system_core_clock"], 120_000_000);
    assert_eq!(before_bad["inputSet"], 1);
    assert_eq!(before_bad["dshot"], 1);
    assert!(before_bad["dshot_goodcounts"].as_u64().unwrap() >= 30);
    assert_eq!(
        after_bad["dshot_badcounts"].as_u64().unwrap(),
        before_bad["dshot_badcounts"].as_u64().unwrap() + 1
    );
    assert_eq!(after_bad["newinput"], 0);
    assert_eq!(
        after_valid["dshot_goodcounts"].as_u64().unwrap(),
        after_bad["dshot_goodcounts"].as_u64().unwrap() + 1
    );
    assert_eq!(after_valid["newinput"], 100);
    assert_eq!(after_valid["armed"], 0);
    assert_eq!(after_valid["sensor_configuration_error"], 0);
}
