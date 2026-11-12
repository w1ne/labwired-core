mod internal_regulator_support;

use internal_regulator_support::{bus, machine, model};
use labwired_config::{CosimAdapter, CosimModelConfig};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::CosimSession;
use labwired_core::AdvanceRequest;
use std::collections::HashMap;
use std::path::Path;

fn rejection(models: &[CosimModelConfig]) -> String {
    let (bus, control, _) = bus();
    let before = control.configuration();
    let message = match CosimSession::new(models, Path::new("."), &bus) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("malformed regulator ownership was admitted"),
    };
    assert_eq!(control.configuration(), before);
    assert!(control.observation().is_none());
    message
}

fn unrelated_model() -> CosimModelConfig {
    CosimModelConfig {
        id: "unrelated".into(),
        adapter: CosimAdapter::Mock,
        model: None,
        step_ns: 500,
        inputs: HashMap::new(),
        outputs: HashMap::new(),
        config: HashMap::from([(
            "outputs".into(),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        )]),
    }
}

#[test]
fn declared_stage_rejects_absent_models_before_any_observation() {
    let message = rejection(&[]);
    assert!(message.contains("regulator"), "{message}");
    assert!(
        message.contains("model") || message.contains("circuit"),
        "{message}"
    );
}

#[test]
fn legacy_bus_without_regulators_keeps_no_cosim_fast_path() {
    let bus = SystemBus::new();
    assert!(CosimSession::new(&[], Path::new("."), &bus)
        .expect("a legacy bus needs no invented analog model")
        .is_none());
}

#[test]
fn unrelated_nonempty_model_does_not_satisfy_declared_stage() {
    let message = rejection(&[unrelated_model()]);
    assert!(message.contains("complete analog model owner"), "{message}");
}

#[test]
fn every_command_and_observation_field_is_required() {
    for input in [true, false] {
        let mut circuit = model(&[("a", 3.3)], false);
        let field = if input {
            "current_limit_a"
        } else {
            "feedback_voltage_v"
        };
        let routes = if input {
            &mut circuit.inputs
        } else {
            &mut circuit.outputs
        };
        assert!(routes.remove(&format!("reg_0.{field}")).is_some());
        let message = rejection(&[circuit]);
        assert!(message.contains(field), "{field}: {message}");
    }
}

#[test]
fn duplicate_field_and_duplicate_model_owners_are_rejected() {
    let mut duplicate_field = model(&[("a", 3.3)], false);
    duplicate_field.inputs.insert(
        "other.current_limit_a".into(),
        "board.regulator.converter.current_limit_a".into(),
    );
    let message = rejection(&[duplicate_field]);
    assert!(message.contains("current_limit_a"), "{message}");

    let first = model(&[("a", 3.3)], false);
    let mut second = first.clone();
    second.id = "other_owner".into();
    let message = rejection(&[first, second]);
    assert!(
        message.contains("exactly one complete analog model owner"),
        "{message}"
    );
}

#[test]
fn descriptor_model_parameters_cannot_be_changed_by_the_inline_card() {
    let mut circuit = model(&[("a", 3.3)], false);
    let text = circuit.config["netlist_text"].as_str().unwrap();
    assert!(text.contains("EFF=0.8"));
    circuit.config.insert(
        "netlist_text".into(),
        text.replace("EFF=0.8", "EFF=0.9").into(),
    );
    let message = rejection(&[circuit]);
    assert!(
        message.contains("descriptor") && message.contains("assumptions"),
        "{message}"
    );
}

#[test]
fn physical_supply_probe_must_observe_the_owned_input_node() {
    let mut circuit = model(&[("a", 3.3)], false);
    circuit
        .config
        .get_mut("probes")
        .unwrap()
        .as_mapping_mut()
        .unwrap()
        .insert("a_input".into(), "v(out_a)".into());
    let message = rejection(&[circuit]);
    assert!(message.contains("different physical node"), "{message}");
}

