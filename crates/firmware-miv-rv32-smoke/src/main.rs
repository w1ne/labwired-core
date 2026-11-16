// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Mi-V RV32 smoke: CPU execution, CoreUART TX-ready polling and UART output,
//! CLINT machine timer and software interrupts, and the PLIC routing the two
//! CoreTimer interrupts to MEIP.
//!
//! The ELF is loaded directly in the Renode Mi-V flash window. The CLINT
//! probes check that `mtime` runs at the 66 MHz CPU clock, an MTIP wake from
//! WFI, an MSIP interrupt taken after the `msip` store, and MSIP held pending
//! while `mstatus.MIE` is clear. The PLIC probes check the fixed priority 1
//! (`prioritiesEnabled: false` in Renode's miv.repl), CoreTimer0 latching
//! source 30 with its enable clear, the MEI trap with claim/complete, and a
//! polled claim of CoreTimer1's source 31. The GPIO probes check that
//! gpioOutputs drives its output register with no PLIC source, and that a
//! looped-back rising edge on gpioInputs pin 3 pends PLIC source 29 and is
//! claimed by polling. The smoke does not claim DDR initialization or a
//! boot-ROM path.

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
    "la t1, TRAP_COUNT",
    "lw t0, 0(t1)",
    "addi t0, t0, 1",
    "sw t0, 0(t1)",
    "csrr t0, mcause",
    "bgez t0, 1f",
    // External interrupt: claim, stop CoreTimer0 and clear its interrupt,
    // complete.
    "li t1, 0x8000000b",
    "bne t0, t1, 3f",
    "li t1, 0x40200004",
    "lw t0, 0(t1)",
    "la t1, TRAP_CLAIM",
    "sw t0, 0(t1)",
    "li t1, 0x70003000",
    "sw zero, 8(t1)",
    "li t2, 1",
    "sw t2, 0x10(t1)",
    "li t1, 0x40200004",
    "sw t0, 0(t1)",
    "mret",
    // Other interrupts: clear msip, disarm mtimecmp (high word first).
    "3:",
    "li t1, 0x44000000",
    "sw zero, 0(t1)",
    "li t0, -1",
    "li t1, 0x44004000",
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

/// Microchip CoreUARTapb at 0x70001000: TxData@+0x00, Status@+0x10;
/// TXRDY is bit 0 and is set while the transmitter can accept a byte.
const UART_BASE: usize = 0x7000_1000;

fn putc_at(base: usize, byte: u8) {
    unsafe {
        while core::ptr::read_volatile((base + 0x10) as *const u32) & 1 == 0 {}
        core::ptr::write_volatile(base as *mut u32, byte as u32);
    }
}

fn putc(byte: u8) {
    putc_at(UART_BASE, byte);
}

fn puts_at(base: usize, text: &str) {
    for byte in text.bytes() {
        putc_at(base, byte);
    }
}

fn puts(text: &str) {
    puts_at(UART_BASE, text);
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

const CLINT_MSIP: *mut u32 = 0x4400_0000 as *mut u32;
const CLINT_MTIMECMP_LO: *mut u32 = 0x4400_4000 as *mut u32;
const CLINT_MTIMECMP_HI: *mut u32 = 0x4400_4004 as *mut u32;
const CLINT_MTIME_LO: *const u32 = 0x4400_BFF8 as *const u32;
const CLINT_MTIME_HI: *const u32 = 0x4400_BFFC as *const u32;
const MIP_MSIP: u32 = 1 << 3;
const MIP_MTIP: u32 = 1 << 7;
const MIP_MEIP: u32 = 1 << 11;
const MSTATUS_MIE: u32 = 1 << 3;

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
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TRAP_CLAIM), 0);
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

