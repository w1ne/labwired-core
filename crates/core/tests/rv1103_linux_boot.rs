//! This gate invokes the shipped zImage CLI once; receipts/fixtures cannot boot it.
use labwired_core::system::rv1103_linux::*;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

const CYCLES: u64 = 24_000_000_000;
const RETIREMENTS: u64 = 2_000_000_000;
const WALL_SECONDS: u64 = 1800;
const CONSOLE_BYTES: u64 = 65536;

fn validate(receipt: &Value, artifacts: &LinuxArtifacts) -> Result<(), String> {
    let require = |condition: bool, message: &str| {
        if condition {
            Ok(())
        } else {
            Err(message.to_owned())
        }
    };
    for (key, expected) in [
        ("board", BOARD),
        ("mode", "reduced-direct-zImage"),
        ("status", "PASS"),
        ("guest_boot", "ACCEPTED"),
        ("physical_acceptance", "UNAVAILABLE"),
        ("stop", "userspace acceptance"),
        ("source_revision", BACKEND_SOURCE_REVISION),
        ("source_archive_sha256", BACKEND_SOURCE_ARCHIVE_SHA256),
        (
            "timing",
            "one retired instruction tick; functional calibration, not physical timing",
        ),
    ] {
        require(receipt[key] == expected, &format!("unexpected {key}"))?;
    }
    require(receipt["schema"] == 1, "receipt schema")?;
    require(
        receipt["compiled_backend_sha256"]
            .as_str()
            .is_some_and(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }),
        "compiled backend SHA256",
    )?;
    for (name, bytes) in [
        ("zImage", &artifacts.kernel),
        ("rv1103-direct.dtb", &artifacts.dtb),
        ("initramfs.cpio", &artifacts.initramfs),
        ("kernel.config", &artifacts.config),
    ] {
        require(
            receipt["actual_input_sha256"][name] == sha256(bytes),
            &format!("actual {name} hash"),
        )?;
        require(
            receipt["artifacts"][name]
                == serde_json::to_value(&artifacts.manifest.artifacts[name]).unwrap(),
            &format!("manifest {name} identity"),
        )?;
    }
    require(
        receipt["cpu_hz"] == TIMER_HZ
            && receipt["cpu_hz_default"] == CPU_HZ
            && receipt["cpu_hz_override"] == true
            && receipt["timer_hz"] == TIMER_HZ,
        "explicit 24 MHz functional calibration",
    )?;
    require(
        receipt["event_scheduler"] == true && receipt["idle_fast_forward"] == true,
        "event/idle execution configuration",
    )?;
    for (key, value) in [
        ("cycles", CYCLES),
        ("retirements", RETIREMENTS),
        ("fuel", CYCLES),
        ("wall_seconds", WALL_SECONDS),
        ("console_bytes", CONSOLE_BYTES),
        ("advance_window", 4096),
        ("batch_cap", 64),
    ] {
        require(receipt["limits"][key] == value, &format!("bounded {key}"))?;
    }
    let number = |key: &str| {
        receipt[key]
            .as_u64()
            .ok_or_else(|| format!("missing {key}"))
    };
    let cycles = number("cycles")?;
    let retired = number("retirements_completed_windows")?;
    let fuel = number("fuel_completed_windows")?;
    let idle = number("idle_cycles")?;
    require(
        retired > 32 && retired <= RETIREMENTS && cycles <= CYCLES && fuel <= CYCLES,
        "real bounded retirement/cycle/fuel totals",
    )?;
    require(
        retired.checked_add(idle) == Some(cycles) && fuel == cycles,
        "instruction ticks plus idle cycles equal Machine fuel/cycles",
    )?;
    require(
        receipt.get("partial_error_accounting").is_none(),
        "partial failed window",
    )?;
    require(
        receipt["wall_elapsed_seconds"]
            .as_f64()
            .is_some_and(|v| v.is_finite() && v > 0.0 && v <= WALL_SECONDS as f64),
        "wall bound",
    )?;
    require(
        receipt["last_pc"].as_str().is_some_and(|pc| {
            pc.len() == 10 && pc.starts_with("0x") && pc[2..].bytes().all(|b| b.is_ascii_hexdigit())
        }),
        "last PC",
    )?;
    let console = receipt["console"].as_str().ok_or("missing UART2 console")?;
    require(console.len() <= CONSOLE_BYTES as usize, "console bound")?;
    require(
        !console.contains("Kernel panic") && !console.contains("LW_LINUX_BOOT_FAILURE"),
        "guest failure",
    )?;
    for token in [
        "Linux version 6.18.56-0-lts ",
        "Run /init as init process",
        "LW_LINUX_MOUNTS proc=ok sysfs=ok devtmpfs=ok",
        "LW_LINUX_PID1 pid=1 kernel=6.18.56-0-lts machine=armv7l",
        "processor",
        "CPU architecture",
        "MemTotal:",
    ] {
        require(console.contains(token), &format!("UART2 evidence: {token}"))?;
    }
    let lines: Vec<_> = console.lines().map(|l| l.trim_end_matches('\r')).collect();
    require(
        lines
            .iter()
            .filter(|l| **l == "LW_LINUX_BOOT_ACCEPTED")
            .count()
            == 1,
        "unique guest acceptance",
    )?;
    let stamp = |prefix: &str| -> Result<f64, String> {
        let values: Vec<_> = lines
            .iter()
            .filter_map(|l| l.strip_prefix(prefix))
            .collect();
        require(values.len() == 1, "unique guest uptime")?;
        let value: f64 = values[0].parse().map_err(|_| "invalid guest uptime")?;
        require(value.is_finite() && value >= 0.0, "finite guest uptime")?;
        Ok(value)
    };
    let before = stamp("LW_LINUX_UPTIME_BEFORE=")?;
    let after = stamp("LW_LINUX_UPTIME_AFTER=")?;
    require(after - before >= 0.8, "real sleep advances guest uptime")?;
    let evidence = &receipt["guest_evidence"];
    require(
        evidence["kernel"] == "6.18.56-0-lts"
            && evidence["pid"] == 1
            && evidence["machine"] == "armv7l"
            && evidence["proc_sys_devtmpfs"] == "mounted by PID1"
            && evidence["sleep"] == "real guest sleep syscall",
        "PID1 evidence",
    )?;
    for (key, expected) in [
        ("uptime_before", before),
        ("uptime_after", after),
        ("uptime_delta", after - before),
    ] {
        require(
            evidence[key]
                .as_f64()
                .is_some_and(|v| (v - expected).abs() < 1e-9),
            "receipt/console uptime agreement",
        )?;
    }
    Ok(())
}

