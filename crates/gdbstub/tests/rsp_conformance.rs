// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Six GDB RSP conformance scenarios pinned to the watchpoint/stop contracts
//! Tasks 1-3 shipped. Each test drives a real `GdbServer` over TCP; the
//! observed wire replies are asserted verbatim where the plan pinned them and
//! annotated where the wire differed from the plan's expectation.

use labwired_core::bus::SystemBus;
use labwired_core::Machine;
use labwired_gdbstub::GdbServer;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

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

/// The checksum-validated end of the first complete `$…#chk` envelope in
/// `data`, if one is fully present. GDB RLE can emit `#` inside a body, so a
/// candidate split only counts when its trailing checksum matches.
fn first_envelope(data: &[u8]) -> Option<(usize, usize)> {
    for start in 0..data.len() {
        if data[start] != b'$' {
            continue;
        }
        for hash in start + 1..data.len() {
            if data[hash] != b'#' {
                continue;
            }
            if hash + 3 > data.len() {
                break;
            }
            let body = &data[start + 1..hash];
            let Ok(cks) = std::str::from_utf8(&data[hash + 1..hash + 3]) else {
                continue;
            };
            let sum = body.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
            if u8::from_str_radix(cks, 16).ok() == Some(sum) {
                return Some((start, hash + 3));
            }
        }
    }
    None
}

/// Read exactly one reply envelope. 2 s deadline; `+` acks are skipped.
/// Panics if a second envelope rides in the same read — one command must
/// produce exactly one reply.
fn read_packet(stream: &mut TcpStream) -> String {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some((start, end)) = first_envelope(&buf) {
            if first_envelope(&buf[end..]).is_some() {
                panic!(
                    "one command produced two envelopes: {:?}",
                    String::from_utf8_lossy(&buf)
                );
            }
            return String::from_utf8_lossy(&buf[start..end]).into_owned();
        }
        if Instant::now() > deadline {
            panic!(
                "timed out reading a GDB reply; data so far: {:?}",
                String::from_utf8_lossy(&buf)
            );
        }
        match stream.read(&mut tmp) {
            Ok(0) => thread::sleep(Duration::from_millis(10)),
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(e) => panic!("error reading a GDB reply: {e:?}"),
        }
    }
}

fn expand_rle(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i + 1] == '*' && i + 2 < chars.len() {
            let repeat = chars[i + 2] as u32 - 29 + 1;
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

/// The body between `$` and `#` of an envelope, byte-exact.
///
/// No trimming: RLE's count character is `0x1C + repeat`, so a repeat of four
/// is `0x20` — a literal trailing space (`00040* ` for `00040000`). Trimming
/// the body would silently corrupt exactly that run.
fn body(reply: &str) -> String {
    let start = reply.find('$').map(|i| i + 1).unwrap_or(0);
    let end = reply[start..]
        .find('#')
        .map(|i| start + i)
        .unwrap_or(reply.len());
    reply[start..end].to_string()
}

/// The two hex digits after the leading `S`/`T` of a stop reply.
fn stop_signal(reply: &str) -> Option<String> {
    let body = body(reply);
    let mut chars = body.chars();
    match chars.next() {
        Some('S') | Some('T') => Some(chars.take(2).collect()),
        _ => None,
    }
}

fn firmware_path() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/uart-ok-thumbv7m.elf")
}

/// Bind an ephemeral port and serve ONE session with the standard fixture.
fn start_session() -> (u16, thread::JoinHandle<anyhow::Result<()>>) {
    start_session_with_faults(true)
}

/// Same session, with the ARMv7-M fault model explicitly toggled through the
/// public per-core override (`CortexM::set_faults_enabled`) before the machine
/// is handed to the server. The alternative — `LABWIRED_CORTEXM_FAULTS` — is a
/// process-global read once at `configure_cortex_m` time, which would race
/// other tests running in parallel; the per-core switch has no such problem.
fn start_session_with_faults(
    faults_enabled: bool,
) -> (u16, thread::JoinHandle<anyhow::Result<()>>) {
    let bound = GdbServer::bind(0).expect("bind an ephemeral port");
    let port = bound.local_addr().port();
    let path = firmware_path();
    let handle = thread::spawn(move || {
        let mut bus = SystemBus::new();
        let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        machine.cpu.set_faults_enabled(faults_enabled);
        let image = labwired_loader::load_elf(&path)?;
        machine.load_firmware(&image)?;
        bound.run(machine)
    });
    (port, handle)
}

