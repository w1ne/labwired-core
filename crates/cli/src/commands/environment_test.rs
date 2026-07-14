// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! Multi-node implementation of `labwired test` environment scripts.

use crate::artifacts::{
    AssertionResult, EnvironmentConfig, EnvironmentNodeProvenance, EnvironmentNodeSnapshot,
    EnvironmentTestResult, Snapshot,
};
use crate::{build_stop_reason_details, TestArgs, EXIT_CONFIG_ERROR, EXIT_RUNTIME_ERROR};
use labwired_config::{EnvTestScript, EnvironmentManifest, StopReason, TestAssertion, TestLimits};
use labwired_core::world::{MachineTrait, World};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const INVALID_ENVIRONMENT_INPUT_PLACEHOLDER: &str = "__labwired_invalid_inputs_env__.yaml";
const WORLD_FIRMWARE_HASH_DOMAIN: &[u8] = b"labwired.environment.world-firmware.v1\0";
const ENVIRONMENT_RESULT_SCHEMA_VERSION: &str = "1.0-environment";
const ENVIRONMENT_RUN_TYPE: &str = "environment";

/// Values the command dispatcher needs to meter an environment run without
/// reparsing the artifacts it just wrote.
pub(crate) struct EnvironmentRunOutcome {
    pub(crate) exit_code: ExitCode,
    pub(crate) world_firmware_hash: String,
    pub(crate) cycles: u64,
}

/// Entry point kept separate from the single-machine runner so its world
/// topology and artifact contract cannot accidentally inherit one-firmware
/// assumptions.
pub(crate) fn run_environment_test(
    args: &TestArgs,
    script: EnvTestScript,
) -> EnvironmentRunOutcome {
    // Scope the thread-local coverage monitor to this world run. The result
    // path drains it regardless of whether an output directory was requested.
    labwired_core::fidelity::reset();

    // Wall-time is a run-wide budget: resolving the world and attaching its
    // topology are part of the environment run, not free prelude work.
    let run_started = Instant::now();
    let environment_path = resolve_script_path(&args.script, &script.inputs.env);
    let limits = resolved_limits(args, &script.limits);

    let manifest = match EnvironmentManifest::from_file(&environment_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            let config = empty_environment_config(args, &environment_path);
            return write_config_error(
                args,
                &limits,
                config,
                format!(
                    "failed to load environment {:?}: {error:#}",
                    environment_path
                ),
            );
        }
    };
    let config = environment_config(args, &environment_path, &manifest);

    if let Some(message) = unsupported_option_message(args) {
        return write_config_error(args, &limits, config, message);
    }
    if limits.max_steps == 0 || limits.max_steps > 50_000_000 {
        return write_config_error(
            args,
            &limits,
            config,
            format!(
                "environment max_steps must be between 1 and 50000000 (got {})",
                limits.max_steps
            ),
        );
    }
    if let Some(duplicate) = duplicate_node_id(&manifest) {
        return write_config_error(
            args,
            &limits,
            config,
            format!("environment contains duplicate node id '{duplicate}'"),
        );
    }
    if let Some(message) = validate_environment_assertions(&script.assertions, &manifest) {
        return write_config_error(args, &limits, config, message);
    }

    let root = environment_path.parent().unwrap_or_else(|| Path::new("."));
    let mut world = match World::from_manifest(manifest, root) {
        Ok(world) => world,
        Err(error) => {
            return write_config_error(
                args,
                &limits,
                config,
                format!("failed to build environment: {error:#}"),
            );
        }
    };

    let mut uart_sinks = BTreeMap::new();
    for id in sorted_node_ids(&world) {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let attach = world
            .machines
            .get_mut(&id)
            .expect("id was collected from world")
            .attach_uart_tx_sink(sink.clone(), !args.no_uart_stdout);
        if let Err(error) = attach {
            return write_config_error(
                args,
                &limits,
                config,
                format!("node '{id}': failed to attach UART capture: {error:#}"),
            );
        }
        uart_sinks.insert(id, sink);
    }

    run_world(
        args,
        script,
        limits,
        config,
        &mut world,
        &uart_sinks,
        run_started,
    )
}

