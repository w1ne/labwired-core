#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

//! Two-terminal brushed DC motor behind an H-bridge (L298N-style channel),
//! driven from a bare-metal STM32F401:
//!
//! * PA0 = IN1, PA1 = IN2 (direction, plain GPIO outputs)
//! * PA6 = ENA, hardware PWM from TIM3 CH1 (AF2), 70 % duty
//!
//! The firmware runs forward, then reverses, then brakes (IN1 = IN2 = 1) and
//! prints each phase on USART2. Nothing here talks to the simulator: the motor
//! plant reads the pads and the timer exactly as a real bridge would.

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;

const GPIOA_MODER: *mut u32 = 0x4002_0000 as *mut u32;
const GPIOA_ODR: *mut u32 = 0x4002_0014 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4002_0020 as *mut u32;

const TIM3_BASE: usize = 0x4000_0400;
const TIM3_CR1: *mut u32 = TIM3_BASE as *mut u32;
const TIM3_EGR: *mut u32 = (TIM3_BASE + 0x14) as *mut u32;
const TIM3_CCMR1: *mut u32 = (TIM3_BASE + 0x18) as *mut u32;
const TIM3_CCER: *mut u32 = (TIM3_BASE + 0x20) as *mut u32;
const TIM3_PSC: *mut u32 = (TIM3_BASE + 0x28) as *mut u32;
const TIM3_ARR: *mut u32 = (TIM3_BASE + 0x2C) as *mut u32;
const TIM3_CCR1: *mut u32 = (TIM3_BASE + 0x34) as *mut u32;

const USART2_DR: *mut u32 = 0x4000_4404 as *mut u32;

/// Busy-wait iterations per motion phase. Long enough for the default plant
/// (mechanical time constant ~7.5 ms) to settle near its no-load speed.
const PHASE_SPIN: u32 = 1_500_000;

fn rd(reg: *mut u32) -> u32 {
    unsafe { core::ptr::read_volatile(reg) }
}

fn wr(reg: *mut u32, value: u32) {
    unsafe { core::ptr::write_volatile(reg, value) }
}

fn print(s: &str) {
    for b in s.bytes() {
        wr(USART2_DR, u32::from(b));
    }
}

fn spin(iterations: u32) {
    for i in 0..iterations {
        core::hint::black_box(i);
    }
}

/// IN1/IN2 on PA0/PA1 in one ODR write.
fn bridge(in1: bool, in2: bool) {
    let odr = rd(GPIOA_ODR) & !0b11;
    wr(GPIOA_ODR, odr | u32::from(in1) | (u32::from(in2) << 1));
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    // Clocks: GPIOA (AHB1 bit 0), TIM3 (APB1 bit 1), USART2 (APB1 bit 17).
    wr(RCC_AHB1ENR, rd(RCC_AHB1ENR) | 1);
    wr(RCC_APB1ENR, rd(RCC_APB1ENR) | (1 << 1) | (1 << 17));

    // PA0/PA1 general-purpose outputs, PA6 alternate function 2 (TIM3_CH1).
    let moder = rd(GPIOA_MODER) & !((0b11) | (0b11 << 2) | (0b11 << 12));
    wr(GPIOA_MODER, moder | 0b01 | (0b01 << 2) | (0b10 << 12));
    wr(GPIOA_AFRL, (rd(GPIOA_AFRL) & !(0xF << 24)) | (2 << 24));
    bridge(false, false);

    // TIM3 CH1: PWM mode 1, preload, 1000-tick period, 70 % duty.
    wr(TIM3_PSC, 0);
    wr(TIM3_ARR, 999);
    wr(TIM3_CCR1, 700);
    wr(TIM3_CCMR1, (0b110 << 4) | (1 << 3));
    wr(TIM3_CCER, 1);
    wr(TIM3_EGR, 1);
    wr(TIM3_CR1, (1 << 7) | 1);

    print("HBRIDGE READY\n");

    bridge(true, false);
    print("FORWARD\n");
    spin(PHASE_SPIN);

    bridge(false, true);
    print("REVERSE\n");
    spin(PHASE_SPIN);

    bridge(true, true);
    print("BRAKE\n");

    #[allow(clippy::empty_loop)] // bare-metal firmware: park the core forever
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    #[allow(clippy::empty_loop)] // bare-metal firmware: park the core forever
    loop {}
}
