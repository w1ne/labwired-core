// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::adapter::LabwiredAdapter;
use anyhow::{anyhow, Result};
// use dap::requests::Request;
// use dap::responses::ResponseBody;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

pub struct DapServer {
    adapter: LabwiredAdapter,
    running: Arc<Mutex<bool>>,
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
    pub fn new() -> Self {
        Self {
            adapter: LabwiredAdapter::new(),
            running: Arc::new(Mutex::new(false)),
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

        std::thread::spawn(move || loop {
            let is_running = {
                let r = running_clone.lock().unwrap();
                *r
            };

            if is_running {
                // Run a chunk
                match adapter_clone.continue_execution_chunk(10_000) {
                    Ok(reason) => {
                        match reason {
                            labwired_core::StopReason::Breakpoint(_) | labwired_core::StopReason::ManualStop => {
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
                        let mut r = running_clone.lock().unwrap();
                        *r = false;
                        tracing::error!("Execution error: {}", e);
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
            }

            // Telemetry polling
            if let Some(telemetry) = adapter_clone.get_telemetry() {
                let _ = sender_clone.send_event("telemetry", Some(serde_json::to_value(telemetry).unwrap()));
            }

            if !is_running {
                std::thread::sleep(std::time::Duration::from_millis(50));
            } else {
                std::thread::yield_now();
            }
        });

        // Start GDB Server
        let gdb_adapter = self.adapter.clone();
        std::thread::spawn(move || {
            let gdb = crate::gdb::GdbServer::new(gdb_adapter);
            if let Err(e) = gdb.listen("127.0.0.1:3333") {
                tracing::error!("GDB Server failed: {}", e);
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

            let command = request.get("command").and_then(|v| v.as_str()).unwrap_or("unknown");
            let req_seq = request.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
            let arguments = request.get("arguments");

            // Handle request
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
                        })),
                    )?;
                    sender.send_event("initialized", None)?;
                }
                "launch" => {
                    let program = arguments.and_then(|a| a.get("program")).and_then(|v| v.as_str());
                    let system_config = arguments.and_then(|a| a.get("systemConfig")).and_then(|v| v.as_str());

                    tracing::info!("Launching: program={:?}, systemConfig={:?}", program, system_config);

                    if let Some(p) = program {
                        if let Err(e) = self.adapter.load_firmware(p.into(), system_config.map(|s| s.into())) {
                            tracing::error!("Failed to load firmware: {}", e);
                            sender.send_error_response(req_seq, "launch", &format!("Failed to load firmware: {}", e))?;
                            continue;
                        }
                    }
                    sender.send_response(req_seq, "launch", None)?;
                }
                "disconnect" => {
                    sender.send_response(req_seq, "disconnect", None)?;
                    return Ok(());
                }
                "setBreakpoints" => {
                    let args = arguments.unwrap();
                    let source = args.get("source").unwrap();
                    let path = source.get("path").and_then(|v| v.as_str()).unwrap_or_default();
                    let lines = args.get("breakpoints").and_then(|v| v.as_array()).map(|arr| {
                        arr.iter().map(|b| b.get("line").and_then(|v| v.as_i64()).unwrap_or(0)).collect::<Vec<i64>>()
                    }).unwrap_or_default();

                    if let Err(e) = self.adapter.set_breakpoints(path.to_string(), lines.clone()) {
                        tracing::error!("Failed to set breakpoints: {}", e);
                    }

                    let breakpoints: Vec<Value> = lines.into_iter().map(|l| json!({
                        "verified": true,
                        "line": l,
                    })).collect();

                    sender.send_response(req_seq, "setBreakpoints", Some(json!({ "breakpoints": breakpoints })))?;
                }
                "configurationDone" => {
                    sender.send_response(req_seq, "configurationDone", None)?;
                    sender.send_event("stopped", Some(json!({
                        "reason": "entry",
                        "threadId": 1,
                        "allThreadsStopped": true
                    })))?;
                }
                "threads" => {
                    sender.send_response(req_seq, "threads", Some(json!({
                        "threads": [{"id": 1, "name": "Core 0"}]
                    })))?;
                }
                "stackTrace" => {
                    let pc = self.adapter.get_pc().unwrap_or(0);
                    let source_loc = self.adapter.lookup_source(pc as u64);

                    let (source, line, name) = if let Some(loc) = source_loc {
                        let source = json!({
                            "name": std::path::Path::new(&loc.file).file_name().and_then(|n| n.to_str()).unwrap_or(&loc.file),
                            "path": loc.file,
                        });
                        (Some(source), loc.line, loc.function.unwrap_or_else(|| "main".to_string()))
                    } else {
                        // If unknown, give it a name like "0x2af00 (No source)"
                        (None, Some(0), format!("{:#x} (No debug symbols)", pc))
                    };

                    sender.send_response(req_seq, "stackTrace", Some(json!({
                        "stackFrames": [{
                            "id": 1,
                            "name": name,
                            "line": line.unwrap_or(0),
                            "column": 0,
                            "source": source,
                            "instructionPointerReference": format!("{:#x}", pc),
                        }],
                        "totalFrames": 1
                    })))?;
                }
                "scopes" => {
                    let reg_count = self.adapter.get_register_names().map(|n| n.len()).unwrap_or(16);
                    sender.send_response(req_seq, "scopes", Some(json!({
                        "scopes": [{
                            "name": "Registers",
                            "variablesReference": 1,
                            "namedVariables": reg_count,
                            "expensive": false,
                        }]
                    })))?;
                }
                "variables" => {
                    let var_ref = arguments.and_then(|a| a.get("variablesReference")).and_then(|v| v.as_i64()).unwrap_or(0);
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
                        sender.send_response(req_seq, "variables", Some(json!({ "variables": variables })))?;
                    } else if var_ref == 0 {
                        sender.send_response(req_seq, "variables", Some(json!({ "variables": [] })))?;
                    } else {
                        // For other references (e.g. memory groups), return empty for now
                        sender.send_response(req_seq, "variables", Some(json!({ "variables": [] })))?;
                    }
                }
                "disassemble" => {
                    let args = arguments.unwrap();
                    let addr_str = args.get("memoryReference").and_then(|v| v.as_str()).unwrap_or("0");
                    let addr = if addr_str.starts_with("0x") {
                        u64::from_str_radix(&addr_str[2..], 16).unwrap_or(0)
                    } else {
                        addr_str.parse().unwrap_or(0)
                    };
                    let instruction_count = args.get("instructionCount").and_then(|v| v.as_i64()).unwrap_or(10) as usize;
                    let instruction_offset = args.get("instructionOffset").and_then(|v| v.as_i64()).unwrap_or(0);
                    
                    let start_addr = (addr as i64 + instruction_offset * 2) as u64; // Assuming 2-byte thumb instructions for simple offset
                    
                    let mut instructions = Vec::new();
                    // Read data in chunks to disassemble
                    if let Ok(data) = self.adapter.read_memory(start_addr, instruction_count * 4) {
                        for i in 0..instruction_count {
                            let curr_addr = start_addr + (i * 2) as u64;
                            let idx = i * 2;
                            if idx + 2 > data.len() { break; }
                            
                            let opcode = (data[idx] as u16) | ((data[idx+1] as u16) << 8);
                            let instr = labwired_core::decoder::decode_thumb_16(opcode);
                            
                            // Better formatting for "Ozone-like" feel
                            let instr_str = format!("{:?}", instr);
                            let display_instr = instr_str.split_whitespace().next().unwrap_or("unknown").to_uppercase();
                            let operands = instr_str.split_once(' ').map(|(_, rest)| rest).unwrap_or("");
                            
                            instructions.push(json!({
                                "address": format!("{:#x}", curr_addr),
                                "instruction": format!("{} {}", display_instr, operands),
                                "instructionBytes": format!("{:02x}{:02x}", data[idx+1], data[idx]),
                            }));
                        }
                    }
                    
                    sender.send_response(req_seq, "disassemble", Some(json!({ "instructions": instructions })))?;
                }
                "readMemory" => {
                    let args = arguments.unwrap();
                    let addr_str = args.get("memoryReference").and_then(|v| v.as_str()).unwrap_or("0");
                    let addr = if addr_str.starts_with("0x") {
                        u64::from_str_radix(&addr_str[2..], 16).unwrap_or(0)
                    } else {
                        addr_str.parse().unwrap_or(0)
                    };
                    let offset = args.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);
                    let count = args.get("count").and_then(|v| v.as_i64()).unwrap_or(64) as usize;
                    
                    let final_addr = (addr as i64 + offset) as u64;
                    
                    match self.adapter.read_memory(final_addr, count) {
                        Ok(data) => {
                            use base64::Engine;
                            let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                            sender.send_response(req_seq, "readMemory", Some(json!({
                                "address": format!("{:#x}", final_addr),
                                "unreadableBytes": 0,
                                "data": encoded,
                            })))?;
                        }
                        Err(e) => {
                            sender.send_error_response(req_seq, "readMemory", &format!("Read failed: {}", e))?;
                        }
                    }
                }
                "continue" => {
                    {
                        let mut r = self.running.lock().unwrap();
                        *r = true;
                    }
                    sender.send_response(req_seq, "continue", Some(json!({ "allThreadsContinued": true })))?;
                }
                "pause" => {
                    {
                        let mut r = self.running.lock().unwrap();
                        *r = false;
                    }
                    sender.send_response(req_seq, "pause", None)?;
                    sender.send_event("stopped", Some(json!({
                        "reason": "pause",
                        "threadId": 1,
                        "allThreadsStopped": true
                    })))?;
                }
                "restart" => {
                    if let Err(e) = self.adapter.reset() {
                        sender.send_error_response(req_seq, "restart", &format!("Reset failed: {}", e))?;
                    } else {
                        sender.send_response(req_seq, "restart", None)?;
                        sender.send_event("stopped", Some(json!({
                            "reason": "entry",
                            "threadId": 1,
                            "allThreadsStopped": true
                        })))?;
                    }
                }
                "gotoTargets" => {
                    // VS Code calls this to find where it can jump
                    let args = arguments.unwrap();
                    let line = args.get("line").and_then(|v| v.as_i64()).unwrap_or(0);
                    let source = args.get("source").unwrap();
                    let path = source.get("path").and_then(|v| v.as_str()).unwrap_or_default();
                    
                    let mut targets = Vec::new();
                    if let Some(target_addr) = self.adapter.lookup_source_reverse(path, line as u32) {
                        targets.push(json!({
                            "id": 1,
                            "label": format!("Jump to line {}", line),
                            "line": line,
                            "column": 0,
                            "instructionPointerReference": format!("{:#x}", target_addr),
                        }));
                    }
                    
                    sender.send_response(req_seq, "gotoTargets", Some(json!({ "targets": targets })))?;
                }
                "goto" => {
                    let args = arguments.unwrap();
                    let _target_id = args.get("targetId").and_then(|v| v.as_i64()).unwrap_or(0);
                    // For now we only have one target id = 1 which means "the resolved instruction"
                    // In a real implementation we'd lookup the target by ID.
                    
                    // VS Code usually sends instructionPointerReference if gotoTargets was used
                    let addr_str = args.get("instructionPointerReference").and_then(|v| v.as_str());
                    let addr = if let Some(a) = addr_str {
                        if a.starts_with("0x") {
                            u32::from_str_radix(&a[2..], 16).unwrap_or(0)
                        } else {
                            a.parse().unwrap_or(0)
                        }
                    } else {
                        0
                    };
                    
                    if addr != 0 {
                        let _ = self.adapter.set_pc(addr);
                        sender.send_response(req_seq, "goto", None)?;
                        sender.send_event("stopped", Some(json!({
                            "reason": "goto",
                            "threadId": 1,
                            "allThreadsStopped": true
                        })))?;
                    } else {
                        sender.send_error_response(req_seq, "goto", "Invalid target address")?;
                    }
                }
                "next" | "stepIn" => {
                    let _ = self.adapter.step();
                    sender.send_response(req_seq, "next", None)?;
                    sender.send_event("stopped", Some(json!({
                        "reason": "step",
                        "threadId": 1,
                        "allThreadsStopped": true
                    })))?;
                }
                "readInstructionTrace" => {
                    let (start_cycle, end_cycle) = if let Some(args) = arguments {
                        let start = args.get("startCycle").and_then(|v| v.as_u64()).unwrap_or(0);
                        let end = args.get("endCycle").and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
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
                    let trace_records: Vec<Value> = traces.iter().map(|t| {
                        json!({
                            "pc": t.pc,
                            "cycle": t.cycle,
                            "instruction": t.instruction,
                            "function": t.function,
                            "registers": t.register_delta,
                        })
                    }).collect();
                    
                    sender.send_response(req_seq, "readInstructionTrace", Some(json!({
                        "traces": trace_records,
                        "totalCycles": self.adapter.get_cycle_count(),
                    })))?;
                }
                _ => {
                    tracing::warn!("Unhandled command: {}", command);
                    sender.send_response(req_seq, command, None)?;
                }
            }
        }
    }
}
