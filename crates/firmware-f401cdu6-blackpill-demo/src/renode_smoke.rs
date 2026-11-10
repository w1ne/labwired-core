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
//!   I2C1   TMP102 temperature-register pointer write + repeated-start read
//!          (25 °C = 0x1900), plus TXE/RXNE buffer IRQs, master-transmit and
//!          master-receive BTF event IRQs, absent-address NACK and error IRQ
//!          checks
//!   I2C2   a separate TMP102 repeated-start read, ITEVTEN-gated SB/ADDR/BTF event IRQs,
//!          TXE/RXNE isolation under ITEVTEN and TXE/RXNE assertion/release
//!          under ITBUFEN,
//!          and a POS-sequenced two-byte receive with
//!          STOP-before-DR, BTF/RXNE source isolation, and both final DR reads
//!          on the imported second controller instance; absent-address AF
//!          status, event/error-line isolation, ITERREN-gated ER IRQ assertion
//!          and release, and recovery
//!   SPI1   MAX31855 reset frame plus TXEIE/RXNEIE level IRQ assertions
//!   USART1 TXEIE, TCIE and RXNEIE assert and release their NVIC line;
//!   USART2 keeps injected bytes hidden until UE and RE are set, then drains
//!   two bytes in order and gates RXNE IRQ 38
//! A summary word goes to RAM at 0x20017F00 (one bit per smoke group passed).

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
const USART1: u32 = 0x4001_1000;
const USART1_SR: u32 = USART1;
const USART1_DR: u32 = USART1 + 0x04;
const USART1_CR1: u32 = USART1 + 0x0C;

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
const I2C2: u32 = 0x4000_5800;
const I2C2_CR1: u32 = I2C2;
const I2C2_CR2: u32 = I2C2 + 0x04;
const I2C2_DR: u32 = I2C2 + 0x10;
const I2C2_SR1: u32 = I2C2 + 0x14;
const I2C2_SR2: u32 = I2C2 + 0x18;
const I2C2_CCR: u32 = I2C2 + 0x1C;
const I2C2_TRISE: u32 = I2C2 + 0x20;

