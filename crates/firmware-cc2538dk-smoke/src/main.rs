// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! CC2538DK (CC2538SF53, Cortex-M3) L1 smoke: UART0 console, a walk over the
//! SmartRF06 LEDs with GPIO_DATA readback, the keys' pad levels, an SSI0
//! loopback self-test, an I2CM/TMP102 transfer, a PKA SRAM readback, a GPTimer0/1 self-test and a heartbeat, in bare
//! registers.
//!
//! Board facts (Contiki-NG `cc2538dk` dev/board.h; RIOT `cc2538dk`):
//!   LED1 red    PC0 (active high) — shared with the USB D+ pull-up, NOT driven
//!   LED2 yellow PC1 (active high)
//!   LED3 green  PC2 (active high)
//!   LED4 orange PC3 (active high)
//!   keys SELECT PA3, LEFT PC4, RIGHT PC5, UP PC6, DOWN PC7 (active low)
//!   UART0 PA0 (RX) / PA1 (TX) through the IOC, 115200 8N1
//!
//! Clock: the firmware stays on the reset clock — the 16-MHz HF-RC oscillator
//! with SYS_DIV = IO_DIV = 001 (SWRU319C Table 7-3: CLOCK_CTRL reset
//! 0x0103_0109, CLOCK_STA reset 0x0101_0109). It reads CLOCK_STA to learn the
//! system clock rather than assuming it.
//!
//! Baud (SWRU319C §18.4.2): UART_CC = 0 selects the SYS_DIV clock, HSE = 0
//! divides by 16, so
//!   BRD   = UARTSysClk / (16 × baud) = 16 000 000 / (16 × 115 200) = 8.6806
//!   IBRD  = 8
//!   FBRD  = integer(0.6806 × 64 + 0.5) = integer(44.06) = 44
//! The code computes the same pair at run time from CLOCK_STA as
//! round(64 × BRD) = round(4 × clk / baud) = 556 = 8 × 64 + 44.
//! Achieved: 16 MHz / (16 × (8 + 44/64)) = 115 108 baud (−0.08 %).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── SYS_CTRL (SWRU319C Table 7-3, base 0x400D_2000) ─────────────────────────
const SYS_CTRL: u32 = 0x400D_2000;
const SYS_CTRL_CLOCK_STA: u32 = SYS_CTRL + 0x04;
const SYS_CTRL_RCGCUART: u32 = SYS_CTRL + 0x28;
/// CLOCK_STA.OSC (bit 16): 1 = 16-MHz HF-RC, 0 = 32-MHz crystal.
const CLOCK_STA_OSC: u32 = 1 << 16;
/// CLOCK_STA.SYS_DIV (bits 2:0): 32 MHz >> SYS_DIV.
const CLOCK_STA_SYS_DIV: u32 = 0x7;
const CLOCK_STA_IO_DIV_SHIFT: u32 = 8;
/// RCGCUART.UART0 (bit 0).
const RCGCUART_UART0: u32 = 1 << 0;
const SYS_CTRL_RCGCSSI: u32 = SYS_CTRL + 0x18;
/// RCGCSSI.SSI0 (bit 0).
const RCGCSSI_SSI0: u32 = 1 << 0;
const SYS_CTRL_RCGCI2C: u32 = SYS_CTRL + 0x20;

// ── IOC (SWRU319C §9.1.1 / Table 9-7, base 0x400D_4000) ─────────────────────
const IOC: u32 = 0x400D_4000;
const IOC_PA1_SEL: u32 = IOC + 0x004;
const IOC_PA0_OVER: u32 = IOC + 0x080;
const IOC_PA1_OVER: u32 = IOC + 0x084;
const IOC_UARTRXD_UART0: u32 = IOC + 0x100;
/// Untouched by this firmware: still at its Table 9-7 reset value (PUE).
const IOC_PD7_OVER: u32 = IOC + 0x0FC;
/// Last Table 9-7 register; INPUT_SEL is bits [4:0], the rest reserved.
const IOC_GPT3OCP2: u32 = IOC + 0x150;
/// Table 9-1: peripheral output select 0x0 = UART0 TXD.
const IOC_SEL_UART0_TXD: u32 = 0x0;
/// IOC_Pxx_OVER: 0x8 = output enable, 0x0 = no override (input, no pull).
const IOC_OVER_OE: u32 = 0x8;
const IOC_OVER_DIS: u32 = 0x0;
/// IOC_UARTRXD_UART0.INPUT_SEL: 0 = PA0.
const IOC_INPUT_PA0: u32 = 0;

// ── GPIO (SWRU319C Table 9-2; ports at Table 4-1 bases) ─────────────────────
const GPIO_A: u32 = 0x400D_9000;
const GPIO_C: u32 = 0x400D_B000;
const GPIO_DIR: u32 = 0x400;
const GPIO_AFSEL: u32 = 0x420;

