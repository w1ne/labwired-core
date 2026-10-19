// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! System-aware `labwired run` driver: attach the external devices declared in
//! a SystemManifest, drive the firmware with the generic `Machine` loop, and
//! apply `--stimulus` entries through the shared `StimulusTrack`.
//!
//! Supports Cortex-M and the optional native Cortex-A7 CPU adapter. RISC-V and Xtensa boot
//! preparation lives in the test runner and is an explicit follow-up; an
//! unsupported arch fails fast instead of running a partially-booted system.

use crate::stimuli::{parse_stimulus_arg, StimulusTrack};
use crate::{emit_error, RunArgs, EXIT_CONFIG_ERROR, EXIT_PASS, EXIT_RUNTIME_ERROR};

use labwired_config::{Arch, ChipDescriptor, StimulusSpec, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Cpu, Machine};
use std::num::NonZeroU32;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

/// Instructions per batched `advance`; keeps stimulus deadlines and UART
/// output responsive without single-stepping multi-million-cycle runs.
const RUN_BATCH_CAP: u64 = 10_000;

pub(crate) fn run_firmware_with_system(
    args: &RunArgs,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
    json: bool,
) -> ExitCode {
    let Some(system_path) = args.system.as_deref() else {
        unreachable!("run_firmware dispatches here only when --chip is absent");
    };
    let Some(max_steps) = args.max_steps else {
        emit_error(
            json,
            "ConfigError",
            "run --system requires an explicit --max-steps budget".to_string(),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    };
    if args.gpio_trace.is_some() {
        emit_error(
            json,
            "ConfigError",
            "--gpio-trace is only supported on the ESP32-S3 run path; it cannot be combined with the system-aware driver (--system without --chip)"
                .to_string(),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }
    if args.rom_boot {
        emit_error(
            json,
            "ConfigError",
            "--rom-boot is only supported on the ESP32-S3 chip path; it cannot be combined with the system-aware driver (--system without --chip)"
                .to_string(),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }
    if !args.break_at.is_empty() {
        emit_error(
            json,
            "ConfigError",
            "--break-at is only supported on the ESP32-S3 chip path; it cannot be combined with the system-aware driver (--system without --chip)"
                .to_string(),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }
    if !args.watch_mem.is_empty() {
        emit_error(
            json,
            "ConfigError",
            "--watch-mem is only supported on the ESP32-S3 chip path; it cannot be combined with the system-aware driver (--system without --chip)"
                .to_string(),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let specs = match parse_all_stimuli(&args.stimulus) {
        Ok(specs) => specs,
        Err((index, message)) => {
            emit_error(
                json,
                "ConfigError",
                message,
                Some(serde_json::json!({ "stimulus_index": index })),
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let manifest = match SystemManifest::from_file(system_path) {
        Ok(m) => m,
        Err(e) => {
            emit_error(
                json,
                "ConfigError",
                format!(
                    "cannot parse system manifest {}: {e:#}",
                    system_path.display()
                ),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    // Resolve the chip the way every other manifest consumer does: a bare id
    // through the built-in registry (then plugins), a path-shaped spec against
    // the manifest's directory. Joining the spec as a file path first refused
    // every GENERATED manifest — `prepareRunTarget` emits `chip: stm32f103`
    // for the hosted builder — with "cannot parse chip descriptor
    // <dir>/stm32f103", which also hid the fail-fast channel inventory from
    // `--stimulus` callers.
    let chip = match ChipDescriptor::resolve_with(
        &manifest.chip,
        system_path.parent().unwrap_or_else(|| Path::new(".")),
        &crate::plugin_chip_yaml(plugins),
    ) {
        Ok(c) => c,
        Err(e) => {
            emit_error(
                json,
                "ConfigError",
                format!("cannot load chip descriptor '{}': {e:#}", manifest.chip),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    if chip.arch != Arch::Arm {
        emit_error(
            json,
            "ConfigError",
            format!(
                "run --system currently supports ARM manifests only; got arch {:?} from chip '{}' \
                 (RISC-V and Xtensa boot paths are a follow-up)",
                chip.arch, manifest.chip
            ),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    // Descriptor paths inside a file-backed chip resolve against the chip
    // file, so anchor `chip:` to the manifest's directory first (as
    // `labwired test` does); otherwise they resolve against the process CWD.
    let anchored = labwired_core::system::builder::anchor_chip_path(
        &manifest,
        system_path.parent().unwrap_or_else(|| Path::new(".")),
    );
    let mut bus = match SystemBus::from_config_with_plugins(&chip, &anchored, plugins) {
        Ok(b) => b,
        Err(e) => {
            emit_error(
                json,
                "ConfigError",
                format!("cannot build system bus: {e:#}"),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Echo UART to stdout only in human mode: with --json, stdout must carry
    // a single parseable JSON document, so UART is captured but not echoed.
    let uart_sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    bus.attach_uart_tx_sink(uart_sink, !json);

    let program = match labwired_loader::load_elf(&args.firmware) {
        Ok(p) => p,
        Err(e) => {
            emit_error(
                json,
                "LoadError",
                format!("cannot load firmware ELF {:?}: {e}", args.firmware),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    use labwired_core::system::arch_policy::{machine_family, MachineFamily};
    let family = match machine_family(&chip) {
        Ok(family) => family,
        Err(error) => {
            emit_error(
                json,
                "ConfigError",
                error.to_string(),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    if family == MachineFamily::CortexA && program.arch != labwired_core::Arch::Arm {
        emit_error(json, "ConfigError", "Cortex-A7 requires a 32-bit ARM ELF; firmware architecture does not match the declared chip".into(), None, EXIT_CONFIG_ERROR);
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }
    let cpu: Box<dyn Cpu> = if family == MachineFamily::CortexA {
        #[cfg(feature = "qemu-arm")]
        {
            match labwired_core::cpu::CortexA7::new() {
                Ok(cpu) => Box::new(cpu),
                Err(error) => {
                    emit_error(
                        json,
                        "ConfigError",
                        error.to_string(),
                        None,
                        EXIT_CONFIG_ERROR,
                    );
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            }
        }
        #[cfg(not(feature = "qemu-arm"))]
        {
            emit_error(
                json,
                "ConfigError",
                "Cortex-A7 needs a native build with --features qemu-arm".into(),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    } else {
        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        Box::new(cpu)
    };
    let mut machine = Machine::new(cpu, bus);
    crate::log_ts::set_hz(machine.bus.cpu_hz);
    machine.config.host_time_mode = args.time_mode;
    machine.config.idle_fast_forward_enabled =
        std::env::var("LABWIRED_IDLE_FAST_FORWARD").as_deref() != Ok("0");
    if let Err(e) = machine.load_firmware(&program) {
        emit_error(
            json,
            "LoadError",
            format!("cannot map firmware into bus: {e}"),
            None,
            EXIT_RUNTIME_ERROR,
        );
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    if let Err(errors) = validate_stimuli(&specs, &mut machine.bus) {
        let available = inventory(&mut machine.bus);
        let message = errors
            .iter()
            .filter_map(|e| e["error"].as_str())
            .collect::<Vec<_>>()
            .join("; ");
        emit_error(
            json,
            "ConfigError",
            format!("stimulus validation failed: {message}"),
            Some(serde_json::json!({ "errors": errors, "available": available })),
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let mut track = StimulusTrack::new(&specs);
    track.apply_at_start(&mut machine);

    let mut steps: u64 = 0;
    while steps < max_steps {
        crate::log_ts::refresh_cycles(machine.total_cycles);
        let current_cycle = machine.total_cycles;
        track.poll(&mut machine, current_cycle);
        let mut limit = (max_steps - steps).min(RUN_BATCH_CAP);
        if let Some(deadline) = track.next_deadline_after(current_cycle) {
            limit = limit.min(deadline - current_cycle);
        }
        let batch = limit.max(1);
        let request = labwired_core::AdvanceRequest::run(Some(batch))
            .with_batch_cap(
                NonZeroU32::new(batch.min(u64::from(u32::MAX)) as u32).expect("batch is non-zero"),
            )
            .with_breakpoints(labwired_core::BreakpointPolicy::Ignore);
        let result = machine.advance(request);
        crate::log_ts::refresh_cycles(machine.total_cycles);
        match result {
            Ok(report) => {
                steps += report.fuel_consumed;
                if report.primary_steps == 0 && report.idle_cycles == 0 {
                    break; // halt
                }
            }
            Err(e) => {
                crate::commands::run::export_bus_trace_if_requested(
                    &args.bus_trace_out,
                    &machine.bus,
                );
                let elf = std::fs::read(&args.firmware).ok();
                let verdict = crate::fault_report::fault_verdict(
                    &machine.cpu,
                    elf.as_deref(),
                    machine.bus.memory_policy_report().as_ref(),
                );
                if let (Some(v), false) = (&verdict, json) {
                    crate::fault_report::eprint(v);
                }
                emit_error(
                    json,
                    "RuntimeError",
                    format!("simulation error at step {steps}: {e}"),
                    Some(serde_json::json!({ "step": steps, "fault_verdict": verdict })),
                    EXIT_RUNTIME_ERROR,
                );
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        }
    }

    crate::commands::run::export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
    let pending = track.pending();
    if !pending.is_empty() {
        let list = pending
            .iter()
            .map(|(i, channel, deadline)| {
                format!("stimulus[{i}] '{channel}' due at cycle {deadline}")
            })
            .collect::<Vec<_>>()
            .join("; ");
        crate::log_ts::eventln!(
            "labwired-cli run (system): warning: {list} never fired before the run ended"
        );
    }
    let elf = std::fs::read(&args.firmware).ok();
    if let Some(v) = crate::fault_report::fault_verdict(
        &machine.cpu,
        elf.as_deref(),
        machine.bus.memory_policy_report().as_ref(),
    ) {
        crate::fault_report::eprint(&v);
    }
    let pc = machine.cpu.get_pc();
    if steps >= max_steps {
        crate::log_ts::eventln!(
            "labwired-cli run (system): reached --max-steps {max_steps}; pc=0x{pc:08x}"
        );
    } else {
        crate::log_ts::eventln!("labwired-cli run (system): halted at step {steps}; pc=0x{pc:08x}");
    }
    ExitCode::from(EXIT_PASS)
}

/// Parse every `--stimulus` argument; the first failure carries its index.
fn parse_all_stimuli(raw: &[String]) -> Result<Vec<StimulusSpec>, (usize, String)> {
    let mut out = Vec::with_capacity(raw.len());
    for (i, arg) in raw.iter().enumerate() {
        match parse_stimulus_arg(i, arg) {
            Ok(spec) => out.push(spec),
            Err(message) => return Err((i, message)),
        }
    }
    Ok(out)
}

/// Resolve and range-check every spec against the live bus without applying
/// anything. Returns all failures so one message can list them.
fn validate_stimuli(
    specs: &[StimulusSpec],
    bus: &mut SystemBus,
) -> Result<(), Vec<serde_json::Value>> {
    let mut errors = Vec::new();
    for (i, s) in specs.iter().enumerate() {
        let Some(target) = s.input_target() else {
            errors.push(serde_json::json!({
                "stimulus_index": i,
                "channel": serde_json::Value::Null,
                "error": format!(
                    "stimulus[{i}]: co-simulation stimuli are not supported by the system-aware driver"
                ),
            }));
            continue;
        };
        match bus.resolve_input(target.component.as_deref(), &target.channel) {
            Ok(ch) => {
                let value = s.value();
                if !value.is_finite() || value < ch.min || value > ch.max {
                    let name = match target.component.as_deref() {
                        Some(c) => format!("{c}/{}", target.channel),
                        None => target.channel.clone(),
                    };
                    errors.push(serde_json::json!({
                        "stimulus_index": i,
                        "channel": target.channel,
                        "error": format!(
                            "stimulus[{i}] {name}: value {value} outside [{}, {}] {}",
                            ch.min, ch.max, ch.unit
                        ),
                    }));
                }
            }
            Err(e) => errors.push(serde_json::json!({
                "stimulus_index": i,
                "channel": target.channel,
                "error": format!("stimulus[{i}]: {e}"),
            })),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The `{component, channel}` drive points the firmware's board exposes, for
/// agent self-correction on validation failures.
fn inventory(bus: &mut SystemBus) -> Vec<serde_json::Value> {
    bus.list_inputs()
        .into_iter()
        .map(|(component, ch)| {
            serde_json::json!({
                "component": component,
                "channel": ch.key,
                "unit": ch.unit,
                "min": ch.min,
                "max": ch.max,
            })
        })
        .collect()
}
