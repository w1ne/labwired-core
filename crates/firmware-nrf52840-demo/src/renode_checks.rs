// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! nRF52840 silicon checks shared by the Renode-onboarding smokes: the chip
//! row (`renode_smoke.rs`) and the nRF52840 DK board row
//! (`nrf52840dk_smoke.rs`) run the same eight blocks — see `renode_smoke.rs`
//! for what each one asserts. Bit n of the returned word is block n.
#![allow(dead_code)]

use core::ptr::{read_volatile, write_volatile};

pub const UART0: u32 = 0x4000_2000;
pub const UART_TASKS_STARTTX: u32 = UART0 + 0x008;
pub const UART_EVENTS_ENDTX: u32 = UART0 + 0x120;
pub const UART_ENABLE: u32 = UART0 + 0x500;
pub const UART_TXD_PTR: u32 = UART0 + 0x544;
pub const UART_TXD_MAXCNT: u32 = UART0 + 0x548;

pub const FICR_INFO_PART: u32 = 0x1000_0100;
pub const NVMC_READY: u32 = 0x4001_E400;

pub const CLOCK: u32 = 0x4000_0000;
pub const CLK_TASKS_HFCLKSTART: u32 = CLOCK;
pub const CLK_TASKS_LFCLKSTART: u32 = CLOCK + 0x008;
pub const CLK_EVENTS_HFCLKSTARTED: u32 = CLOCK + 0x100;
pub const CLK_EVENTS_LFCLKSTARTED: u32 = CLOCK + 0x104;
pub const CLK_HFCLKSTAT: u32 = CLOCK + 0x40C;
pub const CLK_LFCLKSTAT: u32 = CLOCK + 0x418;
pub const CLK_LFCLKSRC: u32 = CLOCK + 0x518;

pub const GPIO0: u32 = 0x5000_0000;
pub const GPIO_OUT: u32 = GPIO0 + 0x504;
pub const GPIO_OUTSET: u32 = GPIO0 + 0x508;
pub const GPIO_OUTCLR: u32 = GPIO0 + 0x50C;
pub const GPIO_IN: u32 = GPIO0 + 0x510;
pub const GPIO_PIN_CNF: u32 = GPIO0 + 0x700;
pub const LED0: u32 = 13;
pub const GPIOTE_PIN: u32 = 14;

pub const GPIOTE: u32 = 0x4000_6000;
pub const GPIOTE_TASKS_OUT0: u32 = GPIOTE;
pub const GPIOTE_CONFIG0: u32 = GPIOTE + 0x510;

pub const TIMER0: u32 = 0x4000_8000;
pub const TIM_TASKS_START: u32 = TIMER0;
pub const TIM_TASKS_STOP: u32 = TIMER0 + 0x004;
pub const TIM_TASKS_CLEAR: u32 = TIMER0 + 0x00C;
pub const TIM_TASKS_CAPTURE1: u32 = TIMER0 + 0x044;
pub const TIM_EVENTS_COMPARE0: u32 = TIMER0 + 0x140;
pub const TIM_MODE: u32 = TIMER0 + 0x504;
pub const TIM_BITMODE: u32 = TIMER0 + 0x508;
pub const TIM_PRESCALER: u32 = TIMER0 + 0x510;
pub const TIM_CC0: u32 = TIMER0 + 0x540;
pub const TIM_CC1: u32 = TIMER0 + 0x544;

pub const RTC0: u32 = 0x4000_B000;
pub const RTC_TASKS_START: u32 = RTC0;
pub const RTC_TASKS_STOP: u32 = RTC0 + 0x004;
pub const RTC_EVENTS_COMPARE0: u32 = RTC0 + 0x140;
pub const RTC_COUNTER: u32 = RTC0 + 0x504;
pub const RTC_PRESCALER: u32 = RTC0 + 0x508;
pub const RTC_EVTENSET: u32 = RTC0 + 0x344;
pub const RTC_EVTENCLR: u32 = RTC0 + 0x348;
pub const RTC_CC0: u32 = RTC0 + 0x540;

pub const RNG: u32 = 0x4000_D000;
pub const RNG_TASKS_START: u32 = RNG;
pub const RNG_TASKS_STOP: u32 = RNG + 0x004;
pub const RNG_EVENTS_VALRDY: u32 = RNG + 0x100;
pub const RNG_VALUE: u32 = RNG + 0x508;

pub const EGU0: u32 = 0x4001_4000;
pub const EGU1: u32 = 0x4001_5000;
pub const EGU_TASKS_TRIGGER0: u32 = 0x000;
pub const EGU_EVENTS_TRIGGERED0: u32 = 0x100;

