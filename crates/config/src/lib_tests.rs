use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn test_valid_script() {
    let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: "path/to/fw.elf"
  system: "path/to/sys.yaml"
limits:
  max_steps: 1000
  wall_time_ms: 5000
assertions:
  - uart_contains: "Hello"
  - expected_stop_reason: halt
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    assert!(script.validate().is_ok());
    assert_eq!(script.inputs.firmware, "path/to/fw.elf");
    assert_eq!(script.limits.max_steps, 1000);
    assert_eq!(script.assertions.len(), 2);
    // stack_paint defaults to true when omitted
    assert!(script.stack_paint);
}

#[test]
fn parses_resource_budget_and_stack_paint() {
    let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: "path/to/fw.elf"
  system: "path/to/sys.yaml"
limits:
  max_steps: 1000
stack_paint: false
assertions:
  - resource_budget:
      max_main_stack_bytes: 512
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    script.validate().unwrap();
    assert!(!script.stack_paint);
    assert_eq!(script.assertions.len(), 1);
    let TestAssertion::ResourceBudget(a) = &script.assertions[0] else {
        panic!(
            "expected resource_budget assertion, got {:?}",
            script.assertions[0]
        );
    };
    assert_eq!(a.resource_budget.max_main_stack_bytes, Some(512));
    assert!(a.resource_budget.max_flash_bytes.is_none());
    assert!(a.resource_budget.max_ram_static_bytes.is_none());
}

#[test]
fn motor_showcase_assertions_parse_with_typed_payloads() {
    let yaml = r#"
schema_version: "1.2"
inputs: { firmware: "motor.elf" }
limits: { max_steps: 1000 }
stimuli:
  - target: { component: drive, channel: stall }
    trigger: !after_cycles { cycles: 500 }
    value: 1.0
assertions:
  - uart_ordered: ["READY", "TARGET", "FAULT", "OFF"]
  - motor_speed_reached: { id: drive, min_abs_rpm: 100.0, max_abs_rpm: 4000.0 }
  - motor_state: { id: drive, control_state: "off:external-enable", fault_contains: stalled }
  - shutdown_latency:
      from_stimulus: { component: drive, channel: stall }
      to_uart: "OFF"
      max_cycles: 300000
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    script.validate().unwrap();
    assert!(matches!(
        &script.assertions[0],
        TestAssertion::UartOrdered(a) if a.uart_ordered.len() == 4
    ));
    assert!(matches!(
        &script.assertions[1],
        TestAssertion::MotorSpeedReached(a)
            if a.motor_speed_reached.min_abs_rpm == 100.0
    ));
    assert!(matches!(
        &script.assertions[2],
        TestAssertion::MotorState(a)
            if a.motor_state.control_state == "off:external-enable"
    ));
    assert!(matches!(
        &script.assertions[3],
        TestAssertion::ShutdownLatency(a)
            if a.shutdown_latency.max_cycles == 300_000
                && a.shutdown_latency.stimulus_occurrence == 1
                && a.shutdown_latency.uart_occurrence == 1
    ));
}

#[test]
fn test_fault_injection_script_roundtrips() {
    let yaml = r#"
schema_version: "1.1"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 1000
faults:
  - id: usart1_no_clock
    kind: missing_clock
    target: { peripheral: usart1 }
  - id: sr_stuck
    kind: stuck_at_bit
    target: { peripheral: usart1, register: sr, bit: 7 }
    level: 1
    trigger: at_start
verdict:
  safe_when:
    - uart_contains: "FAULT_HANDLED"
  require_fault_fired: true
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    script.validate().expect("valid 1.1 fault script");
    assert_eq!(script.faults.len(), 2);
    assert_eq!(script.faults[0].kind, FaultKind::MissingClock);
    assert!(script.verdict.as_ref().unwrap().require_fault_fired);
}