/// Preserve the environment artifact contract even when strict schema parsing
/// rejects an otherwise recognizable `inputs.env` script before it can become
/// an [`EnvTestScript`]. Parsed YAML retains the full `inputs.env` behavior,
/// while malformed YAML is accepted only for the conservative, direct
/// `inputs:` / `env:` mapping shape or a valid inline `inputs: { env: ... }`
/// shape. Other malformed scripts keep the legacy config-error writer
/// unchanged.
pub(crate) fn try_write_load_error_outputs(args: &TestArgs, message: String) -> bool {
    // This branch never reaches `run_environment_test`, but it still emits
    // environment-shaped artifacts when a malformed script has a recognizable
    // `inputs.env`. Keep its thread-local evidence scope equally explicit.
    labwired_core::fidelity::reset();

    let Ok(contents) = std::fs::read_to_string(&args.script) else {
        return false;
    };
    let parsed = serde_yaml::from_str::<serde_yaml::Value>(&contents);
    let (environment_value, limits) = match parsed {
        Ok(raw) => {
            let Some(environment_value) = raw
                .get("inputs")
                .and_then(|inputs| inputs.get("env"))
                .cloned()
            else {
                return false;
            };
            let limits = raw
                .get("limits")
                .cloned()
                .and_then(|limits| serde_yaml::from_value::<TestLimits>(limits).ok())
                .unwrap_or_else(default_environment_limits);
            (environment_value, limits)
        }
        Err(_) => {
            let Some(environment_value) = recognizable_env_input_from_malformed_yaml(&contents)
            else {
                return false;
            };
            (environment_value, default_environment_limits())
        }
    };

    let usable_environment_path = environment_value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(|value| resolve_script_path(&args.script, value));
    let environment_path = usable_environment_path
        .clone()
        .unwrap_or_else(|| invalid_environment_path(args));
    let config = match usable_environment_path {
        Some(_) => match EnvironmentManifest::from_file(&environment_path) {
            Ok(manifest) => environment_config(args, &environment_path, &manifest),
            Err(_) => empty_environment_config(args, &environment_path),
        },
        None => empty_environment_config(args, &environment_path),
    };
    let _ = write_config_error(args, &limits, config, message);
    true
}

/// Extract a direct `inputs.env` scalar from malformed YAML without attempting
/// to repair or broadly reinterpret the document. The scanner deliberately
/// accepts only the indentation shape emitted in the public examples:
///
/// ```yaml
/// inputs:
///   env: "two-node.yaml"
/// ```
///
/// This gives a parse-error report the correct environment provenance while
/// leaving ambiguous or unrelated malformed YAML on the legacy path.
fn recognizable_env_input_from_malformed_yaml(contents: &str) -> Option<serde_yaml::Value> {
    let mut inputs_indent = None;
    let mut child_indent = None;

    for (line_index, line) in contents.lines().enumerate() {
        if line.contains('\t') {
            return None;
        }
        let trimmed = line.trim_start_matches(' ');
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();

        let Some(parent_indent) = inputs_indent else {
            if indent == 0 {
                if let Some(value) = yaml_mapping_value(trimmed, "inputs") {
                    let value = value.trim_start();
                    if value.is_empty() || value.starts_with('#') {
                        inputs_indent = Some(indent);
                    } else if value.starts_with('{') {
                        // Inline mappings are a documented, valid v1.0 input
                        // form. Parse exactly this line: it proves the inline
                        // environment selector occurs before the later syntax
                        // error without attempting to repair the document.
                        let parsed = serde_yaml::from_str::<serde_yaml::Value>(trimmed).ok()?;
                        return parsed
                            .get("inputs")
                            .and_then(|inputs| inputs.get("env"))
                            .cloned();
                    }
                }
            }
            continue;
        };

        if indent <= parent_indent {
            return None;
        }
        let direct_child_indent = *child_indent.get_or_insert(indent);
        if indent != direct_child_indent {
            continue;
        }
        let Some(value) = yaml_mapping_value(trimmed, "env") else {
            continue;
        };
        let value = value.trim_start();
        if value.starts_with('|') || value.starts_with('>') {
            return None;
        }

        // Parsing the prefix proves that `inputs.env` occurs before the YAML
        // syntax error, rather than being text after an already-invalid line.
        let prefix = contents
            .lines()
            .take(line_index + 1)
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = serde_yaml::from_str::<serde_yaml::Value>(&prefix).ok()?;
        return parsed
            .get("inputs")
            .and_then(|inputs| inputs.get("env"))
            .cloned();
    }

    None
}