const SPI1: u32 = 0x4001_3000;
const SPI_CR1: u32 = SPI1;
const SPI_CR2: u32 = SPI1 + 0x04;
const SPI_SR: u32 = SPI1 + 0x08;
const SPI_DR: u32 = SPI1 + 0x0C;
const NVIC_ISPR1: u32 = 0xE000_E204;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;
const NVIC_ICPR1: u32 = 0xE000_E284;
const SPI1_IRQ_BIT: u32 = 1 << (35 - 32);
const USART2_IRQ_BIT: u32 = 1 << (38 - 32);
const USART1_IRQ_BIT: u32 = 1 << (37 - 32);
const I2C1_EV_IRQ_BIT: u32 = 1 << 31;
const I2C1_ER_IRQ_BIT: u32 = 1;
const I2C2_EV_IRQ_BIT: u32 = 1 << 1;
const I2C2_ER_IRQ_BIT: u32 = 1 << 2;

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
    // GPIOA + GPIOC on AHB1; TIM2, USART2, I2C1/2 on APB1; USART1 + SPI1 on APB2.
    wr(RCC_AHB1ENR, rd(RCC_AHB1ENR) | (1 << 0) | (1 << 2));
    wr(
        RCC_APB1ENR,
        rd(RCC_APB1ENR) | (1 << 0) | (1 << 17) | (1 << 21) | (1 << 22),
    );
    wr(RCC_APB2ENR, rd(RCC_APB2ENR) | (1 << 12));
    // HSI = 16 MHz, 115200 baud: USARTDIV 8.6875 -> 0x8B.
    wr(USART_BRR, 0x8B);
    wr(USART_CR1, (1 << 13) | (1 << 3)); // UE | TE
    send(b"BLACKPILL F401 SMOKE\r\n");

    let mut passed = 0u32;

    let ahb = rd(RCC_AHB1ENR) & 0x5;
    let apb = rd(RCC_APB1ENR) & ((1 << 0) | (1 << 17) | (1 << 21) | (1 << 22));
    let rcc_ok = ahb == 0x5 && apb == 0x0062_0001;
    line(b"rcc apb1=", apb, rcc_ok);
    passed |= rcc_ok as u32;

    let gpio_ok = smoke_led();
    passed |= (gpio_ok as u32) << 1;

    let tim_ok = smoke_tim2();
    passed |= (tim_ok as u32) << 2;

    let (
        temperature,
        nack_sr1,
        i2c_buf_irq,
        i2c_buf_released,
        i2c_ev_irq,
        i2c_ev_released,
        i2c_er_irq,
        i2c_irq_released,
        i2c_ok,
    ) = smoke_i2c1();
    send(b"i2c1 tmp102=");
    put_hex32(u32::from(temperature));
    send(b" nack=");
    put_hex32(nack_sr1);
    send(b" buf_irq=");
    send(if i2c_buf_irq { b"1" } else { b"0" });
    send(b" buf_release=");
    send(if i2c_buf_released { b"1" } else { b"0" });
    send(b" ev_irq=");
    send(if i2c_ev_irq { b"1" } else { b"0" });
    send(b" ev_release=");
    send(if i2c_ev_released { b"1" } else { b"0" });
    send(b" er_irq=");
    send(if i2c_er_irq { b"1" } else { b"0" });
    send(b" irq_released=");
    send(if i2c_irq_released { b"1" } else { b"0" });
    send(if i2c_ok { b" ok\r\n" } else { b" FAIL\r\n" });
    passed |= (i2c_ok as u32) << 3;

    let (
        i2c2_temperature,
        i2c2_sb_gate,
        i2c2_addr_event_gate,
        i2c2_txe_isolated,
        i2c2_txe_buf_irq,
        i2c2_txe_buf_released,
        i2c2_btf_event_gate,
        i2c2_rxne_buf_irq,
        i2c2_rxne_buf_released,
    ) = smoke_i2c2();
    let i2c2_ok = i2c2_temperature == 0x1900
        && i2c2_sb_gate
        && i2c2_addr_event_gate
        && i2c2_txe_isolated
        && i2c2_txe_buf_irq
        && i2c2_txe_buf_released
        && i2c2_btf_event_gate
        && i2c2_rxne_buf_irq
        && i2c2_rxne_buf_released;
    send(b"i2c2 tmp102=");
    put_hex32(u32::from(i2c2_temperature));
    send(b" sb_irq_gate=");
    send(if i2c2_sb_gate { b"1" } else { b"0" });
    send(b" addr_irq_gate=");
    send(if i2c2_addr_event_gate { b"1" } else { b"0" });
    send(b" txe_isolated=");
    send(if i2c2_txe_isolated { b"1" } else { b"0" });
    send(b" txe_buf_irq=");
    send(if i2c2_txe_buf_irq { b"1" } else { b"0" });
    send(b" txe_buf_release=");
    send(if i2c2_txe_buf_released { b"1" } else { b"0" });
    send(b" btf_irq_gate=");
    send(if i2c2_btf_event_gate { b"1" } else { b"0" });
    send(b" rxne_buf_irq=");
    send(if i2c2_rxne_buf_irq { b"1" } else { b"0" });
    send(b" rxne_buf_release=");
    send(if i2c2_rxne_buf_released { b"1" } else { b"0" });
    send(if i2c2_ok { b" ok\r\n" } else { b" FAIL\r\n" });
    passed |= (i2c2_ok as u32) << 7;

    let (
        i2c2_rx_btf_data,
        i2c2_pos_armed,
        i2c2_rx_btf_irq,
        i2c2_rxne_isolated,
        i2c2_stop_before_dr,
        i2c2_first_read_keeps_rxne,
        i2c2_rx_btf_released,
        i2c2_final_rxne_cleared,
    ) = smoke_i2c2_receive_btf();
    let i2c2_rx_btf_ok = i2c2_rx_btf_data == 0x1980
        && i2c2_pos_armed
        && i2c2_rx_btf_irq
        && i2c2_rxne_isolated
        && i2c2_stop_before_dr
        && i2c2_first_read_keeps_rxne
        && i2c2_rx_btf_released
        && i2c2_final_rxne_cleared;
    send(b"pos=");
    send(if i2c2_pos_armed { b"1" } else { b"0" });
    send(b" ");
    send(b"i2c2 btf_rx_irq=");
    send(if i2c2_rx_btf_irq { b"1" } else { b"0" });
    send(b" rxne_isolated=");
    send(if i2c2_rxne_isolated { b"1" } else { b"0" });
    send(b" stop_before_dr=");
    send(if i2c2_stop_before_dr { b"1" } else { b"0" });
    send(b" rxne_after_first=");
    send(if i2c2_first_read_keeps_rxne {
        b"1"
    } else {
        b"0"
    });
    send(b" release=");
    send(if i2c2_rx_btf_released { b"1" } else { b"0" });
    send(b" rxne_cleared=");
    send(if i2c2_final_rxne_cleared { b"1" } else { b"0" });
    send(b" data=");
    put_hex32(u32::from(i2c2_rx_btf_data));
    send(if i2c2_rx_btf_ok {
        b" ok\r\n"
    } else {
        b" FAIL\r\n"
    });
    passed |= (i2c2_rx_btf_ok as u32) << 8;

    let (
        i2c2_nack_sr1,
        i2c2_nack_flags,
        i2c2_nack_event_isolated,
        i2c2_nack_er_isolated,
        i2c2_nack_er_irq,
        i2c2_nack_er_masked_release,
        i2c2_nack_af_cleared,
        i2c2_nack_idle,
        i2c2_nack_er_released,
    ) = smoke_i2c2_nack();
    let i2c2_nack_ok = i2c2_nack_flags
        && i2c2_nack_event_isolated
        && i2c2_nack_er_isolated
        && i2c2_nack_er_irq
        && i2c2_nack_er_masked_release
        && i2c2_nack_af_cleared
        && i2c2_nack_idle
        && i2c2_nack_er_released;
    send(b"i2c2 nack=");
    put_hex32(i2c2_nack_sr1);
    send(b" no_addr=");
    send(if i2c2_nack_flags { b"1" } else { b"0" });
    send(b" ev_isolated=");
    send(if i2c2_nack_event_isolated { b"1" } else { b"0" });
    send(b" er_isolated=");
    send(if i2c2_nack_er_isolated { b"1" } else { b"0" });
    send(b" er_irq=");
    send(if i2c2_nack_er_irq { b"1" } else { b"0" });
    send(b" er_masked_release=");
    send(if i2c2_nack_er_masked_release {
        b"1"
    } else {
        b"0"
    });
    send(b" af_cleared=");
    send(if i2c2_nack_af_cleared { b"1" } else { b"0" });
    send(b" idle=");
    send(if i2c2_nack_idle { b"1" } else { b"0" });
    send(b" er_released=");
    send(if i2c2_nack_er_released { b"1" } else { b"0" });
    send(if i2c2_nack_ok {
        b" ok\r\n"
    } else {
        b" FAIL\r\n"
    });
    passed |= (i2c2_nack_ok as u32) << 9;

    let spi_ok = smoke_spi1();
    passed |= (spi_ok as u32) << 4;

    let usart1_irq_ok = smoke_usart1_txe_irq();
    passed |= (usart1_irq_ok as u32) << 5;
    let usart1_tc_irq_ok = smoke_usart1_tcie_irq();
    passed |= (usart1_tc_irq_ok as u32) << 10;
    let usart1_rx_irq_ok = smoke_usart1_rxne_irq();
    passed |= (usart1_rx_irq_ok as u32) << 6;
    let usart2_rx_gate_ok = smoke_usart2_rx_gate();
    passed |= (usart2_rx_gate_ok as u32) << 11;

    wr(RESULT, 0xB1AC_0000 | passed);
    send(if passed == 0xFFF {
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

/// Read the fixture TMP102 through the legacy STM32 I2C controller, then probe
/// an absent address. The exact imported system wires TMP102 at 0x48.
fn smoke_i2c1() -> (u16, u32, bool, bool, bool, bool, bool, bool, bool) {
    wr(I2C_OAR1, 0x4000 | 0xAA); // bit 14 must be kept at 1 by software
    if rd(I2C_OAR1) & 0xFFFF != 0x40AA {
        return (0xFFFF, 0, false, false, false, false, false, false, false);
    }
    // FREQ = 16 MHz APB1 (HSI). Start with ITEVTEN alone so SB drives the
    // shared EV line; buffer and error enables are checked separately.
    wr(I2C_CR2, 16 | (1 << 8) | (1 << 9));
    wr(I2C_CCR, 80); // 100 kHz standard mode
    wr(I2C_TRISE, 17);
    wr(I2C_CR1, 1); // PE

    // Write TMP102's temperature-register pointer (0), then use a repeated
    // START to read its two-byte big-endian result. ACK both address phases;
    // after the first byte, clear ACK so the final byte is NACKed.
    wr(I2C_CR1, rd(I2C_CR1) | (1 << 8)); // START
    let sb_irq =
        wait(NVIC_ISPR0, I2C1_EV_IRQ_BIT, I2C1_EV_IRQ_BIT) && rd(I2C_SR1) & 0x0001 == 0x0001;
    send(b"i2c1 sb_irq=");
    putc(if sb_irq { b'1' } else { b'0' });
    send(if sb_irq { b" ok\r\n" } else { b" FAIL\r\n" });
    if !wait(I2C_SR1, 0x0001, 0x0001) {
        return (0xFFFF, 0, false, false, false, false, false, false, false);
    }
    // SB is cleared by the address write. Switch to ITBUFEN alone first so
    // the next EV assertion can only come from TXE.
    wr(I2C_CR2, 16 | (1 << 8) | (1 << 10));
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    wr(I2C_DR, 0x90); // TMP102 7-bit address 0x48, write
    if !wait(I2C_SR1, 0x0002, 0x0002) {
        return (0xFFFF, 0, false, false, false, false, false, false, false);
    }

    // ADDR and TXE are both set. With ITEVTEN clear, TXE alone must assert
    // I2C1_EV through ITBUFEN. Disabling ITBUFEN must release the level even
    // while TXE remains set.
    let buffer_irq = rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT != 0;
    wr(I2C_CR2, 16 | (1 << 8)); // ITERREN only; ITBUFEN off, ITEVTEN off
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    let mut buffer_irq_released = true;
    for _ in 0..16 {
        let _ = rd(I2C_SR1);
        buffer_irq_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
    }

    // Now enable ITEVTEN alone. ADDR must assert the same EV line; the
    // SR1-to-SR2 sequence clears ADDR and releases it while TXE stays set.
    wr(I2C_CR2, 16 | (1 << 8) | (1 << 9));
    let mut event_irq = false;
    for _ in 0..16 {
        event_irq |= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT != 0;
    }
    let _ = rd(I2C_SR1);
    let _ = rd(I2C_SR2); // clear ADDR
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    let mut event_irq_released = true;
    for _ in 0..16 {
        let _ = rd(I2C_SR1);
        event_irq_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
    }

    // ITEVTEN remains enabled while ITBUFEN is off. TXE alone must not
    // pend EV; BTF is an event source and must assert the same line.
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    wr(I2C_DR, 0x00); // temperature register pointer
    if !wait(I2C_SR1, 0x0004, 0x0004) {
        // BTF: pointer byte sent
        return (
            0xFFFF,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            false,
            false,
        );
    }
    let btf_irq = rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT != 0;
    send(b"i2c1 btf_irq=");
    putc(if btf_irq { b'1' } else { b'0' });
    send(if btf_irq { b" ok\r\n" } else { b" FAIL\r\n" });

    // For the receiver phase, leave only ITBUFEN set. The subsequent RXNE
    // flag must assert EV by itself, then DR must clear the level source.
    wr(I2C_CR2, 16 | (1 << 8) | (1 << 10)); // ITERREN | ITBUFEN; ITEVTEN off
    wr(I2C_CR1, 1 | (1 << 8) | (1 << 10)); // PE | START | ACK
    if !wait(I2C_SR1, 0x0001, 0x0001) {
        return (
            0xFFFF,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            false,
            false,
        );
    }
    wr(I2C_DR, 0x91); // TMP102 address, read
    if !wait(I2C_SR1, 0x0002, 0x0002) {
        return (
            0xFFFF,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            false,
            false,
        );
    }
    let _ = rd(I2C_SR1);
    let _ = rd(I2C_SR2); // clear ADDR; receiver starts clocking
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    if !wait(I2C_SR1, 0x0040, 0x0040) {
        // RXNE
        return (
            0xFFFF,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            false,
            false,
        );
    }
    let rxne_irq = rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT != 0;
    let msb = rd(I2C_DR) as u8;
    // RXNE clears on the DR read. Clear the sticky NVIC pending bit and allow
    // a short interval before the next 1440-cycle byte arrives; EV must stay
    // low even though ITBUFEN remains enabled.
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    let mut rxne_released = true;
    for _ in 0..16 {
        let _ = rd(I2C_SR1);
        rxne_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
    }
    wr(I2C_CR1, 1); // PE; NACK the final byte
    wr(I2C_CR1, 1 | (1 << 9)); // request STOP after the final byte
    if !wait(I2C_SR1, 0x0040, 0x0040) {
        return (
            0xFFFF,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            false,
            false,
        );
    }
    let lsb = rd(I2C_DR) as u8;
    let temperature = u16::from_be_bytes([msb, lsb]);
    let idle = wait(I2C_SR2, 0x0002, 0);

    // The system has no device at 0x50. Silicon sets AF, no ADDR, and keeps
    // MSL|BUSY until STOP; AF is rc_w0.
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    wr(NVIC_ICPR1, I2C1_ER_IRQ_BIT);
    let mut irq_released = true;
    for _ in 0..16 {
        let _ = rd(I2C_SR1);
        irq_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
        irq_released &= rd(NVIC_ISPR1) & I2C1_ER_IRQ_BIT == 0;
    }
    wr(I2C_CR1, rd(I2C_CR1) | (1 << 8)); // START
    if !wait(I2C_SR1, 0x0001, 0x0001) {
        return (
            temperature,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            irq_released,
            false,
        );
    }
    wr(I2C_DR, 0xA0); // absent EEPROM-style address 0x50, write
    if !wait(I2C_SR1, 0x0400, 0x0400) {
        return (
            temperature,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            irq_released,
            false,
        );
    }
    let sr1 = rd(I2C_SR1);
    let sr2 = rd(I2C_SR2);
    let nack_ok = sr1 & 0x0002 == 0 && sr2 & 0x3 == 0x3; // no ADDR; MSL|BUSY
    let er_irq = rd(NVIC_ISPR1) & I2C1_ER_IRQ_BIT != 0;
    wr(I2C_CR1, rd(I2C_CR1) | (1 << 9)); // STOP
    wr(I2C_SR1, !0x0400 & 0xFFFF); // clear AF (rc_w0)
    let af_cleared = rd(I2C_SR1) & 0x0400 == 0;
    let nack_idle = wait(I2C_SR2, 0x0002, 0);
    // Clear the sticky NVIC bits only after STOP and AF clear. With the
    // source flags gone, neither I2C line should pend again while polled.
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    wr(NVIC_ICPR1, I2C1_ER_IRQ_BIT);
    for _ in 0..64 {
        let _ = rd(I2C_SR1);
        irq_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
        irq_released &= rd(NVIC_ISPR1) & I2C1_ER_IRQ_BIT == 0;
    }
    let rxne_ok = rxne_irq && rxne_released;
    send(b"i2c1 rxne_buf_irq=");
    putc(if rxne_irq { b'1' } else { b'0' });
    send(b" release=");
    putc(if rxne_released { b'1' } else { b'0' });
    send(if rxne_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // Repeat the TMP102 read without consuming the first byte. The second
    // byte then waits behind it in the receive shift register, setting BTF.
    // ITEVTEN alone must report BTF while RXNE alone remains disabled.
    let (rx_btf_data, rx_btf_irq, rxne_isolated, rx_btf_released) = smoke_i2c1_receive_btf();
    // The TMP102 fixture advances by 0.5 °C after each complete temperature
    // read, so the second transaction returns 25.5 °C (0x1980).
    let rx_btf_ok = rx_btf_data == 0x1980 && rx_btf_irq && rxne_isolated && rx_btf_released;
    send(b"i2c1 btf_rx_irq=");
    putc(if rx_btf_irq { b'1' } else { b'0' });
    send(b" rxne_isolated=");
    putc(if rxne_isolated { b'1' } else { b'0' });
    send(b" release=");
    putc(if rx_btf_released { b'1' } else { b'0' });
    send(b" data=");
    put_hex32(u32::from(rx_btf_data));
    send(if rx_btf_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    (
        temperature,
        sr1,
        buffer_irq,
        buffer_irq_released,
        event_irq,
        event_irq_released,
        er_irq,
        irq_released,
        temperature == 0x1900
            && idle
            && nack_ok
            && af_cleared
            && nack_idle
            && buffer_irq
            && buffer_irq_released
            && rxne_ok
            && rx_btf_ok
            && event_irq
            && event_irq_released
            && btf_irq
            && er_irq
            && irq_released,
    )
}

/// Read two TMP102 bytes while leaving the first in DR until the second has
/// arrived. With ITEVTEN enabled and ITBUFEN disabled, RXNE must not assert EV;
/// the full receive pipeline must assert BTF and release it when DR is read.
fn smoke_i2c1_receive_btf() -> (u16, bool, bool, bool) {
    const PE: u32 = 1;
    const ACK: u32 = 1 << 10;
    const START: u32 = 1 << 8;
    const STOP: u32 = 1 << 9;
    const SB: u32 = 1;
    const ADDR: u32 = 1 << 1;
    const BTF: u32 = 1 << 2;
    const RXNE: u32 = 1 << 6;

    // ITERREN | ITEVTEN; ITBUFEN stays clear for the receive phase.
    wr(I2C_CR2, 16 | (1 << 8) | (1 << 9));
    wr(I2C_CR1, PE | ACK | START);
    if !wait(I2C_SR1, SB, SB) {
        return (0xFFFF, false, false, false);
    }
    wr(I2C_DR, 0x90);
    if !wait(I2C_SR1, ADDR, ADDR) {
        return (0xFFFF, false, false, false);
    }
    let _ = rd(I2C_SR1);
    let _ = rd(I2C_SR2);
    wr(I2C_DR, 0); // TMP102 temperature register pointer
    if !wait(I2C_SR1, BTF, BTF) {
        return (0xFFFF, false, false, false);
    }

    wr(I2C_CR1, PE | ACK | START); // repeated START
    if !wait(I2C_SR1, SB, SB) {
        return (0xFFFF, false, false, false);
    }
    wr(I2C_DR, 0x91);
    if !wait(I2C_SR1, ADDR, ADDR) {
        return (0xFFFF, false, false, false);
    }
    let _ = rd(I2C_SR1);
    let _ = rd(I2C_SR2);
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);

    // RXNE is a buffer source and must stay out of EV with ITBUFEN disabled.
    if !wait(I2C_SR1, RXNE, RXNE) {
        return (0xFFFF, false, false, false);
    }
    let rxne_isolated = rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
    wr(I2C_CR1, PE); // NACK the second (final) byte while the first stays in DR
    if !wait(I2C_SR1, BTF, BTF) {
        return (0xFFFF, false, rxne_isolated, false);
    }
    let btf_irq = rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT != 0;

    let msb = rd(I2C_DR) as u8; // second byte moves from shift register into DR
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    let mut btf_released = true;
    for _ in 0..16 {
        btf_released &= rd(I2C_SR1) & BTF == 0;
        btf_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
    }

    wr(I2C_CR1, PE | STOP);
    let idle = wait(I2C_SR2, 0x0002, 0);
    let lsb = rd(I2C_DR) as u8;
    let data = u16::from_be_bytes([msb, lsb]);
    (data, btf_irq, rxne_isolated, btf_released && idle)
}

/// Exercise the imported F401's second legacy I2C controller with its own
/// TMP102 and event vector. This catches instance wiring/configuration mistakes
/// that an I2C1-only transaction cannot see. It checks that SB and ADDR stay
/// off the EV line until ITEVTEN is enabled, isolates TXE from ITEVTEN, then
/// checks TXE and BTF event gating plus TXE/RXNE buffer IRQ behavior under
/// ITBUFEN.
fn smoke_i2c2() -> (u16, bool, bool, bool, bool, bool, bool, bool, bool) {
    const PE: u32 = 1;
    const ACK: u32 = 1 << 10;
    const START: u32 = 1 << 8;
    const STOP: u32 = 1 << 9;
    const SB: u32 = 1;
    const ADDR: u32 = 1 << 1;
    const BTF: u32 = 1 << 2;
    const RXNE: u32 = 1 << 6;

    wr(I2C2_CR2, 16); // all I2C interrupt sources disabled
    wr(I2C2_CCR, 80);
    wr(I2C2_TRISE, 17);
    wr(I2C2_CR1, PE);

    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    wr(I2C2_CR1, PE | START);
    if !wait(I2C2_SR1, SB, SB) {
        return (
            0xFFFF, false, false, false, false, false, false, false, false,
        );
    }
    let sb_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    // Enabling ITEVTEN while SB is still set must assert the EV line.
    wr(I2C2_CR2, 16 | (1 << 9));
    let sb_event_gate = sb_isolated && wait(NVIC_ISPR1, I2C2_EV_IRQ_BIT, I2C2_EV_IRQ_BIT);
    wr(I2C2_CR2, 16); // disable ITEVTEN before ADDR is raised
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT); // discard the earlier SB event
    wr(I2C2_DR, 0x92); // independent TMP102 at 0x49, write
    if !wait(I2C2_SR1, ADDR, ADDR) {
        return (
            0xFFFF,
            sb_event_gate,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        );
    }
    // ADDR must remain isolated with all interrupt enables clear; enabling
    // ITEVTEN while the flag is up must assert the EV line.
    let addr_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    wr(I2C2_CR2, 16 | (1 << 9));
    let addr_event_gate = addr_isolated && wait(NVIC_ISPR1, I2C2_EV_IRQ_BIT, I2C2_EV_IRQ_BIT);
    let _ = rd(I2C2_SR1);
    let _ = rd(I2C2_SR2); // clear ADDR; TXE is not an ITEVTEN source
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    let mut txe_isolated = true;
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
        txe_isolated &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    }

    // The same TXE source must assert the same EV vector once ITBUFEN is
    // enabled alongside ITEVTEN, then release when ITBUFEN is removed.
    wr(I2C2_CR2, 16 | (1 << 9) | (1 << 10));
    let mut txe_buf_irq = true;
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
        txe_buf_irq &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT != 0;
    }
    wr(I2C2_CR2, 16 | (1 << 9));
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    let mut txe_buf_released = true;
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
        txe_buf_released &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    }

    // BTF is an event source: keep every interrupt enable clear while the
    // pointer byte completes, then enable ITEVTEN with BTF still set.
    wr(I2C2_CR2, 16);
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    wr(I2C2_DR, 0); // temperature register pointer
    if !wait(I2C2_SR1, BTF, BTF) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            false,
            false,
            false,
        );
    }
    let btf_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    wr(I2C2_CR2, 16 | (1 << 9)); // ITEVTEN only
    let btf_event_gate = btf_isolated && wait(NVIC_ISPR1, I2C2_EV_IRQ_BIT, I2C2_EV_IRQ_BIT);

    // Repeated START, then read the independent sensor's two-byte result.
    wr(I2C2_CR1, PE | ACK | START);
    if !wait(I2C2_SR1, SB, SB) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            btf_event_gate,
            false,
            false,
        );
    }
    wr(I2C2_DR, 0x93);
    if !wait(I2C2_SR1, ADDR, ADDR) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            btf_event_gate,
            false,
            false,
        );
    }
    let _ = rd(I2C2_SR1);
    let _ = rd(I2C2_SR2);
    wr(I2C2_CR2, 16 | (1 << 9)); // ITEVTEN only; RXNE must stay isolated
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    if !wait(I2C2_SR1, RXNE, RXNE) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            btf_event_gate,
            false,
            false,
        );
    }
    let rxne_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    // RXNE remains set; enabling ITBUFEN must assert EV without another byte.
    wr(I2C2_CR2, 16 | (1 << 9) | (1 << 10)); // ITEVTEN | ITBUFEN
    let rxne_irq_asserted = wait(NVIC_ISPR1, I2C2_EV_IRQ_BIT, I2C2_EV_IRQ_BIT);
    let rxne_buf_irq = rxne_isolated && rxne_irq_asserted;
    let msb = rd(I2C2_DR) as u8;
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    let mut rxne_buf_released = true;
    for _ in 0..16 {
        rxne_buf_released &= rd(I2C2_SR1) & RXNE == 0;
        rxne_buf_released &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    }
    send(b"i2c2 rxne_isolated=");
    send(if rxne_isolated { b"1" } else { b"0" });
    send(b" gate_asserted=");
    send(if rxne_irq_asserted { b"1" } else { b"0" });
    send(b" release=");
    send(if rxne_buf_released { b"1" } else { b"0" });
    send(if rxne_buf_irq && rxne_buf_released {
        b" ok\r\n"
    } else {
        b" FAIL\r\n"
    });
    wr(I2C2_CR1, PE); // NACK the final byte
    wr(I2C2_CR1, PE | STOP);
    if !wait(I2C2_SR1, RXNE, RXNE) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            btf_event_gate,
            rxne_buf_irq,
            rxne_buf_released,
        );
    }
    let lsb = rd(I2C2_DR) as u8;
    if !wait(I2C2_SR2, 0x0002, 0) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            btf_event_gate,
            rxne_buf_irq,
            rxne_buf_released,
        );
    }
    (
        u16::from_be_bytes([msb, lsb]),
        sb_event_gate,
        addr_event_gate,
        txe_isolated,
        txe_buf_irq,
        txe_buf_released,
        btf_event_gate,
        rxne_buf_irq,
        rxne_buf_released,
    )
}

