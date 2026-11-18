use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, cpu::Avr, peripherals::components::servo::Servo, Cpu, Machine,
};
use std::path::PathBuf;

fn rig() -> Machine<Avr> {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/atmega328p.yaml"),
    )
    .unwrap();
    let manifest = SystemManifest::from_yaml(r#"
name: avr-pca-servo
chip: atmega328p
external_devices:
 - {id: pca, type: pca9685, connection: i2c, config: {i2c_address: 0x41}}
 - {id: servo0, type: servo, connection: i2c, config: {model: sg90, source_address: 65, source_channel: 0}}
"#).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let mut cpu = Avr::new();
    for dev in bus.take_i2c_slaves("i2c") {
        cpu.push_i2c_slave(dev);
    }
    Machine::new(cpu, bus)
}
fn emit_write(words: &mut Vec<u16>, addr: u16, value: u8) {
    words.push(0xe000 | ((u16::from(value) & 0xf0) << 4) | u16::from(value & 0xf));
    words.extend([0x9300, addr]); // STS data address, r16
}
fn transaction(words: &mut Vec<u16>, reg: u8, data: &[u8]) {
    emit_write(words, 0xbc, 0xa4); // START
    for byte in [0x82, reg].into_iter().chain(data.iter().copied()) {
        emit_write(words, 0xbb, byte);
        emit_write(words, 0xbc, 0x84); // address/data
    }
    emit_write(words, 0xbc, 0x94); // STOP
}
#[test]
fn avr_firmware_twi_command_survives_full_off_without_inspection() {
    let mut machine = rig();
    let mut words = Vec::new();
    transaction(&mut words, 0, &[0x10]);
    transaction(&mut words, 0xfe, &[121]);
    transaction(&mut words, 0, &[0x20]);
    transaction(&mut words, 0x06, &[0, 0, 0x33, 0x01]);
    transaction(&mut words, 0x06, &[0, 0, 0, 0x10]);
    machine.cpu.load_words(0, &words);
    machine.cpu.set_pc(0);
    for _ in 0..words.len() * 2 / 3 {
        machine.step().unwrap();
    }
    let servo = machine.bus.observed_of::<Servo>().next().unwrap();
    assert!(
        servo.is_commanded() && servo.angle_degrees() > 80.0,
        "AVR TWI firmware command must survive FULL_OFF without inspection"
    );
}

fn run_command(machine: &mut Machine<Avr>) {
    let mut words = Vec::new();
    transaction(&mut words, 0, &[0x10]);
    transaction(&mut words, 0xfe, &[121]);
    transaction(&mut words, 0, &[0x20]);
    transaction(&mut words, 0x06, &[0, 0, 0x33, 0x01]);
    machine.cpu.load_words(0, &words);
    machine.cpu.set_pc(0);
    for _ in 0..words.len() * 2 / 3 {
        machine.step().unwrap();
    }
}
#[test]
fn avr_replaced_unstamped_source_cannot_drive_servo() {
    let mut machine = rig();
    let replacement = labwired_core::peripherals::components::GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("pca9685").unwrap(),
        0x41,
    )
    .unwrap();
    machine.cpu.i2c_slaves[0] = Box::new(replacement);
    run_command(&mut machine);
    assert!(!machine
        .bus
        .observed_of::<Servo>()
        .next()
        .unwrap()
        .is_commanded());
}
#[test]
fn avr_detached_source_does_not_retain_servo_callbacks() {
    let mut machine = rig();
    run_command(&mut machine);
    let before = machine
        .bus
        .observed_of::<Servo>()
        .next()
        .unwrap()
        .angle_degrees();
    assert!(before > 80.0);
    let mut source = machine.cpu.i2c_slaves.remove(0);
    source.start();
    source.write(0x06);
    for byte in [0, 0, 0x90, 0x01] {
        source.write(byte);
    }
    source.stop();
    assert_eq!(
        machine
            .bus
            .observed_of::<Servo>()
            .next()
            .unwrap()
            .angle_degrees(),
        before
    );
}

