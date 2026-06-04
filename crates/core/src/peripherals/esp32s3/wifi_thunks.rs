// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WiFi + lwIP socket thunks — the firmware-reachability layer of the ESP32
//! WiFi functional model (simulated endpoints).
//!
//! The ESP32 WiFi MAC/PHY is a closed binary blob on an RF coprocessor, so
//! we never run it. Instead we intercept the firmware's calls at two layers
//! and emulate the *functional* outcome:
//!
//!   * **arduino WiFi** — `WiFiSTAClass::begin`/`status` short-circuit to
//!     "connected" (`WL_CONNECTED`), so the whole `esp_wifi_init`/`start`/
//!     `connect` path inside `begin` is skipped.
//!   * **lwIP BSD sockets** — `lwip_socket`/`connect`/`send`/`write`/`recv`/
//!     `read`/`close`/`ioctl`/`fcntl`/`setsockopt` are routed to a
//!     [`SimNet`](crate::network::sim::SimNet) installed via
//!     [`install_sim_net`]. The firmware's `WiFiClient`/`HTTPClient` thus
//!     talk to the in-sim virtual servers with no real network and no lwIP
//!     internals running.
//!
//! Thunk ABI mirrors [`rom_thunks`](super::rom_thunks): args are at logical
//! registers `base + i` (`base = callinc==0 ? 2 : callinc*4 + 2`), the
//! return value goes back through `RomThunkBank::return_with`, and firmware
//! memory is reached through the `bus`. State (the network, the fd→conn
//! table, the captured request/response and Serial output) lives in
//! thread-locals on the simulation thread.
//!
//! ## Status
//!
//! Validated end to end by `tests/e2e_labwired_wifi`: a real arduino-esp32
//! `HTTPClient` sketch associates, sends a valid `GET /status`, and the
//! in-sim `HttpServer` answers `200 OK` with a JSON body — captured via
//! [`sent_log`]/[`recv_log`]. Known nuance: the firmware's `HTTPClient`
//! surfaces a read-timeout code rather than 200 because our `recv` delivers
//! the whole response in a single shot, which the client's body-buffer
//! state machine doesn't expect; the simulated endpoint itself is correct.
//! Chunked/connection-close `recv` semantics to make `HTTPClient` return a
//! clean 200 are a follow-up.

use super::rom_thunks::RomThunkBank;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::network::sim::SimNet;
use crate::{Bus, SimResult};
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};

/// arduino `wl_status_t::WL_CONNECTED`.
const WL_CONNECTED: u32 = 3;
/// `ioctl` request: bytes-available-to-read (BSD `FIONREAD`).
const FIONREAD: u32 = 0x4004667F;

#[derive(Default)]
struct FdState {
    conn: u32,
    /// Bytes received from the server but not yet handed to the firmware
    /// (a single `recv` can under-read a buffered response).
    pending: Vec<u8>,
}

