use labwired_core::{
    bus::SystemBus,
    peripherals::{
        components::supply_domain::{GatedI2cDevice, SupplyGate},
        device::I2cDevice,
    },
};
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct Counts {
    bus: usize,
    time: usize,
    pins: usize,
}
struct Probe(Arc<Mutex<Counts>>);
impl I2cDevice for Probe {
    fn address(&self) -> u8 {
        0x53
    }
    fn select_address(&mut self, _: u8) {
        self.0.lock().unwrap().bus += 1;
    }
    fn start(&mut self) {
        self.0.lock().unwrap().bus += 1;
    }
    fn stop(&mut self) {
        self.0.lock().unwrap().bus += 1;
    }
    fn read(&mut self) -> u8 {
        self.0.lock().unwrap().bus += 1;
        0xe5
    }
    fn write(&mut self, _: u8) {
        self.0.lock().unwrap().bus += 1;
    }
    fn advance_time_us(&mut self, _: u64) {
        self.0.lock().unwrap().time += 1;
    }
    fn take_pin_drives(&mut self) -> Vec<(String, bool)> {
        self.0.lock().unwrap().pins += 1;
        vec![]
    }
}
#[test]
fn shared_gate_suppresses_all_inner_operations_and_advances_only_on_transitions() {
    let mut b = SystemBus::new();
    let g = b.register_supply_domain("external3v3", 2.7).unwrap();
    let p = Arc::new(Mutex::new(Counts::default()));
    let mut d = GatedI2cDevice::new(
        Box::new(Probe(p.clone())),
        SupplyGate::routed(g.clone(), true),
    );
    assert!(!d.claims_address(0x53));
    assert_eq!(d.read(), 0xff);
    d.select_address(0x53);
    d.start();
    d.write(1);
    d.stop();
    d.advance_time_us(99);
    d.take_pin_drives();
    assert_eq!(p.lock().unwrap().bus, 0);
    assert_eq!(p.lock().unwrap().time, 0);
    assert_eq!(p.lock().unwrap().pins, 0);
    b.update_supply_domain("external3v3", 3.3).unwrap();
    let epoch = d.transaction_generation();
    assert!(d.claims_address(0x53));
    assert_eq!(d.read(), 0xe5);
    b.update_supply_domain("external3v3", 3.2).unwrap();
    assert_eq!(d.transaction_generation(), epoch);
    b.update_supply_domain("external3v3", 0.0).unwrap();
    b.update_supply_domain("external3v3", 3.3).unwrap();
    assert_ne!(d.transaction_generation(), epoch);
    let off = GatedI2cDevice::new(Box::new(Probe(p)), SupplyGate::routed(g, false));
    assert!(!off.claims_address(0x53));
    assert!(b.update_supply_domain("missing", 3.3).is_err());
    for v in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(b.update_supply_domain("external3v3", v).is_err());
    }
    assert!(b.register_supply_domain("external3v3", 2.8).is_err());
    assert!(b.register_supply_domain("bad", f64::NAN).is_err());
}
use labwired_core::peripherals::armv7::{gic::Gic, rk3x_i2c::Rk3xI2c};
use labwired_core::Peripheral;
#[test]
fn controller_rejects_selection_after_cut_restore_without_off_poll() {
    let mut b = SystemBus::new();
    let g = b.register_supply_domain("rail", 2.7).unwrap();
    b.update_supply_domain("rail", 3.3).unwrap();
    let p = Arc::new(Mutex::new(Counts::default()));
    let mut c = Rk3xI2c::new(Arc::new(Mutex::new(Gic::default())), 53);
    c.push_slave(Box::new(GatedI2cDevice::new(
        Box::new(Probe(p.clone())),
        SupplyGate::routed(g, true),
    )));
    c.write_u32(0, 1).unwrap();
    c.write_u32(0x100, 0x0153_u32 << 1).unwrap();
    c.write_u32(16, 2).unwrap();
    let count = p.lock().unwrap().bus;
    b.update_supply_domain("rail", 0.0).unwrap();
    b.update_supply_domain("rail", 3.3).unwrap();
    c.write_u32(28, 0x7f).unwrap();
    c.write_u32(16, 1).unwrap();
    assert_eq!(c.read_u32(28).unwrap(), 0x40);
    assert_eq!(p.lock().unwrap().bus, count);
}
#[test]
fn threshold_boundary_and_rejected_updates_preserve_state_and_canonical_handle() {
    let mut b = SystemBus::empty();
    let a = b.register_supply_domain("rail", 2.7).unwrap();
    let second = b.register_supply_domain("rail", 2.7).unwrap();
    assert!(Arc::ptr_eq(&a, &second));
    assert!(!a.powered());
    b.update_supply_domain("rail", 2.699).unwrap();
    assert!(!a.powered());
    b.update_supply_domain("rail", 2.7).unwrap();
    assert!(a.powered());
    let epoch = a.generation();
    assert!(b.update_supply_domain("rail", f64::NAN).is_err());
    assert!(a.powered());
    assert_eq!(a.generation(), epoch);
}
#[test]
fn cut_between_chunks_nacks_without_inner_stop_or_clock() {
    let mut b = SystemBus::empty();
    let g = b.register_supply_domain("rail", 2.7).unwrap();
    b.update_supply_domain("rail", 3.3).unwrap();
    let p = Arc::new(Mutex::new(Counts::default()));
    let mut c = Rk3xI2c::new(Arc::new(Mutex::new(Gic::default())), 53);
    c.push_slave(Box::new(GatedI2cDevice::new(
        Box::new(Probe(p.clone())),
        SupplyGate::routed(g, true),
    )));
    c.write_u32(0, 1).unwrap();
    c.write_u32(0x100, 0xa6).unwrap();
    c.write_u32(16, 1).unwrap();
    let count = p.lock().unwrap().bus;
    b.update_supply_domain("rail", 0.0).unwrap();
    c.write_u32(28, 0x7f).unwrap();
    c.write_u32(16, 1).unwrap();
    assert_eq!(c.read_u32(28).unwrap(), 0x40);
    c.write_u32(0, 17).unwrap();
    c.advance_attached_device_time_us(1000);
    c.drain_attached_pin_drives(&mut vec![]);
    assert_eq!(p.lock().unwrap().bus, count);
    assert_eq!(p.lock().unwrap().time, 0);
    assert_eq!(p.lock().unwrap().pins, 0);
}
#[test]
fn zero_count_chunk_cannot_complete_a_stale_selected_transaction() {
    let mut b = SystemBus::empty();
    let g = b.register_supply_domain("rail", 2.7).unwrap();
    b.update_supply_domain("rail", 3.3).unwrap();
    let p = Arc::new(Mutex::new(Counts::default()));
    let mut c = Rk3xI2c::new(Arc::new(Mutex::new(Gic::default())), 53);
    c.push_slave(Box::new(GatedI2cDevice::new(
        Box::new(Probe(p)),
        SupplyGate::routed(g, true),
    )));
    c.write_u32(0, 1).unwrap();
    c.write_u32(0x100, 0xa6).unwrap();
    c.write_u32(16, 1).unwrap();
    b.update_supply_domain("rail", 0.0).unwrap();
    b.update_supply_domain("rail", 3.3).unwrap();
    c.write_u32(28, 0x7f).unwrap();
    c.write_u32(16, 0).unwrap();
    assert_eq!(c.read_u32(28).unwrap(), 0x40);
}
#[test]
fn zero_count_rx_cannot_complete_a_stale_selected_transaction() {
    let mut b = SystemBus::empty();
    let g = b.register_supply_domain("rail", 2.7).unwrap();
    b.update_supply_domain("rail", 3.3).unwrap();
    let mut c = Rk3xI2c::new(Arc::new(Mutex::new(Gic::default())), 53);
    c.push_slave(Box::new(GatedI2cDevice::new(
        Box::new(Probe(Arc::new(Mutex::new(Counts::default())))),
        SupplyGate::routed(g, true),
    )));
    c.write_u32(0, 5).unwrap();
    c.write_u32(8, 0x010000a7).unwrap();
    c.write_u32(20, 1).unwrap();
    b.update_supply_domain("rail", 0.0).unwrap();
    b.update_supply_domain("rail", 3.3).unwrap();
    c.write_u32(28, 0x7f).unwrap();
    c.write_u32(20, 0).unwrap();
    assert_eq!(c.read_u32(28).unwrap(), 0x40);
}
