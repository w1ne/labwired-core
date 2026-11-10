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
//!   SPI1   MAX31855 reset frame plus TXEIE/RXNEIE level IRQ assertions
//! A summary word goes to RAM at 0x20017F00 (one bit per block passed).

use core::ptr::{read_volatile, write_volatile};

const RCC_AHB1ENR: u32 = 0x4002_3830;
const RCC_APB1ENR: u32 = 0x4002_3840;
const RCC_APB2ENR: u32 = 0x4002_3844;

const GPIOA: u32 = 0x4002_0000;
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

const SPI1: u32 = 0x4001_3000;
const SPI_CR1: u32 = SPI1;
const SPI_CR2: u32 = SPI1 + 0x04;
const SPI_SR: u32 = SPI1 + 0x08;
const SPI_DR: u32 = SPI1 + 0x0C;
const NVIC_ISPR1: u32 = 0xE000_E204;
const NVIC_ICPR1: u32 = 0xE000_E284;
const SPI1_IRQ_BIT: u32 = 1 << (35 - 32);

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
    // GPIOA + GPIOC on AHB1; TIM2, USART2, I2C1 on APB1; SPI1 on APB2.
    wr(RCC_AHB1ENR, rd(RCC_AHB1ENR) | (1 << 0) | (1 << 2));
    wr(
        RCC_APB1ENR,
        rd(RCC_APB1ENR) | (1 << 0) | (1 << 17) | (1 << 21),
    );
    wr(RCC_APB2ENR, rd(RCC_APB2ENR) | (1 << 12));
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

    let spi_ok = smoke_spi1();
    passed |= (spi_ok as u32) << 4;

    wr(RESULT, 0xB1AC_0000 | passed);
    send(if passed == 0x1F {
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

/// The fixture MAX31855 is on SPI1 with PA4 as GPIO chip select. Check the
/// idle TXE interrupt level, then the receive level before reading each DR
/// byte and its release after the last byte is consumed.
fn smoke_spi1() -> bool {
    const TXE: u32 = 1 << 1;
    const RXNE: u32 = 1;
    const BSY: u32 = 1 << 7;

    // PA4 as a push-pull output, initially deasserted (high).
    wr(
        GPIOA + MODER,
        (rd(GPIOA + MODER) & !(0b11 << 8)) | (0b01 << 8),
    );
    wr(GPIOA + BSRR, 1 << 4);

    // SPI1 master, software NSS, mode 0, 8-bit frames, PCLK2/8.
    wr(SPI_CR1, (1 << 9) | (1 << 8) | (0b010 << 3) | (1 << 2));
    wr(SPI_CR1, rd(SPI_CR1) | (1 << 6)); // SPE
    wr(NVIC_ICPR1, SPI1_IRQ_BIT);
    wr(SPI_CR2, 1 << 7); // TXEIE
    let txe_pended = rd(SPI_SR) & TXE != 0 && rd(NVIC_ISPR1) & SPI1_IRQ_BIT != 0;
    wr(SPI_CR2, 0);
    let txe_released = rd(NVIC_ISPR1) & SPI1_IRQ_BIT == 0;
    wr(NVIC_ICPR1, SPI1_IRQ_BIT);

    wr(SPI_CR2, 1 << 6); // RXNEIE
    wr(GPIOA + BSRR, 1 << (4 + 16)); // select MAX31855
    let mut frame = 0u32;
    let mut rxne_pended = true;
    let mut transfer_ok = true;
    for _ in 0..4 {
        if !wait(SPI_SR, TXE, TXE) {
            transfer_ok = false;
            break;
        }
        wr(SPI_DR, 0);
        if !wait(SPI_SR, RXNE, RXNE) {
            transfer_ok = false;
            break;
        }
        rxne_pended &= rd(NVIC_ISPR1) & SPI1_IRQ_BIT != 0;
        frame = (frame << 8) | (rd(SPI_DR) & 0xFF);
    }
    let idle = wait(SPI_SR, BSY, 0);
    wr(GPIOA + BSRR, 1 << 4); // deselect
    let rxne_released = rd(NVIC_ISPR1) & SPI1_IRQ_BIT == 0 && rd(SPI_SR) & RXNE == 0;
    wr(SPI_CR2, 0);
    wr(SPI_CR1, 0);

    let ok = transfer_ok
        && idle
        && frame == 0x0190_1600
        && txe_pended
        && txe_released
        && rxne_pended
        && rxne_released;
    send(b"spi1 max31855=");
    put_hex32(frame);
    send(b" txe_irq=");
    putc(if txe_pended { b'1' } else { b'0' });
    send(b" rxne_irq=");
    putc(if rxne_pended { b'1' } else { b'0' });
    send(b" released=");
    putc(if txe_released && rxne_released {
        b'1'
    } else {
        b'0'
    });
    send(if ok { b" ok\r\n" } else { b" FAIL\r\n" });
    ok
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
