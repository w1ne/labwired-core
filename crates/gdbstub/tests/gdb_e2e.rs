// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::bus::SystemBus;
use labwired_core::Machine;
use labwired_gdbstub::GdbServer;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

fn compute_checksum(data: &str) -> String {
    let sum: u8 = data
        .as_bytes()
        .iter()
        .fold(0, |acc, &x| acc.wrapping_add(x));
    format!("{:02x}", sum)
}

fn send_packet(stream: &mut TcpStream, data: &str) {
    let packet = format!("${}#{}", data, compute_checksum(data));
    stream.write_all(packet.as_bytes()).unwrap();
    stream.flush().unwrap();
}

fn read_packet(stream: &mut TcpStream) -> String {
    let mut buffer = [0; 2048];
    let mut response = String::new();
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(2) {
            panic!("Timed out reading GDB packet. Data so far: {:?}", response);
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                response.push_str(&String::from_utf8_lossy(&buffer[..n]));
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(e) => panic!("Error reading GDB packet: {:?}", e),
        }

        // If it's just a '+', wait for the next bit
        if response == "+" {
            response.clear();
            continue;
        }

        // If we found a '$' packet and its checksum, we have enough
        if response.contains('$') && response.contains('#') {
            let hash_idx = response.find('#').unwrap();
            if response.len() >= hash_idx + 3 {
                break;
            }
        }
    }
    response
}

#[test]
fn test_gdb_rsp_basic_commands() {
    let _ = tracing_subscriber::fmt::try_init();
    println!("Starting GDB E2E test...");
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let firmware_path = workspace_root.join("tests/fixtures/uart-ok-thumbv7m.elf");
    assert!(
        firmware_path.exists(),
        "Firmware fixture not found at {:?}",
        firmware_path
    );

    // 1. Setup a machine and GDB server in a background thread
    let port = 9001;
    thread::spawn(move || {
        let mut bus = SystemBus::new();
        let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);

        // Load firmware
        let image = labwired_loader::load_elf(&firmware_path).unwrap();
        machine.load_firmware(&image).unwrap();

        let server = GdbServer::new(port);
        server.run(machine).unwrap();
    });

    // Wait for server to start
    thread::sleep(Duration::from_millis(100));

    // 2. Connect client
    println!("Connecting to GDB server on port {}...", port);
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();

    // 3. Send ACK for initial connect
    stream.write_all(b"+").unwrap();

    // 4. Test Read All Registers ($g)
    println!("Sending g...");
    send_packet(&mut stream, "g");
    let resp = read_packet(&mut stream);
    assert!(
        !resp.contains("E"),
        "Failed to read registers. Got: {}",
        resp
    );
    // Register response ($g) might be RLE encoded.
    // e.g. $00000000... or $0*...
    assert!(
        resp.starts_with("$") || resp.starts_with("+$"),
        "Invalid GDB response: {}",
        resp
    );

    // 5. Test Single Step ($s)
    send_packet(&mut stream, "s");
    let step_resp = read_packet(&mut stream);
    assert!(
        step_resp.contains("05"),
        "GDB did not return stop reply (SIGTRAP) after step. Got: {}",
        step_resp
    );

    // 6. Verify PC Changed — read all registers with $g and extract PC (reg 15).
    //    GDB RSP $g response is RLE-encoded: "X*N" means repeat X for (N - 29) times.
    //    Expand it, then extract bytes 15*8..16*8 (PC, little-endian).
    send_packet(&mut stream, "g");
    let resp = read_packet(&mut stream);
    eprintln!("[gdb_e2e] g response after step: {resp:?}");
    assert!(
        !resp.contains('E'),
        "Failed to read registers after step. Got: {resp}"
    );
    let raw = resp
        .trim_start_matches('+')
        .trim_start_matches('$')
        .split('#')
        .next()
        .unwrap_or("")
        .trim();

    // Expand GDB RSP RLE encoding.
    fn expand_rle(s: &str) -> String {
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            if i + 1 < chars.len() && chars[i + 1] == '*' && i + 2 < chars.len() {
                let repeat = chars[i + 2] as u32 - 29 + 1; // total count including the original
                for _ in 0..repeat {
                    out.push(chars[i]);
                }
                i += 3;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        out
    }
    let reg_hex = expand_rle(raw);
    eprintln!(
        "[gdb_e2e] g expanded ({} chars): {}…",
        reg_hex.len(),
        &reg_hex[..reg_hex.len().min(32)]
    );

    // PC is register 15; 16 ARM regs × 8 hex chars = 128 total.
    let pc_after_step: u32 = if reg_hex.len() >= 16 * 8 {
        let pc_le = &reg_hex[15 * 8..15 * 8 + 8];
        u32::from_str_radix(pc_le, 16)
            .ok()
            .map(u32::swap_bytes)
            .unwrap_or(0)
    } else {
        0
    };
    eprintln!(
        "[gdb_e2e] pc_after_step=0x{:08x} (expanded {} chars)",
        pc_after_step,
        reg_hex.len()
    );

    // ELF entry is 0x401 (Thumb). After one step, PC advances by at least 2.
    assert!(
        pc_after_step > 0x400,
        "PC 0x{pc_after_step:08x} must have advanced beyond ELF entry 0x401 after one step"
    );

    // 6b. Read the first 4 bytes from the ELF entry region (0x400) and verify
    //     they are non-zero (real instruction bytes, not blank flash).
    send_packet(&mut stream, "m400,4");
    let mem_resp = read_packet(&mut stream);
    assert!(
        !mem_resp.contains("E01"),
        "Memory read at 0x400 failed: {mem_resp}"
    );
    let mem_hex = mem_resp
        .trim_start_matches('+')
        .trim_start_matches('$')
        .split('#')
        .next()
        .unwrap_or("")
        .trim();
    assert!(
        mem_hex.len() == 8,
        "Expected 4 bytes (8 hex chars) from memory read, got: {mem_hex:?}"
    );
    // The uart-ok firmware places real Thumb-2 instructions at 0x400; they
    // must not be all-zeros (blank flash).
    assert!(
        mem_hex != "00000000",
        "Instruction bytes at 0x400 are all zero — firmware not loaded correctly"
    );
    eprintln!("[gdb_e2e] instruction bytes at 0x400: {mem_hex}");

    // 7. Test Interrupt (Pause)
    // Send we continue, then interrupt
    send_packet(&mut stream, "c");
    thread::sleep(Duration::from_millis(100));
    stream.write_all(&[0x03]).unwrap();
    stream.flush().unwrap();
    let resp = read_packet(&mut stream);
    // SIGINT is 02. gdbstub returns stop reply for SIGINT.
    assert!(
        resp.contains("02") || resp.contains("T02") || resp.contains("S02"),
        "GDB did not return SIGINT stop reply after pause. Got: {}",
        resp
    );

    // 8. Test Read Memory ($m)
    send_packet(&mut stream, "m0,4");
    let resp = read_packet(&mut stream);
    assert!(!resp.contains("E01"), "GDB memory read failed");
}