const LED_YELLOW: u32 = 1; // PC1
const LED_GREEN: u32 = 2; // PC2
const LED_ORANGE: u32 = 3; // PC3
/// The LEDs the smoke drives (PC1..PC3). PC0 (LED1) is left alone.
const LED_MASK: u32 = (1 << LED_YELLOW) | (1 << LED_GREEN) | (1 << LED_ORANGE);

const KEY_SELECT: u32 = 3; // PA3
const KEY_LEFT: u32 = 4; // PC4
const KEY_RIGHT: u32 = 5; // PC5
const KEY_UP: u32 = 6; // PC6
const KEY_DOWN: u32 = 7; // PC7

// ── UART0 (SWRU319C Table 18-4, base 0x4000_C000) ───────────────────────────
const UART0: u32 = 0x4000_C000;
const UART_DR: u32 = UART0;
const UART_FR: u32 = UART0 + 0x018;
const UART_IBRD: u32 = UART0 + 0x024;
const UART_FBRD: u32 = UART0 + 0x028;
const UART_LCRH: u32 = UART0 + 0x02C;
const UART_CTL: u32 = UART0 + 0x030;
const UART_CC: u32 = UART0 + 0xFC8;
/// FR.TXFF (bit 5): transmit FIFO full.
const UART_FR_TXFF: u32 = 1 << 5;
/// LCRH.WLEN = 0x3 (8 bits), no parity, one stop, FIFOs off: 0x60 (§18.5).
const UART_LCRH_8N1: u32 = 0x3 << 5;
/// CTL: UARTEN (0), TXE (8), RXE (9).
const UART_CTL_EN: u32 = (1 << 0) | (1 << 8) | (1 << 9);
/// UART_CC.CS = 0: baud clock from the SYS_DIV setting.
const UART_CC_SYSDIV: u32 = 0x0;

const BAUD: u32 = 115_200;

// ── SSI0 (SWRU319C §19, base 0x4000_8000) ───────────────────────────────────
const SSI0: u32 = 0x4000_8000;
const SSI_CR0: u32 = SSI0;
const SSI_CR1: u32 = SSI0 + 0x004;
const SSI_DR: u32 = SSI0 + 0x008;
const SSI_SR: u32 = SSI0 + 0x00C;
const SSI_CPSR: u32 = SSI0 + 0x010;
const SSI_RIS: u32 = SSI0 + 0x018;
const SSI_ICR: u32 = SSI0 + 0x020;
const SSI_CC: u32 = SSI0 + 0xFC8;
/// CR0.DSS: 0x7 = 8-bit, 0xF = 16-bit frames; FRF = 0 (Motorola SPI).
const SSI_CR0_DSS8: u32 = 0x7;
const SSI_CR0_DSS16: u32 = 0xF;
/// CR1: LBM (bit 0) loopback, SSE (bit 1) enable; MS = 0 (master).
const SSI_CR1_LBM: u32 = 1 << 0;
const SSI_CR1_SSE: u32 = 1 << 1;
/// SR.RNE (bit 2): receive FIFO not empty.
const SSI_SR_RNE: u32 = 1 << 2;
/// ICR.RORIC (bit 0): clear the receive-overrun interrupt.
const SSI_ICR_RORIC: u32 = 1 << 0;
/// The SSI FIFOs are eight frames deep (§19.2).
const SSI_FIFO_DEPTH: u32 = 8;

// ── I2C master (SWRU319C §21, base 0x4002_0000) ────────────────────────────
const I2CM: u32 = 0x4002_0000;
const I2CM_SA: u32 = I2CM;
const I2CM_CTRL_STAT: u32 = I2CM + 0x004;
const I2CM_DR: u32 = I2CM + 0x008;
const I2CM_IMR: u32 = I2CM + 0x010;
const I2CM_RIS: u32 = I2CM + 0x014;
const I2CM_ICR: u32 = I2CM + 0x01C;
const I2CM_CR: u32 = I2CM + 0x020;
const I2CM_MFE: u32 = 1 << 4;
const I2CM_RUN: u32 = 1 << 0;
const I2CM_START: u32 = 1 << 1;
const I2CM_STOP: u32 = 1 << 2;
const I2CM_ACK: u32 = 1 << 3;
const I2CM_STAT_ERROR: u32 = 1 << 1;
const I2CM_STAT_ADRACK: u32 = 1 << 2;
const I2CM_STAT_DATACK: u32 = 1 << 3;
const I2CM_STAT_IDLE: u32 = 1 << 5;
const IRQ_I2CM: u32 = 8;

