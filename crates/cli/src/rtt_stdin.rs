// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bytes already waiting on stdin for `run --rtt`.
//!
//! The reader is non-blocking. A thread parked in `read_line` would keep the
//! process alive after the simulation stops.

use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

/// Read whatever stdin can return without waiting. Includes the newline when
/// the operator pressed Enter, which is the byte `SEGGER_RTT_GetKey` expects.
pub fn drain_rtt_stdin() -> Vec<u8> {
    let mut stdin = io::stdin().lock();
    set_nonblocking();
    let mut buf = [0u8; 256];
    let mut out = Vec::new();
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::Interrupted =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    out
}

fn set_nonblocking() {
    static ARMED: AtomicBool = AtomicBool::new(false);
    if ARMED.swap(true, Ordering::Relaxed) {
        return;
    }
    #[cfg(unix)]
    unsafe {
        let fd = std::io::stdin().as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_ready_line_keeps_its_newline() {
        let mut src = Cursor::new(b"q\n".to_vec());
        let mut buf = [0u8; 256];
        let mut out = Vec::new();
        loop {
            match src.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        assert_eq!(out, b"q\n");
    }
}
