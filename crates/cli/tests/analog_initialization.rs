//! Production entry points expose the accepted analog initialization contract.
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn manifest(initialization: &str) -> String {
    format!(
        r#"name: explicit-initial-state
chip: stm32f401
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 1000
    outputs: {{ voltage: board.analog.pa0_volts }}
    config:
      substeps: 1
      probes: {{ voltage: 'v(out)' }}
{initialization}      netlist_text: |
        V1 vin 0 dc 1
        R1 vin out 1k
        C1 out 0 1u
"#
    )
}
const EXPLICIT: &str = "      initialization:\n        mode: transient_state\n        capacitor_voltages_v: { C1: 0 }\n        inductor_currents_a: {}\n";

#[test]
fn cosim_step_reports_real_initialization_and_preserves_default_op() {
    let temp = labwired_cli::test_support::unique_temp_dir("analog-initialization-step");
    std::fs::create_dir_all(&temp).unwrap();
    let path = temp.join("system.yaml");
    for (config, mode, peak) in [("", "operating_point", 3), (EXPLICIT, "transient_state", 4)] {
        std::fs::write(&path, manifest(config)).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
            .arg("cosim-step")
            .arg(&path)
            .args(["--steps", "1", "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(rows[0]["initialization"]["mode"], mode);
        assert_eq!(rows[0]["initialization"]["base_unknowns"], 3);
        assert_eq!(rows[0]["initialization"]["initialization_unknowns"], peak);
        let v = rows[0]["outputs"]["board.analog.pa0_volts"]
            .as_f64()
            .unwrap();
        let expected = if mode == "operating_point" {
            1.0
        } else {
            1.0 / 1001.0
        };
        assert!((v - expected).abs() < 1e-10, "{v} vs {expected}");
    }
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
fn test_result_records_the_model_contract_after_actual_cpu_execution() {
    let temp = labwired_cli::test_support::unique_temp_dir("analog-initialization-test");
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::write(temp.join("system.yaml"), manifest(EXPLICIT)).unwrap();
    let firmware =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/stm32f401-blinky.elf");
    let script = temp.join("script.yaml");
    std::fs::write(&script, format!("schema_version: '1.2'\ninputs:\n  firmware: '{}'\n  system: './system.yaml'\nlimits:\n  max_steps: 64\nassertions:\n  - expected_stop_reason: max_steps\n", firmware.display())).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("test")
        .arg("--script")
        .arg(&script)
        .arg("--output-dir")
        .arg(&temp)
        .arg("--no-uart-stdout")
        .output()
        .unwrap();
    let result: Value =
        serde_json::from_slice(&std::fs::read(temp.join("result.json")).unwrap()).unwrap();
    assert!(
        output.status.success(),
        "{}\n{result}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(result["steps_executed"], 64);
    assert_eq!(
        result["analog_initialization"]["circuit"],
        serde_json::json!({
            "mode":"transient_state", "base_unknowns":3,"initialization_unknowns":4
        })
    );
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
fn rejected_initial_state_is_not_reported_as_an_accepted_solution() {
    let temp = labwired_cli::test_support::unique_temp_dir("analog-initialization-refusal");
    std::fs::create_dir_all(&temp).unwrap();
    let path = temp.join("system.yaml");
    std::fs::write(&path, manifest(&EXPLICIT.replace("{ C1: 0 }", "{}"))).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("cosim-step")
        .arg(&path)
        .args(["--steps", "1", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("initialization_unknowns"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("C1"));
    std::fs::remove_dir_all(temp).unwrap();
}