thread_local! {
    /// The simulated network the lwIP thunks route to.
    static SIM_NET: RefCell<SimNet> = RefCell::new(SimNet::new());
    /// Open file descriptors → connection state.
    static FDS: RefCell<HashMap<u32, FdState>> = RefCell::new(HashMap::new());
    /// Next synthetic fd to hand out (BSD fds 0–2 are stdio).
    static NEXT_FD: RefCell<u32> = const { RefCell::new(3) };
    /// Captured `HardwareSerial` output (the firmware's `Serial.print*`).
    static SERIAL_OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// All bytes the firmware sent over its socket(s) (the HTTP request).
    static SENT_LOG: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// All bytes the firmware received over its socket(s) (the HTTP response).
    static RECV_LOG: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Drain and return the firmware's captured `Serial` output so far.
pub fn serial_output() -> Vec<u8> {
    SERIAL_OUT.with(|s| s.borrow().clone())
}

/// All bytes the firmware has sent over its sockets (the HTTP request).
pub fn sent_log() -> Vec<u8> {
    SENT_LOG.with(|s| s.borrow().clone())
}

/// All bytes the firmware has received over its sockets (the HTTP response).
pub fn recv_log() -> Vec<u8> {
    RECV_LOG.with(|s| s.borrow().clone())
}

/// `HardwareSerial::write(uint8_t) -> size_t` — capture the byte. The sim's
/// arduino path stubs the real UART driver (its baud init divides by a
/// zero APB clock), so route `Serial` output here instead.
pub fn serial_write_byte(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let byte = arg(cpu, 1) as u8;
    SERIAL_OUT.with(|s| s.borrow_mut().push(byte));
    RomThunkBank::return_with(cpu, 1);
    Ok(())
}

/// `HardwareSerial::write(const uint8_t*, size_t) -> size_t` — capture the
/// buffer.
pub fn serial_write_buf(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let ptr = arg(cpu, 1) as u64;
    let len = arg(cpu, 2);
    for i in 0..len as u64 {
        let b = bus.read_u8(ptr + i)?;
        SERIAL_OUT.with(|s| s.borrow_mut().push(b));
    }
    RomThunkBank::return_with(cpu, len);
    Ok(())
}

/// Install the simulated network the lwIP thunks route to, replacing any
/// prior one and clearing fd state. Call from the run harness after
/// registering the AP/servers, before stepping the firmware.
pub fn install_sim_net(net: SimNet) {
    SIM_NET.with(|n| *n.borrow_mut() = net);
    FDS.with(|f| f.borrow_mut().clear());
    NEXT_FD.with(|f| *f.borrow_mut() = 3);
    SERIAL_OUT.with(|s| s.borrow_mut().clear());
    SENT_LOG.with(|s| s.borrow_mut().clear());
    RECV_LOG.with(|s| s.borrow_mut().clear());
}

/// Logical-register index of the first call argument for the current frame.
fn arg_base(cpu: &XtensaLx7) -> u8 {
    if cpu.ps.callinc() == 0 {
        2
    } else {
        cpu.ps.callinc() * 4 + 2
    }
}

fn arg(cpu: &XtensaLx7, n: u8) -> u32 {
    cpu.regs.read_logical(arg_base(cpu) + n)
}

/// `WiFiSTAClass::begin(...) -> wl_status_t` — skip the esp_wifi blob and
/// report the station already associated.
pub fn wifi_sta_begin(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, WL_CONNECTED);
    Ok(())
}

/// `WiFiSTAClass::status() -> wl_status_t`.
pub fn wifi_sta_status(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, WL_CONNECTED);
    Ok(())
}

/// `lwip_socket(domain, type, protocol) -> fd`. Hands out a synthetic fd;
/// the SimNet connection is created lazily on `connect`.
pub fn lwip_socket(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    let fd = NEXT_FD.with(|f| {
        let mut v = f.borrow_mut();
        let fd = *v;
        *v += 1;
        fd
    });
    if std::env::var("LABWIRED_TRACE_LWIP").is_ok() {
        eprintln!("[lwip] socket => fd {fd}");
    }
    RomThunkBank::return_with(cpu, fd);
    Ok(())
}

/// `lwip_connect(fd, *sockaddr_in, addrlen) -> 0 | -1`.
pub fn lwip_connect(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let fd = arg(cpu, 0);
    let addr_ptr = arg(cpu, 1) as u64;
    // lwIP sockaddr_in: [0]=sin_len [1]=sin_family [2..4]=sin_port (network
    // order) [4..8]=sin_addr (network order = dotted-quad in byte order).
    let port = ((bus.read_u8(addr_ptr + 2)? as u16) << 8) | (bus.read_u8(addr_ptr + 3)? as u16);
    let ip = Ipv4Addr::new(
        bus.read_u8(addr_ptr + 4)?,
        bus.read_u8(addr_ptr + 5)?,
        bus.read_u8(addr_ptr + 6)?,
        bus.read_u8(addr_ptr + 7)?,
    );
    let sock = SocketAddrV4::new(ip, port);
    let conn = SIM_NET.with(|n| n.borrow_mut().connect(sock));
    if std::env::var("LABWIRED_TRACE_LWIP").is_ok() {
        eprintln!("[lwip] connect fd={fd} -> {sock} => {conn:?}");
    }
    let ret = match conn {
        Some(cid) => {
            FDS.with(|f| {
                f.borrow_mut().insert(
                    fd,
                    FdState {
                        conn: cid,
                        pending: Vec::new(),
                    },
                )
            });
            0
        }
        None => u32::MAX, // -1: connection refused
    };
    RomThunkBank::return_with(cpu, ret);
    Ok(())
}

