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

pub const SAADC: u32 = 0x4000_7000;
pub const SAADC_RESULT: u32 = 0x2000_1000;

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

pub const TEMP: u32 = 0x4000_C000;
pub const TEMP_TASKS_START: u32 = TEMP;
pub const TEMP_TASKS_STOP: u32 = TEMP + 0x004;
pub const TEMP_EVENTS_DATARDY: u32 = TEMP + 0x100;
pub const TEMP_TEMP: u32 = TEMP + 0x508;

pub const ECB: u32 = 0x4000_E000;
pub const ECB_TASKS_STARTECB: u32 = ECB;
pub const ECB_EVENTS_ENDECB: u32 = ECB + 0x100;
pub const ECB_EVENTS_ERRORECB: u32 = ECB + 0x104;
pub const ECB_ECBDATAPTR: u32 = ECB + 0x504;

/// ECBDATAPTR block: key[16], cleartext[16], ciphertext[16] (PS v1.7 §6.5).
#[repr(C, align(4))]
struct EcbData([u8; 48]);
static mut ECB_DATA: EcbData = EcbData([0; 48]);

/// TEMP: TASKS_START -> EVENTS_DATARDY, then TEMP holds a signed value in
/// 0.25 degC steps (PS v1.7 §6.32). The range check is the chip's operating
/// range, -40..85 degC; the UART line prints the raw value the model gives.
pub fn check_temp(l: &mut Line) -> bool {
    wr(TEMP_EVENTS_DATARDY, 0);
    let idle = rd(TEMP_EVENTS_DATARDY) == 0;
    wr(TEMP_TASKS_START, 1);
    let ready = wait_set(TEMP_EVENTS_DATARDY);
    let raw = rd(TEMP_TEMP) as i32;
    wr(TEMP_TASKS_STOP, 1);
    wr(TEMP_EVENTS_DATARDY, 0);
    let pass = idle && ready && (-160..=340).contains(&raw);
    l.s(b"temp datardy=")
        .hex(ready as u32, 1)
        .s(b" raw=")
        .hex(raw as u32, 8)
        .ok(pass)
        .send();
    pass
}

/// ECB: AES-128 known-answer test from FIPS-197 Appendix C.1 through the
/// EasyDMA data block. key = 000102..0f, plaintext = 00112233..ff,
/// ciphertext = 69c4e0d86a7b0430d8cdb78070b4c55a.
pub fn check_ecb_fips197(l: &mut Line) -> bool {
    const CT: [u8; 16] = [
        0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4, 0xc5,
        0x5a,
    ];
    let data = core::ptr::addr_of_mut!(ECB_DATA) as *mut u8;
    for i in 0..16u8 {
        unsafe {
            write_volatile(data.add(i as usize), i);
            write_volatile(data.add(16 + i as usize), i * 0x11);
            write_volatile(data.add(32 + i as usize), 0);
        }
    }
    wr(ECB_EVENTS_ENDECB, 0);
    wr(ECB_EVENTS_ERRORECB, 0);
    wr(ECB_ECBDATAPTR, data as u32);
    wr(ECB_TASKS_STARTECB, 1);
    let ended = wait_set(ECB_EVENTS_ENDECB);
    let error = rd(ECB_EVENTS_ERRORECB);
    let mut ct = [0u8; 16];
    for (i, b) in ct.iter_mut().enumerate() {
        *b = unsafe { read_volatile(data.add(32 + i)) };
    }
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let pass = ended && error == 0 && ct == CT;
    l.s(b"ecb fips197 ct=")
        .hex(be(&ct[0..4]), 8)
        .s(b"..")
        .hex(be(&ct[12..16]), 8)
        .s(b" end=")
        .hex(ended as u32, 1)
        .ok(pass)
        .send();
    wr(ECB_EVENTS_ENDECB, 0);
    pass
}

