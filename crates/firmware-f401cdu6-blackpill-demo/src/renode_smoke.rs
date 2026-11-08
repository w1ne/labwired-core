#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

//! Renode-onboarding smoke for the STM32F401CDU6 Black Pill. Bare registers,
//! no .data/.bss (minimal.ld has neither). Each block reads its own
//! peripheral back and reports over USART2:
//!   RCC    AHB1ENR/APB1ENR enable bits read back as written
//!   GPIOC  PC13 (active-low user LED) as push-pull output, toggled via BSRR,
//!          ODR read back after each write
//!   TIM2   PSC/ARR readback, up-counter advances, UIF latches and clears (W1C 0)
//!   I2C1   OAR1 readback, START -> SB, address with no slave -> AF (NACK),
//!          AF rc_w0 clear, STOP releases BUSY (the bare board has no I2C parts)
//! A summary word goes to RAM at 0x20017F00 (one bit per block passed).

use core::ptr::{read_volatile, write_volatile};

const RCC_AHB1ENR: u32 = 0x4002_3830;
const RCC_APB1ENR: u32 = 0x4002_3840;

const GPIOC: u32 = 0x4002_0800;
const MODER: u32 = 0x00;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;

const USART2: u32 = 0x4000_4400;
const USART_SR: u32 = USART2;
const USART_DR: u32 = USART2 + 0x04;
const USART_BRR: u32 = USART2 + 0x08;
const USART_CR1: u32 = USART2 + 0x0C;

const TIM2: u32 = 0x4000_0000;
const TIM_CR1: u32 = TIM2;
const TIM_SR: u32 = TIM2 + 0x10;
const TIM_EGR: u32 = TIM2 + 0x14;
const TIM_CNT: u32 = TIM2 + 0x24;
const TIM_PSC: u32 = TIM2 + 0x28;
const TIM_ARR: u32 = TIM2 + 0x2C;

const I2C1: u32 = 0x4000_5400;
const I2C_CR1: u32 = I2C1;
const I2C_CR2: u32 = I2C1 + 0x04;
const I2C_OAR1: u32 = I2C1 + 0x08;
const I2C_DR: u32 = I2C1 + 0x10;
const I2C_SR1: u32 = I2C1 + 0x14;
const I2C_SR2: u32 = I2C1 + 0x18;
const I2C_CCR: u32 = I2C1 + 0x1C;
const I2C_TRISE: u32 = I2C1 + 0x20;

const RESULT: u32 = 0x2001_7F00;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn putc(b: u8) {
    while rd(USART_SR) & (1 << 7) == 0 {}
    wr(USART_DR, b as u32);
}

fn send(s: &[u8]) {
    for b in s {
        putc(*b);
    }
}

fn put_hex32(v: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in (0..8).rev() {
        putc(HEX[((v >> (i * 4)) & 0xF) as usize]);
    }
}

fn line(label: &[u8], value: u32, ok: bool) {
    send(label);
    put_hex32(value);
    send(if ok { b" ok\r\n" } else { b" FAIL\r\n" });
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    // GPIOA + GPIOC on AHB1; TIM2, USART2, I2C1 on APB1.
    wr(RCC_AHB1ENR, rd(RCC_AHB1ENR) | (1 << 0) | (1 << 2));
    wr(
        RCC_APB1ENR,
        rd(RCC_APB1ENR) | (1 << 0) | (1 << 17) | (1 << 21),
    );
    // HSI = 16 MHz, 115200 baud: USARTDIV 8.6875 -> 0x8B.
    wr(USART_BRR, 0x8B);
    wr(USART_CR1, (1 << 13) | (1 << 3)); // UE | TE
    send(b"BLACKPILL F401 SMOKE\r\n");

    let mut passed = 0u32;

    let ahb = rd(RCC_AHB1ENR) & 0x5;
    let apb = rd(RCC_APB1ENR) & ((1 << 0) | (1 << 17) | (1 << 21));
    let rcc_ok = ahb == 0x5 && apb == 0x0022_0001;
    line(b"rcc apb1=", apb, rcc_ok);
    passed |= rcc_ok as u32;

    let gpio_ok = smoke_led();
    passed |= (gpio_ok as u32) << 1;

    let tim_ok = smoke_tim2();
    passed |= (tim_ok as u32) << 2;

    let (nack_sr1, i2c_ok) = smoke_i2c1();
    line(b"i2c1 nack sr1=", nack_sr1, i2c_ok);
    passed |= (i2c_ok as u32) << 3;

    wr(RESULT, 0xB1AC_0000 | passed);
    send(if passed == 0xF {
        b"BLACKPILL PASS\r\n"
    } else {
        b"BLACKPILL FAIL\r\n"
    });

    loop {}
}

