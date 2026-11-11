// SPDX-License-Identifier: MIT
//! Real TWIM/EasyDMA drives declarative outputs; source-built firmware samples
//! the resolved P0.25 pad and falling-edge GPIOTE events. No host pad injection.
use labwired_config::ExternalDevice;
use labwired_core::{Bus, DebugControl};

#[path = "common/microbit_sensor_guest.rs"]
mod guest;
use guest::{observed, RAM};

fn machine(autonomous: bool, invert_a: bool, interval: u32) -> guest::Guest {
    guest::machine("shared-gpiote.S", interval, |manifest| {
        // This is a generic wiring fixture, independent of the onboard sensor
        // descriptors. Only the explicitly placed test participants drive P0.25.
        manifest.external_devices.clear();
        for (id, address) in [("probe-a", 0x20), ("probe-b", 0x21)] {
            manifest.external_devices.push(ExternalDevice {
                id: id.into(),
                r#type: "pcf8574".into(),
                connection: "i2c0".into(),
                channel: None,
                route: Default::default(),
                config: [
                    ("i2c_address".into(), serde_yaml::Value::from(address)),
                    ("p0_pin".into(), serde_yaml::Value::from("P0.25")),
                    (
                        "open_drain_pullups".into(),
                        serde_yaml::from_str("[P0]").unwrap(),
                    ),
                ]
                .into_iter()
                .collect(),
            });
            if invert_a && id == "probe-a" {
                manifest.external_devices.last_mut().unwrap().config.insert(
                    "inverting_collectors".into(),
                    serde_yaml::from_str("[P0]").unwrap(),
                );
            }
        }
        if autonomous {
            let desc = labwired_config::DeviceDescriptor::from_yaml(r#"
schema: labwired.part/v1
type: autonomous_ready_probe
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x22
    registers:
      - { name: START, addr: 0, width: 1, endian: be, access: rw, reset: 0 }
      - { name: STATUS, addr: 1, width: 1, endian: be, access: r, reset: 0 }
      - { name: RESULT, addr: 2, width: 1, endian: be, access: r, reset: 0 }
    data_ready:
      - { name: sample, start_register: START, start_mask: 1, ready_register: STATUS, ready_mask: 1, conversion_us: 1000, clear_on_read: [RESULT] }
  outputs: [INT]
  output_pins: { INT: int_pin }
  rules:
    - on: { data_ready: sample }
      do: [ { pin: INT, level: 0 } ]
    - on: { read: RESULT }
      do: [ { pin: INT, level: 1 } ]
"#).unwrap();
            manifest
                .parts
                .push(labwired_config::PartPack::Inline(Box::new(desc)));
            manifest.external_devices.push(ExternalDevice {
                id: "autonomous-probe".into(),
                r#type: "autonomous_ready_probe".into(),
                connection: "i2c0".into(),
                channel: None,
                route: Default::default(),
                config: [
                    ("int_pin".into(), serde_yaml::Value::from("P0.25")),
                    (
                        "open_drain_pullups".into(),
                        serde_yaml::from_str("[INT]").unwrap(),
                    ),
                ]
                .into_iter()
                .collect(),
            });
        }
    })
}

/// Host-configure the actual TWIM and EasyDMA while the guest samples GPIO.
/// A read uses the controller's repeated-START shortcut after the pointer.
fn probe_transfer(
    machine: &mut guest::Guest,
    address: u8,
    bytes: &[u8],
    read: bool,
) -> (u32, u32, u32, u32) {
    const TWIM: u64 = 0x4000_3000;
    const TX: u64 = 0x2000_0200;
    const RX: u64 = 0x2000_0210;
    for (index, byte) in bytes.iter().enumerate() {
        machine.bus.write_u8(TX + index as u64, *byte).unwrap();
    }
    machine.bus.write_u32(TWIM + 0x508, 8).unwrap();
    machine.bus.write_u32(TWIM + 0x50c, 16).unwrap();
    machine.bus.write_u32(TWIM + 0x524, 0x0640_0000).unwrap();
    machine.bus.write_u32(TWIM + 0x500, 6).unwrap();
    machine.bus.write_u32(TWIM + 0x104, 0).unwrap();
    machine.bus.write_u32(TWIM + 0x124, 0).unwrap();
    machine.bus.write_u32(TWIM + 0x4c4, 6).unwrap();
    machine
        .bus
        .write_u32(
            TWIM + 0x200,
            if read { (1 << 7) | (1 << 12) } else { 1 << 9 },
        )
        .unwrap();
    machine
        .bus
        .write_u32(TWIM + 0x588, u32::from(address))
        .unwrap();
    machine.bus.write_u32(TWIM + 0x544, TX as u32).unwrap();
    machine
        .bus
        .write_u32(TWIM + 0x548, bytes.len() as u32)
        .unwrap();
    if read {
        machine.bus.write_u32(TWIM + 0x534, RX as u32).unwrap();
        machine.bus.write_u32(TWIM + 0x538, 1).unwrap();
    }
    machine.bus.write_u32(TWIM + 0x008, 1).unwrap();
    // Poll promptly so this test can cancel a still-pending conversion;
    // an arbitrary long post-transfer run could pass its deadline first.
    for _ in 0..5_000 {
        if machine.bus.read_u32(TWIM + 0x104).unwrap() != 0 {
            return (
                machine.bus.read_u32(TWIM + 0x124).unwrap(),
                machine.bus.read_u32(TWIM + 0x4c4).unwrap(),
                machine.bus.read_u32(TWIM + 0x54c).unwrap(),
                if read {
                    machine.bus.read_u32(TWIM + 0x53c).unwrap()
                } else {
                    0
                },
            );
        }
        machine.run(Some(32)).unwrap();
    }
    panic!("TWIM transfer did not stop");
}

fn write_probe(machine: &mut guest::Guest, address: u8, value: u8) {
    assert_eq!(
        probe_transfer(machine, address, &[value], false),
        (0, 0, 1, 0)
    );
}

fn write_probe_register(machine: &mut guest::Guest, register: u8, value: u8) {
    assert_eq!(
        probe_transfer(machine, 0x22, &[register, value], false),
        (0, 0, 2, 0)
    );
}

fn read_probe_register(machine: &mut guest::Guest, register: u8) -> u8 {
    assert_eq!(
        probe_transfer(machine, 0x22, &[register], true),
        (0, 0, 1, 1)
    );
    machine.bus.read_u8(0x2000_0210).unwrap()
}

#[test]
fn twim_probe_rejects_an_absent_address() {
    for &interval in guest::profiles() {
        let mut m = machine(false, false, interval);
        let (error, source, tx, rx) = probe_transfer(&mut m, 0x26, &[0xff], false);
        assert_eq!((error, source & 2, tx, rx), (1, 2, 0, 0));
    }
}

#[test]
fn guest_observes_two_independent_retained_drivers() {
    for &interval in guest::profiles() {
        let mut m = machine(false, false, interval);
        m.run(Some(10_000)).unwrap();
        observed(&mut m, 1, 0);
        for (address, value, level, falls) in [
            (0x20, 0xff, 1, 0),
            (0x20, 0xfe, 0, 1),
            (0x21, 0xfe, 0, 1),
            (0x20, 0xff, 0, 1),
            (0x21, 0xff, 1, 1),
            (0x21, 0xfe, 0, 2),
            (0x20, 0xfe, 0, 2),
            (0x21, 0xff, 0, 2),
            (0x20, 0xff, 1, 2),
        ] {
            let samples = m.bus.read_u32(RAM + 4).unwrap();
            write_probe(&mut m, address, value);
            m.run(Some(10_000)).unwrap();
            assert!(
                m.bus.read_u32(RAM + 4).unwrap() > samples,
                "fresh guest sample"
            );
            observed(&mut m, level, falls);
        }
    }
}

/// The host starts a conversion over TWIM. Between START and the first falling
/// edge neither the host nor firmware reads a device register. Firmware reads
/// only GPIO/GPIOTE; host receipts come from TWIM registers and guest RAM.
#[test]
fn guest_sees_autonomous_data_ready_without_polling() {
    for &interval in guest::profiles() {
        let mut m = machine(true, false, interval);
        m.run(Some(1_000)).unwrap();
        observed(&mut m, 1, 0);
        write_probe_register(&mut m, 0, 1);
        let cancel_started = m.total_cycles;
        m.run(Some(100)).unwrap();
        observed(&mut m, 1, 0);
        write_probe_register(&mut m, 0, 0);
        let cancel_cycles = m.total_cycles - cancel_started;
        assert!(
            cancel_cycles < 32_000,
            "cancel before the 1,000-us deadline: {cancel_cycles} cycles"
        );
        println!("MICROBIT_READY_CANCEL_CYCLES={cancel_cycles};interval={interval}");
        m.run(Some(200_000)).unwrap();
        observed(&mut m, 1, 0);

        write_probe_register(&mut m, 0, 1);
        m.run(Some(100)).unwrap();
        observed(&mut m, 1, 0);
        m.run(Some(200_000)).unwrap();
        observed(&mut m, 0, 1);
        m.run(Some(100_000)).unwrap();
        observed(&mut m, 0, 1); // a ready conversion emits only one edge

        // RESULT acknowledges and starts the next conversion. Require a fresh
        // guest observation of release within 500 us, before it can assert again.
        let before_read = m.total_cycles;
        let before_sample = m.bus.read_u32(RAM + 4).unwrap();
        read_probe_register(&mut m, 2);
        let mut released = false;
        for _ in 0..1_000 {
            let elapsed = m.total_cycles - before_read;
            assert!(
                elapsed < 32_000,
                "release must precede the next conversion: {elapsed} cycles"
            );
            let samples = m.bus.read_u32(RAM + 4).unwrap();
            let level = m.bus.read_u32(RAM).unwrap();
            if samples > before_sample && level == 1 {
                println!("MICROBIT_READY_RELEASE_CYCLES={elapsed};interval={interval}");
                released = true;
                break;
            }
            m.run(Some(32)).unwrap();
        }
        assert!(released, "guest must observe the result-read release");
        observed(&mut m, 1, 1);
        m.run(Some(200_000)).unwrap();
        observed(&mut m, 0, 2);
    }
}

#[test]
fn guest_observes_inverting_collector_and_direct_shared_driver() {
    for &interval in guest::profiles() {
        let mut m = machine(false, true, interval);
        m.run(Some(10_000)).unwrap();
        observed(&mut m, 1, 0);
        for (address, value, level, falls) in [
            (0x20, 0xfe, 1, 0),
            (0x21, 0xff, 1, 0),
            (0x20, 0xff, 0, 1),
            (0x21, 0xfe, 0, 1),
            (0x20, 0xfe, 0, 1),
            (0x21, 0xff, 1, 1),
            (0x21, 0xfe, 0, 2),
            (0x20, 0xff, 0, 2),
            (0x21, 0xff, 0, 2),
            (0x20, 0xfe, 1, 2),
        ] {
            write_probe(&mut m, address, value);
            m.run(Some(10_000)).unwrap();
            observed(&mut m, level, falls);
        }
    }
}

/// Do not prime GPIOTE with a fake edge: its first real falling transition
/// after firmware enables Event mode must be observed from the pulled-up pad.
#[test]
fn guest_observes_first_gpiote_falling_edge_after_channel_enable() {
    for &interval in guest::profiles() {
        let mut m = machine(false, true, interval);
        m.run(Some(10_000)).unwrap();
        observed(&mut m, 1, 0);
        write_probe(&mut m, 0x20, 0xff);
        m.run(Some(10_000)).unwrap();
        observed(&mut m, 0, 1);
        write_probe(&mut m, 0x20, 0xfe);
        m.run(Some(10_000)).unwrap();
        observed(&mut m, 1, 1); // the channel masks the rising edge
        write_probe(&mut m, 0x21, 0xfe);
        m.run(Some(10_000)).unwrap();
        observed(&mut m, 0, 2);
    }
}

/// CPU reset does not power-cycle off-chip devices. A new SystemBus does:
/// every registered driver starts released, independent of the old machine.
#[test]
fn cpu_reset_retains_external_low_and_cold_construction_releases_it() {
    for &interval in guest::profiles() {
        let mut m = machine(false, false, interval);
        m.run(Some(10_000)).unwrap();
        write_probe(&mut m, 0x20, 0xfe);
        m.run(Some(10_000)).unwrap();
        observed(&mut m, 0, 1);
        m.reset().unwrap();
        m.run(Some(10_000)).unwrap();
        assert_eq!(
            m.bus.read_u32(RAM).unwrap(),
            0,
            "CPU reset retains the external sink"
        );
        assert!(m.bus.read_u32(RAM + 4).unwrap() > 0);

        let mut cold = machine(false, false, interval);
        cold.run(Some(10_000)).unwrap();
        observed(&mut cold, 1, 0);
    }
}
