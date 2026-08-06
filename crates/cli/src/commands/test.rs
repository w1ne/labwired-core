// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired test` subcommand: run the Tier-1 protocol suite.

use crate::*;
use tracing::warn;

/// Opt-in Arduino-matrix speed path (`LABWIRED_MATRIX_SPEED=1`).
///
/// Enables idle/delay fast-forward only (requires a CLI build with
/// `--features event-scheduler`). Tick-interval widening is intentionally
/// **not** applied here: under the event-scheduler feature, wide ticks have
/// regressed ESP classic/S3/C3 FreeRTOS labs before, and the matrix already
/// gets most of its wall-time win from shorter L2 delays + Waiti park.
/// Default CLI / green labs are unchanged when the env var is unset.
fn apply_matrix_speed_opts<C: labwired_core::Cpu>(machine: &mut labwired_core::Machine<C>) {
    if std::env::var("LABWIRED_MATRIX_SPEED").as_deref() != Ok("1") {
        return;
    }
    machine.config.idle_fast_forward_enabled = true;
    eprintln!(
        "labwired-cli test: LABWIRED_MATRIX_SPEED=1 (idle_ff=on, event_scheduler={})",
        cfg!(feature = "event-scheduler")
    );
}

/// Apply the script's faults to the built bus before the run, logging any that
/// could not be applied. Returns the provisional evidence; runtime-observed
/// outcomes (and the require_fault_fired gate) are finalised after the run in
/// execute_test_loop.
fn handle_faults(
    bus: &mut labwired_core::bus::SystemBus,
    faults: &[labwired_config::FaultSpec],
) -> Vec<labwired_cli::faults::FaultEvidence> {
    if faults.is_empty() {
        return Vec::new();
    }
    let evidence = labwired_cli::faults::apply_faults(bus, faults);
    for e in &evidence {
        if let Some(err) = &e.error {
            error!(
                "fault '{}' ({}) could not be applied: {}",
                e.id, e.kind, err
            );
        }
    }
    evidence
}

/// Encode the public CLI exit contract for best-effort API metering.
fn metering_exit_status(exit_code: &ExitCode) -> i32 {
    if *exit_code == ExitCode::from(EXIT_PASS) {
        0
    } else if *exit_code == ExitCode::from(EXIT_ASSERT_FAIL) {
        1
    } else if *exit_code == ExitCode::from(EXIT_RUNTIME_ERROR) {
        3
    } else {
        2
    }
}

use super::esp32_boot_state::resolve_esp_partitions_bin;

