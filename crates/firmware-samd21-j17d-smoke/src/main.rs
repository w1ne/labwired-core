// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! ATSAMD21J17D smoke firmware — the SAM D21 bring-up sequence, written
//! against the register map rather than a HAL.
//!
//! Sequence (the same order the Arduino SAMD core's `SystemInit()` uses):
//!   1. NVMCTRL wait states, before the clock goes up.
//!   2. Wait for OSC8M in SYSCTRL.PCLKSR — the ready-flag poll that is a hang
//!      on any model whose PCLKSR reads zero.
//!   3. GCLK generator 0 from OSC8M, then route SERCOM0_CORE to it.
//!   4. PM.APBCMASK — SERCOM0's APB clock is NOT on at reset (APBCMASK resets
//!      to 0x0001_0000, ADC only).
//!   5. PORT.WRCONFIG — mux PA10/PA11 to peripheral function C (SERCOM0
//!      PAD[2]/PAD[3]) and make PA17, the Arduino Zero LED, an output.
//!   6. SERCOM0 in USART mode: 8N1, TX+RX enabled, then ENABLE.
//!   7. Print, then toggle the LED so a GPIO assertion has something to watch.
//!   8. Exercise the rest of what Renode's `atsamd21j17d-aft.repl` models and
//!      this chip profile claims: PORT group B, TC4 (16-bit MFRQ with the
//!      IRQ 19 pend), TC6 (8-bit one-shot down-counter), the SERCOM1 and
//!      SERCOM3 USARTs and the RTC (MODE2 calendar alarm with the IRQ 3
//!      pend, MODE0 clear-on-match, MODE1 period). A summary word lands at
//!      0x2000_2000.
//!
//! The register map is shared with the SAM D21 family and cross-checked against
//! `ATSAMD21G18A.svd` (Microchip, Apache-2.0), vendored at
//! `tests/fixtures/real_world/atsamd21g18a.svd`.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Peripheral bases ─────────────────────────────────────────────────────────
const PM_BASE: u32 = 0x4000_0400;
const SYSCTRL_BASE: u32 = 0x4000_0800;
const GCLK_BASE: u32 = 0x4000_0C00;
const NVMCTRL_BASE: u32 = 0x4100_4000;
const PORTA_BASE: u32 = 0x4100_4400;
const SERCOM0_BASE: u32 = 0x4200_0800;

// ── NVMCTRL ──────────────────────────────────────────────────────────────────
const NVMCTRL_CTRLB: *mut u32 = (NVMCTRL_BASE + 0x04) as *mut u32;
/// `CTRLB.RWS` is bits [4:1]. One wait state is what the datasheet requires
/// once the core runs above 24 MHz.
const NVMCTRL_CTRLB_RWS_HALF: u32 = 1 << 1;

// ── SYSCTRL ──────────────────────────────────────────────────────────────────
const SYSCTRL_PCLKSR: *const u32 = (SYSCTRL_BASE + 0x0C) as *const u32;
const PCLKSR_OSC8MRDY: u32 = 1 << 3;

// ── GCLK ─────────────────────────────────────────────────────────────────────
const GCLK_STATUS: *const u8 = (GCLK_BASE + 0x01) as *const u8;
const GCLK_CLKCTRL: *mut u16 = (GCLK_BASE + 0x02) as *mut u16;
const GCLK_GENCTRL: *mut u32 = (GCLK_BASE + 0x04) as *mut u32;
const GCLK_GENDIV: *mut u32 = (GCLK_BASE + 0x08) as *mut u32;
const GCLK_STATUS_SYNCBUSY: u8 = 1 << 7;
/// `GENCTRL.SRC` 0x06 = OSC8M (SVD `GCLK_GENCTRL.SRC` enum).
const GCLK_SRC_OSC8M: u32 = 0x06;
const GCLK_GENCTRL_GENEN: u32 = 1 << 16;
/// `CLKCTRL.ID` 0x14 = SERCOM0_CORE (SVD `GCLK_CLKCTRL.ID` enum).
const GCLK_ID_SERCOM0_CORE: u16 = 0x14;
const GCLK_CLKCTRL_CLKEN: u16 = 1 << 14;