// ── GPTimer (SWRU319C §11; GPTIMER0/1 at 0x4003_0000/0x4003_1000) ──────────
const SYS_CTRL_RCGCGPT: u32 = SYS_CTRL + 0x08;
/// RCGCGPT.GPT0/GPT1 (bits 0/1).
const RCGCGPT_GPT01: u32 = 0x3;
const GPT0: u32 = 0x4003_0000;
const GPT1: u32 = 0x4003_1000;
const GPT_CFG: u32 = 0x000;
const GPT_TAMR: u32 = 0x004;
const GPT_TBMR: u32 = 0x008;
const GPT_CTL: u32 = 0x00C;
const GPT_IMR: u32 = 0x018;
const GPT_RIS: u32 = 0x01C;
const GPT_ICR: u32 = 0x024;
const GPT_TAILR: u32 = 0x028;
const GPT_TBILR: u32 = 0x02C;
const GPT_TBMATCHR: u32 = 0x034;
const GPT_TBPR: u32 = 0x03C;
const GPT_TAR: u32 = 0x048;
/// CFG = 4: two 16-bit timers.
const GPT_CFG_16BIT: u32 = 0x4;
/// TnMR: TnMR = 1 one-shot / 2 periodic, TnCDIR (bit 4) up, TnMIE (bit 5).
const GPT_MR_ONE_SHOT: u32 = 0x1;
const GPT_MR_PERIODIC: u32 = 0x2;
const GPT_MR_UP: u32 = 1 << 4;
const GPT_MR_MIE: u32 = 1 << 5;
/// CTL.TAEN (bit 0), CTL.TBEN (bit 8).
const GPT_CTL_TAEN: u32 = 1 << 0;
const GPT_CTL_TBEN: u32 = 1 << 8;
/// IMR/RIS: TATO (bit 0), TBTO (bit 8), TBM (bit 11).
const GPT_INT_TATO: u32 = 1 << 0;
const GPT_INT_TBTO: u32 = 1 << 8;
const GPT_INT_TBM: u32 = 1 << 11;
/// Table 5-2 regular map: GPTimer0A = IRQ 19, GPTimer1B = IRQ 22.
const IRQ_GPT0A: u32 = 19;
const IRQ_GPT1B: u32 = 22;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;

/// Customer Configuration Area (SWRU319C §8.6, Tables 8-3 to 8-6), placed at
/// 0x0027_FFD4 by memory.x. The ROM boot code reads it at reset: a valid image
/// (word 1 == 0) whose vector table is at 0x0020_0000 (word 2) is entered
/// directly. Word 0's top byte is the backdoor configuration: 0xF3 = reserved
/// ones, backdoor enabled, active low, pad A pin 3 (the SELECT key — the RIOT
/// `cc2538dk` choice). The upper 32 bytes leave every page and debug unlocked.
/// The simulator fast-boots the ELF and does not run the ROM; the CCA is here
/// so the same image would boot on silicon.
#[link_section = ".flashcca"]
#[used]
static CCA: [u32; 11] = [
    0xF3FF_FFFF, // bytes 2004-2007: backdoor config (byte 2007 = 0xF3)
    0x0000_0000, // bytes 2008-2011: image valid
    0x0020_0000, // bytes 2012-2015: vector table address
    0xFFFF_FFFF, // bytes 2016-2047: page lock bits + LOCK_DEBUG_N
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
];

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// GPIO_DATA through the address mask: offset bits [9:2] select the pins a
/// read returns or a write alters (SWRU319C §9.2.2.1.2).
fn data_addr(port: u32, mask: u32) -> u32 {
    port + ((mask & 0xFF) << 2)
}

fn putc(byte: u8) {
    while rd(UART_FR) & UART_FR_TXFF != 0 {}
    wr(UART_DR, u32::from(byte));
}

fn puts(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

fn put_dec32(value: u32) {
    let mut digits = [0u8; 10];
    let mut remaining = value;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    for &digit in &digits[start..] {
        putc(digit);
    }
}

fn put_hex8(value: u32) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    putc(HEX[((value >> 4) & 0xF) as usize]);
    putc(HEX[(value & 0xF) as usize]);
}

/// IOC register read-back (SWRU319C Table 9-7): the pad routing written at
/// start-up, an untouched OVER register at its 0x4 reset, and a masked write
/// to IOC_GPT3OCP2 (all ones keeps INPUT_SEL[4:0] only), restored afterwards.
fn ioc_selftest() {
    wr(IOC_GPT3OCP2, 0xFFFF_FFFF);
    let gpt3ocp2 = rd(IOC_GPT3OCP2);
    wr(IOC_GPT3OCP2, 0);
    puts(b"ioc pa1_sel=");
    put_hex8(rd(IOC_PA1_SEL));
    puts(b" pa1_over=");
    put_hex8(rd(IOC_PA1_OVER));
    puts(b" pa0_over=");
    put_hex8(rd(IOC_PA0_OVER));
    puts(b" rx0=");
    put_hex8(rd(IOC_UARTRXD_UART0));
    puts(b" pd7_over=");
    put_hex8(rd(IOC_PD7_OVER));
    puts(b" gpt3ocp2=");
    put_hex8(gpt3ocp2);
    puts(b" restored=");
    put_hex8(rd(IOC_GPT3OCP2));
    putc(b'\n');
}

