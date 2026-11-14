//! Actual busy-poll firmware must observe scheduled GPIO edges with either
//! scheduler feature configuration, including peripheral ticks wider than bits.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Bus, Machine};
use std::path::PathBuf;

fn machine(kind: &str, config: &str, firmware: &str, interval: u32) -> Machine<CortexM> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/stm32l476.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: schedule-firmware\nchip: unused\nexternal_devices:\n  - id: sensor\n    type: {kind}\n    connection: gpio\n    config:\n      cpu_hz: 80000000\n{config}\n"
    )).unwrap();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).unwrap();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = interval;
    machine.bus.config.peripheral_tick_interval = interval;
    let image = labwired_loader::load_elf(
        &root
            .join("crates/core/tests/fixtures/gpio-schedules")
            .join(firmware),
    )
    .unwrap();
    machine.load_firmware(&image).unwrap();
    machine
}

fn run(machine: &mut Machine<CortexM>, batched: bool) {
    if batched {
        let report = machine
            .advance(AdvanceRequest::run(Some(1_000_000)))
            .unwrap();
        assert_eq!(report.fuel_consumed, 1_000_000);
    } else {
        for _ in 0..1_000_000 {
            machine.step().unwrap();
        }
    }
}

#[test]
fn busy_polled_dht_frames_preserve_every_interval_with_coarse_ticks() {
    for (kind, temperature, humidity, expected, batched) in [
        ("dht22", 22.0, 50.0, [1, 244, 0, 220, 209], true),
        ("dht22", -0.01, 0.0, [0, 0, 128, 0, 128], false),
        ("dht22", -1e-50, 0.0, [0, 0, 0, 0, 0], true),
        ("dht11", 22.7, 60.5, [61, 0, 23, 0, 84], true),
    ] {
        let mut m = machine(kind, "      data_pin: PC2", "dht-frame.elf", 5000);
        m.bus
            .set_input(Some("sensor"), "temperature", temperature)
            .unwrap();
        m.bus
            .set_input(Some("sensor"), "humidity", humidity)
            .unwrap();
        run(&mut m, batched);
        assert_eq!(m.config.peripheral_tick_interval, 5000);
        assert_eq!(
            m.bus.read_u32(0x20000100).unwrap(),
            83,
            "{kind}: complete frame"
        );
        let widths: Vec<_> = (0..83)
            .map(|i| m.bus.read_u32(0x20000104 + i * 4).unwrap())
            .collect();
        let mut bytes = [0u8; 5];
        for bit in 0..40 {
            bytes[bit / 8] |= u8::from(widths[3 + bit * 2] > widths[2 + bit * 2]) << (7 - bit % 8);
        }
        assert_eq!(bytes, expected, "{kind}: firmware-decoded bytes");
        assert!(
            widths[82].abs_diff(widths[2]) < widths[2] / 10,
            "trailing LOW"
        );
    }
}

#[test]
fn busy_polled_ultrasonic_echo_preserves_fractional_distance() {
    let mut widths = Vec::new();
    for distance in [24.25, 97.0] {
        let mut m = machine(
            "hc-sr04",
            "      trig_pin: PA0\n      echo_pin: PB0",
            "echo-pulse.elf",
            1,
        );
        m.bus
            .set_input(Some("sensor"), "distance", distance)
            .unwrap();
        run(&mut m, true);
        assert_eq!(m.bus.read_u32(0x20000100).unwrap(), 2);
        widths.push(m.bus.read_u32(0x20000104).unwrap());
    }
    assert!(widths[0] > 0);
    assert!((f64::from(widths[1]) / f64::from(widths[0]) - 4.0).abs() < 0.005);
}
