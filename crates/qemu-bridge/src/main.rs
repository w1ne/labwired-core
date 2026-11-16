// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Serves one BCM2835 BSC (I²C controller) model to QEMU's
//! `labwired-mmio-bridge` device. A BME280 sits on the bus at 0x76 unless
//! `--no-device` is given. Wire protocol v2: `qemu/PROTOCOL.md`.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::ExitCode;

use labwired_core::peripherals::bcm2835_bsc::{Bcm2835Bsc, DEFAULT_CORE_CLK_HZ};
use labwired_core::peripherals::components::bme280::Bme280;
use labwired_core::peripherals::device::I2cSlaveHost;

pub const PROTOCOL_VERSION: u64 = 2;
const REQ_LEN: usize = 32;
const REPLY_LEN: usize = 24;
const MSG_HELLO: u32 = 0;
const MSG_READ: u32 = 1;
const MSG_WRITE: u32 = 2;
const MSG_SYNC: u32 = 3;
pub const NO_DEADLINE: u64 = u64::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Request {
    kind: u32,
    size: u32,
    t_ns: u64,
    offset: u64,
    value: u64,
}

impl Request {
    #[cfg(test)]
    fn encode(&self) -> [u8; REQ_LEN] {
        let mut b = [0u8; REQ_LEN];
        b[0..4].copy_from_slice(&self.kind.to_le_bytes());
        b[4..8].copy_from_slice(&self.size.to_le_bytes());
        b[8..16].copy_from_slice(&self.t_ns.to_le_bytes());
        b[16..24].copy_from_slice(&self.offset.to_le_bytes());
        b[24..32].copy_from_slice(&self.value.to_le_bytes());
        b
    }

    fn decode(b: &[u8; REQ_LEN]) -> Self {
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        Self {
            kind: u32_at(0),
            size: u32_at(4),
            t_ns: u64_at(8),
            offset: u64_at(16),
            value: u64_at(24),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reply {
    value: u64,
    irq_level: u32,
    next_deadline_ns: u64,
}

impl Reply {
    fn encode(&self) -> [u8; REPLY_LEN] {
        let mut b = [0u8; REPLY_LEN];
        b[0..8].copy_from_slice(&self.value.to_le_bytes());
        b[8..12].copy_from_slice(&self.irq_level.to_le_bytes());
        // 12..16 reserved, zero
        b[16..24].copy_from_slice(&self.next_deadline_ns.to_le_bytes());
        b
    }
}

struct Args {
    socket: PathBuf,
    no_device: bool,
    trace: bool,
    transcript: Option<PathBuf>,
    core_clk_hz: u64,
}

const USAGE: &str = "usage: qemu-bridge --socket PATH [--no-device] [--trace] \
[--transcript FILE] [--core-clk-hz HZ (default 350000000)]";

fn parse_args() -> Result<Args, String> {
    let mut socket = None;
    let mut no_device = false;
    let mut trace = false;
    let mut transcript = None;
    let mut core_clk_hz = DEFAULT_CORE_CLK_HZ;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--socket" => socket = it.next().map(PathBuf::from),
            "--no-device" => no_device = true,
            "--trace" => trace = true,
            "--transcript" => transcript = it.next().map(PathBuf::from),
            "--core-clk-hz" => {
                core_clk_hz = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|&v: &u64| v > 0)
                    .ok_or("--core-clk-hz needs a positive integer")?
            }
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument: {other}\n{USAGE}")),
        }
    }
    Ok(Args {
        socket: socket.ok_or(USAGE)?,
        no_device,
        trace,
        transcript,
        core_clk_hz,
    })
}

/// One BSC behind the bridge. Sub-word accesses act on the containing
/// 32-bit register.
struct Bridge {
    bsc: Bcm2835Bsc,
    irq: bool,
    irq_raises: u64,
    trace: bool,
    transcript: Option<Box<dyn Write>>,
}

