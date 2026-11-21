// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired test` for portable Flex projects (`inputs.flex`).
//!
//! The shared runner in `labwired_core::flex_script` executes the script; the
//! browser binding calls the same runner. This file reads the inputs, writes
//! the artifacts, and maps the result to an exit code. Nothing in this path
//! falls back to a target CPU: a script that wants target instructions names
//! `inputs.system`.

use crate::{TestArgs, EXIT_ASSERT_FAIL, EXIT_CONFIG_ERROR, EXIT_PASS, EXIT_RUNTIME_ERROR};
use labwired_config::{FlexAssertion, FlexStep, FlexTestLimits, FlexTestScript};
use labwired_core::flex::{FlexAnalogTraceBatch, FlexDigitalTraceBatch, FlexRuntime};
use labwired_core::flex_console::FlexConsoleCapture;
use labwired_core::flex_i2c::FlexI2cTraceBatch;
use labwired_core::flex_script::{
    run_script, FlexResultConfig, FlexScriptRun, FlexStopReason, FlexTestResult,
};
use labwired_core::flex_spi::FlexSpiTraceBatch;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

pub(crate) struct FlexRunOutcome {
    pub exit_code: ExitCode,
    pub firmware_hash: String,
}

/// The existing instruments' capture, in the shape the browser snapshots use.
#[derive(Debug, Serialize)]
struct FlexTraceFile {
    digital: FlexDigitalTraceBatch,
    analog: FlexAnalogTraceBatch,
    /// Every analog channel over the whole run as change points; the scope
    /// ring above keeps only the newest samples.
    analog_series: labwired_core::analog::AnalogSeries,
    /// The transfers of the manifest's I2C buses. Absent without a bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    i2c: Option<FlexI2cTraceBatch>,
    /// The transfers of the manifest's SPI buses. Absent without a bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    spi: Option<FlexSpiTraceBatch>,
    /// The manifest console with the time of each write. Absent without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    console: Option<FlexConsoleCapture>,
}

fn unsupported_option_message(args: &TestArgs) -> Option<String> {
    let mut unsupported = Vec::new();
    if args.firmware.is_some() {
        unsupported.push("--firmware");
    }
    if args.system.is_some() {
        unsupported.push("--system");
    }
    if !args.breakpoint.is_empty() {
        unsupported.push("--breakpoint");
    }
    if args.max_cycles.is_some() {
        unsupported.push("--max-cycles");
    }
    if args.max_uart_bytes.is_some() {
        unsupported.push("--max-uart-bytes");
    }
    if args.detect_stuck.is_some() {
        unsupported.push("--detect-stuck");
    }
    if args.max_vcd_bytes.is_some() {
        unsupported.push("--max-vcd-bytes");
    }
    if args.trace || args.trace_max.is_some() {
        unsupported.push("--trace/--trace-max");
    }
    if args.bus_trace_out.is_some() {
        unsupported.push("--bus-trace-out");
    }
    if args.inspect_bytes_max != 0 {
        unsupported.push("--inspect-bytes-max");
    }
    if args.coverage {
        unsupported.push("--coverage");
    }
    if args.rom_boot || args.capture_app_entry.is_some() || args.resume_snapshot.is_some() {
        unsupported.push("--rom-boot/--capture-app-entry/--resume-snapshot");
    }
    if args.run_manifest {
        unsupported.push("--run-manifest");
    }
    if !args.watch_gpio.is_empty() {
        unsupported.push("--watch-gpio");
    }
    (!unsupported.is_empty()).then(|| {
        format!(
            "Flex test scripts do not support {}; these options need the detailed target path (inputs.system or inputs.chip)",
            unsupported.join(", ")
        )
    })
}

fn resolve_script_path(script: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        script.parent().unwrap_or_else(|| Path::new(".")).join(path)
    }
}

