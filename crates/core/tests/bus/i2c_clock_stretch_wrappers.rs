// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Device decorators must preserve the timing seen by an I2C controller.
use labwired_core::{
    bus::{
        bus_trace::{new_log, wrap_i2c},
        SystemBus,
    },
    peripherals::{
        components::{
            supply_domain::{GatedI2cDevice, SupplyGate},
            Tca9548a,
        },
        device::{I2cDevice, I2cSlaveHost},
        sam::sercom_i2cm::{Samd51SercomI2cm, SERCOM_INSTANCES},
    },
    Bus, CycleClock, DmaRequest, Peripheral, SimResult, SimulationConfig,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

struct StretchingDevice {
    address: u8,
    hold_ns: Arc<AtomicU64>,
    writes: Arc<Mutex<Vec<u8>>>,
}
impl I2cDevice for StretchingDevice {
    fn address(&self) -> u8 {
        self.address
    }
    fn read(&mut self) -> u8 {
        0x5a
    }
    fn write(&mut self, byte: u8) {
        self.writes.lock().unwrap().push(byte);
    }
    fn clock_stretch_ns(&self) -> u64 {
        self.hold_ns.load(Ordering::Relaxed)
    }
}
fn device(address: u8, hold_ns: u64) -> StretchingDevice {
    StretchingDevice {
        address,
        hold_ns: Arc::new(AtomicU64::new(hold_ns)),
        writes: Arc::new(Mutex::new(Vec::new())),
    }
}
fn enable_channels(mux: &mut Tca9548a, channels: u8) {
    mux.select_address(mux.address());
    mux.write(channels);
}

// Only the controller's clock and pin registers are supplied here. The actual
// SERCOM executes its address/data handshake and computes the byte deadline.
struct TimingBus {
    memory: HashMap<u64, u8>,
    config: SimulationConfig,
}
impl Bus for TimingBus {
    fn read_u8(&self, address: u64) -> SimResult<u8> {
        Ok(*self.memory.get(&address).unwrap_or(&0))
    }
    fn write_u8(&mut self, address: u64, value: u8) -> SimResult<()> {
        self.memory.insert(address, value);
        Ok(())
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }
    fn execute_dma(&mut self, _: &[DmaRequest]) -> SimResult<()> {
        Ok(())
    }
    fn config(&self) -> &SimulationConfig {
        &self.config
    }
}
impl TimingBus {
    fn new() -> Self {
        let mut bus = Self {
            memory: HashMap::new(),
            config: SimulationConfig::default(),
        };
        bus.write_u32(0x40001c20, 6 | 1 << 8).unwrap(); // GCLK0: DFLL48M
        bus.write_u8(0x4000101c, 0x82).unwrap(); // DFLL enabled
        bus.write_u8(0x40000805, 1).unwrap(); // CPU /1
        bus.write_u32(0x40000818, 1 << 9).unwrap(); // APBB: SERCOM2
        bus.write_u32(0x40001c80 + 23 * 4, 1 << 6).unwrap();
        bus.write_u8(0x41008034, 0x33).unwrap(); // PA8/PA9 function D
        bus.write_u8(0x41008048, 1).unwrap();
        bus.write_u8(0x41008049, 1).unwrap();
        bus
    }
}

#[test]
fn payload_deadline_survives_trace_supply_and_mux_wrappers() {
    for case in ["direct", "trace", "gate", "mux", "combined"] {
        let target = device(0x40, 25_000);
        let writes = target.writes.clone();
        let mut wrapped: Box<dyn I2cDevice> = Box::new(target);
        if matches!(case, "mux" | "combined") {
            let mut mux = Tca9548a::new(0x70);
            // The first claimant is faster. The shared line stays low until
            // the slower claimant on the other enabled channel releases it.
            mux.attach(0, Box::new(device(0x40, 500))).unwrap();
            mux.attach(1, wrapped).unwrap();
            enable_channels(&mut mux, 0b11);
            wrapped = Box::new(mux);
        }
        if matches!(case, "gate" | "combined") {
            wrapped = Box::new(GatedI2cDevice::new(wrapped, SupplyGate::legacy(true)));
        }
        if matches!(case, "trace" | "combined") {
            wrapped = wrap_i2c("sercom2", &new_log(), wrapped);
        }
        let mut master = Samd51SercomI2cm::new(SERCOM_INSTANCES[2]);
        master.attach_slave(wrapped);
        let clock = CycleClock::default();
        master.attach_cycle_clock(clock.clone());
        let ctrla = (5 << 2) | (2 << 20);
        master.write_u32(0, ctrla).unwrap();
        master.write_u32(0, ctrla | 2).unwrap();
        master.write_u16(0x1a, 1 << 4).unwrap(); // BUSSTATE.IDLE
        master.write_u32(0x24, 0x40 << 1).unwrap();
        let mut bus = TimingBus::new();
        master.tick_with_bus(&mut bus);
        clock.publish(10_000);
        master.tick_with_bus(&mut bus);
        assert!(!master.busy(), "{case}: address must select the device");

        master.write(0x28, 0xa5).unwrap();
        master.tick_with_bus(&mut bus);
        // BAUD=0, CPU=GCLK=48 MHz: nine periods * ten cycles plus a
        // 25 us hold * 48 cycles/us = 1290 cycles for this payload byte.
        clock.publish(10_000 + 1289);
        master.tick_with_bus(&mut bus);
        assert!(master.busy(), "{case}: byte completed before SCL release");
        assert!(writes.lock().unwrap().is_empty(), "{case}: early write");
        clock.publish(10_000 + 1290);
        master.tick_with_bus(&mut bus);
        assert!(
            !master.busy(),
            "{case}: byte did not finish at its deadline"
        );
        assert_eq!(*writes.lock().unwrap(), vec![0xa5], "{case}");
    }
}

#[test]
fn routed_supply_cutoff_releases_stretch_through_nested_tracing() {
    let mut bus = SystemBus::empty();
    let domain = bus.register_supply_domain("sensor", 2.7).unwrap();
    let target = device(0x40, 25_000);
    let hold = target.hold_ns.clone();
    let wrapped = wrap_i2c("inner", &new_log(), Box::new(target));
    let gated = GatedI2cDevice::new(wrapped, SupplyGate::routed(domain, true));
    let wrapped = wrap_i2c("outer", &new_log(), Box::new(gated));
    assert!(!wrapped.claims_address(0x40));
    assert_eq!(wrapped.clock_stretch_ns(), 0);
    bus.update_supply_domain("sensor", 3.3).unwrap();
    assert!(wrapped.claims_address(0x40));
    assert_eq!(wrapped.clock_stretch_ns(), 25_000);
    hold.store(75_000, Ordering::Relaxed);
    assert_eq!(wrapped.clock_stretch_ns(), 75_000, "hold stays live");
    bus.update_supply_domain("sensor", 0.0).unwrap();
    assert!(!wrapped.claims_address(0x40));
    assert_eq!(
        wrapped.clock_stretch_ns(),
        0,
        "unpowered device releases SCL"
    );
    bus.update_supply_domain("sensor", 3.3).unwrap();
    assert_eq!(wrapped.clock_stretch_ns(), 75_000);
}

#[test]
fn mux_stretch_uses_only_selected_enabled_claimants() {
    let mut mux = Tca9548a::new(0x70);
    mux.attach(0, Box::new(device(0x40, 25_000))).unwrap();
    mux.attach(1, Box::new(device(0x40, 75_000))).unwrap();
    mux.attach(2, Box::new(device(0x41, 100_000))).unwrap();
    mux.select_address(0x40);
    assert_eq!(mux.clock_stretch_ns(), 0, "all channels start isolated");
    enable_channels(&mut mux, 0b111);
    mux.select_address(0x40);
    assert_eq!(mux.clock_stretch_ns(), 75_000, "maximum, not first or sum");
    mux.select_address(0x41);
    assert_eq!(mux.clock_stretch_ns(), 100_000);
    enable_channels(&mut mux, 0b001);
    mux.select_address(0x40);
    assert_eq!(
        mux.clock_stretch_ns(),
        25_000,
        "disabled channel cannot hold SCL"
    );
    mux.select_address(0x41);
    assert_eq!(
        mux.clock_stretch_ns(),
        0,
        "disabled address cannot hold SCL"
    );
    mux.select_address(0x70);
    assert_eq!(
        mux.clock_stretch_ns(),
        0,
        "switch control has no downstream hold"
    );
    mux.select_address(0x42);
    assert_eq!(mux.clock_stretch_ns(), 0, "unclaimed address has no hold");
}