/// A fault trigger the runner does not evaluate is refused, not silently
/// applied at start.
#[test]
fn test_fault_triggers_other_than_at_start_are_refused() {
    for trigger in [
        "!after_cycles { cycles: 1000 }",
        "!on_write { register: \"CR1\" }",
        "!on_read { register: \"SR\" }",
    ] {
        let yaml = format!(
            r#"
schema_version: "1.1"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
faults:
  - id: late_clock
    kind: missing_clock
    target: {{ peripheral: usart1 }}
    trigger: {trigger}
"#
        );
        let script: TestScript = serde_yaml::from_str(&yaml).unwrap();
        let err = script.validate().unwrap_err().to_string();
        assert!(err.contains("late_clock"), "{trigger}: {err}");
        assert!(
            err.contains("not yet supported for faults"),
            "{trigger}: {err}"
        );
    }
}

#[test]
fn test_faults_require_v1_1() {
    let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
faults:
  - id: x
    kind: missing_clock
    target: { peripheral: usart1 }
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    let err = script.validate().unwrap_err();
    assert!(err.to_string().contains("require schema_version '1.1'"));
}

#[test]
fn test_fault_missing_required_param_rejected() {
    // stuck_at_bit without a level.
    let yaml = r#"
schema_version: "1.1"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
faults:
  - id: bad
    kind: stuck_at_bit
    target: { peripheral: usart1, register: sr, bit: 7 }
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    let err = script.validate().unwrap_err();
    assert!(err.to_string().contains("level"));
}

#[test]
fn test_duplicate_fault_id_rejected() {
    let yaml = r#"
schema_version: "1.1"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
faults:
  - id: dup
    kind: missing_clock
    target: { peripheral: a }
  - id: dup
    kind: missing_clock
    target: { peripheral: b }
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    let err = script.validate().unwrap_err();
    assert!(err.to_string().contains("Duplicate fault id"));
}

#[test]
fn test_v1_0_script_still_valid_without_faults() {
    let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    assert!(script.validate().is_ok());
    assert!(script.faults.is_empty());
}

#[test]
fn test_invalid_version() {
    let yaml = r#"
schema_version: "2.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    let err = script.validate().unwrap_err();
    assert!(err.to_string().contains("Unsupported schema_version"));
}

#[test]
fn test_invalid_max_steps() {
    let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 0
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    let err = script.validate().unwrap_err();
    assert!(err.to_string().contains("max_steps"));
}

#[test]
fn test_empty_firmware_is_permitted_for_rom_boot() {
    // An empty `inputs.firmware` is now VALID at the schema level: the
    // faithful ESP32-C3 rom-boot path carries no debug ELF, so the builder
    // emits `firmware: ""`. `labwired test` decides whether that is the
    // ELF-less rom-boot path (--rom-boot on an esp32c3) or a config error.
    let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: ""
limits:
  max_steps: 100
"#;
    let script: TestScript = serde_yaml::from_str(yaml).unwrap();
    assert!(script.validate().is_ok());
}

#[test]
fn test_system_manifest_accepts_uart_device_board_io_kind() {
    let yaml = r#"
name: "uart-device-smoke"
chip: "inline"
board_io:
  - id: "iolink_master"
    kind: "uart_device"
    peripheral: "uart2"
    pin: 2
    signal: "output"
    active_high: true
"#;

    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(manifest.board_io[0].kind, BoardIoKind::UartDevice);
}

#[test]
fn test_external_device_preserves_target_neutral_bus_signal_route() {
    let yaml = r#"
name: "stm32-i2c-route-shape"
chip: "inline"
external_devices:
  - id: "oled"
    type: "oled-ssd1306-128x32"
    connection: "i2c1"
    route:
      sda: "PB7"
      scl: "PB6"
    config:
      i2c_address: 0x3c
"#;

    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
    let route = &manifest.external_devices[0].route;
    assert_eq!(route.get("sda").map(String::as_str), Some("PB7"));
    assert_eq!(route.get("scl").map(String::as_str), Some("PB6"));

    let round_trip = serde_yaml::to_string(&manifest).unwrap();
    assert!(round_trip.contains("route:"));
    assert!(round_trip.contains("sda: PB7"));
    assert!(round_trip.contains("scl: PB6"));
}

fn write_temp_file(prefix: &str, contents: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("labwired-config-tests");
    let _ = std::fs::create_dir_all(&dir);

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = dir.join(format!("{}-{}.yaml", prefix, nonce));
    std::fs::write(&path, contents).expect("Failed to write temp file");
    path
}

