use labwired_core::{
    bus::SystemBus, peripherals::i2c::I2cDevice, system::xtensa::configure_xtensa_esp32, Bus,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
struct Count(Arc<AtomicUsize>);
impl I2cDevice for Count {
    fn address(&self) -> u8 {
        0x41
    }
    fn read(&mut self) -> u8 {
        0
    }
    fn write(&mut self, _: u8) {}
    fn start(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn command(bus: &mut SystemBus, end: u32) {
    let base = 0x3ff5_3000u64;
    bus.write_u32(base + 0x58, 0).unwrap();
    bus.write_u32(base + 0x5c, (1 << 11) | 2).unwrap();
    bus.write_u32(base + 0x60, end << 11).unwrap();
    bus.write_u32(base + 0x1c, 0x82).unwrap();
    bus.write_u32(base + 0x1c, 0x55).unwrap();
    bus.write_u32(base + 0x24, u32::MAX).unwrap();
    bus.write_u32(base + 4, 1 << 5).unwrap();
}
#[test]
fn repeated_start_delivers_exactly_one_start_per_wire_event() {
    let mut bus = SystemBus::new();
    configure_xtensa_esp32(&mut bus);
    bus.refresh_peripheral_index();
    let count = Arc::new(AtomicUsize::new(0));
    bus.attach_i2c_slave("i2c0", Box::new(Count(count.clone())))
        .unwrap();
    command(&mut bus, 4);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    command(&mut bus, 3);
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "repeated START must not notify the same slave twice"
    );
}

#[test]
fn repeated_start_split_across_command_lists_is_not_double_delivered() {
    let mut bus = SystemBus::new();
    configure_xtensa_esp32(&mut bus);
    bus.refresh_peripheral_index();
    let count = Arc::new(AtomicUsize::new(0));
    bus.attach_i2c_slave("i2c0", Box::new(Count(count.clone())))
        .unwrap();
    command(&mut bus, 4);
    let base = 0x3ff5_3000u64;
    bus.write_u32(base + 0x58, 0).unwrap();
    bus.write_u32(base + 0x5c, 4 << 11).unwrap();
    bus.write_u32(base + 0x24, u32::MAX).unwrap();
    bus.write_u32(base + 4, 1 << 5).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    bus.write_u32(base + 0x58, (1 << 11) | 2).unwrap();
    bus.write_u32(base + 0x5c, 3 << 11).unwrap();
    bus.write_u32(base + 0x1c, 0x82).unwrap();
    bus.write_u32(base + 0x1c, 0x55).unwrap();
    bus.write_u32(base + 0x24, u32::MAX).unwrap();
    bus.write_u32(base + 4, 1 << 5).unwrap();
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "continuation after END must not issue a second START"
    );
}