/// Read two bytes from the I2C2 TMP102 with POS delaying the ACK decision by
/// one byte. With ITEVTEN enabled and ITBUFEN disabled, RXNE must not assert
/// EV; BTF must assert the configured EV line (IRQ 33). STOP is issued while
/// both bytes remain pending, then both DR reads must preserve the bytes and
/// clear BTF/RXNE in order.
fn smoke_i2c2_receive_btf() -> (u16, bool, bool, bool, bool, bool, bool, bool) {
    const PE: u32 = 1;
    const ACK: u32 = 1 << 10;
    const POS: u32 = 1 << 11;
    const START: u32 = 1 << 8;
    const STOP: u32 = 1 << 9;
    const SB: u32 = 1;
    const ADDR: u32 = 1 << 1;
    const BTF: u32 = 1 << 2;
    const RXNE: u32 = 1 << 6;
    const FAILED: (u16, bool, bool, bool, bool, bool, bool, bool) =
        (0xFFFF, false, false, false, false, false, false, false);

    // ITEVTEN only; ITBUFEN remains clear for receive source isolation.
    wr(I2C2_CR2, 16 | (1 << 9));
    wr(I2C2_CR1, PE | ACK | START);
    if !wait(I2C2_SR1, SB, SB) {
        return FAILED;
    }
    wr(I2C2_DR, 0x92); // TMP102 at 0x49, write
    if !wait(I2C2_SR1, ADDR, ADDR) {
        return FAILED;
    }
    let _ = rd(I2C2_SR1);
    let _ = rd(I2C2_SR2);
    wr(I2C2_DR, 0); // temperature register pointer
    if !wait(I2C2_SR1, BTF, BTF) {
        return FAILED;
    }

    // Arm POS while ACK is set so the address phase captures an ACK for the
    // first byte; the write after ADDR clears ACK for the second byte.
    wr(I2C2_CR1, PE | ACK | POS | START);
    if !wait(I2C2_SR1, SB, SB) {
        return FAILED;
    }
    wr(I2C2_DR, 0x93); // TMP102 at 0x49, read
    if !wait(I2C2_SR1, ADDR, ADDR) {
        return FAILED;
    }
    let _ = rd(I2C2_SR1);
    let _ = rd(I2C2_SR2);
    let pos_armed = rd(I2C2_CR1) & POS != 0;
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    wr(I2C2_CR1, PE | POS); // POS delays the NACK so the first byte is ACKed.

    if !wait(I2C2_SR1, RXNE, RXNE) {
        return (0xFFFF, pos_armed, false, false, false, false, false, false);
    }
    let rxne_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    if !wait(I2C2_SR1, BTF, BTF) {
        return (
            0xFFFF,
            pos_armed,
            false,
            rxne_isolated,
            false,
            false,
            false,
            false,
        );
    }
    let btf_irq = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT != 0;

    // Complete the receive before draining DR. STOP must release BUSY while
    // retaining both completed bytes for the following two DR reads.
    wr(I2C2_CR1, PE | POS | STOP);
    let stop_before_dr = wait(I2C2_SR2, 0x0002, 0);
    let msb = rd(I2C2_DR) as u8;
    let first_read_keeps_rxne = rd(I2C2_SR1) & RXNE != 0;
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    let mut btf_released = true;
    for _ in 0..16 {
        btf_released &= rd(I2C2_SR1) & BTF == 0;
        btf_released &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    }
    let lsb = rd(I2C2_DR) as u8;
    let final_rxne_cleared = rd(I2C2_SR1) & RXNE == 0;
    (
        u16::from_be_bytes([msb, lsb]),
        pos_armed,
        btf_irq,
        rxne_isolated,
        stop_before_dr,
        first_read_keeps_rxne,
        btf_released,
        final_rxne_cleared,
    )
}

