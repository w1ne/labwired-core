// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Wire protocol v2 (`qemu/PROTOCOL.md`) and the per-instance request loop.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use labwired_core::peripherals::bcm2835_bsc::Bcm2835Bsc;

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

/// One BSC behind the bridge. Sub-word accesses act on the containing
/// 32-bit register.
pub struct Bridge {
    pub bsc: Bcm2835Bsc,
    irq: bool,
    pub irq_raises: u64,
    pub trace: bool,
    pub transcript: Option<Box<dyn Write + Send>>,
    pub label: String,
}

impl Bridge {
    pub fn new(bsc: Bcm2835Bsc) -> Self {
        Self {
            bsc,
            irq: false,
            irq_raises: 0,
            trace: false,
            transcript: None,
            label: String::new(),
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
            eprintln!("[qemu-bridge {}] {line}", self.label);
        }
        if let Some(w) = self.transcript.as_mut() {
            writeln!(w, "{line}").map_err(|e| format!("transcript: {e}"))?;
        }
        Ok(reply)
    }
}

pub fn serve(mut stream: UnixStream, bridge: &mut Bridge) -> Result<u64, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_core::peripherals::bcm2835_bsc::DEFAULT_CORE_CLK_HZ;
    use labwired_core::peripherals::components::bme280::Bme280;
    use labwired_core::peripherals::device::I2cSlaveHost;

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
