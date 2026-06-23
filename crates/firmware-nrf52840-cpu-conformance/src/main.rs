// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
//! nRF52840 **CPU**-conformance firmware.
//!
//! Where `firmware-nrf52840-conformance` exercises peripherals, this firmware
//! exercises four ARMv7-M *core* behaviours that were recently modelled while
//! bringing up Zephyr, and whose values are architecture-defined (identical on
//! the simulator and on real silicon):
//!
//!   1. **SVC delivery + `MRS IPSR` in exception context** — execute `svc #0`,
//!      and inside the SVCall handler read IPSR (`mrs rN, IPSR`). In a SVCall
//!      handler IPSR must read 11 (the SVCall exception number). This single
//!      check exercises BOTH the SVC instruction (it must pend/take SVCall, not
//!      fall through as a NOP) AND `MRS IPSR` (it must return the active
//!      exception number, not 0).
//!
//!   2. **`ldr.w pc,[rn,rm,lsl#2]` switch dispatch** — a dense integer `match`
//!      that the compiler lowers to a PC-relative jump table (`ldr.w pc,[...]`).
//!      Several inputs are dispatched and a position-weighted accumulator proves
//!      every case branched to the RIGHT arm (a mis-modelled load-to-PC would
//!      land on the wrong arm and change the accumulator).
//!
//!   3. **MPU_TYPE.DREGION** — read `MPU_TYPE` (`0xE000ED90`) and store the
//!      DREGION field (bits[15:8]). The nRF52840 Cortex-M4F reports 8 regions.
//!      This is the value most in need of confirmation against silicon, so it is
//!      the most important digest word to diff against HW.
//!
//! Layout mirrors the peripheral conformance firmware: `VERDICT[0]` is the DONE
//! sentinel (written LAST, after every check); `VERDICT[1..]` are per-check
//! digest words. The harness (`crates/hw-oracle/tests/nrf52_cpu_conformance.rs`)
//! polls `VERDICT[0]` then diffs the block sim-vs-silicon.
#![no_std]
#![no_main]

use core::ptr::write_volatile;
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// ── Digest block ─────────────────────────────────────────────────────────────

/// Fixed RAM block for the observable-state digest (well below the stack).
const VERDICT: u32 = 0x2000_3000;
/// Written to `VERDICT[0]` last, after every check completes.
const DONE_MAGIC: u32 = 0x5284_0D0E;

const IDX_DONE: usize = 0;
const IDX_IPSR_IN_SVC: usize = 1; // IPSR read inside SVCall handler (expect 11)
const IDX_SWITCH_ACC: usize = 2; // compiler switch-table accumulator
const IDX_MPU_DREGION: usize = 3; // MPU_TYPE.DREGION (bits[15:8]); expect 8
const IDX_LDRPC_ACC: usize = 4; // explicit `ldr.w pc,[rn,rm,lsl#2]` dispatch

const VERDICT_WORDS: usize = 16;

// ── Core register addresses (ARMv7-M System Control Space) ────────────────────

/// MPU_TYPE — ARMv7-M MPU type register. DREGION = bits[15:8].
const MPU_TYPE: u32 = 0xE000_ED90;

// ── Cross-handler scratch ─────────────────────────────────────────────────────

/// IPSR value captured inside the SVCall handler. The handler runs in exception
/// context; `main` reads this back after the `svc` returns.
static IPSR_IN_SVC: AtomicU32 = AtomicU32::new(0xFFFF_FFFF);

// ── MMIO helpers ─────────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn wr(addr: u32, val: u32) {
    write_volatile(addr as *mut u32, val);
}

#[inline(always)]
unsafe fn rd(addr: u32) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline(always)]
unsafe fn digest(idx: usize, val: u32) {
    wr(VERDICT + (idx as u32) * 4, val);
}

// ── SVCall exception handler ──────────────────────────────────────────────────
//
// Reached by `svc #0` from `main`. In a SVCall handler the IPSR (xPSR[8:0])
// must read 11 — the exception number for SVCall on every ARMv7-M part. Read it
// with `mrs` and stash it for `main`. (Exercises both SVC delivery AND MRS
// IPSR: a NOP-modelled SVC never enters here; an IPSR-reads-0 model stores 0.)
#[exception]
fn SVCall() {
    let ipsr: u32;
    unsafe {
        core::arch::asm!("mrs {0}, IPSR", out(reg) ipsr, options(nomem, nostack, preserves_flags));
    }
    IPSR_IN_SVC.store(ipsr, Ordering::Relaxed);
}