fn write_digital_vcd(batch: &FlexDigitalTraceBatch, path: &Path) -> std::io::Result<()> {
    let mut writer = vcd::Writer::new(std::fs::File::create(path)?);
    writer.timescale(1, vcd::TimescaleUnit::NS)?;
    writer.add_module("flex")?;
    let mut ids = Vec::with_capacity(batch.channels.len());
    for channel in &batch.channels {
        // Prefer the firmware-facing port name; internal signals keep their path.
        let name: String = channel
            .port_names
            .first()
            .unwrap_or(&channel.signal)
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        ids.push((channel.id, writer.add_wire(1, &name)?));
    }
    writer.upscope()?;
    writer.enddefinitions()?;
    let id_of = |ch: u32| ids.iter().find(|(id, _)| *id == ch).map(|(_, code)| *code);
    writer.timestamp(0)?;
    for channel in &batch.channels {
        if let Some(id) = id_of(channel.id) {
            writer.change_scalar(id, channel.initial)?;
        }
    }
    let mut time = 0;
    for edge in &batch.edges {
        if edge.time_ns != time {
            time = edge.time_ns;
            writer.timestamp(time)?;
        }
        if let Some(id) = id_of(edge.ch) {
            writer.change_scalar(id, edge.value != 0)?;
        }
    }
    Ok(())
}

fn write_junit(path: &Path, result: &FlexTestResult, duration: Duration) -> std::io::Result<()> {
    let mut tests = 1_u64;
    let mut failures = 0_u64;
    let mut errors = 0_u64;
    let details = format!(
        "result_schema_version={}\nexecution_path=flex_portable_wasm\nstop_reason={:?}\ntime_ns={}\nsteps_executed={}\nflex={}\nfirmware={}\nscript={}",
        result.result_schema_version,
        result.stop_reason,
        result.time_ns,
        result.steps_executed,
        result.config.flex.display(),
        result.config.firmware.display(),
        result.config.script.display(),
    );
    let mut cases = format!(
        "  <testcase classname=\"labwired\" name=\"run\" time=\"{:.6}\">\n",
        duration.as_secs_f64()
    );
    let message = result.message.as_deref().unwrap_or(&details);
    if result.status == "error" {
        errors += 1;
        cases.push_str(&format!(
            "    <error message=\"{:?}\">{}</error>\n",
            result.stop_reason,
            crate::xml_escape(message)
        ));
    } else if result.stop_reason != FlexStopReason::Completed {
        failures += 1;
        cases.push_str(&format!(
            "    <failure message=\"{:?}\">{}</failure>\n",
            result.stop_reason,
            crate::xml_escape(message)
        ));
    }
    cases.push_str("  </testcase>\n");
    for (index, assertion) in result.assertions.iter().enumerate() {
        tests += 1;
        let name = format!(
            "assertion {}: {}",
            index + 1,
            match &assertion.assertion {
                FlexAssertion::EntryResult(_) => "entry_result".to_owned(),
                FlexAssertion::PortValue(a) => format!("port_value {}", a.port_value.port),
                FlexAssertion::DigitalEdges(a) =>
                    format!("digital_edges {}", a.digital_edges.target()),
                FlexAssertion::AnalogSample(a) => format!(
                    "analog_sample {} at {} ns",
                    a.analog_sample.target(),
                    a.analog_sample.at_ns
                ),
                FlexAssertion::AnalogWindow(a) =>
                    format!("analog_window {}", a.analog_window.target()),
                FlexAssertion::AnalogCrossings(a) => format!(
                    "analog_crossings {} {:?} {}",
                    a.analog_crossings.target(),
                    a.analog_crossings.direction,
                    a.analog_crossings.level
                ),
                FlexAssertion::FinalTime(_) => "final_time_ns".to_owned(),
                FlexAssertion::I2cTransfers(a) => format!("i2c_transfers {}", a.i2c_transfers.bus),
                FlexAssertion::SpiTransfers(a) => format!("spi_transfers {}", a.spi_transfers.bus),
                FlexAssertion::ConsoleContains(_) => "console_contains".to_owned(),
            }
        );
        cases.push_str(&format!(
            "  <testcase classname=\"labwired\" name=\"{}\" time=\"0.000000\">\n",
            crate::xml_escape(&name)
        ));
        if !assertion.passed {
            failures += 1;
            cases.push_str(&format!(
                "    <failure message=\"assertion failed\">{}</failure>\n",
                crate::xml_escape(&format!(
                    "{name}\nobserved={:?}\nobserved_max={:?}\nobserved_times_ns={:?}\n\n{details}",
                    assertion.observed, assertion.observed_max, assertion.observed_times_ns
                ))
            ));
        }
        cases.push_str("  </testcase>\n");
    }
    std::fs::write(
        path,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"labwired\" tests=\"{tests}\" failures=\"{failures}\" errors=\"{errors}\" time=\"{:.6}\">\n{cases}\n</testsuite>\n",
            duration.as_secs_f64(),
        ),
    )
}

