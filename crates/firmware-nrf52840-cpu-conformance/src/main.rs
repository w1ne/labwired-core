// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
//! nRF52840 **CPU**-conformance firmware.
//!
//! Where `firmware-nrf52840-conformance` exercises peripherals, this firmware
//! exercises ARMv7-M *core* behaviours that were modelled while bringing up
//! Zephyr, and whose values are architecture-defined (identical on the simulator
//! and on real silicon):
//!
//!   1. **SVC delivery + `MRS IPSR` in exception context** — `svc #0`, and inside
//!      the SVCall handler read IPSR (`mrs rN, IPSR`). Must read 11 (SVCall).
//!   2. **`ldr.w pc,[rn,rm,lsl#2]` switch dispatch** — compiler jump table.
//!   3. **MPU_TYPE.DREGION** — `0xE000ED90`, bits[15:8]; the M4F reports 8.
//!   4. **explicit `ldr.w pc` jump table** — hand-emitted absolute-address table.
//!
//! Words 5..8 lock the four fixes made while running Zephyr ztest, each probed so
//! its observable result is architecture-defined and identical on sim/silicon:
//!
//!   5. **BASEPRI** (#355) — priority masking: a configured-priority SysTick is
//!      pended while BASEPRI masks it (must NOT run), then BASEPRI is lowered (it
//!      runs). Also exercises BASEPRI_MAX only-raises semantics.
//!   6. **FAULTMASK** (#356) — a pended SysTick is masked by FAULTMASK (must NOT
//!      run), then FAULTMASK is cleared (it runs); and FAULTMASK auto-clears on
//!      exception return (set inside a SysTick handler → reads 0 after return).
//!   7. **MSP/PSP banking** (#354) — set PSP + CONTROL.SPSEL=1, MSP/PSP read
//!      distinct, take an exception from thread/PSP (frame stacks on PSP, handler
//!      sees EXC_RETURN=0xFFFFFFFD), return and confirm PSP is restored.
//!   8. **ICSR write-only/self-clearing** (#358) — PENDSVSET self-clears on take;
//!      PENDSVCLR reads back 0; the `ICSR |= PENDSVSET` RMW re-pends exactly once
//!      (a stale PENDSVCLR must not cancel it).
//!
//! `VERDICT[0]` is the DONE sentinel (written LAST); `VERDICT[1..]` are per-check
//! digest words. The harness polls `VERDICT[0]` then diffs sim-vs-silicon.
#![no_std]
#![no_main]
#![allow(asm_sub_register)]

use core::ptr::{addr_of_mut, write_volatile};
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
const IDX_BASEPRI: usize = 5; // BASEPRI priority masking (#355)
const IDX_FAULTMASK: usize = 6; // FAULTMASK masking + auto-clear (#356)
const IDX_MSP_PSP: usize = 7; // MSP/PSP banking + EXC_RETURN (#354)
const IDX_ICSR: usize = 8; // ICSR write-only/self-clearing PENDSV (#358)

const VERDICT_WORDS: usize = 16;

// ── Core register addresses (ARMv7-M System Control Space) ────────────────────

const MPU_TYPE: u32 = 0xE000_ED90;
/// Interrupt Control and State Register.
const ICSR: u32 = 0xE000_ED04;
const ICSR_PENDSVSET: u32 = 1 << 28;
const ICSR_PENDSVCLR: u32 = 1 << 27;
const ICSR_PENDSTSET: u32 = 1 << 26;
/// System Handler Priority Register 3: PendSV = byte[23:16], SysTick = byte[31:24].
const SHPR3: u32 = 0xE000_ED20;

/// Priority assigned to SysTick for the masking probes. On a 3-priority-bit core
/// the low 5 bits are RAZ, so 0x40 is a real, distinct, maskable level.
const SYSTICK_PRIO: u32 = 0x40;

// ── Cross-handler scratch ─────────────────────────────────────────────────────

static IPSR_IN_SVC: AtomicU32 = AtomicU32::new(0xFFFF_FFFF);
/// EXC_RETURN (LR) captured at the top of the SVCall handler.
static EXC_RETURN_SEEN: AtomicU32 = AtomicU32::new(0);
static SYSTICK_COUNT: AtomicU32 = AtomicU32::new(0);
static PENDSV_COUNT: AtomicU32 = AtomicU32::new(0);
/// When 1, the SysTick handler sets FAULTMASK before returning (auto-clear probe).
static SET_FAULTMASK_IN_SYSTICK: AtomicU32 = AtomicU32::new(0);