pub const PPI: u32 = 0x4001_F000;
pub const PPI_CHENSET: u32 = PPI + 0x504;
pub const PPI_CHENCLR: u32 = PPI + 0x508;
pub const PPI_CH0_EEP: u32 = PPI + 0x510;
pub const PPI_CH0_TEP: u32 = PPI + 0x514;

pub const RESULT: u32 = 0x2003_FF00;
pub const SPIN: u32 = 200_000;

const SCB_ICSR: u32 = 0xE000_ED04;
const SCB_VTOR: u32 = 0xE000_ED08;
const SCB_SHPR3: u32 = 0xE000_ED20;
const SYSTICK_CSR: u32 = 0xE000_E010;
const ICSR_PENDSTSET: u32 = 1 << 26;
const ICSR_PENDSTCLR: u32 = 1 << 25;
const ICSR_PENDSVSET: u32 = 1 << 28;
const ICSR_PENDSVCLR: u32 = 1 << 27;
const TEST_VECTOR_TABLE: u32 = 0x2000_0080;
const TAILCHAIN_RECORD: u32 = 0x2000_0100;
const THREAD_R0: u32 = 0x5448_5244;
const SYSTICK_R0: u32 = 0x5359_5354;

static mut TX: [u8; 64] = [0; 64];

pub fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

pub fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// Poll `addr` until it reads non-zero; false on timeout.
pub fn wait_set(addr: u32) -> bool {
    for _ in 0..SPIN {
        if rd(addr) != 0 {
            return true;
        }
    }
    false
}

/// A few bus reads so a task's queued pad drive lands before the next read.
pub fn settle() {
    for _ in 0..16 {
        let _ = rd(NVMC_READY);
    }
}

pub struct Line {
    len: usize,
}

impl Line {
    pub fn new() -> Self {
        Line { len: 0 }
    }

    pub fn s(&mut self, s: &[u8]) -> &mut Self {
        for &b in s {
            if self.len < 63 {
                unsafe { (*core::ptr::addr_of_mut!(TX))[self.len] = b };
                self.len += 1;
            }
        }
        self
    }

    pub fn hex(&mut self, v: u32, digits: u32) -> &mut Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for i in (0..digits).rev() {
            self.s(&[HEX[((v >> (i * 4)) & 0xF) as usize]]);
        }
        self
    }

    pub fn ok(&mut self, pass: bool) -> &mut Self {
        self.s(if pass { b" ok" } else { b" FAIL" })
    }

    /// EasyDMA reads the line from RAM, as the UARTE data path requires.
    pub fn send(&mut self) {
        self.s(b"\n");
        wr(UART_EVENTS_ENDTX, 0);
        wr(UART_TXD_PTR, core::ptr::addr_of!(TX) as u32);
        wr(UART_TXD_MAXCNT, self.len as u32);
        wr(UART_TASKS_STARTTX, 1);
        while rd(UART_EVENTS_ENDTX) == 0 {}
        self.len = 0;
    }
}

