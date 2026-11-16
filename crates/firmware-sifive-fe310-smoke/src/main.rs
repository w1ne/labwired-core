// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SiFive FE310 smoke: RV32IM execution, M-mode ecall/ebreak and MRET status
//! paths, CLINT machine timer and software interrupts, the PLIC routing the
//! uart0 interrupt to MEIP, UART0 TX, and HiFive1 Rev B GPIO.
//!
//! The ELF is loaded into the FE310's QSPI XIP window and entered directly.
//! It checks the imported system's RISC-V core, uart0, and GPIO software path
//! used by the board's active-low green LED. The trap probes check the
//! architectural synchronous-trap/mret paths. The CLINT probes check the
//! `mtime` rate against the 32 768 Hz lfclk, an MTIP wake from WFI, an MSIP
//! interrupt taken after the `msip` store, and MSIP held pending while
//! `mstatus.MIE` is clear. The PLIC probes raise uart0's TX-watermark
//! interrupt (source 3) and check pending, enable, priority and threshold
//! gating of MEIP, then take the external interrupt and claim/complete it,
//! and check that a complete re-pends the source only while its line is high.
//! The GPIO interrupt probes drive pads through the output loopback: a rising
//! edge on pin 2 pends PLIC source 10 and is taken as a trap, a falling edge on
//! pin 31 pends source 39 and is claimed by polling, and HIGH_IP re-latches
//! while the level holds.
//! It does not exercise the mask-ROM boot flow, GPIO IO functions, the uart0
//! RX watermark or uart1.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

#[no_mangle]
#[used]
static mut TRAP_COUNT: u32 = 0;
#[no_mangle]
#[used]
static mut TRAP_MCAUSE: u32 = 0;
#[no_mangle]
#[used]
static mut TRAP_MEPC: u32 = 0;
#[no_mangle]
#[used]
static mut TRAP_MSTATUS: u32 = 0;
#[no_mangle]
#[used]
static mut TRAP_CLAIM: u32 = 0;

core::arch::global_asm!(
    ".section .text.trap_entry, \"ax\"",
    ".align 2",
    ".global trap_entry",
    ".type trap_entry, @function",
    "trap_entry:",
    "csrr t0, mcause",
    "la t1, TRAP_MCAUSE",
    "sw t0, 0(t1)",
    "csrr t0, mepc",
    "la t1, TRAP_MEPC",
    "sw t0, 0(t1)",
    "csrr t0, mstatus",
    "la t1, TRAP_MSTATUS",
    "sw t0, 0(t1)",
    "la t1, TRAP_COUNT",
    "lw t0, 0(t1)",
    "addi t0, t0, 1",
    "sw t0, 0(t1)",
    "csrr t0, mcause",
    "bgez t0, 1f",
    // External interrupt: claim, mask uart0's interrupt enables, clear the
    // GPIO RISE/FALL pending latches, complete.
    "li t1, 0x8000000b",
    "bne t0, t1, 3f",
    "li t1, 0x0C200004",
    "lw t0, 0(t1)",
    "la t1, TRAP_CLAIM",
    "sw t0, 0(t1)",
    "li t1, 0x10013010",
    "sw zero, 0(t1)",
    "li t1, 0x10012000",
    "li t2, -1",
    "sw t2, 0x1C(t1)",
    "sw t2, 0x24(t1)",
    "li t1, 0x0C200004",
    "sw t0, 0(t1)",
    "mret",
    // Other interrupts: clear msip, disarm mtimecmp (high word first).
    "3:",
    "li t1, 0x02000000",
    "sw zero, 0(t1)",
    "li t0, -1",
    "li t1, 0x02004000",
    "sw t0, 4(t1)",
    "sw t0, 0(t1)",
    "mret",
    "1:",
    "csrr t0, mepc",
    "addi t0, t0, 4",
    "csrw mepc, t0",
    "mret",
    ".size trap_entry, .-trap_entry",
);

/// FE310 uart0 at 0x10013000: txdata@+0x00, with bit 31 indicating full.
const UART0_TXDATA: *mut u32 = 0x1001_3000 as *mut u32;
const GPIO_INPUT_VAL: *mut u32 = 0x1001_2000 as *mut u32;
const GPIO_INPUT_EN: *mut u32 = 0x1001_2004 as *mut u32;
const GPIO_OUTPUT_EN: *mut u32 = 0x1001_2008 as *mut u32;
const GPIO_OUTPUT_VAL: *mut u32 = 0x1001_200C as *mut u32;
const GREEN_LED: u32 = 1 << 19;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART0_TXDATA) & (1 << 31) != 0 {}
        core::ptr::write_volatile(UART0_TXDATA, byte as u32);
    }
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

