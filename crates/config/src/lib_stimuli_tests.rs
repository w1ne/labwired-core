use super::*;

fn script(schema: &str, stimuli_block: &str) -> String {
    format!(
        r#"
schema_version: "{schema}"
inputs:
  firmware: "cow.elf"
  system: "sys.yaml"
limits:
  max_steps: 1000
{stimuli_block}
"#
    )
}

#[test]
fn stimuli_parse_and_validate_on_1_2() {
    let yaml = script(
        "1.2",
        r#"stimuli:
  - target: { component: fxos8700, channel: x }
    trigger: !after_cycles { cycles: 800000 }
    value: 2.0
  - target: { channel: z }
    value: 1.0
"#,
    );
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    s.validate().unwrap();
    assert_eq!(s.stimuli.len(), 2);
    let target = s.stimuli[0].input_target().expect("an input stimulus");
    assert_eq!(target.channel, "x");
    assert_eq!(target.component.as_deref(), Some("fxos8700"));
    assert_eq!(s.stimuli[0].value(), 2.0);
    // Default trigger is at_start.
    assert!(matches!(s.stimuli[1].trigger, FaultTrigger::AtStart));
}

#[test]
fn stimuli_require_schema_1_2() {
    for schema in ["1.0", "1.1"] {
        let yaml = script(
            schema,
            "stimuli:\n  - target: { channel: x }\n    value: 1.0\n",
        );
        let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
        let err = s.validate().unwrap_err().to_string();
        assert!(err.contains("require schema_version '1.2'"), "{err}");
    }
}

#[test]
fn empty_channel_rejected() {
    let yaml = script(
        "1.2",
        "stimuli:\n  - target: { channel: \"\" }\n    value: 1.0\n",
    );
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    assert!(s
        .validate()
        .unwrap_err()
        .to_string()
        .contains("channel cannot be empty"));
}

/// `cosim_signal: { path, value }` is a stimulus like any other: same list,
/// same trigger forms, same schema gate.
#[test]
fn cosim_signal_stimuli_parse_with_the_existing_trigger_forms() {
    let yaml = script(
        "1.2",
        r#"stimuli:
  - cosim_signal: { path: ui.touch.pressed, value: 1 }
    trigger: !after_cycles { cycles: 8000000 }
  - cosim_signal: { path: ui.knob.volts, value: 2.5 }
  - target: { channel: x }
    value: 1.0
"#,
    );
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    s.validate().unwrap();
    assert_eq!(
        s.stimuli[0].action,
        StimulusAction::CosimSignal(CosimSignalStimulus {
            path: "ui.touch.pressed".to_string(),
            value: 1.0,
        })
    );
    assert_eq!(
        s.stimuli[0].trigger,
        FaultTrigger::AfterCycles { cycles: 8_000_000 }
    );
    assert_eq!(s.stimuli[1].trigger, FaultTrigger::AtStart);
    assert_eq!(s.stimuli[1].value(), 2.5);
    assert!(s.stimuli[1].input_target().is_none());
    assert!(s.stimuli[2].input_target().is_some());

    // Both shapes serialize back to the keys they were written with.
    let round_trip: Vec<StimulusSpec> =
        serde_yaml::from_str(&serde_yaml::to_string(&s.stimuli).unwrap()).unwrap();
    assert_eq!(round_trip, s.stimuli);
    let text = serde_yaml::to_string(&s.stimuli[2]).unwrap();
    assert!(!text.contains("cosim_signal"), "{text}");
}

#[test]
fn a_stimulus_is_exactly_one_shape() {
    for (block, expected) in [
        (
            "stimuli:\n  - cosim_signal: { path: ui.a.b, value: 1 }\n    value: 1.0\n",
            "cannot also set",
        ),
        (
            "stimuli:\n  - cosim_signal: { path: ui.a.b, value: 1 }\n    target: { channel: x }\n",
            "cannot also set",
        ),
        ("stimuli:\n  - trigger: at_start\n", "a stimulus needs"),
        (
            "stimuli:\n  - target: { channel: x }\n",
            "missing field `value`",
        ),
        ("stimuli:\n  - cosim_signal: { path: ui.a.b }\n", "value"),
    ] {
        let err = serde_yaml::from_str::<TestScript>(&script("1.2", block))
            .expect_err(block)
            .to_string();
        assert!(err.contains(expected), "{block}: {err}");
    }
}

#[test]
fn cosim_signal_needs_a_path_and_a_finite_value() {
    let empty = script(
        "1.2",
        "stimuli:\n  - cosim_signal: { path: \"\", value: 1 }\n",
    );
    let s: TestScript = serde_yaml::from_str(&empty).unwrap();
    let err = s.validate().unwrap_err().to_string();
    assert!(err.contains("cosim_signal.path cannot be empty"), "{err}");

    let infinite = script(
        "1.2",
        "stimuli:\n  - cosim_signal: { path: ui.a.b, value: .inf }\n",
    );
    let s: TestScript = serde_yaml::from_str(&infinite).unwrap();
    let err = s.validate().unwrap_err().to_string();
    assert!(err.contains("finite"), "{err}");

    let on_write = script(
            "1.2",
            "stimuli:\n  - cosim_signal: { path: ui.a.b, value: 1 }\n    trigger: !on_write { register: \"FOO\" }\n",
        );
    let s: TestScript = serde_yaml::from_str(&on_write).unwrap();
    let err = s.validate().unwrap_err().to_string();
    assert!(err.contains("not yet supported for stimuli"), "{err}");
}

#[test]
fn register_triggers_rejected_for_stimuli() {
    let yaml = script(
        "1.2",
        r#"stimuli:
  - target: { channel: x }
    trigger: !on_write { register: "FOO" }
    value: 1.0
"#,
    );
    let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
    assert!(s
        .validate()
        .unwrap_err()
        .to_string()
        .contains("not yet supported for stimuli"));
}
