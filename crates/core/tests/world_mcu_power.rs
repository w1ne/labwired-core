//! Real Cortex-M scheduling and MNA with synthetic named supply domains.
//! No exact RP device/peripheral or cold-board qualification is inferred.
mod mcu_power_support;
use labwired_config::{ChipDescriptor, EnvironmentManifest, SystemManifest};
use labwired_core::cosim::CosimSignalValue;
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{ResolvedWorldNode, World};
use std::path::Path;

fn chip() -> ChipDescriptor {
    let mut chip = ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    chip.supply_monitor = None;
    chip.power_domains = Some(mcu_power_support::config());
    chip.gpio_input_thresholds=Some(serde_yaml::from_str("{vil_max_v: 0.8, vih_min_v: 2.0, nominal_io_voltage_v: 3.3, minimum_io_voltage_v: 2.5}").unwrap());
    chip.reset_input=Some(serde_yaml::from_str("{pin: RUN, active_low: true, vil_max_v: 0.8, vih_min_v: 2.0, io_domain: io, nominal_io_voltage_v: 3.3, min_assert_ns: 0, invalid_pulse_ns: 0}").unwrap());
    chip
}
fn environment(dynamic: bool) -> EnvironmentManifest {
    let (mut inputs, mut outputs, mut netlist, mut sources, mut probes) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );
    for node in ["a", "b"] {
        for (domain, nominal) in [
            ("core", 1.1),
            ("io", 3.3),
            ("detector", 3.3),
            ("flash", 3.3),
            ("run", 3.3),
        ] {
            let name = format!("{node}_{domain}");
            let voltage = if dynamic { 0.0 } else { nominal };
            netlist.push_str(&format!("        V{name} {name} 0 dc {voltage}\n"));
            let route = if domain == "run" {
                "board.reset.run_volts".to_owned()
            } else {
                format!("board.power.{domain}_volts")
            };
            outputs.push_str(&format!("      {name}: node.{node}.{route}\n"));
            probes.push_str(&format!("        {name}: v({name})\n"));
            if dynamic {
                inputs.push_str(&format!("      {name}: ui.{node}.{domain}\n"));
                sources.push_str(&format!("        {name}: V{name}\n"));
            }
        }
    }
    // The capacitor is a physical timing witness while both CPUs are held.
    netlist.push_str("        Vexc drive 0 dc 0\n        Rtime drive cap 1000\n        Ctime cap 0 1u\n        .end\n");
    inputs.push_str("      excitation: ui.excitation\n");
    sources.push_str("        excitation: Vexc\n");
    serde_yaml::from_str(&format!("name: named-power\nnodes:\n  - {{id: a, system: a.yaml, firmware: a.elf}}\n  - {{id: b, system: b.yaml, firmware: b.elf}}\ncosim_models:\n  - id: circuit\n    adapter: analog\n    step_ns: 1000\n    inputs:\n{inputs}    outputs:\n{outputs}    config:\n      substeps: 1\n      netlist_text: |\n{netlist}      sources:\n{sources}      probes:\n{probes}      trace: [v(cap)]\n")).unwrap()
}
fn build(manifest: EnvironmentManifest) -> anyhow::Result<World> {
    World::from_resolved(
        manifest,
        ["a", "b"]
            .into_iter()
            .map(|id| ResolvedWorldNode {
                id: id.into(),
                system: serde_yaml::from_str::<SystemManifest>(
                    "name: controller\nchip: stm32f401\n",
                )
                .unwrap(),
                chip: chip(),
                firmware: NodeFirmware::from_bytes(mcu_power_support::firmware()),
                blobs: Default::default(),
            })
            .collect(),
    )
}
fn set(world: &mut World, node: &str, domain: &str, volts: f64) {
    world
        .set_cosim_signal(&format!("ui.{node}.{domain}"), CosimSignalValue::F64(volts))
        .unwrap();
}
fn power(world: &mut World, node: &str) {
    for (domain, volts) in [
        ("core", 1.1),
        ("io", 3.3),
        ("detector", 3.3),
        ("flash", 3.3),
        ("run", 3.3),
    ] {
        set(world, node, domain, volts);
    }
}
fn run(world: &mut World, ns: u64) {
    let result = world.run_until_ps(ns * 1000).unwrap();
    assert!(result.values().all(Result::is_ok), "{result:?}");
    assert_eq!(world.cosim_status().unwrap().completed_time_ns, ns);
}

