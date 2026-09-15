// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Scripted debug probe for agents (v1): one-shot run-to-stop with optional breakpoints.
//! Output is agent-friendly JSON; never claims oracle proof.

use clap::Args;
use labwired_core::DebugControl;
use labwired_loader::SymbolProvider;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use crate::{EXIT_CONFIG_ERROR, EXIT_PASS, EXIT_RUNTIME_ERROR};

/// Max serial capture retained in the probe result (agent context budget).
const SERIAL_CAP_BYTES: usize = 32_768;

/// CLI args for `labwired debug-probe`.
#[derive(Args, Debug)]
pub struct DebugProbeArgs {
    /// Firmware ELF path (required in v1; no flash-only rom-boot path)
    #[arg(short = 'f', long)]
    pub firmware: Option<PathBuf>,

    /// System manifest YAML
    #[arg(short = 's', long)]
    pub system: PathBuf,

    /// JSON array of breakpoint specs
    #[arg(long)]
    pub breakpoints_json: Option<String>,

    /// Path to breakpoints JSON file (alternative to --breakpoints-json)
    #[arg(long)]
    pub breakpoints_file: Option<PathBuf>,

    /// Maximum steps before stop (instruction/step budget)
    #[arg(long, default_value_t = 2_000_000)]
    pub max_steps: u32,

    /// Comma list: serial,regs,pc,location (default all)
    #[arg(long, default_value = "serial,regs,pc,location")]
    pub read: String,

    /// After the primary stop, take one single-instruction step
    #[arg(long, default_value_t = false)]
    pub step_after_stop: bool,

    /// Write result.json under this directory (also prints JSON to stdout)
    #[arg(long)]
    pub output_dir: Option<PathBuf>,
}

/// Breakpoint request shapes accepted by the debug probe (JSON untagged).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum BreakpointSpec {
    Address {
        address: String,
    },
    Symbol {
        symbol: String,
    },
    Line {
        line: u32,
        #[serde(default)]
        file: Option<String>,
    },
}

/// Resolution outcome for one requested breakpoint.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BreakpointOutcome {
    pub requested: serde_json::Value,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Agent-facing JSON result for a debug probe run.
///
/// Observational only: `proven` is always `false`.
#[derive(Debug, Clone, Serialize)]
pub struct DebugProbeResult {
    pub status: String, // "ok" | "error"
    pub stop_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycles: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default)]
    pub breakpoints: Vec<BreakpointOutcome>,
    /// Always false — observational probe only.
    pub proven: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
}

impl DebugProbeResult {
    /// Build an error result. Never sets `proven`.
    pub fn error(code: &str, detail: &str) -> Self {
        Self {
            status: "error".into(),
            stop_reason: "config_error".into(),
            pc: None,
            cycles: None,
            location: None,
            registers: None,
            serial: None,
            breakpoints: vec![],
            proven: false,
            error: Some(json!({ "code": code, "detail": detail })),
        }
    }

    /// Runtime failure during probe execution. Never sets `proven`.
    pub fn runtime_error(code: &str, detail: &str) -> Self {
        Self {
            status: "error".into(),
            stop_reason: "runtime_error".into(),
            pc: None,
            cycles: None,
            location: None,
            registers: None,
            serial: None,
            breakpoints: vec![],
            proven: false,
            error: Some(json!({ "code": code, "detail": detail })),
        }
    }
}

/// Parse an address string the same way as CLI `parse_u32_addr`:
/// optional `0x`/`0X` hex, otherwise decimal. No underscore separators.
pub fn parse_address(s: &str) -> Option<u32> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        t.parse().ok()
    }
}

/// Map engine stop reasons to the agent-facing string contract.
pub fn stop_reason_str(r: &labwired_core::StopReason) -> &'static str {
    match r {
        labwired_core::StopReason::Breakpoint(_) => "breakpoint",
        labwired_core::StopReason::StepDone => "step_done",
        labwired_core::StopReason::MaxStepsReached => "max_steps",
        labwired_core::StopReason::ManualStop => "halt",
        // Firmware ended its own run through the `simctl` device. The contract
        // is a fixed string, so the code is not spelled here; it travels with
        // the `AdvanceReport`, which is the surface a harness reads.
        labwired_core::StopReason::FirmwareExit(_) => "firmware_exit",
    }
}

/// Which result fields the client asked to include.
#[derive(Debug, Clone, Copy)]
struct ReadSet {
    serial: bool,
    regs: bool,
    pc: bool,
    location: bool,
}