// ── PM ───────────────────────────────────────────────────────────────────────
const PM_APBCMASK: *mut u32 = (PM_BASE + 0x20) as *mut u32;
/// `APBCMASK.SERCOM0_` is bit 2. APBCMASK resets to 0x0001_0000 (ADC only), so
/// this bit is genuinely off until firmware sets it.
const PM_APBCMASK_SERCOM0: u32 = 1 << 2;

// ── PORT (GROUP 0 = PA) ──────────────────────────────────────────────────────
const PORTA_DIRSET: *mut u32 = (PORTA_BASE + 0x08) as *mut u32;
const PORTA_OUTSET: *mut u32 = (PORTA_BASE + 0x18) as *mut u32;
const PORTA_OUTTGL: *mut u32 = (PORTA_BASE + 0x1C) as *mut u32;
const PORTA_WRCONFIG: *mut u32 = (PORTA_BASE + 0x28) as *mut u32;
const WRCONFIG_PMUXEN: u32 = 1 << 16;
const WRCONFIG_INEN: u32 = 1 << 17;
const WRCONFIG_WRPMUX: u32 = 1 << 28;
const WRCONFIG_WRPINCFG: u32 = 1 << 30;
/// `WRCONFIG.PMUX` 0x2 = peripheral function **C**, which is where SERCOM0's
/// pads live on PA10/PA11 (SAM D21 datasheet, "I/O Multiplexing and
/// Considerations").
const PMUX_FUNCTION_C: u32 = 0x2;
/// The Arduino Zero / Feather M0 user LED.
const LED_PIN: u32 = 17;

// ── SERCOM0 (USART mode) ─────────────────────────────────────────────────────
const SERCOM0_CTRLA: *mut u32 = SERCOM0_BASE as *mut u32;
const SERCOM0_CTRLB: *mut u32 = (SERCOM0_BASE + 0x04) as *mut u32;
const SERCOM0_BAUD: *mut u16 = (SERCOM0_BASE + 0x0C) as *mut u16;
const SERCOM0_INTFLAG: *const u8 = (SERCOM0_BASE + 0x18) as *const u8;
const SERCOM0_SYNCBUSY: *const u32 = (SERCOM0_BASE + 0x1C) as *const u32;
const SERCOM0_DATA: *mut u16 = (SERCOM0_BASE + 0x28) as *mut u16;

/// `CTRLA.MODE` 0x1 = USART with the internal clock.
const CTRLA_MODE_USART_INT_CLK: u32 = 0x1 << 2;
/// `CTRLA.RXPO` [21:20] = 3 → RX on PAD[3] (PA11).
const CTRLA_RXPO_PAD3: u32 = 0x3 << 20;
/// `CTRLA.TXPO` [17:16] = 1 → TX on PAD[2] (PA10).
const CTRLA_TXPO_PAD2: u32 = 0x1 << 16;
/// `CTRLA.DORD` bit 30 = 1 → LSB first, which is what a UART frame is.
const CTRLA_DORD_LSB: u32 = 1 << 30;
const CTRLA_ENABLE: u32 = 1 << 1;
const CTRLB_TXEN: u32 = 1 << 16;
const CTRLB_RXEN: u32 = 1 << 17;
const INTFLAG_DRE: u8 = 1 << 0;
/// 115200 baud from an 8 MHz core clock, 16× oversampling:
/// `65536 * (1 - 16 * 115200 / 8_000_000)` = 50438.
const BAUD_115200_AT_8MHZ: u16 = 50438;

/// Spin until GCLK has finished synchronising. Bounded, so a twin that never
/// clears SYNCBUSY fails the smoke run instead of hanging it.
fn gclk_sync() {
    for _ in 0..10_000 {
        if unsafe { read_volatile(GCLK_STATUS) } & GCLK_STATUS_SYNCBUSY == 0 {
            return;
        }
    }
}

fn sercom_sync() {
    for _ in 0..10_000 {
        if unsafe { read_volatile(SERCOM0_SYNCBUSY) } == 0 {
            return;
        }
    }
}