fn put_hex32(value: u32) {
    for shift in (0..8).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

/// Trigger a synchronous M-mode environment call and verify trap entry and
/// return. The handler stores architectural CSRs in SRAM and advances mepc
/// past the four-byte ecall before mret.
fn ecall_trap_probe() -> bool {
    let ecall_pc: u32;
    let returned_mstatus: u32;
    unsafe {
        core::arch::asm!(
            "la t0, trap_entry",
            "csrw mtvec, t0",
            "csrs mstatus, {mie}",
            "la {ecall_pc}, 1f",
            "1: ecall",
            "csrr {returned_mstatus}, mstatus",
            "csrc mstatus, {mie}",
            mie = in(reg) 1u32 << 3,
            ecall_pc = lateout(reg) ecall_pc,
            returned_mstatus = lateout(reg) returned_mstatus,
            out("t0") _,
            out("t1") _,
            options(nostack),
        );
    }

    let (count, cause, mepc, trap_mstatus) = unsafe {
        (
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MCAUSE)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MEPC)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MSTATUS)),
        )
    };
    let pass = count == 1
        && cause == 0x0000_000B
        && mepc == ecall_pc
        && trap_mstatus & 0x1888 == 0x1880
        && returned_mstatus & (1 << 3) != 0;

    puts("ecall trap mcause=");
    put_hex32(cause);
    puts(" mepc_match=");
    putc(if mepc == ecall_pc { b'1' } else { b'0' });
    puts(" count=");
    put_hex32(count);
    puts(" entry_mie=");
    putc(if trap_mstatus & (1 << 3) == 0 {
        b'0'
    } else {
        b'1'
    });
    puts(" entry_mpie=");
    putc(if trap_mstatus & (1 << 7) != 0 {
        b'1'
    } else {
        b'0'
    });
    puts(" entry_mpp=");
    put_hex32((trap_mstatus >> 11) & 3);
    puts(" return_mie=");
    putc(if returned_mstatus & (1 << 3) != 0 {
        b'1'
    } else {
        b'0'
    });
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// Trigger a breakpoint exception and verify that MRET resumes at the next
/// instruction exactly once. The handler records the faulting PC before
/// advancing mepc by the four-byte EBREAK instruction.
fn ebreak_trap_probe() -> bool {
    let ebreak_pc: u32;
    let resumed: u32;
    let returned_mstatus: u32;
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_COUNT), 0);
        core::arch::asm!(
            "la t0, trap_entry",
            "csrw mtvec, t0",
            "csrs mstatus, {mie}",
            "la {ebreak_pc}, 1f",
            "1: ebreak",
            "li {resumed}, 1",
            "csrr {returned_mstatus}, mstatus",
            "csrc mstatus, {mie}",
            mie = in(reg) 1u32 << 3,
            ebreak_pc = lateout(reg) ebreak_pc,
            resumed = lateout(reg) resumed,
            returned_mstatus = lateout(reg) returned_mstatus,
            out("t0") _,
            out("t1") _,
            options(nostack),
        );
    }

    let (count, cause, mepc, trap_mstatus) = unsafe {
        (
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MCAUSE)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MEPC)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MSTATUS)),
        )
    };
    let pass = count == 1
        && cause == 3
        && mepc == ebreak_pc
        && resumed == 1
        && trap_mstatus & 0x1888 == 0x1880
        && returned_mstatus & (1 << 3) != 0;

    puts("ebreak trap mcause=");
    put_hex32(cause);
    puts(" mepc_match=");
    putc(if mepc == ebreak_pc { b'1' } else { b'0' });
    puts(" count=");
    put_hex32(count);
    puts(" resumed=");
    putc(if resumed == 1 { b'1' } else { b'0' });
    puts(" entry_mie=");
    putc(if trap_mstatus & (1 << 3) == 0 {
        b'0'
    } else {
        b'1'
    });
    puts(" entry_mpie=");
    putc(if trap_mstatus & (1 << 7) != 0 {
        b'1'
    } else {
        b'0'
    });
    puts(" return_mie=");
    putc(if returned_mstatus & (1 << 3) != 0 {
        b'1'
    } else {
        b'0'
    });
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// Enter an M-mode ecall with MIE clear and check that MRET keeps interrupts
/// disabled while setting MPIE back to one.
fn ecall_masked_mret_probe() -> bool {
    let ecall_pc: u32;
    let returned_mstatus: u32;
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_COUNT), 0);
        core::arch::asm!(
            "la t0, trap_entry",
            "csrw mtvec, t0",
            "csrc mstatus, {mie}",
            "la {ecall_pc}, 1f",
            "1: ecall",
            "csrr {returned_mstatus}, mstatus",
            "csrc mstatus, {mie}",
            mie = in(reg) 1u32 << 3,
            ecall_pc = lateout(reg) ecall_pc,
            returned_mstatus = lateout(reg) returned_mstatus,
            out("t0") _,
            out("t1") _,
            options(nostack),
        );
    }

    let (count, cause, mepc, trap_mstatus) = unsafe {
        (
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MCAUSE)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MEPC)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MSTATUS)),
        )
    };
    let entry_mie_clear = trap_mstatus & (1 << 3) == 0;
    let entry_mpie_clear = trap_mstatus & (1 << 7) == 0;
    let return_mie_clear = returned_mstatus & (1 << 3) == 0;
    let return_mpie_set = returned_mstatus & (1 << 7) != 0;
    let pass = count == 1
        && cause == 0x0000_000B
        && mepc == ecall_pc
        && entry_mie_clear
        && entry_mpie_clear
        && return_mie_clear
        && return_mpie_set;

    puts("ecall masked mret entry_mie=");
    putc(if entry_mie_clear { b'0' } else { b'1' });
    puts(" entry_mpie=");
    putc(if entry_mpie_clear { b'0' } else { b'1' });
    puts(" return_mie=");
    putc(if return_mie_clear { b'0' } else { b'1' });
    puts(" return_mpie=");
    putc(if return_mpie_set { b'1' } else { b'0' });
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