#[test]
fn enable_qualification_cannot_be_rebound_to_a_different_physical_node() {
    let mut circuit = model(&[("a", 3.3)], false);
    circuit.config.get_mut("regulators").unwrap()["reg_0"]["enable_node"] = "in".into();
    let message = rejection(&[circuit]);
    assert!(message.contains("different physical node"), "{message}");
}

#[test]
fn initial_configuration_must_match_the_declared_stage() {
    let mut circuit = model(&[("a", 3.3)], false);
    circuit.config.get_mut("regulators").unwrap()["reg_0"]["initial"]["rated_load_a"] = 0.1.into();
    let message = rejection(&[circuit]);
    assert!(
        message.contains("initial configuration differs"),
        "{message}"
    );
}

#[test]
fn generated_primitives_cannot_acquire_an_ordinary_source_owner() {
    for primitive in [
        "X_a#vh", "X_a#ra", "X_a#rh", "X_a#ro", " x_A#vh ", " X_a#ra ",
    ] {
        let mut circuit = model(&[("a", 3.3)], false);
        circuit.inputs.insert("bypass".into(), "ui.bypass".into());
        circuit
            .config
            .get_mut("sources")
            .unwrap()
            .as_mapping_mut()
            .unwrap()
            .insert("bypass".into(), primitive.into());
        let message = rejection(&[circuit]);
        assert!(
            message.contains("controlled BUCK"),
            "{primitive}: {message}"
        );
    }
}

#[test]
fn generated_output_resistance_cannot_acquire_a_thermistor_owner() {
    for primitive in ["X_a#ro", " x_A#ro "] {
        let mut circuit = model(&[("a", 3.3)], false);
        circuit
            .inputs
            .insert("temperature".into(), "ui.temperature".into());
        circuit.config.insert("thermistors".into(), serde_yaml::from_str(&format!(
            "temperature: {{resistor: '{primitive}', r_ref_ohms: 0.01, beta_kelvin: 3950, reference_c: 25, min_c: 0, max_c: 85}}"
        )).unwrap());
        let message = rejection(&[circuit]);
        assert!(
            message.contains("controlled BUCK"),
            "{primitive}: {message}"
        );
    }
}

// Direct session boundaries make the temporal assertion exact: this fixture's
// 16 MHz time base has eight cycles per half-period and sixteen per full period.
// No CPU instruction timing or controller register encoding is substituted.
fn halfway_command_is_held_until_the_next_owner_interval(mixed_rate: bool) {
    let (mut bus, control, observed) = bus();
    let mut models = vec![model(&[("a", 3.3)], false)];
    if mixed_rate {
        models.push(unrelated_model());
    }
    let mut session = CosimSession::new(&models, Path::new("."), &bus)
        .unwrap()
        .unwrap();
    session.set_signal_number("ui.a.enable", 3.3).unwrap();
    let (_, errors) = session.advance_to(8, &mut bus).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(control.observation().is_none());
    let mut changed = control.configuration();
    changed.current_limit_a = 0.22;
    control.request(changed).unwrap();
    assert!(control.pending_update());

    let (_, errors) = session.advance_to(16, &mut bus).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let first = control.observation().unwrap();
    assert_eq!((first.time_ns, first.applied_revision), (1000, 0));
    assert!(first.output_enabled && first.qualified);
    assert!(control.pending_update());
    if mixed_rate {
        let (_, errors) = session.advance_to(24, &mut bus).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            control.observation(),
            Some(first),
            "a faster peer must not refresh a held observation"
        );
    }
    let (_, errors) = session.advance_to(32, &mut bus).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let second = control.observation().unwrap();
    assert_eq!((second.time_ns, second.applied_revision), (2000, 1));
    assert!(second.output_enabled && second.qualified);
    assert!(!control.pending_update());
    let callbacks = observed.lock().unwrap();
    assert_eq!(
        callbacks
            .iter()
            .map(|o| (o.time_ns, o.applied_revision))
            .collect::<Vec<_>>(),
        vec![(1000, 0), (2000, 1)]
    );
}

#[test]
fn a_halfway_standalone_command_cannot_acknowledge_the_elapsed_interval() {
    halfway_command_is_held_until_the_next_owner_interval(false);
}