fn yaml_mapping_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let value = line.strip_prefix(key)?.strip_prefix(':')?;
    match value.chars().next() {
        None | Some('#') => Some(value),
        Some(first) if first.is_whitespace() => Some(value),
        Some(_) => None,
    }
}

fn default_environment_limits() -> TestLimits {
    TestLimits {
        max_steps: 0,
        max_cycles: None,
        max_uart_bytes: None,
        no_progress_steps: None,
        wall_time_ms: None,
        max_vcd_bytes: None,
        stop_when_assertions_pass: false,
        stop_when_assertions_pass_settle_steps: 0,
        stop_when_assertions_pass_min_steps: 0,
    }
}

fn resolved_limits(args: &TestArgs, script_limits: &TestLimits) -> TestLimits {
    let mut limits = script_limits.clone();
    if let Some(value) = args.max_steps {
        limits.max_steps = value;
    }
    if let Some(value) = args.max_cycles {
        limits.max_cycles = Some(value);
    }
    if let Some(value) = args.max_uart_bytes {
        limits.max_uart_bytes = Some(value);
    }
    limits
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
    if args.detect_stuck.is_some() {
        unsupported.push("--detect-stuck");
    }
    if args.max_vcd_bytes.is_some() {
        unsupported.push("--max-vcd-bytes");
    }
    if args.trace || args.vcd.is_some() || args.trace_max.is_some() {
        unsupported.push("--trace/--vcd/--trace-max");
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
            "environment test scripts do not support {}; topology comes exclusively from inputs.env",
            unsupported.join(", ")
        )
    })
}

fn duplicate_node_id(manifest: &EnvironmentManifest) -> Option<String> {
    let mut ids = manifest
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| pair[0].to_string())
}

fn validate_environment_assertions(
    assertions: &[TestAssertion],
    manifest: &EnvironmentManifest,
) -> Option<String> {
    for (index, assertion) in assertions.iter().enumerate() {
        let TestAssertion::MemoryValue(memory) = assertion else {
            return Some(format!(
                "environment assertion {index} is not a node-qualified memory_value assertion"
            ));
        };
        let node = memory.memory_value.node.as_deref().unwrap_or_default();
        if !manifest.nodes.iter().any(|candidate| candidate.id == node) {
            return Some(format!(
                "environment memory_value assertion {index} references nonexistent node '{node}'"
            ));
        }
        match memory.memory_value.size.unwrap_or(32) {
            1 | 2 | 4 | 8 | 16 | 32 => {}
            size => {
                return Some(format!(
                    "environment memory_value assertion {index} has unsupported size {size}; use 1/2/4 bytes or 8/16/32 bits"
                ));
            }
        }
    }
    None
}