impl Bridge {
    fn new(bsc: Bcm2835Bsc) -> Self {
        Self {
            bsc,
            irq: false,
            irq_raises: 0,
            trace: false,
            transcript: None,
        }
    }

    fn handle(&mut self, req: Request) -> Result<Reply, String> {
        if req.kind != MSG_HELLO && req.t_ns < self.bsc.now_ns() {
            return Err(format!(
                "time went backwards: {} < {}",
                req.t_ns,
                self.bsc.now_ns()
            ));
        }
        let reg = req.offset & !3;
        let shift = (req.offset & 3) * 8;
        let (kind, value) = match req.kind {
            MSG_HELLO => {
                if req.value != PROTOCOL_VERSION {
                    return Err(format!(
                        "QEMU speaks protocol v{}, this bridge v{PROTOCOL_VERSION}",
                        req.value
                    ));
                }
                ("HELLO", PROTOCOL_VERSION)
            }
            MSG_READ => {
                let v = (self.bsc.read_reg(req.t_ns, reg) as u64) >> shift;
                let v = if req.size >= 4 {
                    v
                } else {
                    v & ((1u64 << (req.size * 8)) - 1)
                };
                ("READ", v)
            }
            MSG_WRITE => {
                self.bsc
                    .write_reg(req.t_ns, reg, (req.value << shift) as u32);
                ("WRITE", req.value)
            }
            MSG_SYNC => {
                self.bsc.advance_to(req.t_ns);
                ("SYNC", 0)
            }
            k => return Err(format!("unexpected message type {k}")),
        };
        let irq = self.bsc.irq_pending();
        if irq && !self.irq {
            self.irq_raises += 1;
        }
        self.irq = irq;
        let deadline = self.bsc.next_deadline().unwrap_or(NO_DEADLINE);
        let reply = Reply {
            value: if req.kind == MSG_WRITE { 0 } else { value },
            irq_level: irq as u32,
            next_deadline_ns: deadline,
        };
        let dl = if deadline == NO_DEADLINE {
            "none".to_string()
        } else {
            deadline.to_string()
        };
        let line = format!(
            "{} {} {:#04x} {:#010x} {} {}",
            req.t_ns, kind, req.offset, value, irq as u8, dl
        );
        if self.trace {
            eprintln!("[qemu-bridge] {line}");
        }
        if let Some(w) = self.transcript.as_mut() {
            writeln!(w, "{line}").map_err(|e| format!("transcript: {e}"))?;
        }
        Ok(reply)
    }
}

