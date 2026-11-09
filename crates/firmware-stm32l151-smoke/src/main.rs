#![no_std]
#![no_main]

//! STM32L151xD smoke for the imported Renode platform: USART1/2/3, GPIOA..H,
//! SYSCFG EXTICR + EXTI, TIM2..TIM7 and TIM9..TIM11. Register offsets, bases
//! and IRQ numbers are ST's CMSIS `stm32l151xd.h`.
//!
//! Interrupts stay disabled in the NVIC: each check reads the line's
//! ISPR bit, which proves the block reaches the right NVIC input without a
//! device vector table. Every check prints one line and sets one bit of the
//! summary word at `SUMMARY`.

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const SUMMARY: u32 = 0x2000_8000;

// USART (classic SR/DR/BRR/CR1 layout).
const USART1: u32 = 0x4001_3800;
const USART2: u32 = 0x4000_4400;
const USART3: u32 = 0x4000_4800;
const U_SR: u32 = 0x00;
const U_DR: u32 = 0x04;
const U_BRR: u32 = 0x08;
const U_CR1: u32 = 0x0C;
const USART_SR_TXE: u32 = 1 << 7;

// GPIO (AHB). The L1 address order puts H before F and G.
const GPIOA: u32 = 0x4002_0000;
const GPIOB: u32 = 0x4002_0400;
const GPIOC: u32 = 0x4002_0800;
const GPIOH: u32 = 0x4002_1400;
const GPIOF: u32 = 0x4002_1800;
const GPIOG: u32 = 0x4002_1C00;
const G_MODER: u32 = 0x00;
const G_OSPEEDR: u32 = 0x08;
const G_PUPDR: u32 = 0x0C;
const G_IDR: u32 = 0x10;
const G_ODR: u32 = 0x14;
const G_BSRR: u32 = 0x18;
const G_BRR: u32 = 0x28;

// SYSCFG / EXTI.
const SYSCFG_EXTICR1: u32 = 0x4001_0008;
const SYSCFG_EXTICR4: u32 = 0x4001_0014;
const EXTI: u32 = 0x4001_0400;
const E_IMR: u32 = 0x00;
const E_FTSR: u32 = 0x0C;
const E_SWIER: u32 = 0x10;
const E_PR: u32 = 0x14;
// SYSCFG_EXTICRx port codes (stm32l151xd.h SYSCFG_EXTICR1_EXTI0_Px).
const CODE_PC: u32 = 2;
const CODE_PH: u32 = 5;
const CODE_PG: u32 = 7;

// TIMx.
const T_CR1: u32 = 0x00;
const T_DIER: u32 = 0x0C;
const T_SR: u32 = 0x10;
const T_CNT: u32 = 0x24;
const T_PSC: u32 = 0x28;
const T_ARR: u32 = 0x2C;

const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;

/// (name, base, NVIC line, counter width in bits) — `stm32l151xd.h`
/// TIMx_BASE / TIMx_IRQn; widths from RM0038 (TIM5 is the 32-bit one).
const TIMERS: [(&str, u32, u32, u32); 9] = [
    ("tim2", 0x4000_0000, 28, 16),
    ("tim3", 0x4000_0400, 29, 16),
    ("tim4", 0x4000_0800, 30, 16),
    ("tim5", 0x4000_0C00, 46, 32),
    ("tim6", 0x4000_1000, 43, 16),
    ("tim7", 0x4000_1400, 44, 16),
    ("tim9", 0x4001_0800, 25, 16),
    ("tim10", 0x4001_0C00, 26, 16),
    ("tim11", 0x4001_1000, 27, 16),
];

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn nvic(reg: u32, irq: u32) -> (u32, u32) {
    (reg + 4 * (irq / 32), irq % 32)
}

fn pending(irq: u32) -> bool {
    let (ispr, b) = nvic(NVIC_ISPR0, irq);
    rd(ispr) & (1 << b) != 0
}

fn unpend(irq: u32) {
    let (icpr, b) = nvic(NVIC_ICPR0, irq);
    wr(icpr, 1 << b);
}

fn uart_init(base: u32) {
    wr(base + U_BRR, 0x45);
    wr(base + U_CR1, (1 << 13) | (1 << 3)); // UE | TE
}