/// `lwip_send(fd, buf, len, flags)` / `lwip_write(fd, buf, len) -> nbytes`.
/// Both place `fd`/`buf`/`len` in the first three args.
pub fn lwip_send(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    if std::env::var("LABWIRED_TRACE_LWIP").is_ok() {
        let p = arg(cpu, 1) as u64;
        let l = arg(cpu, 2).min(120);
        let mut v = Vec::new();
        for i in 0..l as u64 {
            v.push(bus.read_u8(p + i).unwrap_or(0));
        }
        eprintln!(
            "[lwip] send fd={} {} bytes: {:?}",
            arg(cpu, 0),
            arg(cpu, 2),
            String::from_utf8_lossy(&v)
        );
    }
    let fd = arg(cpu, 0);
    let buf = arg(cpu, 1) as u64;
    let len = arg(cpu, 2);
    let mut data = Vec::with_capacity(len as usize);
    for i in 0..len as u64 {
        data.push(bus.read_u8(buf + i)?);
    }
    SENT_LOG.with(|s| s.borrow_mut().extend_from_slice(&data));
    if let Some(conn) = FDS.with(|f| f.borrow().get(&fd).map(|s| s.conn)) {
        SIM_NET.with(|n| {
            let _ = n.borrow_mut().send(conn, &data);
        });
    }
    RomThunkBank::return_with(cpu, len);
    Ok(())
}

/// `lwip_recv(fd, buf, len, flags)` / `lwip_read(fd, buf, len) -> nbytes`.
/// Returns 0 at end of the server's response (EOF).
pub fn lwip_recv(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    if std::env::var("LABWIRED_TRACE_LWIP").is_ok() {
        eprintln!("[lwip] lwip_recv fd={}", arg(cpu, 0));
    }
    let fd = arg(cpu, 0);
    let buf = arg(cpu, 1) as u64;
    let len = arg(cpu, 2) as usize;
    // Refill the per-fd pending buffer from SimNet, then hand back up to `len`.
    let chunk: Vec<u8> = FDS.with(|f| {
        let mut fds = f.borrow_mut();
        let Some(st) = fds.get_mut(&fd) else {
            return Vec::new();
        };
        if st.pending.is_empty() {
            st.pending = SIM_NET.with(|n| n.borrow_mut().recv(st.conn));
        }
        let take = len.min(st.pending.len());
        st.pending.drain(..take).collect()
    });
    if std::env::var("LABWIRED_TRACE_LWIP").is_ok() {
        eprintln!(
            "[lwip] recv fd={} => {} bytes: {:?}",
            fd,
            chunk.len(),
            String::from_utf8_lossy(&chunk)
        );
    }
    RECV_LOG.with(|s| s.borrow_mut().extend_from_slice(&chunk));
    for (i, byte) in chunk.iter().enumerate() {
        bus.write_u8(buf + i as u64, *byte)?;
    }
    RomThunkBank::return_with(cpu, chunk.len() as u32);
    Ok(())
}