fn run_world(
    args: &TestArgs,
    script: EnvTestScript,
    limits: TestLimits,
    config: EnvironmentConfig,
    world: &mut World,
    uart_sinks: &BTreeMap<String, Arc<Mutex<Vec<u8>>>>,
    start: Instant,
) -> EnvironmentRunOutcome {
    let mut stop_reason = StopReason::MaxSteps;
    let mut message = None;
    let mut runtime_error = false;
    let mut rounds = 0_u64;
    let mut instructions = 0_u64;

    while rounds < limits.max_steps {
        let cycles = max_cycles(world);
        let uart_bytes = total_uart_bytes(uart_sinks);
        if let Some(reason) = reached_world_limit_stop(&limits, cycles, uart_bytes, &start) {
            stop_reason = reason;
            break;
        }

        let outcomes = world.step_all();
        rounds += 1;
        // `instructions` is the total number of successful individual machine
        // steps, not the number of world rounds. This makes a heterogeneous
        // environment's result explicit and reproducible.
        for id in sorted_node_ids(world) {
            if outcomes.get(&id).is_some_and(Result::is_ok) {
                instructions += 1;
            }
        }
        if let Some((id, error)) = sorted_node_ids(world).into_iter().find_map(|id| {
            outcomes
                .get(&id)
                .and_then(|outcome| outcome.as_ref().err().map(|error| (id, error)))
        }) {
            runtime_error = true;
            stop_reason = stop_reason_for_simulation_error(error);
            message = Some(format!("node '{id}': {error}"));
            break;
        }
        // A final allowed world round can itself reach a configured limit.
        // Check again before the loop condition turns that limit into an
        // apparently normal `max_steps` pass. Runtime errors above retain
        // their existing precedence; otherwise wall-time, cycles, then UART
        // determine the stop reason, matching the pre-round contract.
        let cycles = max_cycles(world);
        let uart_bytes = total_uart_bytes(uart_sinks);
        if let Some(reason) = reached_world_limit_stop(&limits, cycles, uart_bytes, &start) {
            stop_reason = reason;
            break;
        }
    }

    let duration = start.elapsed();
    let cycles = max_cycles(world);
    let uart_bytes = total_uart_bytes(uart_sinks);
    let assertions = evaluate_assertions(&script.assertions, world);
    let all_assertions_passed = assertions.iter().all(|assertion| assertion.passed);
    let safety_stop_requires_failure = matches!(
        stop_reason,
        StopReason::WallTime | StopReason::MaxUartBytes | StopReason::NoProgress
    );
    // Mirror the single-machine runner's precedence: a failed assertion is a
    // test failure even if a runtime fault also occurred; wall/UART/no-progress
    // limits are safety stops and cannot certify a world as passed.
    let status = if !all_assertions_passed || safety_stop_requires_failure {
        "fail"
    } else if runtime_error {
        "error"
    } else {
        "pass"
    };
    let stop_reason_details = build_stop_reason_details(
        &stop_reason,
        &limits,
        rounds,
        cycles,
        uart_bytes,
        0,
        duration,
        0,
    );
    let result = EnvironmentTestResult {
        result_schema_version: ENVIRONMENT_RESULT_SCHEMA_VERSION.to_string(),
        run_type: ENVIRONMENT_RUN_TYPE.to_string(),
        status: status.to_string(),
        steps_executed: rounds,
        cycles,
        instructions,
        stop_reason: stop_reason.clone(),
        stop_reason_details: stop_reason_details.clone(),
        limits: limits.clone(),
        message,
        assertions,
        fidelity: labwired_core::fidelity::take().to_gaps(),
        config: config.clone(),
    };
    let snapshot = Snapshot::Environment {
        status: status.to_string(),
        message: result.message.clone(),
        steps_executed: rounds,
        cycles,
        instructions,
        stop_reason: stop_reason.clone(),
        stop_reason_details,
        limits,
        config,
        nodes: world_snapshots(world),
    };
    write_environment_artifacts(
        args,
        &result,
        &snapshot,
        &render_uart_log(uart_sinks),
        duration,
    );

    let exit_code = if !all_assertions_passed || safety_stop_requires_failure {
        ExitCode::from(crate::EXIT_ASSERT_FAIL)
    } else if runtime_error {
        ExitCode::from(EXIT_RUNTIME_ERROR)
    } else {
        ExitCode::SUCCESS
    };
    EnvironmentRunOutcome {
        exit_code,
        world_firmware_hash: result.config.world_firmware_hash.clone(),
        cycles,
    }
}

fn stop_reason_for_simulation_error(error: &labwired_core::SimulationError) -> StopReason {
    match error {
        labwired_core::SimulationError::MemoryViolation(_) => StopReason::MemoryViolation,
        labwired_core::SimulationError::DecodeError(_) => StopReason::DecodeError,
        labwired_core::SimulationError::Halt | labwired_core::SimulationError::BreakpointHit(_) => {
            StopReason::Halt
        }
        labwired_core::SimulationError::SnapshotSchemaMismatch { .. }
        | labwired_core::SimulationError::NotImplemented(_)
        | labwired_core::SimulationError::ExceptionRaised { .. }
        | labwired_core::SimulationError::Other(_) => StopReason::Exception,
    }
}