/// True when the system manifest at `sys_path` targets the ESP32-C3 (the only
/// chip family with an ELF-less faithful rom-boot machine). Reads the manifest
/// and its referenced chip descriptor; any load failure → false (fall back to
/// requiring firmware). Mirrors the C3 detection used on the fast-boot path.
fn system_is_esp32c3(
    system: &labwired_config::ResolvedSystem,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> bool {
    system
        .chip_with_plugins(&crate::plugin_chip_yaml(plugins))
        .map(|c| c.name == "esp32c3")
        .unwrap_or(false)
}

/// Faithful ESP32-C3 (RISC-V) rom-boot with NO debug ELF. The flash image
/// (LABWIRED_ESP32C3_FLASH) IS the program the real mask ROM loads, so no ELF is
/// needed — the hosted compile deliberately ships flash images but no
/// firmware_ref for rom-boot chips (a multi-MB ELF overflows the D1 blob row).
///
/// This mirrors the ELF `Arch::RiscV` rom-boot / resume arms in `run_test`, but
/// builds the bus + machine directly instead of dispatching on `program.arch`
/// (there is no ELF to read the arch from). Symbol-dependent diagnostics degrade
/// gracefully: `execute_test_loop` is handed an empty firmware slice, so
/// `resolve_symbol_in_elf` returns `None` and `--capture-app-entry` falls back
/// to the XIP app-window detector. Snapshot-invalid resume errors deliberately
/// write NO result.json so the builder falls back to a cold `--rom-boot` (the
/// same fallback contract the ELF resume arm relies on).
#[allow(clippy::too_many_arguments)]
fn run_c3_rom_boot_no_elf(
    args: &TestArgs,
    resolved_limits: &TestLimits,
    system_path: Option<&std::path::PathBuf>,
    system: Option<&labwired_config::ResolvedSystem>,
    assertions: &[TestAssertion],
    faults: &[labwired_config::FaultSpec],
    require_fault_fired: bool,
    stimuli: &[labwired_config::StimulusSpec],
    uart_injections: &[labwired_config::UartInjectionSpec],
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    // Build the from_config bus (peripherals + external devices) exactly as the
    // ELF rom-boot path does before build_c3_rom_boot_machine.
    let mut bus =
        match labwired_core::system::builder::build_system_bus_with_plugins(system, plugins) {
            Ok(bus) => bus,
            Err(e) => {
                let msg = format!("{:#}", e);
                error!("{}", msg);
                write_config_error_outputs(
                    args,
                    None,
                    system_path,
                    None,
                    Some(resolved_limits),
                    msg,
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };

    // Load the manifest once: it drives both the UART sink selection (debug_uart)
    // and — the universal WiFi adapter — the `wifi_ap` attach below.
    let manifest_opt = system.map(|s| s.manifest.clone());
    let debug_uart = manifest_opt.as_ref().and_then(|m| m.debug_uart.clone());
    // Seed the station eFuse MAC when a `wifi_ap` is present so the C3 stays
    // associated (a zero eFuse MAC associates but drops); None otherwise keeps
    // non-WiFi rom-boot byte-identical.
    let wifi_station_mac = manifest_opt
        .as_ref()
        .and_then(labwired_core::system::wifi::wifi_ap_station_mac);

    // UART capture, mirroring the main flow: honour debug_uart, else all UARTs,
    // plus the IO-Link master log sink.
    let uart_tx = Arc::new(Mutex::new(Vec::new()));
    if let Some(debug_uart) = debug_uart.as_deref() {
        if !bus.attach_uart_tx_sink_named(debug_uart, uart_tx.clone(), !args.no_uart_stdout) {
            warn!(
                "debug_uart '{}' did not resolve to a UART peripheral; falling back to all UARTs",
                debug_uart
            );
            bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);
        }
    } else {
        bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);
    }
    bus.attach_iolink_master_log_sink(uart_tx.clone());

    // Resume from a captured app-entry snapshot when requested (the cache-hit
    // path), else cold rom-boot. The ELF is never needed: the snapshot self-key
    // is keyed on the chip + flash SHA-256, not the ELF.
    let mut machine = if let Some(snap_path) = &args.resume_snapshot {
        let snap_bytes = match std::fs::read(snap_path) {
            Ok(b) => b,
            Err(e) => {
                let msg = format!("cannot read resume snapshot {snap_path:?}: {e}");
                error!("{}", msg);
                write_config_error_outputs(
                    args,
                    None,
                    system_path,
                    None,
                    Some(resolved_limits),
                    msg,
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };
        let snap = match labwired_core::runtime_snapshot::MachineRuntimeSnapshot::from_bytes(
            &snap_bytes,
        ) {
            Ok(s) => s,
            Err(e) => {
                // Corrupt/version-mismatched blob → write NO result.json so the
                // caller cold-boots and refreshes the cache.
                error!("invalid resume snapshot {snap_path:?}: {e}; cold-boot required");
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };
        let (chip, fw_sha) = match crate::rom_boot_flash_self_key() {
            Some(v) => v,
            None => {
                let msg = "--resume-snapshot needs LABWIRED_ESP32C3_FLASH set (the same flash \
                           image the snapshot was captured against)"
                    .to_string();
                error!("{}", msg);
                write_config_error_outputs(
                    args,
                    None,
                    system_path,
                    None,
                    Some(resolved_limits),
                    msg,
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };
        if let Err(e) = snap.validate_self_key(chip, &fw_sha) {
            // Stale/foreign snapshot → write NO result.json (cold-boot fallback).
            error!("resume snapshot self-key mismatch ({e}); cold-boot required");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
        let mut machine = match crate::build_c3_rom_boot_machine(bus, wifi_station_mac) {
            Ok(m) => m,
            Err(code) => return code,
        };
        if let Err(e) = machine.apply_runtime_snapshot(&snap) {
            // Structurally incompatible snapshot → write NO result.json so the
            // caller cold-boots and refreshes the cache with a compatible capture.
            error!(
                "resume snapshot incompatible with this machine ({e}); \
                 cold-boot required (stale/foreign snapshot)"
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
        eprintln!(
            "labwired-riscv: resumed from app-entry snapshot {snap_path:?} (chip {chip}); \
             mask-ROM replay skipped (ELF-less)"
        );
        machine
    } else {
        // Cold faithful rom-boot. --capture-app-entry (the cache-miss path) is
        // handled inside execute_test_loop; with no ELF the app-entry PC falls
        // back to the XIP app-window detector.
        match crate::build_c3_rom_boot_machine(bus, wifi_station_mac) {
            Ok(m) => m,
            Err(code) => return code,
        }
    };

    // Universal WiFi adapter: if the diagram carries a `wifi_ap`, attach every
    // real WiFi MAC to a per-lab virtual-WiFi medium so the device associates →
    // DHCP → HTTP under the hosted `test` path exactly like the CLI solo path and
    // the browser. No-op when there is no `wifi_ap`.
    if let Some(manifest) = manifest_opt.as_ref() {
        labwired_core::system::wifi::attach_configured_wifi_ap(&mut machine.bus, manifest);
    }

    let metrics = std::sync::Arc::new(labwired_core::metrics::PerformanceMetrics::new());
    apply_matrix_speed_opts(&mut machine);
    machine.observers.push(metrics.clone());
    let fault_evidence = handle_faults(&mut machine.bus, faults);

    // No ELF: empty firmware bytes degrade symbol/hash diagnostics gracefully; a
    // placeholder path is recorded as config.firmware in result.json.
    let placeholder = std::path::PathBuf::from("<flash-image>");
    execute_test_loop(
        args,
        &mut machine,
        resolved_limits,
        assertions,
        &[],
        &uart_tx,
        &metrics,
        &placeholder,
        system_path,
        faults,
        require_fault_fired,
        fault_evidence,
        stimuli,
        uart_injections,
        // rom-boot is never JIT-eligible (it forces cycle-accurate stepping).
        false,
    )
}

pub(crate) fn run_test(
    args: TestArgs,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
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
                eprintln!(
                    "⚠️  Monthly cycle quota exceeded. Upgrade your plan or wait until next billing period."
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
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
            if super::environment_test::try_write_load_error_outputs(&args, msg.clone()) {
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
            write_config_error_outputs(&args, None, args.system.as_ref(), None, None, msg);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // `inputs.chip` names a built-in chip for firmware with nothing wired to it.
    // It is read before the destructuring match because only the 1.0 schema has it.
    let script_chip = match &loaded {
        LoadedTestScript::V1_0(script) => script.inputs.chip.clone(),
        _ => None,
    };

    // Read before the destructuring match for the same reason as `chip`: only
    // the 1.0 schema carries it, and the machine build needs it much later.
    let script_profile = match &loaded {
        LoadedTestScript::V1_0(script) => script.inputs.profile.clone(),
        _ => None,
    };

    let (
        script_firmware,
        script_system,
        script_max_steps,
        script_max_cycles,
        script_max_uart_bytes,
        script_no_progress_steps,
        script_wall_time_ms,
        script_max_vcd_bytes,
        script_stop_when_assertions_pass,
        script_stop_when_assertions_pass_settle_steps,
        script_stop_when_assertions_pass_min_steps,
        assertions,
        faults,
        verdict,
        stimuli,
        uart_injections,
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
            script.limits.stop_when_assertions_pass,
            script.limits.stop_when_assertions_pass_settle_steps,
            script.limits.stop_when_assertions_pass_min_steps,
            script.assertions,
            script.faults,
            script.verdict,
            script.stimuli,
            script.uart_injections,
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
                false,
                100_000,
                0,
                script.assertions,
                Vec::new(),
                None,
                Vec::new(),
                Vec::new(),
            )
        }
        LoadedTestScript::Env(script) => {
            let outcome = super::environment_test::run_environment_test(&args, script, plugins);
            if let Some(ref key) = api_key_opt {
                let duration_ms = run_start.elapsed().as_millis() as u64;
                api_client::record_run(
                    key,
                    &outcome.world_firmware_hash,
                    outcome.cycles,
                    duration_ms,
                    metering_exit_status(&outcome.exit_code),
                );
            }
            return outcome.exit_code;
        }
    };

    // Fault injection (schema_version 1.1): the verdict's safe_when entries are
    // evaluated as ordinary assertions; require_fault_fired gates the run on the
    // faults actually taking effect.
    let require_fault_fired = verdict
        .as_ref()
        .map(|v| v.require_fault_fired)
        .unwrap_or(false);
    let mut assertions = assertions;
    if let Some(v) = &verdict {
        assertions.extend(v.safe_when.iter().cloned());
    }

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
        stop_when_assertions_pass: script_stop_when_assertions_pass,
        stop_when_assertions_pass_settle_steps: script_stop_when_assertions_pass_settle_steps,
        stop_when_assertions_pass_min_steps: script_stop_when_assertions_pass_min_steps,
    };

    // Guard against accidentally huge runs from CI misconfiguration. The
    // faithful --rom-boot path spends ~150M steps in the real mask ROM +
    // 2nd-stage bootloader BEFORE the app runs a single instruction, so it
    // gets a proportionally higher ceiling (wall-clock caps still apply).
    const MAX_ALLOWED_STEPS: u64 = 50_000_000;
    const MAX_ALLOWED_STEPS_ROM_BOOT: u64 = 500_000_000;
    // An Arduino fast boot running a POLLING sketch is the most step-hungry
    // shape we support: bring-up alone is tens of millions, and every poll
    // costs four I2C sensor reads with a ready-spin. Ryan's rig reaches only
    // ~126M cycles inside the 500M rom-boot ceiling — one status line, far too
    // early to assert on anything a stimulus caused. The ceiling exists to
    // catch CI misconfiguration, not to cap a legitimately long run, and the
    // wall-clock caps still bound a runaway sim.
    const MAX_ALLOWED_STEPS_ARDUINO_FAST_BOOT: u64 = 4_000_000_000;
    // A run boots the real ROM (and needs the higher ceiling) not only when
    // --rom-boot is set, but whenever it captures/resumes an app-entry snapshot
    // OR a flash-image env is present: the compiled-source ESP32-C3/S3 path
    // supplies the merged flash image via LABWIRED_ESP32{C3,S3}_FLASH and boots
    // the ROM from it, yet did not always carry the --rom-boot flag — so it
    // wrongly got the 50M ceiling. `--capture-app-entry` and `--resume-snapshot`
    // are rom-boot runs too (same predicate the machine build uses). Key the
    // ceiling off EFFECTIVE rom-boot so headless device proving has the budget
    // it needs (acceptance markers still halt early; wall-clock caps still
    // bound runaway sims).
    let flash_env_present = |k: &str| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false);
    let rom_boot_effective = args.rom_boot
        || args.capture_app_entry.is_some()
        || args.resume_snapshot.is_some()
        || flash_env_present("LABWIRED_ESP32C3_FLASH")
        || flash_env_present("LABWIRED_ESP32S3_FLASH");
    // An Arduino-ESP32 fast boot spends tens of millions of steps in
    // initArduino + FreeRTOS bring-up before the sketch's setup() runs, and a
    // sketch that then POLLS (Ryan's bay-occupancy rig reads four sensors
    // through an I2C switch) needs many more before it has produced enough
    // output to assert on. At the 50M ceiling such a run stopped with ~47
    // bytes of UART and stop_reason=max_steps, which reads like a broken
    // firmware rather than a budget the runner refused to grant. Give it the
    // same headroom rom-boot already gets — acceptance markers still halt
    // early, and the wall-clock caps still bound a runaway sim.
    let arduino_fast_boot = script_profile.as_deref() == Some("arduino-esp32");
    let max_allowed_steps = if arduino_fast_boot {
        MAX_ALLOWED_STEPS_ARDUINO_FAST_BOOT
    } else if rom_boot_effective {
        MAX_ALLOWED_STEPS_ROM_BOOT
    } else {
        MAX_ALLOWED_STEPS
    };
    if max_steps > max_allowed_steps {
        let msg = format!(
            "max_steps {} exceeds MAX_ALLOWED_STEPS {}",
            max_steps, max_allowed_steps
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

    // system_path is resolved first: when no ELF is provided we must inspect the
    // manifest's chip to decide whether the ELF-less C3 rom-boot path applies.
    let system_path = args.system.clone().or_else(|| {
        script_system
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| resolve_script_path(&args.script, s))
    });

    // Resolve the system once, from either a manifest file or an `inputs.chip`
    // name. Every site below that needs the chip, the debug UART, or the wifi
    // AP reads it from here instead of re-parsing the manifest.
    let resolved_system = match (&system_path, &script_chip) {
        (Some(path), _) => match labwired_config::ResolvedSystem::from_manifest_file(path) {
            Ok(s) => Some(s),
            Err(e) => {
                let msg = format!("Failed to load system manifest {path:?}: {e:#}");
                error!("{}", msg);
                write_config_error_outputs(&args, None, Some(path), None, None, msg);
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        },
        (None, Some(chip)) => {
            match labwired_config::ResolvedSystem::from_builtin_chip_with_plugins(
                chip,
                &crate::plugin_chip_yaml(plugins),
            ) {
                Ok(s) => Some(s),
                Err(e) => {
                    let msg = format!("{e:#}");
                    error!("{}", msg);
                    write_config_error_outputs(&args, None, None, None, None, msg);
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            }
        }
        (None, None) => None,
    };

    // Resolve the firmware source. Normally an ELF is required (via --firmware or
    // inputs.firmware). The ONE exception is the faithful ESP32-C3 (RISC-V)
    // rom-boot path: the flash image (LABWIRED_ESP32C3_FLASH) is the program the
    // real mask ROM loads, so no debug ELF is needed — the hosted compile
    // deliberately withholds it (a multi-MB ELF overflows the D1 blob row →
    // SQLITE_TOOBIG). See run_c3_rom_boot_no_elf.
    let firmware_path_opt: Option<std::path::PathBuf> = match args.firmware.clone() {
        Some(p) => Some(p),
        None => script_firmware
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| resolve_script_path(&args.script, s)),
    };

    // ELF-less C3 rom-boot: no firmware given, --rom-boot (or a --resume-snapshot
    // cache-hit) requested, the flash image env pin is set, and the manifest's
    // chip is esp32c3. Every other missing-firmware case is still a config error,
    // and no other chip family has an ELF-less rom-boot machine.
    let no_elf_rom_boot = firmware_path_opt.is_none()
        && (args.rom_boot || args.resume_snapshot.is_some())
        && std::env::var("LABWIRED_ESP32C3_FLASH").is_ok()
        && resolved_system
            .as_ref()
            .map(|s| system_is_esp32c3(s, plugins))
            .unwrap_or(false);

    if no_elf_rom_boot {
        eprintln!(
            "labwired-cli test: no --firmware provided; faithful ESP32-C3 rom-boot from \
             LABWIRED_ESP32C3_FLASH (the flash image is the program; ELF-less)"
        );
        let exit_code = run_c3_rom_boot_no_elf(
            &args,
            &resolved_limits,
            system_path.as_ref(),
            resolved_system.as_ref(),
            &assertions,
            &faults,
            require_fault_fired,
            &stimuli,
            &uart_injections,
            plugins,
        );
        // Best-effort Pro-tier metering (no ELF → hash the empty program; the
        // no-key MCP path never meters). Mirrors the ELF paths' tail metering.
        if let Some(ref key) = api_key_opt {
            use sha2::{Digest, Sha256};
            let firmware_hash = format!("{:x}", Sha256::new().finalize());
            let duration_ms = run_start.elapsed().as_millis() as u64;
            api_client::record_run(
                key,
                &firmware_hash,
                0,
                duration_ms,
                metering_exit_status(&exit_code),
            );
        }
        return exit_code;
    }

    let firmware_path = match firmware_path_opt {
        Some(p) => p,
        None => {
            let msg = "Missing firmware path (provide --firmware or set inputs.firmware in script)"
                .to_string();
            error!("{}", msg);
            write_config_error_outputs(
                &args,
                None,
                system_path.as_ref(),
                None,
                Some(&resolved_limits),
                msg,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

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
    let esp32_manifest: Option<labwired_config::SystemManifest> = resolved_system
        .as_ref()
        .filter(|s| {
            s.chip_with_plugins(&crate::plugin_chip_yaml(plugins))
                .map(|c| c.arch == labwired_config::Arch::Xtensa)
                .unwrap_or(false)
        })
        .map(|s| s.manifest.clone());
    let is_xtensa = esp32_manifest.is_some();

    // For Xtensa, short-circuit: build bus + CPU together via build_esp32_system_from_manifest.
    if is_xtensa {
        // --resume-snapshot is wired for the C3 (RISC-V) rom-boot path only.
        // The S3 faithful machine has no load_firmware step and populates some
        // app regions via bootloader copies during the cold boot, so resuming
        // it needs cache re-derivation work that is deferred; fail loudly rather
        // than silently restore a partial state. (--capture-app-entry still
        // works on S3 — it flows through the generic execute_test_loop.)
        if args.resume_snapshot.is_some() {
            let msg = "--resume-snapshot is not yet supported for ESP32-S3 (Xtensa); \
                       cold-boot with --rom-boot instead"
                .to_string();
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
        if let (Some(sys_path), Some(manifest)) = (system_path.as_ref(), esp32_manifest.as_ref()) {
            let uart_tx = Arc::new(Mutex::new(Vec::new()));
            // Load the ELF up front. The classic-Xtensa path fast-boots it into
            // memory and jumps to its entry; the faithful S3 ROM-boot path uses
            // it only for symbol/diagnostic context (the flash image is the
            // program the real ROM loads).
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

            // Distinguish ESP32-S3 (Xtensa LX7) from classic ESP32 (LX6): both
            // parse to `Arch::Xtensa`, but only S3 has a faithful rom-boot
            // machine. `--rom-boot` on an S3 chip takes the real-ROM path;
            // classic ESP32 stays on the legacy fast-boot (its rom-boot is a
            // separate task).
            let is_esp32s3 = {
                let chip_dir = sys_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                labwired_config::ChipDescriptor::resolve_with(
                    &manifest.chip,
                    chip_dir,
                    &crate::plugin_chip_yaml(plugins),
                )
                .map(|c| c.name == "esp32s3")
                .unwrap_or(false)
            };

            let mut machine = if is_esp32s3 {
                // ── ESP32-S3 (LX7): S3 memmap + XIP, NOT classic ESP32 map ──
                // Classic `build_esp32_system_from_manifest` uses LX6 IRAM/DROM
                // bases — Arduino-S3 ELF segments (0x3C00_xxxx DROM, 0x3FC8_xxxx
                // DRAM, 0x4200_xxxx IROM, 0x4037_xxxx IRAM) never load.
                use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
                use labwired_core::system::xtensa::{
                    configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts,
                };
                if args.rom_boot {
                    // Faithful BROM path (mirrors `run`): needs flash image.
                    if std::env::var("LABWIRED_ESP32S3_FLASH").is_err() {
                        let msg =
                            "--rom-boot needs LABWIRED_ESP32S3_FLASH set (the firmware flash image)"
                                .to_string();
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
                    let mut bus = labwired_core::bus::SystemBus::new();
                    let opts = Esp32s3Opts {
                        real_reset_boot: true,
                        ..Esp32s3Opts::default()
                    };
                    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
                    // Wire matrix kits (INA219, OLED, …) the same way classic ESP32 does.
                    if let Err(e) = labwired_core::system::xtensa::attach_esp32_external_devices(
                        &mut bus, manifest,
                    ) {
                        let msg = format!("ESP32-S3 external_devices attach: {e:#}");
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
                    bus.refresh_peripheral_index();
                    if wiring.boot_mode != Esp32s3BootMode::Faithful {
                        let msg =
                            "--rom-boot needs the real ESP32-S3 boot ROM, but none was found. \
                             Install the ESP toolchain or set LABWIRED_ESP32S3_ROM_ELF \
                             (or pin LABWIRED_ESP32S3_ROM/_DROM)."
                                .to_string();
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
                    let mut cpu = wiring.cpu;
                    cpu.faithful_windows = true;
                    bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);
                    let mut machine = labwired_core::Machine::new(cpu, bus);
                    machine.observers.push(metrics.clone());
                    machine
                } else {
                    // Matrix / plain `labwired test`: fast-boot — configure S3
                    // map, load ELF into IRAM/DRAM + identity FlashXip, jump to
                    // app entry. Force harness ROM thunks: full faithful ROM
                    // busy-waits in unmodelled analog/cache/delay paths during
                    // Arduino `system_early_init`, while harness
                    // `ets_set_appcpu_boot_addr` + dual APP_CPU release the
                    // `s_cpu_up` spin (honest dual-core, not a firmware patch).
                    let mut bus = labwired_core::bus::SystemBus::new();
                    // Scoped: provision_rom_images checks this once.
                    let _fast = std::env::var_os("LABWIRED_ESP32S3_FASTBOOT");
                    std::env::set_var("LABWIRED_ESP32S3_FASTBOOT", "1");
                    // The matrix loads the app at the factory partition
                    // (`factory_flash_base` below) and seeds the flash MMU via
                    // `seed_factory_mmu_for_cache2phys` so `spi_flash_mmap` /
                    // `cache2phys` resolve — that requires the MMU-XIP window, so
                    // request it explicitly (FASTBOOT alone no longer implies it,
                    // to keep bare fast_boot fixtures on identity XIP).
                    let _mmu_xip = std::env::var_os("LABWIRED_ESP32S3_MMU_XIP");
                    std::env::set_var("LABWIRED_ESP32S3_MMU_XIP", "1");
                    let opts = Esp32s3Opts::default();
                    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
                    if _fast.is_none() {
                        std::env::remove_var("LABWIRED_ESP32S3_FASTBOOT");
                    }
                    if _mmu_xip.is_none() {
                        std::env::remove_var("LABWIRED_ESP32S3_MMU_XIP");
                    }
                    // Matrix L3 kits live in system.yaml external_devices — attach
                    // after the SoC bank is registered (classic path does this in
                    // build_esp32_system_from_manifest; S3 must do it here too).
                    if let Err(e) = labwired_core::system::xtensa::attach_esp32_external_devices(
                        &mut bus, manifest,
                    ) {
                        let msg = format!("ESP32-S3 external_devices attach: {e:#}");
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
                    // Debugger register NAMES come from the chip YAML even
                    // though the S3 bank above is programmatic; without this
                    // every `debug_schema:` the chip declares is inert. Never
                    // fatal — a debugger convenience must not fail a run.
                    let chip_dir = sys_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."));
                    if let Ok(chip) = labwired_config::ChipDescriptor::resolve_with(
                        &manifest.chip,
                        chip_dir,
                        &crate::plugin_chip_yaml(plugins),
                    ) {
                        bus.attach_debug_schemas(
                            &chip,
                            &labwired_core::system::builder::anchor_chip_path(manifest, chip_dir),
                        );
                    }
                    bus.refresh_peripheral_index();
                    // Seed partition table + app image magic into D-cache
                    // identity window (VA 0x3C00_0000 → dcache[off]).
                    {
                        if let Ok(mut d) = wiring.dcache_backing.lock() {
                            if let Some(p) =
                                resolve_esp_partitions_bin(std::path::Path::new(&firmware_path))
                            {
                                if let Ok(pt) = std::fs::read(&p) {
                                    let n = pt.len().min(0xC00);
                                    if d.len() >= 0x8000 + n {
                                        d[0x8000..0x8000 + n].copy_from_slice(&pt[..n]);
                                        eprintln!(
                                            "labwired-cli test: seeded S3 dcache partitions ({} bytes) from {}",
                                            n,
                                            p.display()
                                        );
                                    }
                                }
                            } else {
                                // No table → esp_partition's load_partitions finds
                                // no MD5 entry and calls panic_abort BEFORE the
                                // console is up, so the run looks like a silent
                                // hang (~86k steps, zero UART) with nothing to go
                                // on. Name the cause instead of leaving the
                                // caller to bisect a firmware image.
                                eprintln!(
                                    "labwired-cli test: warn: no partitions.bin beside {} — an \
                                     Arduino/ESP-IDF app will abort in esp_partition (\"No MD5 \
                                     found in partition table\") before printing anything. Place \
                                     the partition table (flash 0x8000) next to the ELF.",
                                    firmware_path.display()
                                );
                            }
                            // App magic 0xE9: identity used off 0x30000; factory
                            // MMU maps VA 0x3C03_0000 → phys page 4 (0x40000).
                            for off in [0x30000usize, 0x40000, 0x10000] {
                                if d.len() > off {
                                    d[off] = 0xE9;
                                }
                            }
                        }
                    }
                    bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);
                    // Arduino may also print via USB-Serial-JTAG.
                    {
                        use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
                        for p in bus.peripherals.iter_mut() {
                            if p.name == "usb_serial_jtag" {
                                if let Some(any) = p.dev.as_any_mut() {
                                    if let Some(jtag) = any.downcast_mut::<UsbSerialJtag>() {
                                        jtag.set_sink(Some(uart_tx.clone()), !args.no_uart_stdout);
                                    }
                                }
                            }
                        }
                    }
                    let mut pro_cpu = wiring.cpu;
                    if let Err(e) = fast_boot(
                        &firmware_bytes,
                        &mut bus,
                        &mut pro_cpu,
                        &BootOpts {
                            stack_top_fallback: 0x3FCD_FFF0,
                            icache_backing: Some(wiring.icache_backing.clone()),
                            dcache_backing: Some(wiring.dcache_backing.clone()),
                            factory_flash_base: Some(0x1_0000),
                        },
                    ) {
                        let msg = format!("ESP32-S3 fast_boot: {e:#}");
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
                    // Bootloader-equivalent MMU: factory app @ flash 0x10000 so
                    // `spi_flash_cache2phys` / OTA running-partition succeed
                    // (fast-boot never programs DR_REG_MMU_TABLE).
                    labwired_core::boot::esp32s3::seed_factory_mmu_for_cache2phys(
                        &mut bus, 4, // IROM ~0x22 KiB → 2 pages; pad
                        8, // DROM window pages used by .flash.rodata @ 0x3C03_xxxx
                    );
                    eprintln!(
                        "labwired-cli test: seeded S3 factory MMU for cache2phys (app0 @ 0x10000)"
                    );
                    // Post-BROM flash-attach state: the ROM's flash-attach fills
                    // `rom_spiflash_legacy_data->chip.chip_size`; fast-boot skips
                    // it, so `spi_flash_mmap` (partition-table load) rejects every
                    // mmap with ESP_ERR_INVALID_ARG (0x102) and `load_partitions`
                    // aborts. Seed the descriptor with the configured flash size.
                    labwired_core::boot::esp32s3::seed_esp32s3_rom_flashchip(
                        &mut bus,
                        4 * 1024 * 1024,
                    );
                    // Arduino dual-core: `system_early_init` calls
                    // `ets_set_appcpu_boot_addr(call_start_cpu1)` then spins on
                    // `s_cpu_up[0] & s_cpu_up[1]`. Without APP_CPU the wait is
                    // forever. Same model as classic: halted APP until PRO
                    // releases via the silicon boot path.
                    use labwired_core::cpu::xtensa_lx7::XtensaLx7;
                    use labwired_core::peripherals::esp_xtensa_common::rom_thunks;
                    // Dual-core handshake: PRO waits on both s_cpu_up[0]&[1]
                    // and s_cpu_inited[0]&[1] (2-byte arrays). ets_set_appcpu_boot_addr
                    // thunk marks these when APP is "released".
                    let mut app_flags = Vec::new();
                    for sym in ["s_cpu_up", "s_cpu_inited"] {
                        if let Some(a) =
                            labwired_loader::resolve_symbol_in_elf(&firmware_bytes, sym)
                        {
                            app_flags.push(a);
                            app_flags.push(a.wrapping_add(1));
                        }
                    }
                    if !app_flags.is_empty() {
                        eprintln!(
                            "labwired-cli test: S3 APP handshake flags @ {:#010x?}",
                            app_flags
                        );
                        rom_thunks::set_appcpu_up_flags(app_flags);
                    }
                    // Hybrid window preserve parks under FreeRTOS TCB via
                    // pxCurrentTCBs[core]. S3 BSS is not the classic address.
                    for sym in ["pxCurrentTCBs", "pxCurrentTCB"] {
                        if let Some(a) =
                            labwired_loader::resolve_symbol_in_elf(&firmware_bytes, sym)
                        {
                            rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(a)));
                            eprintln!(
                                "labwired-cli test: pxCurrentTCBs @0x{a:08x} (hybrid preserve key)"
                            );
                            break;
                        }
                    }
                    super::esp32_boot_state::install_xtensa_freertos_workarounds(
                        &mut bus,
                        &firmware_bytes,
                    );
                    // Real APP_CPU: start_cpu0 waits on s_system_inited[0]&[1];
                    // APP sets [1] in do_system_init_fn after s_resume_cores.
                    // ets_set_appcpu_boot_addr still raises s_cpu_up/s_cpu_inited
                    // via set_appcpu_up_flags (early PRO wait) and stashes
                    // APPCPU_BOOT_ADDR so Machine unhalts core 1 at call_start_cpu1.
                    let mut app_cpu = XtensaLx7::new_app_cpu();
                    // DRAM below PRO stack (fast_boot ~0x3FCD_FFF0); 16B aligned.
                    app_cpu.set_sp(0x3FCD_8000);
                    let mut machine =
                        labwired_core::Machine::new(pro_cpu, bus).with_secondary_cpu(app_cpu);
                    machine.observers.push(metrics.clone());
                    eprintln!(
                        "labwired-cli test: ESP32-S3 fast-boot entry=0x{:08x} (dual-core APP_CPU)",
                        program.entry_point
                    );
                    machine
                }
            } else {
                let (mut esp_bus, pro_cpu, app_cpu) =
                    match labwired_core::system::builder::build_esp32_system_from_manifest_with_plugins(
                        manifest,
                        sys_path,
                        &crate::plugin_chip_yaml(plugins),
                    ) {
                        Ok(triple) => triple,
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
                // Real partition table at flash 0x8000 when the PIO matrix
                // build left partitions.bin beside the firmware (or under
                // the usual _pio_work path). Enables esp_ota_get_running_partition
                // without a product-path OTA firmware thunk.
                if let Some(path) = resolve_esp_partitions_bin(std::path::Path::new(&firmware_path))
                {
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            if let Err(e) =
                                labwired_core::peripherals::esp32::flash_mmu::seed_esp32_flash_image(
                                    &mut esp_bus,
                                    Some(&bytes),
                                )
                            {
                                eprintln!(
                                    "labwired-cli test: warn: seed partitions from {}: {e}",
                                    path.display()
                                );
                            } else {
                                eprintln!(
                                    "labwired-cli test: seeded {} ({} bytes) @ flash 0x8000",
                                    path.display(),
                                    bytes.len()
                                );
                            }
                        }
                        Err(e) => eprintln!(
                            "labwired-cli test: warn: read partitions {}: {e}",
                            path.display()
                        ),
                    }
                }
                // Dual-core die: APP_CPU starts halted; PRO releases it through
                // the real boot path (ROM `ets_set_appcpu_boot_addr` →
                // Machine::release_secondary_cpu_if_requested). No firmware
                // flash-thunks, no forged s_cpu_up — APP_CPU runs call_start_cpu1.
                let mut machine =
                    labwired_core::Machine::new(pro_cpu, esp_bus).with_secondary_cpu(app_cpu);
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
                // (`e2e_labwired_ereader`). The BROM reset vector (0x4000_0400)
                // is fine for firmware compiled to boot from BROM, but playground
                // ELFs are pre-linked to start at the app entry.
                //
                // Seed both cores' stacks the way BROM would before
                // call_start_cpu0 / call_start_cpu1 (PRO high DRAM, APP separate
                // region below). See FIDELITY.md §C.
                machine.cpu.set_pc(program.entry_point as u32);
                machine.cpu.set_sp(0x3FFE_0000);
                if let Some(cpu1) = machine.cpu_secondary.as_mut() {
                    cpu1.set_sp(0x3FFD_8000);
                }
                // Post-BROM DRAM: flash chip descriptor + CCOUNT tick rates
                // that the skipped boot ROM would have left (no firmware patches).
                super::esp32_boot_state::seed_esp32_post_brom_dram(
                    &mut machine.bus,
                    &firmware_bytes,
                );
                // Skipped under the arduino-esp32 profile: that profile installs
                // the SAME `xthal_window_spill_nw` thunk. Installing it twice
                // patches BREAK bytes over an already-patched site, so the saved
                // original instruction is lost and the spill helper returns to
                // garbage — setup() still completed but loop() then produced
                // nothing, which looked like a firmware hang rather than a
                // double-install. One owner per thunk.
                if script_profile.as_deref() != Some("arduino-esp32") {
                    super::esp32_boot_state::install_xtensa_freertos_workarounds(
                        &mut machine.bus,
                        &firmware_bytes,
                    );
                }
                // `inputs.profile: arduino-esp32` opts into the FAST BOOT — the
                // same profile `snapshot capture` installs, from the one shared
                // home in core. Without it an Arduino-ESP32 sketch never
                // reaches setup() here, so the runner that owns `stimuli:` and
                // assertions could not exercise one at all.
                //
                // Installed AFTER the seeds above so its flash thunks are the
                // last writes into flash — seeding order clobbers otherwise.
                if script_profile.as_deref() == Some("arduino-esp32") {
                    let symbols = labwired_loader::extract_arduino_esp32_thunks(&firmware_bytes);
                    match labwired_core::system::xtensa::install_arduino_esp32_profile(
                        &mut machine,
                        symbols,
                        program.entry_point as u32,
                    ) {
                        Ok(p) => eprintln!(
                            "labwired-cli test: arduino-esp32 fast boot — {} thunks ({} symbols)",
                            p.thunks_installed,
                            p.symbols.len()
                        ),
                        Err(e) => {
                            eprintln!("error: arduino-esp32 profile: {e}");
                            return ExitCode::from(EXIT_RUNTIME_ERROR);
                        }
                    }
                }
                machine
            };
            let fault_evidence = handle_faults(&mut machine.bus, &faults);
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
                &faults,
                require_fault_fired,
                fault_evidence,
                &stimuli,
                &uart_injections,
                // Xtensa (ESP32) path: never JIT-eligible (the RV32IMC JIT is
                // RISC-V only), so keep the exact current observer-based metrics.
                false,
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

    let mut bus = match labwired_core::system::builder::build_system_bus_with_plugins(
        resolved_system.as_ref(),
        plugins,
    ) {
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

    // ESP32-C3 fast-boot (ELF app entry): behavioral models the declarative
    // stubs can't supply — same set `build_rom_boot_machine` / wasm C3 path
    // install. Without them, ROM clock bring-up faults on unmapped ANA_I2C
    // (0x6000_E000) and cache invalidate busy-polls forever.
    //
    // These stubs are the ELF-app-entry ALTERNATIVE to the faithful rom-boot
    // machine: `build_c3_rom_boot_machine` (below) installs the same set,
    // including the real 512-entry MMU table. On the rom-boot / capture /
    // resume paths that machine is built on THIS bus, so installing the
    // fast-boot stubs here would double-register `mmu_table` (a 128-entry
    // ELF-app-entry stub + the 512-entry rom-boot table). A resume snapshot
    // then carries two `mmu_table` blobs, and `apply_runtime_snapshot` — which
    // resolves peripherals by name to the FIRST match — misroutes the 512-entry
    // blob onto the 128-entry stub and fails ("snapshot has 512 entries, table
    // has 128"). Only install the fast-boot stubs on the plain ELF path.
    let rom_boot_path =
        args.rom_boot || args.capture_app_entry.is_some() || args.resume_snapshot.is_some();
    if !rom_boot_path {
        let is_c3 = system_path.as_ref().and_then(|sys_path| {
            labwired_config::SystemManifest::from_file(sys_path)
                .ok()
                .and_then(|m| {
                    let chip_dir = sys_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."));
                    labwired_config::ChipDescriptor::resolve_with(
                        &m.chip,
                        chip_dir,
                        &crate::plugin_chip_yaml(plugins),
                    )
                    .ok()
                    .map(|c| c.name == "esp32c3")
                })
        });
        if is_c3 == Some(true) {
            // SPIMEM / ANA I2C / cache / SYSTIMER / SAR / RMT / MMU+XIP /
            // irq routing — see esp32_boot_state::install_esp32c3_fast_boot.
            super::esp32_boot_state::install_esp32c3_fast_boot(
                &mut bus,
                std::path::Path::new(&firmware_path),
            );
        }
    }

    let uart_tx = Arc::new(Mutex::new(Vec::new()));
    let debug_uart = system_path
        .as_ref()
        .and_then(|path| labwired_config::SystemManifest::from_file(path).ok())
        .and_then(|manifest| manifest.debug_uart);
    if let Some(debug_uart) = debug_uart.as_deref() {
        if !bus.attach_uart_tx_sink_named(debug_uart, uart_tx.clone(), !args.no_uart_stdout) {
            warn!(
                "debug_uart '{}' did not resolve to a UART peripheral; falling back to all UARTs",
                debug_uart
            );
            bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);
        }
    } else {
        bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);
    }
    // ESP32-C3 / S3: Arduino USB-CDC `Serial` writes USB_SERIAL_JTAG, not UART0.
    // Mirror those bytes into the same uart_tx buffer so uart_contains works for
    // stock sketches (no dual Serial0 prints required).
    {
        use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
        for p in bus.peripherals.iter_mut() {
            if p.name == "usb_serial_jtag" {
                if let Some(any) = p.dev.as_any_mut() {
                    if let Some(jtag) = any.downcast_mut::<UsbSerialJtag>() {
                        jtag.set_sink(Some(uart_tx.clone()), !args.no_uart_stdout);
                    }
                }
            }
        }
    }
    // Let any attached IO-Link master record what it received over IO-Link into
    // the same captured buffer, so `uart_contains` can assert on the MASTER
    // side (MASTER PD= / MASTER VERDICT / MASTER EVENT), not just the device
    // console. No-op when no IO-Link master is attached.
    bus.attach_iolink_master_log_sink(uart_tx.clone());

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

    macro_rules! run_machine {
        ($machine:expr) => {{
            let mut machine = $machine;
            apply_matrix_speed_opts(&mut machine);
            // JIT-eligible RISC-V runs source cycles/instructions from the
            // machine's own counters (see `execute_test_loop`), so the metrics
            // step observer must NOT be installed — its presence would gate the
            // RV32IMC JIT's correctness check shut. Every other run keeps the
            // exact current behavior: metrics is the live per-step observer.
            // Gate on the `jit-core` build feature, which enables ONLY the core
            // `jit` feature (NOT `event-scheduler` — see crates/cli/Cargo.toml
            // for why the scheduler is deliberately left out). The C3
            // tick-widening path is byte-identical without the scheduler; that
            // is proven empirically by the differential tests
            // (riscv_jit_c3_oled_test_differential: JIT on vs off, and
            // riscv_tick_interval_fidelity_differential: tick interval 1 vs 64),
            // not by the scheduler. In a plain build `cfg!` is false, so every
            // run keeps the exact current observer-based, single-step behavior.
            let jit_eligible = cfg!(feature = "jit-core")
                && riscv_jit_test_eligible(
                    &args,
                    &resolved_limits,
                    &assertions,
                    &machine,
                    program.arch,
                );
            if !jit_eligible {
                machine.observers.push(metrics.clone());
            }
            let fault_evidence = handle_faults(&mut machine.bus, &faults);
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
                &faults,
                require_fault_fired,
                fault_evidence,
                &stimuli,
                &uart_injections,
                jit_eligible,
            )
        }};
    }

    macro_rules! setup_and_run {
        ($cpu:expr) => {{
            let mut machine = labwired_core::Machine::new($cpu, bus);
            // FreeRTOS on ESP32-C3 is interrupt-driven (yield + SYSTIMER tick).
            // Instruction batching freezes peripheral tick / IRQ delivery
            // across large step batches and strands the scheduler — same
            // reason rom-boot forces cycle-accurate stepping.
            if machine.bus.esp32c3_irq_routing {
                machine.config.batch_mode_enabled = false;
            }
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
            // RISC-V Arduino-ESP32 images enter at `call_start_cpu0`, which
            // assumes the second-stage bootloader already left a DRAM stack.
            // Cortex-M gets SP from the vector table in `load_firmware`; RISC-V
            // does not — seed SP at the top of chip RAM (16B aligned) and force
            // PC to the ELF entry so we don't start at SP=0 → fault @ 0xfffffffc.
            if matches!(program.arch, labwired_core::Arch::RiscV) {
                // Fast-boot skips mask-ROM reset's `.data` unpack into high
                // DRAM (`ets_ops_table_ptr` / `rom_spiflash_legacy_*` /
                // `g_flash_guard_ops` @ 0x3FCD_FFxx). Without that copy, ROM
                // helpers jalr through garbage (fault @ 0x451c8082). Mirror the
                // wasm / e2e C3 fast-start path.
                {
                    use labwired_core::boot::esp32c3_rom::{c3_rom_data_init_writes, IROM_BASE};
                    let irom = machine
                        .bus
                        .extra_mem
                        .iter()
                        .find(|m| m.base_addr == IROM_BASE as u64)
                        .map(|m| m.data.clone());
                    if let Some(irom) = irom {
                        // Only apply when IROM looks real (not all-zero).
                        if irom.iter().any(|&b| b != 0) {
                            for (dst, bytes) in c3_rom_data_init_writes(&irom) {
                                for (i, b) in bytes.iter().enumerate() {
                                    let _ = machine.bus.write_u8(dst as u64 + i as u64, *b);
                                }
                            }
                        }
                    }
                }
                if let Some(sys_path) = system_path.as_ref() {
                    if let Ok(manifest) = labwired_config::SystemManifest::from_file(sys_path) {
                        let chip_dir = sys_path
                            .parent()
                            .unwrap_or_else(|| std::path::Path::new("."))
                            ;
                        if let Ok(chip) = labwired_config::ChipDescriptor::resolve_with(&manifest.chip, chip_dir, &crate::plugin_chip_yaml(plugins)) {
                            if let Ok(ram_sz) = labwired_config::parse_size(&chip.ram.size) {
                                let mut sp_top = (chip.ram.base + ram_sz) as u32;
                                // ESP32-C3 boot stack placement:
                                // - IDF `SOC_DRAM_HIGH` = 0x3FCE_0000; SP must
                                //   be < that for `s_task_stack_is_sane_when_cache_frozen`.
                                // - BROM `.data` occupies ~0x3FCD_E710..0x3FCE_0000
                                //   (ets_ops / flash_guard tables). SP must sit
                                //   below that so the boot stack does not stomp
                                //   ROM globals (was: SP@0x3FCD_FFF0 → wild jalr).
                                if chip.name == "esp32c3" {
                                    const C3_BOOT_STACK_TOP: u32 = 0x3FCD_C000;
                                    sp_top = C3_BOOT_STACK_TOP;
                                }
                                machine.cpu.set_sp(sp_top & !0xF);
                            }
                            // Arduino-ESP32 `system_early_init` validates the
                            // ESP app image magic (0xE9) at the DROM-mapped
                            // flash header (`0x3C03_0000` on this C3 link).
                            // ELF load leaves `.flash_rodata_dummy` (NOBITS)
                            // as zeros — on silicon that VA maps the on-flash
                            // image header. Seed the magic only (honest XIP
                            // content, not a firmware patch).
                            if chip.name == "esp32c3" {
                                // After ELF load: FlashXip serves flash via MMU.
                                // ELF landed in DROM extra_mem first — copy into
                                // the shared NOR image so rodata/appdesc remain
                                // visible through XIP; keep partitions @ 0x8000
                                // and image magic @ 0x30000 (VA 0x3C03_0000).
                                //
                                // Then program MMU entries only for pages that
                                // hold ELF DROM content (bootloader-equivalent
                                // DROM map). Leave other entries invalid so
                                // `spi_flash_mmap` can allocate free pages for
                                // the partition table at flash 0x8000.
                                let flash_arc = machine
                                    .bus
                                    .find_peripheral_index_by_name("spimem1_flash")
                                    .and_then(|idx| {
                                        machine.bus.peripherals[idx].dev.as_any().and_then(
                                            |a| {
                                                a.downcast_ref::<
                                                    labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash,
                                                >()
                                                .map(|spi| spi.flash_backing())
                                            },
                                        )
                                    });
                                // Bootloader-equivalent MMU + flash image layout.
                                //
                                // C3 IROM/DROM share one table: entry =
                                // (vaddr>>16)&0x7F. `esp_ota_get_running_partition`
                                // does cache2phys(code) and requires the phys
                                // address to fall inside the factory app
                                // partition (default 0x10000). Identity map to
                                // flash 0 maps code to 0x3df8 — outside factory
                                // → abort(). Place the app image at the factory
                                // base and map virt page P → phys page
                                // (factory/PAGE + P).
                                //
                                // Leave unused entries invalid so spi_flash_mmap
                                // can allocate free pages for the partition
                                // table at flash 0x8000.
                                // C3 MMU entry_id = (vaddr >> 16) & 0x7F — IROM
                                // (0x4200_xxxx) and DROM (0x3C00_xxxx) share the
                                // same 128-entry table (0x4200&0x7F == 0, not
                                // 0x20). Factory app partition @ flash 0x10000.
                                // cache2phys(IROM code) must land inside app0 or
                                // esp_ota_get_running_partition aborts.
                                const PAGE: usize = 64 * 1024;
                                const FACTORY_OFF: usize = 0x1_0000;
                                const FACTORY_PAGE: u32 = (FACTORY_OFF / PAGE) as u32; // 1
                                // virt_page index within the 8 MiB window
                                let mut virt_pages: Vec<u32> = Vec::new();
                                let irom_len = machine.bus.flash.data.len();
                                for page in 0..irom_len.div_ceil(PAGE) {
                                    if page >= 128 {
                                        break;
                                    }
                                    let start = page * PAGE;
                                    let end = (start + PAGE).min(irom_len);
                                    if machine.bus.flash.data[start..end]
                                        .iter()
                                        .any(|&b| b != 0)
                                    {
                                        virt_pages.push(page as u32);
                                    }
                                }
                                if let Some(flash) = flash_arc {
                                    let mut f = flash.lock().unwrap();
                                    let drom_snapshot: Option<Vec<u8>> = machine
                                        .bus
                                        .extra_mem
                                        .iter()
                                        .find(|m| m.base_addr == 0x3C00_0000)
                                        .map(|m| m.data.clone());
                                    if let Some(drom) = drom_snapshot {
                                        for page in 0..drom.len().div_ceil(PAGE)
                                        {
                                            if page >= 128 {
                                                break;
                                            }
                                            let start = page * PAGE;
                                            let end = (start + PAGE).min(drom.len());
                                            if !drom[start..end].iter().any(|&b| b != 0)
                                            {
                                                continue;
                                            }
                                            if !virt_pages.contains(&(page as u32)) {
                                                virt_pages.push(page as u32);
                                            }
                                            let dst = FACTORY_OFF + start;
                                            if dst + (end - start) <= f.len() {
                                                f[dst..dst + (end - start)]
                                                    .copy_from_slice(&drom[start..end]);
                                            }
                                        }
                                    }
                                    // Mirror IROM into factory pages (cache2phys);
                                    // execute still uses bus.flash at 0x4200_0000.
                                    for page in virt_pages.clone() {
                                        let start = page as usize * PAGE;
                                        let end = (start + PAGE).min(irom_len);
                                        let dst = FACTORY_OFF + start;
                                        if start >= irom_len || dst >= f.len() {
                                            continue;
                                        }
                                        let n = (end - start).min(f.len() - dst);
                                        for i in 0..n {
                                            let b = machine.bus.flash.data[start + i];
                                            if b != 0 {
                                                f[dst + i] = b;
                                            }
                                        }
                                    }
                                    if let Some(p) = resolve_esp_partitions_bin(
                                        std::path::Path::new(&firmware_path),
                                    ) {
                                        if let Ok(pt) = std::fs::read(&p) {
                                            let n = pt.len().min(0xC00);
                                            f[0x8000..0x8000 + n]
                                                .copy_from_slice(&pt[..n]);
                                        }
                                    }
                                    // App image magic @ VA 0x3C03_0000 → factory+0x30000
                                    let magic_off = FACTORY_OFF + 0x30000;
                                    if f.len() > magic_off {
                                        f[magic_off] = 0xE9;
                                    }
                                }
                                virt_pages.sort_unstable();
                                virt_pages.dedup();
                                for vp in &virt_pages {
                                    let phys = FACTORY_PAGE + *vp;
                                    let mmu_addr = 0x600C_5000u64 + (*vp as u64) * 4;
                                    let _ = machine.bus.write_u32(mmu_addr, phys);
                                }
                                if !virt_pages.is_empty() {
                                    eprintln!(
                                        "labwired-cli test: C3 MMU factory@{:#x} mapped {} virt page(s) {:?} → phys+{}; free entries for mmap",
                                        FACTORY_OFF,
                                        virt_pages.len(),
                                        virt_pages,
                                        FACTORY_PAGE
                                    );
                                }
                            }
                        }
                    }
                }
                machine.cpu.set_pc(program.entry_point as u32);
            }
            run_machine!(machine)
        }};
    }

    let exit_code = match program.arch {
        labwired_core::Arch::Arm => {
            let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            setup_and_run!(cpu)
        }
        labwired_core::Arch::RiscV => {
            if let Some(snap_path) = &args.resume_snapshot {
                // ── Resume from a captured app-entry snapshot (no cold boot) ──
                // Build the SAME faithful rom-boot machine (which loads the real
                // boot ROM + flash image and wires every peripheral), then stamp
                // the snapshot on top. take_runtime_snapshot skips the flash/rom
                // mirrors — they are re-derived here from the freshly-loaded
                // flash — so restoring REQUIRES the identical firmware, enforced
                // by the self-key gate below. The snapshot overwrites the CPU's
                // PC to app-entry, so the mask ROM is never replayed: execution
                // starts in the application immediately.
                let snap_bytes = match std::fs::read(snap_path) {
                    Ok(b) => b,
                    Err(e) => {
                        let msg = format!("cannot read resume snapshot {snap_path:?}: {e}");
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
                let snap = match labwired_core::runtime_snapshot::MachineRuntimeSnapshot::from_bytes(
                    &snap_bytes,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        // Corrupt or version-mismatched blob. Snapshot-invalid:
                        // write NO result.json so the caller cold-boots and
                        // refreshes the cache.
                        error!("invalid resume snapshot {snap_path:?}: {e}; cold-boot required");
                        return ExitCode::from(EXIT_CONFIG_ERROR);
                    }
                };
                let (chip, fw_sha) = match crate::rom_boot_flash_self_key() {
                    Some(v) => v,
                    None => {
                        let msg = "--resume-snapshot needs LABWIRED_ESP32C3_FLASH set (the same \
                                   flash image the snapshot was captured against)"
                            .to_string();
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
                if let Err(e) = snap.validate_self_key(chip, &fw_sha) {
                    // Stale/foreign snapshot (captured against a different chip or
                    // firmware). Write NO result.json so a cache-backed caller
                    // treats it as "resume did not run" and cold-boots to refresh
                    // the cache — the snapshot-invalid contract the resume-error
                    // fallback in the run service depends on.
                    error!("resume snapshot self-key mismatch ({e}); cold-boot required");
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
                let mut machine = match crate::build_c3_rom_boot_machine(bus, None) {
                    Ok(m) => m,
                    Err(code) => return code,
                };
                if let Err(e) = machine.apply_runtime_snapshot(&snap) {
                    // The snapshot self-keyed clean (same chip + firmware) but is
                    // structurally incompatible with the freshly-built machine —
                    // e.g. a stale blob captured by an older, buggy core whose bus
                    // topology differs (the C3 double-`mmu_table` window). This is
                    // a snapshot-invalid signal, NOT a firmware fault: deliberately
                    // write NO result.json so the caller (which resumes from a
                    // cache) sees the resume did not run and falls back to a cold
                    // `--rom-boot`, refreshing the cache with a compatible capture.
                    // Same contract as a self-key mismatch above.
                    error!(
                        "resume snapshot incompatible with this machine ({e}); \
                         cold-boot required (stale/foreign snapshot)"
                    );
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
                eprintln!(
                    "labwired-riscv: resumed from app-entry snapshot {snap_path:?} (chip {chip}); \
                     mask-ROM replay skipped"
                );
                run_machine!(machine)
            } else if args.rom_boot {
                // Faithful boot: real mask ROM → 2nd-stage bootloader → app,
                // loading from the flash image (LABWIRED_ESP32C3_FLASH), on
                // the SAME from_config bus — external devices and assertions
                // work exactly as on the fast-boot path. The ELF is NOT
                // loaded into memory (the flash image is the program; the
                // ELF still feeds symbols/diagnostics).
                let machine = match crate::build_c3_rom_boot_machine(bus, None) {
                    Ok(m) => m,
                    Err(code) => return code,
                };
                run_machine!(machine)
            } else {
                let mut cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
                // C3 has no standard CLINT (line 7 is an ESP matrix line).
                // Default mtimecmp=0 self-pends MTIP and breaks FreeRTOS first
                // yield via FROM_CPU — same disable as rom-boot.
                if bus.esp32c3_irq_routing {
                    cpu.mtimecmp = u64::MAX;
                }
                setup_and_run!(cpu)
            }
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