const CLINT_MSIP: *mut u32 = 0x0200_0000 as *mut u32;
const CLINT_MTIMECMP_LO: *mut u32 = 0x0200_4000 as *mut u32;
const CLINT_MTIMECMP_HI: *mut u32 = 0x0200_4004 as *mut u32;
const CLINT_MTIME_LO: *const u32 = 0x0200_BFF8 as *const u32;
const CLINT_MTIME_HI: *const u32 = 0x0200_BFFC as *const u32;
const MIP_MSIP: u32 = 1 << 3;
const MIP_MTIP: u32 = 1 << 7;

fn rdcycle() -> u32 {
    let value: u32;
    unsafe { core::arch::asm!("csrr {0}, cycle", out(reg) value, options(nomem, nostack)) };
    value
}

fn read_mip() -> u32 {
    let value: u32;
    unsafe { core::arch::asm!("csrr {0}, mip", out(reg) value, options(nomem, nostack)) };
    value
}

fn flag(name: &str, value: bool) {
    puts(name);
    putc(if value { b'1' } else { b'0' });
}

fn reset_trap_record() {
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_COUNT), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_MCAUSE), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_MEPC), 0);
    }
}

fn trap_record() -> (u32, u32, u32) {
    unsafe {
        (
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_COUNT)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MCAUSE)),
            core::ptr::read_volatile(core::ptr::addr_of!(TRAP_MEPC)),
        )
    }
}