#[test]
fn avr_power_cycle_does_not_replay_stale_pulse_into_new_servo() {
    use labwired_core::peripherals::components::{
        servo::ServoCal,
        supply_domain::{GatedI2cDevice, SupplyGate},
    };
    use std::sync::Arc;
    let mut machine = rig();
    let domain = machine
        .bus
        .register_supply_domain("avr-pca-vdd", 3.0)
        .unwrap();
    machine
        .bus
        .update_supply_domain("avr-pca-vdd", 3.3)
        .unwrap();
    let device = machine.cpu.i2c_slaves.remove(0);
    machine.cpu.i2c_slaves.push(Box::new(GatedI2cDevice::new(
        device,
        SupplyGate::routed(domain, true),
    )));
    run_command(&mut machine);
    assert!(machine
        .bus
        .observed_of::<Servo>()
        .next()
        .unwrap()
        .is_commanded());
    machine
        .bus
        .update_supply_domain("avr-pca-vdd", 0.0)
        .unwrap();
    machine
        .bus
        .update_supply_domain("avr-pca-vdd", 3.3)
        .unwrap();
    machine.bus.observed.clear();
    machine.bus.observe_device(Arc::new(Servo::with_source(
        "servo0",
        ServoCal::sg90(),
        0x41,
        0,
    )));
    let mut words = Vec::new();
    emit_write(&mut words, 0xbc, 0xa4);
    emit_write(&mut words, 0xbb, 0x82);
    emit_write(&mut words, 0xbc, 0x84);
    machine.cpu.load_words(0, &words);
    machine.cpu.set_pc(0);
    for _ in 0..words.len() * 2 / 3 {
        machine.step().unwrap();
    }
    assert!(
        !machine
            .bus
            .observed_of::<Servo>()
            .next()
            .unwrap()
            .is_commanded(),
        "restored power must not replay the old pulse"
    );
}

#[test]
fn avr_duplicate_source_fails_closed_and_recovers_after_removal() {
    let mut machine = rig();
    let extra = labwired_core::peripherals::components::GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("pca9685").unwrap(),
        0x41,
    )
    .unwrap();
    machine.cpu.i2c_slaves.push(Box::new(extra));
    run_command(&mut machine);
    assert!(
        !machine
            .bus
            .observed_of::<Servo>()
            .next()
            .unwrap()
            .is_commanded(),
        "duplicate source must be ambiguous"
    );
    machine.cpu.i2c_slaves.pop();
    run_command(&mut machine);
    assert!(
        machine
            .bus
            .observed_of::<Servo>()
            .next()
            .unwrap()
            .angle_degrees()
            > 80.0,
        "removing conflict must restore delivery"
    );
}

#[test]
fn avr_ack_latched_output_updates_before_stop() {
    let mut machine = rig();
    let mut words = Vec::new();
    transaction(&mut words, 0, &[0x10]);
    transaction(&mut words, 0xfe, &[121]);
    transaction(&mut words, 1, &[0x0c]); // OCH=1, totem-pole
    transaction(&mut words, 0, &[0x20]);
    emit_write(&mut words, 0xbc, 0xa4);
    for byte in [0x82, 0x06, 0, 0, 0x33, 0x01] {
        emit_write(&mut words, 0xbb, byte);
        emit_write(&mut words, 0xbc, 0x84);
    }
    machine.cpu.load_words(0, &words);
    machine.cpu.set_pc(0);
    for _ in 0..words.len() * 2 / 3 {
        machine.step().unwrap();
    }
    assert!(
        machine
            .bus
            .observed_of::<Servo>()
            .next()
            .unwrap()
            .angle_degrees()
            > 80.0,
        "OCH=1 must commit at the last data ACK without STOP"
    );
}

#[test]
fn avr_mux_topology_is_not_guessed() {
    let mut machine = rig();
    machine.cpu.i2c_slaves.push(Box::new(
        labwired_core::peripherals::components::tca9548a::Tca9548a::new(0x70),
    ));
    run_command(&mut machine);
    assert!(
        !machine
            .bus
            .observed_of::<Servo>()
            .next()
            .unwrap()
            .is_commanded(),
        "unqualified CPU mux topology must fail closed"
    );
}

#[test]
fn compiled_avr_firmware_delivers_servo_command_without_inspection() {
    let mut machine = rig();
    let elf = include_bytes!("../../../tests/fixtures/avr/pca_servo_twi.elf");
    let image = labwired_loader::load_elf_bytes(elf).unwrap();
    machine.cpu.load_program_image(&image);
    for _ in 0..2000 {
        machine.step().unwrap();
    }
    assert!(
        machine
            .bus
            .observed_of::<Servo>()
            .next()
            .unwrap()
            .angle_degrees()
            > 80.0,
        "compiled firmware command must survive FULL_OFF"
    );
}