fn evaluate_assertions(assertions: &[TestAssertion], world: &World) -> Vec<AssertionResult> {
    assertions
        .iter()
        .map(|assertion| {
            let passed = match assertion {
                TestAssertion::MemoryValue(memory) => {
                    let node = memory.memory_value.node.as_deref().unwrap_or_default();
                    world
                        .machines
                        .get(node)
                        .map(|machine| memory_assertion_passes(machine.as_ref(), memory))
                        .unwrap_or(false)
                }
                _ => false,
            };
            AssertionResult {
                assertion: assertion.clone(),
                passed,
            }
        })
        .collect()
}

fn memory_assertion_passes(
    machine: &dyn MachineTrait,
    assertion: &labwired_config::MemoryValueAssertion,
) -> bool {
    let size = assertion.memory_value.size.unwrap_or(32);
    let width = match size {
        1 | 8 => 1,
        2 | 16 => 2,
        4 | 32 => 4,
        _ => return false,
    };
    let mut value = 0_u32;
    for offset in 0..width {
        let byte = match machine.read_u8(assertion.memory_value.address + offset) {
            Ok(byte) => byte,
            Err(_) => return false,
        };
        value |= u32::from(byte) << (offset * 8);
    }
    let mask = assertion.memory_value.mask.unwrap_or(0xffff_ffff) as u32;
    let expected = assertion.memory_value.expected_value as u32;
    (value & mask) == (expected & mask)
}

fn max_cycles(world: &World) -> u64 {
    world
        .machines
        .values()
        .map(|machine| machine.total_cycles())
        .max()
        .unwrap_or(0)
}

fn sorted_node_ids(world: &World) -> Vec<String> {
    let mut ids = world.machines.keys().cloned().collect::<Vec<_>>();
    ids.sort();
    ids
}

fn total_uart_bytes(sinks: &BTreeMap<String, Arc<Mutex<Vec<u8>>>>) -> u64 {
    sinks
        .values()
        .map(|sink| sink.lock().map(|bytes| bytes.len() as u64).unwrap_or(0))
        .sum()
}

/// Return a configured limit reached at a world-round boundary. The order is
/// wall-time, maximum cycles, then total captured UART bytes, matching the
/// runner's established pre-round precedence. Limits are checked both before
/// and after a round so the final `max_steps` round cannot bypass one.
fn reached_world_limit_stop(
    limits: &TestLimits,
    cycles: u64,
    uart_bytes: u64,
    start: &Instant,
) -> Option<StopReason> {
    if limits
        .wall_time_ms
        .is_some_and(|limit| start.elapsed().as_millis() >= u128::from(limit))
    {
        return Some(StopReason::WallTime);
    }
    if limits.max_cycles.is_some_and(|limit| cycles >= limit) {
        return Some(StopReason::MaxCycles);
    }
    if limits
        .max_uart_bytes
        .is_some_and(|limit| uart_bytes >= limit)
    {
        return Some(StopReason::MaxUartBytes);
    }
    None
}

fn render_uart_log(sinks: &BTreeMap<String, Arc<Mutex<Vec<u8>>>>) -> Vec<u8> {
    let mut output = Vec::new();
    for (id, sink) in sinks {
        output.extend_from_slice(format!("[node:{id}]\n").as_bytes());
        let bytes = sink.lock().map(|bytes| bytes.clone()).unwrap_or_default();
        output.extend_from_slice(&bytes);
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            output.push(b'\n');
        }
    }
    output
}

fn world_snapshots(world: &World) -> Vec<EnvironmentNodeSnapshot> {
    sorted_node_ids(world)
        .into_iter()
        .map(|id| {
            let machine = world
                .machines
                .get(&id)
                .expect("id was collected from world");
            EnvironmentNodeSnapshot {
                cycles: machine.total_cycles(),
                state: machine.snapshot(),
                id,
            }
        })
        .collect()
}

fn empty_environment_config(args: &TestArgs, environment: &Path) -> EnvironmentConfig {
    EnvironmentConfig {
        script: resolved_path(&args.script),
        environment: resolved_path(environment),
        world_firmware_hash: aggregate_world_firmware_hash(&[]),
        nodes: Vec::new(),
    }
}