#[test]
fn test_load_legacy_v1_script() {
    let script_path = write_temp_file(
        "legacy-v1",
        r#"
schema_version: 1
max_steps: 0
assertions: []
"#,
    );

    let loaded = load_test_script(&script_path).unwrap();
    assert!(matches!(loaded, LoadedTestScript::LegacyV1(_)));
}

#[test]
fn legacy_script_rejects_node_qualified_memory_assertions() {
    for (name, node) in [("legacy-node", "tester"), ("legacy-null-node", "null")] {
        let script_path = write_temp_file(
            name,
            &format!(
                r#"
schema_version: 1
max_steps: 10
assertions:
  - memory_value:
      node: {node}
      address: 0x20010000
      expected_value: 1
"#
            ),
        );

        let err = load_test_script(&script_path).unwrap_err().to_string();
        assert!(err.contains("node"), "unexpected error: {err}");
        assert!(err.contains("legacy"), "unexpected error: {err}");
    }
}

#[test]
fn legacy_explicit_null_memory_node_round_trips_as_invalid() {
    let script: LegacyTestScriptV1 = serde_yaml::from_str(
        r#"
schema_version: 1
max_steps: 10
assertions:
  - memory_value:
      node: null
      address: 0x20010000
      expected_value: 1
"#,
    )
    .unwrap();
    let err = script.validate().unwrap_err().to_string();
    assert!(err.contains("legacy"), "unexpected error: {err}");

    let serialized = serde_yaml::to_string(&script).unwrap();
    assert!(
        serialized.contains("node: null"),
        "explicit null node was lost during serialization: {serialized}"
    );
    let round_tripped: LegacyTestScriptV1 = serde_yaml::from_str(&serialized).unwrap();
    let err = round_tripped.validate().unwrap_err().to_string();
    assert!(err.contains("legacy"), "unexpected error: {err}");
}

#[test]
fn load_env_script_selects_env_variant_and_preserves_allowed_fields() {
    let script_path = write_temp_file(
        "env-script",
        r#"
schema_version: "1.0"
inputs:
  env: "twonode-env.yaml"
limits:
  max_steps: 50000
  max_cycles: 75000
  max_uart_bytes: 2048
  wall_time_ms: 3000
assertions:
  - memory_value:
      node: tester
      address: 0x20010000
      expected_value: 0xA5
      size: 1
"#,
    );

    let loaded = load_test_script(&script_path).unwrap();
    match loaded {
        LoadedTestScript::Env(script) => {
            assert_eq!(script.inputs.env, "twonode-env.yaml");
            assert_eq!(script.limits.max_steps, 50_000);
            assert_eq!(script.limits.max_cycles, Some(75_000));
            assert_eq!(script.limits.max_uart_bytes, Some(2_048));
            assert_eq!(script.limits.wall_time_ms, Some(3_000));
            let TestAssertion::MemoryValue(assertion) = &script.assertions[0] else {
                panic!("expected memory_value assertion");
            };
            assert_eq!(assertion.memory_value.node.as_deref(), Some("tester"));
        }
        other => panic!("expected environment script, got {other:?}"),
    }
}

#[test]
fn env_script_serialization_does_not_introduce_unsupported_runner_options() {
    let script: EnvTestScript = serde_yaml::from_str(
        r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits:
  max_steps: 10
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
    )
    .unwrap();
    script.validate().unwrap();

    let serialized = serde_yaml::to_string(&script).unwrap();
    for option in [
        "no_progress_steps",
        "max_vcd_bytes",
        "stop_when_assertions_pass",
        "stop_when_assertions_pass_settle_steps",
        "stop_when_assertions_pass_min_steps",
        "faults:",
        "verdict:",
        "stimuli:",
    ] {
        assert!(
            !serialized.contains(option),
            "unexpected serialized script: {serialized}"
        );
    }

    let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
    round_tripped.validate().unwrap();
}