/// Probe an absent address on the imported I2C2 instance. Check that the
/// controller records AF while ADDR is clear, that ITERREN gates the ER vector,
/// then request STOP, clear AF, and verify the bus and error source recover.
fn smoke_i2c2_nack() -> (u32, bool, bool, bool, bool, bool, bool, bool, bool) {
    const PE: u32 = 1;
    const START: u32 = 1 << 8;
    const STOP: u32 = 1 << 9;
    const SB: u32 = 1;
    const ADDR: u32 = 1 << 1;
    const AF: u32 = 1 << 10;
    const MSL_BUSY: u32 = 0x3;

    // Keep ITERREN and ITBUFEN disabled initially. The NACK should set AF with
    // neither I2C2 interrupt vector active; enabling ITERREN should then pend
    // only the error vector.
    wr(I2C2_CR2, 16 | (1 << 9));
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT | I2C2_ER_IRQ_BIT);
    wr(I2C2_CR1, PE | START);
    if !wait(I2C2_SR1, SB, SB) {
        return (0, false, false, false, false, false, false, false, false);
    }
    wr(I2C2_DR, 0xA0); // absent 7-bit address 0x50, write
    if !wait(I2C2_SR1, AF, AF) {
        return (
            rd(I2C2_SR1),
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        );
    }

    // The START event left a sticky NVIC pending bit. Clear it after the DR
    // write cleared SB so the following observations isolate the NACK phase.
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    let sr1 = rd(I2C2_SR1);
    let sr2 = rd(I2C2_SR2);
    let nack_flags = sr1 & AF != 0 && sr1 & ADDR == 0 && sr2 & MSL_BUSY == MSL_BUSY;
    let event_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;

    let er_isolated = rd(NVIC_ISPR1) & I2C2_ER_IRQ_BIT == 0;
    wr(I2C2_CR2, 16 | (1 << 8) | (1 << 9)); // enable ITERREN
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
    }
    let er_irq = rd(NVIC_ISPR1) & I2C2_ER_IRQ_BIT != 0;

    // ITERREN gates the level source even while AF remains set. Clear the
    // sticky NVIC bit after disabling it, then ensure it does not re-pend.
    wr(I2C2_CR2, 16 | (1 << 9));
    wr(NVIC_ICPR1, I2C2_ER_IRQ_BIT);
    let mut er_masked_release = true;
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
        er_masked_release &= rd(NVIC_ISPR1) & I2C2_ER_IRQ_BIT == 0;
    }

    wr(I2C2_CR1, PE | STOP);
    wr(I2C2_SR1, !(AF) & 0xFFFF); // AF is cleared by writing zero to the flag
    let af_cleared = rd(I2C2_SR1) & AF == 0;
    let idle = wait(I2C2_SR2, 1 << 1, 0);
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT | I2C2_ER_IRQ_BIT);
    let mut event_released = true;
    let mut er_released = true;
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
        event_released &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
        er_released &= rd(NVIC_ISPR1) & I2C2_ER_IRQ_BIT == 0;
    }
    (
        sr1,
        nack_flags,
        event_isolated && event_released,
        er_isolated,
        er_irq,
        er_masked_release,
        af_cleared,
        idle,
        er_released,
    )
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

