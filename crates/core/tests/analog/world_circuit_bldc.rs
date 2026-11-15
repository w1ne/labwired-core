//! Generic two-node/one-island winding proof. Tiny real Cortex-M programs run
//! only as scheduler witnesses; this is not AM32 or a sensorless commutation test.
use labwired_config::{ChipDescriptor, EnvironmentManifest, SystemManifest};
use labwired_core::cosim::CosimSignalValue;
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{ResolvedWorldNode, World};
use std::path::Path;

const PHASE_R: f64 = 0.35;
const PHASE_L: f64 = 0.00018;
const INERTIA: f64 = 1e-7;

fn firmware() -> Vec<u8> {
    // ELF32 ARM, vector table, ADD r0,#1; B loop. The register witnesses hold.
    let code = [0x3001u16, 0xe7fd];
    let length = 8 + code.len() as u32 * 2;
    let mut bytes = vec![0; 84 + length as usize];
    bytes[0..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    for (offset, value) in [(16, 2u16), (18, 40), (40, 52), (42, 32), (44, 1)] {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [
        (20, 1u32),
        (24, 0x08000009),
        (28, 52),
        (36, 0x05000000),
        (52, 1),
        (56, 84),
        (60, 0x08000000),
        (64, 0x08000000),
        (68, length),
        (72, length),
        (76, 5),
        (80, 4),
        (84, 0x20001000),
        (88, 0x08000009),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (i, instruction) in code.iter().enumerate() {
        bytes[92 + i * 2..94 + i * 2].copy_from_slice(&instruction.to_le_bytes());
    }
    bytes
}

fn system() -> SystemManifest {
    serde_yaml::from_str(
        r#"
name: three-lead-winding
chip: stm32f401
external_devices:
  - id: M1
    type: bldc-motor
    connection: gpio
    config:
      armature: circuit
      resistance_ohm: 0.35
      inductance_h: 0.00018
      torque_constant_nm_per_a: 0.04
      back_emf_constant_v_per_rad_s: 0.04
      rotor_inertia_kg_m2: 0.0000001
      viscous_friction_nm_per_rad_s: 0
      load_torque_nm: 0
      encoder_cpr: 2048
      pole_pairs: 7
      current_limit_a: 40
"#,
    )
    .unwrap()
}

fn environment(step_ns: u64, logic_v: f64, measured_supply: bool) -> EnvironmentManifest {
    let mut inputs = String::new();
    let mut outputs = String::new();
    let mut sources = String::new();
    let mut probes = String::from("        rail: v(rail)\n        battery_current: i(Vbat)\n");
    let mut netlist =
        format!("Vbat battery 0 dc 6\nRsource battery rail 0.2\nVlogic logic 0 dc {logic_v}\n");
    for owner in ["a", "b"] {
        inputs.push_str(&format!("      enable_{owner}: ui.{owner}.enabled\n"));
        outputs.push_str(&format!(
            "      logic_{owner}: node.{owner}.board.power.vdd_volts\n"
        ));
        probes.push_str(&format!("        logic_{owner}: v(logic)\n"));
        if measured_supply {
            outputs.push_str(&format!(
                "      supply_{owner}: node.{owner}.board.motor.M1.supply_v\n"
            ));
            probes.push_str(&format!("        supply_{owner}: v(rail)\n"));
        }
        let (high, low) = if owner == "a" { ("c", "b") } else { ("b", "c") };
        netlist.push_str(&format!("Sh_{owner} rail {owner}_{high} enable_{owner} ron=0.01 roff=1e12\nSl_{owner} {owner}_{low} 0 enable_{owner} ron=0.01 roff=1e12\n"));
        for phase in ["a", "b", "c"] {
            let tag = format!("{owner}_{phase}");
            inputs.push_str(&format!(
                "      emf_{tag}: node.{owner}.board.motor.M1.phase_{phase}_back_emf_v\n"
            ));
            outputs.push_str(&format!(
                "      current_{tag}: node.{owner}.board.motor.M1.phase_{phase}_current_a\n"
            ));
            sources.push_str(&format!("        emf_{tag}: Vemf_{tag}\n"));
            probes.push_str(&format!("        current_{tag}: i(L{tag})\n"));
            netlist.push_str(&format!("R{tag} {tag} {tag}r {PHASE_R}\nL{tag} {tag}r {tag}l {PHASE_L}\nVemf_{tag} {tag}l {owner}_star dc 0\n"));
        }
    }
    let deck = netlist
        .lines()
        .map(|line| format!("        {line}\n"))
        .collect::<String>();
    serde_yaml::from_str(&format!("name: two-shafts-one-island\nnodes:\n  - {{id: a, system: a.yaml, firmware: a.elf}}\n  - {{id: b, system: b.yaml, firmware: b.elf}}\ncosim_models:\n  - id: circuit\n    adapter: analog\n    step_ns: {step_ns}\n    inputs:\n{inputs}    outputs:\n{outputs}    config:\n      substeps: 1\n      netlist_text: |\n{deck}      sources:\n{sources}      probes:\n{probes}")).unwrap()
}

fn from_environment(manifest: EnvironmentManifest) -> anyhow::Result<World> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/stm32f401.yaml"))?;
    let nodes = ["a", "b"]
        .into_iter()
        .map(|id| ResolvedWorldNode {
            id: id.into(),
            system: system(),
            chip: chip.clone(),
            firmware: NodeFirmware::from_bytes(firmware()),
            blobs: Default::default(),
        })
        .collect();
    World::from_resolved(manifest, nodes)
}
fn world(step_ns: u64, logic_v: f64, measured_supply: bool) -> World {
    from_environment(environment(step_ns, logic_v, measured_supply)).unwrap()
}
fn enable(world: &mut World, owner: &str) {
    world
        .set_cosim_signal(&format!("ui.{owner}.enabled"), CosimSignalValue::Bool(true))
        .unwrap();
}
fn run(world: &mut World, target_ns: u64) {
    let results = world
        .run_until_ps(target_ns * 1000)
        .expect("analog-only world must advance");
    assert!(results.values().all(Result::is_ok), "{results:?}");
    assert_eq!(world.cosim_status().unwrap().completed_time_ns, target_ns);
}
fn current(world: &World, owner: &str, phase: &str) -> f64 {
    match world.cosim_signals().unwrap().get(&format!(
        "node.{owner}.board.motor.M1.phase_{phase}_current_a"
    )) {
        Some(CosimSignalValue::F64(v)) => *v,
        other => panic!("missing actual phase current: {other:?}"),
    }
}
fn probe(world: &World, name: &str) -> f64 {
    let channel = world
        .analog_channels()
        .iter()
        .position(|c| c.name == format!("circuit.{name}"))
        .unwrap();
    f64::from(
        world
            .analog_trace_snapshot(0)
            .samples
            .last()
            .unwrap()
            .values[channel],
    )
}

#[test]
fn same_named_motors_are_independent_but_load_one_real_supply() {
    let mut one = world(1000, 0.0, false);
    let mut both = world(1000, 0.0, false);
    enable(&mut one, "a");
    enable(&mut both, "a");
    enable(&mut both, "b");
    run(&mut one, 100_000);
    run(&mut both, 100_000);
    let a = both.machines["a"].motor_snapshots().remove(0);
    let b = both.machines["b"].motor_snapshots().remove(0);
    assert!(a.speed_rpm > 0.0 && b.speed_rpm < 0.0, "{a:?} {b:?}");
    assert!(one.machines["b"].motor_snapshots()[0].speed_rpm.abs() < 1e-4);
    assert!(
        probe(&both, "rail") < probe(&one, "rail"),
        "two actual loads must sag the same rail"
    );
    assert!(a.speed_rpm < one.machines["a"].motor_snapshots()[0].speed_rpm);
    assert_eq!(
        a.phase_currents_a,
        Some([
            current(&both, "a", "a"),
            current(&both, "a", "b"),
            current(&both, "a", "c")
        ])
    );
    assert_eq!(a.current_a, None);
    assert_eq!(a.bus_voltage_v, None);
    let timing = a.coupling.unwrap();
    assert_eq!(timing.mode, "interval-start-zero-order-hold");
    assert_eq!(timing.step_ns, Some(1000));
    assert_eq!(timing.completed_time_ns, Some(100_000));
    assert_eq!(both.machines["a"].motor_series().end_time_ns, 100_000);
}

#[test]
fn reset_hold_does_not_freeze_mechanics_and_cpu_ticks_do_not_integrate_twice() {
    let mut held = world(1000, 0.0, true);
    let mut running = world(1000, 3.3, true);
    enable(&mut held, "a");
    enable(&mut running, "a");
    run(&mut held, 100_000);
    run(&mut running, 100_000);
    assert_eq!(held.machines["a"].get_register(0), 0);
    assert!(running.machines["a"].get_register(0) > 0);
    assert_eq!(
        held.machines["a"].motor_snapshots(),
        running.machines["a"].motor_snapshots()
    );
    let motor = held.machines["a"].motor_snapshots().remove(0);
    assert!(motor.speed_rpm > 0.0);
    assert!((motor.bus_voltage_v.unwrap() - probe(&held, "rail")).abs() < 1e-6);
    assert_eq!(held.cosim_status().unwrap().model_steps, 100);
}

#[test]
fn declared_circuit_bldc_requires_a_complete_model_owner() {
    let mut manifest = environment(1000, 0.0, false);
    let model = &mut manifest.cosim_models[0];
    model
        .inputs
        .retain(|_, path| !path.starts_with("node.a.board.motor.M1."));
    model
        .outputs
        .retain(|_, path| !path.starts_with("node.a.board.motor.M1."));
    let error = from_environment(manifest)
        .err()
        .expect("a declared circuit winding must not silently lose all six routes")
        .to_string();
    assert!(error.contains("no complete analog-model owner"), "{error}");
}

#[test]
fn missing_phase_current_or_emf_and_unknown_motor_fail_before_running() {
    for route in ["phase_a_current_a", "phase_b_back_emf_v"] {
        let mut manifest = environment(1000, 0.0, false);
        manifest.cosim_models[0]
            .inputs
            .retain(|_, path| !path.ends_with(route));
        manifest.cosim_models[0]
            .outputs
            .retain(|_, path| !path.ends_with(route));
        let error = from_environment(manifest)
            .err()
            .expect("incomplete winding must fail")
            .to_string();
        assert!(error.contains("all three phase"), "{error}");
    }
    let mut manifest = environment(1000, 0.0, false);
    for path in manifest.cosim_models[0].inputs.values_mut() {
        *path = path.replace(".M1.", ".missing.");
    }
    let error = from_environment(manifest)
        .err()
        .expect("unknown shaft must fail")
        .to_string();
    assert!(error.contains("missing"), "{error}");
}

fn energy_run(step_ns: u64) -> (f64, f64, f64) {
    let mut world = world(step_ns, 0.0, false);
    enable(&mut world, "a");
    let mut input_energy = 0.0;
    let mut resistor_heat = 0.0;
    let dt = step_ns as f64 * 1e-9;
    for time in (step_ns..=100_000).step_by(step_ns as usize) {
        run(&mut world, time);
        let battery_i = -probe(&world, "battery_current");
        input_energy += 6.0 * battery_i * dt;
        let mut phase_loss = 0.0;
        let mut switch_loss = 0.0;
        for owner in ["a", "b"] {
            for phase in ["a", "b", "c"] {
                phase_loss += PHASE_R * current(&world, owner, phase).powi(2);
            }
            if owner == "a" {
                switch_loss += 0.01
                    * (current(&world, owner, "b").powi(2) + current(&world, owner, "c").powi(2));
            }
        }
        resistor_heat += (phase_loss + switch_loss + 0.2 * battery_i.powi(2)) * dt;
    }
    let mut stored = 0.0;
    for owner in ["a", "b"] {
        let snapshot = world.machines[owner].motor_snapshots().remove(0);
        let omega = snapshot.speed_rpm * std::f64::consts::TAU / 60.0;
        stored += 0.5 * INERTIA * omega * omega;
        stored += 0.5
            * PHASE_L
            * snapshot
                .phase_currents_a
                .unwrap()
                .iter()
                .map(|i| i * i)
                .sum::<f64>();
    }
    (
        input_energy - resistor_heat - stored,
        input_energy,
        world.machines["a"].motor_snapshots()[0].speed_rpm,
    )
}

#[test]
fn actual_winding_energy_and_motion_converge_as_the_hold_interval_shrinks() {
    let coarse = energy_run(2000);
    let medium = energy_run(1000);
    let fine = energy_run(500);
    eprintln!("energy residual/input/speed: {coarse:?} {medium:?} {fine:?}");
    assert!(fine.0.abs() < medium.0.abs() && medium.0.abs() < coarse.0.abs());
    assert!(fine.0.abs() / fine.1 < 0.02, "{fine:?}");
    assert!((fine.2 - medium.2).abs() < (medium.2 - coarse.2).abs());
    assert!((fine.2 - medium.2).abs() / fine.2.abs() < 0.01);
}

#[test]
fn routing_requires_each_boundary_once_and_rejects_stale_or_invalid_current_vectors() {
    use labwired_core::bus::SystemBus;
    use labwired_core::cosim::{CosimSignals, SignalRouter};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/stm32f401.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &system()).unwrap();
    let before = bus.motor_snapshots().remove(0);
    assert_eq!(before.control_state, "circuit:no-current");
    assert_eq!(before.phase_currents_a, None);
    assert_eq!(before.current_a, None);
    assert_eq!(before.bus_voltage_v, None);
    assert_eq!(before.coupling.unwrap().step_ns, None);
    let mut model = environment(1000, 0.0, false).cosim_models.remove(0);
    model
        .inputs
        .retain(|_, p| p.starts_with("node.a.board.motor."));
    model
        .outputs
        .retain(|_, p| p.starts_with("node.a.board.motor."));
    for path in model.inputs.values_mut().chain(model.outputs.values_mut()) {
        *path = path.strip_prefix("node.a.").unwrap().to_string();
    }
    let (mut router, errors) = SignalRouter::bind(&[model.clone()], &bus);
    assert!(errors.is_empty(), "{errors:?}");
    let signals = |currents: [f64; 3]| -> CosimSignals {
        ["a", "b", "c"]
            .into_iter()
            .zip(currents)
            .map(|(phase, current)| {
                (
                    format!("board.motor.M1.phase_{phase}_current_a"),
                    CosimSignalValue::F64(current),
                )
            })
            .collect()
    };
    assert!(router
        .apply(&mut bus, &signals([0.0, -2.0, 2.0]), 0)
        .is_empty());
    assert!(router
        .apply(&mut bus, &signals([0.0, -2.0, 2.0]), 1000)
        .is_empty());
    let last = bus.motor_snapshots();
    for time in [1000, 3000] {
        assert!(!router
            .apply(&mut bus, &signals([0.0, -2.0, 2.0]), time)
            .is_empty());
        assert_eq!(bus.motor_snapshots(), last);
    }
    let mut partial = signals([0.0, -2.0, 2.0]);
    partial.remove("board.motor.M1.phase_c_current_a");
    assert!(
        !router.apply(&mut bus, &partial, 2000).is_empty(),
        "a rejected earlier vector must not supply the missing phase"
    );
    assert_eq!(bus.motor_snapshots(), last);
    assert!(!router
        .apply(&mut bus, &signals([0.0, -2.0, 3.0]), 2000)
        .is_empty());
    assert_eq!(bus.motor_snapshots(), last);
    let mut duplicate = model.clone();
    duplicate.id = "second-island".into();
    let (_, errors) = SignalRouter::bind(&[model.clone(), duplicate], &bus);
    assert!(errors
        .iter()
        .any(|error| error.to_string().contains("multiple co-simulation models")));
    let mut slower = model.clone();
    slower.id = "slower-independent-model".into();
    slower.step_ns = 2000;
    slower.inputs.clear();
    slower.outputs.clear();
    let (_, errors) = SignalRouter::bind(&[model.clone(), slower], &bus);
    assert!(errors
        .iter()
        .any(|error| error.to_string().contains("same step_ns")));
    let mut alias = model.clone();
    alias
        .outputs
        .insert("alias".into(), "board.motor.M1.phase_a_current_a".into());
    let (_, errors) = SignalRouter::bind(&[alias], &bus);
    assert!(errors
        .iter()
        .any(|error| error.to_string().contains("exactly once")));
    let mut dc = system();
    dc.external_devices[0].r#type = "dc-motor".into();
    dc.external_devices[0].config.remove("pole_pairs");
    dc.external_devices[0].config.remove("current_limit_a");
    let dc_bus = SystemBus::from_config(&chip, &dc).unwrap();
    let (_, errors) = SignalRouter::bind(&[model], &dc_bus);
    assert!(errors
        .iter()
        .any(|error| error.to_string().contains("required circuit winding kind")));
}