#[test]
fn env_script_accepts_and_round_trips_assertion_completion_limits() {
    let script: EnvTestScript = serde_yaml::from_str(
        r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits:
  max_steps: 10
  stop_when_assertions_pass: true
  stop_when_assertions_pass_settle_steps: 7
  stop_when_assertions_pass_min_steps: 3
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
    )
    .unwrap();

    script.validate().unwrap();
    assert!(script.limits.stop_when_assertions_pass);
    assert_eq!(script.limits.stop_when_assertions_pass_settle_steps, 7);
    assert_eq!(script.limits.stop_when_assertions_pass_min_steps, 3);

    let serialized = serde_yaml::to_string(&script).unwrap();
    for field in [
        "stop_when_assertions_pass: true",
        "stop_when_assertions_pass_settle_steps: 7",
        "stop_when_assertions_pass_min_steps: 3",
    ] {
        assert!(serialized.contains(field), "missing {field}: {serialized}");
    }
    let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
    round_tripped.validate().unwrap();
    assert_eq!(
        round_tripped.limits.stop_when_assertions_pass,
        script.limits.stop_when_assertions_pass
    );
    assert_eq!(
        round_tripped.limits.stop_when_assertions_pass_settle_steps,
        script.limits.stop_when_assertions_pass_settle_steps
    );
    assert_eq!(
        round_tripped.limits.stop_when_assertions_pass_min_steps,
        script.limits.stop_when_assertions_pass_min_steps
    );
}

#[test]
fn env_script_preserves_explicit_assertion_completion_defaults() {
    let script: EnvTestScript = serde_yaml::from_str(
        r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits:
  max_steps: 10
  stop_when_assertions_pass: false
  stop_when_assertions_pass_settle_steps: 100000
  stop_when_assertions_pass_min_steps: 0
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
    )
    .unwrap();

    script.validate().unwrap();
    let serialized = serde_yaml::to_string(&script).unwrap();
    for field in [
        "stop_when_assertions_pass: false",
        "stop_when_assertions_pass_settle_steps: 100000",
        "stop_when_assertions_pass_min_steps: 0",
    ] {
        assert!(serialized.contains(field), "missing {field}: {serialized}");
    }

    let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
    round_tripped.validate().unwrap();
    assert!(!round_tripped.limits.stop_when_assertions_pass);
    assert_eq!(
        round_tripped.limits.stop_when_assertions_pass_settle_steps,
        100_000
    );
    assert_eq!(round_tripped.limits.stop_when_assertions_pass_min_steps, 0);
}

#[test]
fn env_script_rejects_null_assertion_completion_limits() {
    for (name, extra, field) in [
        (
            "early-pass-null",
            "  stop_when_assertions_pass: null",
            "stop_when_assertions_pass",
        ),
        (
            "early-pass-settle-null",
            "  stop_when_assertions_pass_settle_steps: null",
            "stop_when_assertions_pass_settle_steps",
        ),
        (
            "early-pass-minimum-null",
            "  stop_when_assertions_pass_min_steps: null",
            "stop_when_assertions_pass_min_steps",
        ),
    ] {
        let script_path = write_temp_file(
            name,
            &format!(
                r#"
schema_version: "1.0"
inputs: {{ env: "twonode-env.yaml" }}
limits:
  max_steps: 10
{extra}
assertions:
  - memory_value: {{ node: tester, address: 0x20010000, expected_value: 1 }}
"#
            ),
        );
        let err = load_test_script(&script_path).unwrap_err().to_string();
        assert!(err.contains(field), "unexpected error: {err}");
        assert!(err.contains("must not be null"), "unexpected error: {err}");
    }
}

#[test]
fn env_script_preserves_invalid_null_assertion_completion_limits() {
    for (field, value) in [
        ("stop_when_assertions_pass", "null"),
        ("stop_when_assertions_pass_settle_steps", "null"),
        ("stop_when_assertions_pass_min_steps", "null"),
    ] {
        let script: EnvTestScript = serde_yaml::from_str(&format!(
            r#"
schema_version: "1.0"
inputs: {{ env: "twonode-env.yaml" }}
limits:
  max_steps: 10
  {field}: {value}
assertions:
  - memory_value: {{ node: tester, address: 0x20010000, expected_value: 1 }}
"#
        ))
        .unwrap();

        let serialized = serde_yaml::to_string(&script).unwrap();
        assert!(
            serialized.contains(&format!("{field}: {value}")),
            "explicit null {field} was lost during serialization: {serialized}"
        );
        let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
        let err = round_tripped.validate().unwrap_err().to_string();
        assert!(err.contains(field), "unexpected error: {err}");
        assert!(err.contains("must not be null"), "unexpected error: {err}");
    }
}

