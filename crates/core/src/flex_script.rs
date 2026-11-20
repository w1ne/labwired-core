// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Runner for Flex test scripts, shared by `labwired test` and the browser
//! binding so one script gives one result on both hosts.
//!
//! The firmware is a WebAssembly module whose `labwired.read/write/advance_ns`
//! imports call a [`FlexRuntime`]. A script runs one `entry` call or a
//! `sequence` of calls and test-bench stimuli on one firmware instance, so
//! firmware state persists between steps. The module runs in `wasmi` on both
//! hosts: its fuel is the step budget, and a resumable call is what lets an
//! `until_ns` suspend firmware whose `main` never returns. No `Cpu`,
//! `Machine`, or MMIO map is built here, and nothing in this path falls back
//! to one.

use crate::analog::{AnalogCrossing, AnalogWindowTally};
use crate::flex::{
    FlexAnalogTraceBatch, FlexDigitalChannel, FlexDigitalEdgeTally, FlexDigitalTraceBatch,
    FlexRunReport, FlexRuntime,
};
use labwired_config::{
    FlexAssertion, FlexCrossingDirection, FlexDigitalEdgesDetails, FlexEntryArg, FlexPort,
    FlexPortAccess, FlexPortKind, FlexStep, FlexTestLimits, FlexTestScript,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use wasmi::{
    Caller, Config, Engine, Func, Linker, Module, ResumableCall, ResumableCallHostTrap, Store,
    StoreLimits, StoreLimitsBuilder, TrapCode, Val, ValType,
};

/// Portable firmware holds state in a few pages; this only stops a runaway
/// `memory.grow` from taking the host down.
const MAX_FIRMWARE_MEMORY_BYTES: usize = 64 << 20;
/// Edge times `result.json` lists for one `digital_edges` assertion. The edge
/// count is exact beyond this; only the list of times stops.
const MAX_REPORTED_EDGE_TIMES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlexStopReason {
    /// The script, manifest, or firmware module was rejected before any
    /// firmware instruction ran.
    ConfigError,
    /// Every step ran: each call returned or was suspended at its `until_ns`.
    Completed,
    /// The firmware call used all `limits.max_steps` WebAssembly fuel.
    MaxSteps,
    /// The firmware asked to advance past `limits.max_time_ns`.
    MaxTimeNs,
    /// A port or model rejected a firmware host call.
    ModelError,
    /// A step could not run in the state the earlier steps left: a `resume`
    /// after the firmware returned, or an `until_ns` that is not in the future.
    SequenceError,
    /// The firmware module trapped by itself.
    FirmwareTrap,
}

#[derive(Debug, Serialize)]
pub struct FlexResultConfig {
    pub script: PathBuf,
    pub flex: PathBuf,
    pub firmware: PathBuf,
    pub firmware_sha256: Option<String>,
    /// The executed steps in order; a single `entry` is a one-call sequence.
    pub sequence: Vec<FlexStep>,
    pub limits: FlexTestLimits,
}

/// What one step of the sequence did.
#[derive(Debug, Clone, Serialize)]
pub struct FlexStepResult {
    pub step: FlexStep,
    /// Calls that returned (of `repeat`), or 1 for an applied `set`. A
    /// `resume` counts the return of the call it continued.
    pub completed: u32,
    /// The step ended at its `until_ns` with the firmware suspended in the call.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub suspended: bool,
    /// The last returned i32 result of a call step, read as unsigned.
    pub result: Option<u32>,
    /// Virtual time when the step ended or the run stopped inside it; `None`
    /// for a step the run never reached.
    pub time_ns: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct FlexPortObservation {
    pub name: String,
    pub access: FlexPortAccess,
    pub kind: FlexPortKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Final value of a read port. `None` for write ports and for any port
    /// the stopped runtime can no longer observe.
    pub value: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct FlexAssertionResult {
    pub assertion: FlexAssertion,
    pub passed: bool,
    /// What the run produced for this assertion; `None` means nothing was
    /// observed, which never passes.
    pub observed: Option<f64>,
    /// The highest value of an `analog_window`; `observed` is its lowest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_max: Option<f64>,
    /// The edge times of a `digital_edges` window, the crossing times of
    /// `analog_crossings`, the time of the sample an `analog_sample` read, or
    /// the times of the lowest and highest value of an `analog_window`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_times_ns: Option<Vec<u64>>,
    /// `observed_times_ns` holds only the first times of the window;
    /// `observed` is still the exact number of edges or crossings.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub observed_times_truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct FlexTraceSummary {
    pub time_unit: &'static str,
    pub digital_channels: usize,
    pub digital_edges: usize,
    pub digital_dropped: u64,
    pub analog_channels: usize,
    pub analog_samples: usize,
    pub analog_dropped: u64,
}

#[derive(Debug, Serialize)]
pub struct FlexTestResult {
    pub result_schema_version: &'static str,
    pub status: &'static str,
    pub stop_reason: FlexStopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub config: FlexResultConfig,
    pub time_ns: u64,
    /// WebAssembly fuel the firmware call consumed. Not CPU cycles: no target
    /// CPU is emulated on this path.
    pub steps_executed: u64,
    /// Result of the last call or resume step; `None` unless every step
    /// completed and that step saw the firmware return.
    pub entry_result: Option<u32>,
    pub steps: Vec<FlexStepResult>,
    pub ports: Vec<FlexPortObservation>,
    pub assertions: Vec<FlexAssertionResult>,
    pub traces: Option<FlexTraceSummary>,
    /// Same report the browser binding returns from `run_report()`.
    pub run_report: Option<FlexRunReport>,
}

struct Host {
    runtime: FlexRuntime,
    max_time_ns: u64,
    stop: Option<(FlexStopReason, String)>,
    /// The `until_ns` of the running step.
    until_ns: Option<u64>,
    /// Set by the `advance_ns` that reached `until_ns`; the host error it
    /// returns then suspends the call and is not a stop.
    suspended: bool,
    /// What the suspended `advance_ns` still has to advance after `until_ns`.
    pending_ns: u64,
    limits: StoreLimits,
}

impl Host {
    fn stop(&mut self, reason: FlexStopReason, message: String) -> wasmi::Error {
        // The first stop is the cause; later host calls only see its echo.
        self.stop.get_or_insert((reason, message.clone()));
        wasmi::Error::new(message)
    }

    fn model<T>(&mut self, result: crate::SimResult<T>) -> Result<T, wasmi::Error> {
        result.map_err(|error| self.stop(FlexStopReason::ModelError, error.to_string()))
    }

    /// Advance to the step's `until_ns` if `delta` reaches it, and keep the
    /// rest of `delta` for the `resume`. `Ok(false)`: `until_ns` is further.
    fn advance_to_until(&mut self, delta: u64) -> Result<bool, wasmi::Error> {
        let now = self.runtime.time_ns();
        let Some(until) = self
            .until_ns
            .filter(|until| now.saturating_add(delta) >= *until)
        else {
            return Ok(false);
        };
        let result = self.runtime.advance_ns(until - now);
        self.model(result)?;
        self.pending_ns = delta - (until - now);
        self.suspended = true;
        Ok(true)
    }
}

/// Settle a resumable call: `Ok(Some(trap))` is the firmware suspended at the
/// step's `until_ns`, `Ok(None)` a call that returned.
fn settle(
    store: &mut Store<Host>,
    call: Result<ResumableCall, wasmi::Error>,
) -> Result<Option<ResumableCallHostTrap>, wasmi::Error> {
    match call? {
        ResumableCall::Finished => Ok(None),
        ResumableCall::HostTrap(trap) if std::mem::take(&mut store.data_mut().suspended) => {
            Ok(Some(trap))
        }
        ResumableCall::HostTrap(trap) => Err(trap.into_host_error()),
        ResumableCall::OutOfFuel(_) => Err(wasmi::Error::from(TrapCode::OutOfFuel)),
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
        until_ns: Option<u64>,
    },
    Set {
        port: u32,
        value: f64,
    },
    Resume {
        until_ns: u64,
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
        until_ns: None,
        suspended: false,
        pending_ns: 0,
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
                    if host.advance_to_until(delta)? {
                        return Err(wasmi::Error::new("suspended at until_ns"));
                    }
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
            suspended: false,
            result: None,
            time_ns: None,
        })
        .collect();
    // The call an `until_ns` suspended, and whether it returns an i32.
    let mut suspended: Option<(ResumableCallHostTrap, bool)> = None;
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
                        until_ns: step.call.until_ns,
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
                FlexStep::Resume(step) => planned.push(Planned::Resume {
                    until_ns: step.resume.until_ns,
                }),
            }
        }
        for (index, (planned, result)) in planned.iter().zip(results.iter_mut()).enumerate() {
            let outcome = match planned {
                Planned::Call {
                    func,
                    inputs,
                    returns,
                    until_ns: Some(until_ns),
                    ..
                } => {
                    let now = store.data().runtime.time_ns();
                    if *until_ns <= now {
                        let message = format!(
                            "{}.until_ns ({until_ns} ns) is not after the current time ({now} ns)",
                            script.step_path(index)
                        );
                        Err(store
                            .data_mut()
                            .stop(FlexStopReason::SequenceError, message))
                    } else {
                        let mut outputs = vec![Val::I32(0); usize::from(*returns)];
                        store.data_mut().until_ns = Some(*until_ns);
                        let call = func.call_resumable(&mut *store, inputs, &mut outputs);
                        let settled = settle(&mut *store, call);
                        store.data_mut().until_ns = None;
                        settled.map(|trap| match trap {
                            Some(trap) => {
                                suspended = Some((trap, *returns));
                                result.suspended = true;
                            }
                            None => {
                                result.completed = 1;
                                if let Some(Val::I32(value)) = outputs.first() {
                                    result.result = Some(*value as u32);
                                }
                            }
                        })
                    }
                }
                Planned::Resume { until_ns } => match suspended.take() {
                    None => {
                        let message = format!(
                            "{} has no suspended call: the firmware returned before {} ns",
                            script.step_path(index),
                            store.data().runtime.time_ns()
                        );
                        Err(store
                            .data_mut()
                            .stop(FlexStopReason::SequenceError, message))
                    }
                    Some((trap, returns)) => {
                        let host = store.data_mut();
                        host.until_ns = Some(*until_ns);
                        // First the rest of the `advance_ns` the call is suspended in.
                        let pending = std::mem::take(&mut host.pending_ns);
                        let settled = match host.advance_to_until(pending) {
                            Err(error) => Err(error),
                            Ok(true) => {
                                host.suspended = false;
                                Ok(Some(trap))
                            }
                            Ok(false) => {
                                let advanced = host.runtime.advance_ns(pending);
                                host.model(advanced).and_then(|()| {
                                    let mut outputs = vec![Val::I32(0); usize::from(returns)];
                                    let call = trap.resume(&mut *store, &[], &mut outputs);
                                    settle(&mut *store, call).inspect(|trap| {
                                        if trap.is_none() {
                                            result.completed = 1;
                                            if let Some(Val::I32(value)) = outputs.first() {
                                                result.result = Some(*value as u32);
                                            }
                                        }
                                    })
                                })
                            }
                        };
                        store.data_mut().until_ns = None;
                        settled.map(|trap| {
                            result.suspended = trap.is_some();
                            suspended = trap.map(|trap| (trap, returns));
                        })
                    }
                },
                Planned::Call {
                    func,
                    inputs,
                    returns,
                    repeat,
                    until_ns: None,
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
                .find(|result| !matches!(result.step, FlexStep::Set(_)))
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

/// The logic-analyzer channel a `digital_edges` assertion names.
fn digital_channel<'a>(
    digital: &'a FlexDigitalTraceBatch,
    details: &FlexDigitalEdgesDetails,
) -> Option<&'a FlexDigitalChannel> {
    digital.channels.iter().find(|channel| match &details.port {
        Some(port) => channel.port_names.contains(port),
        None => details.signal.as_ref() == Some(&channel.signal),
    })
}

/// The oscilloscope column an analog assertion names, by port or by channel.
fn analog_channel(
    index: usize,
    kind: &str,
    port: &Option<String>,
    channel: &Option<String>,
    ports: &[FlexPort],
    analog: &FlexAnalogTraceBatch,
) -> Result<usize, String> {
    let name = port.as_deref().or(channel.as_deref()).unwrap_or("");
    if let Some(port) = port {
        let Some(port) = ports.iter().find(|candidate| &candidate.name == port) else {
            return Err(format!(
                "assertions[{index}].{kind} names unknown Flex port '{name}'"
            ));
        };
        if port.kind != FlexPortKind::Number || port.access != FlexPortAccess::Read {
            return Err(format!(
                "assertions[{index}].{kind} needs a numeric read port; '{name}' is not one"
            ));
        }
    }
    analog
        .channels
        .iter()
        .position(|candidate| match port {
            Some(port) => candidate.port_names.contains(port),
            None => channel.as_ref() == Some(&candidate.name),
        })
        .ok_or_else(|| {
            format!(
                "assertions[{index}].{kind}: the oscilloscope has no channel for '{name}'; its channels are [{}]",
                analog
                    .channels
                    .iter()
                    .map(|channel| channel.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn within<T: PartialOrd + Copy>(
    value: T,
    equals: Option<T>,
    min: Option<T>,
    max: Option<T>,
) -> bool {
    equals.is_none_or(|equals| value == equals)
        && min.is_none_or(|min| value >= min)
        && max.is_none_or(|max| value <= max)
}

fn evaluate(
    assertion: &FlexAssertion,
    run: &FlexTestResult,
    tally: Option<&FlexDigitalEdgeTally>,
    analog: Option<&AnalogWindowTally>,
) -> FlexAssertionResult {
    let mut observed_max = None;
    let mut observed_times_ns = None;
    let mut observed_times_truncated = false;
    let (observed, passed) = match assertion {
        FlexAssertion::EntryResult(assertion) => {
            let observed = match assertion.entry_result.step {
                None => run.entry_result,
                // A call step that did not return every repeat has no result.
                Some(index) => run.steps.get(index).and_then(|step| match &step.step {
                    FlexStep::Call(call) if step.completed == call.call.repeat => step.result,
                    FlexStep::Resume(_) => step.result,
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
            let observed = run
                .ports
                .iter()
                .find(|port| port.name == details.port)
                .and_then(|port| port.value);
            let passed = observed
                .is_some_and(|value| within(value, details.equals, details.min, details.max));
            (observed, passed)
        }
        FlexAssertion::DigitalEdges(assertion) => {
            let details = &assertion.digital_edges;
            // The tally saw every edge when it was recorded, so the count is
            // exact after the capture ring drops its oldest edges. A window
            // the run did not reach the end of was not observed.
            let tally = tally.filter(|_| details.to_ns.is_none_or(|to_ns| to_ns <= run.time_ns));
            let passed = tally.is_some_and(|tally| {
                details.count.is_none_or(|count| tally.count == count)
                    && details.times_ns.as_ref().is_none_or(|expected| {
                        tally.count == expected.len() as u64 && &tally.times_ns == expected
                    })
            });
            observed_times_truncated =
                tally.is_some_and(|tally| tally.count > tally.times_ns.len() as u64);
            observed_times_ns = tally.map(|tally| tally.times_ns.clone());
            (tally.map(|tally| tally.count as f64), passed)
        }
        FlexAssertion::AnalogSample(assertion) => {
            let details = &assertion.analog_sample;
            // The oscilloscope holds a value until its next sample. The tally
            // kept the sample when it was written, so a long run that
            // overflows the ring still has it. A time the run never reached
            // has none.
            let sample = analog
                .filter(|_| details.at_ns <= run.time_ns)
                .and_then(|tally| tally.last);
            observed_times_ns = sample.map(|(time_ns, _)| vec![time_ns]);
            // Samples are f32; compare in f32 so `equals: 2.4` means the
            // value the oscilloscope shows.
            let passed = sample.is_some_and(|(_, value)| {
                within(
                    value,
                    details.equals.map(|bound| bound as f32),
                    details.min.map(|bound| bound as f32),
                    details.max.map(|bound| bound as f32),
                )
            });
            (sample.map(|(_, value)| f64::from(value)), passed)
        }
        FlexAssertion::AnalogWindow(assertion) => {
            let details = &assertion.analog_window;
            // A window the run did not reach the end of was not observed.
            let range = analog
                .filter(|_| details.to_ns.is_none_or(|to_ns| to_ns <= run.time_ns))
                .and_then(|tally| tally.min.zip(tally.max));
            observed_max = range.map(|(_, (_, max))| f64::from(max));
            observed_times_ns = range.map(|((min_ns, _), (max_ns, _))| vec![min_ns, max_ns]);
            let passed = range.is_some_and(|((_, min), (_, max))| {
                details.min.is_none_or(|bound| min >= bound as f32)
                    && details.max.is_none_or(|bound| max <= bound as f32)
            });
            (range.map(|((_, min), _)| f64::from(min)), passed)
        }
        FlexAssertion::AnalogCrossings(assertion) => {
            let details = &assertion.analog_crossings;
            let tally = analog.filter(|_| details.to_ns.is_none_or(|to_ns| to_ns <= run.time_ns));
            let passed = tally.is_some_and(|tally| {
                details.count.is_none_or(|count| tally.crossings == count)
                    && details.times_ns.as_ref().is_none_or(|expected| {
                        tally.crossings == expected.len() as u64
                            && &tally.crossing_times_ns == expected
                    })
            });
            observed_times_truncated =
                tally.is_some_and(|tally| tally.crossings > tally.crossing_times_ns.len() as u64);
            observed_times_ns = tally.map(|tally| tally.crossing_times_ns.clone());
            (tally.map(|tally| tally.crossings as f64), passed)
        }
        FlexAssertion::FinalTime(assertion) => {
            let details = &assertion.final_time_ns;
            (
                Some(run.time_ns as f64),
                within(run.time_ns, details.equals, details.min, details.max),
            )
        }
    };
    FlexAssertionResult {
        assertion: assertion.clone(),
        passed,
        observed,
        observed_max,
        observed_times_ns,
        observed_times_truncated,
    }
}

/// Every assertion must name a port or channel the run can observe that way.
/// Checked before the firmware runs so a typo is a configuration error, not a
/// silently failing assertion. Returns, per assertion, the id of the tally an
/// instrument assertion reads.
fn validate_assertion_ports(
    runtime: &mut FlexRuntime,
    script: &FlexTestScript,
) -> Result<Vec<Option<usize>>, String> {
    let mut tallies = vec![None; script.assertions.len()];
    // Instrument channels are fixed by the manifest, so they are known at 0 ns.
    let digital = runtime.digital_trace_snapshot(0);
    let analog = runtime.analog_trace_snapshot(0);
    let ports = runtime.manifest().ports.clone();
    for (index, assertion) in script.assertions.iter().enumerate() {
        match assertion {
            FlexAssertion::EntryResult(_) | FlexAssertion::FinalTime(_) => {}
            FlexAssertion::PortValue(assertion) => {
                let name = &assertion.port_value.port;
                let Some(port) = ports.iter().find(|port| &port.name == name) else {
                    return Err(format!(
                        "assertions[{index}].port_value names unknown Flex port '{name}'"
                    ));
                };
                if port.access != FlexPortAccess::Read {
                    return Err(format!(
                        "assertions[{index}].port_value needs a read port; '{name}' is a write port"
                    ));
                }
            }
            FlexAssertion::DigitalEdges(assertion) => {
                let details = &assertion.digital_edges;
                let name = details.target();
                if let Some(port) = &details.port {
                    let Some(port) = ports.iter().find(|candidate| &candidate.name == port) else {
                        return Err(format!(
                            "assertions[{index}].digital_edges names unknown Flex port '{name}'"
                        ));
                    };
                    if port.kind != FlexPortKind::Bool {
                        return Err(format!(
                            "assertions[{index}].digital_edges needs a bool port; '{name}' is numeric"
                        ));
                    }
                }
                let Some(channel) = digital_channel(&digital, details) else {
                    return Err(format!(
                        "assertions[{index}].digital_edges: the logic analyzer has no channel for '{name}'; its channels are [{}]",
                        digital
                            .channels
                            .iter()
                            .map(|channel| channel.signal.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                };
                // One time more than expected shows an extra edge.
                let max_times = details
                    .times_ns
                    .as_ref()
                    .map_or(0, |times| times.len() + 1)
                    .max(MAX_REPORTED_EDGE_TIMES);
                let tally = runtime
                    .tally_digital_edges(
                        channel.id,
                        details.from_ns.unwrap_or(0),
                        details.to_ns.unwrap_or(u64::MAX),
                        max_times,
                    )
                    .map_err(|error| format!("assertions[{index}].digital_edges: {error}"))?;
                tallies[index] = Some(tally);
            }
            FlexAssertion::AnalogSample(assertion) => {
                let details = &assertion.analog_sample;
                let column = analog_channel(
                    index,
                    "analog_sample",
                    &details.port,
                    &details.channel,
                    &ports,
                    &analog,
                )?;
                let tally = runtime
                    .tally_analog_window(column, details.at_ns, details.at_ns, None, 0)
                    .map_err(|error| format!("assertions[{index}].analog_sample: {error}"))?;
                tallies[index] = Some(tally);
            }
            FlexAssertion::AnalogWindow(assertion) => {
                let details = &assertion.analog_window;
                let column = analog_channel(
                    index,
                    "analog_window",
                    &details.port,
                    &details.channel,
                    &ports,
                    &analog,
                )?;
                let tally = runtime
                    .tally_analog_window(
                        column,
                        details.from_ns.unwrap_or(0),
                        details.to_ns.unwrap_or(u64::MAX),
                        None,
                        0,
                    )
                    .map_err(|error| format!("assertions[{index}].analog_window: {error}"))?;
                tallies[index] = Some(tally);
            }
            FlexAssertion::AnalogCrossings(assertion) => {
                let details = &assertion.analog_crossings;
                let column = analog_channel(
                    index,
                    "analog_crossings",
                    &details.port,
                    &details.channel,
                    &ports,
                    &analog,
                )?;
                let direction = match details.direction {
                    FlexCrossingDirection::Rising => AnalogCrossing::Rising,
                    FlexCrossingDirection::Falling => AnalogCrossing::Falling,
                };
                // One time more than expected shows an extra crossing.
                let max_times = details
                    .times_ns
                    .as_ref()
                    .map_or(0, |times| times.len() + 1)
                    .max(MAX_REPORTED_EDGE_TIMES);
                let tally = runtime
                    .tally_analog_window(
                        column,
                        details.from_ns.unwrap_or(0),
                        details.to_ns.unwrap_or(u64::MAX),
                        Some((details.level as f32, direction)),
                        max_times,
                    )
                    .map_err(|error| format!("assertions[{index}].analog_crossings: {error}"))?;
                tallies[index] = Some(tally);
            }
        }
    }
    Ok(tallies)
}

/// A finished script run. `runtime` holds the instruments' capture; it is
/// `None` when the run stopped on a configuration error before any step.
pub struct FlexScriptRun {
    pub result: FlexTestResult,
    pub runtime: Option<FlexRuntime>,
}

impl FlexTestResult {
    /// The result of a script that was rejected before any firmware
    /// instruction ran.
    pub fn config_error(config: FlexResultConfig, message: String) -> Self {
        Self {
            result_schema_version: "1.0",
            status: "error",
            stop_reason: FlexStopReason::ConfigError,
            message: Some(message),
            config,
            time_ns: 0,
            steps_executed: 0,
            entry_result: None,
            steps: Vec::new(),
            ports: Vec::new(),
            assertions: Vec::new(),
            traces: None,
            run_report: None,
        }
    }
}

/// Run `script` on the Flex manifest text and the firmware module bytes.
/// `config` names the inputs for the result; `config.limits.max_steps` is the
/// fuel budget and `config.firmware_sha256` is set here.
pub fn run_script(
    script: &FlexTestScript,
    mut config: FlexResultConfig,
    manifest: &str,
    firmware: &[u8],
) -> FlexScriptRun {
    config.firmware_sha256 = Some(format!("{:x}", Sha256::digest(firmware)));
    let max_steps = config.limits.max_steps;
    // Load everything before running anything; each failure here is a
    // configuration error with its own message.
    let loaded = FlexRuntime::from_yaml(manifest)
        .map_err(|error| format!("failed to load Flex manifest {:?}: {error}", config.flex))
        .and_then(|mut runtime| {
            let tallies = validate_assertion_ports(&mut runtime, script)?;
            Ok((runtime, tallies))
        });
    let (run_report, tallies, executed) = match loaded {
        Ok((runtime, tallies)) => (
            Some(runtime.run_report()),
            tallies,
            execute(runtime, firmware, script, max_steps),
        ),
        Err(message) => (None, Vec::new(), Err(message)),
    };
    let executed = match executed {
        Ok(executed) => executed,
        Err(message) => {
            let mut result = FlexTestResult::config_error(config, message);
            result.run_report = run_report;
            return FlexScriptRun {
                result,
                runtime: None,
            };
        }
    };
    let mut runtime = executed.runtime;
    let mut result = FlexTestResult {
        result_schema_version: "1.0",
        status: "error",
        stop_reason: executed.stop_reason,
        message: executed.message,
        config,
        time_ns: runtime.time_ns(),
        steps_executed: executed.steps_executed,
        entry_result: executed.entry_result,
        steps: executed.steps,
        ports: port_observations(&runtime),
        assertions: Vec::new(),
        traces: None,
        run_report,
    };
    // Cursor 0 reads the whole retained ring.
    let digital = runtime.digital_trace_snapshot(0);
    let analog = runtime.analog_trace_snapshot(0);
    let assertions = script
        .assertions
        .iter()
        .zip(&tallies)
        .map(|(assertion, tally)| match assertion {
            FlexAssertion::DigitalEdges(_) => {
                let tally = tally.and_then(|id| runtime.digital_edge_tally(id));
                evaluate(assertion, &result, tally, None)
            }
            _ => {
                let tally = tally.and_then(|id| runtime.analog_window_tally(id));
                evaluate(assertion, &result, None, tally.as_ref())
            }
        })
        .collect();
    result.assertions = assertions;
    result.traces = Some(FlexTraceSummary {
        time_unit: "ns",
        digital_channels: digital.channels.len(),
        digital_edges: digital.edges.len(),
        digital_dropped: digital.dropped,
        analog_channels: analog.channels.len(),
        analog_samples: analog.samples.len(),
        analog_dropped: analog.dropped,
    });
    result.status = match result.stop_reason {
        FlexStopReason::ConfigError
        | FlexStopReason::ModelError
        | FlexStopReason::SequenceError
        | FlexStopReason::FirmwareTrap => "error",
        // A limit stop cannot certify a pass, whatever the assertions say.
        FlexStopReason::MaxSteps | FlexStopReason::MaxTimeNs => "fail",
        FlexStopReason::Completed if result.assertions.iter().all(|a| a.passed) => "pass",
        FlexStopReason::Completed => "fail",
    };
    FlexScriptRun {
        result,
        runtime: Some(runtime),
    }
}