// ── CCTEST (Table 23-11, base 0x4401_0000) / ANA_REGS (Table 23-12) ───────
const CCTEST: u32 = 0x4401_0000;
const CCTEST_IO: u32 = CCTEST;
const CCTEST_OBSSEL3: u32 = CCTEST + 0x20;
const CCTEST_TR0: u32 = CCTEST + 0x34;
const CCTEST_USBCTRL: u32 = CCTEST + 0x50;
const ANA_REGS_IVCTRL: u32 = 0x400D_6004;

/// CCTEST and ANA_REGS register read-back: reset values, then all-ones
/// writes that keep only each register's field bits (IO.SC, OBSSEL EN|SEL,
/// TR0[2:0], USBCTRL.USB_STB, IVCTRL[5:0]), restored afterwards.
fn cctest_ana_selftest() {
    let regs = [
        CCTEST_IO,
        CCTEST_OBSSEL3,
        CCTEST_TR0,
        CCTEST_USBCTRL,
        ANA_REGS_IVCTRL,
    ];
    let names: [&[u8]; 5] = [b"io=", b" obssel3=", b" tr0=", b" usbctrl=", b" ivctrl="];
    let mut reset = [0u32; 5];
    let mut ones = [0u32; 5];
    for (i, &reg) in regs.iter().enumerate() {
        reset[i] = rd(reg);
        wr(reg, 0xFFFF_FFFF);
        ones[i] = rd(reg);
        wr(reg, reset[i]);
    }
    puts(b"cctest ");
    for i in 0..5 {
        puts(names[i]);
        put_hex8(reset[i]);
        putc(b'/');
        put_hex8(ones[i]);
    }
    puts(b" restored=");
    put_hex8(rd(ANA_REGS_IVCTRL));
    putc(b'\n');
}

fn ssi_drain() {
    while rd(SSI_SR) & SSI_SR_RNE != 0 {
        rd(SSI_DR);
    }
}

/// SSI0 in internal loopback (CR1.LBM, §19.4): frames written while the port
/// is disabled wait in the TX FIFO, leave when SSE is set and come back in the
/// RX FIFO; a 16-bit frame keeps all 16 bits; a ninth frame overruns the
/// eight-deep RX FIFO (RIS.RORRIS) until ICR clears it.
fn ssi0_selftest() {
    wr(SYS_CTRL_RCGCSSI, rd(SYS_CTRL_RCGCSSI) | RCGCSSI_SSI0);
    wr(SSI_CR1, 0);
    wr(SSI_CC, 0);
    wr(SSI_CPSR, 2);
    wr(SSI_CR0, SSI_CR0_DSS8);
    wr(SSI_DR, 0xA5);
    wr(SSI_DR, 0x3C);
    puts(b"ssi0 queued sr=");
    put_hex8(rd(SSI_SR));
    wr(SSI_CR1, SSI_CR1_LBM | SSI_CR1_SSE);
    puts(b" enabled sr=");
    put_hex8(rd(SSI_SR));
    puts(b" ris=");
    put_hex8(rd(SSI_RIS));
    puts(b" rx=");
    put_hex8(rd(SSI_DR));
    putc(b' ');
    put_hex8(rd(SSI_DR));
    putc(b'\n');

    wr(SSI_CR1, 0);
    wr(SSI_CR0, SSI_CR0_DSS16);
    wr(SSI_CR1, SSI_CR1_LBM | SSI_CR1_SSE);
    wr(SSI_DR, 0xBEEF);
    let w16 = rd(SSI_DR);
    puts(b"ssi0 w16=");
    put_hex8(w16 >> 8);
    put_hex8(w16);
    for frame in 0..=SSI_FIFO_DEPTH {
        wr(SSI_DR, frame);
    }
    puts(b" overrun sr=");
    put_hex8(rd(SSI_SR));
    puts(b" ris=");
    put_hex8(rd(SSI_RIS));
    wr(SSI_ICR, SSI_ICR_RORIC);
    puts(b" cleared ris=");
    put_hex8(rd(SSI_RIS));
    ssi_drain();
    puts(b" drained ris=");
    put_hex8(rd(SSI_RIS));
    putc(b'\n');
    wr(SSI_CR1, 0);
}