fn write_json<T: Serialize>(path: &Path, value: &T) {
    let written = std::fs::File::create(path)
        .map_err(|error| error.to_string())
        .and_then(|file| {
            serde_json::to_writer_pretty(file, value).map_err(|error| error.to_string())
        });
    if let Err(error) = written {
        tracing::error!("failed to write Flex artifact {:?}: {error}", path);
    }
}

fn write_artifacts(
    args: &TestArgs,
    result: &FlexTestResult,
    runtime: Option<&mut FlexRuntime>,
    duration: Duration,
) {
    let mut junit_paths: Vec<PathBuf> = args.junit.iter().cloned().collect();
    let mut trace_path = None;
    if let Some(output_dir) = &args.output_dir {
        match std::fs::create_dir_all(output_dir) {
            Ok(()) => {
                write_json(&output_dir.join("result.json"), result);
                if let Some(console) = &result.console {
                    let path = output_dir.join("console.log");
                    if let Err(error) = std::fs::write(&path, console.text.as_bytes()) {
                        tracing::error!("failed to write Flex console {path:?}: {error}");
                    }
                }
                junit_paths.push(output_dir.join("junit.xml"));
                trace_path = Some(output_dir.join("flex-trace.json"));
            }
            Err(error) => {
                tracing::error!("failed to create Flex output directory {output_dir:?}: {error}")
            }
        }
    }
    for path in junit_paths {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = write_junit(&path, result, duration) {
            tracing::error!("failed to write Flex JUnit {path:?}: {error}");
        }
    }
    let Some(runtime) = runtime else { return };
    if let Some(path) = &args.analog_trace {
        let batch = runtime.analog_engine_trace(0);
        if let Err(error) = crate::analog_trace::write_analog_trace(&batch, path) {
            tracing::error!("failed to write analog trace {path:?}: {error}");
        }
    }
    let digital = runtime.digital_trace_snapshot(0);
    if let Some(path) = &args.vcd {
        if let Err(error) = write_digital_vcd(&digital, path) {
            tracing::error!("failed to write digital VCD {path:?}: {error}");
        }
    }
    if let Some(path) = trace_path {
        let trace = FlexTraceFile {
            digital,
            analog: runtime.analog_trace_snapshot(0),
            analog_series: runtime.analog_series(),
            i2c: Some(runtime.i2c_trace_snapshot(0)).filter(|i2c| !i2c.buses.is_empty()),
            spi: Some(runtime.spi_trace_snapshot(0)).filter(|spi| !spi.buses.is_empty()),
            console: runtime.console_capture(),
        };
        write_json(&path, &trace);
    }
}

