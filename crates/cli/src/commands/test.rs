// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired test` subcommand: run the Tier-1 protocol suite.

use crate::*;
use tracing::warn;

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
            )
        }
        LoadedTestScript::Env(script) => {
            let outcome = super::environment_test::run_environment_test(&args, script);
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
    let max_allowed_steps = if args.rom_boot {
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
                let chip_path = sys_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(&manifest.chip);
                labwired_config::ChipDescriptor::from_file(&chip_path)
                    .map(|c| c.name == "esp32s3")
                    .unwrap_or(false)
            };

            let mut machine = if args.rom_boot && is_esp32s3 {
                // ── Faithful ESP32-S3 ROM boot (mirrors the `run` command) ──
                // Reset the CPU at the BROM vector 0x40000400 and let the real
                // mask ROM run: it loads the 2nd-stage bootloader + app from the
                // flash image (LABWIRED_ESP32S3_FLASH) through the SPI-flash
                // controller and jumps to the app — exactly like silicon. No
                // fast_boot, no ELF pre-load, no PC/SP seeding: zero thunks.
                // Single-core (PRO_CPU): esp-hal apps run entirely on core 0;
                // the ESP-IDF 2nd-stage bootloader is single-core at boot.
                use labwired_core::system::xtensa::{
                    configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts,
                };
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
                if wiring.boot_mode != Esp32s3BootMode::Faithful {
                    let msg = "--rom-boot needs the real ESP32-S3 boot ROM, but none was found. \
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
                // The real ROM + firmware install the OF/UF window vectors and
                // build a proper stack save chain, so use the faithful
                // per-access overflow / RETW underflow path (no sim shadow
                // stack) — matching the `run` command's rom-boot path.
                cpu.faithful_windows = true;
                bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);
                let mut machine = labwired_core::Machine::new(cpu, bus);
                machine.observers.push(metrics.clone());
                // No load_firmware / set_pc: the CPU resets at 0x40000400 and
                // the mapped ROM boots the app from flash exactly like silicon.
                machine
            } else {
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
                // Seed SP to the top of DRAM (0x3FFE_0000): Arduino-ESP32
                // firmware (call_start_cpu0) expects BROM to have placed SP here
                // before jumping to the app entry. We skip BROM, so do it
                // ourselves — matching `install_esp32_arduino_quirks` in the WASM
                // path. Native Xtensa firmware that sets its own SP will
                // overwrite this immediately.
                // CHEAT(SKIP): bypasses the boot ROM and hand-seeds PC/SP — real:
                // the CPU resets at 0x4000_0400 and the mapped BROM sets up SP and
                // jumps to the app. See FIDELITY.md §C.
                machine.cpu.set_pc(program.entry_point as u32);
                machine.cpu.set_sp(0x3FFE_0000);
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
                && riscv_jit_test_eligible(&args, &resolved_limits, &machine, program.arch);
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
                jit_eligible,
            )
        }};
    }

    macro_rules! setup_and_run {
        ($cpu:expr) => {{
            let mut machine = labwired_core::Machine::new($cpu, bus);
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
                        let msg = format!("invalid resume snapshot {snap_path:?}: {e}");
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
                    let msg =
                        format!("resume snapshot self-key mismatch ({e}); cold-boot required");
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
                let mut machine = match crate::build_c3_rom_boot_machine(bus, None) {
                    Ok(m) => m,
                    Err(code) => return code,
                };
                if let Err(e) = machine.apply_runtime_snapshot(&snap) {
                    let msg = format!("failed to apply resume snapshot: {e}");
                    error!("{}", msg);
                    write_config_error_outputs(
                        &args,
                        Some(&firmware_path),
                        system_path.as_ref(),
                        Some(&firmware_bytes),
                        Some(&resolved_limits),
                        msg,
                    );
                    return ExitCode::from(EXIT_RUNTIME_ERROR);
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
                let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
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
