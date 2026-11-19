// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired test` for portable Flex projects (`inputs.flex`).
//!
//! The firmware is a WebAssembly module whose `labwired.read/write/advance_ns`
//! imports call the same `FlexRuntime` the browser binding wraps. A script
//! runs one `entry` call or a `sequence` of calls and test-bench stimuli on
//! one firmware instance, so firmware state persists between steps. No `Cpu`,
//! `Machine`, or MMIO map is built here, and nothing in this path falls back
//! to one: a script that wants target instructions names `inputs.system`.

use crate::{TestArgs, EXIT_ASSERT_FAIL, EXIT_CONFIG_ERROR, EXIT_PASS, EXIT_RUNTIME_ERROR};
use labwired_config::{
    FlexAssertion, FlexEntryArg, FlexPortAccess, FlexPortKind, FlexStep, FlexTestLimits,
    FlexTestScript,
};
use labwired_core::flex::{
    FlexAnalogTraceBatch, FlexDigitalTraceBatch, FlexRunReport, FlexRuntime,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};
use wasmi::{
    Caller, Config, Engine, Func, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode,
    Val, ValType,
};

/// Portable firmware holds state in a few pages; this only stops a runaway
/// `memory.grow` from taking the host down.
const MAX_FIRMWARE_MEMORY_BYTES: usize = 64 << 20;

pub(crate) struct FlexRunOutcome {
    pub exit_code: ExitCode,
    pub firmware_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FlexStopReason {
    /// The script, manifest, or firmware module was rejected before any
    /// firmware instruction ran.
    ConfigError,
    /// The entry export returned.
    Completed,
    /// The firmware call used all `limits.max_steps` WebAssembly fuel.
    MaxSteps,
    /// The firmware asked to advance past `limits.max_time_ns`.
    MaxTimeNs,
    /// A port or model rejected a firmware host call.
    ModelError,
    /// The firmware module trapped by itself.
    FirmwareTrap,
}

#[derive(Debug, Serialize)]
struct FlexResultConfig {
    script: PathBuf,
    flex: PathBuf,
    firmware: PathBuf,
    firmware_sha256: Option<String>,
    /// The executed steps in order; a single `entry` is a one-call sequence.
    sequence: Vec<FlexStep>,
    limits: FlexTestLimits,
}

/// What one step of the sequence did.
#[derive(Debug, Clone, Serialize)]
struct FlexStepResult {
    step: FlexStep,
    /// Calls that returned (of `repeat`), or 1 for an applied `set`.
    completed: u32,
    /// The last returned i32 result of a call step, read as unsigned.
    result: Option<u32>,
    /// Virtual time when the step ended or the run stopped inside it; `None`
    /// for a step the run never reached.
    time_ns: Option<u64>,
}

#[derive(Debug, Serialize)]
struct FlexPortObservation {
    name: String,
    access: FlexPortAccess,
    kind: FlexPortKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
    /// Final value of a read port. `None` for write ports and for any port
    /// the stopped runtime can no longer observe.
    value: Option<f64>,
}

#[derive(Debug, Serialize)]
struct FlexAssertionResult {
    assertion: FlexAssertion,
    passed: bool,
    /// What the run produced for this assertion; `None` means nothing was
    /// observed, which never passes.
    observed: Option<f64>,
}

#[derive(Debug, Serialize)]
struct FlexTraceSummary {
    time_unit: &'static str,
    digital_channels: usize,
    digital_edges: usize,
    digital_dropped: u64,
    analog_channels: usize,
    analog_samples: usize,
    analog_dropped: u64,
}

#[derive(Debug, Serialize)]
struct FlexTestResult {
    result_schema_version: &'static str,
    status: &'static str,
    stop_reason: FlexStopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    config: FlexResultConfig,
    time_ns: u64,
    /// WebAssembly fuel the firmware call consumed. Not CPU cycles: no target
    /// CPU is emulated on this path.
    steps_executed: u64,
    /// Result of the last call step; `None` unless every step completed.
    entry_result: Option<u32>,
    steps: Vec<FlexStepResult>,
    ports: Vec<FlexPortObservation>,
    assertions: Vec<FlexAssertionResult>,
    traces: Option<FlexTraceSummary>,
    /// Same report the browser binding returns from `run_report()`.
    run_report: Option<FlexRunReport>,
}

/// The existing instruments' capture, in the shape the browser snapshots use.
#[derive(Debug, Serialize)]
struct FlexTraceFile {
    digital: FlexDigitalTraceBatch,
    analog: FlexAnalogTraceBatch,
}

struct Host {
    runtime: FlexRuntime,
    max_time_ns: u64,
    stop: Option<(FlexStopReason, String)>,
    limits: StoreLimits,
}

impl Host {
    fn stop(&mut self, reason: FlexStopReason, message: String) -> wasmi::Error {
        // The first stop is the cause; later host calls only see its echo.
        self.stop.get_or_insert((reason, message.clone()));
        wasmi::Error::new(message)
    }