/// Read the virtual TMP102's 16-bit temperature register using a register
/// pointer write followed by a repeated-start read. The onboarding system
/// attaches the TMP102 as an explicit test fixture; other CC2538 systems may
/// leave the bus empty and report the expected address NACK.
fn i2cm_selftest() {
    wr(SYS_CTRL_RCGCI2C, rd(SYS_CTRL_RCGCI2C) | 1);
    wr(NVIC_ICPR0, 1 << IRQ_I2CM);
    wr(I2CM_CR, I2CM_MFE);
    wr(I2CM_IMR, 1);
    wr(I2CM_SA, 0x48 << 1);
    wr(I2CM_DR, 0x00); // TMP102 temperature-register pointer.
    wr(I2CM_CTRL_STAT, I2CM_START | I2CM_RUN);

    wr(I2CM_SA, (0x48 << 1) | 1);
    wr(I2CM_CTRL_STAT, I2CM_START | I2CM_RUN | I2CM_ACK);
    let msb = rd(I2CM_DR) & 0xFF;
    wr(I2CM_CTRL_STAT, I2CM_RUN | I2CM_STOP);
    let lsb = rd(I2CM_DR) & 0xFF;
    let status = rd(I2CM_CTRL_STAT);
    let ris = rd(I2CM_RIS);
    let irq8 = (rd(NVIC_ISPR0) >> IRQ_I2CM) & 1;
    wr(I2CM_ICR, 1);
    wr(NVIC_ICPR0, 1 << IRQ_I2CM);

    if status & (I2CM_STAT_ERROR | I2CM_STAT_ADRACK | I2CM_STAT_DATACK) == 0
        && status & I2CM_STAT_IDLE != 0
    {
        puts(b"i2cm read=");
        put_hex8(msb);
        put_hex8(lsb);
        puts(b" stat=");
        put_hex8(status);
        puts(b" irq8=");
        put_dec32(irq8);
        puts(b" ris=");
        put_dec32(ris);
        puts(b" cleared=");
        put_dec32(rd(I2CM_RIS));
        putc(b'\n');
    } else {
        puts(b"i2cm nack stat=");
        put_hex8(status);
        putc(b'\n');
    }
}

// ── SOC_ADC (SWRU319C Tables 15-1, 16-1, 17-1, base 0x400D_7000) ──────────
const SOC_ADC: u32 = 0x400D_7000;
const ADC_ADCCON1: u32 = SOC_ADC;
const ADC_ADCCON3: u32 = SOC_ADC + 0x08;
const ADC_ADCL: u32 = SOC_ADC + 0x0C;
const ADC_ADCH: u32 = SOC_ADC + 0x10;
const ADC_RNDL: u32 = SOC_ADC + 0x14;
const ADC_RNDH: u32 = SOC_ADC + 0x18;
const ADC_CMPCTL: u32 = SOC_ADC + 0x24;
const ADCCON1_EOC: u32 = 1 << 7;
/// ADCCON1: STSEL = 11 (reset), RCTRL = 01 (clock the LFSR once).
const ADCCON1_RNG_CLOCK: u32 = 0x30 | 0x04;
/// ADCCONx: internal reference, decimation 512 (12 bits).
const ADC_REF_INT_DIV_512: u32 = 0x30;
const ADC_CH_GND: u32 = 0xC;
const ADC_CH_TEMP: u32 = 0xE;
const ADC_CH_VDD_3: u32 = 0xF;
const CMPCTL_EN: u32 = 1 << 1;
/// Table 5-2: ADC interrupt.
const IRQ_ADC: u32 = 14;

/// Contiki-NG `adc_get`: write ADCCON3, poll EOC, read ADCL then ADCH (which
/// clears EOC). Returns the MSB-aligned 16-bit result.
fn adc_single(control: u32) -> u32 {
    wr(ADC_ADCCON3, control);
    while rd(ADC_ADCCON1) & ADCCON1_EOC == 0 {}
    let lo = rd(ADC_ADCL) & 0xFC;
    (rd(ADC_ADCH) << 8) | lo
}

fn rng_value() -> u32 {
    (rd(ADC_RNDH) << 8) | rd(ADC_RNDL)
}