fn putc(byte: u8) {
    // The DRE spin every SAM D21 serial driver performs.
    for _ in 0..10_000 {
        if unsafe { read_volatile(SERCOM0_INTFLAG) } & INTFLAG_DRE != 0 {
            break;
        }
    }
    unsafe { write_volatile(SERCOM0_DATA, u16::from(byte)) };
}

fn puts(s: &str) {
    for byte in s.as_bytes() {
        putc(*byte);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // 1. Flash wait states before the clock rises.
        write_volatile(NVMCTRL_CTRLB, NVMCTRL_CTRLB_RWS_HALF);

        // 2. Wait for the 8 MHz RC oscillator. On silicon this is a handful of
        //    cycles; on a twin whose PCLKSR reads 0 it never returns.
        for _ in 0..10_000 {
            if read_volatile(SYSCTRL_PCLKSR) & PCLKSR_OSC8MRDY != 0 {
                break;
            }
        }

        // 3. GCLK generator 0 = OSC8M, undivided; then SERCOM0_CORE on gen 0.
        write_volatile(GCLK_GENDIV, 0); // ID 0, DIV 0
        gclk_sync();
        write_volatile(GCLK_GENCTRL, (GCLK_SRC_OSC8M << 8) | GCLK_GENCTRL_GENEN);
        gclk_sync();
        write_volatile(GCLK_CLKCTRL, GCLK_ID_SERCOM0_CORE | GCLK_CLKCTRL_CLKEN);
        gclk_sync();

        // 4. SERCOM0's APB clock.
        let mask = read_volatile(PM_APBCMASK) | PM_APBCMASK_SERCOM0;
        write_volatile(PM_APBCMASK, mask);

        // 5. PA10 (TX) and PA11 (RX) to peripheral function C, in one store —
        //    PINMASK bits 10 and 11, low half of the port.
        write_volatile(
            PORTA_WRCONFIG,
            (1 << 10)
                | (1 << 11)
                | WRCONFIG_PMUXEN
                | WRCONFIG_INEN
                | (PMUX_FUNCTION_C << 24)
                | WRCONFIG_WRPMUX
                | WRCONFIG_WRPINCFG,
        );
        //    PA17: plain output, LED off.
        write_volatile(PORTA_DIRSET, 1 << LED_PIN);

        // 6. SERCOM0 as a USART. CTRLB before CTRLA.ENABLE, as the datasheet
        //    requires — CTRLB is enable-protected.
        write_volatile(
            SERCOM0_CTRLA,
            CTRLA_MODE_USART_INT_CLK | CTRLA_RXPO_PAD3 | CTRLA_TXPO_PAD2 | CTRLA_DORD_LSB,
        );
        write_volatile(SERCOM0_CTRLB, CTRLB_TXEN | CTRLB_RXEN);
        sercom_sync();
        write_volatile(SERCOM0_BAUD, BAUD_115200_AT_8MHZ);
        let ctrla = read_volatile(SERCOM0_CTRLA) | CTRLA_ENABLE;
        write_volatile(SERCOM0_CTRLA, ctrla);
        sercom_sync();
    }

    // 7. Proof of life on both observable surfaces: the console and a pad.
    puts("OK\n");
    puts("samd21 sercom0 up\n");

    unsafe { write_volatile(PORTA_OUTSET, 1 << LED_PIN) };
    puts("led on\n");

    // 8. The rest of the surface the Renode platform models: PORT group B,
    //    TC4 (16-bit MFRQ) and TC6 (8-bit one-shot down-counter), and the
    //    SERCOM1/SERCOM3 USARTs. Every check feeds one bit of the summary word.
    let mut pass = 0u32;
    pass |= u32::from(check_portb()) << 0;
    pass |= u32::from(check_tc4()) << 1;
    pass |= u32::from(check_tc6()) << 2;
    pass |= u32::from(check_usart("sercom1", SERCOM1_BASE, PM_APBCMASK_SERCOM1)) << 3;
    pass |= u32::from(check_usart("sercom3", SERCOM3_BASE, PM_APBCMASK_SERCOM3)) << 4;
    pass |= u32::from(check_rtc()) << 5;
    unsafe { write_volatile(SUMMARY, SUMMARY_TAG | pass) };
    if pass == 0x3f {
        puts("SAMD21 PASS\n");
    } else {
        puts("SAMD21 FAIL ");
        put_hex(pass);
        puts("\n");
    }

    loop {
        unsafe { write_volatile(PORTA_OUTTGL, 1 << LED_PIN) };
        for _ in 0..1_000 {
            cortex_m::asm::nop();
        }
    }
}

