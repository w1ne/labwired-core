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
                    let opts = Esp32s3Opts::default();
                    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
                    if _fast.is_none() {
                        std::env::remove_var("LABWIRED_ESP32S3_FASTBOOT");
                    }
                    // Seed partition table + app image magic into D-cache
                    // identity window (VA 0x3C00_0000 → dcache[off]).
                    {
                        let pt_candidates = [
                            std::path::PathBuf::from(
                                "validation/arduino-matrix/out/_pio_work/esp32s3__L0_serial_boot/.pio/build/matrix/partitions.bin",
                            ),
                            std::path::PathBuf::from(
                                "validation/arduino-matrix/out/_pio_work/esp32__L0_serial_boot/.pio/build/matrix/partitions.bin",
                            ),
                        ];
                        if let Ok(mut d) = wiring.dcache_backing.lock() {
                            for p in &pt_candidates {
                                if let Ok(pt) = std::fs::read(p) {
                                    let n = pt.len().min(0xC00);
                                    if d.len() >= 0x8000 + n {
                                        d[0x8000..0x8000 + n].copy_from_slice(&pt[..n]);
                                        eprintln!(
                                            "labwired-cli test: seeded S3 dcache partitions ({} bytes) from {}",
                                            n,
                                            p.display()
                                        );
                                    }
                                    break;
                                }
                            }
                            // system_early_init validates ESP app magic 0xE9 at
                            // DROM-mapped header (this link: VA 0x3C03_0000).
                            if d.len() > 0x30000 {
                                d[0x30000] = 0xE9;
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
                                        jtag.set_sink(
                                            Some(uart_tx.clone()),
                                            !args.no_uart_stdout,
                                        );
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
                            icache_backing: Some(wiring.icache_backing),
                            dcache_backing: Some(wiring.dcache_backing),
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
                    // Same xthal spill CPU-model workaround as classic.
                    if let Some(pc) = labwired_loader::resolve_symbol_in_elf(
                        &firmware_bytes,
                        "xthal_window_spill_nw",
                    ) {
                        if let Err(e) = bus
                            .install_flash_thunk(pc, rom_thunks::xthal_window_spill_thunk)
                        {
                            eprintln!(
                                "labwired-cli test: warn: xthal_window_spill_nw install failed: {e}"
                            );
                        } else {
                            eprintln!(
                                "labwired-cli test: installed xthal_window_spill_nw CPU spill workaround @0x{pc:08x}"
                            );
                        }
                    }
                    // Prefer real APP_CPU; if it is missing, the
                    // ets_set_appcpu_boot_addr thunk still raises s_cpu_up via
                    // set_appcpu_up_flags so PRO can leave system_early_init.
                    // Running a half-booted APP (no ROM vectors) currently
                    // faults the machine — flags-only is the silicon-observable
                    // handshake for this fast-boot path (same approach the
                    // classic thunk documents for short-timeout IDF).
                    let mut machine = labwired_core::Machine::new(pro_cpu, bus);
                    machine.observers.push(metrics.clone());
                    eprintln!(
                        "labwired-cli test: ESP32-S3 fast-boot entry=0x{:08x} (APP via s_cpu_up flags)",
                        program.entry_point
                    );
                    machine
                }
            } else {
                let (mut esp_bus, pro_cpu, app_cpu) =
                    match labwired_core::system::builder::build_esp32_system_from_manifest(
                        manifest, sys_path,
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
                {
                    let fw_path = std::path::Path::new(&firmware_path);
                    let candidates = [
                        fw_path
                            .parent()
                            .map(|p| p.join("partitions.bin"))
                            .unwrap_or_default(),
                        fw_path
                            .parent()
                            .and_then(|p| p.parent())
                            .map(|p| {
                                p.join("_pio_work/esp32__L0_serial_boot/.pio/build/matrix/partitions.bin")
                            })
                            .unwrap_or_default(),
                        // validation/arduino-matrix/out/esp32/L0_... → out/_pio_work/...
                        fw_path
                            .parent()
                            .and_then(|p| p.parent())
                            .and_then(|p| p.parent())
                            .map(|p| {
                                p.join("_pio_work/esp32__L0_serial_boot/.pio/build/matrix/partitions.bin")
                            })
                            .unwrap_or_default(),
                    ];
                    let pt = candidates.iter().find(|p| p.is_file()).and_then(|p| {
                        std::fs::read(p).ok().map(|b| (p.clone(), b))
                    });
                    if let Some((path, bytes)) = pt {
                        if let Err(e) = labwired_core::peripherals::esp32::flash_mmu::seed_esp32_flash_image(
                            &mut esp_bus,
                            Some(&bytes),
                        ) {
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
                }
                // Dual-core die: APP_CPU starts halted; PRO releases it through
                // the real boot path (ROM `ets_set_appcpu_boot_addr` →
                // Machine::release_secondary_cpu_if_requested). No firmware
                // flash-thunks, no forged s_cpu_up — APP_CPU runs call_start_cpu1.
                let mut machine = labwired_core::Machine::new(pro_cpu, esp_bus)
                    .with_secondary_cpu(app_cpu);
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
                // CPU-model workaround for shadow-window vs firmware spill
                // (FIDELITY Batch D). Not a flash-init firmware thunk.
                if let Some(pc) =
                    labwired_loader::resolve_symbol_in_elf(&firmware_bytes, "xthal_window_spill_nw")
                {
                    use labwired_core::peripherals::esp_xtensa_common::rom_thunks;
                    if let Err(e) = machine
                        .bus
                        .install_flash_thunk(pc, rom_thunks::xthal_window_spill_thunk)
                    {
                        eprintln!(
                            "labwired-cli test: warn: xthal_window_spill_nw install failed: {e}"
                        );
                    } else {
                        eprintln!(
                            "labwired-cli test: installed xthal_window_spill_nw CPU spill workaround @0x{pc:08x}"
                        );
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

    // ESP32-C3 fast-boot (ELF app entry): behavioral models the declarative
    // stubs can't supply — same set `build_rom_boot_machine` / wasm C3 path
    // install. Without them, ROM clock bring-up faults on unmapped ANA_I2C
    // (0x6000_E000) and cache invalidate busy-polls forever.
    {
        let is_c3 = system_path.as_ref().and_then(|sys_path| {
            labwired_config::SystemManifest::from_file(sys_path).ok().and_then(|m| {
                let chip_path = sys_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(&m.chip);
                labwired_config::ChipDescriptor::from_file(&chip_path)
                    .ok()
                    .map(|c| c.name == "esp32c3")
            })
        });
        if is_c3 == Some(true) {
            use std::sync::{Arc, Mutex};
            // SPIMEM0/1 flash-command controllers: Arduino `call_start_cpu0` →
            // `bootloader_read_flash_id` busy-polls CMD until 0. Declarative
            // stubs never clear. Same S3 SPIMEM model as rom-boot (layout match).
            let mut flash_img = vec![0xFFu8; 4 * 1024 * 1024];
            // Partition table @ flash 0x8000 — `esp_partition` requires MD5
            // trailer (0xEBEB…). Prefer matrix/PIO build artifact, else the
            // classic Arduino default table (same on-disk format for C3).
            let pt_candidates = [
                std::path::PathBuf::from(
                    "validation/arduino-matrix/out/_pio_work/esp32c3__L0_serial_boot/.pio/build/matrix/partitions.bin",
                ),
                std::path::PathBuf::from(
                    "validation/arduino-matrix/out/_pio_work/esp32__L0_serial_boot/.pio/build/matrix/partitions.bin",
                ),
            ];
            for p in &pt_candidates {
                if let Ok(pt) = std::fs::read(p) {
                    let n = pt.len().min(0xC00);
                    flash_img[0x8000..0x8000 + n].copy_from_slice(&pt[..n]);
                    eprintln!(
                        "labwired-cli test: seeded C3 flash partitions ({} bytes) from {}",
                        n,
                        p.display()
                    );
                    break;
                }
            }
            let flash = Arc::new(Mutex::new(flash_img));
            bus.add_peripheral(
                "spimem1_flash",
                0x6000_2000,
                0x100,
                None,
                Box::new(
                    labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(
                        flash.clone(),
                    ),
                ),
            );
            bus.add_peripheral(
                "spimem0_flash",
                0x6000_3000,
                0x100,
                None,
                Box::new(
                    labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(
                        flash.clone(),
                    ),
                ),
            );
            bus.add_peripheral(
                "rtc_i2c_ana",
                0x6000_E000,
                0x400,
                None,
                Box::new(labwired_core::peripherals::esp32c3::ana_i2c::Esp32c3AnaI2c::new()),
            );
            bus.add_peripheral(
                "extmem_cache",
                0x600C_4000,
                0x400,
                None,
                Box::new(labwired_core::peripherals::esp32c3::cache::Esp32c3Cache::new()),
            );
            // SYSTIMER: declarative stub never asserts VALUE_VALID → spin in
            // systimer_hal_get_counter_value. S3 model (same IP) + C3 IRQ 37.
            bus.add_peripheral(
                "systimer",
                0x6002_3000,
                0x100,
                None,
                Box::new(
                    labwired_core::peripherals::esp32s3::systimer::Systimer::new_with_source(
                        160_000_000, 37,
                    ),
                ),
            );
            // SAR ADC: calibrate loops need valid conversions.
            bus.add_peripheral(
                "apb_saradc",
                0x6004_0000,
                0x100,
                None,
                Box::new(labwired_core::peripherals::esp32c3::sar_adc::Esp32c3SarAdc::new()),
            );
            // RMT @ 0x6001_6000: Arduino `LED_BUILTIN`/pin 30 is RGB_BUILTIN →
            // `rgbLedWrite` → RMT TX. Instant TX_END so digitalWrite completes.
            bus.add_peripheral(
                "rmt",
                0x6001_6000,
                0x800,
                None,
                Box::new(labwired_core::peripherals::esp32c3::rmt::Esp32c3Rmt::new_default()),
            );
            // Flash cache MMU @ 0x600C_5000 + DROM FlashXip.
            //
            // Start with *all* entries invalid (silicon reset). Full identity
            // mapping of 128 pages left `spi_flash_mmap` with zero free entries
            // → partition load never maps flash[0x8000] → silent hang / empty
            // UART. Bootloader-equivalent maps for app DROM pages are applied
            // after ELF load (see post-load C3 flash sync below) only for pages
            // that actually hold rodata, leaving the rest free for mmap.
            //
            // `optimized_bus_access=false` so FlashXip wins over DROM extra_mem
            // when an entry is valid. IROM (0x4200_0000) stays on chip `flash`
            // / extra_mem (no XIP window installed there for CLI fast-boot).
            use labwired_core::peripherals::esp32s3::flash_xip::{
                Esp32s3MmuTable, FlashXipPeripheral, SharedMmu, MMU_FMT_C3,
            };
            use std::sync::atomic::AtomicU64;
            let entries = vec![MMU_FMT_C3.invalid_bit; 128];
            let mmu_table = Arc::new(SharedMmu {
                entries: Mutex::new(entries),
                generation: AtomicU64::new(1),
            });
            bus.add_peripheral(
                "mmu_table",
                0x600C_5000,
                0x800,
                None,
                Box::new(Esp32s3MmuTable::new(mmu_table.clone())),
            );
            bus.add_peripheral(
                "flash_xip_drom",
                0x3C00_0000,
                0x80_0000,
                None,
                Box::new(FlashXipPeripheral::new_mmu_fmt(
                    flash.clone(),
                    0x3C00_0000,
                    mmu_table,
                    MMU_FMT_C3,
                )),
            );
            bus.config.optimized_bus_access = false;
            // FreeRTOS first context switch: vPortYield → write SYSTEM
            // CPU_INTR_FROM_CPU_0 (0x600C_0028) → FROM_CPU source 50 →
            // esp_crosscore_isr → vPortYieldFromISR. Without this flag the
            // bus never routes matrix sources into `riscv_irq_lines`, so
            // yield is a no-op and `vTaskStartScheduler` returns into the
            // intentional infinite loop at start_cpu0+0x56.
            bus.esp32c3_irq_routing = true;
            bus.refresh_peripheral_index();
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
                        let chip_path = sys_path
                            .parent()
                            .unwrap_or_else(|| std::path::Path::new("."))
                            .join(&manifest.chip);
                        if let Ok(chip) = labwired_config::ChipDescriptor::from_file(&chip_path) {
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
                                for page in 0..(irom_len + PAGE - 1) / PAGE {
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
                                        for page in 0..(drom.len() + PAGE - 1) / PAGE
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
                                    let pt_candidates = [
                                        std::path::PathBuf::from(
                                            "validation/arduino-matrix/out/_pio_work/esp32c3__L0_serial_boot/.pio/build/matrix/partitions.bin",
                                        ),
                                        std::path::PathBuf::from(
                                            "validation/arduino-matrix/out/_pio_work/esp32__L0_serial_boot/.pio/build/matrix/partitions.bin",
                                        ),
                                    ];
                                    for p in &pt_candidates {
                                        if let Ok(pt) = std::fs::read(p) {
                                            let n = pt.len().min(0xC00);
                                            f[0x8000..0x8000 + n]
                                                .copy_from_slice(&pt[..n]);
                                            break;
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