fn send(base: u32, bytes: &[u8]) {
    for byte in bytes {
        let mut remaining = 100_000;
        while rd(base + U_SR) & USART_SR_TXE == 0 && remaining > 0 {
            remaining -= 1;
        }
        wr(base + U_DR, u32::from(*byte));
    }
}

fn puts(s: &str) {
    send(USART1, s.as_bytes());
}

fn flag(name: &str, ok: bool) {
    puts(name);
    puts(if ok { "=1 " } else { "=0 " });
}

fn verdict(ok: bool) {
    puts(if ok { "ok\r\n" } else { "FAIL\r\n" });
}

fn hex(v: u32) {
    let digits = b"0123456789ABCDEF";
    let mut buf = [0u8; 10];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..8 {
        buf[2 + i] = digits[((v >> (28 - 4 * i)) & 0xF) as usize];
    }
    send(USART1, &buf);
}

/// RM0038 reset values of the debug-port pads, and set/reset through
/// BSRR/BRR on port H (the first port after E, at 0x40021400).
fn check_gpio() -> bool {
    let pa = rd(GPIOA + G_MODER) == 0xA800_0000 && rd(GPIOA + G_PUPDR) == 0x6400_0000;
    let pb = rd(GPIOB + G_MODER) == 0x0000_0280
        && rd(GPIOB + G_OSPEEDR) == 0x0000_00C0
        && rd(GPIOB + G_PUPDR) == 0x0000_0100;
    let pc = rd(GPIOC + G_MODER) == 0;
    // PH0 output; toggle it four times for the gpio_edges assertion.
    wr(GPIOH + G_MODER, 0b01);
    let mut toggles = true;
    for _ in 0..2 {
        wr(GPIOH + G_BSRR, 1);
        toggles &= rd(GPIOH + G_ODR) & 1 == 1 && rd(GPIOH + G_IDR) & 1 == 1;
        wr(GPIOH + G_BRR, 1);
        toggles &= rd(GPIOH + G_ODR) & 1 == 0;
    }
    // F and G are separate ports above H.
    wr(GPIOF + G_ODR, 0x00F0);
    wr(GPIOG + G_ODR, 0x0F00);
    let fg = rd(GPIOF + G_ODR) == 0x00F0 && rd(GPIOG + G_ODR) == 0x0F00 && rd(GPIOH + G_ODR) == 0;
    puts("gpio ");
    flag("pa", pa);
    flag("pb", pb);
    flag("pc", pc);
    flag("ph", toggles);
    flag("fg", fg);
    let ok = pa && pb && pc && toggles && fg;
    verdict(ok);
    ok
}

/// SWIER on lines 0, 7 and 12 reaches the grouped NVIC vectors
/// (EXTI0 = 6, EXTI9_5 = 23, EXTI15_10 = 40), and only while unmasked.
fn check_exti_swier() -> bool {
    wr(EXTI + E_IMR, 0);
    wr(EXTI + E_SWIER, 1 << 0);
    let masked = rd(EXTI + E_PR) & 1 == 1 && !pending(6);
    wr(EXTI + E_PR, 1);
    unpend(6);
    let mut routed = true;
    for &(line, irq) in &[(0u32, 6u32), (7, 23), (12, 40)] {
        wr(EXTI + E_IMR, 1 << line);
        wr(EXTI + E_SWIER, 1 << line);
        routed &= rd(EXTI + E_PR) & (1 << line) != 0 && pending(irq);
        wr(EXTI + E_PR, 1 << line);
        unpend(irq);
        routed &= rd(EXTI + E_PR) & (1 << line) == 0 && !pending(irq);
    }
    wr(EXTI + E_IMR, 0);
    puts("exti swier ");
    flag("masked", masked);
    flag("routed", routed);
    let ok = masked && routed;
    verdict(ok);
    ok
}

fn wait_level(port: u32, pin: u32, high: bool) -> bool {
    for _ in 0..2_000_000 {
        if (rd(port + G_IDR) >> pin) & 1 == u32::from(high) {
            return true;
        }
    }
    false
}