impl ReadSet {
    fn parse(s: &str) -> Self {
        let mut set = ReadSet {
            serial: false,
            regs: false,
            pc: false,
            location: false,
        };
        for part in s.split(',') {
            match part.trim().to_ascii_lowercase().as_str() {
                "serial" => set.serial = true,
                "regs" | "registers" => set.regs = true,
                "pc" => set.pc = true,
                "location" => set.location = true,
                "" => {}
                other => warn!("unknown --read field '{other}' (ignored)"),
            }
        }
        // Empty list → default all (matches product contract).
        if !set.serial && !set.regs && !set.pc && !set.location {
            set.serial = true;
            set.regs = true;
            set.pc = true;
            set.location = true;
        }
        set
    }
}

fn format_addr(addr: u32) -> String {
    format!("0x{addr:08x}")
}

/// Clear Thumb/ISA selection bit so breakpoints match engine checks (`pc & !1`).
fn executable_pc(addr: u32) -> u32 {
    addr & !1
}

fn spec_to_json(spec: &BreakpointSpec) -> serde_json::Value {
    serde_json::to_value(spec).unwrap_or(json!({}))
}

/// Resolve breakpoints against an optional SymbolProvider.
/// Unverified BPs never invent a fake address.
fn resolve_breakpoints(
    specs: &[BreakpointSpec],
    symbols: Option<&SymbolProvider>,
) -> (Vec<BreakpointOutcome>, Vec<u32>) {
    let mut outcomes = Vec::with_capacity(specs.len());
    let mut addrs = Vec::new();

    for spec in specs {
        let requested = spec_to_json(spec);
        match spec {
            BreakpointSpec::Address { address } => match parse_address(address) {
                Some(addr) => {
                    let addr = executable_pc(addr);
                    outcomes.push(BreakpointOutcome {
                        requested,
                        verified: true,
                        address: Some(format_addr(addr)),
                        message: None,
                    });
                    addrs.push(addr);
                }
                None => {
                    outcomes.push(BreakpointOutcome {
                        requested,
                        verified: false,
                        address: None,
                        message: Some(format!(
                            "invalid address '{address}' (expected 0x-hex or decimal)"
                        )),
                    });
                }
            },
            BreakpointSpec::Symbol { symbol } => {
                let resolved = symbols.and_then(|s| s.resolve_symbol(symbol));
                match resolved {
                    Some(addr) => {
                        let addr = executable_pc(addr as u32);
                        outcomes.push(BreakpointOutcome {
                            requested,
                            verified: true,
                            address: Some(format_addr(addr)),
                            message: None,
                        });
                        addrs.push(addr);
                    }
                    None => {
                        let message = if symbols.is_none() {
                            format!(
                                "symbol '{symbol}' not resolved (no ELF symbol table available)"
                            )
                        } else {
                            format!("symbol '{symbol}' not found in ELF")
                        };
                        outcomes.push(BreakpointOutcome {
                            requested,
                            verified: false,
                            address: None,
                            message: Some(message),
                        });
                    }
                }
            }
            BreakpointSpec::Line { line, file } => {
                let file_hint = file.as_deref().unwrap_or("main.c");
                let resolved = symbols.and_then(|s| s.location_to_pc(file_hint, *line));
                match resolved {
                    Some(addr) => {
                        let addr = executable_pc(addr as u32);
                        outcomes.push(BreakpointOutcome {
                            requested,
                            verified: true,
                            address: Some(format_addr(addr)),
                            message: None,
                        });
                        addrs.push(addr);
                    }
                    None => {
                        let message = if symbols.is_none() {
                            format!(
                                "line {line} not resolved (firmware has no line info; \
                                 use symbol or address, or compile with debug)"
                            )
                        } else if file.is_none() {
                            format!(
                                "line {line} not found (no file given; tried '{file_hint}'; \
                                 specify file for multi-CU firmware, or use symbol/address)"
                            )
                        } else {
                            format!("line {line} in '{file_hint}' not found in DWARF line table")
                        };
                        outcomes.push(BreakpointOutcome {
                            requested,
                            verified: false,
                            address: None,
                            message: Some(message),
                        });
                    }
                }
            }
        }
    }

    (outcomes, addrs)
}