/// SOC_ADC: single conversions of the internal channels (VDD/3, temperature
/// sensor, GND) against the 1.19 V internal reference, EOC cleared by the ADCH
/// read, the IRQ 14 pend; the RNG LFSR seeded 0x1234 and clocked once, a
/// CRC16 of "123456789" through RNDH; the comparator reset/enabled outputs.
fn soc_adc_selftest() {
    let con1 = rd(ADC_ADCCON1);
    wr(NVIC_ICPR0, 1 << IRQ_ADC);
    let vdd3 = adc_single(ADC_REF_INT_DIV_512 | ADC_CH_VDD_3);
    let irq14 = (rd(NVIC_ISPR0) >> IRQ_ADC) & 1;
    wr(NVIC_ICPR0, 1 << IRQ_ADC);
    let eoc = (rd(ADC_ADCCON1) & ADCCON1_EOC) >> 7;
    let temp = adc_single(ADC_REF_INT_DIV_512 | ADC_CH_TEMP);
    let gnd = adc_single(ADC_REF_INT_DIV_512 | ADC_CH_GND);
    wr(NVIC_ICPR0, 1 << IRQ_ADC);
    puts(b"adc con1=");
    put_hex8(con1);
    puts(b" vdd3=");
    put_hex16(vdd3);
    puts(b" temp=");
    put_hex16(temp);
    puts(b" gnd=");
    put_hex16(gnd);
    puts(b" eoc=");
    put_dec32(eoc);
    puts(b" irq14=");
    put_dec32(irq14);
    putc(b'\n');

    let reset = rng_value();
    wr(ADC_RNDL, 0x12);
    wr(ADC_RNDL, 0x34);
    let seeded = rng_value();
    wr(ADC_ADCCON1, ADCCON1_RNG_CLOCK);
    let clocked = rng_value();
    let rctrl = (rd(ADC_ADCCON1) >> 2) & 0x3;
    wr(ADC_RNDL, 0);
    wr(ADC_RNDL, 0);
    for byte in b"123456789" {
        wr(ADC_RNDH, *byte as u32);
    }
    puts(b"rng reset=");
    put_hex16(reset);
    puts(b" seed=");
    put_hex16(seeded);
    puts(b" clock=");
    put_hex16(clocked);
    puts(b" rctrl=");
    put_dec32(rctrl);
    puts(b" crc=");
    put_hex16(rng_value());
    putc(b'\n');

    let cmp_off = rd(ADC_CMPCTL);
    wr(ADC_CMPCTL, CMPCTL_EN);
    let cmp_on = rd(ADC_CMPCTL);
    wr(ADC_CMPCTL, 0);
    puts(b"cmp off=");
    put_hex8(cmp_off);
    puts(b" on=");
    put_hex8(cmp_on);
    putc(b'\n');
}

// ── PKA SRAM (SWRU319C Table 4-1: 0x4400_6000-0x4400_67FF) ────────────────
const PKA_SRAM: u32 = 0x4400_6000;
const PKA_SRAM_WORDS: u32 = 2048 / 4;
/// RCGCSEC (Contiki-NG arch/cpu/cc2538/dev/sys-ctrl.h, 0x400D_2048); bit 0
/// clocks the PKA in active mode.
const SYS_CTRL_RCGCSEC: u32 = SYS_CTRL + 0x48;
const RCGCSEC_PKA: u32 = 0x1;

/// Word pattern for PKA SRAM slot `i`: distinct per word and never zero, so a
/// zero-answering window cannot pass.
fn pka_pattern(i: u32) -> u32 {
    (i.wrapping_mul(0x9E37_79B9)) ^ 0xA5A5_0000 ^ i
}

/// Clock the PKA, fill all 512 PKA SRAM words, then read every one back.
fn pka_sram_selftest() {
    wr(SYS_CTRL_RCGCSEC, rd(SYS_CTRL_RCGCSEC) | RCGCSEC_PKA);
    for i in 0..PKA_SRAM_WORDS {
        wr(PKA_SRAM + i * 4, pka_pattern(i));
    }
    let mut bad = 0;
    for i in 0..PKA_SRAM_WORDS {
        if rd(PKA_SRAM + i * 4) != pka_pattern(i) {
            bad += 1;
        }
    }
    puts(b"pka_sram words=");
    put_dec32(PKA_SRAM_WORDS);
    puts(b" bad=");
    put_dec32(bad);
    puts(b" last=");
    put_hex16(rd(PKA_SRAM + (PKA_SRAM_WORDS - 1) * 4) >> 16);
    put_hex16(rd(PKA_SRAM + (PKA_SRAM_WORDS - 1) * 4));
    putc(b'\n');
}

fn put_hex16(value: u32) {
    put_hex8(value >> 8);
    put_hex8(value);
}

/// Poll `addr` until a bit of `mask` reads 1; false after a bounded wait.
fn wait_set(addr: u32, mask: u32) -> bool {
    for _ in 0..100_000 {
        if rd(addr) & mask != 0 {
            return true;
        }
    }
    false
}

