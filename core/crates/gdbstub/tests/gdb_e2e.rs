// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;
use labwired_core::{Machine, DebugControl};
use labwired_core::bus::SystemBus;
use labwired_gdbstub::{GdbServer, LabwiredTarget};

fn compute_checksum(data: &str) -> String {
    let sum: u8 = data.as_bytes().iter().fold(0, |acc, &x| acc.wrapping_add(x));
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
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let firmware_path = workspace_root.join("tests/fixtures/uart-ok-thumbv7m.elf");
    assert!(firmware_path.exists(), "Firmware fixture not found at {:?}", firmware_path);

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
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();

    // 3. Send ACK for initial connect
    stream.write_all(b"+").unwrap();

    // 4. Test Read R0 ($p0)
    send_packet(&mut stream, "p0");
    let resp = read_packet(&mut stream);
    assert!(!resp.contains("E"), "Failed to read R0. Got: {}", resp);

    // 5. Test Single Step ($s)
    send_packet(&mut stream, "s");
    let resp = read_packet(&mut stream);
    assert!(resp.contains("05"), "GDB did not return stop reply (SIGTRAP) after step. Got: {}", resp);

    // 6. Verify PC Changed
    send_packet(&mut stream, "p0f");
    let resp = read_packet(&mut stream);
    assert!(!resp.contains("E"), "Failed to read PC. Got: {}", resp);
    let pc_after_step = resp.trim_start_matches('+').trim_start_matches('$').split('#').next().unwrap();
    // We don't compare values yet, just that we got a valid response.

    // 7. Test Interrupt (Pause)
    // Send we continue, then interrupt
    send_packet(&mut stream, "c");
    thread::sleep(Duration::from_millis(100));
    stream.write_all(&[0x03]).unwrap();
    stream.flush().unwrap();
    let resp = read_packet(&mut stream);
    // SIGINT is 02. gdbstub returns stop reply for SIGINT.
    assert!(resp.contains("02") || resp.contains("T02") || resp.contains("S02"), "GDB did not return SIGINT stop reply after pause. Got: {}", resp);

    // 8. Test Read Memory ($m)
    send_packet(&mut stream, "m0,4");
    let resp = read_packet(&mut stream);
    assert!(!resp.contains("E01"), "GDB memory read failed");
}