    fn model<T>(&mut self, result: labwired_core::SimResult<T>) -> Result<T, wasmi::Error> {
        result.map_err(|error| self.stop(FlexStopReason::ModelError, error.to_string()))
    }
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

/// Convert one script argument to the entry export's declared parameter type.
/// Nothing is rounded or wrapped: a value the type cannot hold exactly is a
/// configuration error.
fn entry_value(
    runtime: &FlexRuntime,
    path: &str,
    index: usize,
    arg: &FlexEntryArg,
    ty: ValType,
) -> Result<Val, String> {
    match arg {
        FlexEntryArg::Port(arg) => {
            let id = runtime
                .port_id(&arg.port)
                .map_err(|error| format!("{path}.args[{index}]: {error}"))?;
            match ty {
                ValType::I32 => Ok(Val::I32(id as i32)),
                other => Err(format!(
                    "{path}.args[{index}] is port '{}', but the firmware parameter is {other:?}; port ids are i32",
                    arg.port
                )),
            }
        }
        FlexEntryArg::Number(arg) => {
            let number = arg.number;
            let integral = number.fract() == 0.0;
            match ty {
                ValType::F64 => Ok(Val::F64(number.into())),
                ValType::F32 if f64::from(number as f32) == number => {
                    Ok(Val::F32((number as f32).into()))
                }
                ValType::I32 if integral && (0.0..=f64::from(u32::MAX)).contains(&number) => {
                    Ok(Val::I32(number as u32 as i32))
                }
                ValType::I32 if integral && (f64::from(i32::MIN)..0.0).contains(&number) => {
                    Ok(Val::I32(number as i32))
                }
                ValType::I64 if integral && number.abs() <= (1u64 << 53) as f64 => {
                    Ok(Val::I64(number as i64))
                }
                other => Err(format!(
                    "{path}.args[{index}] number {number} is not exactly representable as the firmware parameter type {other:?}"
                )),
            }
        }
    }
}

struct Executed {
    stop_reason: FlexStopReason,
    message: Option<String>,
    entry_result: Option<u32>,
    steps: Vec<FlexStepResult>,
    steps_executed: u64,
    runtime: FlexRuntime,
}

/// A step resolved against the instantiated firmware and the manifest.
enum Planned {
    Call {
        func: Func,
        inputs: Vec<Val>,
        returns: bool,
        repeat: u32,
    },
    Set {
        port: u32,
        value: f64,
    },
}

/// Instantiate the firmware against `runtime` and run the script's steps in
/// order. `Err` is a configuration error: the module or a step did not fit
/// the contract, and no step ran.
fn execute(
    runtime: FlexRuntime,
    firmware: &[u8],
    script: &FlexTestScript,
    max_steps: u64,
) -> Result<Executed, String> {
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let host = Host {
        runtime,
        max_time_ns: script.limits.max_time_ns,
        stop: None,
        limits: StoreLimitsBuilder::new()
            .memory_size(MAX_FIRMWARE_MEMORY_BYTES)
            .build(),
    };
    let mut store = Store::new(&engine, host);
    store.limiter(|host| &mut host.limits);
    let module = match Module::new(&engine, firmware) {
        Ok(module) => module,
        Err(error) => {
            return Err(format!(
                "firmware is not a valid WebAssembly module: {error}"
            ))
        }
    };
    let mut linker = <Linker<Host>>::new(&engine);
    let defined = linker
        .func_wrap(
            "labwired",
            "read",
            |mut caller: Caller<'_, Host>, port: i32| -> Result<f64, wasmi::Error> {
                let host = caller.data_mut();
                let value = host.runtime.read(port as u32);
                host.model(value)
            },
        )
        .and_then(|linker| {
            linker.func_wrap(
                "labwired",
                "write",
                |mut caller: Caller<'_, Host>, port: i32, value: f64| -> Result<(), wasmi::Error> {
                    let host = caller.data_mut();
                    let result = host.runtime.write(port as u32, value);
                    host.model(result)
                },
            )
        })
        .and_then(|linker| {
            linker.func_wrap(
                "labwired",
                "advance_ns",
                |mut caller: Caller<'_, Host>, delta: i32| -> Result<(), wasmi::Error> {
                    let host = caller.data_mut();
                    // The portable ABI passes an unsigned 32-bit delta.
                    let delta = u64::from(delta as u32);
                    if host.runtime.time_ns().saturating_add(delta) > host.max_time_ns {
                        let message = format!(
                            "firmware advanced past limits.max_time_ns ({} ns) at {} ns",
                            host.max_time_ns,
                            host.runtime.time_ns()
                        );
                        return Err(host.stop(FlexStopReason::MaxTimeNs, message));
                    }
                    let result = host.runtime.advance_ns(delta);
                    host.model(result)
                },
            )
        })
        .map(|_| ());
    if let Err(error) = defined {
        return Err(format!(
            "failed to define the labwired host imports: {error}"
        ));
    }

    // Fuel covers a start function too, so it cannot spin before the entry.
    if let Err(error) = store.set_fuel(max_steps) {
        return Err(format!("failed to set the firmware step budget: {error}"));
    }
    let steps = script.steps();
    let mut results: Vec<FlexStepResult> = steps
        .iter()
        .map(|step| FlexStepResult {
            step: step.clone(),
            completed: 0,
            result: None,
            time_ns: None,
        })
        .collect();
    let mut run = |store: &mut Store<Host>| -> Result<Result<(), wasmi::Error>, String> {
        let instance = match linker.instantiate_and_start(&mut *store, &module) {
            Ok(instance) => instance,
            // A trapping start function is firmware behavior, not bad config.
            Err(error) if error.as_trap_code().is_some() || store.data().stop.is_some() => {
                return Ok(Err(error))
            }
            Err(error) => {
                return Err(format!(
                    "firmware does not fit the portable Flex ABI (imports labwired.read(i32) -> f64, labwired.write(i32, f64), labwired.advance_ns(i32)): {error}"
                ))
            }
        };
        // Resolve every step before the first one runs, so a contract
        // mismatch late in the sequence is still a configuration error.
        let mut planned = Vec::with_capacity(steps.len());
        for (index, step) in steps.iter().enumerate() {
            let path = script.step_path(index);
            match step {
                FlexStep::Call(step) => {
                    let export = &step.call.export;
                    let Some(func) = instance.get_func(&*store, export) else {
                        return Err(format!(
                            "firmware has no exported function '{export}' ({path}.export)"
                        ));
                    };
                    let ty = func.ty(&*store);
                    if ty.params().len() != step.call.args.len() {
                        return Err(format!(
                            "firmware export '{export}' takes {} parameters, but {path}.args has {}",
                            ty.params().len(),
                            step.call.args.len()
                        ));
                    }
                    let mut inputs = Vec::with_capacity(ty.params().len());
                    for (index, (arg, param)) in step.call.args.iter().zip(ty.params()).enumerate()
                    {
                        inputs.push(entry_value(
                            &store.data().runtime,
                            &path,
                            index,
                            arg,
                            *param,
                        )?);
                    }
                    let returns = match ty.results() {
                        [] => false,
                        [ValType::I32] => true,
                        other => {
                            return Err(format!(
                                "firmware export '{export}' returns {other:?}; supported results are none or one i32"
                            ))
                        }
                    };
                    planned.push(Planned::Call {
                        func,
                        inputs,
                        returns,
                        repeat: step.call.repeat,
                    });
                }
                FlexStep::Set(step) => {
                    let runtime = &store.data().runtime;
                    let name = &step.set.port;
                    let port = runtime
                        .port_id(name)
                        .map_err(|error| format!("{path}: {error}"))?;
                    let writable = runtime
                        .manifest()
                        .ports
                        .iter()
                        .any(|port| &port.name == name && port.access == FlexPortAccess::Write);
                    if !writable {
                        return Err(format!(
                            "{path} needs a write port; '{name}' is a read port"
                        ));
                    }
                    planned.push(Planned::Set {
                        port,
                        value: step.set.value,
                    });
                }
            }
        }
        for (planned, result) in planned.iter().zip(results.iter_mut()) {
            let outcome = match planned {
                Planned::Call {
                    func,
                    inputs,
                    returns,
                    repeat,
                } => (0..*repeat).try_for_each(|_| {
                    let mut outputs = if *returns {
                        vec![Val::I32(0)]
                    } else {
                        Vec::new()
                    };
                    func.call(&mut *store, inputs, &mut outputs)?;
                    result.completed += 1;
                    if let Some(Val::I32(value)) = outputs.first() {
                        result.result = Some(*value as u32);
                    }
                    Ok(())
                }),
                Planned::Set { port, value } => {
                    let host = store.data_mut();
                    let written = host.runtime.write(*port, *value);
                    host.model(written).map(|()| result.completed = 1)
                }
            };
            result.time_ns = Some(store.data().runtime.time_ns());
            if let Err(error) = outcome {
                return Ok(Err(error));
            }
        }
        Ok(Ok(()))
    };
    let called = run(&mut store);
    let steps_executed = max_steps.saturating_sub(store.get_fuel().unwrap_or(0));
    let host = store.into_data();
    let called = called?;
    let (stop_reason, message, entry_result) = match called {
        Ok(()) => (
            FlexStopReason::Completed,
            None,
            results
                .iter()
                .rev()
                .find(|result| matches!(result.step, FlexStep::Call(_)))
                .and_then(|result| result.result),
        ),
        Err(error) => match host.stop {
            Some((reason, message)) => (reason, Some(message), None),
            None if error.as_trap_code() == Some(TrapCode::OutOfFuel) => (
                FlexStopReason::MaxSteps,
                Some(format!(
                    "firmware used all limits.max_steps ({max_steps}) WebAssembly fuel"
                )),
                None,
            ),
            None => (
                FlexStopReason::FirmwareTrap,
                Some(format!("firmware trapped: {error}")),
                None,
            ),
        },
    };
    Ok(Executed {
        stop_reason,
        message,
        entry_result,
        steps: results,
        steps_executed,
        runtime: host.runtime,
    })
}