fn environment_config(
    args: &TestArgs,
    environment: &Path,
    manifest: &EnvironmentManifest,
) -> EnvironmentConfig {
    let root = environment.parent().unwrap_or_else(|| Path::new("."));
    let mut nodes = manifest
        .nodes
        .iter()
        .map(|node| {
            let system = resolved_path(&root.join(&node.system));
            let firmware = resolved_path(&root.join(&node.firmware));
            EnvironmentNodeProvenance {
                id: node.id.clone(),
                system_hash: file_hash(&system),
                firmware_hash: file_hash(&firmware),
                system,
                firmware,
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.firmware.cmp(&right.firmware))
            .then_with(|| left.firmware_hash.cmp(&right.firmware_hash))
    });
    EnvironmentConfig {
        script: resolved_path(&args.script),
        environment: resolved_path(environment),
        world_firmware_hash: aggregate_world_firmware_hash(&nodes),
        nodes,
    }
}

fn aggregate_world_firmware_hash(nodes: &[EnvironmentNodeProvenance]) -> String {
    let mut hash = Sha256::new();
    hash.update(WORLD_FIRMWARE_HASH_DOMAIN);
    hash.update((nodes.len() as u64).to_be_bytes());
    for node in nodes {
        hash_framed(&mut hash, node.id.as_bytes());
        hash_framed(&mut hash, node.firmware.to_string_lossy().as_bytes());
        hash_framed(
            &mut hash,
            &std::fs::read(&node.firmware).unwrap_or_default(),
        );
    }
    format!("{:x}", hash.finalize())
}

