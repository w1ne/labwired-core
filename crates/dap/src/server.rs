// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::adapter::LabwiredAdapter;
use anyhow::{anyhow, Result};
// use dap::requests::Request;
// use dap::responses::ResponseBody;
use base64::Engine;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct DapServer {
    pub adapter: LabwiredAdapter,
    running: Arc<Mutex<bool>>,
    stop_on_entry: Arc<Mutex<bool>>,
}

#[derive(Serialize)]
struct DapResponse {
    seq: i64,
    #[serde(rename = "type")]
    type_: String,
    request_seq: i64,
    success: bool,
    command: String,
    message: Option<String>,
    body: Option<Value>,
}

#[derive(Serialize)]
struct DapEvent {
    seq: i64,
    #[serde(rename = "type")]
    type_: String,
    event: String,
    body: Option<Value>,
}

#[derive(Serialize)]
#[allow(dead_code)]
struct OutputEventBody {
    category: String,
    output: String,
}

enum HandleResult {
    Continue,
    Shutdown,
}

#[derive(Serialize)]
struct ProfileNode {
    name: String,
    value: u64,
    children: std::collections::HashMap<String, ProfileNode>,
}

impl ProfileNode {
    fn new(name: String) -> Self {
        Self {
            name,
            value: 0,
            children: std::collections::HashMap::new(),
        }
    }
}

fn profile_to_json(node: ProfileNode) -> Value {
    let mut children: Vec<Value> = node.children.into_values().map(profile_to_json).collect();
    // Sort by value descending
    children.sort_by(|a, b| b["value"].as_u64().cmp(&a["value"].as_u64()));

    json!({
        "name": node.name,
        "value": node.value,
        "children": children
    })
}