/// `mtimecmp` resets disarmed, and `mtime` counts the 66 MHz CPU clock, so
/// over a polling window its advance equals the `cycle` advance.
fn clint_rate_probe() -> bool {
    let (cmp_lo, cmp_hi) = unsafe {
        (
            core::ptr::read_volatile(CLINT_MTIMECMP_LO),
            core::ptr::read_volatile(CLINT_MTIMECMP_HI),
        )
    };
    let c0 = rdcycle();
    let t0 = unsafe { core::ptr::read_volatile(CLINT_MTIME_LO) };
    let mut t1 = t0;
    while t1.wrapping_sub(t0) < 256 {
        t1 = unsafe { core::ptr::read_volatile(CLINT_MTIME_LO) };
    }
    let c1 = rdcycle();
    let hi = unsafe { core::ptr::read_volatile(CLINT_MTIME_HI) };
    let ticks = t1.wrapping_sub(t0);
    let clocks = c1.wrapping_sub(c0);
    // The window opens one instruction before the first mtime read and
    // closes one after the last.
    let rate = clocks >= ticks && clocks - ticks <= 4;
    let disarmed = cmp_lo == u32::MAX && cmp_hi == u32::MAX;
    let pass = disarmed && rate && hi == 0 && read_mip() & MIP_MTIP == 0;
    puts("clint mtimecmp=");
    put_hex32(cmp_hi);
    put_hex32(cmp_lo);
    flag(" cpu_clock_rate=", rate);
    flag(" mtip_low=", read_mip() & MIP_MTIP == 0);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// Arm `mtimecmp` 2000 CPU clocks ahead and wait in WFI with MTIE enabled.
fn clint_mtip_probe() -> bool {
    reset_trap_record();
    let start = unsafe { core::ptr::read_volatile(CLINT_MTIME_LO) };
    let target = start.wrapping_add(2000);
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
            mie = in(reg) MSTATUS_MIE,
            out("t0") _,
            out("t1") _,
            out("t2") _,
            options(nostack),
        );
    }
    let woke = unsafe { core::ptr::read_volatile(CLINT_MTIME_LO) };
    let (count, cause, _) = trap_record();
    let disarmed = unsafe { core::ptr::read_volatile(CLINT_MTIMECMP_HI) } == u32::MAX;
    let reached = woke.wrapping_sub(start) >= 2000;
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
            mie = in(reg) MSTATUS_MIE,
            msip = in(reg) CLINT_MSIP,
            after = lateout(reg) after_store,
            out("t0") _,
            out("t1") _,
            out("t2") _,
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

const PLIC_PRIORITY_BASE: usize = 0x4000_0000;
const PLIC_PENDING: *const u32 = 0x4000_1000 as *const u32;
const PLIC_ENABLE: *mut u32 = 0x4000_2000 as *mut u32;
const PLIC_THRESHOLD: *mut u32 = 0x4020_0000 as *mut u32;
const PLIC_CLAIM: *mut u32 = 0x4020_0004 as *mut u32;
const TIMER0_BASE: usize = 0x7000_3000;
const TIMER1_BASE: usize = 0x7000_4000;
const TIMER0_SOURCE: u32 = 30;
const TIMER1_SOURCE: u32 = 31;

fn plic_priority(source: u32) -> *mut u32 {
    (PLIC_PRIORITY_BASE + 4 * source as usize) as *mut u32
}

fn timer_reg(base: usize, offset: usize) -> *mut u32 {
    (base + offset) as *mut u32
}

/// CoreTimer one-shot: Load = 16 counts, prescale /2, enable + interrupt
/// enable + one-shot.
fn start_one_shot(base: usize) {
    unsafe {
        core::ptr::write_volatile(timer_reg(base, 0x08), 0);
        core::ptr::write_volatile(timer_reg(base, 0x10), 1);
        core::ptr::write_volatile(timer_reg(base, 0x0C), 0);
        core::ptr::write_volatile(timer_reg(base, 0x00), 16);
        core::ptr::write_volatile(timer_reg(base, 0x08), 0b111);
    }
}

/// Poll the CoreTimer's raw interrupt status for at most `tries` reads.
fn wait_timer_raw(base: usize, tries: u32) -> bool {
    for _ in 0..tries {
        if unsafe { core::ptr::read_volatile(timer_reg(base, 0x14)) } & 1 != 0 {
            return true;
        }
    }
    false
}