/// `lwip_close(fd) -> 0`.
pub fn lwip_close(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    if std::env::var("LABWIRED_TRACE_LWIP").is_ok() {
        eprintln!("[lwip] lwip_close fd={}", arg(cpu, 0));
    }
    let fd = arg(cpu, 0);
    if let Some(conn) = FDS.with(|f| f.borrow_mut().remove(&fd).map(|s| s.conn)) {
        SIM_NET.with(|n| n.borrow_mut().close(conn));
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `lwip_ioctl(fd, request, argp) -> 0 | -1`. Implements `FIONREAD`
/// (`WiFiClient::available()`); other requests succeed as no-ops.
pub fn lwip_ioctl(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    if std::env::var("LABWIRED_TRACE_LWIP").is_ok() {
        eprintln!("[lwip] lwip_ioctl fd={}", arg(cpu, 0));
    }
    let fd = arg(cpu, 0);
    let request = arg(cpu, 1);
    let argp = arg(cpu, 2) as u64;
    if request == FIONREAD {
        // Bytes ready = per-fd pending + whatever the server has buffered.
        let avail = FDS.with(|f| {
            let mut fds = f.borrow_mut();
            let Some(st) = fds.get_mut(&fd) else {
                return 0u32;
            };
            if st.pending.is_empty() {
                st.pending = SIM_NET.with(|n| n.borrow_mut().recv(st.conn));
            }
            st.pending.len() as u32
        });
        bus.write_u32(argp, avail)?;
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `lwip_fcntl(fd, cmd, arg) -> 0`. Non-blocking flag toggling is a no-op
/// (our sockets are synchronous).
pub fn lwip_fcntl(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    if std::env::var("LABWIRED_TRACE_LWIP").is_ok() {
        eprintln!("[lwip] lwip_fcntl fd={}", arg(cpu, 0));
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `lwip_setsockopt(...) -> 0` — accept and ignore socket options.
pub fn lwip_sockopt_ok(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `lwip_getsockopt(fd, level, optname, optval*, optlen*) -> 0`. Reports
/// "no error" by zeroing `*optval` — the `SO_ERROR` check arduino's
/// non-blocking `connect()` runs after `select()`. (Our connect is
/// synchronous-success, so there is never a pending error.)
pub fn lwip_getsockopt(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    if std::env::var("LABWIRED_TRACE_LWIP").is_ok() {
        eprintln!("[lwip] lwip_getsockopt fd={}", arg(cpu, 0));
    }
    let optval = arg(cpu, 3) as u64;
    if optval != 0 {
        bus.write_u32(optval, 0)?;
    }
    RomThunkBank::return_with(cpu, 0);
    Ok(())
}

/// `lwip_select(nfds, readfds, writefds, exceptfds, timeout) -> 1`. Reports
/// one fd ready. arduino's connect/read loops `select()` on the socket;
/// our SimNet is synchronous, so the fd is always ready. The caller's
/// `fd_set`s already mark the socket, and leaving them untouched keeps the
/// `FD_ISSET` check true.
pub fn lwip_select(cpu: &mut XtensaLx7, _bus: &mut dyn Bus) -> SimResult<()> {
    if std::env::var("LABWIRED_TRACE_LWIP").is_ok() {
        eprintln!("[lwip] lwip_select fd={}", arg(cpu, 0));
    }
    RomThunkBank::return_with(cpu, 1);
    Ok(())
}

/// DRAM scratch for synthetic strings returned by thunks (above the seeded
/// stacks, within ESP32 internal SRAM).
const STR_SCRATCH: u32 = 0x3FFE_FF00;

/// `pcTaskGetName(handle) -> char*`. Called by some WiFi/event init before
/// the scheduler has set `pxCurrentTCB`, which would trip
/// `configASSERT(pxTCB)`. Return a stable synthetic name instead.
pub fn pc_task_get_name(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    for (i, b) in b"sys\0".iter().enumerate() {
        bus.write_u8(STR_SCRATCH as u64 + i as u64, *b)?;
    }
    RomThunkBank::return_with(cpu, STR_SCRATCH);
    Ok(())
}

/// Read a NUL-terminated C string from firmware memory (bounded).
fn read_cstr(bus: &dyn Bus, ptr: u32, max: u64) -> String {
    if ptr == 0 {
        return String::new();
    }
    let mut bytes = Vec::new();
    for i in 0..max {
        match bus.read_u8(ptr as u64 + i) {
            Ok(0) | Err(_) => break,
            Ok(b) => bytes.push(b),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Debug-only thunk for `__assert_func(file, line, func, expr)`: prints the
/// failed assertion (which a plain `abort_halt` swallows) then halts the CPU.
/// Wire it in place of `abort_halt` for `__assert_func` to diagnose early
/// boot asserts.
pub fn debug_assert_func(cpu: &mut XtensaLx7, bus: &mut dyn Bus) -> SimResult<()> {
    let file = arg(cpu, 0);
    let line = arg(cpu, 1);
    let func = arg(cpu, 2);
    let expr = arg(cpu, 3);
    eprintln!(
        "[ASSERT] {}:{} in {}(): {}",
        read_cstr(bus, file, 256),
        line,
        read_cstr(bus, func, 128),
        read_cstr(bus, expr, 256),
    );
    cpu.halted = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::sim::{HttpResponse, HttpServer};
    use std::sync::Arc;

    #[test]
    fn install_sim_net_resets_fd_state() {
        let mut net = SimNet::new();
        net.listen(
            SocketAddrV4::new(Ipv4Addr::new(192, 168, 4, 1), 80),
            Arc::new(HttpServer::new().get("/", HttpResponse::ok("ok"))),
        );
        install_sim_net(net);
        // Fresh fd numbering starts at 3.
        NEXT_FD.with(|f| assert_eq!(*f.borrow(), 3));
        FDS.with(|f| assert!(f.borrow().is_empty()));
        // The installed network is reachable.
        let conn = SIM_NET.with(|n| {
            n.borrow_mut()
                .connect(SocketAddrV4::new(Ipv4Addr::new(192, 168, 4, 1), 80))
        });
        assert!(conn.is_some());
    }
}