// ── Extended checks ─────────────────────────────────────────────────────────

/// Summary word: `0x5D21_0000 | pass bits`, all six checks = `0x5D21_003F`.
const SUMMARY: *mut u32 = 0x2000_2000 as *mut u32;
const SUMMARY_TAG: u32 = 0x5D21_0000;

const PORTB_BASE: u32 = 0x4100_4480;
const PORTB_DIR: *const u32 = PORTB_BASE as *const u32;
const PORTB_DIRSET: *mut u32 = (PORTB_BASE + 0x08) as *mut u32;
const PORTB_OUT: *const u32 = (PORTB_BASE + 0x10) as *const u32;
const PORTB_OUTCLR: *mut u32 = (PORTB_BASE + 0x14) as *mut u32;
const PORTB_OUTSET: *mut u32 = (PORTB_BASE + 0x18) as *mut u32;

const TC4_BASE: u32 = 0x4200_3000;
const TC6_BASE: u32 = 0x4200_3800;
/// `APBCMASK.TCn` = bit (n + 8): TC4 bit 12, TC6 bit 14.
const PM_APBCMASK_TC4: u32 = 1 << 12;
const PM_APBCMASK_TC6: u32 = 1 << 14;
/// `CLKCTRL.ID` 0x1C = TC4_TC5, 0x1D = TC6_TC7.
const GCLK_ID_TC4_TC5: u16 = 0x1C;
const GCLK_ID_TC6_TC7: u16 = 0x1D;
const TC_CTRLA_ENABLE: u16 = 1 << 1;
const TC_CTRLA_MODE_COUNT8: u16 = 1 << 2;
const TC_CTRLA_WAVEGEN_MFRQ: u16 = 1 << 5;
const TC_CTRLA_PRESCALER_DIV8: u16 = 3 << 8;
const TC_CTRLB_DIR: u8 = 1 << 0;
const TC_CTRLB_ONESHOT: u8 = 1 << 2;
const TC_CMD_RETRIGGER: u8 = 1 << 6;
const TC_CMD_STOP: u8 = 2 << 6;
const TC_INT_OVF: u8 = 1 << 0;
const TC_INT_MC0: u8 = 1 << 4;
const TC_STATUS_STOP: u8 = 1 << 3;
/// TC4 is IRQ 19 on the SAM D21 interrupt line map.
const TC4_IRQ: u32 = 19;
const NVIC_ISPR: *const u32 = 0xE000_E200 as *const u32;
const NVIC_ICPR: *mut u32 = 0xE000_E280 as *mut u32;

const SERCOM1_BASE: u32 = 0x4200_0C00;
const SERCOM3_BASE: u32 = 0x4200_1400;
const PM_APBCMASK_SERCOM1: u32 = 1 << 3;
const PM_APBCMASK_SERCOM3: u32 = 1 << 5;
const INTFLAG_TXC: u8 = 1 << 1;

fn tc_reg8(base: u32, off: u32) -> *mut u8 {
    (base + off) as *mut u8
}
fn tc_reg16(base: u32, off: u32) -> *mut u16 {
    (base + off) as *mut u16
}

fn put_hex(v: u32) {
    for i in (0..8).rev() {
        let n = ((v >> (i * 4)) & 0xf) as u8;
        putc(if n < 10 { b'0' + n } else { b'a' + n - 10 });
    }
}

fn put_hex8(v: u8) {
    for n in [v >> 4, v & 0xf] {
        putc(if n < 10 { b'0' + n } else { b'a' + n - 10 });
    }
}

fn put_bit(b: bool) {
    putc(if b { b'1' } else { b'0' });
}

fn apb_enable(bit: u32) {
    unsafe {
        let mask = read_volatile(PM_APBCMASK) | bit;
        write_volatile(PM_APBCMASK, mask);
    }
}

fn gclk_route(id: u16) {
    unsafe { write_volatile(GCLK_CLKCTRL, id | GCLK_CLKCTRL_CLKEN) };
    gclk_sync();
}