/// Poll `mip` until MEIP reaches `level`, for at most `tries` reads.
fn wait_meip(level: bool, tries: u32) -> bool {
    for _ in 0..tries {
        if (read_mip() & MIP_MEIP != 0) == level {
            return true;
        }
    }
    false
}

/// With `mstatus.MIE` clear: priorities are fixed at 1 and the threshold at
/// 0; CoreTimer0's interrupt latches source 30 while its enable is clear, and
/// MEIP follows the enable without a trap.
fn plic_timer0_masked_probe() -> bool {
    reset_trap_record();
    let (prio_fixed, threshold_fixed, raw, pending, disabled_low, meip) = unsafe {
        core::ptr::write_volatile(PLIC_ENABLE, 0);
        core::ptr::write_volatile(plic_priority(TIMER0_SOURCE), 0);
        let prio_fixed = core::ptr::read_volatile(plic_priority(TIMER0_SOURCE)) == 1
            && core::ptr::read_volatile(plic_priority(TIMER1_SOURCE)) == 1;
        core::ptr::write_volatile(PLIC_THRESHOLD, 1);
        let threshold_fixed = core::ptr::read_volatile(PLIC_THRESHOLD) == 0;
        start_one_shot(TIMER0_BASE);
        let raw = wait_timer_raw(TIMER0_BASE, 1000);
        // The source latches in the gateway while its enable is clear.
        let mut pending = false;
        for _ in 0..1000 {
            if core::ptr::read_volatile(PLIC_PENDING) & (1 << TIMER0_SOURCE) != 0 {
                pending = true;
                break;
            }
        }
        let disabled_low = read_mip() & MIP_MEIP == 0;
        core::ptr::write_volatile(PLIC_ENABLE, 1 << TIMER0_SOURCE);
        let meip = wait_meip(true, 1000);
        (
            prio_fixed,
            threshold_fixed,
            raw,
            pending,
            disabled_low,
            meip,
        )
    };
    let (count, _, _) = trap_record();
    let pass =
        prio_fixed && threshold_fixed && raw && pending && disabled_low && meip && count == 0;
    flag("plic timer0 prio_fixed=", prio_fixed);
    flag(" threshold_fixed=", threshold_fixed);
    flag(" raw=", raw);
    flag(" pending=", pending);
    flag(" disabled_low=", disabled_low);
    flag(" meip=", meip);
    puts(" traps=");
    put_hex32(count);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// Take the pending CoreTimer0 interrupt as a machine external interrupt.
/// The handler claims source 30, stops the timer, clears its interrupt and
/// completes; afterwards the source is neither pending nor claimable and
/// MEIP is low.
fn plic_timer0_trap_probe() -> bool {
    reset_trap_record();
    unsafe {
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
            mie = in(reg) MSTATUS_MIE,
            out("t0") _,
            out("t1") _,
            out("t2") _,
            options(nostack),
        );
    }
    let (count, cause, _) = trap_record();
    let claim = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TRAP_CLAIM)) };
    let (masked, pending, reclaim) = unsafe {
        (
            core::ptr::read_volatile(timer_reg(TIMER0_BASE, 0x18)),
            core::ptr::read_volatile(PLIC_PENDING) & (1 << TIMER0_SOURCE),
            core::ptr::read_volatile(PLIC_CLAIM),
        )
    };
    let meip_low = read_mip() & MIP_MEIP == 0;
    unsafe { core::ptr::write_volatile(PLIC_ENABLE, 0) };
    let pass = count == 1
        && cause == 0x8000_000B
        && claim == TIMER0_SOURCE
        && masked == 0
        && pending == 0
        && reclaim == 0
        && meip_low;
    puts("plic timer0 mcause=");
    put_hex32(cause);
    puts(" count=");
    put_hex32(count);
    puts(" claim=");
    put_hex32(claim);
    flag(" timer_cleared=", masked == 0);
    flag(" pending_clear=", pending == 0);
    puts(" reclaim=");
    put_hex32(reclaim);
    flag(" meip_low=", meip_low);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// CoreTimer1 on source 31, claimed by polling with `mstatus.MIE` clear.