fn connect(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match TcpStream::connect(format!("127.0.0.1:{port}")) {
            Ok(s) => break s,
            Err(e) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
                let _ = e;
            }
            Err(e) => panic!("could not reach the GDB server on port {port}: {e}"),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    stream.write_all(b"+").unwrap();
    stream.flush().unwrap();
    stream
}

/// Write code bytes with `M` packets, 64 bytes per packet.
fn write_thumb_code(stream: &mut TcpStream, addr: u32, bytes: &[u8]) {
    for (i, chunk) in bytes.chunks(64).enumerate() {
        write_mem(stream, addr + (i as u32) * 64, chunk);
    }
}

/// One `M` packet; asserts the `OK` ack.
fn write_mem(stream: &mut TcpStream, addr: u32, data: &[u8]) {
    let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
    send_packet(stream, &format!("M{addr:x},{:x}:{hex}", data.len()));
    let reply = read_packet(stream);
    assert_eq!(body(&reply), "OK", "M at {addr:#x} was refused: {reply:?}");
}

/// `m` read; expands the crate's RLE before hex-decoding.
fn read_mem(stream: &mut TcpStream, addr: u32, len: usize) -> Vec<u8> {
    send_packet(stream, &format!("m{addr:x},{len:x}"));
    let reply = read_packet(stream);
    let expanded = expand_rle(&body(&reply));
    assert!(
        expanded.len() == len * 2,
        "m {addr:#x},{len} answered {:?} ({expanded:?})",
        body(&reply)
    );
    expanded
        .as_bytes()
        .chunks(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}

/// All registers via `g`, RLE-expanded (336 hex chars: 17 ARM core regs then
/// the arch stub's 100 bytes of `xx` FPU placeholders).
fn read_all_regs(stream: &mut TcpStream) -> String {
    send_packet(stream, "g");
    let reply = read_packet(stream);
    let regs = expand_rle(&body(&reply));
    assert!(!body(&reply).starts_with('E'), "g was refused: {reply:?}");
    assert!(regs.len() >= 16 * 8, "short g reply: {} chars", regs.len());
    regs
}

/// Write one core register with a real `P` packet. The wire format is
/// `P reg=val` (the `=` is required by both the RSP spec and the crate's
/// parser) and `val` is the value in target byte order — the little-endian
/// byte sequence on both supported cores. The reply must be `OK`: before the
/// target implemented `SingleRegisterAccess`, gdbstub answered the empty
/// envelope `$#00` and silently changed nothing.
fn set_reg_via_p(stream: &mut TcpStream, reg_hex: &str, val: u32) {
    let le: String = val
        .to_le_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    send_packet(stream, &format!("P{reg_hex}={le}"));
    let reply = read_packet(stream);
    assert_eq!(
        body(&reply),
        "OK",
        "P write of register {reg_hex} was refused: {reply:?}"
    );
}

/// `p n` single-register read; expands the crate's RLE before decoding the
/// little-endian value (gdbstub RLE-compresses these replies too).
fn read_reg_via_p(stream: &mut TcpStream, reg_hex: &str) -> u32 {
    send_packet(stream, &format!("p{reg_hex}"));
    let reply = read_packet(stream);
    let hex = expand_rle(&body(&reply));
    assert_eq!(hex.len(), 8, "p {reg_hex} answered {reply:?}");
    let bytes: [u8; 4] = std::array::from_fn(|i| {
        u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("p reply is hex")
    });
    u32::from_le_bytes(bytes)
}

/// Nonblocking drain: panics if another complete `$…#chk` envelope arrives
/// within `within_ms` without a new continue. Catches the doubled stop
/// replies of #978 that a same-read would have hidden.
fn assert_no_second_stop(stream: &mut TcpStream, within_ms: u64) {
    stream.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_millis(within_ms);
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("read error while checking for a second stop: {e:?}"),
        }
        if let Some((start, _)) = first_envelope(&buf) {
            panic!(
                "a second packet arrived without a new continue: {:?}",
                String::from_utf8_lossy(&buf[start..])
            );
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    stream.set_nonblocking(false).unwrap();
}

/// Wait up to 5 s for the server thread and assert the session ended cleanly.
fn join_session(handle: thread::JoinHandle<anyhow::Result<()>>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.is_finished() {
        assert!(
            Instant::now() < deadline,
            "GDB server did not exit within 5 s"
        );
        thread::sleep(Duration::from_millis(10));
    }
    handle
        .join()
        .expect("GDB server thread panicked")
        .expect("GDB server session failed");
}

/// `k`, drop, join within 5 s.
fn shutdown(mut stream: TcpStream, handle: thread::JoinHandle<anyhow::Result<()>>) {
    send_packet(&mut stream, "k");
    drop(stream);
    join_session(handle);
}

const CODE: u32 = 0x2000_0800;

/// Renode #51: with the core parked in a WFI/B-loop, the host resets PC while
/// stopped and continues; the target must keep answering Ctrl-C and step.
#[test]
fn reset_pc_while_stopped_in_wfi_stays_responsive() {
    let (port, handle) = start_session();
    let mut stream = connect(port);

    // wfi at 0x800, then b . (fe e7) at 0x802.
    write_thumb_code(&mut stream, CODE, &[0x30, 0xbf, 0xfe, 0xe7]);

    // Regression: `P` used to be a silent no-op — the target did not
    // implement `SingleRegisterAccess`, so the crate answered the empty
    // envelope `$#00` and left every register where it was. This scenario's
    // reset-PC move could therefore "pass" while changing nothing. The two
    // reads below are what make the move observable: `p f` (single-register
    // read) and then `g` (bulk), so the #51 point is pinned end to end.
    assert_ne!(
        read_reg_via_p(&mut stream, "f"),
        CODE,
        "precondition: the PC does not already sit at the WFI site"
    );
    set_reg_via_p(&mut stream, "f", CODE);
    assert_eq!(
        read_reg_via_p(&mut stream, "f"),
        CODE,
        "P f must move the PC"
    );
    let regs = read_all_regs(&mut stream);
    assert_eq!(
        &regs[15 * 8..15 * 8 + 8],
        "00080020",
        "g must show the P-written PC"
    );

    send_packet(&mut stream, "c");
    thread::sleep(Duration::from_millis(200));
    stream.write_all(&[0x03]).unwrap();
    stream.flush().unwrap();
    let stop = read_packet(&mut stream);
    assert_eq!(stop_signal(&stop).as_deref(), Some("02"), "{stop:?}");
    assert_no_second_stop(&mut stream, 200);

    // Reset PC to the `b .` site while the target is stopped. The plan said
    // 0x804, but `fe e7` is the 2-byte branch directly after the 2-byte `wfi`,
    // so the branch site is 0x802 — the scenario's stated intent.
    set_reg_via_p(&mut stream, "f", CODE + 2);
    assert_eq!(
        read_reg_via_p(&mut stream, "f"),
        CODE + 2,
        "the second P f must move the PC too"
    );
    send_packet(&mut stream, "c");
    thread::sleep(Duration::from_millis(200));
    stream.write_all(&[0x03]).unwrap();
    stream.flush().unwrap();
    let stop = read_packet(&mut stream);
    assert_eq!(stop_signal(&stop).as_deref(), Some("02"), "{stop:?}");
    assert_no_second_stop(&mut stream, 200);

    send_packet(&mut stream, "s");
    let stop = read_packet(&mut stream);
    assert_eq!(stop_signal(&stop).as_deref(), Some("05"), "{stop:?}");

    shutdown(stream, handle);
}

/// Renode #52: a continue on the freshly attached target must run and be
/// interruptible even though no stop reason was ever reported first.
#[test]
fn continue_while_halted_without_prior_start() {
    let (port, handle) = start_session();
    let mut stream = connect(port);

    send_packet(&mut stream, "c");
    thread::sleep(Duration::from_millis(200));
    stream.write_all(&[0x03]).unwrap();
    stream.flush().unwrap();
    let stop = read_packet(&mut stream);
    assert_eq!(stop_signal(&stop).as_deref(), Some("02"), "{stop:?}");
    assert_no_second_stop(&mut stream, 200);

    // The plan expected the latched SIGINT back from `?`. The pinned
    // gdbstub 0.7.10 answers its canned stop reply instead (OBSERVED):
    // `?` always reports T05, never the latched signal. The session is alive
    // and the reply is a well-formed T packet.
    send_packet(&mut stream, "?");
    let reply = read_packet(&mut stream);
    assert_eq!(reply, "$T05thread:01;#07");
    assert_eq!(stop_signal(&reply).as_deref(), Some("05"));

    shutdown(stream, handle);
}

/// Renode #74: stepping into a faulting access reports the fault and leaves
/// the target alive and steppable.
#[test]
fn step_into_fault_reports_fault_and_target_survives() {
    let (port, handle) = start_session();
    let mut stream = connect(port);

    // ldr r0, [r1] over an unmapped address.
    write_thumb_code(&mut stream, CODE, &[0x08, 0x68]);
    set_reg_via_p(&mut stream, "1", 0x3000_0004);
    set_reg_via_p(&mut stream, "f", CODE);

    send_packet(&mut stream, "s");
    let stop = read_packet(&mut stream);
    // OBSERVED: S05, not the plan's S0b. The ARMv7-M fault model is on by
    // default, so the unmapped load pends a precise BusFault escalated to
    // HardFault and the step finishes as StepDone/SIGTRAP. The Err arm's
    // S0b is reachable with fault escalation off and is pinned by
    // `step_into_fault_with_faults_disabled_reports_segv` below.
    assert_eq!(stop_signal(&stop).as_deref(), Some("05"), "{stop:?}");
    assert_no_second_stop(&mut stream, 200);

    let regs = read_all_regs(&mut stream);
    assert_eq!(
        &regs[15 * 8..15 * 8 + 8],
        "00080020",
        "the faulting ldr must not retire; PC is still the ldr site"
    );

    // The pended escalated fault is dispatched by the NEXT step (OBSERVED):
    // PC enters the fixture's HardFault handler, vector[3] = 0x463.
    send_packet(&mut stream, "s");
    let stop = read_packet(&mut stream);
    assert_eq!(stop_signal(&stop).as_deref(), Some("05"), "{stop:?}");
    let regs = read_all_regs(&mut stream);
    assert_eq!(
        &regs[15 * 8..15 * 8 + 8],
        "62040000",
        "exception entry lands in the HardFault handler at 0x462"
    );

    // Recovery: a clean `b .` site steps to SIGTRAP and the target survives.
    write_thumb_code(&mut stream, CODE + 0x10, &[0x00, 0xbf, 0xfe, 0xe7]);
    set_reg_via_p(&mut stream, "f", CODE + 0x10);
    send_packet(&mut stream, "s");
    let stop = read_packet(&mut stream);
    assert_eq!(stop_signal(&stop).as_deref(), Some("05"), "{stop:?}");
    let regs = read_all_regs(&mut stream);
    assert_eq!(
        &regs[15 * 8..15 * 8 + 8],
        "12080020",
        "the clean nop retired (PC = CODE + 0x12)"
    );

    shutdown(stream, handle);
}

/// The other half of #74: with fault escalation turned off, the unmapped load
/// is not converted into a pended HardFault — the machine's `Err` lands in the
/// event loop's `Err` arm, which must answer `S0b` (SIGSEGV). The switch is the
/// public per-core override, so no process-global env is touched.
#[test]
fn step_into_fault_with_faults_disabled_reports_segv() {
    let (port, handle) = start_session_with_faults(false);
    let mut stream = connect(port);

    // ldr r0, [r1] over an unmapped address.
    write_thumb_code(&mut stream, CODE, &[0x08, 0x68]);
    set_reg_via_p(&mut stream, "1", 0x3000_0004);
    set_reg_via_p(&mut stream, "f", CODE);

    send_packet(&mut stream, "s");
    let stop = read_packet(&mut stream);
    assert_eq!(
        stop_signal(&stop).as_deref(),
        Some("0b"),
        "the Err arm must report SIGSEGV when escalation is off: {stop:?}"
    );
    assert_no_second_stop(&mut stream, 200);

    let regs = read_all_regs(&mut stream);
    assert_eq!(
        &regs[15 * 8..15 * 8 + 8],
        "00080020",
        "the faulting ldr must not retire; PC is still the ldr site"
    );

    shutdown(stream, handle);
}

/// Renode #70: a watchpoint on a 2 KiB object stops the core after the store
/// retires (PC = store + 2), the store is visible, and oversized requests are
/// refused without disarming the target.
#[test]
fn watchpoint_on_a_2kib_object_stops_the_core() {
    let (port, handle) = start_session();
    let mut stream = connect(port);

    // str r0, [r1] with r1 = the last word of the 2 KiB object at 0x2000_0000.
    write_thumb_code(&mut stream, CODE, &[0x08, 0x60]);
    set_reg_via_p(&mut stream, "0", 0xAB);
    set_reg_via_p(&mut stream, "1", 0x2000_07FC);
    set_reg_via_p(&mut stream, "f", CODE);

    send_packet(&mut stream, "Z2,20000000,800");
    assert_eq!(body(&read_packet(&mut stream)), "OK");

    send_packet(&mut stream, "c");
    let stop = read_packet(&mut stream);
    assert_eq!(stop_signal(&stop).as_deref(), Some("05"), "{stop:?}");
    assert_no_second_stop(&mut stream, 200);

    // Pinned post-retirement contract: the store retired, nothing else did.
    let regs = read_all_regs(&mut stream);
    assert_eq!(&regs[15 * 8..15 * 8 + 8], "02080020", "PC must be CODE + 2");
    assert_eq!(read_mem(&mut stream, 0x2000_07FC, 4), vec![0xAB, 0, 0, 0]);

    send_packet(&mut stream, "z2,20000000,800");
    assert_eq!(body(&read_packet(&mut stream)), "OK");

    // 4097 bytes is over the 4 KiB cap → Machine::add_watchpoint → Ok(false),
    // which gdbstub 0.7.10 renders as NonFatalError(22): `E16` (OBSERVED).
    send_packet(&mut stream, "Z2,20000000,1001");
    let refusal = read_packet(&mut stream);
    assert_eq!(refusal, "$E16#ac");
    assert_ne!(body(&refusal), "OK");

    // The refused watch did not arm: continue and interrupt normally. Park
    // the PC in a controlled `b .` spin first — the previous continuation
    // started at CODE + 2 in zero-filled RAM (`movs r0, r0` sled) and, given
    // the 200 ms head start, could reach the end of RAM and take a
    // BusFault → HardFault (S0b) before the 0x03 landed. CI caught that race;
    // with the spin the only possible stops are ones this test controls.
    write_thumb_code(&mut stream, CODE + 0x100, &[0xfe, 0xe7]);
    set_reg_via_p(&mut stream, "f", CODE + 0x100);
    assert_eq!(
        read_reg_via_p(&mut stream, "f"),
        CODE + 0x100,
        "P f must move the PC to the spin"
    );
    send_packet(&mut stream, "c");
    thread::sleep(Duration::from_millis(200));
    stream.write_all(&[0x03]).unwrap();
    stream.flush().unwrap();
    let stop = read_packet(&mut stream);
    assert_eq!(stop_signal(&stop).as_deref(), Some("02"), "{stop:?}");
    assert_eq!(
        read_reg_via_p(&mut stream, "f"),
        CODE + 0x100,
        "the interrupt must land while the core is still in the spin"
    );
    assert_no_second_stop(&mut stream, 200);

    shutdown(stream, handle);
}

/// Renode #978: one resume must produce exactly one stop reply — no doubled
/// stop envelopes riding behind the first.
#[test]
fn exactly_one_stop_reply_per_stop() {
    let (port, handle) = start_session();
    let mut stream = connect(port);

    // Two stores into the watched range, then a self-loop, so each continue
    // has a fresh watched access to stop on.
    write_thumb_code(&mut stream, CODE, &[0x08, 0x60, 0x08, 0x60, 0xfe, 0xe7]);
    set_reg_via_p(&mut stream, "0", 0xAB);
    set_reg_via_p(&mut stream, "1", 0x2000_07FC);
    set_reg_via_p(&mut stream, "f", CODE);

    send_packet(&mut stream, "Z2,200007FC,4");
    assert_eq!(body(&read_packet(&mut stream)), "OK");

    send_packet(&mut stream, "c");
    let stop = read_packet(&mut stream);
    assert_eq!(stop_signal(&stop).as_deref(), Some("05"), "{stop:?}");
    assert_no_second_stop(&mut stream, 250);
    let regs = read_all_regs(&mut stream);
    assert_eq!(&regs[15 * 8..15 * 8 + 8], "02080020");

    send_packet(&mut stream, "c");
    let stop = read_packet(&mut stream);
    assert_eq!(stop_signal(&stop).as_deref(), Some("05"), "{stop:?}");
    assert_no_second_stop(&mut stream, 250);
    let regs = read_all_regs(&mut stream);
    assert_eq!(&regs[15 * 8..15 * 8 + 8], "04080020");

    shutdown(stream, handle);
}

/// Renode #57: attach/detach under breakpoint traffic. Loose-bound stress,
/// not a reproduction claim: 20 fresh sessions, 10 closed with `k` and 10 by
/// EOF; every server thread must exit within the 5 s budget.
#[test]
fn attach_detach_stress_under_breakpoint_traffic() {
    for iteration in 0..20 {
        let (port, handle) = start_session();
        let mut stream = connect(port);

        send_packet(&mut stream, "Z0,400,2");
        assert_eq!(
            body(&read_packet(&mut stream)),
            "OK",
            "iteration {iteration}: breakpoint at the ELF entry refused"
        );

        send_packet(&mut stream, "c");
        let stop = read_packet(&mut stream);
        assert_eq!(
            stop_signal(&stop).as_deref(),
            Some("05"),
            "iteration {iteration}: {stop:?}"
        );

        let regs = read_all_regs(&mut stream);
        assert_eq!(
            &regs[15 * 8..15 * 8 + 8],
            "00040000",
            "iteration {iteration}: PC is the ELF entry 0x400"
        );

        if iteration < 10 {
            shutdown(stream, handle);
        } else {
            // EOF path: no `k`; the server must notice the closed socket.
            drop(stream);
            join_session(handle);
        }
    }
}

/// Optional client smoke: run `scripts/gdb-conformance.sh`, which launches the
/// `labwired` server (`cargo run -q -p labwired-cli -- --gdb ...`) and drives
/// one batch `gdb-multiarch` (or `arm-none-eabi-gdb`) session against it.
/// Ignored by default so a box without either tool skips instead of failing;
/// run with
/// `cargo test -p labwired-gdbstub --test rsp_conformance -- --ignored`.
///
/// The script prefers a prebuilt `target/debug/labwired`: `cargo test`
/// holds the target-dir build lock for the whole run, so the script's `cargo
/// run` fallback would block forever under it. Without that binary the test
/// skips with a note — build it with `cargo build -p labwired-cli` first, or
/// run the script standalone where the fallback is safe.
#[test]
#[ignore = "requires gdb-multiarch; run scripts/gdb-conformance.sh"]
fn gdb_multiarch_batch_smoke() {
    fn tool_absent(tool: &str) -> bool {
        std::process::Command::new(tool)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_err()
    }

    if tool_absent("gdb-multiarch") && tool_absent("arm-none-eabi-gdb") {
        println!("skipping gdb_multiarch_batch_smoke: neither gdb-multiarch nor arm-none-eabi-gdb is installed");
        return;
    }

    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let cli_present = match std::env::var("LABWIRED_CLI_BIN") {
        Ok(path) => std::path::Path::new(&path).is_file(),
        Err(_) => {
            let target_dir = std::env::var("CARGO_TARGET_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| repo_root.join("target"));
            let target_dir = if target_dir.is_absolute() {
                target_dir
            } else {
                repo_root.join(target_dir)
            };
            target_dir.join("debug/labwired").is_file()
        }
    };
    if !cli_present {
        println!(
            "skipping gdb_multiarch_batch_smoke: no prebuilt target/debug/labwired \
             (cargo test holds the target-dir lock); run `cargo build -p labwired-cli` \
             and rerun, or run scripts/gdb-conformance.sh standalone"
        );
        return;
    }

    let output = std::process::Command::new("bash")
        .arg(repo_root.join("scripts/gdb-conformance.sh"))
        .current_dir(&repo_root)
        .output()
        .expect("spawn scripts/gdb-conformance.sh");
    assert!(
        output.status.success(),
        "scripts/gdb-conformance.sh failed with {}:\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