/// PB10 as an output: DIRSET/OUTSET/OUTCLR with DIR/OUT readback.
fn check_portb() -> bool {
    unsafe {
        write_volatile(PORTB_DIRSET, 1 << 10);
        write_volatile(PORTB_OUTSET, 1 << 10);
        let dir = read_volatile(PORTB_DIR);
        let set = read_volatile(PORTB_OUT);
        write_volatile(PORTB_OUTCLR, 1 << 10);
        let clr = read_volatile(PORTB_OUT);
        puts("portb dir=");
        put_hex(dir);
        puts(" out=");
        put_hex(set);
        puts(" clr=");
        put_hex(clr);
        puts("\n");
        dir & (1 << 10) != 0 && set & (1 << 10) != 0 && clr & (1 << 10) == 0
    }
}

/// TC4: COUNT16, MFRQ (TOP = CC0 = 999), DIV8. The counter must advance, hit
/// the CC0 match, wrap with OVF, pend IRQ 19 in the NVIC, and release it once
/// INTFLAG is cleared.
fn check_tc4() -> bool {
    apb_enable(PM_APBCMASK_TC4);
    gclk_route(GCLK_ID_TC4_TC5);
    let b = TC4_BASE;
    unsafe {
        let ctrla = TC_CTRLA_WAVEGEN_MFRQ | TC_CTRLA_PRESCALER_DIV8;
        write_volatile(tc_reg16(b, 0x00), ctrla);
        write_volatile(tc_reg16(b, 0x18), 999);
        write_volatile(tc_reg8(b, 0x0D), TC_INT_OVF | TC_INT_MC0);
        let cc0 = read_volatile(tc_reg16(b, 0x18));
        write_volatile(tc_reg16(b, 0x00), ctrla | TC_CTRLA_ENABLE);
        let mut advanced = false;
        let mut flags = 0u8;
        for _ in 0..20_000 {
            if read_volatile(tc_reg16(b, 0x10)) != 0 {
                advanced = true;
            }
            flags = read_volatile(tc_reg8(b, 0x0E));
            if flags & TC_INT_OVF != 0 {
                break;
            }
        }
        let pended = read_volatile(NVIC_ISPR) & (1 << TC4_IRQ) != 0;
        write_volatile(tc_reg8(b, 0x0E), TC_INT_OVF | TC_INT_MC0);
        write_volatile(NVIC_ICPR, 1 << TC4_IRQ);
        let cleared = read_volatile(tc_reg8(b, 0x0E)) & (TC_INT_OVF | TC_INT_MC0);
        // Stop it so it does not keep re-pending through the rest of the run.
        write_volatile(tc_reg8(b, 0x05), TC_CMD_STOP);
        let stopped = read_volatile(tc_reg8(b, 0x0F)) & TC_STATUS_STOP != 0;
        puts("tc4 cc0=");
        put_hex(u32::from(cc0));
        puts(" adv=");
        put_bit(advanced);
        puts(" flags=");
        put_hex8(flags);
        puts(" nvic19=");
        put_bit(pended);
        puts(" cleared=");
        put_hex8(cleared);
        puts(" stop=");
        put_bit(stopped);
        puts("\n");
        cc0 == 999
            && advanced
            && flags & (TC_INT_OVF | TC_INT_MC0) == (TC_INT_OVF | TC_INT_MC0)
            && pended
            && cleared == 0
            && stopped
    }
}