fn hash_framed(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

fn file_hash(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let mut hash = Sha256::new();
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

fn resolved_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn invalid_environment_path(args: &TestArgs) -> PathBuf {
    resolved_path(&args.script)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(INVALID_ENVIRONMENT_INPUT_PLACEHOLDER)
}

fn resolve_script_path(script: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        script.parent().unwrap_or_else(|| Path::new(".")).join(path)
    }
}

fn write_config_error(
    args: &TestArgs,
    limits: &TestLimits,
    config: EnvironmentConfig,
    message: String,
) -> EnvironmentRunOutcome {
    let world_firmware_hash = config.world_firmware_hash.clone();
    let stop_reason = StopReason::ConfigError;
    let details = build_stop_reason_details(&stop_reason, limits, 0, 0, 0, 0, Duration::ZERO, 0);
    let result = EnvironmentTestResult {
        result_schema_version: ENVIRONMENT_RESULT_SCHEMA_VERSION.to_string(),
        run_type: ENVIRONMENT_RUN_TYPE.to_string(),
        status: "error".to_string(),
        steps_executed: 0,
        cycles: 0,
        instructions: 0,
        stop_reason: stop_reason.clone(),
        stop_reason_details: details.clone(),
        limits: limits.clone(),
        message: Some(message.clone()),
        assertions: Vec::new(),
        fidelity: labwired_core::fidelity::take().to_gaps(),
        config: config.clone(),
    };
    let empty_sinks = config
        .nodes
        .iter()
        .map(|node| (node.id.clone(), Arc::new(Mutex::new(Vec::<u8>::new()))))
        .collect::<BTreeMap<_, _>>();
    let snapshot = Snapshot::Environment {
        status: "error".to_string(),
        message: Some(message),
        steps_executed: 0,
        cycles: 0,
        instructions: 0,
        stop_reason,
        stop_reason_details: details,
        limits: limits.clone(),
        nodes: config
            .nodes
            .iter()
            .map(|node| EnvironmentNodeSnapshot {
                id: node.id.clone(),
                cycles: 0,
                state: None,
            })
            .collect(),
        config,
    };
    write_environment_artifacts(
        args,
        &result,
        &snapshot,
        &render_uart_log(&empty_sinks),
        Duration::ZERO,
    );
    EnvironmentRunOutcome {
        exit_code: ExitCode::from(EXIT_CONFIG_ERROR),
        world_firmware_hash,
        cycles: 0,
    }
}

fn write_environment_artifacts(
    args: &TestArgs,
    result: &EnvironmentTestResult,
    snapshot: &Snapshot,
    uart: &[u8],
    duration: Duration,
) {
    if let Some(output_dir) = &args.output_dir {
        if let Err(error) = std::fs::create_dir_all(output_dir) {
            tracing::error!(
                "failed to create environment output directory {:?}: {error}",
                output_dir
            );
        } else {
            write_json(&output_dir.join("result.json"), result);
            write_json(&output_dir.join("snapshot.json"), snapshot);
            if let Err(error) = std::fs::write(output_dir.join("uart.log"), uart) {
                tracing::error!("failed to write environment uart.log: {error}");
            }
            if let Err(error) =
                write_environment_junit(&output_dir.join("junit.xml"), result, duration)
            {
                tracing::error!("failed to write environment junit.xml: {error}");
            }
        }
    }
    if let Some(path) = &args.junit {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = write_environment_junit(path, result, duration) {
            tracing::error!("failed to write environment JUnit {:?}: {error}", path);
        }
    }
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
    match std::fs::File::create(path) {
        Ok(file) => {
            if let Err(error) = serde_json::to_writer_pretty(file, value) {
                tracing::error!("failed to write environment artifact {:?}: {error}", path);
            }
        }
        Err(error) => tracing::error!("failed to create environment artifact {:?}: {error}", path),
    }
}

fn write_environment_junit(
    path: &Path,
    result: &EnvironmentTestResult,
    duration: Duration,
) -> std::io::Result<()> {
    let mut tests = 1_u64;
    let mut failures = 0_u64;
    let mut errors = 0_u64;
    let safety_stop_requires_failure = matches!(
        result.stop_reason,
        StopReason::WallTime | StopReason::MaxUartBytes | StopReason::NoProgress
    );
    let details = format!(
        "result_schema_version={}\nstop_reason={:?}\nsteps_executed={}\ncycles={}\ninstructions={}\nenvironment={}\nscript={}",
        result.result_schema_version,
        result.stop_reason,
        result.steps_executed,
        result.cycles,
        result.instructions,
        result.config.environment.display(),
        result.config.script.display(),
    );
    let mut cases = format!(
        "  <testcase classname=\"labwired\" name=\"run\" time=\"{:.6}\">\n",
        duration.as_secs_f64()
    );
    if result.status == "error" {
        errors += 1;
        let error_kind = if result.stop_reason == StopReason::ConfigError {
            "config error"
        } else {
            "runtime error"
        };
        cases.push_str(&format!(
            "    <error message=\"{}\">{}</error>\n",
            crate::xml_escape(error_kind),
            crate::xml_escape(result.message.as_deref().unwrap_or(&details))
        ));
    } else if result.status == "fail" && safety_stop_requires_failure {
        failures += 1;
        cases.push_str(&format!(
            "    <failure message=\"{}\">{}</failure>\n",
            crate::xml_escape("safety stop cannot certify a passing environment run"),
            crate::xml_escape(&details)
        ));
    } else if result.status == "fail"
        && !result.assertions.iter().any(|assertion| !assertion.passed)
    {
        failures += 1;
        cases.push_str(&format!(
            "    <failure message=\"{}\">{}</failure>\n",
            crate::xml_escape("failure"),
            crate::xml_escape(&details)
        ));
    }
    cases.push_str("  </testcase>\n");
    for (index, assertion) in result.assertions.iter().enumerate() {
        tests += 1;
        let name = format!(
            "assertion {}: {}",
            index + 1,
            crate::assertion_short_name(&assertion.assertion)
        );
        cases.push_str(&format!(
            "  <testcase classname=\"labwired\" name=\"{}\" time=\"0.000000\">\n",
            crate::xml_escape(&name)
        ));
        if !assertion.passed {
            failures += 1;
            cases.push_str(&format!(
                "    <failure message=\"assertion failed\">{}</failure>\n",
                crate::xml_escape(&format!("{name}\n\n{details}"))
            ));
        }
        cases.push_str("  </testcase>\n");
    }
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"labwired\" tests=\"{tests}\" failures=\"{failures}\" errors=\"{errors}\" time=\"{:.6}\">\n{}\n</testsuite>\n",
        duration.as_secs_f64(),
        cases
    );
    std::fs::write(path, xml)
}