pub const TWIM0: u32 = 0x4000_3000;
pub const TWIM_TASKS_STARTRX: u32 = TWIM0;
pub const TWIM_TASKS_STARTTX: u32 = TWIM0 + 0x008;
pub const TWIM_TASKS_STOP: u32 = TWIM0 + 0x014;
pub const TWIM_EVENTS_STOPPED: u32 = TWIM0 + 0x104;
pub const TWIM_EVENTS_ERROR: u32 = TWIM0 + 0x124;
pub const TWIM_EVENTS_LASTRX: u32 = TWIM0 + 0x15C;
pub const TWIM_EVENTS_LASTTX: u32 = TWIM0 + 0x160;
pub const TWIM_SHORTS: u32 = TWIM0 + 0x200;
pub const TWIM_ERRORSRC: u32 = TWIM0 + 0x4C4;
pub const TWIM_ENABLE: u32 = TWIM0 + 0x500;
pub const TWIM_PSEL_SCL: u32 = TWIM0 + 0x508;
pub const TWIM_PSEL_SDA: u32 = TWIM0 + 0x50C;
pub const TWIM_FREQUENCY: u32 = TWIM0 + 0x524;
pub const TWIM_RXD_PTR: u32 = TWIM0 + 0x534;
pub const TWIM_RXD_MAXCNT: u32 = TWIM0 + 0x538;
pub const TWIM_RXD_AMOUNT: u32 = TWIM0 + 0x53C;
pub const TWIM_TXD_PTR: u32 = TWIM0 + 0x544;
pub const TWIM_TXD_MAXCNT: u32 = TWIM0 + 0x548;
pub const TWIM_TXD_AMOUNT: u32 = TWIM0 + 0x54C;
pub const TWIM_ADDRESS: u32 = TWIM0 + 0x588;
const TWIM_SHORT_LASTTX_STARTRX: u32 = 1 << 7;
const TWIM_SHORT_LASTTX_STOP: u32 = 1 << 9;
const TWIM_SHORT_LASTRX_STOP: u32 = 1 << 12;
const TWIM_ERRORSRC_ANACK: u32 = 1 << 1;
/// TMP102 at its ADD0 = GND address; 0x49 has no device on the bus.
const TMP102_ADDR: u32 = 0x48;
const ABSENT_ADDR: u32 = 0x49;

#[repr(C, align(4))]
struct TwimBuf([u8; 4]);
static mut TWIM_TX: TwimBuf = TwimBuf([0; 4]);
static mut TWIM_RX: TwimBuf = TwimBuf([0; 4]);

/// One TWIM0 transaction through EasyDMA: write `tx`, then (if `rx_len` > 0)
/// a repeated START and read `rx_len` bytes, then STOP via SHORTS. Returns
/// (stopped, TXD.AMOUNT, RXD.AMOUNT, first two RX bytes big-endian).
fn twim_xfer(addr: u32, tx: &[u8], rx_len: u32) -> (bool, u32, u32, u32) {
    let txb = core::ptr::addr_of_mut!(TWIM_TX) as *mut u8;
    let rxb = core::ptr::addr_of_mut!(TWIM_RX) as *mut u8;
    for (i, b) in tx.iter().enumerate() {
        unsafe { write_volatile(txb.add(i), *b) };
    }
    for i in 0..4 {
        unsafe { write_volatile(rxb.add(i), 0) };
    }
    for ev in [
        TWIM_EVENTS_STOPPED,
        TWIM_EVENTS_ERROR,
        TWIM_EVENTS_LASTRX,
        TWIM_EVENTS_LASTTX,
    ] {
        wr(ev, 0);
    }
    wr(TWIM_ADDRESS, addr);
    wr(TWIM_TXD_PTR, txb as u32);
    wr(TWIM_TXD_MAXCNT, tx.len() as u32);
    wr(TWIM_RXD_PTR, rxb as u32);
    wr(TWIM_RXD_MAXCNT, rx_len);
    wr(
        TWIM_SHORTS,
        if rx_len > 0 {
            TWIM_SHORT_LASTTX_STARTRX | TWIM_SHORT_LASTRX_STOP
        } else {
            TWIM_SHORT_LASTTX_STOP
        },
    );
    wr(TWIM_TASKS_STARTTX, 1);
    let stopped = wait_set(TWIM_EVENTS_STOPPED);
    let word = unsafe { u32::from(read_volatile(rxb)) << 8 | u32::from(read_volatile(rxb.add(1))) };
    (stopped, rd(TWIM_TXD_AMOUNT), rd(TWIM_RXD_AMOUNT), word)
}