/// `mtimecmp` resets disarmed, and `mtime` advances once per lfclk period:
/// 62 MHz / 32 768 Hz = 1892.09 CPU clocks per tick. The poll loop adds at
/// most a few clocks of error per edge.
fn clint_rate_probe() -> bool {
    let (cmp_lo, cmp_hi) = unsafe {
        (
            core::ptr::read_volatile(CLINT_MTIMECMP_LO),
            core::ptr::read_volatile(CLINT_MTIMECMP_HI),
        )
    };
    // Start on a tick edge so the window holds whole lfclk periods.
    let first = unsafe { core::ptr::read_volatile(CLINT_MTIME_LO) };
    let mut t0 = first;
    while t0 == first {
        t0 = unsafe { core::ptr::read_volatile(CLINT_MTIME_LO) };
    }
    let c0 = rdcycle();
    let mut t1 = t0;
    while t1.wrapping_sub(t0) < 16 {
        t1 = unsafe { core::ptr::read_volatile(CLINT_MTIME_LO) };
    }
    let c1 = rdcycle();
    let hi = unsafe { core::ptr::read_volatile(CLINT_MTIME_HI) };
    let ticks = t1.wrapping_sub(t0);
    let clocks = c1.wrapping_sub(c0);
    let per_tick = clocks / ticks;
    let disarmed = cmp_lo == u32::MAX && cmp_hi == u32::MAX;
    let rate = (1890..=1894).contains(&per_tick);
    let pass = disarmed && rate && hi == 0 && read_mip() & MIP_MTIP == 0;
    puts("clint mtimecmp=");
    put_hex32(cmp_hi);
    put_hex32(cmp_lo);
    puts(" ticks=");
    put_hex32(ticks);
    puts(" clocks_per_tick=");
    put_hex32(per_tick);
    flag(" lfclk_rate=", rate);
    flag(" mtip_low=", read_mip() & MIP_MTIP == 0);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// Arm `mtimecmp` four lfclk ticks ahead and wait in WFI with MTIE enabled.
fn clint_mtip_probe() -> bool {
    reset_trap_record();
    let start = unsafe { core::ptr::read_volatile(CLINT_MTIME_LO) };
    let target = start.wrapping_add(4);
    let early_mip;
    unsafe {
        core::ptr::write_volatile(CLINT_MTIMECMP_LO, u32::MAX);
        core::ptr::write_volatile(CLINT_MTIMECMP_HI, 0);
        core::ptr::write_volatile(CLINT_MTIMECMP_LO, target);
        early_mip = read_mip();
        core::arch::asm!(
            "la t0, trap_entry",
            "csrw mtvec, t0",
            "csrs mie, {mtie}",
            "csrs mstatus, {mie}",
            // WFI is a hint: loop until the handler has run.
            "2: wfi",
            "la t1, TRAP_COUNT",
            "lw t0, 0(t1)",
            "beqz t0, 2b",
            "csrc mstatus, {mie}",
            "csrc mie, {mtie}",
            mtie = in(reg) MIP_MTIP,
            mie = in(reg) 1u32 << 3,
            out("t0") _,
            out("t1") _,
            options(nostack),
        );
    }
    let woke = unsafe { core::ptr::read_volatile(CLINT_MTIME_LO) };
    let (count, cause, _) = trap_record();
    let disarmed = unsafe { core::ptr::read_volatile(CLINT_MTIMECMP_HI) } == u32::MAX;
    let reached = woke.wrapping_sub(start) >= 4;
    let pass = early_mip & MIP_MTIP == 0
        && count == 1
        && cause == 0x8000_0007
        && reached
        && disarmed
        && read_mip() & MIP_MTIP == 0;
    puts("clint mtip mcause=");
    put_hex32(cause);
    puts(" count=");
    put_hex32(count);
    flag(" armed_low=", early_mip & MIP_MTIP == 0);
    flag(" wfi_wake=", reached);
    flag(" disarmed=", disarmed);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// `msip = 1` with MSIE and MIE set traps after the store, at the next PC.
fn clint_msip_probe() -> bool {
    reset_trap_record();
    let after_store: u32;
    unsafe {
        core::arch::asm!(
            "la t0, trap_entry",
            "csrw mtvec, t0",
            "csrs mie, {msie}",
            "csrs mstatus, {mie}",
            "li t1, 1",
            "sw t1, 0({msip})",
            "1: nop",
            "csrc mstatus, {mie}",
            "csrc mie, {msie}",
            "la {after}, 1b",
            msie = in(reg) MIP_MSIP,
            mie = in(reg) 1u32 << 3,
            msip = in(reg) CLINT_MSIP,
            after = lateout(reg) after_store,
            out("t0") _,
            out("t1") _,
            options(nostack),
        );
    }
    let (count, cause, mepc) = trap_record();
    let cleared = unsafe { core::ptr::read_volatile(CLINT_MSIP) } == 0;
    let pass = count == 1
        && cause == 0x8000_0003
        && mepc == after_store
        && cleared
        && read_mip() & MIP_MSIP == 0;
    puts("clint msip mcause=");
    put_hex32(cause);
    puts(" count=");
    put_hex32(count);
    flag(" mepc_next=", mepc == after_store);
    flag(" cleared=", cleared);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// With `mstatus.MIE` clear, `msip` only sets `mip.MSIP`; clearing it drops
/// the bit without a trap.
fn clint_msip_masked_probe() -> bool {
    reset_trap_record();
    let (pending, released) = unsafe {
        core::ptr::write_volatile(CLINT_MSIP, 1);
        let pending = read_mip() & MIP_MSIP != 0;
        core::ptr::write_volatile(CLINT_MSIP, 0);
        (pending, read_mip() & MIP_MSIP == 0)
    };
    let (count, _, _) = trap_record();
    let pass = pending && released && count == 0;
    flag("clint msip masked pending=", pending);
    flag(" released=", released);
    puts(" traps=");
    put_hex32(count);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

const PLIC_UART0_PRIORITY: *mut u32 = 0x0C00_000C as *mut u32;
const PLIC_PENDING: *const u32 = 0x0C00_1000 as *const u32;
const PLIC_ENABLE: *mut u32 = 0x0C00_2000 as *mut u32;
const PLIC_THRESHOLD: *mut u32 = 0x0C20_0000 as *mut u32;
const PLIC_CLAIM: *mut u32 = 0x0C20_0004 as *mut u32;
const UART0_TXCTRL: *mut u32 = 0x1001_3008 as *mut u32;
const UART0_IE: *mut u32 = 0x1001_3010 as *mut u32;
const UART0_IP: *const u32 = 0x1001_3014 as *const u32;
const UART0_SOURCE: u32 = 3;
const MIP_MEIP: u32 = 1 << 11;

/// Poll `mip` until MEIP reaches `level`, for at most `tries` reads.
fn wait_meip(level: bool, tries: u32) -> bool {
    for _ in 0..tries {
        if (read_mip() & MIP_MEIP != 0) == level {
            return true;
        }
    }
    false
}

/// Arm uart0's TX watermark (`txcnt = 1`: the empty FIFO is below it) with
/// `mstatus.MIE` clear. The PLIC latches source 3, and MEIP follows the
/// enable, priority and threshold gates without a trap.
fn plic_uart0_masked_probe() -> bool {
    reset_trap_record();
    let (ip, pending, disabled_low, meip, masked, unmasked, prio_warl) = unsafe {
        core::ptr::write_volatile(PLIC_THRESHOLD, 0);
        core::ptr::write_volatile(PLIC_UART0_PRIORITY, 1);
        core::ptr::write_volatile(UART0_TXCTRL, 1 | (1 << 16));
        core::ptr::write_volatile(UART0_IE, 1);
        let ip = core::ptr::read_volatile(UART0_IP);
        // The source latches in the gateway while its enable is clear.
        let mut pending = false;
        for _ in 0..1000 {
            if core::ptr::read_volatile(PLIC_PENDING) & (1 << UART0_SOURCE) != 0 {
                pending = true;
                break;
            }
        }
        let disabled_low = read_mip() & MIP_MEIP == 0;
        core::ptr::write_volatile(PLIC_ENABLE, 1 << UART0_SOURCE);
        let meip = wait_meip(true, 1000);
        core::ptr::write_volatile(PLIC_THRESHOLD, 1);
        let masked = read_mip() & MIP_MEIP == 0;
        core::ptr::write_volatile(PLIC_THRESHOLD, 0);
        let unmasked = read_mip() & MIP_MEIP != 0;
        core::ptr::write_volatile(PLIC_UART0_PRIORITY, 0xFF);
        let prio_warl = core::ptr::read_volatile(PLIC_UART0_PRIORITY) == 7;
        core::ptr::write_volatile(PLIC_UART0_PRIORITY, 1);
        (ip, pending, disabled_low, meip, masked, unmasked, prio_warl)
    };
    let (count, _, _) = trap_record();
    let pass = ip & 1 != 0
        && pending
        && disabled_low
        && meip
        && masked
        && unmasked
        && prio_warl
        && count == 0;
    puts("plic uart0 ip=");
    put_hex32(ip);
    flag(" pending=", pending);
    flag(" disabled_low=", disabled_low);
    flag(" meip=", meip);
    flag(" threshold_masks=", masked);
    flag(" unmasked=", unmasked);
    flag(" prio_warl=", prio_warl);
    puts(" traps=");
    put_hex32(count);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// Take the pending uart0 interrupt as a machine external interrupt. The
/// handler claims source 3, clears uart0 `ie` and completes; afterwards the
/// source is neither pending nor claimable and MEIP is low.
fn plic_uart0_trap_probe() -> bool {
    reset_trap_record();
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_CLAIM), 0);
        core::arch::asm!(
            "la t0, trap_entry",
            "csrw mtvec, t0",
            "csrs mie, {meie}",
            "csrs mstatus, {mie}",
            "2: la t1, TRAP_COUNT",
            "lw t0, 0(t1)",
            "beqz t0, 2b",
            "csrc mstatus, {mie}",
            "csrc mie, {meie}",
            meie = in(reg) MIP_MEIP,
            mie = in(reg) 1u32 << 3,
            out("t0") _,
            out("t1") _,
            out("t2") _,
            options(nostack),
        );
    }
    let (count, cause, _) = trap_record();
    let claim = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_CLAIM)) };
    let (ie, pending, reclaim) = unsafe {
        (
            core::ptr::read_volatile(UART0_IE),
            core::ptr::read_volatile(PLIC_PENDING) & (1 << UART0_SOURCE),
            core::ptr::read_volatile(PLIC_CLAIM),
        )
    };
    let meip_low = read_mip() & MIP_MEIP == 0;
    unsafe { core::ptr::write_volatile(PLIC_ENABLE, 0) };
    let pass = count == 1
        && cause == 0x8000_000B
        && claim == UART0_SOURCE
        && ie == 0
        && pending == 0
        && reclaim == 0
        && meip_low;
    puts("plic uart0 mcause=");
    put_hex32(cause);
    puts(" count=");
    put_hex32(count);
    puts(" claim=");
    put_hex32(claim);
    flag(" ie_cleared=", ie == 0);
    flag(" pending_clear=", pending == 0);
    puts(" reclaim=");
    put_hex32(reclaim);
    flag(" meip_low=", meip_low);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// Claim and complete uart0's level TX-watermark source by polling, with
