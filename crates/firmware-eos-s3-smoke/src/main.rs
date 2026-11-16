#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// Renode platforms/cpus/eos-s3.repl: PL011 at 0x4001_0000.
const UART_DATA: *mut u32 = 0x4001_0000 as *mut u32;
const UART_CR: *mut u32 = (0x4001_0000 + 0x30) as *mut u32;
const UART_IMSC: *mut u32 = (0x4001_0000 + 0x38) as *mut u32;
const UART_RIS: *mut u32 = (0x4001_0000 + 0x3C) as *mut u32;
// m4mem: 512 KiB SRAM at 0x2000_0000, mirrored at 0x0.
const SRAM: u32 = 0x2000_0000;
// Renode systemDmaSram: 4 KiB at 0x4000_F000.
const SDMA_SRAM: *mut u32 = 0x4000_F000 as *mut u32;
// qorc-sdk eoss3_dev.h: MISC_CTRL 0x4000_5000, IO_INPUT +0x100, IO_OUTPUT +0x104.
const IO_INPUT: *mut u32 = 0x4000_5100 as *mut u32;
const IO_OUTPUT: *mut u32 = 0x4000_5104 as *mut u32;
// qorc-sdk eoss3_dev.h INTR_CTRL at 0x4000_4800.
const INTR_CTRL: u32 = 0x4000_4800;
const GPIO_INTR_EN_M4: *mut u32 = (INTR_CTRL + 0x14) as *mut u32;
const OTHER_INTR: *mut u32 = (INTR_CTRL + 0x30) as *mut u32;
const OTHER_INTR_EN_M4: *mut u32 = (INTR_CTRL + 0x38) as *mut u32;
const SW_INTR_1: *mut u32 = (INTR_CTRL + 0x40) as *mut u32;
const SW_INTR_1_EN_M4: *mut u32 = (INTR_CTRL + 0x48) as *mut u32;
const SW_INTR_2: *mut u32 = (INTR_CTRL + 0x50) as *mut u32;
const SW_INTR_2_EN_M4: *mut u32 = (INTR_CTRL + 0x58) as *mut u32;
// NVIC ISER0 / ISPR0. SwInt2_IRQn = 0, SwInt1_IRQn = 1.
const NVIC_ISER0: *mut u32 = 0xE000_E100 as *mut u32;
const NVIC_ISPR0: *mut u32 = 0xE000_E200 as *mut u32;
// qorc-sdk eoss3_dev.h SPT_BASE = AON_PERIPH_BASE + 0x5C00 (SPT_REGS_TypeDef).
const SPT: u32 = 0x4000_5C00;
const SPT_CFG: *mut u32 = SPT as *mut u32;
const SPT_UPDATE_TMR_VAL: *mut u32 = (SPT + 0x2C) as *mut u32;
const SPT_TIMER_VAL: *mut u32 = (SPT + 0x34) as *mut u32;
const SPT_EVENT_CNT_VAL: *mut u32 = (SPT + 0x38) as *mut u32;
const SPT_MS_CNT_VAL: *mut u32 = (SPT + 0x3C) as *mut u32;
// SysTick on the core clock: the reference for one SPT millisecond.
const SYST_CSR: *mut u32 = 0xE000_E010 as *mut u32;
const SYST_RVR: *mut u32 = 0xE000_E014 as *mut u32;
const SYST_CVR: *mut u32 = 0xE000_E018 as *mut u32;
// Smoke summary: 0xE0530000 | pass bits.
const SUMMARY: *mut u32 = 0x2007_FF00 as *mut u32;