const MAX_PROFILE_DEPTH: usize = 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaunchArgs {
    #[serde(default)]
    program: Option<String>,
    #[serde(default)]
    system_config: Option<String>,
    #[serde(default)]
    stop_on_entry: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct SourceArg {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BreakpointArg {
    line: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetBreakpointsArgs {
    source: SourceArg,
    #[serde(default)]
    breakpoints: Vec<BreakpointArg>,
}

#[derive(Debug, Deserialize)]
struct EvaluateArgs {
    expression: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DisassembleArgs {
    memory_reference: String,
    #[serde(default)]
    instruction_count: Option<i64>,
    #[serde(default)]
    instruction_offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadMemoryArgs {
    memory_reference: String,
    #[serde(default)]
    offset: Option<i64>,
    #[serde(default)]
    count: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteMemoryArgs {
    memory_reference: String,
    #[serde(default)]
    offset: Option<i64>,
    data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GotoTargetsArgs {
    #[serde(default)]
    line: Option<i64>,
    source: SourceArg,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GotoArgs {
    #[serde(default)]
    instruction_pointer_reference: Option<String>,
}

fn parse_address(value: &str) -> Option<u64> {
    if let Some(stripped) = value.strip_prefix("0x") {
        u64::from_str_radix(stripped, 16).ok()
    } else {
        value.parse().ok()
    }
}

fn build_profile_tree(traces: Vec<crate::trace::InstructionTrace>) -> ProfileNode {
    let mut root = ProfileNode::new("root".to_string());
    let mut path: Vec<String> = Vec::new();
    let mut stack_baseline: Option<u32> = None;

    for t in traces {
        let func_name = t.function.unwrap_or_else(|| "unknown".to_string());

        // Stack grows downward on Cortex-M; convert absolute SP into relative depth.
        let baseline = stack_baseline.get_or_insert(t.stack_depth);
        let depth_words = baseline.saturating_sub(t.stack_depth) / 4;
        let depth = usize::min(depth_words as usize, MAX_PROFILE_DEPTH);

        if path.len() <= depth {
            path.resize(depth + 1, func_name.clone());
        } else {
            path.truncate(depth + 1);
        }
        path[depth] = func_name;

        let mut curr = &mut root;
        for segment in &path {
            curr = curr
                .children
                .entry(segment.clone())
                .or_insert(ProfileNode::new(segment.clone()));
        }
        curr.value += 1;
        root.value += 1;
    }

    root
}

struct MessageSender<W: Write> {
    output: Arc<Mutex<W>>,
    seq: Arc<AtomicI64>,
}

impl<W: Write> MessageSender<W> {
    fn send_response(&self, request_seq: i64, command: &str, body: Option<Value>) -> Result<()> {
        let response = DapResponse {
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            type_: "response".to_string(),
            request_seq,
            success: true,
            command: command.to_string(),
            message: None,
            body,
        };
        self.send(serde_json::to_string(&response)?)
    }

    fn send_error_response(&self, request_seq: i64, command: &str, message: &str) -> Result<()> {
        let response = DapResponse {
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            type_: "response".to_string(),
            request_seq,
            success: false,
            command: command.to_string(),
            message: Some(message.to_string()),
            body: None,
        };
        self.send(serde_json::to_string(&response)?)
    }

    fn send_event(&self, event_name: &str, body: Option<Value>) -> Result<()> {
        let event = DapEvent {
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            type_: "event".to_string(),
            event: event_name.to_string(),
            body,
        };
        self.send(serde_json::to_string(&event)?)
    }

    fn send(&self, json: String) -> Result<()> {
        tracing::info!("DAP OUT: {}", json);
        let mut out = self.output.lock().map_err(|_| anyhow!("Mutex poisoned"))?;
        write!(out, "Content-Length: {}\r\n\r\n{}", json.len(), json)?;
        out.flush()?;
        Ok(())
    }
}

impl Default for DapServer {
    fn default() -> Self {
        Self::new()
    }
}

impl DapServer {
    fn parse_args<T: DeserializeOwned, W: Write>(
        req_seq: i64,
        command: &str,
        arguments: Option<&Value>,
        sender: &MessageSender<W>,
    ) -> Result<Option<T>> {
        let Some(args) = arguments else {
            sender.send_error_response(req_seq, command, "Missing required arguments")?;
            return Ok(None);
        };

        match serde_json::from_value::<T>(args.clone()) {
            Ok(parsed) => Ok(Some(parsed)),
            Err(err) => {
                sender.send_error_response(
                    req_seq,
                    command,
                    &format!("Invalid arguments: {}", err),
                )?;
                Ok(None)
            }
        }
    }

    pub fn new() -> Self {
        Self {
            adapter: LabwiredAdapter::new(),
            running: Arc::new(Mutex::new(false)),
            stop_on_entry: Arc::new(Mutex::new(true)),
        }
    }

    pub fn run<R: Read, W: Write + Send + 'static>(&self, input: R, output: W) -> Result<()> {
        let mut reader = BufReader::new(input);
        let output = Arc::new(Mutex::new(output));
        let seq = Arc::new(AtomicI64::new(1));
        let sender = MessageSender {
            output: output.clone(),
            seq: seq.clone(),
        };

        // Start Execution/UART/Telemetry loop
        let adapter_clone = self.adapter.clone();
        let sender_clone = MessageSender {
            output: output.clone(),
            seq: seq.clone(),
        };
        let running_clone = self.running.clone();

        std::thread::spawn(move || {
            let mut last_telemetry_sent = Instant::now()
                .checked_sub(Duration::from_millis(500))
                .unwrap_or_else(Instant::now);
            loop {
                let is_running = {
                    let r = running_clone.lock().unwrap();
                    *r
                };

                if is_running {
                    // Run a chunk
                    match adapter_clone.continue_execution_chunk(10_000) {
                        Ok(reason) => {
                            match reason {
                                labwired_core::StopReason::Breakpoint(_)
                                | labwired_core::StopReason::ManualStop => {
                                    {
                                        let mut r = running_clone.lock().unwrap();
                                        *r = false;
                                    }
                                    let _ = sender_clone.send_event("stopped", Some(json!({
                                    "reason": match reason {
                                        labwired_core::StopReason::Breakpoint(_) => "breakpoint",
                                        _ => "pause",
                                    },
                                    "threadId": 1,
                                    "allThreadsStopped": true
                                })));
                                }
                                _ => {} // Continue running
                            }
                        }
                        Err(e) => {
                            let error_message = format!("Execution error: {}", e);
                            let mut r = running_clone.lock().unwrap();
                            *r = false;
                            tracing::error!("{}", error_message);
                            let _ = sender_clone.send_event(
                                "output",
                                Some(json!({
                                    "category": "stderr",
                                    "output": format!("{}\n", error_message),
                                })),
                            );
                            let _ = sender_clone.send_event(
                                "stopped",
                                Some(json!({
                                    "reason": "exception",
                                    "description": error_message,
                                    "text": error_message,
                                    "threadId": 1,
                                    "allThreadsStopped": true
                                })),
                            );
                        }
                    }
                }

                // UART polling
                let data = adapter_clone.poll_uart();
                if !data.is_empty() {
                    let text = String::from_utf8_lossy(&data).to_string();
                    let _ = sender_clone.send_event(
                        "output",
                        Some(json!({
                            "category": "stdout",
                            "output": text,
                        })),
                    );
                    // Duplicate UART stream as a custom event so host IDEs that
                    // don't surface standard `output` through custom-event hooks
                    // can still render UART in extension UI panels.
                    let _ = sender_clone.send_event(
                        "uart",
                        Some(json!({
                            "output": text,
                        })),
                    );
                }

                // Telemetry polling
                let telemetry_interval = if is_running {
                    Duration::from_millis(100)
                } else {
                    Duration::from_millis(400)
                };

                if last_telemetry_sent.elapsed() >= telemetry_interval {
                    if let Some(telemetry) = adapter_clone.get_telemetry() {
                        let _ = sender_clone.send_event(
                            "telemetry",
                            Some(serde_json::to_value(telemetry).unwrap()),
                        );
                    }
                    last_telemetry_sent = Instant::now();
                }

                if !is_running {
                    std::thread::sleep(Duration::from_millis(50));
                } else {
                    std::thread::yield_now();
                }
            }
        });

        loop {
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line)? == 0 {
                    return Ok(()); // EOF
                }
                let line = line.trim();
                if line.is_empty() {
                    break; // End of headers
                }
                if let Some(rest) = line.strip_prefix("Content-Length: ") {
                    if let Ok(len) = rest.parse() {
                        content_length = len;
                    }
                }
            }

            if content_length == 0 {
                continue;
            }

            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body)?;

            let request: Value = match serde_json::from_slice(&body) {
                Ok(req) => req,
                Err(e) => {
                    tracing::error!("Failed to parse request: {}", e);
                    continue;
                }
            };
            tracing::info!("Received request: {}", request);

            let command = request
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let req_seq = request.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
            let arguments = request.get("arguments");

            // Handle request
            match self.handle_request(req_seq, command, arguments, &sender) {
                Ok(HandleResult::Shutdown) => return Ok(()),
                Ok(HandleResult::Continue) => {}
                Err(e) => tracing::error!("Error handling request {}: {}", command, e),
            }
        }
    }

    fn handle_request<W: Write>(
        &self,
        req_seq: i64,
        command: &str,
        arguments: Option<&Value>,
        sender: &MessageSender<W>,
    ) -> Result<HandleResult> {
        match command {
            "initialize" => {
                sender.send_response(
                    req_seq,
                    "initialize",
                    Some(json!({
                        "supportsConfigurationDoneRequest": true,
                        "supportsFunctionBreakpoints": true,
                        "supportsConditionalBreakpoints": true,
                        "supportsDisassembleRequest": true,
                        "supportsReadMemoryRequest": true,
                        "supportsSteppingGranularity": true,
                        "supportsPauseRequest": true,
                        "supportsRestartRequest": true,
                        "supportsGotoTargetsRequest": true,
                        "supportsStepBack": true,
                    })),
                )?;
                sender.send_event("initialized", None)?;
            }
            "launch" => {
                let launch = if let Some(parsed) =
                    Self::parse_args::<LaunchArgs, _>(req_seq, "launch", arguments, sender)?
                {
                    parsed
                } else {
                    return Ok(HandleResult::Continue);
                };
                let stop_on_entry = launch.stop_on_entry.unwrap_or(true);

                tracing::info!(
                    "Launching: program={:?}, systemConfig={:?}, stopOnEntry={}",
                    launch.program,
                    launch.system_config,
                    stop_on_entry
                );
                *self.stop_on_entry.lock().unwrap() = stop_on_entry;

                if let Some(p) = launch.program {
                    if let Err(e) = self
                        .adapter
                        .load_firmware(p.into(), launch.system_config.map(|s| s.into()))
                    {
                        tracing::error!("Failed to load firmware: {}", e);
                        sender.send_error_response(
                            req_seq,
                            "launch",
                            &format!("Failed to load firmware: {}", e),
                        )?;
                        return Ok(HandleResult::Continue);
                    }
                }
                sender.send_response(req_seq, "launch", None)?;
            }
            "disconnect" => {
                sender.send_response(req_seq, "disconnect", None)?;
                return Ok(HandleResult::Shutdown);
            }
            "setBreakpoints" => {
                let Some(args) = Self::parse_args::<SetBreakpointsArgs, _>(
                    req_seq,
                    "setBreakpoints",
                    arguments,
                    sender,
                )?
                else {
                    return Ok(HandleResult::Continue);
                };
                let Some(path) = args.source.path else {
                    sender.send_error_response(
                        req_seq,
                        "setBreakpoints",
                        "Missing required field: source.path",
                    )?;
                    return Ok(HandleResult::Continue);
                };
                let lines = args
                    .breakpoints
                    .into_iter()
                    .map(|b| b.line.unwrap_or(0))
                    .collect::<Vec<i64>>();

                let resolutions = match self.adapter.set_breakpoints(path, lines.clone()) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("Failed to set breakpoints: {}", e);
                        lines
                            .into_iter()
                            .map(|requested_line| crate::adapter::BreakpointResolution {
                                requested_line,
                                verified: false,
                                resolved_line: None,
                                address: None,
                                message: Some(format!("Failed to set breakpoint: {}", e)),
                            })
                            .collect()
                    }
                };

                let breakpoints: Vec<Value> = resolutions
                    .into_iter()
                    .map(|bp| {
                        let line = bp
                            .resolved_line
                            .map(|v| v as i64)
                            .unwrap_or(bp.requested_line);
                        let mut obj = json!({
                            "verified": bp.verified,
                            "line": line,
                        });
                        if let Some(message) = bp.message {
                            obj["message"] = json!(message);
                        }
                        obj
                    })
                    .collect();

                sender.send_response(
                    req_seq,
                    "setBreakpoints",
                    Some(json!({ "breakpoints": breakpoints })),
                )?;
            }
            "configurationDone" => {
                sender.send_response(req_seq, "configurationDone", None)?;
                let stop_on_entry = *self.stop_on_entry.lock().unwrap();
                if stop_on_entry {
                    sender.send_event(
                        "stopped",
                        Some(json!({
                            "reason": "entry",
                            "threadId": 1,
                            "allThreadsStopped": true
                        })),
                    )?;
                } else {
                    {
                        let mut r = self.running.lock().unwrap();
                        *r = true;
                    }
                    sender.send_event(
                        "continued",
                        Some(json!({
                            "threadId": 1,
                            "allThreadsContinued": true
                        })),
                    )?;
                }
            }
            "threads" => {
                sender.send_response(
                    req_seq,
                    "threads",
                    Some(json!({
                        "threads": [{"id": 1, "name": "Core 0"}]
                    })),
                )?;
            }
            "stackTrace" => {
                let pc = self.adapter.get_pc().unwrap_or(0);
                let source_loc = self.adapter.lookup_source(pc as u64);

                let (source, line, name) = if let Some(loc) = source_loc {
                    let source = json!({
                        "name": std::path::Path::new(&loc.file).file_name().and_then(|n| n.to_str()).unwrap_or(&loc.file),
                        "path": loc.file,
                    });
                    (
                        Some(source),
                        loc.line,
                        loc.function.unwrap_or_else(|| "main".to_string()),
                    )
                } else {
                    // If unknown, give it a name like "0x2af00 (No source)"
                    (None, Some(0), format!("{:#x} (No debug symbols)", pc))
                };

                sender.send_response(
                    req_seq,
                    "stackTrace",
                    Some(json!({
                        "stackFrames": [{
                            "id": 1,
                            "name": name,
                            "line": line.unwrap_or(0),
                            "column": 0,
                            "source": source,
                            "instructionPointerReference": format!("{:#x}", pc),
                        }],
                        "totalFrames": 1
                    })),
                )?;
            }
            "scopes" => {
                let reg_count = self
                    .adapter
                    .get_register_names()
                    .map(|n| n.len())
                    .unwrap_or(16);
                sender.send_response(
                    req_seq,
                    "scopes",
                    Some(json!({
                        "scopes": [
                            {
                                "name": "Registers",
                                "variablesReference": 1,
                                "namedVariables": reg_count,
                                "expensive": false,
                            },
                            {
                                "name": "Locals",
                                "variablesReference": 100,
                                "namedVariables": 0,
                                "expensive": false,
                            }
                        ]
                    })),
                )?;
            }
            "variables" => {
                let var_ref = arguments
                    .and_then(|a| a.get("variablesReference"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                if var_ref == 1 {
                    let mut variables = Vec::new();
                    if let Ok(names) = self.adapter.get_register_names() {
                        for (i, name) in names.into_iter().enumerate() {
                            let val = self.adapter.get_register(i as u8).unwrap_or(0);
                            variables.push(json!({
                                "name": name,
                                "value": format!("{:#x}", val),
                                "variablesReference": 0,
                                "type": "uint32",
                                "presentationHint": { "kind": "property", "attributes": ["readOnly"] }
                            }));
                        }
                    }
                    sender.send_response(
                        req_seq,
                        "variables",
                        Some(json!({ "variables": variables })),
                    )?;
                } else if var_ref == 100 {
                    let pc = self.adapter.get_pc().unwrap_or(0);
                    let locals = self.adapter.get_locals(pc);
                    let mut variables = Vec::new();
                    for local in locals {
                        let val = match local.location {
                            labwired_loader::DwarfLocation::Register(r) => {
                                let val = self.adapter.get_register(r as u8).unwrap_or(0);
                                format!("{:#x}", val)
                            }
                            labwired_loader::DwarfLocation::FrameRelative(offset) => {
                                let sp = self.adapter.get_register(13).unwrap_or(0);
                                let addr = (sp as i64 + offset) as u32;
                                if let Ok(data) = self.adapter.read_memory(addr as u64, 4) {
                                    let val = (data[0] as u32)
                                        | ((data[1] as u32) << 8)
                                        | ((data[2] as u32) << 16)
                                        | ((data[3] as u32) << 24);
                                    format!("{:#x}", val)
                                } else {
                                    "error".to_string()
                                }
                            }
                            _ => "not available".to_string(),
                        };
                        variables.push(json!({
                            "name": local.name,
                            "value": val,
                            "variablesReference": 0,
                        }));
                    }
                    sender.send_response(
                        req_seq,
                        "variables",
                        Some(json!({ "variables": variables })),
                    )?;
                } else if var_ref == 0 {
                    sender.send_response(req_seq, "variables", Some(json!({ "variables": [] })))?;
                } else {
                    // For other references (e.g. memory groups), return empty for now
                    sender.send_response(req_seq, "variables", Some(json!({ "variables": [] })))?;
                }
            }
            "evaluate" => {
                let Some(args) =
                    Self::parse_args::<EvaluateArgs, _>(req_seq, "evaluate", arguments, sender)?
                else {
                    return Ok(HandleResult::Continue);
                };
                let expression = args.expression;

                // Try to evaluate as a register first
                let mut result_val = None;
                if let Ok(names) = self.adapter.get_register_names() {
                    for (i, name) in names.iter().enumerate() {
                        if name.eq_ignore_ascii_case(&expression) {
                            let val = self.adapter.get_register(i as u8).unwrap_or(0);
                            result_val = Some(format!("{:#x}", val));
                            break;
                        }
                    }
                }

                // If not a register, try as a symbol
                if result_val.is_none() {
                    if let Some(addr) = self.adapter.resolve_symbol(&expression) {
                        if let Ok(data) = self.adapter.read_memory(addr, 4) {
                            let val = (data[0] as u32)
                                | ((data[1] as u32) << 8)
                                | ((data[2] as u32) << 16)
                                | ((data[3] as u32) << 24);
                            result_val = Some(format!("{:#x}", val));
                        }
                    }
                }

                // If not a symbol, try as a local
                if result_val.is_none() {
                    let pc = self.adapter.get_pc().unwrap_or(0);
                    let locals = self.adapter.get_locals(pc);
                    for local in locals {
                        if local.name == expression {
                            result_val = match local.location {
                                labwired_loader::DwarfLocation::Register(r) => {
                                    let val = self.adapter.get_register(r as u8).unwrap_or(0);
                                    Some(format!("{:#x}", val))
                                }
                                labwired_loader::DwarfLocation::FrameRelative(offset) => {
                                    let sp = self.adapter.get_register(13).unwrap_or(0);
                                    let addr = (sp as i64 + offset) as u32;
                                    if let Ok(data) = self.adapter.read_memory(addr as u64, 4) {
                                        let val = (data[0] as u32)
                                            | ((data[1] as u32) << 8)
                                            | ((data[2] as u32) << 16)
                                            | ((data[3] as u32) << 24);
                                        Some(format!("{:#x}", val))
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            };
                            if result_val.is_some() {
                                break;
                            }
                        }
                    }
                }

                if let Some(val) = result_val {
                    sender.send_response(
                        req_seq,
                        "evaluate",
                        Some(json!({
                            "result": val,
                            "variablesReference": 0,
                            "type": "uint32"
                        })),
                    )?;
                } else {
                    sender.send_error_response(
                        req_seq,
                        "evaluate",
                        &format!("Failed to evaluate expression: {}", expression),
                    )?;
                }
            }
            "disassemble" => {
                let Some(args) = Self::parse_args::<DisassembleArgs, _>(
                    req_seq,
                    "disassemble",
                    arguments,
                    sender,
                )?
                else {
                    return Ok(HandleResult::Continue);
                };
                let addr = parse_address(&args.memory_reference).unwrap_or(0);
                let instruction_count = args.instruction_count.unwrap_or(10).max(0) as usize;
                let instruction_offset = args.instruction_offset.unwrap_or(0);

                let start_addr = (addr as i64 + instruction_offset * 2) as u64; // Assuming 2-byte thumb instructions for simple offset

                let mut instructions = Vec::new();
                // Read data in chunks to disassemble
                if let Ok(data) = self.adapter.read_memory(start_addr, instruction_count * 4) {
                    for i in 0..instruction_count {
                        let curr_addr = start_addr + (i * 2) as u64;
                        let idx = i * 2;
                        if idx + 2 > data.len() {
                            break;
                        }

                        let opcode = (data[idx] as u16) | ((data[idx + 1] as u16) << 8);
                        let instr = labwired_core::decoder::decode_thumb_16(opcode);

                        // Better formatting for "Ozone-like" feel
                        let instr_str = format!("{:?}", instr);
                        let display_instr = instr_str
                            .split_whitespace()
                            .next()
                            .unwrap_or("unknown")
                            .to_uppercase();
                        let operands = instr_str
                            .split_once(' ')
                            .map(|(_, rest)| rest)
                            .unwrap_or("");

                        instructions.push(json!({
                            "address": format!("{:#x}", curr_addr),
                            "instruction": format!("{} {}", display_instr, operands),
                            "instructionBytes": format!("{:02x}{:02x}", data[idx+1], data[idx]),
                        }));
                    }
                }

                sender.send_response(
                    req_seq,
                    "disassemble",
                    Some(json!({ "instructions": instructions })),
                )?;
            }
            "readMemory" => {
                let Some(args) = Self::parse_args::<ReadMemoryArgs, _>(
                    req_seq,
                    "readMemory",
                    arguments,
                    sender,
                )?
                else {
                    return Ok(HandleResult::Continue);
                };
                let addr = parse_address(&args.memory_reference).unwrap_or(0);
                let offset = args.offset.unwrap_or(0);
                let count = args.count.unwrap_or(64).max(0) as usize;

                let final_addr = (addr as i64 + offset) as u64;

                match self.adapter.read_memory(final_addr, count) {
                    Ok(data) => {
                        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                        sender.send_response(
                            req_seq,
                            "readMemory",
                            Some(json!({
                                "address": format!("{:#x}", final_addr),
                                "unreadableBytes": 0,
                                "data": encoded,
                            })),
                        )?;
                    }
                    Err(e) => {
                        sender.send_error_response(
                            req_seq,
                            "readMemory",
                            &format!("Read failed: {}", e),
                        )?;
                    }
                }
            }
            "writeMemory" => {
                let Some(args) = Self::parse_args::<WriteMemoryArgs, _>(
                    req_seq,
                    "writeMemory",
                    arguments,
                    sender,
                )?
                else {
                    return Ok(HandleResult::Continue);
                };
                let addr = parse_address(&args.memory_reference).unwrap_or(0);
                let offset = args.offset.unwrap_or(0);

                let data = match base64::engine::general_purpose::STANDARD.decode(&args.data) {
                    Ok(data) => data,
                    Err(e) => {
                        sender.send_error_response(
                            req_seq,
                            "writeMemory",
                            &format!("Base64 decode failed: {:?}", e),
                        )?;
                        return Ok(HandleResult::Continue);
                    }
                };

                let final_addr = (addr as i64 + offset) as u64;

                match self.adapter.write_memory(final_addr, &data) {
                    Ok(_) => {
                        sender.send_response(
                            req_seq,
                            "writeMemory",
                            Some(json!({
                                "bytesWritten": data.len(),
                            })),
                        )?;
                    }
                    Err(e) => {
                        sender.send_error_response(
                            req_seq,
                            "writeMemory",
                            &format!("Write failed: {}", e),
                        )?;
                    }
                }
            }
            "continue" => {
                {
                    let mut r = self.running.lock().unwrap();
                    *r = true;
                }
                sender.send_response(
                    req_seq,
                    "continue",
                    Some(json!({ "allThreadsContinued": true })),
                )?;
            }
            "pause" => {
                {
                    let mut r = self.running.lock().unwrap();
                    *r = false;
                }
                sender.send_response(req_seq, "pause", None)?;
                sender.send_event(
                    "stopped",
                    Some(json!({
                        "reason": "pause",
                        "threadId": 1,
                        "allThreadsStopped": true
                    })),
                )?;
            }
            "restart" => {
                if let Err(e) = self.adapter.reset() {
                    sender.send_error_response(
                        req_seq,
                        "restart",
                        &format!("Reset failed: {}", e),
                    )?;
                } else {
                    sender.send_response(req_seq, "restart", None)?;
                    sender.send_event(
                        "stopped",
                        Some(json!({
                            "reason": "entry",
                            "threadId": 1,
                            "allThreadsStopped": true
                        })),
                    )?;
                }
            }
            "gotoTargets" => {
                // VS Code calls this to find where it can jump
                let Some(args) = Self::parse_args::<GotoTargetsArgs, _>(
                    req_seq,
                    "gotoTargets",
                    arguments,
                    sender,
                )?
                else {
                    return Ok(HandleResult::Continue);
                };
                let line = args.line.unwrap_or(0);
                let Some(path) = args.source.path else {
                    sender.send_error_response(
                        req_seq,
                        "gotoTargets",
                        "Missing required field: source.path",
                    )?;
                    return Ok(HandleResult::Continue);
                };

                let mut targets = Vec::new();
                if let Some(target_addr) = self.adapter.lookup_source_reverse(&path, line as u32) {
                    targets.push(json!({
                        "id": 1,
                        "label": format!("Jump to line {}", line),
                        "line": line,
                        "column": 0,
                        "instructionPointerReference": format!("{:#x}", target_addr),
                    }));
                }

                sender.send_response(
                    req_seq,
                    "gotoTargets",
                    Some(json!({ "targets": targets })),
                )?;
            }
            "goto" => {
                let Some(args) =
                    Self::parse_args::<GotoArgs, _>(req_seq, "goto", arguments, sender)?
                else {
                    return Ok(HandleResult::Continue);
                };
                // For now we only have one target id = 1 which means "the resolved instruction"
                // In a real implementation we'd lookup the target by ID.

                // VS Code usually sends instructionPointerReference if gotoTargets was used
                let addr = args
                    .instruction_pointer_reference
                    .as_deref()
                    .and_then(parse_address)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(0);

                if addr != 0 {
                    let _ = self.adapter.set_pc(addr);
                    sender.send_response(req_seq, "goto", None)?;
                    sender.send_event(
                        "stopped",
                        Some(json!({
                            "reason": "goto",
                            "threadId": 1,
                            "allThreadsStopped": true
                        })),
                    )?;
                } else {
                    sender.send_error_response(req_seq, "goto", "Invalid target address")?;
                }
            }
            "next" | "stepIn" => {
                let reason = if command == "next" {
                    self.adapter
                        .step_over_source_line(512)
                        .unwrap_or(labwired_core::StopReason::StepDone)
                } else {
                    self.adapter
                        .step()
                        .unwrap_or(labwired_core::StopReason::StepDone)
                };
                sender.send_response(req_seq, command, None)?;
                sender.send_event(
                    "stopped",
                    Some(json!({
                        "reason": match reason {
                            labwired_core::StopReason::Breakpoint(_) => "breakpoint",
                            _ => "step",
                        },
                        "threadId": 1,
                        "allThreadsStopped": true
                    })),
                )?;
            }
            "stepBack" => {
                let _ = self.adapter.step_back();
                sender.send_response(req_seq, "stepBack", None)?;
                sender.send_event(
                    "stopped",
                    Some(json!({
                        "reason": "step",
                        "threadId": 1,
                        "allThreadsStopped": true
                    })),
                )?;
            }
            "stepOut" => {
                let _ = self.adapter.step_out();
                sender.send_response(req_seq, "stepOut", None)?;
                sender.send_event(
                    "stopped",
                    Some(json!({
                        "reason": "step",
                        "threadId": 1,
                        "allThreadsStopped": true
                    })),
                )?;
            }
            "readInstructionTrace" => {
                let (start_cycle, end_cycle) = if let Some(args) = arguments {
                    let start = args.get("startCycle").and_then(|v| v.as_u64()).unwrap_or(0);
                    let end = args
                        .get("endCycle")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(u64::MAX);
                    (start, end)
                } else {
                    (0, u64::MAX)
                };

                let traces = if start_cycle == 0 && end_cycle == u64::MAX {
                    // Get all traces
                    self.adapter.get_all_traces()
                } else {
                    // Get specific range
                    self.adapter.get_instruction_trace(start_cycle, end_cycle)
                };

                // Convert traces to JSON
                let trace_records: Vec<Value> = traces
                    .iter()
                    .map(|t| {
                        json!({
                            "pc": t.pc,
                            "cycle": t.cycle,
                            "instruction": t.instruction,
                            "function": t.function,
                            "registers": t.register_delta,
                            "memory_writes": t.memory_writes,
                            "stack_depth": t.stack_depth,
                            "mnemonic": t.mnemonic,
                        })
                    })
                    .collect();

                sender.send_response(
                    req_seq,
                    "readInstructionTrace",
                    Some(json!({
                        "traces": trace_records,
                        "totalCycles": self.adapter.get_cycle_count(),
                    })),
                )?;
            }
            "readProfilingData" => {
                let traces = self.adapter.get_all_traces();
                let root = build_profile_tree(traces);
                sender.send_response(req_seq, "readProfilingData", Some(profile_to_json(root)))?;
            }
            "readPeripherals" => {
                let peripherals = self.adapter.get_peripherals_json();
                sender.send_response(req_seq, "readPeripherals", Some(peripherals))?;
            }
            "readRTOSState" => {
                let state = self.adapter.get_rtos_state_json();
                sender.send_response(req_seq, "readRTOSState", Some(state))?;
            }
            _ => {
                tracing::warn!("Unhandled command: {}", command);
                sender.send_response(req_seq, command, None)?;
            }
        }
        Ok(HandleResult::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use labwired_core::DebugControl;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn test_handle_initialize() -> Result<()> {
        let server = DapServer::new();
        let output = Arc::new(Mutex::new(Vec::new()));
        let sender = MessageSender {
            output: output.clone(),
            seq: Arc::new(AtomicI64::new(1)),
        };

        server.handle_request(1, "initialize", None, &sender)?;

        let out = output.lock().unwrap();
        let out_str = String::from_utf8(out.clone())?;

        // Check for both response and initialized event
        assert!(out_str.contains("\"command\":\"initialize\""));
        assert!(out_str.contains("\"event\":\"initialized\""));
        Ok(())
    }

    #[test]
    fn test_handle_scopes() -> Result<()> {
        let server = DapServer::new();
        let output = Arc::new(Mutex::new(Vec::new()));
        let sender = MessageSender {
            output: output.clone(),
            seq: Arc::new(AtomicI64::new(1)),
        };

        server.handle_request(1, "scopes", None, &sender)?;

        let out = output.lock().unwrap();
        let out_str = String::from_utf8(out.clone())?;

        assert!(out_str.contains("\"name\":\"Registers\""));
        assert!(out_str.contains("\"variablesReference\":1"));
        assert!(out_str.contains("\"name\":\"Locals\""));
        assert!(out_str.contains("\"variablesReference\":100"));
        Ok(())
    }

    #[test]
    fn test_handle_variables_locals() -> Result<()> {
        let server = DapServer::new();
        let adapter = server.adapter.clone();

        // Mock a CPU and SP
        let mut bus = labwired_core::bus::SystemBus::new();
        let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = labwired_core::Machine::new(cpu, bus);
        machine.write_core_reg(13, 0x20001000);
        machine
            .write_memory(0x20000FFC, &[0x78, 0x56, 0x34, 0x12])
            .unwrap();
        *adapter.machine.lock().unwrap() = Some(Box::new(machine));

        // Mock symbols/locals
        let mut symbols = labwired_loader::SymbolProvider::new_empty();
        symbols.add_test_local(
            "my_local",
            labwired_loader::DwarfLocation::FrameRelative(-4),
        );
        *adapter.symbols.lock().unwrap() = Some(symbols);

        let output = Arc::new(Mutex::new(Vec::new()));
        let sender = MessageSender {
            output: output.clone(),
            seq: Arc::new(AtomicI64::new(1)),
        };

        let args = json!({ "variablesReference": 100 });
        server.handle_request(1, "variables", Some(&args), &sender)?;

        let out = output.lock().unwrap();
        let out_str = String::from_utf8(out.clone())?;

        assert!(out_str.contains("\"name\":\"my_local\""));
        assert!(out_str.contains("\"value\":\"0x12345678\""));
        Ok(())
    }

    #[test]
    fn test_handle_evaluate_fallback() -> Result<()> {
        let server = DapServer::new();
        let adapter = server.adapter.clone();

        // Set up machine for register R0=0xAA and local my_var=0xBB
        let mut bus = labwired_core::bus::SystemBus::new();
        let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = labwired_core::Machine::new(cpu, bus);
        machine.write_core_reg(0, 0xAA);
        machine.write_core_reg(13, 0x20001000);
        machine
            .write_memory(0x20000FFC, &[0xBB, 0x00, 0x00, 0x00])
            .unwrap();
        *adapter.machine.lock().unwrap() = Some(Box::new(machine));

        let mut symbols = labwired_loader::SymbolProvider::new_empty();
        symbols.add_test_local("my_var", labwired_loader::DwarfLocation::FrameRelative(-4));
        *adapter.symbols.lock().unwrap() = Some(symbols);

        let output = Arc::new(Mutex::new(Vec::new()));
        let sender = MessageSender {
            output: output.clone(),
            seq: Arc::new(AtomicI64::new(1)),
        };

        // 1. Evaluate register
        server.handle_request(1, "evaluate", Some(&json!({"expression": "R0"})), &sender)?;
        assert!(String::from_utf8(output.lock().unwrap().clone())?.contains("\"result\":\"0xaa\""));
        output.lock().unwrap().clear();

        // 2. Evaluate local
        server.handle_request(
            2,
            "evaluate",
            Some(&json!({"expression": "my_var"})),
            &sender,
        )?;
        assert!(String::from_utf8(output.lock().unwrap().clone())?.contains("\"result\":\"0xbb\""));

        Ok(())
    }

    #[test]
    fn test_handle_set_breakpoints_missing_arguments_returns_error() -> Result<()> {
        let server = DapServer::new();
        let output = Arc::new(Mutex::new(Vec::new()));
        let sender = MessageSender {
            output: output.clone(),
            seq: Arc::new(AtomicI64::new(1)),
        };

        server.handle_request(1, "setBreakpoints", None, &sender)?;

        let out = output.lock().unwrap();
        let out_str = String::from_utf8(out.clone())?;
        assert!(out_str.contains("\"command\":\"setBreakpoints\""));
        assert!(out_str.contains("\"success\":false"));
        assert!(out_str.contains("Missing required arguments"));
        Ok(())
    }

    #[test]
    fn test_handle_set_breakpoints_reports_unverified_without_symbols() -> Result<()> {
        let server = DapServer::new();
        let output = Arc::new(Mutex::new(Vec::new()));
        let sender = MessageSender {
            output: output.clone(),
            seq: Arc::new(AtomicI64::new(1)),
        };

        let args = json!({
            "source": { "path": "/tmp/main.rs" },
            "breakpoints": [{ "line": 120 }]
        });

        server.handle_request(1, "setBreakpoints", Some(&args), &sender)?;

        let out = output.lock().unwrap();
        let out_str = String::from_utf8(out.clone())?;
        assert!(out_str.contains("\"command\":\"setBreakpoints\""));
        assert!(out_str.contains("\"verified\":false"));
        Ok(())
    }

    #[test]
    fn test_handle_write_memory_invalid_base64_returns_error() -> Result<()> {
        let server = DapServer::new();
        let output = Arc::new(Mutex::new(Vec::new()));
        let sender = MessageSender {
            output: output.clone(),
            seq: Arc::new(AtomicI64::new(1)),
        };
        let args = json!({
            "memoryReference": "0x20000000",
            "offset": 0,
            "data": "%%%not-base64%%%"
        });

        server.handle_request(1, "writeMemory", Some(&args), &sender)?;

        let out = output.lock().unwrap();
        let out_str = String::from_utf8(out.clone())?;
        assert!(out_str.contains("\"command\":\"writeMemory\""));
        assert!(out_str.contains("\"success\":false"));
        assert!(out_str.contains("Base64 decode failed"));
        Ok(())
    }

    fn max_profile_depth(node: &ProfileNode) -> usize {
        if node.children.is_empty() {
            return 0;
        }
        1 + node
            .children
            .values()
            .map(max_profile_depth)
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn test_build_profile_tree_clamps_depth() {
        let traces = vec![
            crate::trace::InstructionTrace {
                pc: 0,
                instruction: 0,
                cycle: 0,
                function: Some("entry".to_string()),
                register_delta: HashMap::new(),
                memory_writes: Vec::new(),
                stack_depth: 0x2000_1000,
                mnemonic: None,
            },
            crate::trace::InstructionTrace {
                pc: 2,
                instruction: 0,
                cycle: 1,
                function: Some("deep".to_string()),
                register_delta: HashMap::new(),
                memory_writes: Vec::new(),
                stack_depth: 0x2000_1000u32.saturating_sub(((MAX_PROFILE_DEPTH + 500) as u32) * 4),
                mnemonic: None,
            },
        ];

        let root = build_profile_tree(traces);
        assert_eq!(max_profile_depth(&root), MAX_PROFILE_DEPTH + 1);
    }
}