/// TC6: COUNT8, NFRQ (TOP = PER = 0x20), counting DOWN, ONESHOT. It must
/// underflow once, reload PER, set OVF and stop by itself; RETRIGGER restarts
/// it and CMD STOP halts it again.
fn check_tc6() -> bool {
    apb_enable(PM_APBCMASK_TC6);
    gclk_route(GCLK_ID_TC6_TC7);
    let b = TC6_BASE;
    unsafe {
        write_volatile(tc_reg16(b, 0x00), TC_CTRLA_MODE_COUNT8);
        write_volatile(tc_reg8(b, 0x14), 0x20); // PER
        write_volatile(tc_reg8(b, 0x05), TC_CTRLB_DIR | TC_CTRLB_ONESHOT);
        write_volatile(tc_reg8(b, 0x10), 0x20);
        let reset_stop = read_volatile(tc_reg8(b, 0x0F)) & TC_STATUS_STOP != 0;
        write_volatile(tc_reg16(b, 0x00), TC_CTRLA_MODE_COUNT8 | TC_CTRLA_ENABLE);
        let mut status = 0u8;
        for _ in 0..20_000 {
            status = read_volatile(tc_reg8(b, 0x0F));
            if status & TC_STATUS_STOP != 0 {
                break;
            }
        }
        let flags = read_volatile(tc_reg8(b, 0x0E));
        let count = read_volatile(tc_reg8(b, 0x10));
        write_volatile(tc_reg8(b, 0x0E), TC_INT_OVF);
        write_volatile(tc_reg8(b, 0x05), TC_CMD_RETRIGGER);
        let rerun = read_volatile(tc_reg8(b, 0x0F)) & TC_STATUS_STOP == 0;
        write_volatile(tc_reg8(b, 0x05), TC_CMD_STOP);
        let halted = read_volatile(tc_reg8(b, 0x0F)) & TC_STATUS_STOP != 0;
        puts("tc6 reset_stop=");
        put_bit(reset_stop);
        puts(" oneshot_stop=");
        put_bit(status & TC_STATUS_STOP != 0);
        puts(" ovf=");
        put_bit(flags & TC_INT_OVF != 0);
        puts(" count=");
        put_hex8(count);
        puts(" retrigger=");
        put_bit(rerun);
        puts(" halted=");
        put_bit(halted);
        puts("\n");
        reset_stop
            && status & TC_STATUS_STOP != 0
            && flags & TC_INT_OVF != 0
            && count == 0x20
            && rerun
            && halted
    }
}

/// A second SERCOM in USART mode: CTRLA readback, DRE once enabled, TXC after
/// one byte, TXC W1C.
fn check_usart(name: &str, base: u32, apb_bit: u32) -> bool {
    apb_enable(apb_bit);
    unsafe {
        let ctrla = base as *mut u32;
        let ctrlb = (base + 0x04) as *mut u32;
        let baud = (base + 0x0C) as *mut u16;
        let intflag = (base + 0x18) as *mut u8;
        let data = (base + 0x28) as *mut u16;
        let mode = CTRLA_MODE_USART_INT_CLK | CTRLA_DORD_LSB;
        write_volatile(ctrla, mode);
        write_volatile(ctrlb, CTRLB_TXEN);
        write_volatile(baud, BAUD_115200_AT_8MHZ);
        write_volatile(ctrla, mode | CTRLA_ENABLE);
        let readback = read_volatile(ctrla);
        let dre = read_volatile(intflag) & INTFLAG_DRE != 0;
        write_volatile(data, u16::from(b'U'));
        let mut txc = false;
        for _ in 0..20_000 {
            if read_volatile(intflag) & INTFLAG_TXC != 0 {
                txc = true;
                break;
            }
        }
        write_volatile(intflag, INTFLAG_TXC);
        let txc_cleared = read_volatile(intflag) & INTFLAG_TXC == 0;
        puts(name);
        puts(" ctrla=");
        put_hex(readback);
        puts(" dre=");
        put_bit(dre);
        puts(" txc=");
        put_bit(txc);
        puts(" w1c=");
        put_bit(txc_cleared);
        puts("\n");
        readback == mode | CTRLA_ENABLE && dre && txc && txc_cleared
    }
}

// ── RTC ─────────────────────────────────────────────────────────────────────