/// `mstatus.MIE` clear. A complete while the line is still high re-pends
/// source 3 before the next instruction; a complete after uart0 `ie` drops
/// the line leaves it neither pending nor claimable.
fn plic_uart0_level_repend_probe() -> bool {
    let (first, repend, second, low_pending, settled, reclaim) = unsafe {
        core::ptr::write_volatile(PLIC_ENABLE, 1 << UART0_SOURCE);
        core::ptr::write_volatile(UART0_IE, 1);
        let _ = wait_bit(PLIC_PENDING, 1 << UART0_SOURCE, 1000);
        let first = core::ptr::read_volatile(PLIC_CLAIM);
        core::ptr::write_volatile(PLIC_CLAIM, first);
        let repend = core::ptr::read_volatile(PLIC_PENDING) & (1 << UART0_SOURCE) != 0;
        let second = core::ptr::read_volatile(PLIC_CLAIM);
        core::ptr::write_volatile(UART0_IE, 0);
        core::ptr::write_volatile(PLIC_CLAIM, second);
        let low_pending = core::ptr::read_volatile(PLIC_PENDING) & (1 << UART0_SOURCE) != 0;
        let settled = !wait_bit(PLIC_PENDING, 1 << UART0_SOURCE, 1000);
        let reclaim = core::ptr::read_volatile(PLIC_CLAIM);
        core::ptr::write_volatile(PLIC_ENABLE, 0);
        (first, repend, second, low_pending, settled, reclaim)
    };
    let pass = first == UART0_SOURCE
        && repend
        && second == UART0_SOURCE
        && !low_pending
        && settled
        && reclaim == 0;
    puts("plic uart0 level claim=");
    put_hex32(first);
    flag(" repend=", repend);
    puts(" claim2=");
    put_hex32(second);
    flag(" low_no_repend=", !low_pending && settled);
    puts(" reclaim=");
    put_hex32(reclaim);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

const GPIO_RISE_IE: *mut u32 = 0x1001_2018 as *mut u32;
const GPIO_RISE_IP: *mut u32 = 0x1001_201C as *mut u32;
const GPIO_FALL_IE: *mut u32 = 0x1001_2020 as *mut u32;
const GPIO_FALL_IP: *mut u32 = 0x1001_2024 as *mut u32;
const GPIO_HIGH_IP: *mut u32 = 0x1001_202C as *mut u32;
const PLIC_GPIO2_PRIORITY: *mut u32 = 0x0C00_0028 as *mut u32;
const PLIC_GPIO31_PRIORITY: *mut u32 = 0x0C00_009C as *mut u32;
const PLIC_PENDING_HI: *const u32 = 0x0C00_1004 as *const u32;
const PLIC_ENABLE_HI: *mut u32 = 0x0C00_2004 as *mut u32;
/// FE310 GPIO pin `n` drives PLIC source 8 + n.
const GPIO2: u32 = 1 << 2;
const GPIO2_SOURCE: u32 = 10;
const GPIO31: u32 = 1 << 31;
const GPIO31_SOURCE: u32 = 39;

/// Poll `reg` for at most `tries` reads until `reg & mask` is non-zero.
fn wait_bit(reg: *const u32, mask: u32, tries: u32) -> bool {
    for _ in 0..tries {
        if unsafe { core::ptr::read_volatile(reg) } & mask != 0 {
            return true;
        }
    }
    false
}

/// With `mstatus.MIE` clear, a rising edge on output-looped pin 2 latches
/// RISE_IP (not FALL_IP), pends PLIC source 10 while its enable is clear,
/// and raises MEIP once enabled. HIGH_IP re-latches after a write of 1
/// because the pin is still high.
fn gpio_irq_rise_masked_probe() -> bool {
    reset_trap_record();
    let (early, rise, fall, pending, disabled_low, meip, high_relatch) = unsafe {
        core::ptr::write_volatile(PLIC_THRESHOLD, 0);
        core::ptr::write_volatile(PLIC_GPIO2_PRIORITY, 1);
        core::ptr::write_volatile(GPIO_OUTPUT_VAL, 0);
        core::ptr::write_volatile(GPIO_INPUT_EN, GPIO2 | GPIO31);
        core::ptr::write_volatile(GPIO_OUTPUT_EN, GPIO2 | GPIO31);
        core::ptr::write_volatile(GPIO_RISE_IP, u32::MAX);
        core::ptr::write_volatile(GPIO_FALL_IP, u32::MAX);
        core::ptr::write_volatile(GPIO_RISE_IE, GPIO2);
        // No edge yet: the source stays idle.
        let early = wait_bit(PLIC_PENDING, 1 << GPIO2_SOURCE, 200);
        core::ptr::write_volatile(GPIO_OUTPUT_VAL, GPIO2);
        let rise = core::ptr::read_volatile(GPIO_RISE_IP);
        let fall = core::ptr::read_volatile(GPIO_FALL_IP);
        let pending = wait_bit(PLIC_PENDING, 1 << GPIO2_SOURCE, 1000);
        let disabled_low = read_mip() & MIP_MEIP == 0;
        core::ptr::write_volatile(PLIC_ENABLE, 1 << GPIO2_SOURCE);
        let meip = wait_meip(true, 1000);
        core::ptr::write_volatile(GPIO_HIGH_IP, GPIO2);
        let high_relatch = core::ptr::read_volatile(GPIO_HIGH_IP) & GPIO2 != 0;
        (early, rise, fall, pending, disabled_low, meip, high_relatch)
    };
    let (count, _, _) = trap_record();
    let pass = !early
        && rise == GPIO2
        && fall == 0
        && pending
        && disabled_low
        && meip
        && high_relatch
        && count == 0;
    flag("gpio irq rise early=", early);
    puts(" rise_ip=");
    put_hex32(rise);
    puts(" fall_ip=");
    put_hex32(fall);
    flag(" pending=", pending);
    flag(" disabled_low=", disabled_low);
    flag(" meip=", meip);
    flag(" high_relatch=", high_relatch);
    puts(" traps=");
    put_hex32(count);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// Take the pin-2 rising-edge interrupt as a machine external interrupt. The
/// handler claims source 10, clears RISE_IP and completes; the source does
/// not re-pend and MEIP is low.
fn gpio_irq_rise_trap_probe() -> bool {
    reset_trap_record();
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_CLAIM), 0);
        core::arch::asm!(
            "la t0, trap_entry",
            "csrw mtvec, t0",
            "csrs mie, {meie}",
            "csrs mstatus, {mie}",
            "2: la t1, TRAP_COUNT",
            "lw t0, 0(t1)",
            "beqz t0, 2b",
            "csrc mstatus, {mie}",
            "csrc mie, {meie}",
            meie = in(reg) MIP_MEIP,
            mie = in(reg) 1u32 << 3,
            out("t0") _,
            out("t1") _,
            out("t2") _,
            options(nostack),
        );
    }
    let (count, cause, _) = trap_record();
    let claim = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_CLAIM)) };
    let (rise, repended, reclaim) = unsafe {
        let rise = core::ptr::read_volatile(GPIO_RISE_IP);
        let repended = wait_bit(PLIC_PENDING, 1 << GPIO2_SOURCE, 200);
        (rise, repended, core::ptr::read_volatile(PLIC_CLAIM))
    };
    let meip_low = read_mip() & MIP_MEIP == 0;
    unsafe {
        core::ptr::write_volatile(PLIC_ENABLE, 0);
        core::ptr::write_volatile(GPIO_RISE_IE, 0);
    }
    let pass = count == 1
        && cause == 0x8000_000B
        && claim == GPIO2_SOURCE
        && rise == 0
        && !repended
        && reclaim == 0
        && meip_low;
    puts("gpio irq rise mcause=");
    put_hex32(cause);
    puts(" count=");
    put_hex32(count);
    puts(" claim=");
    put_hex32(claim);
    puts(" rise_ip=");
    put_hex32(rise);
    flag(" repended=", repended);
    puts(" reclaim=");
    put_hex32(reclaim);
    flag(" meip_low=", meip_low);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// Pin 31 drives source 39, in the PLIC's second pending/enable word. A