fn port_observations(runtime: &FlexRuntime) -> Vec<FlexPortObservation> {
    runtime
        .manifest()
        .ports
        .iter()
        .map(|port| FlexPortObservation {
            name: port.name.clone(),
            access: port.access,
            kind: port.kind,
            unit: port.unit.clone(),
            value: (port.access == FlexPortAccess::Read)
                .then(|| {
                    runtime
                        .port_id(&port.name)
                        .and_then(|id| runtime.read(id))
                        .ok()
                })
                .flatten(),
        })
        .collect()
}

fn evaluate(
    assertion: &FlexAssertion,
    entry_result: Option<u32>,
    steps: &[FlexStepResult],
    ports: &[FlexPortObservation],
    digital: &FlexDigitalTraceBatch,
) -> FlexAssertionResult {
    let (observed, passed) = match assertion {
        FlexAssertion::EntryResult(assertion) => {
            let observed = match assertion.entry_result.step {
                None => entry_result,
                // A call step that did not return every repeat has no result.
                Some(index) => steps.get(index).and_then(|step| match &step.step {
                    FlexStep::Call(call) if step.completed == call.call.repeat => step.result,
                    _ => None,
                }),
            };
            (
                observed.map(f64::from),
                observed == Some(assertion.entry_result.equals),
            )
        }
        FlexAssertion::PortValue(assertion) => {
            let details = &assertion.port_value;
            let observed = ports
                .iter()
                .find(|port| port.name == details.port)
                .and_then(|port| port.value);
            let passed = observed.is_some_and(|value| {
                details.equals.is_none_or(|equals| value == equals)
                    && details.min.is_none_or(|min| value >= min)
                    && details.max.is_none_or(|max| value <= max)
            });
            (observed, passed)
        }
        FlexAssertion::DigitalEdges(assertion) => {
            let details = &assertion.digital_edges;
            // A gap in the ring buffer makes any exact count unknowable.
            let observed = (digital.dropped == 0)
                .then(|| {
                    digital
                        .channels
                        .iter()
                        .find(|channel| channel.port_names.contains(&details.port))
                })
                .flatten()
                .map(|channel| {
                    digital
                        .edges
                        .iter()
                        .filter(|edge| edge.ch == channel.id)
                        .count() as u64
                });
            (
                observed.map(|count| count as f64),
                observed == Some(details.count),
            )
        }
    };
    FlexAssertionResult {
        assertion: assertion.clone(),
        passed,
        observed,
    }
}