static MARKER: u32 = 0x5EED_E053;
static mut DATA_WORD: u32 = 0;
static SW1_HITS: AtomicU32 = AtomicU32::new(0);
static SW2_HITS: AtomicU32 = AtomicU32::new(0);
static UART_HITS: AtomicU32 = AtomicU32::new(0);
static UART_OTHER_BEFORE: AtomicU32 = AtomicU32::new(0);
static UART_OTHER_AFTER: AtomicU32 = AtomicU32::new(0);
static UART_RIS_BEFORE: AtomicU32 = AtomicU32::new(0);
static UART_RX_BYTE: AtomicU32 = AtomicU32::new(0);
static OTHER_HITS: AtomicU32 = AtomicU32::new(0);
static SPT_MS: AtomicU32 = AtomicU32::new(0);
static SPT_CYCLES: AtomicU32 = AtomicU32::new(0);

// Every IRQ lands here; the handler clears the software request it serves.
#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    match irqn {
        1 => {
            SW1_HITS.fetch_add(1, Ordering::Relaxed);
            write_volatile(SW_INTR_1, 0);
        }
        0 => {
            SW2_HITS.fetch_add(1, Ordering::Relaxed);
            write_volatile(SW_INTR_2, 0);
        }
        7 => {
            UART_OTHER_BEFORE.store(read_volatile(OTHER_INTR), Ordering::Relaxed);
            UART_RIS_BEFORE.store(read_volatile(UART_RIS as *const u32), Ordering::Relaxed);
            UART_RX_BYTE.store(read_volatile(UART_DATA), Ordering::Relaxed);
            UART_OTHER_AFTER.store(read_volatile(OTHER_INTR), Ordering::Relaxed);
            UART_HITS.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            OTHER_HITS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn settle() {
    for _ in 0..16 {
        core::hint::spin_loop();
    }
}

fn puts(s: &[u8]) {
    for byte in s {
        unsafe { write_volatile(UART_DATA, u32::from(*byte)) };
    }
}

fn put_hex32(mut value: u32) {
    for shift in (0..8).rev() {
        let digit = ((value >> (shift * 4)) & 0xF) as u8;
        let byte = if digit < 10 {
            b'0' + digit
        } else {
            b'A' + digit - 10
        };
        unsafe { write_volatile(UART_DATA, u32::from(byte)) };
        value &= !(0xF << (shift * 4));
    }
}

#[entry]
fn main() -> ! {
    let mut pass = 0u32;
    let mut marks = [b'F'; 12];
    let mut check = |i: usize, ok: bool| {
        if ok {
            pass |= 1 << i;
            marks[i] = b'P';
        }
    };
    unsafe {
        // 0: code linked at 0x0 reads back through the 0x2000_0000 view.
        let marker = &MARKER as *const u32 as u32;
        check(
            0,
            read_volatile((SRAM + marker) as *const u32) == 0x5EED_E053,
        );

        // 1: a .data store at 0x2004_xxxx reads back through the 0x0 view.
        let data = core::ptr::addr_of_mut!(DATA_WORD);
        write_volatile(data, 0xA11A_5001);
        check(
            1,
            read_volatile((data as u32 - SRAM) as *const u32) == 0xA11A_5001,
        );

        // 2: a store through the 0x0 view lands in SRAM.
        let low = 0x0003_F000 as *mut u32;
        write_volatile(low, 0x0B5E_55ED);
        check(
            2,
            read_volatile((SRAM + 0x0003_F000) as *const u32) == 0x0B5E_55ED,
        );

        // 3: SDMA SRAM holds a word at both ends.
        write_volatile(SDMA_SRAM, 0x5D3A_0001);
        write_volatile(SDMA_SRAM.add(1023), 0x5D3A_03FF);
        check(
            3,
            read_volatile(SDMA_SRAM) == 0x5D3A_0001
                && read_volatile(SDMA_SRAM.add(1023)) == 0x5D3A_03FF,
        );

        // 4: IO_OUTPUT keeps bits [7:0] only.
        write_volatile(IO_OUTPUT, 0xFFFF_FF50);
        let out = read_volatile(IO_OUTPUT);
        write_volatile(IO_OUTPUT, 0);
        check(4, out == 0x50 && read_volatile(IO_OUTPUT) == 0);

        // 5: IO_INPUT reads the pad inputs (none driven), not IO_OUTPUT.
        write_volatile(IO_OUTPUT, 0xFF);
        let input = read_volatile(IO_INPUT);
        write_volatile(IO_OUTPUT, 0);
        check(5, input == 0);

        // 6: SW_INTR_1 with EN_M4 clear is stored but raises nothing.
        write_volatile(NVIC_ISER0, 0b11);
        write_volatile(SW_INTR_1, 1);
        settle();
        check(
            6,
            read_volatile(SW_INTR_1) == 1
                && SW1_HITS.load(Ordering::Relaxed) == 0
                && read_volatile(NVIC_ISPR0) & 0b11 == 0,
        );

        // 7: setting EN_M4 over the held request enters NVIC 1 once; the
        // handler's clear ends it.
        write_volatile(SW_INTR_1_EN_M4, 1);
        settle();
        check(
            7,
            SW1_HITS.load(Ordering::Relaxed) == 1 && read_volatile(SW_INTR_1) == 0,
        );

        // 8: SW_INTR_2 (enabled first) enters NVIC 0 once; nothing else fires.
        write_volatile(SW_INTR_2_EN_M4, 1);
        write_volatile(SW_INTR_2, 1);
        settle();
        check(
            8,
            SW2_HITS.load(Ordering::Relaxed) == 1
                && read_volatile(SW_INTR_2) == 0
                && SW1_HITS.load(Ordering::Relaxed) == 1
                && read_volatile(NVIC_ISPR0) & 0b11 == 0,
        );

        // 9: a second SW1 request enters again; the enable words keep their
        // documented widths, and OTHER_INTR stays low while UART RX is off.
        write_volatile(SW_INTR_1, 1);
        settle();
        write_volatile(GPIO_INTR_EN_M4, 0xFFFF_FFFF);
        write_volatile(OTHER_INTR_EN_M4, 0xFFFF_FFFF);
        let gpio_en = read_volatile(GPIO_INTR_EN_M4);
        let other_en = read_volatile(OTHER_INTR_EN_M4);
        write_volatile(GPIO_INTR_EN_M4, 0);
        write_volatile(OTHER_INTR_EN_M4, 0);
        check(
            9,
            SW1_HITS.load(Ordering::Relaxed) == 2
                && gpio_en == 0xFF
                && other_en == 0x00FF_FFFF
                && read_volatile(OTHER_INTR) == 0
                && OTHER_HITS.load(Ordering::Relaxed) == 0,
        );

        // 10: PL011 RX routes through OTHER_INTR bit 1 to NVIC 7. The handler
        // reads DR and must deassert the controller status and pending line.
        write_volatile(OTHER_INTR_EN_M4, 1 << 1);
        write_volatile(NVIC_ISER0, 1 << 7);
        write_volatile(UART_CR, (1 << 0) | (1 << 9)); // UARTEN | RXE
        write_volatile(UART_IMSC, 1 << 4); // RXIM
        for _ in 0..50_000 {
            if UART_HITS.load(Ordering::Relaxed) != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        check(
            10,
            UART_HITS.load(Ordering::Relaxed) == 1
                && UART_OTHER_BEFORE.load(Ordering::Relaxed) == 1 << 1
                && UART_RIS_BEFORE.load(Ordering::Relaxed) == 1 << 4
                && UART_RX_BYTE.load(Ordering::Relaxed) == u32::from(b'R')
                && UART_OTHER_AFTER.load(Ordering::Relaxed) == 0
                && read_volatile(OTHER_INTR) == 0
                && read_volatile(NVIC_ISPR0) & (1 << 7) == 0
                && OTHER_HITS.load(Ordering::Relaxed) == 0,
        );

        // 11: SPT. Disabled, UPDATE_TMR_VAL loads TIMER_VAL and it holds.
        // Enabled, it counts 1 ms at the 61.44 MHz core clock (SysTick
        // reference). Disabled again, it freezes.
        write_volatile(SPT_CFG, 0x10 << 10); // FFE_TO_PERIOD = 0x10, SPT_EN = 0
        let reset_ok = read_volatile(SPT_TIMER_VAL) == 0
            && read_volatile(SPT_MS_CNT_VAL) == 0x40
            && read_volatile(SPT_EVENT_CNT_VAL) == 0x10
            && read_volatile(SPT_CFG) == 0x10 << 10;
        write_volatile(SPT_UPDATE_TMR_VAL, 5);
        let loaded = read_volatile(SPT_TIMER_VAL);
        for _ in 0..20_000 {
            core::hint::spin_loop();
        }
        let held = read_volatile(SPT_TIMER_VAL);
        write_volatile(SYST_RVR, 0x00FF_FFFF);
        write_volatile(SYST_CVR, 0);
        write_volatile(SYST_CSR, 0b101); // ENABLE | CLKSOURCE = core
        write_volatile(SPT_CFG, (0x10 << 10) | 1);
        let start = read_volatile(SPT_TIMER_VAL);
        let mut edge = start;
        let mut guard = 0u32;
        while edge == start && guard < 200_000 {
            edge = read_volatile(SPT_TIMER_VAL);
            guard += 1;
        }
        let t0 = read_volatile(SYST_CVR);
        let mut next = edge;
        while next == edge && guard < 400_000 {
            next = read_volatile(SPT_TIMER_VAL);
            guard += 1;
        }
        let t1 = read_volatile(SYST_CVR);
        let cycles = t0.wrapping_sub(t1) & 0x00FF_FFFF;
        write_volatile(SPT_CFG, 0x10 << 10);
        let frozen = read_volatile(SPT_TIMER_VAL);
        for _ in 0..20_000 {
            core::hint::spin_loop();
        }
        let still = read_volatile(SPT_TIMER_VAL);
        SPT_MS.store(next, Ordering::Relaxed);
        SPT_CYCLES.store(cycles, Ordering::Relaxed);
        check(
            11,
            reset_ok
                && loaded == 5
                && held == 5
                && start == 5
                && edge == 6
                && next == 7
                && (55_000..68_000).contains(&cycles)
                && frozen == still
                && still == 7,
        );
    }

    puts(b"SPT ms=");
    put_hex32(SPT_MS.load(Ordering::Relaxed));
    puts(b" cycles=");
    put_hex32(SPT_CYCLES.load(Ordering::Relaxed));
    puts(b"\r\n");

    puts(b"PL011 IRQ7 status=");
    put_hex32(UART_OTHER_BEFORE.load(Ordering::Relaxed));
    puts(b" rx=");
    put_hex32(UART_RX_BYTE.load(Ordering::Relaxed));
    puts(b" cleared=");
    put_hex32(UART_OTHER_AFTER.load(Ordering::Relaxed));
    puts(b" ris=");
    put_hex32(UART_RIS_BEFORE.load(Ordering::Relaxed));
    puts(b" hits=");
    put_hex32(UART_HITS.load(Ordering::Relaxed));
    puts(b"\r\n");

    puts(b"EOS S3 Cortex-M4F PL011 smoke\r\n");
    // GPIO 4: three pulses for the gpio_edges assertion.
    for _ in 0..3 {
        unsafe {
            write_volatile(IO_OUTPUT, read_volatile(IO_OUTPUT) | (1 << 4));
            write_volatile(IO_OUTPUT, read_volatile(IO_OUTPUT) & !(1 << 4));
        }
    }
    unsafe { write_volatile(SUMMARY, 0xE053_0000 | pass) };
    puts(b"EOS S3 checks ");
    puts(&marks);
    puts(b" DONE\r\n");
    loop {
        core::hint::spin_loop();
    }
}