const RTC_BASE: u32 = 0x4000_1400;
const RTC_CTRL: *mut u16 = RTC_BASE as *mut u16;
const RTC_READREQ: *mut u16 = (RTC_BASE + 0x02) as *mut u16;
const RTC_INTENSET: *mut u8 = (RTC_BASE + 0x07) as *mut u8;
const RTC_INTENCLR: *mut u8 = (RTC_BASE + 0x06) as *mut u8;
const RTC_INTFLAG: *mut u8 = (RTC_BASE + 0x08) as *mut u8;
const RTC_STATUS: *const u8 = (RTC_BASE + 0x0A) as *const u8;
const RTC_COUNT32: *mut u32 = (RTC_BASE + 0x10) as *mut u32;
const RTC_COUNT16: *mut u16 = (RTC_BASE + 0x10) as *mut u16;
const RTC_PER: *mut u16 = (RTC_BASE + 0x14) as *mut u16;
const RTC_COMP32: *mut u32 = (RTC_BASE + 0x18) as *mut u32;
const RTC_COMP0_16: *mut u16 = (RTC_BASE + 0x18) as *mut u16;
const RTC_COMP1_16: *mut u16 = (RTC_BASE + 0x1A) as *mut u16;
const RTC_MASK: *mut u8 = (RTC_BASE + 0x1C) as *mut u8;
const RTC_CTRL_SWRST: u16 = 1;
const RTC_CTRL_ENABLE: u16 = 1 << 1;
const RTC_MODE1: u16 = 1 << 2;
const RTC_MODE2: u16 = 2 << 2;
const RTC_CTRL_MATCHCLR: u16 = 1 << 7;
/// `READREQ.RREQ` with `ADDR` = 0x10 (COUNT/CLOCK).
const RTC_READREQ_RREQ: u16 = 1 << 15 | 0x10;
const RTC_INT_CMP0: u8 = 1 << 0;
const RTC_INT_CMP1: u8 = 1 << 1;
const RTC_INT_OVF: u8 = 1 << 7;
/// `MASK.SEL` 6 = YYMMDDHHMMSS.
const RTC_MASK_YYMMDDHHMMSS: u8 = 6;
/// `APBAMASK.RTC_` is bit 5; it resets to 1.
const PM_APBAMASK: *mut u32 = (PM_BASE + 0x18) as *mut u32;
const PM_APBAMASK_RTC: u32 = 1 << 5;
/// `CLKCTRL.ID` 0x04 = RTC; `GENCTRL.SRC` 0x03 = OSCULP32K.
const GCLK_ID_RTC: u16 = 0x04;
const GCLK_SRC_OSCULP32K: u32 = 0x03;
const GCLK_GENCTRL_DIVSEL: u32 = 1 << 20;
const RTC_IRQ: u32 = 3;

fn rtc_sync() {
    for _ in 0..10_000 {
        if unsafe { read_volatile(RTC_STATUS) } & 0x80 == 0 {
            return;
        }
    }
}

/// CTRL.SWRST: legal while enabled, and returns the RTC to MODE0, disabled.
fn rtc_reset() {
    unsafe { write_volatile(RTC_CTRL, RTC_CTRL_SWRST) };
    rtc_sync();
}

/// Poll INTFLAG until any of `want` sets (bounded).
fn rtc_wait(want: u8) -> u8 {
    let mut flags = 0;
    for _ in 0..200_000 {
        flags = unsafe { read_volatile(RTC_INTFLAG) };
        if flags & want != 0 {
            break;
        }
    }
    flags
}