/// Dedicated PSP stack for the banking probe (256 bytes; the 32-byte exception
/// frame fits with room to spare). In `.bss`, so its address is identical on sim
/// and silicon (same linker layout, same RAM base).
const PSP_STACK_WORDS: usize = 64;
static mut PSP_STACK: [u32; PSP_STACK_WORDS] = [0; PSP_STACK_WORDS];

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

// ── Exception handlers ─────────────────────────────────────────────────────────

// Reached by `svc #0`. Capture EXC_RETURN (LR) FIRST — before any call could
// reuse LR — then IPSR (must read 11 in a SVCall handler). Both the IPSR probe
// (entered from thread/MSP, EXC_RETURN 0xFFFFFFF9) and the MSP/PSP probe (entered
// from thread/PSP, EXC_RETURN 0xFFFFFFFD) land here.
#[exception]
fn SVCall() {
    let exc_return: u32;
    let ipsr: u32;
    unsafe {
        core::arch::asm!("mov {0}, lr", out(reg) exc_return, options(nomem, nostack, preserves_flags));
        core::arch::asm!("mrs {0}, IPSR", out(reg) ipsr, options(nomem, nostack, preserves_flags));
    }
    EXC_RETURN_SEEN.store(exc_return, Ordering::Relaxed);
    IPSR_IN_SVC.store(ipsr, Ordering::Relaxed);
}

#[exception]
fn SysTick() {
    SYSTICK_COUNT.fetch_add(1, Ordering::Relaxed);
    if SET_FAULTMASK_IN_SYSTICK.load(Ordering::Relaxed) == 1 {
        // Raise FAULTMASK inside the handler; the architecture must auto-clear it
        // on exception return (this is NOT NMI).
        unsafe {
            core::arch::asm!("cpsid f", options(nomem, nostack, preserves_flags));
        }
    }
}

#[exception]
fn PendSV() {
    PENDSV_COUNT.fetch_add(1, Ordering::Relaxed);
}