#[test]
fn constant_operating_point_primes_all_rails_then_obeys_por_delay() {
    let mut world = build(environment(false)).unwrap();
    let status = world.cosim_status().unwrap();
    for node in ["a", "b"] {
        let power = &status.power_domains[node];
        assert!(power.measured_mode);
        assert!(power.held_in_reset);
        assert_eq!(power.rails["core"].voltage_v, Some(1.1));
        assert_eq!(power.rails["io"].voltage_v, Some(3.3));
        assert_eq!(power.reset_events, 0);
        assert_eq!(world.machines[node].get_register(0), 0);
    }
    run(&mut world, 2000);
    assert_eq!(world.machines["a"].get_register(0), 0);
    run(&mut world, 3000);
    assert_eq!(world.machines["a"].get_register(0), 0);
    let power = &world.cosim_status().unwrap().power_domains["a"];
    assert!(!power.held_in_reset);
    assert_eq!(power.reset_events, 1);
    assert!(!power.restart_pending);
    run(&mut world, 4000);
    assert!(world.machines["a"].get_register(0) > 0);
}

#[test]
fn distinct_nodes_and_rc_physics_continue_during_unqualified_power() {
    let mut world = build(environment(true)).unwrap();
    world
        .set_cosim_signal("ui.excitation", CosimSignalValue::F64(1.0))
        .unwrap();
    run(&mut world, 100_000);
    assert_eq!(world.machines["a"].get_register(0), 0);
    assert_eq!(world.machines["b"].get_register(0), 0);
    let series = world.analog_series().unwrap();
    let cap = series
        .channels
        .iter()
        .find(|channel| channel.name.contains("cap"))
        .unwrap();
    assert!(
        cap.points.last().unwrap().1 > 0.09,
        "MNA continued while CPUs had no qualified power"
    );
    power(&mut world, "a");
    run(&mut world, 105_000);
    assert!(world.machines["a"].get_register(0) > 0);
    assert_eq!(world.machines["b"].get_register(0), 0);
    let state = world.cosim_status().unwrap();
    assert!(state.power_domains["a"].execution_qualified);
    assert!(!state.power_domains["b"].execution_qualified);
}

#[test]
fn run_hold_is_independent_and_release_does_not_duplicate_chip_reset() {
    let mut world = build(environment(true)).unwrap();
    power(&mut world, "a");
    power(&mut world, "b");
    run(&mut world, 10_000);
    let before = world.machines["a"].get_register(0);
    set(&mut world, "a", "run", 0.0);
    run(&mut world, 11_000);
    let held = world.machines["a"].get_register(0);
    assert!(held >= before);
    run(&mut world, 20_000);
    assert_eq!(world.machines["a"].get_register(0), held);
    let status = world.cosim_status().unwrap();
    assert!(status.reset_inputs["a"].held);
    assert_eq!(status.power_domains["a"].reset_events, 2);
    assert_eq!(status.power_domains["b"].reset_events, 1);
    set(&mut world, "a", "run", 3.3);
    run(&mut world, 21_000);
    assert_eq!(
        world.cosim_status().unwrap().power_domains["a"].reset_events,
        2
    );
    run(&mut world, 22_000);
    assert!(world.machines["a"].get_register(0) > held);
}

#[test]
fn unknown_duplicate_or_legacy_alias_rails_fail_before_world_runs() {
    for (port, route) in [
        ("a_core", "node.a.board.power.missing_volts"),
        ("alias", "node.a.board.power.core_volts"),
        ("a_core", "node.a.board.power.vdd_volts"),
    ] {
        let mut manifest = environment(false);
        manifest.cosim_models[0]
            .outputs
            .insert(port.into(), route.into());
        assert!(build(manifest).is_err(), "accepted {route}");
    }
}

#[test]
fn observed_power_status_records_nominal_threshold_scope_without_rounding_rail() {
    let mut world = build(environment(true)).unwrap();
    power(&mut world, "a");
    set(&mut world, "a", "io", 3.300167);
    run(&mut world, 5000);
    let status = world.cosim_status().unwrap();
    let calibration = status.power_domains["a"]
        .input_threshold_calibration
        .as_ref()
        .unwrap();
    assert!(calibration.currently_qualified);
    assert_eq!(calibration.nominal_io_voltage_v, 3.3);
    assert_eq!(calibration.minimum_io_voltage_v, Some(2.5));
    assert!((status.power_domains["a"].rails["io"].voltage_v.unwrap() - 3.300167).abs() < 1e-9);
    set(&mut world, "a", "io", 2.1);
    run(&mut world, 6000);
    let status = world.cosim_status().unwrap();
    assert_eq!(
        status.power_domains["a"].rails["io"].within_operating_range,
        Some(true)
    );
    assert!(
        !status.power_domains["a"]
            .input_threshold_calibration
            .as_ref()
            .unwrap()
            .currently_qualified
    );
}
