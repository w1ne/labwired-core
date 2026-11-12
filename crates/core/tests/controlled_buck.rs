mod controlled_buck_support;
use controlled_buck_support::*;
use labwired_core::analog::{
    controlled_buck::{parse_command, strict_revision},
    AnalogCosimAdapter,
};
use labwired_core::cosim::{CosimAdapter, CosimSignalValue, CosimSignals, CosimStep};
use labwired_core::regulator::RegulatorCommand;
fn step(
    adapter: &mut AnalogCosimAdapter,
    time: u64,
    inputs: CosimSignals,
) -> labwired_core::SimResult<labwired_core::cosim::CosimStepResult> {
    adapter.step(CosimStep {
        time_ns: time,
        dt_ns: 100,
        inputs,
    })
}
#[test]
fn first_operating_point_requires_both_actual_supplies() {
    for (vin, avdd, enabled) in [
        (3.3, 3.3, true),
        (0.0, 3.3, false),
        (3.3, 0.0, false),
        (2.6, 3.3, false),
        (3.3, 3.7, false),
    ] {
        let mut adapter =
            AnalogCosimAdapter::from_netlist(&deck(vin, avdd, 11.0), &config()).unwrap();
        let values = step(&mut adapter, 0, CosimSignals::new()).unwrap().outputs;
        assert_eq!(values["reg.qualified"], CosimSignalValue::Bool(enabled));
        assert_eq!(
            values["reg.output_enabled"],
            CosimSignalValue::Bool(enabled)
        );
        let fb = number(&values, "reg.feedback_voltage_v");
        if enabled {
            assert!((fb - 1.1).abs() < 0.002, "{fb}");
        } else {
            assert!(fb.abs() < 1e-10, "{fb}");
            assert_eq!(number(&values, "reg.input_current_a"), 0.0);
        }
        assert_eq!(adapter.time_ns(), 0);
    }
}
#[test]
fn solved_port_power_matches_actual_input_current_and_explicit_efficiency() {
    let mut adapter = AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &config()).unwrap();
    let values = step(&mut adapter, 0, CosimSignals::new()).unwrap().outputs;
    let solver = adapter.solver();
    let input = -solver.branch_current(solver.circuit().branch_index("Vin").unwrap());
    assert!((input - number(&values, "reg.input_current_a")).abs() < 1e-9);
    let out = solver.node_voltage(solver.circuit().node("sw").unwrap());
    let current = number(&values, "reg.output_current_a");
    let iq = 0.001 / 1.1 * number(&values, "reg.feedback_voltage_v");
    assert!((3.3 * (input - iq) - out * current / 0.8).abs() < 1e-9);
    assert!(current > 0.099 && current < 0.101);
}
#[test]
fn disabled_initial_stage_is_high_impedance_under_external_backdrive() {
    let mut cfg = config();
    cfg.regulators.get_mut("reg").unwrap().initial.enabled = false;
    let netlist = deck(3.3, 3.3, 11.0).replace(
        ".end",
        "Vback out 0 1.2
.end",
    );
    let mut adapter = AnalogCosimAdapter::from_netlist(&netlist, &cfg).unwrap();
    let values = step(&mut adapter, 0, CosimSignals::new()).unwrap().outputs;
    assert!((number(&values, "feedback") - 1.2).abs() < 1e-10);
    assert_eq!(number(&values, "reg.output_current_a"), 0.0);
    assert_eq!(number(&values, "reg.input_current_a"), 0.0);
    assert_eq!(values["reg.qualified"], CosimSignalValue::Bool(true));
    assert_eq!(values["reg.output_enabled"], CosimSignalValue::Bool(false));
}
#[test]
fn actual_avdd_cutoff_and_hiz_remove_paths_without_reinitializing_capacitor() {
    for use_hiz in [true, false] {
        let mut adapter =
            AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &config()).unwrap();
        let before = adapter.solver().capacitor_voltage(0);
        let inputs = if use_hiz {
            let mut command = RegulatorCommand {
                configuration: descriptor().initial,
                command_revision: 1,
            };
            command.configuration.enabled = false;
            frame(command)
        } else {
            CosimSignals::from([("avdd".into(), CosimSignalValue::F64(0.0))])
        };
        let values = step(&mut adapter, 100, inputs).unwrap().outputs;
        assert_eq!(values["reg.output_enabled"], CosimSignalValue::Bool(false));
        assert_eq!(number(&values, "reg.input_current_a"), 0.0);
        let after = adapter.solver().capacitor_voltage(0);
        assert!(
            after > before * 0.99 && after < before,
            "{before} -> {after}"
        );
    }
}
#[test]
fn current_limit_is_distinct_from_rated_load_and_target_can_exceed_input() {
    let mut cfg = config();
    cfg.regulators.get_mut("reg").unwrap().initial.rated_load_a = 0.05;
    let mut adapter = AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 1.0), &cfg).unwrap();
    let values = step(&mut adapter, 0, CosimSignals::new()).unwrap().outputs;
    let current = number(&values, "reg.output_current_a");
    assert!(current > 0.23 && current < 0.25, "{current}");
    let mut cfg = config();
    cfg.regulators
        .get_mut("reg")
        .unwrap()
        .initial
        .target_voltage_v = 5.0;
    let netlist = deck(3.3, 3.3, 100.0).replace("VREF=1.1", "VREF=5");
    let mut adapter = AnalogCosimAdapter::from_netlist(&netlist, &cfg).unwrap();
    let values = step(&mut adapter, 0, CosimSignals::new()).unwrap().outputs;
    assert!(number(&values, "feedback") < 3.4);
}
#[test]
fn complete_frame_revision_is_applied_only_by_a_new_solved_interval() {
    let mut adapter = AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &config()).unwrap();
    let mut command = RegulatorCommand {
        configuration: descriptor().initial,
        command_revision: 1,
    };
    command.configuration.target_voltage_v = 1.15;
    assert!(step(&mut adapter, 0, frame(command)).is_err());
    assert_eq!(adapter.solver().buck_command(0).command_revision, 0);
    let values = step(&mut adapter, 100, frame(command)).unwrap().outputs;
    assert_eq!(number(&values, "reg.applied_revision"), 1.0);
    assert!(number(&values, "feedback").is_finite());
    command.configuration.target_voltage_v = 1.2;
    assert!(step(&mut adapter, 200, frame(command)).is_err());
    command.command_revision = 0;
    assert!(step(&mut adapter, 200, frame(command)).is_err());
}
#[test]
fn partial_wrong_type_and_nonfinite_commands_fail_before_mutation() {
    let mut adapter = AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &config()).unwrap();
    let original = adapter.solver().buck_command(0);
    let mut command = original;
    command.command_revision = 1;
    command.configuration.enabled = false;
    for (key, value) in [
        ("reg.enabled", CosimSignalValue::F64(0.0)),
        ("reg.current_limit_a", CosimSignalValue::F64(f64::NAN)),
        ("reg.command_revision", CosimSignalValue::F64(0.5)),
        ("reg.target_voltage_v", CosimSignalValue::Text("1.2".into())),
    ] {
        let mut inputs = frame(command);
        inputs.insert(key.into(), value);
        assert!(step(&mut adapter, 100, inputs).is_err());
        assert_eq!(adapter.solver().buck_command(0), original);
    }
    let mut inputs = frame(command);
    inputs.remove("reg.rated_load_a");
    assert!(step(&mut adapter, 100, inputs).is_err());
    assert_eq!(adapter.solver().buck_command(0), original);
}
#[test]
fn safe_integer_revision_boundary_is_closed() {
    for value in [
        CosimSignalValue::F64(f64::INFINITY),
        CosimSignalValue::F64(-1.0),
        CosimSignalValue::I64(9_007_199_254_740_992),
        CosimSignalValue::Text("1".into()),
        CosimSignalValue::Bool(true),
    ] {
        assert!(strict_revision(&value).is_err());
    }
    assert_eq!(
        strict_revision(&CosimSignalValue::I64(9_007_199_254_740_991)).unwrap(),
        9_007_199_254_740_991
    );
    assert!(parse_command("reg", &CosimSignals::new())
        .unwrap()
        .is_none());
}
#[test]
fn duplicate_instances_primitive_owners_and_reserved_ports_are_rejected() {
    let mut cfg = config();
    cfg.regulators.insert("second".into(), stage());
    assert!(AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &cfg).is_err());
    let mut cfg = config();
    cfg.probes.insert("reg.qualified".into(), "v(out)".into());
    assert!(AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &cfg).is_err());
    let mut cfg = config();
    cfg.sources.insert("bypass".into(), "Xbuck#iref".into());
    assert!(AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &cfg).is_err());
}
#[test]
fn control_metadata_cannot_disagree_with_the_netlist_initial_state() {
    let mut cfg = config();
    cfg.regulators
        .get_mut("reg")
        .unwrap()
        .initial
        .current_limit_a = 0.5;
    assert!(AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &cfg).is_err());
    let mut cfg = config();
    cfg.regulators.get_mut("reg").unwrap().enable_node = "missing".into();
    assert!(AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &cfg).is_err());
    let mut cfg = config();
    cfg.regulators
        .get_mut("reg")
        .unwrap()
        .initial
        .command_revision = 1;
    assert!(AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &cfg).is_err());
}

