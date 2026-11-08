// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_std]
#![no_main]

//! Renode-onboarding smoke for the imported nRF52840 system. Bare registers;
//! each block reads its own peripheral back and reports over UARTE0 (EasyDMA):
//!   FICR    INFO.PART reads 0x52840
//!   NVMC    READY = 1
//!   CLOCK   HFCLKSTART -> EVENTS_HFCLKSTARTED, HFCLKSTAT; LFCLKSTART (XTAL)
//!           -> EVENTS_LFCLKSTARTED, LFCLKSTAT
//!   GPIO    P0.13 output with the input buffer connected: OUT and IN read back
//!   GPIOTE  CONFIG[0] task mode on P0.14: TASKS_OUT toggles the pad (GPIO IN);
//!           the GPIO OUT latch is not GPIOTE's to change. P0.14 stays a GPIO
//!           input (PIN_CNF = 0, buffer connected) — task mode overrides the
//!           direction on silicon. The engine does NOT yet show a GPIOTE drive
//!           in IN when PIN_CNF.DIR = output (it returns OUT), so that
//!           nrfx-style configuration is deliberately not claimed here.
//!   TIMER0  32-bit timer, CC[0] compare event, CAPTURE advances, CLEAR resets
//!   RTC0    COUNTER advances, CC[0] compare event (EVTEN.COMPARE0 on, as
//!           nrfx does; the INTEN-only event path is not exercised)
//!   RNG     two EVENTS_VALRDY samples
//!   PPI     EGU0 TRIGGERED[0] -> EGU1 TASKS_TRIGGER[0] only while CH0 is on
//! A summary word goes to RAM at 0x2003FF00 (one bit per block passed). TEMP
//! (DATARDY), ECB (AES) and the radio are not exercised: TEMP and ECB are
//! listed as unmodelled residuals in the chip's silicon-conformance block.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const UART0: u32 = 0x4000_2000;
const UART_TASKS_STARTTX: u32 = UART0 + 0x008;
const UART_EVENTS_ENDTX: u32 = UART0 + 0x120;
const UART_ENABLE: u32 = UART0 + 0x500;
const UART_TXD_PTR: u32 = UART0 + 0x544;
const UART_TXD_MAXCNT: u32 = UART0 + 0x548;

const FICR_INFO_PART: u32 = 0x1000_0100;
const NVMC_READY: u32 = 0x4001_E400;

const CLOCK: u32 = 0x4000_0000;
const CLK_TASKS_HFCLKSTART: u32 = CLOCK;
const CLK_TASKS_LFCLKSTART: u32 = CLOCK + 0x008;
const CLK_EVENTS_HFCLKSTARTED: u32 = CLOCK + 0x100;
const CLK_EVENTS_LFCLKSTARTED: u32 = CLOCK + 0x104;
const CLK_HFCLKSTAT: u32 = CLOCK + 0x40C;
const CLK_LFCLKSTAT: u32 = CLOCK + 0x418;
const CLK_LFCLKSRC: u32 = CLOCK + 0x518;

const GPIO0: u32 = 0x5000_0000;
const GPIO_OUT: u32 = GPIO0 + 0x504;
const GPIO_OUTSET: u32 = GPIO0 + 0x508;
const GPIO_OUTCLR: u32 = GPIO0 + 0x50C;
const GPIO_IN: u32 = GPIO0 + 0x510;
const GPIO_PIN_CNF: u32 = GPIO0 + 0x700;
const LED0: u32 = 13;
const GPIOTE_PIN: u32 = 14;

const GPIOTE: u32 = 0x4000_6000;
const GPIOTE_TASKS_OUT0: u32 = GPIOTE;
const GPIOTE_CONFIG0: u32 = GPIOTE + 0x510;

const TIMER0: u32 = 0x4000_8000;
const TIM_TASKS_START: u32 = TIMER0;
const TIM_TASKS_STOP: u32 = TIMER0 + 0x004;
const TIM_TASKS_CLEAR: u32 = TIMER0 + 0x00C;
const TIM_TASKS_CAPTURE1: u32 = TIMER0 + 0x044;
const TIM_EVENTS_COMPARE0: u32 = TIMER0 + 0x140;
const TIM_MODE: u32 = TIMER0 + 0x504;
const TIM_BITMODE: u32 = TIMER0 + 0x508;
const TIM_PRESCALER: u32 = TIMER0 + 0x510;
const TIM_CC0: u32 = TIMER0 + 0x540;
const TIM_CC1: u32 = TIMER0 + 0x544;

