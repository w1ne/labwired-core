//! Synthetic CPU/analog coupling tests; no exact chip or hardware claim.
use crate as labwired_core;
include!("../../tests/internal_regulator_support/mod.rs");
use super::{ResolvedWorldNode, World};
use crate::cosim::CosimSignalValue;

fn controlled_world(
    avdd_a: f64,
) -> (
    World,
    RegulatorControl,
    RegulatorControl,
    Observations,
    Observations,
) {
    let mut world = World::new("controlled-buck".into());
    let (a, control_a, observed_a) = machine();
    let (b, control_b, observed_b) = machine();
    world.add_machine("a".into(), Box::new(a));
    world.add_machine("b".into(), Box::new(b));
    world.set_node_hz("a", 16_000_000);
    world.set_node_hz("b", 16_000_000);
    let config = model(&[("a", avdd_a), ("b", 3.3)], true);
    world.cosim = Some(
        super::cosim::WorldCosim::new(&[config], &mut world.machines, &world.node_hz).unwrap(),
    );
    world
        .set_cosim_signal("ui.a.enable", CosimSignalValue::F64(avdd_a))
        .unwrap();
    world
        .set_cosim_signal("ui.b.enable", CosimSignalValue::F64(3.3))
        .unwrap();
    (world, control_a, control_b, observed_a, observed_b)
}
fn run(world: &mut World, ns: u64) {
    let results = world.run_until_ps(ns * 1000).unwrap();
    assert!(results.values().all(Result::is_ok), "{results:?}");
}
#[test]
fn controlled_buck_world_primes_callback_and_holds_each_node_command_from_interval_start() {
    let (mut world, a, b, observed_a, observed_b) = controlled_world(3.3);
    let initial = observed_a.lock().unwrap()[0];
    assert_eq!(initial.time_ns, 0);
    assert_eq!(initial.applied_revision, 0);
    assert!(initial.output_enabled && initial.qualified);
    assert!((initial.feedback_voltage_v - 1.1).abs() < 0.002);
    assert_eq!(
        world.machines["a"].get_register(0),
        0,
        "CPU has not executed before the operating point callback"
    );
    // Public run_until_ps rounds to the next completed analog interval. Freeze the
    // interval explicitly here, then issue a command before its completion.
    // The standalone integration test separately executes the actual T/2 write.
    world
        .cosim
        .as_mut()
        .unwrap()
        .sample_interval(&world.machines)
        .unwrap();
    assert_eq!(world.cosim_status().unwrap().completed_time_ns, 0);
    let mut next = a.configuration();
    next.enabled = false;
    a.request(next).unwrap();
    run(&mut world, 1000);
    assert_eq!(a.observation().unwrap().applied_revision, 0);
    assert!(a.observation().unwrap().output_enabled);
    assert!(a.pending_update());
    run(&mut world, 2000);
    assert_eq!(a.observation().unwrap().applied_revision, 1);
    assert!(!a.observation().unwrap().output_enabled);
    assert!(!a.pending_update());
    assert_eq!(b.observation().unwrap().applied_revision, 0);
    assert!(b.observation().unwrap().output_enabled);
    assert_eq!(
        observed_a
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.time_ns)
            .collect::<Vec<_>>(),
        vec![0, 1000, 2000]
    );
    assert_eq!(
        observed_b
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.time_ns)
            .collect::<Vec<_>>(),
        vec![0, 1000, 2000]
    );
}
#[test]
fn controlled_buck_autonomous_energy_path_advances_while_the_cpu_is_held() {
    let (mut world, a, b, observed_a, _) = controlled_world(0.0);
    assert!(!a.observation().unwrap().qualified);
    assert_eq!(a.observation().unwrap().input_current_a, 0.0);
    assert!(a.observation().unwrap().feedback_voltage_v.abs() < 1e-10);
    world
        .set_cosim_signal("ui.a.enable", CosimSignalValue::F64(3.3))
        .unwrap();
    run(&mut world, 20_000);
    let observed = a.observation().unwrap();
    assert!(observed.output_enabled && observed.qualified);
    assert!(observed.input_current_a > 0.0);
    assert!(
        observed.feedback_voltage_v > 0.1 && observed.feedback_voltage_v < 0.9,
        "{:?}",
        observed
    );
    assert_eq!(
        world.machines["a"].get_register(0),
        0,
        "regulator charges the real external capacitor before POR release"
    );
    assert!(world.machines["b"].get_register(0) > 0);
    assert!(b.observation().unwrap().output_enabled);
    assert_eq!(observed_a.lock().unwrap().len(), 21);
}
#[test]
fn controlled_buck_empty_world_model_admission_fails_before_firmware_or_peripheral_build() {
    let mut chip = ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    chip.internal_regulators = vec![serde_yaml::from_str(REGULATOR_YAML).unwrap()];
    let environment: labwired_config::EnvironmentManifest = serde_yaml::from_str(
        "name: missing-stage
nodes:
  - {id: a, system: a.yaml, firmware: a.elf}
",
    )
    .unwrap();
    let node = ResolvedWorldNode {
        id: "a".into(),
        chip,
        system: serde_yaml::from_str(
            "name: fixture
chip: stm32f401
",
        )
        .unwrap(),
        firmware: crate::system::node::NodeFirmware::from_bytes(Vec::new()),
        blobs: Default::default(),
    };
    let error = World::from_resolved(environment, vec![node])
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("no co-simulation model"), "{error}");
}
