// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Smoke for the Renode `sam_e70` row (ATSAME70Q21). The run spans three
// boots, told apart by RSTC_SR.RSTTYP:
//
// 1. General reset (0): CHIPID, EEFC FMR wait states, the GETD descriptor
//    (2 MiB, 128 lock regions of 16 KiB), multi-word GLB, the nine GPNVM
//    bits, a bad-key FCMDE; RSTC_SR and the armed-at-reset WDT_MR
//    (0x3FFF_2FFF). Then RSTC_CR PROCRST with the key.
// 2. Software reset (3): WDT_MR = WDRSTEN with WDV = 2, then wait for the
//    watchdog to reboot the core.
// 3. Watchdog reset (2): WDT_MR is back at its reset value; a restart
//    outside the WDD window raises WDERR and pends NVIC line 4, the SR read
//    releases it, a second WDT_MR write is ignored. Then PMC clocks on both
//    banks, the PIO latch on PIOA-E, the PIOA input change interrupt (pull-up
//    on a floating PA11 raises the pad: NVIC line 10), and a line out of
//    every other UART/USART.
//
// Check bits persist across the reboots in a word below the summary (not in
// .bss, which cortex-m-rt zeroes on every boot). Summary = 0xE700_0000 |
// bits; 0xE700_7FFF means all fifteen passed.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// 4 KiB below the top of SRAM, clear of the stack.
const SUMMARY: u32 = 0x2045_F000;
const STATE: u32 = SUMMARY - 0x100;
const ALL: u32 = 0x7FFF;

const PMC_PCER0: u32 = 0x400E_0610;
const PMC_PCSR0: u32 = 0x400E_0618;
const PMC_PCER1: u32 = 0x400E_0700;
const PMC_PCSR1: u32 = 0x400E_0708;
const PMC_SR: u32 = 0x400E_0668;

const CHIPID_CIDR: u32 = 0x400E_0940;
const CHIPID_EXID: u32 = 0x400E_0944;

const EEFC_FMR: u32 = 0x400E_0C00;
const EEFC_FCR: u32 = 0x400E_0C04;
const EEFC_FSR: u32 = 0x400E_0C08;
const EEFC_FRR: u32 = 0x400E_0C0C;
const FKEY: u32 = 0x5A << 24;

const RSTC_CR: u32 = 0x400E_1800;
const RSTC_SR: u32 = 0x400E_1804;

const WDT_CR: u32 = 0x400E_1850;
const WDT_MR: u32 = 0x400E_1854;
const WDT_SR: u32 = 0x400E_1858;
const WDT_MR_RESET: u32 = 0x3FFF_2FFF;
const WDFIEN: u32 = 1 << 12;
const WDRSTEN: u32 = 1 << 13;
const WDDIS: u32 = 1 << 15;
const WDT_RESTART: u32 = 0xA500_0001;

const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;
const IRQ_WDT: u32 = 4;
const IRQ_PIOA: u32 = 10;
const IRQ_USART0: u32 = 13;

const ID_UART0: u32 = 7;
const ID_UART1: u32 = 8;
const ID_PIOA: u32 = 10;
const ID_PIOB: u32 = 11;
const ID_PIOC: u32 = 12;
const ID_USART0: u32 = 13;
const ID_USART1: u32 = 14;
const ID_USART2: u32 = 15;
const ID_PIOD: u32 = 16;
const ID_PIOE: u32 = 17;
// UART2-4 are peripheral IDs 44-46: PMC bank 1, bits 12-14.
const BANK1_UART2_4: u32 = 0b111 << 12;

const PIO_BASES: [u32; 5] = [
    0x400E_0E00,
    0x400E_1000,
    0x400E_1200,
    0x400E_1400,
    0x400E_1600,
];
const PIO_PER: u32 = 0x00;
const PIO_OER: u32 = 0x10;
const PIO_ODR: u32 = 0x14;
const PIO_SODR: u32 = 0x30;
const PIO_CODR: u32 = 0x34;
const PIO_ODSR: u32 = 0x38;
const PIO_PDSR: u32 = 0x3C;
const PIO_IER: u32 = 0x40;
const PIO_IDR: u32 = 0x44;
const PIO_ISR: u32 = 0x4C;
const PIO_PUDR: u32 = 0x60;
const PIO_PUER: u32 = 0x64;

