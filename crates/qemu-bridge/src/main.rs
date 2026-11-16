// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Serves one BCM2835 BSC (I²C controller) model to QEMU's
//! `labwired-mmio-bridge` device. A BME280 sits on the bus at 0x76 unless
//! `--no-device` is given. Wire protocol: `qemu/PROTOCOL.md`.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::ExitCode;

use labwired_core::peripherals::bcm2835_bsc::Bcm2835Bsc;
use labwired_core::peripherals::components::bme280::Bme280;
use labwired_core::peripherals::device::I2cSlaveHost;

const MSG_LEN: usize = 24;
const MSG_READ: u32 = 1;
const MSG_WRITE: u32 = 2;
const MSG_REPLY: u32 = 3;
const MSG_ACK: u32 = 4;
const MSG_IRQ: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Msg {
    kind: u32,
    size: u32,
    offset: u64,
    value: u64,
}

impl Msg {
    fn encode(&self) -> [u8; MSG_LEN] {
        let mut b = [0u8; MSG_LEN];
        b[0..4].copy_from_slice(&self.kind.to_le_bytes());
        b[4..8].copy_from_slice(&self.size.to_le_bytes());
        b[8..16].copy_from_slice(&self.offset.to_le_bytes());
        b[16..24].copy_from_slice(&self.value.to_le_bytes());
        b
    }

    fn decode(b: &[u8; MSG_LEN]) -> Self {
        Self {
            kind: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            size: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            offset: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            value: u64::from_le_bytes(b[16..24].try_into().unwrap()),
        }
    }
}

struct Args {
    socket: PathBuf,
    no_device: bool,
    trace: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut socket = None;
    let mut no_device = false;
    let mut trace = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--socket" => socket = it.next().map(PathBuf::from),
            "--no-device" => no_device = true,
            "--trace" => trace = true,
            "-h" | "--help" => {
                return Err("usage: qemu-bridge --socket PATH [--no-device] [--trace]".into())
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Args {
        socket: socket.ok_or("--socket PATH is required")?,
        no_device,
        trace,
    })
}

/// One BSC behind the bridge. Sub-word accesses act on the containing
/// 32-bit register.
struct Bridge {
    bsc: Bcm2835Bsc,
    irq: bool,
    trace: bool,
}

impl Bridge {
    fn handle(&mut self, req: Msg) -> Result<Vec<Msg>, String> {
        let reg = req.offset & !3;
        let shift = (req.offset & 3) * 8;
        let reply = match req.kind {
            MSG_READ => {
                let v = (self.bsc.read_reg(reg) as u64) >> shift;
                let v = if req.size >= 4 { v } else { v & ((1u64 << (req.size * 8)) - 1) };
                if self.trace {
                    eprintln!("[qemu-bridge] R {:#04x} -> {:#010x}", req.offset, v);
                }
                Msg { kind: MSG_REPLY, size: req.size, offset: req.offset, value: v }
            }
            MSG_WRITE => {
                if self.trace {
                    eprintln!("[qemu-bridge] W {:#04x} <- {:#010x}", req.offset, req.value);
                }
                self.bsc.write_reg(reg, (req.value << shift) as u32);
                Msg { kind: MSG_ACK, size: req.size, offset: req.offset, value: 0 }
            }
            k => return Err(format!("unexpected message type {k}")),
        };
        let mut out = Vec::with_capacity(2);
        let irq = self.bsc.irq_pending();
        if irq != self.irq {
            self.irq = irq;
            if self.trace {
                eprintln!("[qemu-bridge] IRQ {}", irq as u8);
            }
            out.push(Msg { kind: MSG_IRQ, size: 0, offset: 0, value: irq as u64 });
        }
        out.push(reply);
        Ok(out)
    }
}

fn serve(mut stream: UnixStream, bridge: &mut Bridge) -> Result<(), String> {
    let mut buf = [0u8; MSG_LEN];
    let mut accesses: u64 = 0;
    loop {
        match stream.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                eprintln!("[qemu-bridge] QEMU disconnected after {accesses} accesses");
                return Ok(());
            }
            Err(e) => return Err(format!("read: {e}")),
        }
        accesses += 1;
        for m in bridge.handle(Msg::decode(&buf))? {
            stream.write_all(&m.encode()).map_err(|e| format!("write: {e}"))?;
        }
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
    let mut bsc = Bcm2835Bsc::new();
    if args.no_device {
        eprintln!("[qemu-bridge] BSC with no I2C target attached (--no-device)");
    } else {
        bsc.attach_slave(Box::new(Bme280::new(0x76)));
        eprintln!("[qemu-bridge] BSC with LabWired BME280 at 0x76");
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
    let mut bridge = Bridge { bsc, irq: false, trace: args.trace };
    let res = serve(stream, &mut bridge);
    let _ = std::fs::remove_file(&args.socket);
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[qemu-bridge] {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge(with_device: bool) -> Bridge {
        let mut bsc = Bcm2835Bsc::new();
        if with_device {
            bsc.attach_slave(Box::new(Bme280::new(0x76)));
        }
        Bridge { bsc, irq: false, trace: false }
    }

    fn w(b: &mut Bridge, off: u64, v: u64) -> Vec<Msg> {
        b.handle(Msg { kind: MSG_WRITE, size: 4, offset: off, value: v }).unwrap()
    }

    fn r(b: &mut Bridge, off: u64) -> u64 {
        b.handle(Msg { kind: MSG_READ, size: 4, offset: off, value: 0 })
            .unwrap()
            .last()
            .unwrap()
            .value
    }

    #[test]
    fn msg_roundtrip() {
        let m = Msg { kind: MSG_WRITE, size: 4, offset: 0x10, value: 0xd0 };
        assert_eq!(Msg::decode(&m.encode()), m);
    }

    #[test]
    fn quick_write_raises_then_lowers_irq() {
        let mut b = bridge(true);
        w(&mut b, 0x0c, 0x76);
        w(&mut b, 0x08, 0);
        let out = w(&mut b, 0x00, 0x8000 | 0x80 | 0x200 | 0x100);
        assert_eq!(out[0], Msg { kind: MSG_IRQ, size: 0, offset: 0, value: 1 });
        assert_eq!(out[1].kind, MSG_ACK);
        assert_eq!(r(&mut b, 0x04) & 0x102, 0x002, "DONE, no ERR");
        w(&mut b, 0x00, 0x30);
        let out = w(&mut b, 0x04, 0x302);
        assert_eq!(out[0].value, 0, "IRQ lowered");
    }

    #[test]
    fn no_device_nacks() {
        let mut b = bridge(false);
        w(&mut b, 0x0c, 0x76);
        w(&mut b, 0x08, 0);
        w(&mut b, 0x00, 0x8000 | 0x80 | 0x200 | 0x100);
        assert_eq!(r(&mut b, 0x04) & 0x102, 0x102, "DONE + ERR");
    }
}