pub fn chip_checks(l: &mut Line) -> u32 {
    let mut summary = 0u32;
    // FICR / NVMC.
    let part = rd(FICR_INFO_PART);
    let ready = rd(NVMC_READY);
    let pass = part == 0x52840 && ready & 1 == 1;
    l.s(b"ficr part=")
        .hex(part, 5)
        .s(b" nvmc ready=")
        .hex(ready, 1)
        .ok(pass)
        .send();
    summary |= pass as u32;

    // CLOCK: HFXO, then LFXO.
    wr(CLK_EVENTS_HFCLKSTARTED, 0);
    wr(CLK_TASKS_HFCLKSTART, 1);
    let hf_ev = wait_set(CLK_EVENTS_HFCLKSTARTED);
    let hfstat = rd(CLK_HFCLKSTAT);
    wr(CLK_LFCLKSRC, 1);
    wr(CLK_EVENTS_LFCLKSTARTED, 0);
    wr(CLK_TASKS_LFCLKSTART, 1);
    let lf_ev = wait_set(CLK_EVENTS_LFCLKSTARTED);
    let lfstat = rd(CLK_LFCLKSTAT);
    // HFCLKSTAT: SRC=Xtal (bit 0), STATE=Running (bit 16). LFCLKSTAT: SRC=Xtal (1), STATE (bit 16).
    let pass = hf_ev && lf_ev && hfstat == 0x0001_0001 && lfstat == 0x0001_0001;
    l.s(b"clock hfstat=")
        .hex(hfstat, 8)
        .s(b" lfstat=")
        .hex(lfstat, 8)
        .ok(pass)
        .send();
    summary |= (pass as u32) << 1;

    // GPIO: P0.13 output with the input buffer connected (PIN_CNF DIR=1, INPUT=0).
    wr(GPIO_PIN_CNF + LED0 * 4, 1);
    wr(GPIO_OUTSET, 1 << LED0);
    let out_hi = rd(GPIO_OUT) & (1 << LED0);
    let in_hi = rd(GPIO_IN) & (1 << LED0);
    wr(GPIO_OUTCLR, 1 << LED0);
    let out_lo = rd(GPIO_OUT) & (1 << LED0);
    let in_lo = rd(GPIO_IN) & (1 << LED0);
    let pass = out_hi != 0 && in_hi != 0 && out_lo == 0 && in_lo == 0;
    l.s(b"gpio p0.13 in=")
        .hex(in_hi, 8)
        .s(b"/")
        .hex(in_lo, 8)
        .ok(pass)
        .send();
    summary |= (pass as u32) << 2;

    // GPIOTE: task mode (3), PSEL=14, POLARITY=Toggle (3), OUTINIT=low.
    // PIN_CNF = 1: GPIO output (OUT = 0), input buffer connected so IN shows
    // the pad. GPIOTE owns the pin while in task mode, whatever DIR/OUT say.
    wr(GPIO_OUTCLR, 1 << GPIOTE_PIN);
    wr(GPIO_PIN_CNF + GPIOTE_PIN * 4, 1);
    wr(GPIOTE_CONFIG0, 3 | (GPIOTE_PIN << 8) | (3 << 16));
    let latch = rd(GPIO_OUT) & (1 << GPIOTE_PIN);
    wr(GPIOTE_TASKS_OUT0, 1);
    settle();
    let t1 = rd(GPIO_IN) & (1 << GPIOTE_PIN);
    wr(GPIOTE_TASKS_OUT0, 1);
    settle();
    let t2 = rd(GPIO_IN) & (1 << GPIOTE_PIN);
    let pass = t1 != 0 && t2 == 0 && latch == rd(GPIO_OUT) & (1 << GPIOTE_PIN);
    l.s(b"gpiote out0 p0.14 in=")
        .hex(t1 >> GPIOTE_PIN, 1)
        .hex(t2 >> GPIOTE_PIN, 1)
        .s(b" out=")
        .hex(latch >> GPIOTE_PIN, 1)
        .ok(pass)
        .send();
    summary |= (pass as u32) << 3;
    wr(GPIOTE_CONFIG0, 0);

    // TIMER0: 32-bit timer at 1 MHz, CC[0]=50 compare, CAPTURE[1], CLEAR.
    wr(TIM_MODE, 0);
    wr(TIM_BITMODE, 3);
    wr(TIM_PRESCALER, 4);
    wr(TIM_CC0, 50);
    wr(TIM_EVENTS_COMPARE0, 0);
    wr(TIM_TASKS_CLEAR, 1);
    wr(TIM_TASKS_START, 1);
    let cmp = wait_set(TIM_EVENTS_COMPARE0);
    wr(TIM_TASKS_CAPTURE1, 1);
    let cap = rd(TIM_CC1);
    wr(TIM_TASKS_STOP, 1);
    wr(TIM_TASKS_CLEAR, 1);
    wr(TIM_TASKS_CAPTURE1, 1);
    let cleared = rd(TIM_CC1);
    let pass = cmp && cap >= 50 && cleared == 0 && rd(TIM_PRESCALER) == 4;
    l.s(b"timer0 cmp=")
        .hex(cmp as u32, 1)
        .s(b" cap>=50 ")
        .hex((cap >= 50) as u32, 1)
        .s(b" clear=")
        .hex(cleared, 8)
        .ok(pass)
        .send();
    summary |= (pass as u32) << 4;

    // RTC0 off the LFCLK started above: PRESCALER=0, CC[0]=4.
    wr(RTC_PRESCALER, 0);
    wr(RTC_CC0, 4);
    wr(RTC_EVTENSET, 1 << 16);
    wr(RTC_EVENTS_COMPARE0, 0);
    wr(RTC_TASKS_START, 1);
    let rtc_cmp = wait_set(RTC_EVENTS_COMPARE0);
    let count = rd(RTC_COUNTER);
    wr(RTC_TASKS_STOP, 1);
    wr(RTC_EVTENCLR, 1 << 16);
    let pass = rtc_cmp && count >= 4;
    l.s(b"rtc0 cmp=")
        .hex(rtc_cmp as u32, 1)
        .s(b" counter>=4 ")
        .hex((count >= 4) as u32, 1)
        .ok(pass)
        .send();
    summary |= (pass as u32) << 5;

    // RNG: two samples.
    wr(RNG_EVENTS_VALRDY, 0);
    wr(RNG_TASKS_START, 1);
    let v1 = wait_set(RNG_EVENTS_VALRDY);
    let a = rd(RNG_VALUE);
    wr(RNG_EVENTS_VALRDY, 0);
    let v2 = wait_set(RNG_EVENTS_VALRDY);
    let b = rd(RNG_VALUE);
    wr(RNG_TASKS_STOP, 1);
    let pass = v1 && v2 && a <= 0xFF && b <= 0xFF;
    l.s(b"rng valrdy=")
        .hex(v1 as u32, 1)
        .hex(v2 as u32, 1)
        .s(b" bytes<=ff")
        .ok(pass)
        .send();
    summary |= (pass as u32) << 6;

    // PPI CH0: EGU0 EVENTS_TRIGGERED[0] -> EGU1 TASKS_TRIGGER[0].
    wr(PPI_CH0_EEP, EGU0 + EGU_EVENTS_TRIGGERED0);
    wr(PPI_CH0_TEP, EGU1 + EGU_TASKS_TRIGGER0);
    wr(EGU1 + EGU_EVENTS_TRIGGERED0, 0);
    wr(PPI_CHENCLR, 1);
    wr(EGU0 + EGU_TASKS_TRIGGER0, 1);
    for _ in 0..100 {
        let _ = rd(EGU0 + EGU_EVENTS_TRIGGERED0);
    }
    let off = rd(EGU1 + EGU_EVENTS_TRIGGERED0);
    wr(EGU0 + EGU_EVENTS_TRIGGERED0, 0);
    wr(PPI_CHENSET, 1);
    wr(EGU0 + EGU_TASKS_TRIGGER0, 1);
    let on = wait_set(EGU1 + EGU_EVENTS_TRIGGERED0);
    wr(PPI_CHENCLR, 1);
    let pass = off == 0 && on;
    l.s(b"ppi egu0->egu1 off=")
        .hex(off, 1)
        .s(b" on=")
        .hex(on as u32, 1)
        .ok(pass)
        .send();
    summary |= (pass as u32) << 7;

    summary
}