const USART1: u32 = 0x4002_8000;
// Every other serial block, with the name it prints.
const USART0: u32 = 0x4002_4000;
const OTHER_UARTS: [(u32, &str); 7] = [
    (USART0, "usart0"),
    (0x4002_C000, "usart2"),
    (0x400E_0800, "uart0"),
    (0x400E_0A00, "uart1"),
    (0x400E_1A00, "uart2"),
    (0x400E_1C00, "uart3"),
    (0x400E_1E00, "uart4"),
];
const US_CR: u32 = 0x00;
const US_MR: u32 = 0x04;
const US_CSR: u32 = 0x14;
const US_IER: u32 = 0x08;
const US_IDR: u32 = 0x0C;
const US_IMR: u32 = 0x10;
const US_THR: u32 = 0x1C;
const US_BRGR: u32 = 0x20;
const US_CR_RSTRX: u32 = 1 << 2;
const US_CR_RSTTX: u32 = 1 << 3;
const US_CR_RXEN: u32 = 1 << 4;
const US_CR_TXEN: u32 = 1 << 6;
const US_CSR_TXRDY: u32 = 1 << 1;
const US_MR_8N1: u32 = 0x8C0;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn uart_init(base: u32) {
    wr(base + US_CR, US_CR_RSTRX | US_CR_RSTTX);
    wr(base + US_MR, US_MR_8N1);
    wr(base + US_BRGR, 6);
    wr(base + US_CR, US_CR_RXEN | US_CR_TXEN);
}

fn uart_putc(base: u32, byte: u8) {
    while rd(base + US_CSR) & US_CSR_TXRDY == 0 {}
    wr(base + US_THR, byte as u32);
}

fn uart_puts(base: u32, text: &str) {
    text.bytes().for_each(|b| uart_putc(base, b));
}

fn puts(text: &str) {
    uart_puts(USART1, text);
}

fn putc(byte: u8) {
    uart_putc(USART1, byte);
}

fn put_hex(value: u32, digits: u32) {
    for shift in (0..digits).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        });
    }
}