/// GPTimer self-test (§11), polled with the NVIC lines left disabled so the
/// pends show in ISPR without a handler.
/// 1. GPT0 as one 32-bit periodic down-counter, TAILR = 1999: three
///    time-outs, each cleared through ICR, pend GPTimer0A; disabling holds
///    the count.
/// 2. GPT1 split, timer B a one-shot up-counter ÷16 with TBILR = 1000 and
///    TBMATCHR = 500: match then time-out (RIS = 0x0900), TBEN clears, the
///    match pends GPTimer1B, and GPT0 (re-armed as a free 32-bit up-counter)
///    measures the 16 000-cycle span.
fn gpt_selftest() {
    wr(SYS_CTRL_RCGCGPT, rd(SYS_CTRL_RCGCGPT) | RCGCGPT_GPT01);
    wr(NVIC_ICPR0, (1 << IRQ_GPT0A) | (1 << IRQ_GPT1B));

    wr(GPT0 + GPT_CTL, 0);
    wr(GPT0 + GPT_CFG, 0);
    wr(GPT0 + GPT_TAMR, GPT_MR_PERIODIC);
    wr(GPT0 + GPT_TAILR, 1999);
    wr(GPT0 + GPT_IMR, GPT_INT_TATO);
    wr(GPT0 + GPT_CTL, GPT_CTL_TAEN);
    let mut timeouts = 0;
    for _ in 0..3 {
        if !wait_set(GPT0 + GPT_RIS, GPT_INT_TATO) {
            break;
        }
        wr(GPT0 + GPT_ICR, GPT_INT_TATO);
        timeouts += 1;
    }
    let pend0a = (rd(NVIC_ISPR0) >> IRQ_GPT0A) & 1;
    wr(GPT0 + GPT_CTL, 0);
    let held = rd(GPT0 + GPT_TAR);
    spin(100);
    puts(b"gpt0 to=");
    put_dec32(timeouts);
    puts(b" nvic19=");
    put_dec32(pend0a);
    puts(b" held=");
    put_dec32(u32::from(rd(GPT0 + GPT_TAR) == held && held <= 1999));
    putc(b'\n');

    // GPT0 as a free-running 32-bit up-counter: the time base for the span.
    wr(GPT0 + GPT_IMR, 0);
    wr(GPT0 + GPT_ICR, 0xFFFF);
    wr(GPT0 + GPT_TAMR, GPT_MR_PERIODIC | GPT_MR_UP);
    wr(GPT0 + GPT_TAILR, 0xFFFF_FFFF);
    wr(GPT0 + GPT_CTL, GPT_CTL_TAEN);

    wr(GPT1 + GPT_CTL, 0);
    wr(GPT1 + GPT_CFG, GPT_CFG_16BIT);
    wr(GPT1 + GPT_TBMR, GPT_MR_ONE_SHOT | GPT_MR_UP | GPT_MR_MIE);
    wr(GPT1 + GPT_TBPR, 15);
    wr(GPT1 + GPT_TBILR, 1000);
    wr(GPT1 + GPT_TBMATCHR, 500);
    wr(GPT1 + GPT_IMR, GPT_INT_TBM);
    let t0 = rd(GPT0 + GPT_TAR);
    wr(GPT1 + GPT_CTL, GPT_CTL_TBEN);
    let done = wait_set(GPT1 + GPT_RIS, GPT_INT_TBTO);
    let span = rd(GPT0 + GPT_TAR).wrapping_sub(t0);
    puts(b"gpt1b ris=");
    put_hex16(rd(GPT1 + GPT_RIS));
    puts(b" ctl=");
    put_hex16(rd(GPT1 + GPT_CTL));
    puts(b" nvic22=");
    put_dec32((rd(NVIC_ISPR0) >> IRQ_GPT1B) & 1);
    // 1000 counts x 16 cycles, plus the poll loop and prescaler phase.
    puts(if done && (15_900..16_500).contains(&span) {
        b" span=ok "
    } else {
        b" span=bad "
    });
    put_dec32(span);
    putc(b'\n');

    wr(GPT0 + GPT_CTL, 0);
    wr(GPT1 + GPT_CTL, 0);
    wr(GPT1 + GPT_ICR, 0xFFFF);
    wr(NVIC_ICPR0, (1 << IRQ_GPT0A) | (1 << IRQ_GPT1B));
}

/// System clock from CLOCK_STA: 32 MHz >> SYS_DIV.
fn system_clock_hz(clock_sta: u32) -> u32 {
    32_000_000 >> (clock_sta & CLOCK_STA_SYS_DIV)
}

/// One pin's level, read through its own DATA mask.
fn pin(port: u32, n: u32) -> u32 {
    (rd(data_addr(port, 1 << n)) >> n) & 1
}

fn led_set(n: u32, on: bool) {
    // A single masked store: only pin n changes (no read-modify-write).
    wr(data_addr(GPIO_C, 1 << n), if on { 0xFF } else { 0x00 });
}

