// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Firmware branch coverage end to end: run the brake-ECU example's tests
//! with `labwired test --coverage`, merge them with `labwired coverage
//! merge`, and check the exact branches the tests leave untaken.
//!
//! The firmware (`examples/coverage-brake-ecu`) is built for Cortex-M3 and
//! RV32IMC from the same C source; both ELFs are committed fixtures.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn labwired(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Run each named test of the example into `out/<name>`, with coverage.
fn run_tests(out: &Path, tests: &[&str]) {
    let example = root().join("examples/coverage-brake-ecu");
    for t in tests {
        let script = format!("tests/{t}.yaml");
        let dir = out.join(t);
        let (code, _, err) = labwired(
            &example,
            &[
                "test",
                "--script",
                &script,
                "--coverage",
                "--no-uart-stdout",
                "--output-dir",
                dir.to_str().unwrap(),
            ],
        );
        assert_eq!(code, 0, "{t}: {err}");
        for f in ["coverage.info", "coverage.json", "coverage.cobertura.xml"] {
            assert!(dir.join(f).is_file(), "{t}: no {f}");
        }
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lw-covmerge-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The three "obvious" tests reach 81 % of lines but leave every alive-counter
/// check, the safe-state latch and the plausibility check untested — which
/// only branch coverage shows.
#[test]
fn obvious_tests_leave_the_e2e_checks_untested() {
    let out = tmp("baseline");
    run_tests(&out, &["pedal-press", "emergency-flag", "crc-error"]);

    let merged = out.join("merged.info");
    let json = out.join("merged.json");
    let (code, stdout, err) = labwired(
        &out,
        &[
            "coverage",
            "merge",
            ".",
            "-o",
            merged.to_str().unwrap(),
            "--json-out",
            json.to_str().unwrap(),
            "--fail-under-branches",
            "90",
        ],
    );
    assert_eq!(
        code, 1,
        "below the threshold the merge fails CI: {stdout}{err}"
    );
    assert!(
        err.contains("FAIL: branch coverage 76.3% is below"),
        "{err}"
    );
    assert!(stdout.contains("Merged 3 coverage file(s)"), "{stdout}");
    assert!(stdout.contains("66/81"), "lines: {stdout}");
    assert!(stdout.contains("29/38"), "branches: {stdout}");

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    let gaps: Vec<(u64, String)> = doc["uncovered_branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            (
                g["line"].as_u64().unwrap(),
                g["kind"].as_str().unwrap().into(),
            )
        })
        .collect();
    let lines: Vec<u64> = gaps.iter().map(|(l, _)| *l).collect();
    // 75 delta==0, 79 delta>MAX_DELTA, 86 safe_state, 93 switch (two
    // branches), 105 error_count>=MAX, 114 pedal>100, 137 resync loop.
    assert_eq!(lines, [75, 79, 86, 93, 93, 105, 114, 137], "{gaps:?}");
    assert!(gaps.contains(&(93, "never-reached".into())));

    // Each line knows which tests ran it.
    let g75 = &doc["uncovered_branches"][0];
    assert_eq!(g75["file"], "src/brake_ecu.c");
    let tests: Vec<&str> = g75["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    assert_eq!(tests, ["crc-error", "emergency-flag", "pedal-press"]);

    let _ = std::fs::remove_dir_all(&out);
}

/// Add the safety tests: every branch of the firmware is taken. Re-running
/// the merge over the same directory skips the earlier merge result.
#[test]
fn full_suite_takes_every_branch() {
    let out = tmp("full");
    run_tests(
        &out,
        &[
            "pedal-press",
            "emergency-flag",
            "crc-error",
            "e2e-sequence",
            "pedal-implausible",
            "line-noise",
        ],
    );
    let merged = out.join("merged.info");
    let cobertura = out.join("merged.xml");
    for _ in 0..2 {
        let (code, stdout, err) = labwired(
            &out,
            &[
                "coverage",
                "merge",
                ".",
                "-o",
                merged.to_str().unwrap(),
                "--cobertura",
                cobertura.to_str().unwrap(),
                "--fail-under-branches",
                "100",
            ],
        );
        assert_eq!(code, 0, "{stdout}{err}");
        assert!(stdout.contains("Merged 6 coverage file(s)"), "{stdout}");
        assert!(stdout.contains("38/38"), "{stdout}");
    }

    let lcov = std::fs::read_to_string(&merged).unwrap();
    assert!(lcov.starts_with("TN:labwired-merged\n"));
    assert!(lcov.contains("BRF:38\nBRH:38\n"), "{lcov}");
    // The fault handler is the only code no test should reach.
    assert!(lcov.contains("FNDA:0,Default_Handler\n"), "{lcov}");

    let xml = std::fs::read_to_string(&cobertura).unwrap();
    assert!(xml.contains("branch-rate=\"1.0000\" lines-covered=\"79\" lines-valid=\"81\""));
    assert!(xml.contains("filename=\"src/brake_ecu.c\""));

    let _ = std::fs::remove_dir_all(&out);
}

/// The same tests on the RISC-V build find the same untaken branches on the
/// same source lines: branch coverage no longer depends on a Thumb-only
/// instruction-length rule.
#[test]
fn riscv_build_finds_the_same_gaps() {
    let out = tmp("rv32");
    let example = root().join("examples/coverage-brake-ecu");
    for t in ["pedal-press", "emergency-flag", "crc-error"] {
        let yaml = std::fs::read_to_string(example.join(format!("tests/{t}.yaml")))
            .unwrap()
            .replace("coverage-brake-ecu-cm3.elf", "coverage-brake-ecu-rv32.elf")
            .replace("../system.yaml", "../system-rv32.yaml");
        let script = example.join(format!("tests/.rv32-{t}-{}.yaml", std::process::id()));
        std::fs::write(&script, yaml).unwrap();
        let dir = out.join(t);
        let (code, _, err) = labwired(
            &example,
            &[
                "test",
                "--script",
                script.to_str().unwrap(),
                "--coverage",
                "--no-uart-stdout",
                "--output-dir",
                dir.to_str().unwrap(),
            ],
        );
        let _ = std::fs::remove_file(&script);
        assert_eq!(code, 0, "{t}: {err}");
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("coverage.json")).unwrap())
                .unwrap();
        assert_eq!(report["branch_isa"], "riscv");
    }
    let json = out.join("merged.json");
    let (code, stdout, err) = labwired(
        &out,
        &[
            "coverage",
            "merge",
            ".",
            "--json-out",
            json.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        stdout.contains("29/38"),
        "same branch score as Cortex-M3: {stdout}"
    );
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    let lines: Vec<u64> = doc["uncovered_branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["line"].as_u64().unwrap())
        .collect();
    assert_eq!(lines, [75, 79, 86, 93, 93, 105, 114, 137]);

    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn merge_rejects_bad_input() {
    let out = tmp("bad");
    let bad = out.join("bad.info");
    std::fs::write(&bad, "SF:a.c\nDA:1,1\n").unwrap();
    let (code, _, err) = labwired(&out, &["coverage", "merge", bad.to_str().unwrap()]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("not valid LCOV"), "{err}");

    let empty = out.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let (code, _, err) = labwired(&out, &["coverage", "merge", empty.to_str().unwrap()]);
    assert_eq!(code, 2);
    assert!(err.contains("no .info files"), "{err}");
    let _ = std::fs::remove_dir_all(&out);
}

/// `labwired coverage` without a subcommand is still the register-model
/// coverage matrix, flags and all.
#[test]
fn coverage_without_subcommand_is_the_register_matrix() {
    let out = tmp("register");
    let json = out.join("matrix.json");
    let (code, stdout, err) = labwired(
        &root(),
        &[
            "coverage",
            "--svd",
            "tests/fixtures/svd/esp32s3.svd",
            "--json-out",
            json.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(stdout.contains("TOTAL"), "{stdout}");
    assert!(json.is_file());
    let _ = std::fs::remove_dir_all(&out);
}