#[test]
fn a_faster_unrelated_model_cannot_capture_or_acknowledge_a_regulator_command() {
    halfway_command_is_held_until_the_next_owner_interval(true);
}

#[test]
fn machine_advance_preserves_the_same_interval_start_revision_contract() {
    let (mut machine, control, observed) = machine();
    let configs = [model(&[("a", 3.3)], false)];
    let mut session = CosimSession::new(&configs, Path::new("."), &machine.bus)
        .unwrap()
        .unwrap();
    session.set_signal_number("ui.a.enable", 3.3).unwrap();
    // Let the declared 3000 ns POR release complete. The warm-up is bounded;
    // assertions below concern only the later, fully observed operating state.
    session
        .advance_budget(&mut machine, AdvanceRequest::run(None).with_cycle_limit(80))
        .unwrap();
    let before = control
        .observation()
        .expect("warm-up must solve the physical rails");
    assert!(before.time_ns >= 4000 && before.output_enabled);
    assert!(!control.pending_update());
    let start_cycles = machine.total_cycles;
    session
        .advance_budget(&mut machine, AdvanceRequest::run(None).with_cycle_limit(4))
        .unwrap();
    assert!(machine.total_cycles > start_cycles);
    assert_eq!(
        control.observation(),
        Some(before),
        "the short CPU slice must stay within one owner interval"
    );
    let mut changed = control.configuration();
    changed.current_limit_a = 0.22;
    control.request(changed).unwrap();
    let expected_revision = before.applied_revision + 1;
    for _ in 0..64 {
        session
            .advance(&mut machine, AdvanceRequest::single())
            .unwrap();
        if control.observation().unwrap().time_ns > before.time_ns {
            break;
        }
    }
    let first = control.observation().unwrap();
    assert_eq!(first.time_ns, before.time_ns + 1000);
    assert_eq!(first.applied_revision, before.applied_revision);
    assert!(control.pending_update());
    for _ in 0..64 {
        session
            .advance(&mut machine, AdvanceRequest::single())
            .unwrap();
        if control.observation().unwrap().time_ns > first.time_ns {
            break;
        }
    }
    let second = control.observation().unwrap();
    assert_eq!(second.time_ns, first.time_ns + 1000);
    assert_eq!(second.applied_revision, expected_revision);
    assert!(!control.pending_update());
    assert!(observed
        .lock()
        .unwrap()
        .iter()
        .any(|o| o.time_ns == second.time_ns && o.applied_revision == expected_revision));
}

#[test]
fn machine_advance_requires_sampled_regulator_ownership() {
    let (mut machine, control, observed) = machine();
    let initial_cycles = machine.total_cycles;
    let initial_configuration = control.configuration();
    for request in [
        AdvanceRequest::single(),
        AdvanceRequest::run(None).with_cycle_limit(4),
    ] {
        match machine.advance(request) {
            Err(labwired_core::SimulationError::Other(message)) => assert_eq!(
                message,
                "declared internal regulators have no complete attached sampled circuit; CPU execution refused"
            ),
            Err(other) => panic!("unexpected admission error: {other}"),
            Ok(_) => panic!("an unsampled regulator admitted CPU advance"),
        }
        assert_eq!(machine.total_cycles, initial_cycles);
        assert_eq!(control.configuration(), initial_configuration);
        assert!(control.observation().is_none());
        assert!(observed.lock().unwrap().is_empty());
    }

    // Admission is read again after a complete owner is attached to this same
    // machine; a prior refusal must not cache an unsampled state.
    let configs = [model(&[("a", 3.3)], false)];
    let mut session = CosimSession::new(&configs, Path::new("."), &machine.bus)
        .unwrap()
        .unwrap();
    session.set_signal_number("ui.a.enable", 3.3).unwrap();
    session
        .advance_budget(&mut machine, AdvanceRequest::run(None).with_cycle_limit(80))
        .unwrap();
    assert!(machine.total_cycles > initial_cycles);
    assert!(control.observation().is_some());
    assert!(!observed.lock().unwrap().is_empty());
}