/// Pend the lower-priority PendSV exception from SysTick and prove the M4
/// tail-chains by checking the reused exception frame and live handler R0.
pub fn check_systick_pendsv_tailchain(l: &mut Line) -> bool {
    let prior_vtor = rd(SCB_VTOR);
    let prior_shpr3 = rd(SCB_SHPR3);
    let test_shpr3 = (prior_shpr3 & !0xFFFF_0000) | (0x80 << 24) | (0xC0 << 16);
    let prior_primask: u32;

    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
    }

    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    for word in 0..16 {
        wr(TEST_VECTOR_TABLE + word * 4, 0);
    }
    wr(
        TEST_VECTOR_TABLE + 14 * 4,
        (nrf_pendsv_tailchain_handler as *const () as usize as u32) | 1,
    );
    wr(
        TEST_VECTOR_TABLE + 15 * 4,
        (nrf_systick_tailchain_handler as *const () as usize as u32) | 1,
    );
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    wr(SCB_SHPR3, test_shpr3);
    for word in 0..12 {
        wr(TAILCHAIN_RECORD + word * 4, 0);
    }

    unsafe {
        core::arch::asm!(
            "dsb",
            "isb",
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }
    if prior_primask == 0 {
        unsafe { nrf_pend_systick_with_thread_r0() };
    }

    for _ in 0..100_000 {
        if rd(TAILCHAIN_RECORD + 44) == 1 {
            break;
        }
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let parent_ipsr = rd(TAILCHAIN_RECORD);
    let parent_icsr_before = rd(TAILCHAIN_RECORD + 4);
    let parent_icsr_after = rd(TAILCHAIN_RECORD + 8);
    let parent_msp = rd(TAILCHAIN_RECORD + 12);
    let parent_exc_return = rd(TAILCHAIN_RECORD + 16);
    let thread_r0 = rd(TAILCHAIN_RECORD + 20);
    let child_ipsr = rd(TAILCHAIN_RECORD + 24);
    let child_msp = rd(TAILCHAIN_RECORD + 28);
    let child_exc_return = rd(TAILCHAIN_RECORD + 32);
    let child_r0 = rd(TAILCHAIN_RECORD + 36);
    let child_icsr = rd(TAILCHAIN_RECORD + 40);
    let child_hit = rd(TAILCHAIN_RECORD + 44) == 1;
    let priority_configured = rd(SCB_SHPR3) & 0xFFFF_0000 == test_shpr3 & 0xFFFF_0000;
    let thread_ipsr: u32;
    unsafe {
        core::arch::asm!(
            "mrs {ipsr}, IPSR",
            ipsr = out(reg) thread_ipsr,
            options(nomem, nostack, preserves_flags),
        );
    }

    let masked_primask: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) masked_primask,
            options(nomem, nostack, preserves_flags),
        );
    }
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    wr(SCB_VTOR, prior_vtor);
    wr(SCB_SHPR3, prior_shpr3);
    unsafe {
        core::arch::asm!(
            "dsb",
            "isb",
            "msr PRIMASK, {primask}",
            primask = in(reg) masked_primask,
            options(nostack, preserves_flags),
        );
    }

    let deferred = parent_ipsr == 15
        && parent_icsr_before & 0x1FF == 15
        && parent_icsr_after & 0x1FF == 15
        && parent_icsr_after & ICSR_PENDSVSET != 0;
    let frame_reused = parent_msp == child_msp
        && parent_exc_return == 0xFFFF_FFF9
        && child_exc_return == parent_exc_return;
    let r0_live = thread_r0 == THREAD_R0 && child_r0 == SYSTICK_R0 && child_r0 != thread_r0;
    let child_is_pendsv = child_hit
        && child_ipsr == 14
        && child_icsr & 0x1FF == 14
        && child_icsr & ICSR_PENDSVSET == 0;
    let cleanup = rd(SCB_VTOR) == prior_vtor
        && rd(SCB_SHPR3) == prior_shpr3
        && rd(SCB_ICSR) & (ICSR_PENDSTSET | ICSR_PENDSVSET) == 0;
    let ok = prior_primask == 0
        && deferred
        && frame_reused
        && r0_live
        && child_is_pendsv
        && priority_configured
        && thread_ipsr == 0
        && cleanup;

    l.s(b"systick-pendsv frame=")
        .s(if frame_reused { b"1" } else { b"0" })
        .s(b" r0=")
        .s(if r0_live { b"1" } else { b"0" })
        .s(b" pending=")
        .s(if child_is_pendsv { b"1" } else { b"0" })
        .s(b" priority=")
        .s(if priority_configured { b"1" } else { b"0" })
        .ok(ok)
        .send();
    ok
}

