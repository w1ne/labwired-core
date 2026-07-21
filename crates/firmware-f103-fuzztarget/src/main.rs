// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
//! STM32F103 **fuzz target** (Phase 0 of the firmware-fuzzing plan).
//!
//! A binary command parser whose input is a byte stream in a fixed RAM buffer
//! (`FUZZ_LEN` words then `FUZZ_DATA` bytes). The harness fills it — in sim from
//! the fuzzer's `&[u8]`, on the HIL bench via openocd — then runs the firmware;
//! the same crashing input can therefore be **replayed on real silicon**.
//!
//! Input format: a sequence of frames `[op:u8][len:u8][data:len bytes]`.
//! Commands P/A/E are well-behaved (they give the fuzzer coverage structure to
//! explore); **C is the planted bug** — an `unsafe`, unbounded copy of `len`
//! bytes into a 16-byte stack buffer, i.e. a classic firmware stack overflow.
//!
//! Outcome (the crash oracle), written to `VERDICT[0]`:
//!   * `DONE`  — all frames parsed cleanly
//!   * `FAULT` — a fault handler ran (HardFault/Bus/Usage/MemManage)
//!   * neither, after the harness timeout — a hang (lockup / infinite loop)
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};

const FUZZ_LEN: u32 = 0x2000_2800; // u32: number of input bytes the harness wrote
const FUZZ_DATA: u32 = 0x2000_2804; // the input bytes
const FUZZ_MAX: usize = 1024; // cap, so a corrupt length can't run away

const VERDICT: u32 = 0x2000_3000;
const DONE_MAGIC: u32 = 0xC0DE_F022;
const FAULT_MAGIC: u32 = 0xDEAD_FA17;

#[inline(always)]
unsafe fn wr(addr: u32, val: u32) {
    write_volatile(addr as *mut u32, val);
}
#[inline(always)]
unsafe fn rd(addr: u32) -> u32 {
    read_volatile(addr as *const u32)
}
#[inline(always)]
unsafe fn rd8(addr: u32) -> u8 {
    read_volatile(addr as *const u8)
}

#[inline(never)]
fn halt_forever() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[no_mangle]
pub extern "C" fn HardFaultHandler() -> ! {
    unsafe { wr(VERDICT, FAULT_MAGIC) };
    halt_forever()
}
#[no_mangle]
pub extern "C" fn DefaultHandler() -> ! {
    unsafe { wr(VERDICT, FAULT_MAGIC) };
    halt_forever()
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        wr(VERDICT, 0); // clear the outcome; harness already wrote the input buffer
        let len = core::cmp::min(rd(FUZZ_LEN) as usize, FUZZ_MAX);
        parse(len);
        wr(VERDICT, DONE_MAGIC);
    }
    halt_forever()
}

/// Walk the frame stream. Bounds the per-frame data read to the remaining input
/// — the ONLY out-of-bounds is the planted `C` handler.
unsafe fn parse(len: usize) {
    let mut i = 0usize;
    while i < len {
        let op = rd8(FUZZ_DATA + i as u32);
        i += 1;
        if i >= len {
            break;
        }
        let want = rd8(FUZZ_DATA + i as u32) as usize;
        i += 1;
        let avail = len - i; // bytes actually present
        let flen = core::cmp::min(want, avail);
        handle(op, FUZZ_DATA + i as u32, flen, want);
        i += flen;
    }
}

/// `want` is the attacker-controlled length byte; `flen` is what's actually
/// present. Well-behaved handlers use `flen`; the bug uses `want`.
unsafe fn handle(op: u8, data: u32, flen: usize, want: usize) {
    match op {
        b'P' => {
            // ping: a no-op branch (coverage)
            core::hint::black_box(op);
        }
        b'A' => {
            // add: fold the data bytes (coverage + bounded read)
            let mut acc: u32 = 0;
            let mut j = 0;
            while j < flen {
                acc = acc.wrapping_add(rd8(data + j as u32) as u32);
                j += 1;
            }
            core::hint::black_box(acc);
        }
        b'E' => {
            // echo: SAFE bounded copy into a stack scratch (no statics → no .bss)
            let mut echo = [0u8; 64];
            let n = core::cmp::min(flen, 64);
            let mut j = 0;
            while j < n {
                echo[j] = rd8(data + j as u32);
                j += 1;
            }
            core::hint::black_box(echo[0]);
        }
        b'C' => {
            // ── PLANTED BUG ──────────────────────────────────────────────────
            // Reachable only via op 'C' + a large length; the overflow lives in
            // its own (never-inlined) frame so its smashed return address is
            // actually used.
            vuln_copy(data, want);
        }
        _ => {
            // unknown opcode: ignore (coverage: the default arm)
        }
    }
}

/// Classic firmware stack overflow: copy the attacker-controlled `want` bytes
/// into a 16-byte stack buffer with no bound check. Not inlined, so its smashed
/// saved-LR is the address the core actually returns through → fault.
#[inline(never)]
unsafe fn vuln_copy(data: u32, want: usize) {
    let mut buf = [0u8; 16];
    let mut j = 0;
    while j < want {
        core::ptr::write_volatile(buf.as_mut_ptr().add(j), rd8(data + j as u32));
        j += 1;
    }
    core::hint::black_box(buf[0]);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // A Rust panic is a logic bug, not a hardware fault — mark it as FAULT too so
    // the harness treats a panic as a crash, not a hang.
    unsafe { wr(VERDICT, FAULT_MAGIC) };
    loop {}
}