/// TWIM0 (PS v1.7 §6.31) against the TMP102 test fixture on `twi0`:
///   * write CONFIG = 0xFFFF; the TMP102 write mask (SBOS397I Tables
///     6-10/6-11) keeps R1:R0 = 11, AL = 1 and D3:D0 = 0, so it reads back
///     0xFFF0 through a write + repeated-START read;
///   * the first TEMP read is the 25.0 degC reset value 0x1900;
///   * an address with no device sets EVENTS_ERROR and ERRORSRC.ANACK, which
///     is write-1-to-clear.
/// P0.26/P0.27 are the nRF52840 DK's Arduino SCL/SDA pads; no other block
/// uses them.
pub fn check_twim0_tmp102(l: &mut Line) -> bool {
    wr(TWIM_ENABLE, 0);
    wr(TWIM_PSEL_SCL, 26);
    wr(TWIM_PSEL_SDA, 27);
    wr(TWIM_FREQUENCY, 0x0640_0000); // K400
    wr(TWIM_ENABLE, 6);

    let (w_stop, w_tx, _, _) = twim_xfer(TMP102_ADDR, &[0x01, 0xFF, 0xFF], 0);
    let (c_stop, c_tx, c_rx, cfg) = twim_xfer(TMP102_ADDR, &[0x01], 2);
    let (t_stop, _, t_rx, temp) = twim_xfer(TMP102_ADDR, &[0x00], 2);
    let clean = rd(TWIM_ERRORSRC) == 0;

    // Absent address: no SHORTS, so firmware ends the transfer on ERROR as
    // nrfx does.
    for ev in [TWIM_EVENTS_STOPPED, TWIM_EVENTS_ERROR, TWIM_EVENTS_LASTTX] {
        wr(ev, 0);
    }
    wr(TWIM_SHORTS, 0);
    wr(TWIM_ADDRESS, ABSENT_ADDR);
    wr(TWIM_TXD_MAXCNT, 1);
    wr(TWIM_TASKS_STARTTX, 1);
    let error = wait_set(TWIM_EVENTS_ERROR);
    let anack = rd(TWIM_ERRORSRC) & TWIM_ERRORSRC_ANACK != 0;
    wr(TWIM_TASKS_STOP, 1);
    let n_stop = wait_set(TWIM_EVENTS_STOPPED);
    wr(TWIM_ERRORSRC, TWIM_ERRORSRC_ANACK);
    let cleared = rd(TWIM_ERRORSRC) == 0;
    wr(TWIM_ENABLE, 0);

    let pass = w_stop
        && w_tx == 3
        && c_stop
        && c_tx == 1
        && c_rx == 2
        && cfg == 0xFFF0
        && t_stop
        && t_rx == 2
        && temp == 0x1900
        && clean
        && error
        && anack
        && n_stop
        && cleared;
    l.s(b"twim0 tmp102 cfg=")
        .hex(cfg, 4)
        .s(b" temp=")
        .hex(temp, 4)
        .s(b" anack=")
        .hex((error && anack) as u32, 1)
        .s(b" w1c=")
        .hex(cleared as u32, 1)
        .ok(pass)
        .send();
    pass
}

pub const PWM0: u32 = 0x4001_C000;
pub const PWM_TASKS_STOP: u32 = PWM0 + 0x004;
pub const PWM_TASKS_SEQSTART0: u32 = PWM0 + 0x008;
pub const PWM_EVENTS_STOPPED: u32 = PWM0 + 0x104;
pub const PWM_EVENTS_SEQEND0: u32 = PWM0 + 0x110;
pub const PWM_ENABLE: u32 = PWM0 + 0x500;
pub const PWM_MODE: u32 = PWM0 + 0x504;
pub const PWM_COUNTERTOP: u32 = PWM0 + 0x508;
pub const PWM_PRESCALER: u32 = PWM0 + 0x50C;
pub const PWM_DECODER: u32 = PWM0 + 0x510;
pub const PWM_LOOP: u32 = PWM0 + 0x514;
pub const PWM_SEQ0_PTR: u32 = PWM0 + 0x520;
pub const PWM_SEQ0_CNT: u32 = PWM0 + 0x524;
pub const PWM_SEQ0_REFRESH: u32 = PWM0 + 0x528;
pub const PWM_SEQ0_ENDDELAY: u32 = PWM0 + 0x52C;
pub const PWM_PSEL_OUT0: u32 = PWM0 + 0x560;
/// Free on the bare chip row: no other block drives P0.15.
pub const PWM_PIN: u32 = 15;
const PWM_SAMPLES: u32 = 8000;

#[repr(C, align(4))]
struct PwmSeq([u16; 2]);
static mut PWM_SEQ: PwmSeq = PwmSeq([0; 2]);