#[test]
#[ignore = "real Linux boot: requires LABWIRED_RV1103_CLI and LABWIRED_RV1103_ARTIFACTS; up to 1800 seconds"]
fn production_zimage_reaches_pid1_and_advancing_guest_uptime() {
    let cli = PathBuf::from(
        std::env::var_os("LABWIRED_RV1103_CLI")
            .expect("set LABWIRED_RV1103_CLI to the shipped native CLI"),
    );
    let directory = PathBuf::from(
        std::env::var_os("LABWIRED_RV1103_ARTIFACTS")
            .expect("set LABWIRED_RV1103_ARTIFACTS to freshly prepared pinned artifacts"),
    );
    assert!(cli.is_file(), "missing shipped CLI: {}", cli.display());
    let artifacts =
        LinuxArtifacts::read(&directory).expect("strict prepared artifact prerequisites");
    let receipt = std::env::var_os("LABWIRED_RV1103_BOOT_RECEIPT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("rv1103-linux-boot-{}.json", std::process::id()))
        });
    assert!(
        !receipt.try_exists().expect("check fresh receipt"),
        "receipt must be create-new: {}",
        receipt.display()
    );
    // Record the actual invoked executable independently of the CLI's DLL receipt.
    // The sidecar is input provenance, never a substitute for guest acceptance.
    let cli_sha256 = sha256(&std::fs::read(&cli).expect("read shipped CLI for SHA256"));
    let sidecar = receipt.with_extension("gate.json");
    let provenance = serde_json::json!({
        "schema": 1, "gate": "production_zimage_reaches_pid1_and_advancing_guest_uptime",
        "cli_path": cli, "cli_sha256": cli_sha256,
        "artifacts_directory": directory, "boot_receipt": receipt,
        "guest_boot": "NOT_RUN"
    });
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&sidecar)
            .expect("create-new CLI provenance sidecar");
        file.write_all(
            serde_json::to_string_pretty(&provenance)
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        file.sync_all().unwrap();
    }
    eprintln!(
        "actual shipped CLI SHA256: {cli_sha256}; provenance: {}",
        sidecar.display()
    );
    eprintln!(
        "production zImage acceptance receipt (preserved on failure): {}",
        receipt.display()
    );
    let output = Command::new(&cli)
        .arg("rv1103-linux")
        .arg("--artifacts")
        .arg(directory)
        .arg("--receipt")
        .arg(&receipt)
        .args([
            "--cpu-hz",
            "24000000",
            "--max-cycles",
            "24000000000",
            "--max-retirements",
            "2000000000",
            "--max-fuel",
            "24000000000",
            "--wall-seconds",
            "1800",
            "--console-bytes",
            "65536",
        ])
        .output()
        .expect("execute actual shipped native CLI");
    assert_eq!(
        sha256(&std::fs::read(&cli).expect("rehash actual shipped CLI")),
        cli_sha256,
        "shipped CLI changed during gate; preserved provenance {}",
        sidecar.display()
    );
    let result: Value = serde_json::from_slice(&std::fs::read(&receipt).unwrap_or_else(|e| {
        panic!(
            "missing receipt: {e}; CLI stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    }))
    .expect("parse fresh boot receipt");
    assert!(
        output.status.success(),
        "CLI exit {}; last PC {}; stop {}; receipt {}; stderr: {}",
        output.status,
        result["last_pc"],
        result["stop"],
        receipt.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    validate(&result, &artifacts)
        .unwrap_or_else(|e| panic!("acceptance: {e}; preserved receipt {}", receipt.display()));
}