/// USART1 is imported with NVIC IRQ 37. TXE must stay off the line while
/// TXEIE is clear, pend when TXEIE is enabled, and release when disabled.
fn smoke_usart1_txe_irq() -> bool {
    const TXE: u32 = 1 << 7;
    const TXEIE: u32 = 1 << 7;

    wr(RCC_APB2ENR, rd(RCC_APB2ENR) | (1 << 4)); // USART1EN
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    wr(USART1_CR1, (1 << 13) | (1 << 3)); // UE | TE; TXEIE remains clear

    let txe_isolated = rd(USART1_SR) & TXE != 0 && rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;
    wr(USART1_CR1, (1 << 13) | (1 << 3) | TXEIE); // enable TXEIE with TXE already set
    let txe_pended = txe_isolated && wait(NVIC_ISPR1, USART1_IRQ_BIT, USART1_IRQ_BIT);

    wr(USART1_CR1, (1 << 13) | (1 << 3)); // UE | TE, TXEIE off
                                          // Clear the latched NVIC pending bit, then allow the level source to
                                          // settle. A still-asserted USART line will immediately re-pend IRQ 37.
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    for _ in 0..64 {
        let _ = rd(USART1_SR);
    }
    let txe_released = rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;

    send(b"usart1 txe_isolated=");
    putc(if txe_isolated { b'1' } else { b'0' });
    send(b" gate_asserted=");
    putc(if txe_pended { b'1' } else { b'0' });
    send(b" released=");
    putc(if txe_released { b'1' } else { b'0' });
    send(if txe_isolated && txe_pended && txe_released {
        b" ok\r\n"
    } else {
        b" FAIL\r\n"
    });
    txe_isolated && txe_pended && txe_released
}

