use std::process::Command;
#[test]
fn explicit_reduced_linux_command_is_advertised() {
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["rv1103-linux", "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    for arg in [
        "--artifacts",
        "--receipt",
        "--cpu-hz",
        "--max-cycles",
        "--max-retirements",
        "--max-fuel",
        "--wall-seconds",
        "--console-bytes",
    ] {
        assert!(text.contains(arg), "missing {arg}");
    }
}
#[cfg(not(feature = "qemu-arm-system"))]
#[test]
fn disabled_backend_refuses_explicit_mode() {
    #[cfg(not(feature = "qemu-arm-system"))]
    {
        let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
            .args([
                "rv1103-linux",
                "--artifacts",
                "/missing",
                "--receipt",
                "/tmp/rv1103-disabled-test-unused.json",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("qemu-arm-system"));
    }
}

#[cfg(feature = "qemu-arm-system")]
#[test]
#[ignore = "requires independently prepared pinned artifacts in LABWIRED_RV1103_ARTIFACTS"]
fn bounded_native_zimage_run_returns_failure_receipt() {
    let artifacts =
        std::env::var("LABWIRED_RV1103_ARTIFACTS").expect("prepared artifact prerequisite");
    let receipt = std::env::var("LABWIRED_RV1103_FAILURE_RECEIPT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::temp_dir().join(format!(
                "labwired-rv1103-bounded-cli-{}.json",
                std::process::id()
            ))
        });
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("rv1103-linux")
        .arg("--artifacts")
        .arg(artifacts)
        .arg("--receipt")
        .arg(&receipt)
        .args([
            "--cpu-hz",
            "24000000",
            "--max-cycles",
            "32",
            "--max-retirements",
            "32",
            "--max-fuel",
            "32",
            "--wall-seconds",
            "30",
            "--console-bytes",
            "4096",
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a fuel-bounded boot cannot claim guest acceptance"
    );
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
    assert_eq!(result["status"], "FAIL");
    assert_eq!(result["guest_boot"], "NOT_ACCEPTED");
    assert_eq!(result["cycles"], 32);
    assert_eq!(result["retirements_completed_windows"], 32);
    assert_eq!(
        result["actual_input_sha256"]["zImage"],
        labwired_core::system::rv1103_linux::KERNEL_SHA256
    );
    assert_eq!(
        result["compiled_backend_sha256"].as_str().unwrap().len(),
        64
    );
    assert!(result["last_pc"].as_str().unwrap().starts_with("0x"));
    assert_eq!(result["cpu_hz_override"], true);
    assert_eq!(result["timer_hz"], 24000000);
    assert!(result["console"].as_str().unwrap().len() <= 4096);
    eprintln!(
        "bounded native zImage failure receipt: {}",
        receipt.display()
    );
}