#[test]
fn all_generated_primitives_and_thermistors_are_reserved() {
    for primitive in ["Xbuck#vh", "Xbuck#ra", "Xbuck#rh"] {
        let mut cfg = config();
        cfg.sources.insert("bypass".into(), primitive.into());
        assert!(
            AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &cfg)
                .unwrap_err()
                .to_string()
                .contains("one stage owner")
        );
    }
    let mut cfg = config();
    let model = serde_yaml::from_str(
        "resistor: Xbuck#ro
r_ref_ohms: 0.01
beta_kelvin: 3950
reference_c: 25
min_c: -40
max_c: 125
",
    );
    // Construct from the documented struct after confirming its exact field names.
    if let Ok(model) = model {
        cfg.thermistors.insert("temperature".into(), model);
        assert!(
            AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &cfg)
                .unwrap_err()
                .to_string()
                .contains("thermistor")
        );
    } else {
        panic!("thermistor fixture schema is wrong");
    }
}
#[test]
fn active_backdrive_and_downward_target_refuse_unsupported_reverse_current() {
    let backdrive = deck(3.3, 3.3, 11.0).replace(".end", "Vback out 0 1.2\n.end");
    let error = AnalogCosimAdapter::from_netlist(&backdrive, &config())
        .unwrap_err()
        .to_string();
    assert!(error.contains("reverse current"), "{error}");
    let mut adapter = AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &config()).unwrap();
    let mut command = RegulatorCommand {
        configuration: descriptor().initial,
        command_revision: 1,
    };
    command.configuration.target_voltage_v = 0.5;
    let error = step(&mut adapter, 100, frame(command))
        .unwrap_err()
        .to_string();
    assert!(error.contains("reverse current"), "{error}");
}
#[test]
fn rating_trace_uses_applied_frame_and_absolute_current_without_clamping() {
    let mut cfg = config();
    cfg.regulators.get_mut("reg").unwrap().initial.rated_load_a = 0.05;
    let adapter = AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 1.0), &cfg).unwrap();
    let channels = adapter.channels();
    assert_eq!(
        channels
            .iter()
            .find(|channel| channel.name == "reg.rated_load_a")
            .unwrap()
            .unit,
        "A"
    );
    let observed = adapter.solver().buck_observation(0, 0);
    let command = adapter.solver().buck_command(0);
    let trace = labwired_core::analog::controlled_buck::trace_outputs(observed, command);
    assert_eq!(trace[7], CosimSignalValue::F64(0.05));
    assert_eq!(trace[8], CosimSignalValue::Bool(false));
    assert!(observed.output_current_a > 0.23);
    assert!(observed.qualified);
    let mut reverse = observed;
    reverse.output_current_a = -0.1;
    assert_eq!(
        labwired_core::analog::controlled_buck::trace_outputs(reverse, command)[8],
        CosimSignalValue::Bool(false)
    );
    let mut cfg = config();
    cfg.probes
        .insert("reg.load_within_rating".into(), "v(out)".into());
    assert!(AnalogCosimAdapter::from_netlist(&deck(3.3, 3.3, 11.0), &cfg).is_err());
}