fn report(name: &[u8], n: u32, on: bool) {
    puts(b"led=");
    puts(name);
    puts(if on { b" on pin=" } else { b" off pin=" });
    put_dec32(pin(GPIO_C, n));
    // The whole LED nibble through one masked read: proves the store above
    // touched only its own pin.
    puts(b" leds=");
    put_hex8(rd(data_addr(GPIO_C, LED_MASK)));
    putc(b'\n');
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

#[entry]
fn main() -> ! {
    // 1. Clock gate for UART0 (SWRU319C §18.5 step 1). GPIO has no gate.
    wr(SYS_CTRL_RCGCUART, rd(SYS_CTRL_RCGCUART) | RCGCUART_UART0);

    // 2-4. IOC: PA1 driven by UART0 TXD with the pad output enabled; PA0 an
    //      input routed to UART0 RX (§18.5 steps 2-4, §9.1.1).
    wr(IOC_PA1_SEL, IOC_SEL_UART0_TXD);
    wr(IOC_PA1_OVER, IOC_OVER_OE);
    wr(IOC_PA0_OVER, IOC_OVER_DIS);
    wr(IOC_UARTRXD_UART0, IOC_INPUT_PA0);
    // PA0/PA1 under peripheral (hardware) control (§9.2.2.3).
    wr(GPIO_A + GPIO_AFSEL, rd(GPIO_A + GPIO_AFSEL) | 0x03);

    // The baud divisor follows the clock CLOCK_STA reports, not an assumption.
    let clock_sta = rd(SYS_CTRL_CLOCK_STA);
    let sysclk = system_clock_hz(clock_sta);
    let brd64 = (sysclk * 8 / BAUD + 1) / 2; // round(64 × sysclk / (16 × baud))
    let ibrd = brd64 / 64;
    let fbrd = brd64 % 64;

    // UART0: disable, divisors, LCRH (latches the divisors), enable (§18.5).
    wr(UART_CTL, 0);
    wr(UART_CC, UART_CC_SYSDIV);
    wr(UART_IBRD, ibrd);
    wr(UART_FBRD, fbrd);
    wr(UART_LCRH, UART_LCRH_8N1);
    wr(UART_CTL, UART_CTL_EN);

    // LEDs PC1..PC3 as software-controlled outputs, off. PC0 is untouched.
    wr(GPIO_C + GPIO_AFSEL, rd(GPIO_C + GPIO_AFSEL) & !LED_MASK);
    wr(data_addr(GPIO_C, LED_MASK), 0);
    wr(GPIO_C + GPIO_DIR, rd(GPIO_C + GPIO_DIR) | LED_MASK);

    puts(b"CC2538DK SMOKE\n");
    puts(b"cpu=cortex-m3 uart=uart0\n");
    puts(b"clk osc=");
    puts(if clock_sta & CLOCK_STA_OSC != 0 {
        b"hfrc16"
    } else {
        b"xosc32"
    });
    puts(b" sys_div=");
    put_dec32(clock_sta & CLOCK_STA_SYS_DIV);
    puts(b" io_div=");
    put_dec32((clock_sta >> CLOCK_STA_IO_DIV_SHIFT) & 0x7);
    puts(b" sysclk=");
    put_dec32(sysclk);
    putc(b'\n');
    puts(b"uart0 rcgc=");
    put_dec32(rd(SYS_CTRL_RCGCUART) & RCGCUART_UART0);
    puts(b" ibrd=");
    put_dec32(rd(UART_IBRD));
    puts(b" fbrd=");
    put_dec32(rd(UART_FBRD));
    putc(b'\n');

    for (name, n) in [
        (&b"yellow"[..], LED_YELLOW),
        (&b"green"[..], LED_GREEN),
        (&b"orange"[..], LED_ORANGE),
    ] {
        led_set(n, true);
        report(name, n, true);
        led_set(n, false);
        report(name, n, false);
    }

    // Keys are active low with the reset IOC pull-up: released reads 1.
    puts(b"keys select=");
    put_dec32(pin(GPIO_A, KEY_SELECT));
    puts(b" left=");
    put_dec32(pin(GPIO_C, KEY_LEFT));
    puts(b" right=");
    put_dec32(pin(GPIO_C, KEY_RIGHT));
    puts(b" up=");
    put_dec32(pin(GPIO_C, KEY_UP));
    puts(b" down=");
    put_dec32(pin(GPIO_C, KEY_DOWN));
    putc(b'\n');

    ioc_selftest();
    cctest_ana_selftest();
    soc_adc_selftest();
    ssi0_selftest();
    i2cm_selftest();
    pka_sram_selftest();
    gpt_selftest();

    // Leave yellow lit; the heartbeat reports the three driven pad levels.
    led_set(LED_YELLOW, true);

    let mut heartbeat: u32 = 0;
    loop {
        puts(b"hb=");
        put_dec32(heartbeat);
        puts(b" yellow=");
        put_dec32(pin(GPIO_C, LED_YELLOW));
        puts(b" green=");
        put_dec32(pin(GPIO_C, LED_GREEN));
        puts(b" orange=");
        put_dec32(pin(GPIO_C, LED_ORANGE));
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