/// Return a known Thread-mode R0 after software-pending SysTick.
#[unsafe(naked)]
unsafe extern "C" fn nrf_pend_systick_with_thread_r0() {
    core::arch::naked_asm!(
        "ldr r0, ={thread_r0}",
        "ldr r1, ={icsr}",
        "movs r2, #1",
        "lsls r2, r2, #26",
        "str r2, [r1]",
        "bx lr",
        thread_r0 = const THREAD_R0,
        icsr = const SCB_ICSR,
    );
}

/// Pend lower-priority PendSV while SysTick is active, then leave a marker in
/// R0 for the tail-chained handler.
#[unsafe(naked)]
unsafe extern "C" fn nrf_systick_tailchain_handler() {
    core::arch::naked_asm!(
        "mov r3, r0",
        "ldr r0, ={record}",
        "str r3, [r0, #20]",
        "mrs r3, ipsr",
        "str r3, [r0, #0]",
        "mrs r3, msp",
        "str r3, [r0, #12]",
        "mov r3, lr",
        "str r3, [r0, #16]",
        "ldr r1, ={icsr}",
        "ldr r2, [r1]",
        "str r2, [r0, #4]",
        "movs r2, #1",
        "lsls r2, r2, #28",
        "str r2, [r1]",
        "ldr r2, [r1]",
        "str r2, [r0, #8]",
        "ldr r0, ={systick_r0}",
        "bx lr",
        record = const TAILCHAIN_RECORD,
        icsr = const SCB_ICSR,
        systick_r0 = const SYSTICK_R0,
    );
}

/// Capture the PendSV exception state before returning to Thread mode.
#[unsafe(naked)]
unsafe extern "C" fn nrf_pendsv_tailchain_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "mrs r3, ipsr",
        "ldr r0, ={record}",
        "str r3, [r0, #24]",
        "mrs r3, msp",
        "str r3, [r0, #28]",
        "mov r3, lr",
        "str r3, [r0, #32]",
        "str r2, [r0, #36]",
        "ldr r3, ={icsr}",
        "ldr r3, [r3]",
        "str r3, [r0, #40]",
        "movs r3, #1",
        "str r3, [r0, #44]",
        "bx lr",
        record = const TAILCHAIN_RECORD,
        icsr = const SCB_ICSR,
    );
}