fn valid_env_script_for_mutation() -> EnvTestScript {
    serde_yaml::from_str(
        r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits:
  max_steps: 10
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
    )
    .unwrap()
}

#[test]
fn env_validation_and_serialization_reject_publicly_mutated_unsupported_values() {
    macro_rules! assert_rejected_and_round_trips_invalid {
        ($field:literal, $mutate:expr) => {{
            let mut script = valid_env_script_for_mutation();
            $mutate(&mut script);

            let err = script.validate().unwrap_err().to_string();
            assert!(err.contains($field), "unexpected error: {err}");

            let serialized = serde_yaml::to_string(&script).unwrap();
            assert!(
                serialized.contains(&format!("{}:", $field)),
                "missing unsupported field after serialization: {serialized}"
            );
            let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
            let err = round_tripped.validate().unwrap_err().to_string();
            assert!(err.contains($field), "unexpected error: {err}");
        }};
    }

    assert_rejected_and_round_trips_invalid!("no_progress_steps", |script: &mut EnvTestScript| {
        script.limits.no_progress_steps = Some(1)
    });
    assert_rejected_and_round_trips_invalid!("max_vcd_bytes", |script: &mut EnvTestScript| {
        script.limits.max_vcd_bytes = Some(1)
    });
    assert_rejected_and_round_trips_invalid!("faults", |script: &mut EnvTestScript| {
        script.faults.push(
            serde_yaml::from_str(
                r#"
id: unsupported
kind: missing_clock
"#,
            )
            .unwrap(),
        )
    });
    assert_rejected_and_round_trips_invalid!("verdict", |script: &mut EnvTestScript| {
        script.verdict = Some(serde_yaml::from_str("{}").unwrap())
    });
    assert_rejected_and_round_trips_invalid!("stimuli", |script: &mut EnvTestScript| {
        script.stimuli.push(
            serde_yaml::from_str(
                r#"
target: { channel: x }
value: 1.0
"#,
            )
            .unwrap(),
        )
    });
    assert_rejected_and_round_trips_invalid!("uart_injections", |script: &mut EnvTestScript| {
        script.uart_injections.push(
            serde_yaml::from_str(
                r#"
uart: "uart1"
bytes: "A"
"#,
            )
            .unwrap(),
        )
    });
}

#[test]
fn explicitly_unsupported_env_fields_round_trip_as_invalid() {
    for (limits_extra, top_level_extra, expected_serialized, diagnostic) in [
        (
            "  no_progress_steps: null",
            "",
            "no_progress_steps: null",
            "no_progress_steps",
        ),
        (
            "  max_vcd_bytes: null",
            "",
            "max_vcd_bytes: null",
            "max_vcd_bytes",
        ),
        ("", "faults: null", "faults: null", "faults"),
        ("", "faults: []", "faults: []", "faults"),
        ("", "verdict: null", "verdict: null", "verdict"),
        ("", "stimuli: null", "stimuli: null", "stimuli"),
        ("", "stimuli: []", "stimuli: []", "stimuli"),
        (
            "",
            "uart_injections: null",
            "uart_injections: null",
            "uart_injections",
        ),
        (
            "",
            "uart_injections: []",
            "uart_injections: []",
            "uart_injections",
        ),
    ] {
        let yaml = format!(
            r#"
schema_version: "1.0"
inputs: {{ env: "twonode-env.yaml" }}
limits:
  max_steps: 10
{limits_extra}
assertions:
  - memory_value: {{ node: tester, address: 0x20010000, expected_value: 1 }}
{top_level_extra}
"#
        );
        let script: EnvTestScript = serde_yaml::from_str(&yaml).unwrap();

        let err = script.validate().unwrap_err().to_string();
        assert!(err.contains(diagnostic), "unexpected error: {err}");

        let serialized = serde_yaml::to_string(&script).unwrap();
        assert!(
            serialized.contains(expected_serialized),
            "missing explicit unsupported field after serialization: {serialized}"
        );
        let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
        let err = round_tripped.validate().unwrap_err().to_string();
        assert!(err.contains(diagnostic), "unexpected error: {err}");
    }
}