fn serve(mut stream: UnixStream, bridge: &mut Bridge) -> Result<u64, String> {
    let mut buf = [0u8; REQ_LEN];
    let mut messages: u64 = 0;
    loop {
        match stream.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(messages),
            Err(e) => return Err(format!("read: {e}")),
        }
        messages += 1;
        let reply = bridge.handle(Request::decode(&buf))?;
        stream
            .write_all(&reply.encode())
            .map_err(|e| format!("write: {e}"))?;
    }
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let mut bsc = Bcm2835Bsc::new(args.core_clk_hz);
    if args.no_device {
        eprintln!("[qemu-bridge] BSC with no I2C target attached (--no-device)");
    } else {
        bsc.attach_slave(Box::new(Bme280::new(0x76)));
        eprintln!("[qemu-bridge] BSC with LabWired BME280 at 0x76");
    }
    eprintln!(
        "[qemu-bridge] protocol v{PROTOCOL_VERSION}, core clock {} Hz",
        args.core_clk_hz
    );
    let mut bridge = Bridge::new(bsc);
    bridge.trace = args.trace;
    if let Some(p) = &args.transcript {
        match File::create(p) {
            Ok(f) => bridge.transcript = Some(Box::new(BufWriter::new(f))),
            Err(e) => {
                eprintln!("transcript {}: {e}", p.display());
                return ExitCode::FAILURE;
            }
        }
    }
    let _ = std::fs::remove_file(&args.socket);
    let listener = match UnixListener::bind(&args.socket) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind {}: {e}", args.socket.display());
            return ExitCode::FAILURE;
        }
    };
    eprintln!("[qemu-bridge] listening on {}", args.socket.display());
    let (stream, _) = match listener.accept() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("accept: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("[qemu-bridge] QEMU connected");
    let res = serve(stream, &mut bridge);
    if let Some(w) = bridge.transcript.as_mut() {
        let _ = w.flush();
    }
    let _ = std::fs::remove_file(&args.socket);
    match res {
        Ok(n) => {
            eprintln!(
                "[qemu-bridge] QEMU disconnected after {n} messages, {} IRQ raises, model time {} ns",
                bridge.irq_raises,
                bridge.bsc.now_ns()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[qemu-bridge] {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 10_000;

    fn bridge(with_device: bool) -> Bridge {
        let mut bsc = Bcm2835Bsc::new(DEFAULT_CORE_CLK_HZ);
        if with_device {
            bsc.attach_slave(Box::new(Bme280::new(0x76)));
        }
        let mut b = Bridge::new(bsc);
        w(&mut b, 0, 0x14, 3500);
        b
    }

    fn req(kind: u32, t_ns: u64, offset: u64, value: u64) -> Request {
        Request {
            kind,
            size: 4,
            t_ns,
            offset,
            value,
        }
    }

    fn w(b: &mut Bridge, t: u64, off: u64, v: u64) -> Reply {
        b.handle(req(MSG_WRITE, t, off, v)).unwrap()
    }

    #[test]
    fn request_roundtrip() {
        let r = req(MSG_WRITE, 123, 0x10, 0xd0);
        assert_eq!(Request::decode(&r.encode()), r);
    }

    #[test]
    fn hello_checks_version() {
        let mut b = bridge(true);
        assert_eq!(b.handle(req(MSG_HELLO, 0, 0, 2)).unwrap().value, 2);
        assert!(b.handle(req(MSG_HELLO, 0, 0, 1)).is_err());
    }

    #[test]
    fn quick_write_irq_via_sync_at_deadline() {
        let mut b = bridge(true);
        let t0 = 5_000_000;
        w(&mut b, t0, 0x0c, 0x76);
        w(&mut b, t0, 0x08, 0);
        let r = w(&mut b, t0, 0x00, 0x8000 | 0x80 | 0x200 | 0x100);
        assert_eq!(r.irq_level, 0);
        assert_eq!(r.next_deadline_ns, t0 + 10 * T);
        let r = b.handle(req(MSG_SYNC, t0 + 10 * T, 0, 0)).unwrap();
        assert_eq!((r.irq_level, r.next_deadline_ns), (0, t0 + 11 * T));
        let r = b.handle(req(MSG_SYNC, t0 + 11 * T, 0, 0)).unwrap();
        assert_eq!((r.irq_level, r.next_deadline_ns), (1, NO_DEADLINE));
        assert_eq!(
            b.handle(req(MSG_READ, t0 + 12 * T, 0x04, 0)).unwrap().value & 0x102,
            0x002
        );
    }

    #[test]
    fn no_device_nacks() {
        let mut b = bridge(false);
        w(&mut b, 0, 0x0c, 0x76);
        w(&mut b, 0, 0x08, 0);
        w(&mut b, 0, 0x00, 0x8000 | 0x80 | 0x200 | 0x100);
        let r = b.handle(req(MSG_READ, 11 * T, 0x04, 0)).unwrap();
        assert_eq!(r.value & 0x102, 0x102, "DONE + ERR");
        assert_eq!(r.irq_level, 1);
    }

    #[test]
    fn time_must_not_go_backwards() {
        let mut b = bridge(true);
        b.handle(req(MSG_SYNC, 100, 0, 0)).unwrap();
        assert!(b.handle(req(MSG_SYNC, 99, 0, 0)).is_err());
    }
}
