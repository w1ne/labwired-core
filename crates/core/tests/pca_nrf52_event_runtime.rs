use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, peripherals::components::servo::Servo, peripherals::stub::StubPeripheral, Bus,
    Peripheral,
};
use std::path::PathBuf;
fn make_bus() -> SystemBus {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/nrf52840.yaml"),
    )
    .unwrap();
    let manifest:SystemManifest=serde_yaml::from_str(r#"
name: nrf52-pca-servo
chip: nrf52840
external_devices:
 - {id: pca, type: pca9685, connection: i2c0, config: {i2c_address: 0x41}}
 - {id: servo0, type: servo, connection: i2c0, config: {model: sg90, source_address: 65, source_channel: 0}}
"#).unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}
fn transaction(dev: &mut dyn Peripheral, bus: &mut SystemBus, reg: u8, bytes: &[u8]) {
    const RAM: u64 = 0x2000_1000;
    bus.write_u8(RAM, reg).unwrap();
    for (i, &b) in bytes.iter().enumerate() {
        bus.write_u8(RAM + 1 + i as u64, b).unwrap();
    }
    dev.write_u32(0x104, 0).unwrap();
    dev.write_u32(0x544, RAM as u32).unwrap();
    dev.write_u32(0x548, (bytes.len() + 1) as u32).unwrap();
    dev.write_u32(0x008, 1).unwrap();
    for _ in 0..5_000_000 {
        if !dev.needs_bus_tick() {
            break;
        }
        dev.tick_with_bus(bus);
    }
    assert!(!dev.needs_bus_tick(), "TWIM EasyDMA must complete");
    assert_eq!(dev.read_u32(0x104).unwrap(), 1, "STOP must complete");
    assert_eq!(dev.read_u32(0x4c4).unwrap() & 2, 0, "PCA must ACK");
}
#[test]
fn nrf52_serial_instance_delivers_pca_command_without_inspection() {
    let mut bus = make_bus();
    let idx = bus
        .peripherals
        .iter()
        .position(|p| p.name == "i2c0")
        .unwrap();
    let mut dev = std::mem::replace(
        &mut bus.peripherals[idx].dev,
        Box::new(StubPeripheral::new(0)),
    );
    dev.write_u32(0x500, 6).unwrap();
    dev.write_u32(0x588, 0x41).unwrap();
    dev.write_u32(0x200, 1 << 9).unwrap();
    transaction(dev.as_mut(), &mut bus, 0, &[0x10]);
    transaction(dev.as_mut(), &mut bus, 0xfe, &[121]);
    transaction(dev.as_mut(), &mut bus, 0, &[0x20]);
    transaction(dev.as_mut(), &mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    transaction(dev.as_mut(), &mut bus, 0x06, &[0, 0, 0, 0x10]);
    bus.peripherals[idx].dev = dev;
    let servo = bus
        .observed_of::<Servo>()
        .find(|s| s.id() == "servo0")
        .unwrap();
    assert!(
        servo.is_commanded() && servo.angle_degrees() > 80.0,
        "TWIM command must survive full-off without inspection"
    );
}

#[test]
fn standalone_twim_delivers_pca_command_without_inspection() {
    use labwired_core::peripherals::{
        components::GenericI2cDevice,
        nrf52::{serial_instance::Nrf52SerialInstance, twim::Nrf52Twim},
    };
    let mut bus = make_bus();
    let idx = bus
        .peripherals
        .iter()
        .position(|p| p.name == "i2c0")
        .unwrap();
    let source = {
        let host = bus.peripherals[idx]
            .dev
            .as_any()
            .unwrap()
            .downcast_ref::<Nrf52SerialInstance>()
            .unwrap();
        let cells = host.attached_i2c_devices();
        let dummy = GenericI2cDevice::from_yaml(
            labwired_config::embedded_device_yaml("pca9685").unwrap(),
            0x41,
        )
        .unwrap();
        std::mem::replace(&mut *cells[0].borrow_mut(), Box::new(dummy))
    };
    bus.peripherals[idx].dev = Box::new(Nrf52Twim::new());
    bus.attach_i2c_slave("i2c0", source).unwrap();
    let mut dev = std::mem::replace(
        &mut bus.peripherals[idx].dev,
        Box::new(StubPeripheral::new(0)),
    );
    dev.write_u32(0x500, 6).unwrap();
    dev.write_u32(0x588, 0x41).unwrap();
    dev.write_u32(0x200, 1 << 9).unwrap();
    transaction(dev.as_mut(), &mut bus, 0, &[0x10]);
    transaction(dev.as_mut(), &mut bus, 0xfe, &[121]);
    transaction(dev.as_mut(), &mut bus, 0, &[0x20]);
    transaction(dev.as_mut(), &mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    transaction(dev.as_mut(), &mut bus, 0x06, &[0, 0, 0, 0x10]);
    bus.peripherals[idx].dev = dev;
    let servo = bus
        .observed_of::<Servo>()
        .find(|s| s.id() == "servo0")
        .unwrap();
    assert!(
        servo.is_commanded() && servo.angle_degrees() > 80.0,
        "standalone TWIM command must survive full-off without inspection"
    );
}
