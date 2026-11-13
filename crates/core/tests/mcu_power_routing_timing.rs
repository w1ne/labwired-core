//! Fresh rail ownership is independent of unrelated model rates and host calls.
mod mcu_power_support;
use labwired_config::{ChipDescriptor, CosimModelConfig, SystemManifest};
use labwired_core::{
    bus::SystemBus,
    cosim::{CosimAdapter, CosimSession, CosimSignalValue, CosimStep, CosimStepResult},
    power_domains::PowerDomainHandle,
    Peripheral, SimResult,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

struct RailSource {
    ports: Vec<String>,
    omit_flash_after: Option<u64>,
}
impl CosimAdapter for RailSource {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        Ok(CosimStepResult {
            outputs: self
                .ports
                .iter()
                .filter(|port| {
                    !(port.as_str() == "flash"
                        && self
                            .omit_flash_after
                            .is_some_and(|time| step.time_ns >= time))
                })
                .map(|port| {
                    (
                        port.clone(),
                        CosimSignalValue::F64(if port == "core" { 1.1 } else { 3.3 }),
                    )
                })
                .collect(),
        })
    }
}
fn bus() -> SystemBus {
    let mut chip = ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    chip.supply_monitor = None;
    chip.power_domains = Some(mcu_power_support::config());
    // No CPU executes in these routing tests. This explicit clock makes one
    // caller cycle equal one nanosecond, without rounding a sample timestamp.
    chip.cpu_hz = 1_000_000_000;
    let system: SystemManifest =
        serde_yaml::from_str("name: timing-fixture\nchip: stm32f401\n").unwrap();
    SystemBus::from_config(&chip, &system).unwrap()
}
fn model(id: &str, step_ns: u64, ports: &[&str]) -> CosimModelConfig {
    let outputs: String = ports
        .iter()
        .map(|port| format!("  {port}: board.power.{port}_volts\n"))
        .collect();
    serde_yaml::from_str(&format!(
        "id: {id}\nadapter: mock\nstep_ns: {step_ns}\noutputs:\n{outputs}"
    ))
    .unwrap()
}
fn session(
    bus: &SystemBus,
    unrelated_period: Option<u64>,
    omit_flash_after: Option<u64>,
) -> CosimSession {
    let mut configs = vec![
        model("a", 1000, &["core", "io"]),
        model("b", 1000, &["detector", "flash"]),
    ];
    if let Some(period) = unrelated_period {
        configs.push(
            serde_yaml::from_str(&format!(
                "id: unrelated\nadapter: mock\nstep_ns: {period}\n"
            ))
            .unwrap(),
        );
    }
    CosimSession::new_with_factory(&configs, bus, |config| {
        Ok(Box::new(RailSource {
            ports: config.outputs.keys().cloned().collect(),
            omit_flash_after,
        }))
    })
    .unwrap()
    .unwrap()
}
fn advance(session: &mut CosimSession, bus: &mut SystemBus, time: u64) {
    let (_, errors) = session.advance_to(time, bus).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn faster_unrelated_steps_cannot_advance_por_or_apply_queued_bod_control() {
    let mut bus = bus();
    let mut session = session(&bus, Some(100), None);
    advance(&mut session, &mut bus, 100);
    assert_eq!(bus.mcu_power.status().unwrap().last_sample_ns, None);
    for time in (200..=1000).step_by(100) {
        advance(&mut session, &mut bus, time);
    }
    assert_eq!(bus.mcu_power.status().unwrap().last_sample_ns, Some(1000));
    let control = bus.mcu_power.supervisor_control().unwrap();
    control.configure_brown_out(false, 0.946, 0.95546).unwrap();
    for time in (1100..=1900).step_by(100) {
        advance(&mut session, &mut bus, time);
        let status = bus.mcu_power.status().unwrap();
        assert_eq!(status.last_sample_ns, Some(1000));
        assert!(status.brown_out.unwrap().applied.enabled);
    }
    advance(&mut session, &mut bus, 2000);
    assert!(
        !bus.mcu_power
            .status()
            .unwrap()
            .brown_out
            .unwrap()
            .applied
            .enabled
    );
    for time in (2100..=3900).step_by(100) {
        advance(&mut session, &mut bus, time);
    }
    assert!(bus.mcu_power.is_held());
    advance(&mut session, &mut bus, 4000);
    assert!(!bus.mcu_power.is_held());
    assert_eq!(bus.mcu_power.status().unwrap().reset_events, 1);
}

#[test]
fn nondividing_host_boundaries_keep_the_owning_model_timestamp() {
    let mut bus = bus();
    let mut session = session(&bus, Some(700), None);
    for (host_time, expected_sample) in [
        (700, None),
        (1400, Some(1000)),
        (2100, Some(2000)),
        (2800, Some(2000)),
        (3500, Some(3000)),
        (4200, Some(4000)),
    ] {
        advance(&mut session, &mut bus, host_time);
        assert_eq!(
            bus.mcu_power.status().unwrap().last_sample_ns,
            expected_sample
        );
        if host_time == 700 {
            assert_eq!(
                session.cycles_until_boundary(host_time),
                300,
                "CPU scheduling must stop at the 1000 ns rail model, not 1400 ns"
            );
        }
    }
    assert!(!bus.mcu_power.is_held());
    assert_eq!(
        bus.mcu_power.status().unwrap().rails["core"].observed_time_ns,
        Some(4000)
    );
}

#[test]
fn a_missing_fresh_peer_rail_cannot_reuse_the_previous_signal_store_value() {
    let mut bus = bus();
    let mut session = session(&bus, Some(100), Some(2000));
    for time in (100..=1900).step_by(100) {
        advance(&mut session, &mut bus, time);
    }
    assert_eq!(
        session.signals().get("board.power.flash_volts"),
        Some(&CosimSignalValue::F64(3.3))
    );
    let before = serde_json::to_value(bus.mcu_power.status()).unwrap();
    let (_, errors) = session.advance_to(2000, &mut bus).unwrap();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("fresh")),
        "{errors:?}"
    );
    assert_eq!(
        serde_json::to_value(bus.mcu_power.status()).unwrap(),
        before
    );
}