/// Every assertion must name a port the run can observe that way. Checked
/// before the firmware runs so a typo is a configuration error, not a
/// silently failing assertion.
fn validate_assertion_ports(runtime: &FlexRuntime, script: &FlexTestScript) -> Result<(), String> {
    let ports = &runtime.manifest().ports;
    for (index, assertion) in script.assertions.iter().enumerate() {
        let (name, kind, field) = match assertion {
            FlexAssertion::EntryResult(_) => continue,
            FlexAssertion::PortValue(assertion) => (&assertion.port_value.port, None, "port_value"),
            FlexAssertion::DigitalEdges(assertion) => (
                &assertion.digital_edges.port,
                Some(FlexPortKind::Bool),
                "digital_edges",
            ),
        };
        let Some(port) = ports.iter().find(|port| &port.name == name) else {
            return Err(format!(
                "assertions[{index}].{field} names unknown Flex port '{name}'"
            ));
        };
        if field == "port_value" && port.access != FlexPortAccess::Read {
            return Err(format!(
                "assertions[{index}].port_value needs a read port; '{name}' is a write port"
            ));
        }
        if kind.is_some_and(|kind| kind != port.kind) {
            return Err(format!(
                "assertions[{index}].digital_edges needs a bool port; '{name}' is numeric"
            ));
        }
    }
    Ok(())
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
                FlexAssertion::DigitalEdges(a) => format!("digital_edges {}", a.digital_edges.port),
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
                    "{name}\nobserved={:?}\n\n{details}",
                    assertion.observed
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
        };
        write_json(&path, &trace);
    }
}