/// `CLOCK`/`ALARM0` field packing (MODE2).
const fn rtc_time(y: u32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> u32 {
    s | mi << 6 | h << 12 | d << 17 | mo << 22 | y << 26
}

/// The RTCZero clock path — GCLK2 = OSCULP32K / 32 (1.024 kHz) routed to the
/// RTC — then three modes. The prescaler is DIV1 rather than RTCZero's
/// DIV1024 so a calendar "second" is ~1 ms and the smoke stays short.
fn check_rtc() -> bool {
    unsafe {
        let m = read_volatile(PM_APBAMASK) | PM_APBAMASK_RTC;
        write_volatile(PM_APBAMASK, m);
        write_volatile(GCLK_GENDIV, 2 | 4 << 8); // GEN2, DIV = 2^(4+1)
        gclk_sync();
        write_volatile(
            GCLK_GENCTRL,
            2 | GCLK_SRC_OSCULP32K << 8 | GCLK_GENCTRL_GENEN | GCLK_GENCTRL_DIVSEL,
        );
        gclk_sync();
        write_volatile(GCLK_CLKCTRL, GCLK_ID_RTC | 2 << 8 | GCLK_CLKCTRL_CLKEN);
        gclk_sync();

        // MODE2: 2024-02-28 23:59:58, alarm at 2024-02-29 00:00:01 (leap day).
        rtc_reset();
        write_volatile(RTC_CTRL, RTC_MODE2);
        rtc_sync();
        write_volatile(RTC_COUNT32, rtc_time(24, 2, 28, 23, 59, 58));
        write_volatile(RTC_COMP32, rtc_time(24, 2, 29, 0, 0, 1));
        write_volatile(RTC_MASK, RTC_MASK_YYMMDDHHMMSS);
        write_volatile(RTC_INTENSET, RTC_INT_CMP0);
        write_volatile(RTC_CTRL, RTC_MODE2 | RTC_CTRL_ENABLE);
        rtc_sync();
        let alarm = rtc_wait(RTC_INT_CMP0) & RTC_INT_CMP0 != 0;
        write_volatile(RTC_READREQ, RTC_READREQ_RREQ);
        rtc_sync();
        let clock = read_volatile(RTC_COUNT32);
        let pended = read_volatile(NVIC_ISPR) & (1 << RTC_IRQ) != 0;
        write_volatile(RTC_INTFLAG, RTC_INT_CMP0);
        write_volatile(RTC_INTENCLR, RTC_INT_CMP0);
        write_volatile(NVIC_ICPR, 1 << RTC_IRQ);
        let released = read_volatile(NVIC_ISPR) & (1 << RTC_IRQ) == 0;
        let mode2 = alarm
            && pended
            && released
            && clock & !0x3f == rtc_time(24, 2, 29, 0, 0, 0)
            && clock & 0x3f >= 2;
        puts("rtc mode2 alarm=");
        put_bit(alarm);
        puts(" nvic3=");
        put_bit(pended);
        puts(" released=");
        put_bit(released);
        puts(" day=");
        put_hex8(((clock >> 17) & 0x1f) as u8);
        puts(" hm=");
        put_hex8(((clock >> 12) & 0x1f) as u8);
        put_hex8(((clock >> 6) & 0x3f) as u8);
        puts("\n");

        // MODE0: COMP0 = 3 with MATCHCLR — CMP0 and OVF arrive together and
        // the counter restarts from 0.
        rtc_reset();
        write_volatile(RTC_CTRL, RTC_CTRL_MATCHCLR);
        rtc_sync();
        write_volatile(RTC_COMP32, 3);
        write_volatile(RTC_CTRL, RTC_CTRL_MATCHCLR | RTC_CTRL_ENABLE);
        rtc_sync();
        let f0 = rtc_wait(RTC_INT_OVF);
        write_volatile(RTC_READREQ, RTC_READREQ_RREQ);
        let c0 = read_volatile(RTC_COUNT32);
        let mode0 = f0 & (RTC_INT_CMP0 | RTC_INT_OVF) == RTC_INT_CMP0 | RTC_INT_OVF && c0 <= 3;
        puts("rtc mode0 flags=");
        put_hex8(f0);
        puts(" wrapped=");
        put_bit(c0 <= 3);
        puts("\n");

        // MODE1: PER = 4, COMP0 = 1, COMP1 = 2 — both compares, then OVF.
        rtc_reset();
        write_volatile(RTC_CTRL, RTC_MODE1);
        rtc_sync();
        write_volatile(RTC_PER, 4);
        write_volatile(RTC_COMP0_16, 1);
        write_volatile(RTC_COMP1_16, 2);
        let per = read_volatile(RTC_PER);
        write_volatile(RTC_CTRL, RTC_MODE1 | RTC_CTRL_ENABLE);
        rtc_sync();
        let f1 = rtc_wait(RTC_INT_OVF);
        write_volatile(RTC_READREQ, RTC_READREQ_RREQ);
        let c1 = read_volatile(RTC_COUNT16);
        let all = RTC_INT_CMP0 | RTC_INT_CMP1 | RTC_INT_OVF;
        let mode1 = per == 4 && f1 & all == all && c1 <= 4;
        puts("rtc mode1 per=");
        put_hex8(per as u8);
        puts(" flags=");
        put_hex8(f1);
        puts(" wrapped=");
        put_bit(c1 <= 4);
        puts("\n");
        rtc_reset();
        mode2 && mode0 && mode1
    }
}