fn parse_breakpoint_specs(args: &DebugProbeArgs) -> Result<Vec<BreakpointSpec>, String> {
    let raw = if let Some(path) = &args.breakpoints_file {
        std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read breakpoints file {}: {e}", path.display()))?
    } else if let Some(s) = &args.breakpoints_json {
        s.clone()
    } else {
        return Ok(Vec::new());
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    serde_json::from_str(trimmed).map_err(|e| format!("invalid breakpoints JSON: {e}"))
}

fn emit_result(result: &DebugProbeResult, output_dir: Option<&Path>) -> ExitCode {
    let json_text = match serde_json::to_string_pretty(result) {
        Ok(s) => s,
        Err(e) => {
            error!("failed to serialize debug-probe result: {e}");
            println!(
                "{{\"status\":\"error\",\"stop_reason\":\"runtime_error\",\"proven\":false,\
                 \"error\":{{\"code\":\"SERIALIZE\",\"detail\":\"{e}\"}}}}"
            );
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
    };

    if let Some(dir) = output_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            error!("failed to create output dir {}: {e}", dir.display());
        } else {
            let path = dir.join("result.json");
            if let Err(e) = std::fs::write(&path, &json_text) {
                error!("failed to write {}: {e}", path.display());
            } else {
                info!("wrote {}", path.display());
            }
        }
    }

    println!("{json_text}");

    if result.status == "ok" {
        ExitCode::from(EXIT_PASS)
    } else if result.stop_reason == "runtime_error" {
        ExitCode::from(EXIT_RUNTIME_ERROR)
    } else {
        ExitCode::from(EXIT_CONFIG_ERROR)
    }
}

fn collect_registers(machine: &dyn DebugControl) -> BTreeMap<String, String> {
    let names = machine.get_register_names();
    let mut regs = BTreeMap::new();
    for (i, name) in names.into_iter().enumerate() {
        // Core register dump is by index; names come from the CPU in order.
        let val = machine.read_core_reg(i as u8);
        regs.insert(name, format_addr(val));
    }
    regs
}

fn collect_location(symbols: Option<&SymbolProvider>, pc: u32) -> Option<serde_json::Value> {
    let loc = symbols?.lookup(pc as u64)?;
    Some(json!({
        "file": loc.file,
        "line": loc.line,
    }))
}

fn serial_from_sink(sink: &Arc<Mutex<Vec<u8>>>) -> String {
    let bytes = sink.lock().unwrap_or_else(|e| e.into_inner());
    let capped = if bytes.len() > SERIAL_CAP_BYTES {
        &bytes[..SERIAL_CAP_BYTES]
    } else {
        &bytes[..]
    };
    String::from_utf8_lossy(capped).into_owned()
}

/// Run the probe against a concrete `DebugControl` machine.
// Three arch paths call this with the same flat argument list (see the match in
// `run`); bundling them into a struct would only move the arity to the struct
// literal at each call site. Same rationale as `commands/test.rs`.
#[allow(clippy::too_many_arguments)]
fn run_on_machine(
    machine: &mut dyn DebugControl,
    verified_addrs: &[u32],
    max_steps: u32,
    step_after_stop: bool,
    read: ReadSet,
    symbols: Option<&SymbolProvider>,
    uart_tx: &Arc<Mutex<Vec<u8>>>,
    bp_outcomes: Vec<BreakpointOutcome>,
) -> DebugProbeResult {
    for &addr in verified_addrs {
        machine.add_breakpoint(addr);
    }

    let reason = match machine.run(Some(max_steps)) {
        Ok(r) => r,
        Err(e) => {
            let mut res = DebugProbeResult::runtime_error("RUN_FAILED", &format!("{e:#}"));
            res.breakpoints = bp_outcomes;
            return res;
        }
    };

    let mut stop_reason = stop_reason_str(&reason).to_string();

    if step_after_stop {
        match machine.step_single() {
            Ok(r) => {
                // Prefer step_done when the optional post-stop step ran.
                stop_reason = stop_reason_str(&r).to_string();
            }
            Err(e) => {
                warn!("step_after_stop failed: {e:#}");
            }
        }
    }

    let pc = machine.get_pc();
    let cycles = machine.get_cycle_count();

    DebugProbeResult {
        status: "ok".into(),
        stop_reason,
        pc: if read.pc { Some(format_addr(pc)) } else { None },
        cycles: Some(cycles),
        location: if read.location {
            collect_location(symbols, pc)
        } else {
            None
        },
        registers: if read.regs {
            Some(collect_registers(machine))
        } else {
            None
        },
        serial: if read.serial {
            Some(serial_from_sink(uart_tx))
        } else {
            None
        },
        breakpoints: bp_outcomes,
        proven: false,
        error: None,
    }
}