pub(crate) fn run_flex_test(args: &TestArgs, script: FlexTestScript) -> FlexRunOutcome {
    let started = Instant::now();
    let flex_path = resolve_script_path(&args.script, &script.inputs.flex);
    let firmware_path = resolve_script_path(&args.script, &script.inputs.firmware);
    let max_steps = args.max_steps.unwrap_or(script.limits.max_steps);
    let mut config = FlexResultConfig {
        script: args.script.clone(),
        flex: flex_path.clone(),
        firmware: firmware_path.clone(),
        firmware_sha256: None,
        sequence: script.steps(),
        limits: FlexTestLimits {
            max_steps,
            max_time_ns: script.limits.max_time_ns,
        },
    };

    // Each failure to read an input is a configuration error with its own
    // message.
    let inputs = (|| -> Result<(String, Vec<u8>), String> {
        if let Some(message) = unsupported_option_message(args) {
            return Err(message);
        }
        if max_steps == 0 {
            return Err("--max-steps must be > 0".to_owned());
        }
        let firmware = std::fs::read(&firmware_path)
            .map_err(|error| format!("failed to read Flex firmware {firmware_path:?}: {error}"))?;
        config.firmware_sha256 = Some(format!("{:x}", Sha256::digest(&firmware)));
        let manifest = std::fs::read_to_string(&flex_path)
            .map_err(|error| format!("failed to read Flex manifest {flex_path:?}: {error}"))?;
        Ok((manifest, firmware))
    })();
    let FlexScriptRun {
        result,
        mut runtime,
    } = match inputs {
        Ok((manifest, firmware)) => run_script(&script, config, &manifest, &firmware),
        Err(message) => FlexScriptRun {
            result: FlexTestResult::config_error(config, message),
            runtime: None,
        },
    };
    let firmware_hash = result.config.firmware_sha256.clone().unwrap_or_default();

    let exit = match (result.stop_reason, result.status) {
        (FlexStopReason::ConfigError, _) => EXIT_CONFIG_ERROR,
        (_, "pass") => EXIT_PASS,
        (_, "fail") => EXIT_ASSERT_FAIL,
        _ => EXIT_RUNTIME_ERROR,
    };

    let duration = started.elapsed();
    write_artifacts(args, &result, runtime.as_mut(), duration);

    println!(
        "Flex portable run (no target CPU emulated): status={} stop_reason={:?} time_ns={} steps_executed={}",
        result.status, result.stop_reason, result.time_ns, result.steps_executed
    );
    if let Some(message) = &result.message {
        eprintln!("{message}");
    }
    for (index, step) in result.steps.iter().enumerate() {
        let Some(time_ns) = step.time_ns else {
            println!("  step {index}: not reached");
            continue;
        };
        match &step.step {
            FlexStep::Call(call) if step.suspended => println!(
                "  step {index}: {}() suspended at {time_ns} ns",
                call.call.export
            ),
            FlexStep::Resume(_) if step.suspended => {
                println!("  step {index}: resumed, suspended again at {time_ns} ns")
            }
            FlexStep::Resume(_) => println!(
                "  step {index}: resumed, returned {:?} at {time_ns} ns",
                step.result
            ),
            FlexStep::Call(call) => println!(
                "  step {index}: {}() returned {:?} after {}/{} calls at {time_ns} ns",
                call.call.export, step.result, step.completed, call.call.repeat
            ),
            FlexStep::Set(set) => println!(
                "  step {index}: set {} = {} ({}) at {time_ns} ns",
                set.set.port,
                set.set.value,
                if step.completed == 1 {
                    "accepted"
                } else {
                    "rejected"
                }
            ),
        }
    }
    for assertion in &result.assertions {
        println!(
            "  [{}] {} observed={:?}{}",
            if assertion.passed { "PASS" } else { "FAIL" },
            serde_json::to_string(&assertion.assertion).unwrap_or_default(),
            assertion.observed,
            assertion
                .observed_times_ns
                .as_ref()
                .map(|times| {
                    // A long run can list thousands of edge times; result.json has them.
                    let shown = &times[..times.len().min(16)];
                    let more = if shown.len() < times.len() || assertion.observed_times_truncated {
                        " ..."
                    } else {
                        ""
                    };
                    format!(" at_ns={shown:?}{more}")
                })
                .unwrap_or_default()
        );
    }
    if let Some(report) = &result.run_report {
        println!(
            "scope: {} assumptions, {} unsupported checks; full-board coverage and chip parity are {}",
            report.assumptions.len(),
            report.unsupported_checks.len(),
            report.execution_scope.chip_parity
        );
    }

    FlexRunOutcome {
        exit_code: ExitCode::from(exit),
        firmware_hash,
    }
}