/// USART1's idle TC flag must pend imported NVIC IRQ 37 only while TCIE is set.
/// TXEIE stays clear so the completion source is tested independently.
fn smoke_usart1_tcie_irq() -> bool {
    const TC: u32 = 1 << 6;
    const TCIE: u32 = 1 << 6;

    wr(RCC_APB2ENR, rd(RCC_APB2ENR) | (1 << 4)); // USART1EN
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    wr(USART1_CR1, (1 << 13) | (1 << 3)); // UE | TE; TCIE and TXEIE clear

    let tc_isolated = rd(USART1_SR) & TC != 0 && rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;
    wr(USART1_CR1, (1 << 13) | (1 << 3) | TCIE); // enable TCIE with TC already set
    let tc_pended = tc_isolated && wait(NVIC_ISPR1, USART1_IRQ_BIT, USART1_IRQ_BIT);

    wr(USART1_CR1, (1 << 13) | (1 << 3)); // UE | TE, TCIE off
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    for _ in 0..64 {
        let _ = rd(USART1_SR);
    }
    let tc_released = rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;

    send(b"usart1 tc_isolated=");
    putc(if tc_isolated { b'1' } else { b'0' });
    send(b" gate_asserted=");
    putc(if tc_pended { b'1' } else { b'0' });
    send(b" released=");
    putc(if tc_released { b'1' } else { b'0' });
    send(if tc_isolated && tc_pended && tc_released {
        b" ok\r\n"
    } else {
        b" FAIL\r\n"
    });
    tc_isolated && tc_pended && tc_released
}