#[test]
fn negative_switch_voltage_with_positive_current_refuses_reverse_power() {
    // With FB just above the target, the unconstrained amplifier output is negative.
    // A more-negative forced SW draws positive current but sends net energy into the stage.
    let netlist =
        deck(3.3, 3.3, 11.0).replace(".end", "Vswitch sw 0 -0.1\nVfeedback out 0 1.1000005\n.end");
    let error = AnalogCosimAdapter::from_netlist(&netlist, &config())
        .unwrap_err()
        .to_string();
    assert!(error.contains("reverse power"), "{error}");
    assert!(!error.contains("reverse current"), "{error}");
}

fn direct_solver(netlist: &str) -> labwired_core::analog::Solver {
    use labwired_core::analog::{controlled_buck::bind_stages, parse_netlist, Integration, Solver};
    let mut circuit = parse_netlist(netlist).unwrap();
    bind_stages(&mut circuit, &config().regulators).unwrap();
    Solver::new(circuit, Integration::BackwardEuler).unwrap()
}
#[test]
fn direct_solver_transient_cannot_bypass_reverse_current_scope() {
    let mut solver = direct_solver(&deck(3.3, 3.3, 11.0));
    let mut command = RegulatorCommand {
        configuration: descriptor().initial,
        command_revision: 1,
    };
    command.configuration.target_voltage_v = 0.5;
    solver.set_buck_command(0, command).unwrap();
    let error = solver.advance(100e-9).unwrap_err().to_string();
    assert!(error.contains("reverse current"), "{error}");
    assert_eq!(solver.step_index(), 0, "unsupported step is not accepted");
}
#[test]
fn direct_solver_operating_point_cannot_bypass_reverse_current_scope() {
    let netlist = deck(3.3, 3.3, 11.0).replace(".end", "Vback out 0 1.09998\n.end");
    let mut solver = direct_solver(&netlist);
    let source = solver.voltage_source_index("Vback").unwrap();
    solver.set_voltage_source(source, 1.2);
    let error = solver.solve_operating_point().unwrap_err().to_string();
    assert!(error.contains("reverse current"), "{error}");
}