pub(crate) fn run_flex_test(args: &TestArgs, script: FlexTestScript) -> FlexRunOutcome {
    let started = Instant::now();
    let flex_path = resolve_script_path(&args.script, &script.inputs.flex);
    let firmware_path = resolve_script_path(&args.script, &script.inputs.firmware);
    let max_steps = args.max_steps.unwrap_or(script.limits.max_steps);
    let mut result = FlexTestResult {
        result_schema_version: "1.0",
        status: "error",
        stop_reason: FlexStopReason::ConfigError,
        message: None,
        config: FlexResultConfig {
            script: args.script.clone(),
            flex: flex_path.clone(),
            firmware: firmware_path.clone(),
            firmware_sha256: None,
            sequence: script.steps(),
            limits: FlexTestLimits {
                max_steps,
                max_time_ns: script.limits.max_time_ns,
            },
        },
        time_ns: 0,
        steps_executed: 0,
        entry_result: None,
        steps: Vec::new(),
        ports: Vec::new(),
        assertions: Vec::new(),
        traces: None,
        run_report: None,
    };

    // Load everything before running anything; each failure here is a
    // configuration error with its own message.
    let loaded = (|| -> Result<(FlexRuntime, Vec<u8>), String> {
        if let Some(message) = unsupported_option_message(args) {
            return Err(message);
        }
        if max_steps == 0 {
            return Err("--max-steps must be > 0".to_owned());
        }
        let firmware = std::fs::read(&firmware_path)
            .map_err(|error| format!("failed to read Flex firmware {firmware_path:?}: {error}"))?;
        result.config.firmware_sha256 = Some(format!("{:x}", Sha256::digest(&firmware)));
        let manifest = std::fs::read_to_string(&flex_path)
            .map_err(|error| format!("failed to read Flex manifest {flex_path:?}: {error}"))?;
        let runtime = FlexRuntime::from_yaml(&manifest)
            .map_err(|error| format!("failed to load Flex manifest {flex_path:?}: {error}"))?;
        validate_assertion_ports(&runtime, &script)?;
        Ok((runtime, firmware))
    })();
    let executed = loaded.and_then(|(runtime, firmware)| {
        result.run_report = Some(runtime.run_report());
        execute(runtime, &firmware, &script, max_steps)
    });
    let firmware_hash = result.config.firmware_sha256.clone().unwrap_or_default();

    let mut runtime = match executed {
        Ok(executed) => {
            result.stop_reason = executed.stop_reason;
            result.message = executed.message;
            result.entry_result = executed.entry_result;
            result.steps = executed.steps;
            result.steps_executed = executed.steps_executed;
            Some(executed.runtime)
        }
        Err(message) => {
            result.message = Some(message);
            None
        }
    };
    if let Some(runtime) = runtime.as_mut() {
        result.time_ns = runtime.time_ns();
        result.ports = port_observations(runtime);
        // Cursor 0 re-reads the whole retained ring; the artifact writer
        // takes the same snapshot again.
        let digital = runtime.digital_trace_snapshot(0);
        let analog = runtime.analog_trace_snapshot(0);
        result.assertions = script
            .assertions
            .iter()
            .map(|assertion| {
                evaluate(
                    assertion,
                    result.entry_result,
                    &result.steps,
                    &result.ports,
                    &digital,
                )
            })
            .collect();
        result.traces = Some(FlexTraceSummary {
            time_unit: "ns",
            digital_channels: digital.channels.len(),
            digital_edges: digital.edges.len(),
            digital_dropped: digital.dropped,
            analog_channels: analog.channels.len(),
            analog_samples: analog.samples.len(),
            analog_dropped: analog.dropped,
        });
    }

    let exit = match result.stop_reason {
        FlexStopReason::ConfigError => EXIT_CONFIG_ERROR,
        FlexStopReason::ModelError | FlexStopReason::FirmwareTrap => EXIT_RUNTIME_ERROR,
        // A limit stop cannot certify a pass, whatever the assertions say.
        FlexStopReason::MaxSteps | FlexStopReason::MaxTimeNs => EXIT_ASSERT_FAIL,
        FlexStopReason::Completed if result.assertions.iter().all(|a| a.passed) => EXIT_PASS,
        FlexStopReason::Completed => EXIT_ASSERT_FAIL,
    };
    result.status = match exit {
        EXIT_PASS => "pass",
        EXIT_ASSERT_FAIL => "fail",
        _ => "error",
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
            "  [{}] {} observed={:?}",
            if assertion.passed { "PASS" } else { "FAIL" },
            serde_json::to_string(&assertion.assertion).unwrap_or_default(),
            assertion.observed
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