/// The onboarding stimulus queues `R` on USART1 before reset. RXNE must stay
/// off IRQ 37 while RXNEIE is clear, then pend it when RXNEIE is enabled with
/// the byte still waiting. Reading DR must release the source.
fn smoke_usart1_rxne_irq() -> bool {
    const RXNE: u32 = 1 << 5;
    const RXNEIE: u32 = 1 << 5;
    const EXPECTED: u32 = b'R' as u32;

    wr(RCC_APB2ENR, rd(RCC_APB2ENR) | (1 << 4)); // USART1EN
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    wr(USART1_CR1, (1 << 13) | (1 << 2)); // UE | RE; RXNEIE remains clear

    let mut rxne_seen = false;
    for _ in 0..4096 {
        if rd(USART1_SR) & RXNE != 0 {
            rxne_seen = true;
            break;
        }
    }
    let rxne_isolated = rxne_seen && rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;
    wr(USART1_CR1, (1 << 13) | (1 << 2) | RXNEIE); // enable RXNEIE with RXNE pending
    let rxne_pended = wait(NVIC_ISPR1, USART1_IRQ_BIT, USART1_IRQ_BIT);
    let data = rd(USART1_DR) & 0xFF;
    let rxne_cleared = rd(USART1_SR) & RXNE == 0;
    // NVIC pending is sticky. Clear it after consuming DR and make sure the
    // still-enabled RXNEIE source does not pend again with an empty queue.
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    for _ in 0..64 {
        let _ = rd(USART1_SR);
    }
    let irq_released = rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;

    send(b"usart1 rxne_isolated=");
    putc(if rxne_isolated { b'1' } else { b'0' });
    send(b" gate_asserted=");
    putc(if rxne_pended { b'1' } else { b'0' });
    send(b" data=");
    put_hex32(data);
    send(b" released=");
    putc(if rxne_cleared && irq_released {
        b'1'
    } else {
        b'0'
    });
    send(
        if rxne_isolated && rxne_pended && data == EXPECTED && rxne_cleared && irq_released {
            b" ok\r\n"
        } else {
            b" FAIL\r\n"
        },
    );
    rxne_isolated && rxne_pended && data == EXPECTED && rxne_cleared && irq_released
}