fn plic_timer1_poll_probe() -> bool {
    reset_trap_record();
    let (raw, meip, claim, meip_low, reclaim) = unsafe {
        start_one_shot(TIMER1_BASE);
        let raw = wait_timer_raw(TIMER1_BASE, 1000);
        core::ptr::write_volatile(PLIC_ENABLE, 1 << TIMER1_SOURCE);
        let meip = wait_meip(true, 1000);
        let claim = core::ptr::read_volatile(PLIC_CLAIM);
        let meip_low = read_mip() & MIP_MEIP == 0;
        core::ptr::write_volatile(timer_reg(TIMER1_BASE, 0x08), 0);
        core::ptr::write_volatile(timer_reg(TIMER1_BASE, 0x10), 1);
        core::ptr::write_volatile(PLIC_CLAIM, claim);
        let reclaim = core::ptr::read_volatile(PLIC_CLAIM);
        core::ptr::write_volatile(PLIC_ENABLE, 0);
        (raw, meip, claim, meip_low, reclaim)
    };
    let (count, _, _) = trap_record();
    let pass = raw && meip && claim == TIMER1_SOURCE && meip_low && reclaim == 0 && count == 0;
    flag("plic timer1 raw=", raw);
    flag(" meip=", meip);
    puts(" claim=");
    put_hex32(claim);
    flag(" claimed_low=", meip_low);
    puts(" reclaim=");
    put_hex32(reclaim);
    puts(" traps=");
    put_hex32(count);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

const GPIO_IN_BASE: usize = 0x7000_2000;
const GPIO_OUT_BASE: usize = 0x7000_5000;
const GPIO_IN_SOURCE: u32 = 29;
const GPIO_IRQ: usize = 0x80;
const GPIO_INPUT: usize = 0x90;
const GPIO_OUTPUT: usize = 0xA0;
/// CoreGPIO CONFIG_X: OUTREG | INREG | OUTBUFF | INTENABLE, INTTYPE rising.
const GPIO_CFG_LOOPBACK_RISE: u32 = 0b0100_1111;

fn gpio_reg(base: usize, offset: usize) -> *mut u32 {
    (base + offset) as *mut u32
}

/// gpioOutputs (0x70005000) drives its output register and has no PLIC
/// source: a pin with a level-high interrupt enabled pends nothing.
fn gpio_outputs_probe() -> bool {
    reset_trap_record();
    let (driven, status, pending) = unsafe {
        core::ptr::write_volatile(PLIC_ENABLE, 0);
        core::ptr::write_volatile(gpio_reg(GPIO_OUT_BASE, 0), 0b0000_1111);
        core::ptr::write_volatile(gpio_reg(GPIO_OUT_BASE, GPIO_OUTPUT), 1);
        let driven = core::ptr::read_volatile(gpio_reg(GPIO_OUT_BASE, GPIO_OUTPUT));
        let status = core::ptr::read_volatile(gpio_reg(GPIO_OUT_BASE, GPIO_IRQ));
        let mut pending = 0;
        for _ in 0..200 {
            pending |= core::ptr::read_volatile(PLIC_PENDING);
        }
        core::ptr::write_volatile(gpio_reg(GPIO_OUT_BASE, 0), 0);
        (driven, status, pending)
    };
    let (count, _, _) = trap_record();
    let pass = driven == 1 && status == 1 && pending == 0 && count == 0;
    puts("gpio outputs out=");
    put_hex32(driven);
    puts(" irq=");
    put_hex32(status);
    puts(" plic_pending=");
    put_hex32(pending);
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// gpioInputs (0x70002000) drives PLIC source 29. Pin 3 is looped back
/// (output buffer on) with a rising-edge interrupt: driving it high latches
/// its IRQ bit, pends source 29 with its enable clear and raises MEIP once
/// enabled. A polled claim returns 29; clearing the latch before the
/// complete leaves nothing to re-pend.
fn gpio_inputs_plic_probe() -> bool {
    reset_trap_record();
    let (input, status, pending, disabled_low, meip, claim, cleared, meip_low, reclaim) = unsafe {
        core::ptr::write_volatile(PLIC_ENABLE, 0);
        core::ptr::write_volatile(gpio_reg(GPIO_IN_BASE, GPIO_OUTPUT), 0);
        core::ptr::write_volatile(gpio_reg(GPIO_IN_BASE, 3 * 4), GPIO_CFG_LOOPBACK_RISE);
        core::ptr::write_volatile(gpio_reg(GPIO_IN_BASE, GPIO_IRQ), u32::MAX);
        core::ptr::write_volatile(gpio_reg(GPIO_IN_BASE, GPIO_OUTPUT), 1 << 3);
        let input = core::ptr::read_volatile(gpio_reg(GPIO_IN_BASE, GPIO_INPUT));
        let status = core::ptr::read_volatile(gpio_reg(GPIO_IN_BASE, GPIO_IRQ));
        let mut pending = false;
        for _ in 0..1000 {
            if core::ptr::read_volatile(PLIC_PENDING) & (1 << GPIO_IN_SOURCE) != 0 {
                pending = true;
                break;
            }
        }
        let disabled_low = read_mip() & MIP_MEIP == 0;
        core::ptr::write_volatile(PLIC_ENABLE, 1 << GPIO_IN_SOURCE);
        let meip = wait_meip(true, 1000);
        let claim = core::ptr::read_volatile(PLIC_CLAIM);
        core::ptr::write_volatile(gpio_reg(GPIO_IN_BASE, GPIO_IRQ), 1 << 3);
        let cleared = core::ptr::read_volatile(gpio_reg(GPIO_IN_BASE, GPIO_IRQ)) == 0;
        core::ptr::write_volatile(PLIC_CLAIM, claim);
        let meip_low = wait_meip(false, 1000);
        let reclaim = core::ptr::read_volatile(PLIC_CLAIM);
        core::ptr::write_volatile(PLIC_ENABLE, 0);
        core::ptr::write_volatile(gpio_reg(GPIO_IN_BASE, 3 * 4), 0);
        (
            input,
            status,
            pending,
            disabled_low,
            meip,
            claim,
            cleared,
            meip_low,
            reclaim,
        )
    };
    let (count, _, _) = trap_record();
    let pass = input & (1 << 3) != 0
        && status == 1 << 3
        && pending
        && disabled_low
        && meip
        && claim == GPIO_IN_SOURCE
        && cleared
        && meip_low
        && reclaim == 0
        && count == 0;
    puts("plic gpio_in in=");
    put_hex32(input);
    puts(" irq=");
    put_hex32(status);
    flag(" pending=", pending);
    flag(" disabled_low=", disabled_low);
    flag(" meip=", meip);
    puts(" claim=");
    put_hex32(claim);
    flag(" latch_cleared=", cleared);
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
    puts("MIV RV32 SMOKE\n");
    puts("cpu=rv32im uart=coreuart\n");
    puts("mul=");
    put_hex32(multiply(17, 19));
    putc(b'\n');

    let _clint_rate_pass = clint_rate_probe();
    let _clint_mtip_pass = clint_mtip_probe();
    let _clint_msip_pass = clint_msip_probe();
    let _clint_masked_pass = clint_msip_masked_probe();
    let _plic_masked_pass = plic_timer0_masked_probe();
    let _plic_trap_pass = plic_timer0_trap_probe();
    let _plic_poll_pass = plic_timer1_poll_probe();
    let _gpio_out_pass = gpio_outputs_probe();
    let _gpio_in_pass = gpio_inputs_plic_probe();

    #[cfg(feature = "additional-uarts")]
    {
        puts_at(0x7000_8000, "MIV ADDITIONAL UART 1\n");
        puts_at(0x7000_9000, "MIV ADDITIONAL UART 2\n");
    }

    loop {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