const RTC0: u32 = 0x4000_B000;
const RTC_TASKS_START: u32 = RTC0;
const RTC_TASKS_STOP: u32 = RTC0 + 0x004;
const RTC_EVENTS_COMPARE0: u32 = RTC0 + 0x140;
const RTC_COUNTER: u32 = RTC0 + 0x504;
const RTC_PRESCALER: u32 = RTC0 + 0x508;
const RTC_EVTENSET: u32 = RTC0 + 0x344;
const RTC_EVTENCLR: u32 = RTC0 + 0x348;
const RTC_CC0: u32 = RTC0 + 0x540;

const RNG: u32 = 0x4000_D000;
const RNG_TASKS_START: u32 = RNG;
const RNG_TASKS_STOP: u32 = RNG + 0x004;
const RNG_EVENTS_VALRDY: u32 = RNG + 0x100;
const RNG_VALUE: u32 = RNG + 0x508;

const EGU0: u32 = 0x4001_4000;
const EGU1: u32 = 0x4001_5000;
const EGU_TASKS_TRIGGER0: u32 = 0x000;
const EGU_EVENTS_TRIGGERED0: u32 = 0x100;

const PPI: u32 = 0x4001_F000;
const PPI_CHENSET: u32 = PPI + 0x504;
const PPI_CHENCLR: u32 = PPI + 0x508;
const PPI_CH0_EEP: u32 = PPI + 0x510;
const PPI_CH0_TEP: u32 = PPI + 0x514;

const RESULT: u32 = 0x2003_FF00;
const SPIN: u32 = 200_000;

static mut TX: [u8; 64] = [0; 64];

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// Poll `addr` until it reads non-zero; false on timeout.
fn wait_set(addr: u32) -> bool {
    for _ in 0..SPIN {
        if rd(addr) != 0 {
            return true;
        }
    }
    false
}

/// A few bus reads so a task's queued pad drive lands before the next read.
fn settle() {
    for _ in 0..16 {
        let _ = rd(NVMC_READY);
    }
}

struct Line {
    len: usize,
}

impl Line {
    fn new() -> Self {
        Line { len: 0 }
    }

    fn s(&mut self, s: &[u8]) -> &mut Self {
        for &b in s {
            if self.len < 63 {
                unsafe { (*core::ptr::addr_of_mut!(TX))[self.len] = b };
                self.len += 1;
            }
        }
        self
    }

    fn hex(&mut self, v: u32, digits: u32) -> &mut Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for i in (0..digits).rev() {
            self.s(&[HEX[((v >> (i * 4)) & 0xF) as usize]]);
        }
        self
    }

    fn ok(&mut self, pass: bool) -> &mut Self {
        self.s(if pass { b" ok" } else { b" FAIL" })
    }

    /// EasyDMA reads the line from RAM, as the UARTE data path requires.
    fn send(&mut self) {
        self.s(b"\n");
        wr(UART_EVENTS_ENDTX, 0);
        wr(UART_TXD_PTR, core::ptr::addr_of!(TX) as u32);
        wr(UART_TXD_MAXCNT, self.len as u32);
        wr(UART_TASKS_STARTTX, 1);
        while rd(UART_EVENTS_ENDTX) == 0 {}
        self.len = 0;
    }
}

#[entry]
fn main() -> ! {
    let mut summary = 0u32;
    let mut l = Line::new();
    wr(UART_ENABLE, 8);
    l.s(b"NRF52840 RENODE SMOKE").send();

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
    // PIN_CNF = 0: GPIO input, input buffer connected so IN shows the pad.
    wr(GPIO_PIN_CNF + GPIOTE_PIN * 4, 0);
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

    wr(RESULT, 0x5284_0000 | summary);
    if summary == 0xFF {
        l.s(b"NRF52840 PASS").send();
    } else {
        l.s(b"NRF52840 FAIL summary=").hex(summary, 2).send();
    }

    // Heartbeat on P0.13 for the gpio_edges check.
    loop {
        wr(GPIO_OUTSET, 1 << LED0);
        for _ in 0..1000u32 {
            let _ = rd(UART_ENABLE);
        }
        wr(GPIO_OUTCLR, 1 << LED0);
        for _ in 0..1000u32 {
            let _ = rd(UART_ENABLE);
        }
    }
}