// ── ldr-pc switch dispatch ─────────────────────────────────────────────────────
#[inline(never)]
fn switch_dispatch(sel: u32) -> u32 {
    macro_rules! arm {
        ($v:expr) => {{
            let mut acc: u32 = core::hint::black_box($v);
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

// ── Entry point ──────────────────────────────────────────────────────────────

#[entry]
fn main() -> ! {
    unsafe {
        for i in 0..VERDICT_WORDS {
            digest(i, 0);
        }

        // Give PendSV and SysTick a defined, maskable priority for the masking
        // probes (PendSV byte[23:16], SysTick byte[31:24] of SHPR3).
        wr(SHPR3, (SYSTICK_PRIO << 24) | (SYSTICK_PRIO << 16));

        check_svc_ipsr();
        check_switch_dispatch();
        check_mpu_type();
        check_ldr_pc();
        check_basepri();
        check_faultmask();
        check_msp_psp();
        check_icsr();

        digest(IDX_DONE, DONE_MAGIC);
    }
    halt_forever()
}

// ── Check 1: SVC delivery + MRS IPSR ───────────────────────────────────────────
unsafe fn check_svc_ipsr() {
    IPSR_IN_SVC.store(0xFFFF_FFFF, Ordering::Relaxed);
    core::arch::asm!("svc #0", options(nomem, nostack, preserves_flags));
    digest(IDX_IPSR_IN_SVC, IPSR_IN_SVC.load(Ordering::Relaxed));
}

// ── Check 2: ldr-pc switch-table dispatch ──────────────────────────────────────
unsafe fn check_switch_dispatch() {
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

// ── Check 4: explicit ldr.w-pc dispatch ────────────────────────────────────────
unsafe fn check_ldr_pc() {
    let mut acc: u32 = 0;
    let mut i = 0u32;
    while i < 6 {
        acc = acc.wrapping_add(ldr_pc_dispatch(i).wrapping_mul(i + 1));
        i += 1;
    }
    digest(IDX_LDRPC_ACC, acc);
}

// ── Check 5: BASEPRI priority masking (#355) ───────────────────────────────────
//
// SysTick is given priority SYSTICK_PRIO. Pend it with BASEPRI raised to the same
// level (masked → must not run), confirm the counter is unchanged, then drop
// BASEPRI to 0 (unmasked → runs once). Also exercise BASEPRI_MAX "only raises":
// a weaker (numerically larger) write is ignored, a stronger write applies.
//
// Fold = (TAG 0xB << 4) | bool_nibble; all-correct → 0x000000BF.
unsafe fn check_basepri() {
    SYSTICK_COUNT.store(0, Ordering::Relaxed);

    // Mask: BASEPRI = SYSTICK_PRIO, pend SysTick, it must NOT fire.
    set_basepri(SYSTICK_PRIO);
    wr(ICSR, ICSR_PENDSTSET);
    nops();
    let masked_count = SYSTICK_COUNT.load(Ordering::Relaxed);
    let basepri_rb = get_basepri();

    // BASEPRI_MAX only-raises: a weaker level (larger number) is ignored…
    set_basepri_max(0x80);
    let after_weaker = get_basepri();
    // …a stronger level (smaller number) applies.
    set_basepri_max(0x20);
    let after_stronger = get_basepri();

    // Unmask: the still-pending SysTick now fires exactly once.
    set_basepri(0);
    nops();
    let unmasked_count = SYSTICK_COUNT.load(Ordering::Relaxed);

    let b0 = (masked_count == 0) as u32;
    let b1 = (unmasked_count == 1) as u32;
    let b2 = (basepri_rb == SYSTICK_PRIO) as u32;
    let b3 = (after_weaker == SYSTICK_PRIO && after_stronger == 0x20) as u32;
    let nibble = b0 | (b1 << 1) | (b2 << 2) | (b3 << 3);
    digest(IDX_BASEPRI, (0xB << 4) | nibble);
}

// ── Check 6: FAULTMASK masking + auto-clear (#356) ──────────────────────────────
//
// FAULTMASK masks all maskable exceptions regardless of priority. Pend SysTick
// with FAULTMASK set (must NOT run), clear it (runs once). Then verify FAULTMASK
// auto-clears on exception return: a SysTick handler sets FAULTMASK and returns;
// FAULTMASK must read 0 afterwards.
//
// Fold = (TAG 0xF << 4) | bool_nibble; all-correct → 0x000000FF.
unsafe fn check_faultmask() {
    SYSTICK_COUNT.store(0, Ordering::Relaxed);
    SET_FAULTMASK_IN_SYSTICK.store(0, Ordering::Relaxed);

    // Mask with FAULTMASK, pend SysTick — must not fire.
    core::arch::asm!("cpsid f", options(nomem, nostack, preserves_flags));
    wr(ICSR, ICSR_PENDSTSET);
    nops();
    let masked_count = SYSTICK_COUNT.load(Ordering::Relaxed);
    // Clear FAULTMASK — the pending SysTick fires once.
    core::arch::asm!("cpsie f", options(nomem, nostack, preserves_flags));
    nops();
    let unmasked_count = SYSTICK_COUNT.load(Ordering::Relaxed);

    // Auto-clear on exception return: handler raises FAULTMASK, returns.
    SET_FAULTMASK_IN_SYSTICK.store(1, Ordering::Relaxed);
    wr(ICSR, ICSR_PENDSTSET);
    nops();
    SET_FAULTMASK_IN_SYSTICK.store(0, Ordering::Relaxed);
    let faultmask_after: u32 = get_faultmask();

    let b0 = (masked_count == 0) as u32;
    let b1 = (unmasked_count == 1) as u32;
    let b2 = (faultmask_after == 0) as u32; // auto-cleared on return
    let b3 = (SYSTICK_COUNT.load(Ordering::Relaxed) == 2) as u32; // handler ran again
    let nibble = b0 | (b1 << 1) | (b2 << 2) | (b3 << 3);
    digest(IDX_FAULTMASK, (0xF << 4) | nibble);
}

// ── Check 7: MSP/PSP banking + EXC_RETURN (#354) ────────────────────────────────
//
// Set PSP to a private stack, switch CONTROL.SPSEL=1 (thread now uses PSP), prove
// MSP and PSP read distinct, take `svc #0` from thread/PSP — the frame stacks on
// PSP and the handler captures EXC_RETURN=0xFFFFFFFD — then confirm PSP is fully
// restored (frame popped from PSP, not MSP) and return to MSP.
//
// Fold = (EXC_RETURN nibble 0xD << 4) | bool_nibble; all-correct → 0x000000DF.
unsafe fn check_msp_psp() {
    EXC_RETURN_SEEN.store(0, Ordering::Relaxed);
    let psp_top = (addr_of_mut!(PSP_STACK) as u32) + (PSP_STACK_WORDS as u32) * 4;

    let msp_before: u32;
    let psp_rb: u32;
    let psp_after: u32;
    core::arch::asm!(
        "msr psp, {top}",          // set PSP to our private stack
        "mrs {msp}, msp",          // capture MSP (distinct from PSP)
        "mrs {rb}, psp",           // read PSP back (== top)
        "movs {tmp}, #2",          // CONTROL: SPSEL=1, nPRIV=0
        "msr control, {tmp}",
        "isb",
        "svc #0",                  // exception from thread/PSP → frame on PSP, EXC_RETURN=0xFFFFFFFD
        "mrs {after}, psp",        // PSP after return (== top iff popped from PSP)
        "movs {tmp}, #0",          // CONTROL: SPSEL=0 → back to MSP thread
        "msr control, {tmp}",
        "isb",
        top = in(reg) psp_top,
        msp = out(reg) msp_before,
        rb = out(reg) psp_rb,
        after = out(reg) psp_after,
        tmp = out(reg) _,
        options(nostack),
    );

    let exc_return = EXC_RETURN_SEEN.load(Ordering::Relaxed);
    let b0 = (psp_rb == psp_top) as u32; // PSP banked the written value
    let b1 = (msp_before != psp_top) as u32; // MSP and PSP are distinct banks
    let b2 = (exc_return == 0xFFFF_FFFD) as u32; // EXC_RETURN thread/PSP
    let b3 = (psp_after == psp_top) as u32; // frame popped from PSP on return
    let nibble = b0 | (b1 << 1) | (b2 << 2) | (b3 << 3);
    digest(IDX_MSP_PSP, (0xD << 4) | nibble);
}

// ── Check 8: ICSR write-only / self-clearing PENDSV (#358) ──────────────────────
//
// Reproduce the Zephyr arch_swap RMW. PENDSVSET pends PendSV and self-clears on
// take; a written PENDSVCLR must read back 0 (write-only); and the
// `v = ICSR; ICSR = v | PENDSVSET` RMW must re-pend exactly once — a stale
// PENDSVCLR riding along in the read must NOT cancel the new set.
//
// Fold = (TAG 0xC << 4) | bool_nibble; all-correct → 0x000000CF.
unsafe fn check_icsr() {
    PENDSV_COUNT.store(0, Ordering::Relaxed);

    // Pend PendSV (BASEPRI/FAULTMASK are clear here) — it runs once.
    wr(ICSR, ICSR_PENDSVSET);
    nops();
    let count_after_set = PENDSV_COUNT.load(Ordering::Relaxed);
    let set_selfcleared = (rd(ICSR) & ICSR_PENDSVSET) == 0; // self-cleared on take

    // Write PENDSVCLR — it is write-only/self-clearing; must read back 0.
    wr(ICSR, ICSR_PENDSVCLR);
    let clr_reads_zero = (rd(ICSR) & (ICSR_PENDSVSET | ICSR_PENDSVCLR)) == 0;

    // The poisoned RMW: read ICSR (must NOT carry a stale PENDSVCLR), OR in
    // PENDSVSET, write it back. PendSV must fire exactly once more.
    let v = rd(ICSR);
    wr(ICSR, v | ICSR_PENDSVSET);
    nops();
    let count_after_rmw = PENDSV_COUNT.load(Ordering::Relaxed);

    let b0 = (count_after_set == 1) as u32;
    let b1 = set_selfcleared as u32;
    let b2 = clr_reads_zero as u32;
    let b3 = (count_after_rmw == 2) as u32; // RMW re-pended, stale CLR did not cancel
    let nibble = b0 | (b1 << 1) | (b2 << 2) | (b3 << 3);
    digest(IDX_ICSR, (0xC << 4) | nibble);
}

// ── small helpers ──────────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn set_basepri(v: u32) {
    core::arch::asm!("msr basepri, {0}", in(reg) v, options(nomem, nostack, preserves_flags));
}
#[inline(always)]
unsafe fn get_basepri() -> u32 {
    let v: u32;
    core::arch::asm!("mrs {0}, basepri", out(reg) v, options(nomem, nostack, preserves_flags));
    v
}
#[inline(always)]
unsafe fn set_basepri_max(v: u32) {
    core::arch::asm!("msr basepri_max, {0}", in(reg) v, options(nomem, nostack, preserves_flags));
}
#[inline(always)]
unsafe fn get_faultmask() -> u32 {
    let v: u32;
    core::arch::asm!("mrs {0}, faultmask", out(reg) v, options(nomem, nostack, preserves_flags));
    v & 1
}
#[inline(always)]
fn nops() {
    // Enough cycles for a pending-and-unmasked exception to be taken.
    for _ in 0..8 {
        cortex_m::asm::nop();
    }
}

#[inline(never)]
fn halt_forever() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