/// rising edge with only FALL_IE set pends nothing; the falling edge pends
/// source 39, which a polled claim returns. Clearing FALL_IP before the
/// complete leaves MEIP low.
fn gpio_irq_fall_polled_probe() -> bool {
    reset_trap_record();
    let (rise_quiet, fall, meip, claim, claimed_clear, meip_low, reclaim) = unsafe {
        core::ptr::write_volatile(PLIC_GPIO31_PRIORITY, 2);
        core::ptr::write_volatile(PLIC_ENABLE_HI, 1 << (GPIO31_SOURCE - 32));
        core::ptr::write_volatile(GPIO_FALL_IE, GPIO31);
        // Pin 31 rises; pin 2 (still looped back) falls. Drop that latch.
        core::ptr::write_volatile(GPIO_OUTPUT_VAL, GPIO31);
        core::ptr::write_volatile(GPIO_FALL_IP, u32::MAX);
        let rise_quiet = !wait_bit(PLIC_PENDING_HI, 1 << (GPIO31_SOURCE - 32), 200);
        core::ptr::write_volatile(GPIO_OUTPUT_VAL, 0);
        let fall = core::ptr::read_volatile(GPIO_FALL_IP);
        let meip = wait_meip(true, 1000);
        let claim = core::ptr::read_volatile(PLIC_CLAIM);
        let claimed_clear =
            core::ptr::read_volatile(PLIC_PENDING_HI) & (1 << (GPIO31_SOURCE - 32)) == 0;
        core::ptr::write_volatile(GPIO_FALL_IP, GPIO31);
        core::ptr::write_volatile(PLIC_CLAIM, claim);
        let meip_low = !wait_meip(true, 200);
        let reclaim = core::ptr::read_volatile(PLIC_CLAIM);
        core::ptr::write_volatile(PLIC_ENABLE_HI, 0);
        core::ptr::write_volatile(GPIO_FALL_IE, 0);
        core::ptr::write_volatile(GPIO_OUTPUT_EN, 0);
        core::ptr::write_volatile(GPIO_INPUT_EN, 0);
        (
            rise_quiet,
            fall,
            meip,
            claim,
            claimed_clear,
            meip_low,
            reclaim,
        )
    };
    let (count, _, _) = trap_record();
    let pass = rise_quiet
        && fall == GPIO31
        && meip
        && claim == GPIO31_SOURCE
        && claimed_clear
        && meip_low
        && reclaim == 0
        && count == 0;
    flag("gpio irq fall rise_quiet=", rise_quiet);
    puts(" fall_ip=");
    put_hex32(fall);
    flag(" meip=", meip);
    puts(" claim=");
    put_hex32(claim);
    flag(" claimed_clear=", claimed_clear);
    flag(" meip_low=", meip_low);
    puts(" reclaim=");
    put_hex32(reclaim);
    puts(" traps=");
    put_hex32(count);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

#[inline(never)]
fn multiply(left: u32, right: u32) -> u32 {
    let result: u32;
    unsafe {
        core::arch::asm!(
            "mul {result}, {left}, {right}",
            result = lateout(reg) result,
            left = in(reg) left,
            right = in(reg) right,
            options(nomem, nostack, preserves_flags),
        );
    }
    result
}

#[entry]
fn main() -> ! {
    puts("SIFIVE FE310 SMOKE\n");
    puts("cpu=rv32im uart=sifive\n");
    puts("mul=");
    put_hex32(multiply(17, 19));
    putc(b'\n');

    let _trap_pass = ecall_trap_probe();
    let _ebreak_pass = ebreak_trap_probe();
    let _masked_mret_pass = ecall_masked_mret_probe();
    let _clint_rate_pass = clint_rate_probe();
    let _clint_mtip_pass = clint_mtip_probe();
    let _clint_msip_pass = clint_msip_probe();
    let _clint_masked_pass = clint_msip_masked_probe();
    let _plic_masked_pass = plic_uart0_masked_probe();
    let _plic_trap_pass = plic_uart0_trap_probe();
    let _plic_level_pass = plic_uart0_level_repend_probe();
    let _gpio_rise_masked_pass = gpio_irq_rise_masked_probe();
    let _gpio_rise_trap_pass = gpio_irq_rise_trap_probe();
    let _gpio_fall_pass = gpio_irq_fall_polled_probe();

    let gpio_pass = unsafe {
        core::ptr::write_volatile(GPIO_INPUT_EN, GREEN_LED);
        core::ptr::write_volatile(GPIO_OUTPUT_VAL, 0);
        core::ptr::write_volatile(GPIO_OUTPUT_EN, GREEN_LED);

        let mut pass = core::ptr::read_volatile(GPIO_INPUT_EN) == GREEN_LED
            && core::ptr::read_volatile(GPIO_OUTPUT_EN) == GREEN_LED;
        for value in [GREEN_LED, 0, GREEN_LED] {
            core::ptr::write_volatile(GPIO_OUTPUT_VAL, value);
            pass &= core::ptr::read_volatile(GPIO_OUTPUT_VAL) == value;
            pass &= core::ptr::read_volatile(GPIO_INPUT_VAL) & GREEN_LED == value;
        }
        pass
    };
    let (input_en, output_en, output_val, input_val) = unsafe {
        (
            core::ptr::read_volatile(GPIO_INPUT_EN),
            core::ptr::read_volatile(GPIO_OUTPUT_EN),
            core::ptr::read_volatile(GPIO_OUTPUT_VAL),
            core::ptr::read_volatile(GPIO_INPUT_VAL),
        )
    };
    puts("gpio=");
    puts(if gpio_pass { "PASS" } else { "FAIL" });
    puts(" input_en=");
    put_hex32(input_en);
    puts(" output_en=");
    put_hex32(output_en);
    puts(" output_val=");
    put_hex32(output_val);
    puts(" input_val=");
    put_hex32(input_val);
    putc(b'\n');

    loop {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
