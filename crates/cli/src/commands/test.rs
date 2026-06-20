// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired test` subcommand: run the Tier-1 protocol suite.

use crate::*;
use labwired_config::{EnvTestScript, TestAssertion};

/// Run a multi-node environment test described by an `EnvTestScript`.
///
/// Resolves `inputs.env` relative to the script file, builds a `World` from the
/// `EnvironmentManifest`, steps all machines up to `limits.max_steps`, then
/// evaluates `memory_value` assertions per-node.
///
/// # Supported assertions
/// Only `memory_value` assertions are supported. `uart_contains` and `uart_regex`
/// return `EXIT_CONFIG_ERROR` with the message
/// "per-node uart_contains not yet supported in multi-node env scripts".
///
/// # Node binding
/// Each `memory_value` assertion MUST carry `node: <id>`. A missing or unknown
/// node name is `EXIT_CONFIG_ERROR`.
///
/// # Exit codes
/// - `EXIT_PASS` (0): all assertions matched.
/// - `EXIT_ASSERT_FAIL` (1): one or more assertions failed.
/// - `EXIT_CONFIG_ERROR` (2): script/env configuration problem.
fn run_env_test(script_path: &std::path::Path, script: EnvTestScript) -> ExitCode {
    // Validate assertions — only memory_value is supported in env scripts.
    for assertion in &script.assertions {
        match assertion {
            TestAssertion::UartContains(_) | TestAssertion::UartRegex(_) => {
                let msg = "per-node uart_contains not yet supported in multi-node env scripts";
                error!("{}", msg);
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
            TestAssertion::ExpectedStopReason(_) => {
                let msg =
                    "expected_stop_reason not supported in multi-node env scripts";
                error!("{}", msg);
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
            TestAssertion::MemoryValue(mv) => {
                if mv.memory_value.node.is_none() {
                    let msg = format!(
                        "memory_value assertion at address {:#x} is missing required 'node:' field in env script",
                        mv.memory_value.address
                    );
                    error!("{}", msg);
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            }
        }
    }

    // Resolve env manifest path relative to the script file.
    let env_path = resolve_script_path(script_path, &script.inputs.env);

    let env_manifest = match labwired_config::EnvironmentManifest::from_file(&env_path) {
        Ok(m) => m,
        Err(e) => {
            error!("Failed to load env manifest {:?}: {:#}", env_path, e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let root_dir = env_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    let mut world = match labwired_core::world::World::from_manifest(env_manifest, root_dir) {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to build world from env manifest: {:#}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Step all machines up to max_steps.
    let max_steps = script.limits.max_steps;
    for _ in 0..max_steps {
        let results = world.step_all();
        // On any machine error, treat as a non-fatal step fault (machines may halt).
        for (node_id, result) in &results {
            if let Err(e) = result {
                tracing::debug!("node '{}' step error: {:?}", node_id, e);
            }
        }
    }

    // Evaluate memory_value assertions.
    let mut all_passed = true;
    for assertion in &script.assertions {
        let TestAssertion::MemoryValue(mv) = assertion else {
            continue;
        };
        // node presence already validated above
        let node_id = mv.memory_value.node.as_deref().unwrap();

        let machine = match world.machines.get(node_id) {
            Some(m) => m,
            None => {
                error!(
                    "memory_value assertion references unknown node '{}' (not in env manifest)",
                    node_id
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };

        // Read the value using little-endian byte assembly from MachineTrait::read_u8.
        // `size` accepts bytes (1/2/4) or bits (8/16/32); defaults to 4 bytes (32-bit).
        let size_field = mv.memory_value.size.unwrap_or(32);
        let byte_count: u64 = match size_field {
            1 | 8 => 1,
            2 | 16 => 2,
            4 | 32 => 4,
            other => {
                error!(
                    "Unsupported memory assertion size: {} — use 1/2/4 (bytes) or 8/16/32 (bits)",
                    other
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };

        let addr = mv.memory_value.address;
        let mut raw: u64 = 0;
        for i in 0..byte_count {
            match machine.read_u8(addr + i) {
                Ok(b) => raw |= (b as u64) << (i * 8),
                Err(e) => {
                    error!(
                        "node '{}': failed to read address {:#x}+{}: {:?}",
                        node_id, addr, i, e
                    );
                    all_passed = false;
                    break;
                }
            }
        }

        let mask: u64 = mv.memory_value.mask.unwrap_or(match byte_count {
            1 => 0xFF,
            2 => 0xFFFF,
            4 => 0xFFFFFFFF,
            _ => u64::MAX,
        });
        let expected = mv.memory_value.expected_value & mask;
        let actual = raw & mask;

        if actual != expected {
            error!(
                "node '{}': memory assertion FAILED at {:#x} (size {}): expected {:#x}, got {:#x} (mask {:#x})",
                node_id, addr, size_field, expected, actual, mask
            );
            all_passed = false;
        } else {
            tracing::debug!(
                "node '{}': memory assertion PASSED at {:#x}: {:#x}",
                node_id,
                addr,
                actual
            );
        }
    }

    if all_passed {
        ExitCode::from(EXIT_PASS)
    } else {
        ExitCode::from(EXIT_ASSERT_FAIL)
    }
}

pub(crate) fn run_test(args: TestArgs) -> ExitCode {
    // ── API key validation (Pro tier gate) ──────────────────────────────
    // If LABWIRED_API_KEY is set and --no-key is not passed, validate before
    // starting the simulation so we fail fast with a clear message.
    let api_key_opt: Option<String> = if args.no_key {
        None
    } else {
        std::env::var("LABWIRED_API_KEY").ok()
    };

    let run_start = std::time::Instant::now();

    if let Some(ref key) = api_key_opt {
        match api_client::validate_key(key) {
            api_client::ValidateOutcome::Valid {
                workspace_id,
                plan,
                cycles_quota,
                cycles_used_mtd,
            } => {
                info!(
                    "LabWired Pro — workspace={} plan={} cycles_used={}/{} this month",
                    workspace_id, plan, cycles_used_mtd, cycles_quota
                );
            }
            api_client::ValidateOutcome::Invalid => {
                eprintln!(
                    "❌ LABWIRED_API_KEY is invalid. Check your dashboard or unset to use the free tier."
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
            api_client::ValidateOutcome::QuotaExceeded => {
                // Quota exhaustion is non-fatal — the simulation still runs locally.
                // Metering is skipped; billing resets at the next cycle boundary.
                // This matches the NetworkError fall-through: CI must not be blocked
                // by a billing state.
                tracing::warn!(
                    "LabWired API: monthly cycle quota exceeded; continuing simulation locally"
                );
            }
            api_client::ValidateOutcome::NetworkError(e) => {
                // Network errors are non-fatal — fall through to run in free-tier mode
                // to avoid blocking CI when the API is temporarily unreachable.
                tracing::warn!(
                    "LabWired API unreachable ({}); continuing in free-tier mode",
                    e
                );
            }
        }
    }

    let loaded = match load_test_script(&args.script) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("{:#}", e);
            error!("{}", msg);
            write_config_error_outputs(&args, None, args.system.as_ref(), None, None, msg);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // ── Multi-node env dispatch (early return) ───────────────────────────
    // Env scripts bypass the single-node firmware/system machinery entirely.
    if let LoadedTestScript::Env(env_script) = loaded {
        return run_env_test(&args.script, env_script);
    }

    let (
        script_firmware,
        script_system,
        script_max_steps,
        script_max_cycles,
        script_max_uart_bytes,
        script_no_progress_steps,
        script_wall_time_ms,
        script_max_vcd_bytes,
        assertions,
    ) = match loaded {
        LoadedTestScript::V1_0(script) => (
            Some(script.inputs.firmware),
            script.inputs.system,
            script.limits.max_steps,
            script.limits.max_cycles,
            script.limits.max_uart_bytes,
            script.limits.no_progress_steps,
            script.limits.wall_time_ms,
            script.limits.max_vcd_bytes,
            script.assertions,
        ),
        LoadedTestScript::LegacyV1(script) => {
            tracing::warn!(
                "Deprecated test script format detected (schema_version: 1). Please migrate to schema_version: \"1.0\" with inputs/limits nesting."
            );
            (
                script.firmware,
                script.system,
                script.max_steps,
                None,
                None,
                None,
                script.wall_time_ms,
                None,
                script.assertions,
            )
        }
        // Env variant is dispatched and returned early above; this arm is unreachable.
        LoadedTestScript::Env(_) => unreachable!("Env scripts return early before this match"),
    };

    let max_steps = args.max_steps.unwrap_or(script_max_steps);
    let max_cycles = args.max_cycles.or(script_max_cycles);
    let max_uart_bytes = args.max_uart_bytes.or(script_max_uart_bytes);
    let max_vcd_bytes = args.max_vcd_bytes.or(script_max_vcd_bytes);
    let detect_stuck = args.detect_stuck.or(script_no_progress_steps);
    let resolved_limits = TestLimits {
        max_steps,
        max_cycles,
        max_uart_bytes,
        no_progress_steps: detect_stuck,
        wall_time_ms: script_wall_time_ms,
        max_vcd_bytes,
    };

    // Guard against accidentally huge runs from CI misconfiguration.
    const MAX_ALLOWED_STEPS: u64 = 50_000_000;
    if max_steps > MAX_ALLOWED_STEPS {
        let msg = format!(
            "max_steps {} exceeds MAX_ALLOWED_STEPS {}",
            max_steps, MAX_ALLOWED_STEPS
        );
        error!("{}", msg);
        write_config_error_outputs(
            &args,
            None,
            args.system.as_ref(),
            None,
            Some(&resolved_limits),
            msg,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let firmware_path = match args.firmware.clone() {
        Some(p) => p,
        None => match script_firmware
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| resolve_script_path(&args.script, s))
        {
            Some(p) => p,
            None => {
                let msg =
                    "Missing firmware path (provide --firmware or set inputs.firmware in script)"
                        .to_string();
                error!("{}", msg);
                write_config_error_outputs(
                    &args,
                    None,
                    args.system.as_ref(),
                    None,
                    Some(&resolved_limits),
                    msg,
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        },
    };

    let system_path = args.system.clone().or_else(|| {
        script_system
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| resolve_script_path(&args.script, s))
    });

    let firmware_bytes = match std::fs::read(&firmware_path) {
        Ok(b) => b,
        Err(e) => {
            let msg = format!("Failed to read firmware {:?}: {}", firmware_path, e);
            error!("{}", msg);
            write_config_error_outputs(
                &args,
                Some(&firmware_path),
                system_path.as_ref(),
                None,
                Some(&resolved_limits),
                msg,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // For Xtensa/ESP32 system manifests, `SystemBus::from_config` (called
    // inside `build_system_bus`) will fail: it tries to attach external devices
    // (e.g. the SSD1680 e-paper panel) to `spi3`, but `spi3` is not in the
    // chip YAML — it is installed in code by `configure_xtensa_esp32`. Detect
    // the Xtensa arch early by parsing the manifest once, and take the dedicated
    // `build_esp32_system_from_manifest` path that calls configure + attach
    // together, before falling through to `build_system_bus` for all other
    // architectures. The parsed manifest is reused so the file is read only once.
    let esp32_manifest: Option<labwired_config::SystemManifest> =
        if let Some(sys_path) = system_path.as_deref() {
            labwired_config::SystemManifest::from_file(sys_path)
                .ok()
                .filter(|m| {
                    let chip_path = sys_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join(&m.chip);
                    labwired_config::ChipDescriptor::from_file(&chip_path)
                        .map(|c| c.arch == labwired_config::Arch::Xtensa)
                        .unwrap_or(false)
                })
        } else {
            None
        };
    let is_xtensa = esp32_manifest.is_some();

    // For Xtensa, short-circuit: build bus + CPU together via build_esp32_system_from_manifest.
    if is_xtensa {
        if let (Some(sys_path), Some(manifest)) = (system_path.as_ref(), esp32_manifest.as_ref()) {
            let uart_tx = Arc::new(Mutex::new(Vec::new()));
            let (mut esp_bus, esp_cpu) =
                match labwired_core::system::builder::build_esp32_system_from_manifest(
                    manifest, sys_path,
                ) {
                    Ok(pair) => pair,
                    Err(e) => {
                        let msg = format!("{:#}", e);
                        error!("{}", msg);
                        write_config_error_outputs(
                            &args,
                            Some(&firmware_path),
                            system_path.as_ref(),
                            Some(&firmware_bytes),
                            Some(&resolved_limits),
                            msg,
                        );
                        return ExitCode::from(EXIT_CONFIG_ERROR);
                    }
                };
            esp_bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);

            let program = match labwired_loader::load_elf(&firmware_path) {
                Ok(program) => program,
                Err(e) => {
                    let msg = format!("{:#}", e);
                    error!("{}", msg);
                    write_config_error_outputs(
                        &args,
                        Some(&firmware_path),
                        system_path.as_ref(),
                        Some(&firmware_bytes),
                        Some(&resolved_limits),
                        msg,
                    );
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            };

            let metrics = std::sync::Arc::new(labwired_core::metrics::PerformanceMetrics::new());
            let mut machine = labwired_core::Machine::new(esp_cpu, esp_bus);
            machine.observers.push(metrics.clone());
            if let Err(e) = machine.load_firmware(&program) {
                return handle_load_error(
                    &args,
                    &metrics,
                    &resolved_limits,
                    &firmware_bytes,
                    &uart_tx,
                    &machine.cpu,
                    &firmware_path,
                    system_path.as_ref(),
                    e,
                );
            }
            // ESP32 manifest path: skip BROM emulation and jump directly to
            // the ELF entry point — matches the wasm/playground path
            // (`new_from_config_xtensa_esp32`) and the e2e test
            // (`e2e_esp32_epaper.rs`). The BROM reset vector (0x4000_0400)
            // is fine for firmware compiled to boot from BROM, but playground
            // ELFs are pre-linked to start at the app entry.
            //
            // Seed SP to the top of DRAM (0x3FFE_0000): Arduino-ESP32 firmware
            // (call_start_cpu0) expects BROM to have placed SP here before
            // jumping to the app entry. We skip BROM, so do it ourselves —
            // matching `install_esp32_arduino_quirks` in the WASM path.
            // Native Xtensa firmware that sets its own SP will overwrite this
            // immediately.
            // CHEAT(SKIP): bypasses the boot ROM and hand-seeds PC/SP — real: the
            // CPU resets at 0x4000_0400 and the mapped BROM sets up SP and jumps
            // to the app. See FIDELITY.md §C.
            machine.cpu.set_pc(program.entry_point as u32);
            machine.cpu.set_sp(0x3FFE_0000);
            let exit_code = execute_test_loop(
                &args,
                &mut machine,
                &resolved_limits,
                &assertions,
                &firmware_bytes,
                &uart_tx,
                &metrics,
                &firmware_path,
                system_path.as_ref(),
            );
            // Device-block render readout. Surfaces the attached panel block's
            // REAL render state — refresh_gen AND black-plane ink — so a generic
            // verify (e.g. proto.cat's device loop) can judge whether the
            // device-block actually PAINTED, not merely refreshed. A refresh with
            // a blank plane is a false positive (the DC-latch class of bug; see
            // FIDELITY.md §E2). Emitted to stderr alongside the boot logs.
            {
                use labwired_core::peripherals::components::{
                    Ssd1680Tricolor290, Uc8151dTricolor290,
                };
                use labwired_core::peripherals::esp32::spi::Esp32Spi;
                if let Some(idx) = machine.bus.find_peripheral_index_by_name("spi3") {
                    if let Some(any) = machine.bus.peripherals[idx].dev.as_any() {
                        if let Some(spi3) = any.downcast_ref::<Esp32Spi>() {
                            for dev in &spi3.attached_devices {
                                let Some(a) = dev.as_any() else { continue };
                                if let Some(p) = a.downcast_ref::<Ssd1680Tricolor290>() {
                                    let ink =
                                        p.black_plane().iter().filter(|&&b| b != 0xFF).count();
                                    eprintln!(
                                        "[device-block] ssd1680_tricolor_290 refresh_gen={} black_ink={}",
                                        p.refresh_generation(),
                                        ink
                                    );
                                } else if let Some(p) = a.downcast_ref::<Uc8151dTricolor290>() {
                                    let ink =
                                        p.black_plane().iter().filter(|&&b| b != 0xFF).count();
                                    eprintln!(
                                        "[device-block] uc8151d_tricolor_290 refresh_gen={} black_ink={}",
                                        p.refresh_generation(),
                                        ink
                                    );
                                }
                            }
                        }
                    }
                }
            }
            if let Some(ref key) = api_key_opt {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(&firmware_bytes);
                let firmware_hash = format!("{:x}", hasher.finalize());
                let duration_ms = run_start.elapsed().as_millis() as u64;
                let cycles = metrics.get_cycles();
                let exit_val: i32 = if exit_code == ExitCode::from(EXIT_PASS) {
                    0
                } else if exit_code == ExitCode::from(EXIT_ASSERT_FAIL) {
                    1
                } else if exit_code == ExitCode::from(EXIT_RUNTIME_ERROR) {
                    3
                } else {
                    2
                };
                api_client::record_run(key, &firmware_hash, cycles, duration_ms, exit_val);
            }
            return exit_code;
        }
    }

    let mut bus = match labwired_core::system::builder::build_system_bus(system_path.as_deref()) {
        Ok(bus) => bus,
        Err(e) => {
            let msg = format!("{:#}", e);
            error!("{}", msg);
            write_config_error_outputs(
                &args,
                Some(&firmware_path),
                system_path.as_ref(),
                Some(&firmware_bytes),
                Some(&resolved_limits),
                msg,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let uart_tx = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);

    let program = match labwired_loader::load_elf(&firmware_path) {
        Ok(program) => program,
        Err(e) => {
            let msg = format!("{:#}", e);
            error!("{}", msg);
            write_config_error_outputs(
                &args,
                Some(&firmware_path),
                system_path.as_ref(),
                Some(&firmware_bytes),
                Some(&resolved_limits),
                msg,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let metrics = std::sync::Arc::new(labwired_core::metrics::PerformanceMetrics::new());

    macro_rules! setup_and_run {
        ($cpu:expr) => {{
            let mut machine = labwired_core::Machine::new($cpu, bus);
            machine.observers.push(metrics.clone());
            if let Err(e) = machine.load_firmware(&program) {
                return handle_load_error(
                    &args,
                    &metrics,
                    &resolved_limits,
                    &firmware_bytes,
                    &uart_tx,
                    &machine.cpu,
                    &firmware_path,
                    system_path.as_ref(),
                    e,
                );
            }
            execute_test_loop(
                &args,
                &mut machine,
                &resolved_limits,
                &assertions,
                &firmware_bytes,
                &uart_tx,
                &metrics,
                &firmware_path,
                system_path.as_ref(),
            )
        }};
    }

    let exit_code = match program.arch {
        labwired_core::Arch::Arm => {
            let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            setup_and_run!(cpu)
        }
        labwired_core::Arch::RiscV => {
            let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
            setup_and_run!(cpu)
        }
        labwired_core::Arch::XtensaLx7 => {
            // No system manifest present: plain configure path (no external devices).
            let cpu = labwired_core::system::xtensa::configure_xtensa(&mut bus);
            setup_and_run!(cpu)
        }
        _ => {
            let msg = format!("Unsupported architecture: {:?}", program.arch);
            error!("{}", msg);
            write_config_error_outputs(
                &args,
                Some(&firmware_path),
                system_path.as_ref(),
                Some(&firmware_bytes),
                Some(&resolved_limits),
                msg,
            );
            ExitCode::from(EXIT_CONFIG_ERROR)
        }
    };

    // ── Best-effort run metering (Pro tier) ──────────────────────────────
    if let Some(ref key) = api_key_opt {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&firmware_bytes);
        let firmware_hash = format!("{:x}", hasher.finalize());

        let duration_ms = run_start.elapsed().as_millis() as u64;
        let cycles = metrics.get_cycles();
        // Encode exit code as an integer for the API payload.
        // EXIT_PASS=0, EXIT_ASSERT_FAIL=1, EXIT_CONFIG_ERROR=2, EXIT_RUNTIME_ERROR=3
        let exit_val: i32 = if exit_code == ExitCode::from(EXIT_PASS) {
            0
        } else if exit_code == ExitCode::from(EXIT_ASSERT_FAIL) {
            1
        } else if exit_code == ExitCode::from(EXIT_RUNTIME_ERROR) {
            3
        } else {
            2
        };

        // best-effort — don't block on failure
        api_client::record_run(key, &firmware_hash, cycles, duration_ms, exit_val);
    }

    exit_code
}