/// USART2 is both the smoke console and the imported Black Pill's IRQ 38
/// receiver. The script queues two bytes before reset. Neither may appear in
/// SR.RXNE until both legacy CR1.UE and CR1.RE are set; with RXNEIE clear the
/// line stays low, then asserts and releases as DR is drained.
fn smoke_usart2_rx_gate() -> bool {
    const RXNE: u32 = 1 << 5;
    const RXNEIE: u32 = 1 << 5;
    const UE: u32 = 1 << 13;
    const RE: u32 = 1 << 2;
    const TE: u32 = 1 << 3;
    const EXPECTED: u32 = 0x5253; // "RS"

    // main enabled UE|TE for the console, leaving RE disabled. Start-injected
    // bytes must remain on the wire through both one-bit-only configurations.
    let mut re_off = true;
    for _ in 0..16 {
        re_off &= rd(USART_SR) & RXNE == 0;
    }
    wr(USART_CR1, RE | TE); // RE set, UE clear
    let mut ue_off = true;
    for _ in 0..16 {
        ue_off &= rd(USART_SR) & RXNE == 0;
    }

    wr(NVIC_ICPR1, USART2_IRQ_BIT);
    wr(USART_CR1, UE | RE | TE); // both receiver enables; RXNEIE stays clear
    let mut rxne_seen = false;
    for _ in 0..4096 {
        if rd(USART_SR) & RXNE != 0 {
            rxne_seen = true;
            break;
        }
    }
    let irq_isolated = rxne_seen && rd(NVIC_ISPR1) & USART2_IRQ_BIT == 0;
    wr(USART_CR1, UE | RE | TE | RXNEIE);
    let irq_asserted = rxne_seen && wait(NVIC_ISPR1, USART2_IRQ_BIT, USART2_IRQ_BIT);

    let first = rd(USART_DR) & 0xFF;
    let rxne_after_first = rd(USART_SR) & RXNE != 0;
    let second = if rxne_after_first {
        rd(USART_DR) & 0xFF
    } else {
        0
    };
    let rxne_cleared = rd(USART_SR) & RXNE == 0;
    wr(USART_CR1, UE | TE); // disable RXNEIE and RE before final console output
    wr(NVIC_ICPR1, USART2_IRQ_BIT);
    let mut irq_released = true;
    for _ in 0..16 {
        irq_released &= rd(NVIC_ISPR1) & USART2_IRQ_BIT == 0;
    }

    let data = (first << 8) | second;
    let ok = re_off
        && ue_off
        && irq_isolated
        && irq_asserted
        && data == EXPECTED
        && rxne_after_first
        && rxne_cleared
        && irq_released;
    send(b"usart2 rx_gate re_off=");
    send(if re_off { b"1" } else { b"0" });
    send(b" ue_off=");
    send(if ue_off { b"1" } else { b"0" });
    send(b" data=");
    put_hex32(data);
    send(b" rxne_after_first=");
    send(if rxne_after_first { b"1" } else { b"0" });
    send(b" rxne_after=");
    send(if rxne_cleared { b"0" } else { b"1" });
    send(b" irq=");
    send(if irq_isolated {
        b"isolated"
    } else {
        b"not_isolated"
    });
    send(if irq_asserted {
        b",asserted"
    } else {
        b",not_asserted"
    });
    send(if irq_released {
        b",released"
    } else {
        b",not_released"
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