/// Falling edges from the two fixture buttons (PH1, PC13; pressed = low)
/// through the SYSCFG port mux. Line 1 first selects code 7 (PG on the L1):
/// the first PH1 press must not pend it. With code 5 (PH on the L1) the
/// second press must, on NVIC EXTI1 = 7. PC13 lands on EXTI15_10 = 40.
fn check_exti_pads() -> bool {
    // Inputs with pull-up.
    wr(GPIOH + G_PUPDR, 0b01 << 2);
    wr(GPIOC + G_PUPDR, 0b01 << 26);
    wr(SYSCFG_EXTICR1, CODE_PG << 4);
    wr(SYSCFG_EXTICR4, CODE_PC << 4);
    wr(EXTI + E_FTSR, (1 << 1) | (1 << 13));
    wr(EXTI + E_IMR, (1 << 1) | (1 << 13));

    let idle = wait_level(GPIOH, 1, true) && wait_level(GPIOC, 13, true);
    let press1 = wait_level(GPIOH, 1, false) && wait_level(GPIOH, 1, true);
    let other_port_quiet = rd(EXTI + E_PR) & (1 << 1) == 0 && !pending(7);

    wr(SYSCFG_EXTICR1, CODE_PH << 4);
    let press2 = wait_level(GPIOH, 1, false) && wait_level(GPIOH, 1, true);
    let ph = rd(EXTI + E_PR) & (1 << 1) != 0 && pending(7);
    wr(EXTI + E_PR, 1 << 1);
    unpend(7);

    let press3 = wait_level(GPIOC, 13, false) && wait_level(GPIOC, 13, true);
    let pc = rd(EXTI + E_PR) & (1 << 13) != 0 && pending(40);
    wr(EXTI + E_PR, 1 << 13);
    unpend(40);
    wr(EXTI + E_IMR, 0);

    puts("exti pads ");
    flag("idle", idle);
    flag("presses", press1 && press2 && press3);
    flag("pg_quiet", other_port_quiet);
    flag("ph1", ph);
    flag("pc13", pc);
    let ok = idle && press1 && press2 && press3 && other_port_quiet && ph && pc;
    verdict(ok);
    ok
}

/// Counter width (ARR keeps 16 or 32 bits), counting, and the update
/// interrupt reaching the timer's own NVIC line — and no other timer's.
fn check_timer(name: &str, base: u32, irq: u32, width: u32) -> bool {
    wr(base + T_ARR, 0xFFFF_FFFF);
    let top = if width == 32 { 0xFFFF_FFFF } else { 0xFFFF };
    let arr = rd(base + T_ARR);
    let width_ok = arr == top;

    wr(base + T_PSC, 0);
    wr(base + T_ARR, 999);
    wr(base + T_SR, 0);
    unpend(irq);
    wr(base + T_DIER, 1); // UIE
    wr(base + T_CR1, 1); // CEN
    let c0 = rd(base + T_CNT);
    let mut uif = false;
    for _ in 0..200_000 {
        if rd(base + T_SR) & 1 != 0 {
            uif = true;
            break;
        }
    }
    let counted = rd(base + T_CNT) != c0 || uif;
    let pend = pending(irq);
    // No other timer line is pending.
    let alone = TIMERS
        .iter()
        .all(|&(_, _, other, _)| other == irq || !pending(other));
    wr(base + T_CR1, 0);
    wr(base + T_DIER, 0);
    wr(base + T_SR, 0);
    unpend(irq);
    let released = !pending(irq);

    puts(name);
    puts(" arr=");
    hex(arr);
    puts(" ");
    flag("width", width_ok);
    flag("count", counted);
    flag("uif", uif);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = width_ok && counted && uif && pend && alone && released;
    verdict(ok);
    ok
}

#[entry]
fn main() -> ! {
    uart_init(USART1);
    uart_init(USART2);
    uart_init(USART3);
    puts("STM32L151 Cortex-M3 smoke\r\n");
    send(USART2, b"usart2 ok\r\n");
    send(USART3, b"usart3 ok\r\n");

    let mut summary: u32 = 0;
    let mut mark = |bit: u32, ok: bool| {
        if ok {
            summary |= 1 << bit;
        }
    };
    mark(0, check_gpio());
    mark(1, check_exti_swier());
    for (i, &(name, base, irq, width)) in TIMERS.iter().enumerate() {
        mark(2 + i as u32, check_timer(name, base, irq, width));
    }
    mark(11, check_exti_pads());
    let summary = summary | 0x1510_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