// ── ldr-pc switch dispatch ─────────────────────────────────────────────────────
//
// A dense, contiguous integer `match` whose arms have DISTINCT, non-constant
// bodies (each spins a small per-arm loop before producing its value), so the
// compiler cannot collapse the switch into a rodata value-lookup. With ≥12
// branch targets spread far apart in code, the thumbv7em backend emits a
// PC-relative *jump* table — `ldr.w pc,[rn,rm,lsl#2]` — the exact GCC/LLVM
// switch-branch form whose load-to-PC modelling was just fixed (a mis-modelled
// load-to-PC lands on the wrong arm). `core::hint::black_box` blocks the
// constant-folding / value-table rewrite without adding any nondeterminism; the
// returned values stay architecture-defined.
//
// `#[inline(never)]` + a runtime `sel` keep the table in the final binary so the
// disasm check can confirm the `ldr.w pc,[...]` encoding.
#[inline(never)]
fn switch_dispatch(sel: u32) -> u32 {
    // Per-arm: fold a distinct constant through black_box so each arm is a
    // separate, non-foldable basic block. The value is still fully determined.
    macro_rules! arm {
        ($v:expr) => {{
            let mut acc: u32 = core::hint::black_box($v);
            // A short, fixed, per-arm loop makes the bodies distinct in size and
            // defeats the value-table rewrite; the result is deterministic.
            for _ in 0..core::hint::black_box(1u32) {
                acc = core::hint::black_box(acc);
            }
            acc
        }};
    }
    match sel {
        0 => arm!(0x0000_0001),
        1 => arm!(0x0000_0020),
        2 => arm!(0x0000_0300),
        3 => arm!(0x0000_4000),
        4 => arm!(0x0005_0000),
        5 => arm!(0x0060_0000),
        6 => arm!(0x0700_0000),
        7 => arm!(0x8000_0000),
        8 => arm!(0x0000_000A),
        9 => arm!(0x0000_00B0),
        10 => arm!(0x0000_0C00),
        11 => arm!(0x0000_D000),
        _ => arm!(0xDEAD_BEEF),
    }
}

// ── Explicit ldr-to-PC jump table ──────────────────────────────────────────────
//
// The `match` above lowers to TBB (compact byte offsets), which never exercises
// the 32-bit `ldr.w pc,[rn,rm,lsl#2]` absolute-address switch table whose
// load-to-PC modelling was fixed. Emit that exact instruction by hand so it is
// diffed on sim and silicon: the table holds absolute (thumb-bit-set) addresses
// of the case labels; `ldr.w pc,[base,i,lsl#2]` loads table[i] and branches. A
// load-to-PC model that fails to suppress pc_increment lands one halfword past
// the case → a different value → digest mismatch.
#[inline(never)]
fn ldr_pc_dispatch(idx: u32) -> u32 {
    let result: u32;
    unsafe {
        core::arch::asm!(
            "adr   {t}, 20f",
            "ldr.w pc, [{t}, {i}, lsl #2]",
            ".p2align 2",
            "20:",
            ".word 21f + 1",
            ".word 22f + 1",
            ".word 23f + 1",
            ".word 24f + 1",
            ".word 25f + 1",
            ".word 26f + 1",
            "21: movw {r}, #0x1001",
            "b 29f",
            "22: movw {r}, #0x2002",
            "b 29f",
            "23: movw {r}, #0x3003",
            "b 29f",
            "24: movw {r}, #0x4004",
            "b 29f",
            "25: movw {r}, #0x5005",
            "b 29f",
            "26: movw {r}, #0x6006",
            "b 29f",
            "29:",
            t = out(reg) _,
            i = in(reg) idx,
            r = lateout(reg) result,
            options(nostack),
        );
    }
    result
}

// ── Check 4: explicit ldr.w-pc dispatch ────────────────────────────────────────
unsafe fn check_ldr_pc() {
    // Dispatch each index through the hand-built ldr.w-pc table, weighting by
    // index so any wrong-arm landing changes the fold.
    let mut acc: u32 = 0;
    let mut i = 0u32;
    while i < 6 {
        acc = acc.wrapping_add(ldr_pc_dispatch(i).wrapping_mul(i + 1));
        i += 1;
    }
    digest(IDX_LDRPC_ACC, acc);
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[entry]
fn main() -> ! {
    unsafe {
        // Zero the digest block first.
        for i in 0..VERDICT_WORDS {
            digest(i, 0);
        }

        check_svc_ipsr();
        check_switch_dispatch();
        check_mpu_type();
        check_ldr_pc();

        // Sentinel written LAST so the harness knows every check ran.
        digest(IDX_DONE, DONE_MAGIC);
    }
    loop {}
}

// ── Check 1: SVC delivery + MRS IPSR ───────────────────────────────────────────
unsafe fn check_svc_ipsr() {
    IPSR_IN_SVC.store(0xFFFF_FFFF, Ordering::Relaxed);
    // Supervisor call. Must vector to SVCall (exception 11) and run the handler,
    // which records IPSR. A NOP-modelled SVC would leave the sentinel untouched.
    core::arch::asm!("svc #0", options(nomem, nostack, preserves_flags));
    digest(IDX_IPSR_IN_SVC, IPSR_IN_SVC.load(Ordering::Relaxed));
}

// ── Check 2: ldr-pc switch-table dispatch ──────────────────────────────────────
unsafe fn check_switch_dispatch() {
    // Run every defined case plus one out-of-range value, XOR-folding the
    // results. The fold is order-independent but case-sensitive: any input
    // landing on the wrong arm changes the final accumulator.
    //
    // Expected (XOR of all arms 0..=11 and the default):
    //   0x00000001 ^ 0x00000020 ^ 0x00000300 ^ 0x00004000
    // ^ 0x00050000 ^ 0x00600000 ^ 0x07000000 ^ 0x80000000
    // ^ 0x0000000A ^ 0x000000B0 ^ 0x00000C00 ^ 0x0000D000
    // ^ 0xDEADBEEF
    let mut acc: u32 = 0;
    for sel in 0..=12u32 {
        acc ^= switch_dispatch(sel);
    }
    digest(IDX_SWITCH_ACC, acc);
}

// ── Check 3: MPU_TYPE.DREGION ──────────────────────────────────────────────────
unsafe fn check_mpu_type() {
    let mpu_type = rd(MPU_TYPE);
    let dregion = (mpu_type >> 8) & 0xFF;
    digest(IDX_MPU_DREGION, dregion);
}