/// PC13 drives the active-low LED: BSRR reset half lights it.
fn smoke_led() -> bool {
    wr(
        GPIOC + MODER,
        (rd(GPIOC + MODER) & !(0b11 << 26)) | (0b01 << 26),
    );
    let moder_ok = (rd(GPIOC + MODER) >> 26) & 0b11 == 0b01;
    let mut ok = moder_ok;
    for _ in 0..3 {
        wr(GPIOC + BSRR, 1 << (13 + 16)); // LED on (pin low)
        let on = rd(GPIOC + ODR) & (1 << 13);
        wr(GPIOC + BSRR, 1 << 13); // LED off (pin high)
        let off = rd(GPIOC + ODR) & (1 << 13);
        ok &= on == 0 && off == 1 << 13;
    }
    line(b"led pc13 odr=", rd(GPIOC + ODR), ok);
    ok
}

fn smoke_tim2() -> bool {
    wr(TIM_PSC, 3);
    wr(TIM_ARR, 0x40);
    wr(TIM_EGR, 1); // UG: load PSC
    wr(TIM_SR, 0);
    let regs_ok = rd(TIM_PSC) == 3 && rd(TIM_ARR) == 0x40;
    wr(TIM_CR1, 1); // CEN
    let c0 = rd(TIM_CNT);
    let mut c1 = c0;
    for _ in 0..64 {
        c1 = rd(TIM_CNT);
        if c1 != c0 {
            break;
        }
    }
    let mut uif = false;
    for _ in 0..4096 {
        if rd(TIM_SR) & 1 != 0 {
            uif = true;
            break;
        }
    }
    wr(TIM_CR1, 0); // stop first so UIF cannot re-latch before the readback
    wr(TIM_SR, 0);
    let cleared = rd(TIM_SR) & 1 == 0;
    let ok = regs_ok && c1 != c0 && c1 <= 0x40 && uif && cleared;
    line(b"tim2 arr=", rd(TIM_ARR), ok);
    ok
}

/// Bare Black Pill: nothing sits on I2C1, so an addressed write must NACK.
/// Silicon (RM0368 §18.6.6) sets SR1.AF after the address phase with no ADDR,
/// keeps MSL|BUSY until STOP, and AF is rc_w0. Returns SR1 captured at the NACK
/// (0 when a step timed out) and whether the whole sequence matched.
fn smoke_i2c1() -> (u32, bool) {
    wr(I2C_OAR1, 0x4000 | 0xAA); // bit 14 must be kept at 1 by software
    if rd(I2C_OAR1) & 0xFFFF != 0x40AA {
        return (0, false);
    }
    wr(I2C_CR2, 16); // FREQ = 16 MHz APB1 (HSI)
    wr(I2C_CCR, 80); // 100 kHz standard mode
    wr(I2C_TRISE, 17);
    wr(I2C_CR1, 1); // PE
    wr(I2C_CR1, rd(I2C_CR1) | (1 << 8)); // START
    if !wait(I2C_SR1, 0x0001, 0x0001) {
        return (0, false);
    }
    wr(I2C_DR, 0xA0); // EEPROM-style address, write
    if !wait(I2C_SR1, 0x0400, 0x0400) {
        return (0, false);
    }
    let sr1 = rd(I2C_SR1);
    let sr2 = rd(I2C_SR2);
    let nack_ok = sr1 & 0x0002 == 0 && sr2 & 0x3 == 0x3; // no ADDR; MSL|BUSY
    wr(I2C_CR1, rd(I2C_CR1) | (1 << 9)); // STOP
    wr(I2C_SR1, !0x0400 & 0xFFFF); // clear AF (rc_w0)
    let af_cleared = rd(I2C_SR1) & 0x0400 == 0;
    let idle = wait(I2C_SR2, 0x0002, 0);
    (sr1, nack_ok && af_cleared && idle)
}

fn wait(addr: u32, mask: u32, want: u32) -> bool {
    for _ in 0..20_000 {
        if rd(addr) & mask == want {
            return true;
        }
    }
    false
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
