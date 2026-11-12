//! Exercise the production cosim-step command's explicit capacity admission.
use std::process::Command;

fn system(unknowns: usize, max: Option<&str>) -> String {
    let nodes = unknowns - 1;
    let mut yaml = String::from("name: capacity\nchip: stm32f401\ncosim_models:\n  - id: capacity\n    adapter: analog\n    step_ns: 100\n    outputs: { voltage: plant.voltage }\n    config:\n      substeps: 1\n");
    if let Some(max) = max {
        yaml.push_str(&format!("      max_unknowns: {max}\n"));
    }
    yaml.push_str(&format!(
        "      probes: {{ voltage: 'v(n{})' }}\n      netlist_text: |\n        V1 n0 0 dc 1\n",
        nodes - 1
    ));
    for i in 0..nodes - 1 {
        yaml.push_str(&format!("        R{i} n{i} n{} 1k\n", i + 1));
    }
    yaml.push_str(&format!("        Rlast n{} 0 1k\n", nodes - 1));
    yaml
}

#[test]
fn analog_capacity_cli_uses_default_and_explicit_bounds() {
    let temp = labwired_cli::test_support::unique_temp_dir("labwired-analog-capacity");
    std::fs::create_dir_all(&temp).unwrap();
    let path = temp.join("system.yaml");
    for (unknowns, max, ok, detail) in [
        (129, None, false, "128"),
        (379, Some("512"), true, ""),
        (513, Some("512"), false, "513"),
        (667, Some("667"), true, ""),
        (667, Some("512"), false, "667"),
        (1025, Some("1024"), false, "1025"),
        (3, Some("1025"), false, "max_unknowns"),
        (3, Some("1.5"), false, "max_unknowns"),
        (3, Some(".nan"), false, "max_unknowns"),
    ] {
        std::fs::write(&path, system(unknowns, max)).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_labwired"))
            .arg("cosim-step")
            .arg(&path)
            .args(["--steps", "2", "--json"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert_eq!(
            result.status.success(),
            ok,
            "unknowns={unknowns} max={max:?}: {stderr}"
        );
        if ok {
            let rows: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            let rows = rows.as_array().unwrap();
            assert_eq!(rows.len(), 2);
            for row in rows {
                let volts = row["outputs"]["plant.voltage"].as_f64().unwrap();
                assert!(
                    (volts - 1.0 / (unknowns - 1) as f64).abs() < 1e-9,
                    "{volts}"
                );
            }
        } else {
            assert!(stderr.contains(detail), "{stderr}");
        }
    }
    std::fs::remove_dir_all(temp).unwrap();
}
