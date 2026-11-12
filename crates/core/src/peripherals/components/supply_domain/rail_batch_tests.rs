//! Receiver publication must precede both notification families.
use super::*;
use crate::{
    bus::SystemBus,
    peripheral::Peripheral,
    peripherals::device::SpiDevice,
    power_domains::{McuPowerDomains, PowerDomainHandle, PowerSupervisorControl},
    SimResult,
};

#[path = "../../../../tests/mcu_power_support/mod.rs"]
mod mcu_power_support;

type Trace = Arc<Mutex<Vec<(&'static str, Option<f64>, bool)>>>;
struct DeviceObserver {
    core: PowerDomainHandle,
    domain: Arc<SupplyDomain>,
    control: PowerSupervisorControl,
    trace: Trace,
}
impl SpiDevice for DeviceObserver {
    fn transfer(&mut self, _: u8) -> u8 {
        0xff
    }
    fn cs_pin(&self) -> &str {
        "PA4"
    }
    fn sync_supply_state(&mut self) {
        self.trace.lock().unwrap().push((
            "device",
            self.core.observation().voltage_v,
            self.domain.powered(),
        ));
        // A notification can enqueue the next sampled control. Holding the
        // live control lock or publishing its old snapshot later would either
        // deadlock or lose this command.
        self.control.configure_brown_out(false, 0.93, 0.96).unwrap();
    }
}
struct BoundaryObserver {
    core: PowerDomainHandle,
    domain: Arc<SupplyDomain>,
    devices: Vec<Box<dyn SpiDevice>>,
    trace: Trace,
}
impl std::fmt::Debug for BoundaryObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BoundaryObserver")
    }
}
impl Peripheral for BoundaryObserver {
    fn read(&self, _: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _: u64, _: u8) -> SimResult<()> {
        Ok(())
    }
    fn spi_attached_devices_mut(
        &mut self,
    ) -> Option<&mut Vec<Box<dyn crate::peripherals::device::SpiDevice>>> {
        Some(&mut self.devices)
    }
    fn sync_power_domains(&mut self) {
        self.trace.lock().unwrap().push((
            "mcu",
            self.core.observation().voltage_v,
            self.domain.powered(),
        ));
    }
}

fn fixture() -> (SystemBus, Arc<SupplyDomain>, Trace) {
    let mut bus = SystemBus::new();
    bus.mcu_power = McuPowerDomains::new(Some(mcu_power_support::config())).unwrap();
    bus.mcu_power
        .attach_sampled_routes(
            &[
                "core".into(),
                "io".into(),
                "detector".into(),
                "flash".into(),
            ],
            1000,
        )
        .unwrap();
    let domain = Arc::new(SupplyDomain::new(BTreeMap::from([
        (
            "vdd".into(),
            RailBounds {
                min_v: 1.71,
                max_v: Some(3.6),
            },
        ),
        (
            "vddio".into(),
            RailBounds {
                min_v: 1.2,
                max_v: Some(3.6),
            },
        ),
    ])));
    bus.supply_domains
        .devices
        .insert("observer".into(), domain.clone());
    let trace = Arc::new(Mutex::new(Vec::new()));
    let core = bus.mcu_power.handle("core").unwrap();
    let device = DeviceObserver {
        core: core.clone(),
        domain: domain.clone(),
        control: bus.mcu_power.supervisor_control().unwrap(),
        trace: trace.clone(),
    };
    bus.replace_or_add_peripheral(
        "rail_observer",
        0x7000_0000,
        0x100,
        None,
        Box::new(BoundaryObserver {
            core,
            domain: domain.clone(),
            devices: vec![Box::new(device)],
            trace: trace.clone(),
        }),
    );
    (bus, domain, trace)
}
fn mcu_values(core: f64) -> Vec<(String, f64)> {
    vec![
        ("core".into(), core),
        ("io".into(), 3.3),
        ("detector".into(), 3.3),
        ("flash".into(), 3.3),
    ]
}
fn device_values(vdd: f64) -> Vec<(String, String, f64)> {
    vec![
        ("observer".into(), "vdd".into(), vdd),
        ("observer".into(), "vddio".into(), 3.3),
    ]
}
fn publish(bus: &mut SystemBus, core: f64, vdd: f64, time_ns: u64) {
    bus.apply_power_rail_observations(&device_values(vdd), Some((&mcu_values(core), time_ns)))
        .unwrap();
}