#[test]
fn single_node_and_legacy_validation_reject_public_memory_node_mutations() {
    let mut single_node: TestScript = serde_yaml::from_str(
        r#"
schema_version: "1.0"
inputs: { firmware: "fw.elf" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { address: 0x20010000, expected_value: 1 }
"#,
    )
    .unwrap();
    {
        let TestAssertion::MemoryValue(assertion) = &mut single_node.assertions[0] else {
            panic!("expected memory_value assertion");
        };
        assertion.memory_value.node = Some("tester".to_string());
    }
    let err = single_node.validate().unwrap_err().to_string();
    assert!(err.contains("single-node"), "unexpected error: {err}");

    let mut legacy: LegacyTestScriptV1 = serde_yaml::from_str(
        r#"
schema_version: 1
max_steps: 10
assertions:
  - memory_value: { address: 0x20010000, expected_value: 1 }
"#,
    )
    .unwrap();
    {
        let TestAssertion::MemoryValue(assertion) = &mut legacy.assertions[0] else {
            panic!("expected memory_value assertion");
        };
        assertion.memory_value.node = Some("tester".to_string());
    }
    let err = legacy.validate().unwrap_err().to_string();
    assert!(err.contains("legacy"), "unexpected error: {err}");
}

#[test]
fn load_single_node_script_still_selects_v1_0_variant() {
    let script_path = write_temp_file(
        "single-node-script",
        r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
  system: "system.yaml"
limits:
  max_steps: 1000
"#,
    );

    assert!(matches!(
        load_test_script(&script_path).unwrap(),
        LoadedTestScript::V1_0(_)
    ));
}

#[test]
fn single_node_script_rejects_node_qualified_memory_assertions() {
    let script_path = write_temp_file(
        "single-node-memory-node",
        r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 1000
assertions:
  - memory_value:
      node: tester
      address: 0x20010000
      expected_value: 1
"#,
    );

    let err = load_test_script(&script_path).unwrap_err().to_string();
    assert!(err.contains("node"), "unexpected error: {err}");
    assert!(err.contains("single-node"), "unexpected error: {err}");
}

#[test]
fn single_node_script_rejects_explicit_null_memory_node() {
    let script_path = write_temp_file(
        "single-node-memory-null-node",
        r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 1000
assertions:
  - memory_value:
      node: null
      address: 0x20010000
      expected_value: 1
"#,
    );

    let err = load_test_script(&script_path).unwrap_err().to_string();
    assert!(err.contains("node"), "unexpected error: {err}");
    assert!(err.contains("single-node"), "unexpected error: {err}");
}

#[test]
fn single_node_explicit_null_memory_node_round_trips_as_invalid() {
    let script: TestScript = serde_yaml::from_str(
        r#"
schema_version: "1.0"
inputs: { firmware: "fw.elf" }
limits: { max_steps: 10 }
assertions:
  - memory_value:
      node: null
      address: 0x20010000
      expected_value: 1
"#,
    )
    .unwrap();
    let err = script.validate().unwrap_err().to_string();
    assert!(err.contains("single-node"), "unexpected error: {err}");

    let serialized = serde_yaml::to_string(&script).unwrap();
    assert!(
        serialized.contains("node: null"),
        "explicit null node was lost during serialization: {serialized}"
    );
    let round_tripped: TestScript = serde_yaml::from_str(&serialized).unwrap();
    let err = round_tripped.validate().unwrap_err().to_string();
    assert!(err.contains("single-node"), "unexpected error: {err}");
}

#[test]
fn ordinary_memory_assertion_without_node_round_trips_without_node_key() {
    let script: TestScript = serde_yaml::from_str(
        r#"
schema_version: "1.0"
inputs: { firmware: "fw.elf" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { address: 0x20010000, expected_value: 1 }
"#,
    )
    .unwrap();
    script.validate().unwrap();

    let serialized = serde_yaml::to_string(&script).unwrap();
    assert!(
        !serialized.contains("node:"),
        "unexpected serialized script: {serialized}"
    );

    let round_tripped: TestScript = serde_yaml::from_str(&serialized).unwrap();
    round_tripped.validate().unwrap();
}

#[test]
fn env_script_rejects_missing_or_blank_env_path() {
    for (name, yaml) in [
        (
            "blank-env",
            r#"
schema_version: "1.0"
inputs: { env: "   " }
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
        ),
        (
            "missing-env",
            r#"
schema_version: "1.0"
inputs: {}
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
        ),
    ] {
        let script_path = write_temp_file(name, yaml);
        let err = load_test_script(&script_path).unwrap_err().to_string();
        assert!(err.contains("env"), "unexpected error: {err}");
    }
}

#[test]
fn env_script_requires_schema_v1_0_and_positive_step_limit() {
    for (name, yaml, diagnostic) in [
        (
            "wrong-env-schema",
            r#"
schema_version: "1.1"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
            "schema_version",
        ),
        (
            "zero-env-steps",
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 0 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
            "max_steps",
        ),
    ] {
        let script_path = write_temp_file(name, yaml);
        let err = load_test_script(&script_path).unwrap_err().to_string();
        assert!(err.contains(diagnostic), "unexpected error: {err}");
    }
}

#[test]
fn env_script_requires_memory_assertions_with_nodes() {
    for (name, yaml, diagnostic) in [
        (
            "missing-node",
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { address: 0x20010000, expected_value: 1 }
"#,
            "node",
        ),
        (
            "blank-node",
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: " ", address: 0x20010000, expected_value: 1 }
"#,
            "node",
        ),
        (
            // Still unsupported, and still refused: the world runner has no
            // per-node stop reason to compare against. The refusal must
            // survive UART assertions becoming legal, or admitting those
            // would have quietly admitted everything.
            "unsupported-assertion",
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - expected_stop_reason: max_steps
"#,
            "cannot observe",
        ),
    ] {
        let script_path = write_temp_file(name, yaml);
        let err = load_test_script(&script_path).unwrap_err().to_string();
        assert!(err.contains(diagnostic), "unexpected error: {err}");
    }
}

/// UART assertions carry no node id, so the node rules above do not apply
/// to them; they are satisfied by any node printing the text. They load
/// alone and alongside a node-qualified `memory_value`.
#[test]
fn env_script_accepts_uart_assertions() {
    for (name, yaml) in [
        (
            "uart-only",
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - uart_contains: "PASS"
  - uart_regex: "PA+SS"
  - uart_ordered: ["boot", "PASS"]
"#,
        ),
        (
            "uart-and-memory",
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
  - uart_contains: "PASS"
"#,
        ),
    ] {
        let script_path = write_temp_file(name, yaml);
        let script = load_test_script(&script_path)
            .unwrap_or_else(|error| panic!("{name} must load: {error}"));
        assert!(
            matches!(script, LoadedTestScript::Env(_)),
            "{name} must load as an environment script"
        );
    }
}

#[test]
fn env_script_may_assert_nothing_and_still_load() {
    // An observational world run: it reports what each node printed and
    // whether anything faulted, with no author oracle. `status` then rests
    // on the safety stop alone, which is exactly the claim a hosted verify
    // makes ("every chip compiled and the world ran without faulting").
    // A script that DOES carry assertions is still held to every rule
    // above, so a gate cannot quietly weaken itself into a green.
    let script_path = write_temp_file(
        "observational-env",
        r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions: []
"#,
    );
    let script = load_test_script(&script_path).expect("observational env script must load");
    let LoadedTestScript::Env(env) = script else {
        panic!("expected an environment script");
    };
    assert!(env.assertions.is_empty());
}

#[test]
fn env_script_rejects_combined_firmware_and_env_inputs() {
    let script_path = write_temp_file(
        "combined-inputs",
        r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml", firmware: "fw.elf" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
    );

    let err = load_test_script(&script_path).unwrap_err().to_string();
    assert!(err.contains("both") && err.contains("env") && err.contains("firmware"));
}

#[test]
fn env_script_rejects_runner_options_it_cannot_honor() {
    for (name, extra, diagnostic) in [
        ("no-progress", "  no_progress_steps: 5", "no_progress_steps"),
        (
            "no-progress-null",
            "  no_progress_steps: null",
            "no_progress_steps",
        ),
        ("vcd", "  max_vcd_bytes: 1024", "max_vcd_bytes"),
        ("vcd-null", "  max_vcd_bytes: null", "max_vcd_bytes"),
        (
            "faults",
            "faults:\n  - id: x\n    kind: missing_clock",
            "faults",
        ),
        ("faults-empty", "faults: []", "faults"),
        ("faults-null", "faults: null", "faults"),
        ("verdict", "verdict: {}", "verdict"),
        ("verdict-null", "verdict: null", "verdict"),
        (
            "stimuli",
            "stimuli:\n  - target: { channel: x }\n    value: 1.0",
            "stimuli",
        ),
        ("stimuli-empty", "stimuli: []", "stimuli"),
        ("stimuli-null", "stimuli: null", "stimuli"),
        (
            "uart-injections",
            "uart_injections:\n  - uart: uart1\n    bytes: \"A\"",
            "uart_injections",
        ),
        (
            "uart-injections-empty",
            "uart_injections: []",
            "uart_injections",
        ),
        (
            "uart-injections-null",
            "uart_injections: null",
            "uart_injections",
        ),
    ] {
        let script_path = write_temp_file(
            name,
            &format!(
                r#"
schema_version: "1.0"
inputs: {{ env: "twonode-env.yaml" }}
limits:
  max_steps: 10
{extra}
assertions:
  - memory_value: {{ node: tester, address: 0x20010000, expected_value: 1 }}
"#
            ),
        );

        let err = load_test_script(&script_path).unwrap_err().to_string();
        assert!(err.contains(diagnostic), "unexpected error: {err}");
    }
}

#[test]
fn test_peripheral_descriptor_parsing() {
    let yaml = r#"
peripheral: "SPI"
version: "1.0"
registers:
  - id: "CR1"
    address_offset: 0x00
    size: 16
    access: "R/W"
    reset_value: 0x0000
    fields:
      - name: "SPE"
        bit_range: [6, 6]
        description: "SPI Enable"
  - id: "DR"
    address_offset: 0x0C
    size: 16
    access: "R/W"
    reset_value: 0x0000
    side_effects:
      on_read: "clear_rxne"
      on_write: "start_tx"
"#;
    let desc = PeripheralDescriptor::from_yaml(yaml).unwrap();
    assert_eq!(desc.peripheral, "SPI");
    assert_eq!(desc.registers.len(), 2);
    assert_eq!(desc.registers[0].id, "CR1");
    assert_eq!(desc.registers[0].access, Access::ReadWrite);
    assert_eq!(
        desc.registers[1].side_effects.as_ref().unwrap().on_read,
        Some("clear_rxne".to_string())
    );
}

#[test]
fn uds_tester_assertion_parses_result_done() {
    let yaml = r#"
- uds_tester:
    id: "uds-tester"
    result: done
"#;
    let assertions: Vec<TestAssertion> = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(assertions.len(), 1);
    match &assertions[0] {
        TestAssertion::UdsTester(a) => {
            assert_eq!(a.uds_tester.id, "uds-tester");
            assert!(matches!(a.uds_tester.result, UdsTesterResult::Done));
        }
        other => panic!("expected UdsTester variant, got {:?}", other),
    }
}

#[test]
fn env_script_rejects_semihosting_contains_explicitly() {
    let script_path = write_temp_file(
        "env-semihost-assertion",
        r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - semihosting_contains: "semihost hello"
"#,
    );

    let err = load_test_script(&script_path).unwrap_err().to_string();
    assert!(err.contains("cannot observe"), "unexpected error: {err}");
}

#[test]
fn env_script_rejects_itm_contains_explicitly() {
    let script_path = write_temp_file(
        "env-itm-assertion",
        r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - itm_contains: "ITM hello"
"#,
    );

    let err = load_test_script(&script_path).unwrap_err().to_string();
    assert!(err.contains("cannot observe"), "unexpected error: {err}");
}

#[test]
fn env_script_rejects_rtt_contains_explicitly() {
    let script_path = write_temp_file(
        "env-rtt-assertion",
        r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - rtt_contains: "RTT hello"
"#,
    );

    let err = load_test_script(&script_path).unwrap_err().to_string();
    assert!(err.contains("cannot observe"), "unexpected error: {err}");
}