/// PWM0 (PS v1.7 §6.17) on P0.15: MODE Up, PWM_CLK 16 MHz, COUNTERTOP 400
/// (25 us period), one Common step 0x8000 | 100 read through EasyDMA.
/// POLARITY bit 15 set starts each period high and falls at COMP, so the pad
/// is high 100/400 = 25% of the time. GPIO IN samples the pad 8000 times
/// (many periods); the duty must be 20..30% with at least 10 rising edges.
/// After TASKS_STOP the pad falls back to the GPIO OUT latch (0).
/// Returns (pass, high samples, rising edges).
pub fn check_pwm0_duty(l: &mut Line) -> (bool, u32, u32) {
    let bit = 1 << PWM_PIN;
    wr(GPIO_OUTCLR, bit);
    wr(GPIO_PIN_CNF + PWM_PIN * 4, 1);
    let seq = core::ptr::addr_of_mut!(PWM_SEQ) as *mut u16;
    unsafe { write_volatile(seq, 0x8000 | 100) };
    wr(PWM_PSEL_OUT0, PWM_PIN);
    wr(PWM_ENABLE, 1);
    wr(PWM_MODE, 0);
    wr(PWM_PRESCALER, 0);
    wr(PWM_COUNTERTOP, 400);
    wr(PWM_DECODER, 0);
    wr(PWM_LOOP, 0);
    wr(PWM_SEQ0_PTR, seq as u32);
    wr(PWM_SEQ0_CNT, 1);
    wr(PWM_SEQ0_REFRESH, 0);
    wr(PWM_SEQ0_ENDDELAY, 0);
    wr(PWM_EVENTS_SEQEND0, 0);
    wr(PWM_EVENTS_STOPPED, 0);
    wr(PWM_TASKS_SEQSTART0, 1);
    let seqend = wait_set(PWM_EVENTS_SEQEND0);

    let mut high = 0u32;
    let mut rises = 0u32;
    let mut last = rd(GPIO_IN) & bit;
    // Branch-free, so every sample costs the same cycles and the sample
    // count is proportional to time: a longer "high" path under-counts high.
    for _ in 0..PWM_SAMPLES {
        let v = rd(GPIO_IN) & bit;
        high += v >> PWM_PIN;
        rises += (v & !last) >> PWM_PIN;
        last = v;
    }

    wr(PWM_TASKS_STOP, 1);
    let stopped = wait_set(PWM_EVENTS_STOPPED);
    settle();
    let idle = rd(GPIO_IN) & bit;
    wr(PWM_ENABLE, 0);
    wr(PWM_PSEL_OUT0, 0xFFFF_FFFF);

    let duty_ok = high * 10 >= PWM_SAMPLES * 2 && high * 10 <= PWM_SAMPLES * 3;
    let pass = seqend && duty_ok && rises >= 10 && stopped && idle == 0;
    l.s(b"pwm0 p0.15 seqend=")
        .hex(seqend as u32, 1)
        .s(b" duty=")
        .hex(duty_ok as u32, 1)
        .s(b" rises=")
        .hex((rises >= 10) as u32, 1)
        .s(b" stop=")
        .hex(stopped as u32, 1)
        .s(b" idle=")
        .hex(idle, 1)
        .ok(pass)
        .send();
    (pass, high, rises)
}

/// Convert the held 1.65 V input on AIN3 into one 12-bit EasyDMA sample.
/// CONFIG=0 selects single-ended, gain 1/6 and the 600 mV internal reference:
/// floor(1650 mV * (1/6) * 4096 / 600 mV) = 1877 (0x755).
pub fn check_saadc_ain3(l: &mut Line) -> bool {
    // CH[0]: AIN3 is PSELP=4; PSELN is ignored in single-ended mode.
    wr(SAADC + 0x510, 4);
    wr(SAADC + 0x514, 0);
    wr(SAADC + 0x518, 0);
    wr(SAADC + 0x5F0, 2); // 12-bit resolution
    wr(SAADC + 0x62C, SAADC_RESULT);
    wr(SAADC + 0x630, 1);
    for event in [0x100, 0x104, 0x108, 0x10C] {
        wr(SAADC + event, 0);
    }
    wr(SAADC + 0x500, 1); // ENABLE
    wr(SAADC, 1); // TASKS_START
    let started = rd(SAADC + 0x100) == 1;
    wr(SAADC + 0x004, 1); // TASKS_SAMPLE
    let ended = wait_set(SAADC + 0x104);
    let amount = rd(SAADC + 0x634);
    let sample = rd(SAADC_RESULT) & 0xFFFF;
    let done = rd(SAADC + 0x108) == 1;
    let pass = started && ended && amount == 1 && sample == 0x755 && done;
    l.s(b"saadc ain3=")
        .hex(sample, 8)
        .s(b" amount=")
        .hex(amount, 1)
        .s(b" end=")
        .hex(ended as u32, 1)
        .ok(pass)
        .send();
    wr(SAADC + 0x008, 1); // TASKS_STOP
    wr(SAADC + 0x500, 0);
    pass
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