fn finish(label: &str, ok: bool, bit: u32) {
    puts(label);
    puts(if ok { " ok\n" } else { " FAIL\n" });
    if ok {
        wr(STATE, rd(STATE) | 1 << bit);
    }
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

fn eefc(cmd: u32, arg: u32) {
    wr(EEFC_FCR, FKEY | (arg << 8) | cmd);
    while rd(EEFC_FSR) & 1 == 0 {}
}

fn boot_general() {
    wr(STATE, 0);

    let (cidr, exid) = (rd(CHIPID_CIDR), rd(CHIPID_EXID));
    puts("chipid cidr=");
    put_hex(cidr, 8);
    puts(" exid=");
    put_hex(exid, 8);
    finish("", cidr == 0xA102_0E00 && exid == 2, 0);

    wr(EEFC_FMR, 6 << 8);
    let fmr = rd(EEFC_FMR);
    puts("eefc fmr=");
    put_hex(fmr, 8);
    finish("", fmr == 6 << 8, 1);

    eefc(0x00, 0); // GETD
    let mut d = [0u32; 8];
    for word in d.iter_mut() {
        *word = rd(EEFC_FRR);
    }
    puts("eefc getd size=");
    put_hex(d[1], 8);
    puts(" page=");
    put_hex(d[2], 4);
    puts(" locks=");
    put_hex(d[5], 4);
    puts(" region=");
    put_hex(d[6], 8);
    let getd_ok = d[1] == 0x20_0000
        && d[2] == 512
        && d[3] == 1
        && d[4] == 0x20_0000
        && d[5] == 128
        && d[6] == 16384
        && d[7] == 16384;
    finish("", getd_ok, 2);

    // SLB on page 3200 (1600 KiB): lock region 100 = GLB word 3, bit 4.
    eefc(0x08, 3200);
    eefc(0x0A, 0); // GLB
    let mut glb = [0u32; 4];
    for word in glb.iter_mut() {
        *word = rd(EEFC_FRR);
    }
    eefc(0x09, 3200); // CLB
    eefc(0x0A, 0);
    let cleared = (0..4).fold(0, |acc, _| acc | rd(EEFC_FRR));
    eefc(0x0D, 0); // GGPB
    let boot = rd(EEFC_FRR);
    eefc(0x0B, 8); // SGPB 8 (TCM configuration)
    eefc(0x0D, 0);
    let set = rd(EEFC_FRR);
    eefc(0x0C, 8);
    puts("eefc glb3=");
    put_hex(glb[3], 2);
    puts(" clb=");
    put_hex(cleared, 1);
    puts(" gpnvm=");
    put_hex(boot, 3);
    puts(" set8=");
    put_hex(set, 3);
    finish(
        "",
        glb == [0, 0, 0, 1 << 4] && cleared == 0 && boot == 0b10 && set == 0x102,
        3,
    );

    wr(EEFC_FCR, FKEY | (9 << 8) | 0x0B); // SGPB 9: no such bit
    let bad_bit = rd(EEFC_FSR);
    wr(EEFC_FCR, 0x00); // no FKEY
    let bad_key = rd(EEFC_FSR);
    let after = rd(EEFC_FSR);
    puts("eefc fsr gpnvm9=");
    put_hex(bad_bit, 1);
    puts(" badkey=");
    put_hex(bad_key, 1);
    puts(" after=");
    put_hex(after, 1);
    finish("", bad_bit == 0b11 && bad_key == 0b11 && after == 0b01, 4);

    let sr = rd(RSTC_SR);
    let mr = rd(WDT_MR);
    puts("rstc sr=");
    put_hex(sr, 8);
    puts(" wdt mr=");
    put_hex(mr, 8);
    finish(
        "",
        (sr >> 8) & 7 == 0 && sr & (1 << 16) != 0 && mr == WDT_MR_RESET,
        5,
    );

    puts("rstc software reset\n");
    wr(RSTC_CR, 0xA500_0001); // PROCRST (the E70 has no PERRST bit)
    spin(10_000);
    puts("rstc reset did not happen\n");
}

fn boot_software() {
    finish("rstc rsttyp=3", true, 6);
    puts("wdt arm wdv=2\n");
    wr(WDT_MR, WDRSTEN | (0xFFF << 16) | 2);
    // Two counts at SLCK/128 from the 12 MHz reset clock: ~94k cycles.
    spin(400_000);
    puts("wdt reset did not happen\n");
}

fn boot_watchdog() {
    let mr = rd(WDT_MR);
    puts("wdt rsttyp=2 mr=");
    put_hex(mr, 8);
    finish("", mr == WDT_MR_RESET, 7);

    // WDV 0x100, WDD 0x80, WDFIEN without WDRSTEN: restarting at once (counter
    // 0x100 > WDD) is a window error.
    wr(WDT_MR, (0x80 << 16) | WDFIEN | 0x100);
    wr(WDT_CR, WDT_RESTART);
    let pended = rd(NVIC_ISPR0) & (1 << IRQ_WDT) != 0;
    let sr = rd(WDT_SR);
    wr(NVIC_ICPR0, 1 << IRQ_WDT);
    let released = rd(NVIC_ISPR0) & (1 << IRQ_WDT) == 0 && rd(WDT_SR) == 0;
    puts("wdt window sr=");
    put_hex(sr, 1);
    puts(if pended { " pended" } else { " idle" });
    puts(if released { " released" } else { " stuck" });
    finish("", sr == 2 && pended && released, 8);

    wr(WDT_MR, WDDIS);
    let locked = rd(WDT_MR);
    puts("wdt mr once=");
    put_hex(locked, 8);
    finish("", locked == (0x80 << 16) | WDFIEN | 0x100, 9);
}

fn peripherals() {
    let bank0 = (1 << ID_UART0)
        | (1 << ID_UART1)
        | (1 << ID_PIOA)
        | (1 << ID_PIOB)
        | (1 << ID_PIOC)
        | (1 << ID_USART0)
        | (1 << ID_USART1)
        | (1 << ID_USART2)
        | (1 << ID_PIOD)
        | (1 << ID_PIOE);
    wr(PMC_PCER0, bank0);
    wr(PMC_PCER1, BANK1_UART2_4);
    let (pcsr0, pcsr1) = (rd(PMC_PCSR0), rd(PMC_PCSR1));
    puts("pmc pcsr0=");
    put_hex(pcsr0, 8);
    puts(" pcsr1=");
    put_hex(pcsr1, 8);
    finish(
        "",
        pcsr0 & bank0 == bank0 && pcsr1 & BANK1_UART2_4 == BANK1_UART2_4,
        10,
    );

    let mut pio_ok = true;
    for &base in PIO_BASES.iter() {
        let pin = 1 << 3;
        wr(base + PIO_PER, pin);
        wr(base + PIO_OER, pin);
        wr(base + PIO_SODR, pin);
        let high = rd(base + PIO_ODSR) & pin != 0;
        wr(base + PIO_CODR, pin);
        let low = rd(base + PIO_ODSR) & pin == 0;
        pio_ok &= high && low;
    }
    puts("pio ports=5");
    finish("", pio_ok, 11);

    // PA11 as a floating input with its change interrupt enabled: enabling
    // the pull-up raises the pad (one change), disabling it drops it again.
    let pioa = PIO_BASES[0];
    let pa11 = 1 << 11;
    wr(pioa + PIO_PUDR, pa11);
    wr(pioa + PIO_PER, pa11);
    wr(pioa + PIO_ODR, pa11);
    let _ = rd(pioa + PIO_ISR);
    wr(NVIC_ICPR0, 1 << IRQ_PIOA);
    wr(pioa + PIO_IER, pa11);
    wr(pioa + PIO_PUER, pa11);
    let level_up = rd(pioa + PIO_PDSR) & pa11 != 0;
    let rise_pend = rd(NVIC_ISPR0) & (1 << IRQ_PIOA) != 0;
    let isr_up = rd(pioa + PIO_ISR) & pa11;
    wr(NVIC_ICPR0, 1 << IRQ_PIOA);
    let released = rd(NVIC_ISPR0) & (1 << IRQ_PIOA) == 0;
    wr(pioa + PIO_PUDR, pa11);
    let isr_down = rd(pioa + PIO_ISR) & pa11;
    wr(pioa + PIO_IDR, pa11);
    wr(NVIC_ICPR0, 1 << IRQ_PIOA);
    puts("pioa pa11 change up=");
    put_hex(level_up as u32, 1);
    puts(" isr=");
    put_hex(isr_up >> 11, 1);
    put_hex(isr_down >> 11, 1);
    puts(" pend=");
    put_hex(rise_pend as u32, 1);
    puts(" released=");
    put_hex(released as u32, 1);
    finish(
        "",
        level_up && rise_pend && isr_up == pa11 && released && isr_down == pa11,
        12,
    );

    // USART0 TXRDY is a level source: IER sets its mask, CSR.TXRDY asserts
    // IRQ 13, and IDR must clear both IMR and the pending NVIC level.
    uart_init(USART0);
    wr(NVIC_ICPR0, 1 << IRQ_USART0);
    wr(USART0 + US_IER, US_CSR_TXRDY);
    spin(8);
    let imr = rd(USART0 + US_IMR);
    let pended = rd(NVIC_ISPR0) & (1 << IRQ_USART0) != 0;
    wr(USART0 + US_IDR, US_CSR_TXRDY);
    let released = rd(USART0 + US_IMR) == 0 && rd(NVIC_ISPR0) & (1 << IRQ_USART0) == 0;
    puts("usart0 txrdy imr=");
    put_hex(imr, 1);
    puts(" pend=");
    put_hex(pended as u32, 1);
    puts(" released=");
    put_hex(released as u32, 1);
    finish("", imr == US_CSR_TXRDY && pended && released, 14);

    for &(base, name) in OTHER_UARTS.iter() {
        uart_init(base);
        uart_puts(base, name);
        uart_puts(base, " tx\n");
    }
    puts("uarts=7");
    finish("", true, 13);
}

#[entry]
fn main() -> ! {
    // The interrupt-pend checks read NVIC_ISPR0 and never take the vector.
    wr(NVIC_ICER0, 0xFFFF_FFFF);
    wr(PMC_PCER0, 1 << ID_USART1);
    while rd(PMC_SR) & (1 << 3) == 0 {}
    uart_init(USART1);
    puts("SAM E70 RENODE SMOKE\n");

    let rsttyp = (rd(RSTC_SR) >> 8) & 7;
    match rsttyp {
        0 => boot_general(),
        3 => boot_software(),
        2 => {
            boot_watchdog();
            peripherals();
        }
        other => {
            puts("unexpected rsttyp=");
            put_hex(other, 1);
            puts("\n");
        }
    }

    let bits = rd(STATE);
    wr(SUMMARY, 0xE700_0000 | bits);
    puts(if bits == ALL {
        "SAM E70 PASS\n"
    } else {
        "SAM E70 FAIL\n"
    });
    loop {
        spin(20_000);
    }
}
