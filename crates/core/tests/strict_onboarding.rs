use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

/// Chips that have an example directory but have NOT yet received an
/// io-smoke.yaml. This list is shrink-only: removing a chip from here when
/// io-smoke.yaml is added is mandatory (the test fails if an allowlisted chip
/// gains a smoke, preventing stale entries from silently accumulating).
///
/// Adding a new chip to this list is TEMPORARY. Update with a tracking comment
/// and a GitHub issue number so the debt is visible.
const SMOKE_LESS_ALLOWLIST: &[&str] = &[
    "stm32f407",     // HIL oracle capture pending (nucleo-f407 I2C board)
    "stm32f401cdu6", // BlackPill variant; shared smoke with stm32f401 pending
    "stm32g474re",   // STM32G4 peripheral models in progress
    "stm32l476",     // L476 smoke added per survival tests, io-smoke.yaml pending
    "stm32wb55",     // BLE peripheral not yet modelled
    "stm32wba52",    // WBA series, early onboarding
    "esp32",         // Classic ESP32 Xtensa; separate e2e lane
    "nrf52840",      // io-smoke pruned with its example by #300; restore pending — see #311
];

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
    let mut unexpected_skips = Vec::new();
    // Track which allowlisted chips we actually saw without a smoke, so we can
    // detect when an allowlisted chip GAINS a smoke and the allowlist entry
    // becomes stale.
    let mut allowlisted_seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in fs::read_dir(&chips_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            let file_stem = path.file_stem().unwrap().to_str().unwrap();

            // Skip CI fixtures or base templates if any (usually start with _)
            if file_stem.starts_with('_') || file_stem.starts_with("ci-fixture") {
                continue;
            }

            // ESP32-S3 examples (esp32s3-blinky, esp32s3-hello-world,
            // esp32s3-i2c-tmp102) use the `+esp` toolchain and live outside
            // the main workspace, with their own Cargo + .cargo/config. The
            // strict-onboarding test invokes a generic `cargo test` runner
            // that can't drive those builds, so the chip is exercised by the
            // dedicated `e2e_blinky` / `e2e_hello_world` / `e2e_i2c_tmp102`
            // tests gated on `--features esp32s3-fixtures` instead.
            // The `esp32s3` chip itself is likewise not exercised by the generic
            // smoke: its IRAM (0x40370000), where Xtensa code loads, isn't mapped
            // by configs/chips/esp32s3.yaml, and no esp32s3.yaml-compatible looping
            // Xtensa fixture exists — so the ARM `uart-ok` fixture faults at entry
            // (captured len=0) instead of reaching max_steps. Xtensa execution is
            // validated by the hw-oracle Xtensa fixtures (fixtures/xtensa-asm/*)
            // instead.
            if file_stem == "esp32s3-zero" || file_stem == "esp32s3" {
                println!(
                    "  [SKIP] {} — Xtensa covered by hw-oracle / e2e fixture tests, not strict onboarding.",
                    file_stem
                );
                continue;
            }

            // nrf52832 and esp32c3 are exercised by the tier-1 fixture /
            // silicon-validation lane (examples/tier1-fixture/<chip>), which the
            // generic cargo-driven strict-onboarding runner can't build. They
            // have no top-level `system.yaml` example yet, so a dedicated
            // io-smoke.yaml is still pending — tracked in #309 (nrf52832) and
            // #310 (esp32c3). Skip here like the esp32s3 lane above.
            if file_stem == "nrf52832" || file_stem == "esp32c3" {
                println!(
                    "  [SKIP] {} — covered by the tier-1 fixture lane; io-smoke.yaml pending (see #309/#310).",
                    file_stem
                );
                continue;
            }

            // These chips are exercised by firmware-survival/conformance tests
            // rather than strict example-directory onboarding. Do not put them
            // in SMOKE_LESS_ALLOWLIST: that list is only for chips with an
            // example directory but no io-smoke.yaml.
            if file_stem == "mkw41z4" || file_stem == "nrf5340" {
                println!(
                    "  [SKIP] {} — covered by firmware survival/conformance gates, not strict onboarding.",
                    file_stem
                );
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
                    if SMOKE_LESS_ALLOWLIST.contains(&file_stem) {
                        // Known gap — tracked in the allowlist. Record it so the
                        // post-loop check can verify this chip is genuinely smoke-less.
                        allowlisted_seen.insert(file_stem.to_string());
                        println!(
                            "  [SKIP] {} — in SMOKE_LESS_ALLOWLIST (io-smoke.yaml not yet authored).",
                            file_stem
                        );
                        continue;
                    } else {
                        // NOT in the allowlist: every new chip must have an
                        // io-smoke.yaml the moment it lands, or be added to
                        // SMOKE_LESS_ALLOWLIST with a tracking comment.
                        println!(
                            "  [FAIL] {} — example dir present but no io-smoke.yaml \
                             and chip is NOT in SMOKE_LESS_ALLOWLIST. Add the smoke \
                             or add the chip to SMOKE_LESS_ALLOWLIST with a tracking comment.",
                            file_stem
                        );
                        failed_boards.push(format!(
                            "{} (missing io-smoke.yaml, not in allowlist)",
                            file_stem
                        ));
                        continue;
                    }
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
                println!(
                    "  [FAIL] No example directory found matching chip '{}'.",
                    file_stem
                );
                unexpected_skips.push(file_stem.to_string());
            }
        }
    }

    if !unexpected_skips.is_empty() {
        return Err(anyhow::anyhow!(
            "Strict Board Onboarding has unexpected example gaps: {:?}",
            unexpected_skips
        ));
    }

    // Shrink-only check: if an allowlisted chip now HAS an io-smoke.yaml (it
    // passed the smoke test above and was NOT added to allowlisted_seen), it
    // must be removed from SMOKE_LESS_ALLOWLIST. Stale allowlist entries defeat
    // the purpose of the gate.
    let mut stale_allowlist: Vec<&str> = Vec::new();
    for &chip in SMOKE_LESS_ALLOWLIST {
        if !allowlisted_seen.contains(chip as &str) {
            // This chip is in the allowlist but was NOT seen as smoke-less.
            // Either it gained an io-smoke.yaml (must be removed from allowlist)
            // or its example directory no longer exists (remove from list too).
            stale_allowlist.push(chip);
        }
    }
    if !stale_allowlist.is_empty() {
        return Err(anyhow::anyhow!(
            "SMOKE_LESS_ALLOWLIST entries are stale (chip gained io-smoke.yaml or no \
             longer exists): {:?}. Remove them from the allowlist in strict_onboarding.rs.",
            stale_allowlist
        ));
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

    let Ok(relative_path) = firmware_path.strip_prefix(project_root) else {
        return Ok(firmware_path.exists());
    };

    let parts: Vec<String> = relative_path
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let Some(target_idx) = parts.iter().position(|p| p == "target") else {
        // Firmware isn't a cargo `target/` build artifact — it's a committed,
        // pre-built fixture (e.g. tests/fixtures/*.elf produced by an external
        // toolchain like arm-none-eabi-gcc for the nRF54L15 examples). There is
        // nothing for the generic cargo runner to build; just require it to be
        // present on disk.
        return Ok(firmware_path.exists());
    };

    if parts.len() <= target_idx + 3 {
        return Ok(false);
    }

    let target = parts[target_idx + 1].clone();
    let profile = parts[target_idx + 2].clone();
    let package = parts[target_idx + 3].clone();
    let needs_thumbv6m_link_arg = target == "thumbv6m-none-eabi";

    let mut args = vec![
        "build".to_string(),
        "-p".to_string(),
        package.clone(),
        "--target".to_string(),
        target,
    ];
    if profile == "release" {
        args.push("--release".to_string());
    }

    let package_dir_crates = project_root.join("crates").join(&package);
    let package_dir_examples = project_root.join("examples").join(&package);

    let build_dir = if package_dir_crates.exists() {
        package_dir_crates.clone()
    } else if package_dir_examples.exists() {
        package_dir_examples.clone()
    } else {
        project_root.to_path_buf()
    };

    let mut command = Command::new("cargo");
    command
        .current_dir(&build_dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .args(args);
    if needs_thumbv6m_link_arg {
        command.env("RUSTFLAGS", "-C link-arg=-Tlink.x");
    }

    let status = command.status()?;

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
    // Collect every example whose system.yaml references this chip, then
    // prefer a canonical "smoke" example over richer sensor labs. The
    // strict-onboarding gate wants the simplest io-smoke per chip; sensor
    // labs (adxl345-*, ili9341-*, ssd1306-*, etc.) have their own coverage
    // and would otherwise mask the smoke-test signal here.

    let examples = root.join("examples");
    if !examples.exists() {
        return None;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(examples).ok()? {
        let entry = entry.ok()?;
        if entry.path().is_dir() {
            let system_yaml = entry.path().join("system.yaml");
            if system_yaml.exists() {
                let content = fs::read_to_string(&system_yaml).unwrap_or_default();
                if content.contains(&format!("chips/{}.yaml", chip_name))
                    || content.contains(&format!("chips/{}", chip_name))
                {
                    candidates.push(entry.path());
                }
            }
        }
    }

    if candidates.is_empty() {
        return None;
    }

    // Preferred canonical names (in order): demo-blinky for STM32F1, the
    // HIL displacement showcase for H5, then any *-blinky/hello-world
    // before sensor labs (whose io-smokes need richer external state).
    let preferred_substrings = [
        "demo-blinky",
        "hil-displacement-showcase",
        "blinky",
        "hello-world",
        "rp2040-pio-onboarding",
    ];
    for needle in &preferred_substrings {
        if let Some(pick) = candidates.iter().find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.contains(needle))
                .unwrap_or(false)
        }) {
            return Some(pick.clone());
        }
    }

    // No preferred match — sort by directory name so results are
    // deterministic across machines (read_dir isn't ordered on Linux).
    candidates.sort();
    candidates.into_iter().next()
}
