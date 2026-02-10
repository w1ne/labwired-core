use crate::adapter::LabwiredAdapter;
use anyhow::Result;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

pub struct GdbServer {
    adapter: LabwiredAdapter,
}

impl GdbServer {
    pub fn new(adapter: LabwiredAdapter) -> Self {
        Self { adapter }
    }

    pub fn listen(&self, addr: &str) -> Result<()> {
        let listener = TcpListener::bind(addr)?;
        tracing::info!("GDB Server listening on {}", addr);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let adapter = self.adapter.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = handle_client(stream, adapter) {
                            tracing::error!("GDB Client error: {}", e);
                        }
                    });
                }
                Err(e) => tracing::error!("GDB connection failed: {}", e),
            }
        }
        Ok(())
    }
}

fn handle_client(mut stream: TcpStream, adapter: LabwiredAdapter) -> Result<()> {
    tracing::info!("GDB client connected");
    let mut buffer = [0u8; 4096];

    loop {
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            break;
        }

        let data = &buffer[..n];
        for (i, &byte) in data.iter().enumerate() {
            if byte == b'$' {
                // Find end of packet
                if let Some(end) = data[i..].iter().position(|&b| b == b'#') {
                    let packet_start = i + 1;
                    let packet_end = i + end;
                    let packet = String::from_utf8_lossy(&data[packet_start..packet_end]);

                    // Acknowledge
                    stream.write_all(b"+")?;

                    let response = handle_packet(&packet, &adapter)?;
                    send_packet(&mut stream, &response)?;
                }
            } else if byte == 0x03 {
                // Ctrl-C / Pause
                let _ = adapter.step(); // Force a stop or just return status
                send_packet(&mut stream, "S05")?; // Stop reason TRAP
            }
        }
    }
    tracing::info!("GDB client disconnected");
    Ok(())
}

fn handle_packet(packet: &str, adapter: &LabwiredAdapter) -> Result<String> {
    if packet == "?" {
        return Ok("S05".to_string()); // Stopped with SIGTRAP
    }

    if packet.starts_with("qSupported") {
        return Ok("PacketSize=1000;hwbreak+;swbreak+;vCont+;qXfer:features:read+".to_string());
    }

    if packet.starts_with("qfThreadInfo") {
        return Ok("m1".to_string()); // One thread (Core 0)
    }

    if packet.starts_with("qsThreadInfo") {
        return Ok("l".to_string()); // End of list
    }

    if packet.starts_with("H") {
        return Ok("OK".to_string()); // Accept all thread selections
    }

    if packet == "g" {
        // Read all registers
        let mut regs = String::new();
        if adapter.get_register_names().is_ok() {
            // ARM GDB expects 16 registers + PSR (optional)
            for i in 0..16 {
                let val = adapter.get_register(i as u8).unwrap_or(0);
                // GDB expects LITTLE ENDIAN hex strings
                let bytes = val.to_le_bytes();
                for b in bytes {
                    regs.push_str(&format!("{:02x}", b));
                }
            }
        }
        return Ok(regs);
    }

    if let Some(stripped) = packet.strip_prefix('m') {
        // Read memory: m addr,len
        let parts: Vec<&str> = stripped.split(',').collect();
        if parts.len() == 2 {
            let addr = u64::from_str_radix(parts[0], 16).unwrap_or(0);
            let len = usize::from_str_radix(parts[1], 16).unwrap_or(0);
            if let Ok(data) = adapter.read_memory(addr, len) {
                return Ok(data
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>());
            }
        }
        return Ok("E01".to_string());
    }

    if let Some(stripped) = packet.strip_prefix('M') {
        // Write memory (hex): M addr,len:data
        let parts: Vec<&str> = stripped.split(':').collect();
        if parts.len() == 2 {
            let addr_len: Vec<&str> = parts[0].split(',').collect();
            if addr_len.len() == 2 {
                let addr = u64::from_str_radix(addr_len[0], 16).unwrap_or(0);
                let _len = usize::from_str_radix(addr_len[1], 16).unwrap_or(0);
                let data_hex = parts[1];
                let mut data = Vec::new();
                for i in 0..(data_hex.len() / 2) {
                    if let Ok(b) = u8::from_str_radix(&data_hex[i * 2..i * 2 + 2], 16) {
                        data.push(b);
                    }
                }
                if adapter.write_memory(addr, &data).is_ok() {
                    return Ok("OK".to_string());
                }
            }
        }
        return Ok("E01".to_string());
    }

    if packet == "c" || packet.starts_with("vCont;c") {
        // Continue
        let _ = adapter.continue_execution();
        return Ok("S05".to_string());
    }

    if packet == "s" || packet.starts_with("vCont;s") {
        // Step
        let _ = adapter.step();
        return Ok("S05".to_string());
    }

    if let Some(stripped) = packet.strip_prefix('P') {
        // Write single register: P n=val
        let parts: Vec<&str> = stripped.split('=').collect();
        if parts.len() == 2 {
            let n = u8::from_str_radix(parts[0], 16).unwrap_or(0);
            let val_hex = parts[1];
            if val_hex.len() == 8 {
                let mut bytes = [0u8; 4];
                for i in 0..4 {
                    bytes[i] = u8::from_str_radix(&val_hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
                }
                let val = u32::from_le_bytes(bytes);
                let _ = adapter.set_register(n, val);
                return Ok("OK".to_string());
            }
        }
        return Ok("E01".to_string());
    }

    if packet.starts_with("Z0") {
        // Insert software breakpoint: Z0,addr,kind
        let parts: Vec<&str> = packet[3..].split(',').collect();
        if !parts.is_empty() {
            let addr = u32::from_str_radix(parts[0], 16).unwrap_or(0);
            let _ = adapter.add_breakpoint_addr(addr);
        }
        return Ok("OK".to_string());
    }

    if packet.starts_with("z0") {
        // Remove software breakpoint: z0,addr,kind
        let parts: Vec<&str> = packet[3..].split(',').collect();
        if !parts.is_empty() {
            let addr = u32::from_str_radix(parts[0], 16).unwrap_or(0);
            let _ = adapter.remove_breakpoint_addr(addr);
        }
        return Ok("OK".to_string());
    }

    Ok("".to_string()) // Not supported
}

fn send_packet(stream: &mut TcpStream, packet: &str) -> Result<()> {
    let checksum = packet
        .as_bytes()
        .iter()
        .fold(0u8, |acc, &b| acc.wrapping_add(b));
    let formatted = format!("${}#{:02x}", packet, checksum);
    stream.write_all(formatted.as_bytes())?;
    Ok(())
}