#[derive(Debug)]
struct BatchWitness {
    handles: [PowerDomainHandle; 4],
    seen: Arc<Mutex<Vec<[Option<f64>; 4]>>>,
}
impl Peripheral for BatchWitness {
    fn read(&self, _: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _: u64, _: u8) -> SimResult<()> {
        Ok(())
    }
    fn sync_power_domains(&mut self) {
        self.seen.lock().unwrap().push(std::array::from_fn(|index| {
            self.handles[index].observation().voltage_v
        }));
    }
}
#[test]
fn two_same_rate_models_publish_one_complete_atomic_rail_batch() {
    let mut bus = bus();
    let seen = Arc::new(Mutex::new(Vec::new()));
    bus.add_peripheral(
        "batch-witness",
        0x60000000,
        4,
        None,
        Box::new(BatchWitness {
            handles: ["core", "io", "detector", "flash"]
                .map(|name| bus.mcu_power.handle(name).unwrap()),
            seen: seen.clone(),
        }),
    );
    let mut session = session(&bus, None, None);
    advance(&mut session, &mut bus, 1000);
    assert_eq!(
        *seen.lock().unwrap(),
        [[None; 4], [Some(1.1), Some(3.3), Some(3.3), Some(3.3)]]
    );
    assert_eq!(bus.mcu_power.status().unwrap().last_sample_ns, Some(1000));
}

#[test]
fn skipped_owner_intervals_fail_without_publishing_the_last_value_as_history() {
    let mut bus = bus();
    let mut session = session(&bus, None, None);
    let (_, errors) = session.advance_to(3000, &mut bus).unwrap();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("skipped")),
        "{errors:?}"
    );
    assert_eq!(bus.mcu_power.status().unwrap().last_sample_ns, None);
    assert_eq!(
        bus.mcu_power
            .handle("core")
            .unwrap()
            .observation()
            .voltage_v,
        None
    );
}