/// Build bus + UART sink shared by all arch paths (mirrors `labwired test` / DAP).
// The tuple mirrors what `labwired test` / DAP already destructure at the call
// site; naming it would add a type that exists only to satisfy the lint.
#[allow(clippy::type_complexity)]
fn build_bus_and_uart(
    system: &Path,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> Result<
    (
        labwired_core::bus::SystemBus,
        Arc<Mutex<Vec<u8>>>,
        Option<labwired_config::SystemManifest>,
    ),
    String,
> {
    let resolved = labwired_config::ResolvedSystem::from_manifest_file(system)
        .map_err(|e| format!("failed to load system manifest {}: {e:#}", system.display()))?;

    let mut bus =
        labwired_core::system::builder::build_system_bus_with_plugins(Some(&resolved), plugins)
            .map_err(|e| format!("failed to build system bus: {e:#}"))?;

    let manifest = resolved.manifest.clone();
    let debug_uart = manifest.debug_uart.clone();
    let uart_tx = Arc::new(Mutex::new(Vec::new()));

    if let Some(debug_uart) = debug_uart.as_deref() {
        if !bus.attach_uart_tx_sink_named(debug_uart, uart_tx.clone(), false) {
            warn!(
                "debug_uart '{debug_uart}' did not resolve to a UART peripheral; \
                 falling back to all UARTs"
            );
            bus.attach_uart_tx_sink(uart_tx.clone(), false);
        }
    } else {
        bus.attach_uart_tx_sink(uart_tx.clone(), false);
    }

    // ESP32: Arduino USB-CDC may write USB_SERIAL_JTAG rather than UART0.
    {
        use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
        for p in bus.peripherals.iter_mut() {
            if p.name == "usb_serial_jtag" {
                if let Some(any) = p.dev.as_any_mut() {
                    if let Some(jtag) = any.downcast_mut::<UsbSerialJtag>() {
                        jtag.set_sink(Some(uart_tx.clone()), false);
                    }
                }
            }
        }
    }
    bus.attach_iolink_master_log_sink(uart_tx.clone());

    Ok((bus, uart_tx, Some(manifest)))
}

/// Entry point for `labwired debug-probe`.
pub fn run(args: DebugProbeArgs, plugins: &[&dyn labwired_core::plugin::ChipPlugin]) -> ExitCode {
    let read = ReadSet::parse(&args.read);

    let firmware = match &args.firmware {
        Some(p) => p.clone(),
        None => {
            return emit_result(
                &DebugProbeResult::error(
                    "NO_FIRMWARE",
                    "debug_probe requires a symbol-bearing ELF in v1 \
                     (flash-only rom-boot is not supported)",
                ),
                args.output_dir.as_deref(),
            );
        }
    };

    if !firmware.is_file() {
        return emit_result(
            &DebugProbeResult::error(
                "NO_FIRMWARE",
                &format!("firmware ELF not found: {}", firmware.display()),
            ),
            args.output_dir.as_deref(),
        );
    }

    let specs = match parse_breakpoint_specs(&args) {
        Ok(s) => s,
        Err(detail) => {
            return emit_result(
                &DebugProbeResult::error("BAD_BREAKPOINTS", &detail),
                args.output_dir.as_deref(),
            );
        }
    };

    // Symbols are best-effort: missing DWARF still allows address BPs + run.
    let symbols = match SymbolProvider::new(&firmware) {
        Ok(s) => Some(s),
        Err(e) => {
            warn!(
                "SymbolProvider unavailable for {}: {e:#}",
                firmware.display()
            );
            None
        }
    };

    let (bp_outcomes, verified_addrs) = resolve_breakpoints(&specs, symbols.as_ref());

    let program = match labwired_loader::load_elf(&firmware) {
        Ok(p) => p,
        Err(e) => {
            let mut res = DebugProbeResult::error(
                "LOAD_ELF",
                &format!("failed to load firmware ELF {}: {e:#}", firmware.display()),
            );
            res.breakpoints = bp_outcomes;
            return emit_result(&res, args.output_dir.as_deref());
        }
    };

    let (mut bus, uart_tx, _manifest) = match build_bus_and_uart(&args.system, plugins) {
        Ok(v) => v,
        Err(detail) => {
            let mut res = DebugProbeResult::error("SYSTEM_LOAD", &detail);
            res.breakpoints = bp_outcomes;
            return emit_result(&res, args.output_dir.as_deref());
        }
    };

    info!(
        "debug-probe: firmware={} system={} max_steps={} breakpoints_verified={}",
        firmware.display(),
        args.system.display(),
        args.max_steps,
        verified_addrs.len()
    );

    // Arch dispatch mirrors DAP adapter / machine load paths (no rom-boot dual-core
    // special cases in v1 — plain ELF load + DebugControl::run).
    let result = match program.arch {
        labwired_core::Arch::Arm => {
            let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
            if let Err(e) = machine.load_firmware(&program) {
                let mut res =
                    DebugProbeResult::error("LOAD_FIRMWARE", &format!("load_firmware failed: {e}"));
                res.breakpoints = bp_outcomes;
                return emit_result(&res, args.output_dir.as_deref());
            }
            run_on_machine(
                &mut machine,
                &verified_addrs,
                args.max_steps,
                args.step_after_stop,
                read,
                symbols.as_ref(),
                &uart_tx,
                bp_outcomes,
            )
        }
        labwired_core::Arch::RiscV => {
            let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
            // FreeRTOS / ESP32-C3: keep peripheral ticks aligned with steps.
            if machine.bus.irq_fabric.esp32c3.routing {
                machine.config.batch_mode_enabled = false;
            }
            if let Err(e) = machine.load_firmware(&program) {
                let mut res =
                    DebugProbeResult::error("LOAD_FIRMWARE", &format!("load_firmware failed: {e}"));
                res.breakpoints = bp_outcomes;
                return emit_result(&res, args.output_dir.as_deref());
            }
            run_on_machine(
                &mut machine,
                &verified_addrs,
                args.max_steps,
                args.step_after_stop,
                read,
                symbols.as_ref(),
                &uart_tx,
                bp_outcomes,
            )
        }
        labwired_core::Arch::XtensaLx7 => {
            let cpu = labwired_core::system::xtensa::configure_xtensa(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
            if let Err(e) = machine.load_firmware(&program) {
                let mut res =
                    DebugProbeResult::error("LOAD_FIRMWARE", &format!("load_firmware failed: {e}"));
                res.breakpoints = bp_outcomes;
                return emit_result(&res, args.output_dir.as_deref());
            }
            run_on_machine(
                &mut machine,
                &verified_addrs,
                args.max_steps,
                args.step_after_stop,
                read,
                symbols.as_ref(),
                &uart_tx,
                bp_outcomes,
            )
        }
        other => {
            let mut res = DebugProbeResult::error(
                "UNSUPPORTED_ARCH",
                &format!("debug-probe does not support architecture {other:?} in v1"),
            );
            res.breakpoints = bp_outcomes;
            return emit_result(&res, args.output_dir.as_deref());
        }
    };

    emit_result(&result, args.output_dir.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_address_hex_and_decimal() {
        assert_eq!(parse_address("0x40000000"), Some(0x4000_0000));
        assert_eq!(parse_address("0XABCD"), Some(0xABCD));
        assert_eq!(parse_address("1234"), Some(1234));
        assert_eq!(parse_address("  0x10  "), Some(0x10));
        assert_eq!(parse_address("nope"), None);
        assert_eq!(parse_address(""), None);
        // Underscores are not supported (match main.rs parse_u32_addr).
        assert_eq!(parse_address("0x4000_0000"), None);
    }

    #[test]
    fn error_result_never_proven() {
        let r = DebugProbeResult::error("NO_FIRMWARE", "missing elf");
        assert!(!r.proven);
        assert_eq!(r.status, "error");
        assert_eq!(r.stop_reason, "config_error");
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["proven"], false);
        assert_eq!(v["error"]["code"], "NO_FIRMWARE");
        assert_eq!(v["error"]["detail"], "missing elf");
    }

    #[test]
    fn ok_payload_always_proven_false() {
        let mut r = DebugProbeResult::error("x", "y");
        r.status = "ok".into();
        r.stop_reason = "breakpoint".into();
        r.proven = false;
        r.error = None;
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["proven"], false);
        assert_eq!(v["status"], "ok");
    }

    #[test]
    fn stop_reason_mapping() {
        assert_eq!(
            stop_reason_str(&labwired_core::StopReason::Breakpoint(0x0800_0100)),
            "breakpoint"
        );
        assert_eq!(
            stop_reason_str(&labwired_core::StopReason::StepDone),
            "step_done"
        );
        assert_eq!(
            stop_reason_str(&labwired_core::StopReason::MaxStepsReached),
            "max_steps"
        );
        assert_eq!(
            stop_reason_str(&labwired_core::StopReason::ManualStop),
            "halt"
        );
    }

    #[test]
    fn breakpoint_spec_untagged_deser() {
        let addr: BreakpointSpec = serde_json::from_str(r#"{"address":"0x08000100"}"#).unwrap();
        assert_eq!(
            addr,
            BreakpointSpec::Address {
                address: "0x08000100".into()
            }
        );

        let sym: BreakpointSpec = serde_json::from_str(r#"{"symbol":"main"}"#).unwrap();
        assert_eq!(
            sym,
            BreakpointSpec::Symbol {
                symbol: "main".into()
            }
        );

        let line: BreakpointSpec = serde_json::from_str(r#"{"line":42,"file":"main.c"}"#).unwrap();
        assert_eq!(
            line,
            BreakpointSpec::Line {
                line: 42,
                file: Some("main.c".into())
            }
        );

        let line_only: BreakpointSpec = serde_json::from_str(r#"{"line":7}"#).unwrap();
        assert_eq!(
            line_only,
            BreakpointSpec::Line {
                line: 7,
                file: None
            }
        );
    }

    #[test]
    fn breakpoint_outcome_serializes_optional_fields() {
        let verified = BreakpointOutcome {
            requested: json!({"address":"0x1000"}),
            verified: true,
            address: Some("0x00001000".into()),
            message: None,
        };
        let v = serde_json::to_value(&verified).unwrap();
        assert_eq!(v["verified"], true);
        assert_eq!(v["address"], "0x00001000");
        assert!(v.get("message").is_none());

        let unverified = BreakpointOutcome {
            requested: json!({"symbol":"foo"}),
            verified: false,
            address: None,
            message: Some("symbol not found".into()),
        };
        let v = serde_json::to_value(&unverified).unwrap();
        assert_eq!(v["verified"], false);
        assert!(v.get("address").is_none());
        assert_eq!(v["message"], "symbol not found");
    }

    #[test]
    fn resolve_address_bp_without_symbols() {
        let specs = vec![BreakpointSpec::Address {
            address: "0x08000100".into(),
        }];
        let (outcomes, addrs) = resolve_breakpoints(&specs, None);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].verified);
        assert_eq!(outcomes[0].address.as_deref(), Some("0x08000100"));
        assert_eq!(addrs, vec![0x0800_0100]);
    }

    #[test]
    fn resolve_clears_thumb_bit_for_install() {
        // ELF Thumb symbols often have LSB set; engine matches pc & !1.
        let specs = vec![BreakpointSpec::Address {
            address: "0x080004bd".into(),
        }];
        let (outcomes, addrs) = resolve_breakpoints(&specs, None);
        assert!(outcomes[0].verified);
        assert_eq!(outcomes[0].address.as_deref(), Some("0x080004bc"));
        assert_eq!(addrs, vec![0x0800_04bc]);
    }

    #[test]
    fn resolve_symbol_unverified_without_symbols() {
        let specs = vec![BreakpointSpec::Symbol {
            symbol: "main".into(),
        }];
        let (outcomes, addrs) = resolve_breakpoints(&specs, None);
        assert!(!outcomes[0].verified);
        assert!(outcomes[0].address.is_none());
        assert!(outcomes[0].message.as_ref().unwrap().contains("symbol"));
        assert!(addrs.is_empty());
    }

    #[test]
    fn resolve_invalid_address_never_fakes() {
        let specs = vec![BreakpointSpec::Address {
            address: "not-an-addr".into(),
        }];
        let (outcomes, addrs) = resolve_breakpoints(&specs, None);
        assert!(!outcomes[0].verified);
        assert!(outcomes[0].address.is_none());
        assert!(addrs.is_empty());
    }

    #[test]
    fn read_set_defaults_when_empty() {
        let r = ReadSet::parse("");
        assert!(r.serial && r.regs && r.pc && r.location);
        let r2 = ReadSet::parse("pc,serial");
        assert!(r2.pc && r2.serial && !r2.regs && !r2.location);
    }
}
