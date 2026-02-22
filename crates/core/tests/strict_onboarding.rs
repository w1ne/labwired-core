use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn test_strict_board_onboarding() -> anyhow::Result<()> {
    // Locate the `core` root (where Cargo.toml is)
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // configs/chips is invalid relative to crate root in workspace, need to go up
    // layout:
    // core/crates/core/tests/strict_onboarding.rs
    // core/configs/chips
    // So manifest_dir is .../core/crates/core
    // We need to go up two levels: ../../configs/chips
    let project_root = manifest_dir.parent().unwrap().parent().unwrap();
    let chips_dir = project_root.join("configs/chips");

    println!("Scanning for chips in: {:?}", chips_dir);

    let mut failed_boards = Vec::new();

    for entry in fs::read_dir(&chips_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            let file_stem = path.file_stem().unwrap().to_str().unwrap();

            // Skip CI fixtures or base templates if any (usually start with _)
            if file_stem.starts_with('_') || file_stem.starts_with("ci-fixture") {
                continue;
            }

            println!("---------------------------------------------------");
            println!("Verifying Strict Onboarding for: {}", file_stem);

            // 1. Check for io-smoke test existence
            // Convention: examples/nucleo-<arch>/io-smoke.yaml
            // This is tricky because "nucleo-h563zi" != "stm32h563".
            // We need a way to map chip -> board/example.
            // For now, we search for *any* example directory that uses this chip?
            // Or simpler: strictly require a test config at `examples/<board>/io-smoke.yaml`
            // where strict mapping isn't easy without metadata.

            // Heuristic: Search for a `system.yaml` or `io-smoke.yaml` that references this chip.
            // But that's slow.
            // Alternative: The plan implies checking if *supported* boards are broken.
            // Let's look for known example paths.

            let example_dir = find_example_for_chip(project_root, file_stem);

            if let Some(dir) = example_dir {
                println!("  Found example directory: {:?}", dir);
                let smoke_test = dir.join("io-smoke.yaml");

                if !smoke_test.exists() {
                    println!("  [FAIL] Missing io-smoke.yaml in {:?}", dir);
                    failed_boards.push(format!("{} (missing io-smoke.yaml)", file_stem));
                    continue;
                }

                // 1.5 Build firmware if io-smoke references a workspace target output path.
                if !ensure_smoke_firmware_exists(project_root, &smoke_test)? {
                    println!("  [FAIL] Firmware build failed for {}", file_stem);
                    failed_boards.push(format!("{} (firmware build failed)", file_stem));
                    continue;
                }

                // 2. Run the smoke test in Emulator mode
                // cargo run -p labwired-cli -- test --script <path> ...
                let status = Command::new("cargo")
                    .current_dir(project_root)
                    .args([
                        "run",
                        "-q",
                        "-p",
                        "labwired-cli",
                        "--",
                        "test",
                        "--script",
                        smoke_test.to_str().unwrap(),
                        "--no-uart-stdout", // Keep stdout clean
                    ])
                    .status()?;

                if !status.success() {
                    println!("  [FAIL] io-smoke test failed for {}", file_stem);
                    failed_boards.push(format!("{} (smoke test failed)", file_stem));
                } else {
                    println!("  [PASS] {} is strictly onboarded.", file_stem);
                }
            } else {
                println!("  [WARN] No example directory found matching chip '{}'. Skipping strict check.", file_stem);
            }
        }
    }

    if !failed_boards.is_empty() {
        return Err(anyhow::anyhow!(
            "Strict Board Onboarding Failed for: {:?}",
            failed_boards
        ));
    }

    Ok(())
}

fn ensure_smoke_firmware_exists(project_root: &Path, smoke_test: &Path) -> anyhow::Result<bool> {
    let Some(firmware_path) = firmware_path_from_smoke(smoke_test)? else {
        return Ok(true);
    };

    if firmware_path.exists() {
        return Ok(true);
    }

    let Ok(relative_path) = firmware_path.strip_prefix(project_root) else {
        return Ok(false);
    };

    let parts: Vec<String> = relative_path
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let Some(target_idx) = parts.iter().position(|p| p == "target") else {
        return Ok(false);
    };

    if parts.len() <= target_idx + 3 {
        return Ok(false);
    }

    let target = parts[target_idx + 1].clone();
    let profile = parts[target_idx + 2].clone();
    let package = parts[target_idx + 3].clone();

    let mut args = vec![
        "build".to_string(),
        "-p".to_string(),
        package,
        "--target".to_string(),
        target,
    ];
    if profile == "release" {
        args.push("--release".to_string());
    }

    let status = Command::new("cargo")
        .current_dir(project_root)
        .args(args)
        .status()?;

    Ok(status.success())
}

fn firmware_path_from_smoke(smoke_test: &Path) -> anyhow::Result<Option<PathBuf>> {
    let content = fs::read_to_string(smoke_test)?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("firmware:") {
            let rel = rest.trim().trim_matches('"').trim_matches('\'');
            if rel.is_empty() {
                return Ok(None);
            }
            let base = smoke_test.parent().unwrap_or_else(|| Path::new("."));
            return Ok(Some(base.join(rel)));
        }
    }
    Ok(None)
}

fn find_example_for_chip(root: &std::path::Path, chip_name: &str) -> Option<PathBuf> {
    // Semi-hardcoded lookup or decent heuristic.
    // stm32h563 -> nucleo-h563zi
    // stm32f401 -> nucleo-f401re (hypothetical)
    // stm32f103 -> bluepill (hypothetical)

    // Better: Grep all examples/**/system.yaml for "chip: .*<chip_name>"
    // This is robust.

    let examples = root.join("examples");
    if !examples.exists() {
        return None;
    }

    for entry in fs::read_dir(examples).ok()? {
        let entry = entry.ok()?;
        if entry.path().is_dir() {
            let system_yaml = entry.path().join("system.yaml");
            if system_yaml.exists() {
                let content = fs::read_to_string(&system_yaml).ok()?;
                if content.contains(&format!("chips/{}.yaml", chip_name))
                    || content.contains(&format!("chips/{}", chip_name))
                {
                    return Some(entry.path());
                }
            }
        }
    }
    None
}