#[test]
fn both_callback_families_observe_the_complete_boundary_without_held_locks() {
    let (mut bus, domain, trace) = fixture();
    let old_handle = bus.mcu_power.handle("core").unwrap();
    publish(&mut bus, 1.1, 1.8, 0);
    assert_eq!(
        *trace.lock().unwrap(),
        [("device", Some(1.1), true), ("mcu", Some(1.1), true)]
    );
    let status = bus.mcu_power.status().unwrap();
    let brown_out = status.brown_out.unwrap();
    assert_eq!(brown_out.pending_changes, 1);
    assert!(!brown_out.accepted.enabled);
    assert!(brown_out.applied.enabled);

    trace.lock().unwrap().clear();
    publish(&mut bus, 0.0, 0.0, 1000);
    assert_eq!(
        *trace.lock().unwrap(),
        [("device", Some(0.0), false), ("mcu", Some(0.0), false)]
    );
    assert_eq!(old_handle.observation().voltage_v, Some(0.0));
    assert!(!domain.powered());
    let brown_out = bus.mcu_power.status().unwrap().brown_out.unwrap();
    assert_eq!(brown_out.pending_changes, 0);
    assert!(!brown_out.applied.enabled);
}

#[test]
fn device_generation_exhaustion_cannot_publish_mcu_state_or_callbacks() {
    let (mut bus, domain, trace) = fixture();
    publish(&mut bus, 1.1, 1.8, 0);
    domain.state.lock().unwrap().generation = u64::MAX;
    let before = serde_json::to_value(bus.mcu_power.status()).unwrap();
    let callbacks_before = trace.lock().unwrap().clone();
    let error = bus
        .apply_power_rail_observations(&device_values(0.0), Some((&mcu_values(0.0), 1000)))
        .unwrap_err();
    assert!(matches!(error, crate::power::RailBatchError::Device(_)));
    assert_eq!(
        serde_json::to_value(bus.mcu_power.status()).unwrap(),
        before
    );
    assert!(domain.powered());
    assert_eq!(domain.generation(), u64::MAX);
    assert_eq!(*trace.lock().unwrap(), callbacks_before);
}

#[test]
fn duplicate_device_target_cannot_consume_a_pending_mcu_control() {
    let (mut bus, domain, trace) = fixture();
    publish(&mut bus, 1.1, 1.8, 0);
    let before = serde_json::to_value(bus.mcu_power.status()).unwrap();
    let callbacks_before = trace.lock().unwrap().clone();
    let mut updates = device_values(0.0);
    updates.push(("observer".into(), "vdd".into(), 1.8));
    let error = bus
        .apply_power_rail_observations(&updates, Some((&mcu_values(0.0), 1000)))
        .unwrap_err();
    assert!(matches!(error, crate::power::RailBatchError::Device(_)));
    assert_eq!(
        serde_json::to_value(bus.mcu_power.status()).unwrap(),
        before
    );
    assert!(domain.powered());
    assert_eq!(*trace.lock().unwrap(), callbacks_before);
}

#[test]
fn poisoned_device_lock_is_reported_before_any_mcu_publication() {
    let (mut bus, domain, trace) = fixture();
    publish(&mut bus, 1.1, 1.8, 0);
    let before = serde_json::to_value(bus.mcu_power.status()).unwrap();
    let callbacks_before = trace.lock().unwrap().clone();
    let poisoned = domain.clone();
    assert!(std::thread::spawn(move || {
        let _guard = poisoned.state.lock().unwrap();
        panic!("test-only poisoned receiver");
    })
    .join()
    .is_err());
    let error = bus
        .apply_power_rail_observations(&device_values(0.0), Some((&mcu_values(0.0), 1000)))
        .unwrap_err();
    assert!(matches!(error, crate::power::RailBatchError::Device(_)));
    assert_eq!(
        serde_json::to_value(bus.mcu_power.status()).unwrap(),
        before
    );
    assert_eq!(*trace.lock().unwrap(), callbacks_before);
    let state = domain.state.lock().unwrap_err().into_inner();
    assert!(state.on);
    assert_eq!(state.voltages["vdd"], 1.8);
}

#[test]
fn mcu_deadline_overflow_cannot_publish_prepared_device_states() {
    let (mut bus, domain, trace) = fixture();
    publish(&mut bus, 1.1, 1.8, 0);
    let before = serde_json::to_value(bus.mcu_power.status()).unwrap();
    let callbacks_before = trace.lock().unwrap().clone();
    let error = bus
        .apply_power_rail_observations(&device_values(0.0), Some((&mcu_values(1.09), u64::MAX)))
        .unwrap_err();
    assert!(
        matches!(error, crate::power::RailBatchError::Mcu(ref reason) if reason.contains("time overflow"))
    );
    assert_eq!(
        serde_json::to_value(bus.mcu_power.status()).unwrap(),
        before
    );
    assert!(domain.powered());
    assert_eq!(domain.state.lock().unwrap().voltages["vdd"], 1.8);
    assert_eq!(*trace.lock().unwrap(), callbacks_before);
}
