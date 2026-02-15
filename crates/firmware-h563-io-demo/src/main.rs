#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

// NUCLEO-H563ZI virtual COM (COM1) maps to USART3 in BSP.
const USART3_TDR_PTR: *mut u8 = (0x4000_4800 + 0x28) as *mut u8;

// GPIO base addresses from `stm32h563xx.h` (non-secure aliases used by our config).
const GPIOB_BASE: u32 = 0x4202_0400;
const GPIOC_BASE: u32 = 0x4202_0800;
const GPIOF_BASE: u32 = 0x4202_1400;
const GPIOG_BASE: u32 = 0x4202_1800;

// STM32H5-style GPIO offsets.
const GPIO_MODER: u32 = 0x00;
const GPIO_IDR: u32 = 0x10;
const GPIO_BSRR: u32 = 0x18;
const GPIO_BRR: u32 = 0x28;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn write_u32(addr: u32, value: u32) {
    unsafe {
        core::ptr::write_volatile(addr as *mut u32, value);
    }
}

fn read_u32(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn uart_write_byte(ch: u8) {
    unsafe {
        core::ptr::write_volatile(USART3_TDR_PTR, ch);
    }
}

fn uart_write_str(s: &str) {
    for &b in s.as_bytes() {
        uart_write_byte(b);
    }
}

fn uart_write_bit(v: u32) {
    if v == 0 {
        uart_write_byte(b'0');
    } else {
        uart_write_byte(b'1');
    }
}

fn gpio_config_pin_output(base: u32, pin: u32) {
    let shift = pin * 2;
    let mut moder = read_u32(base + GPIO_MODER);
    moder &= !(0x3 << shift);
    moder |= 0x1 << shift; // 01: general purpose output mode
    write_u32(base + GPIO_MODER, moder);
}

fn set_led_state(on: bool) {
    // NUCLEO-H563ZI LEDs from BSP:
    // LED1: PB0, LED2: PF4, LED3: PG4
    let (mask_b, mask_f, mask_g) = (1u32 << 0, 1u32 << 4, 1u32 << 4);

    if on {
        write_u32(GPIOB_BASE + GPIO_BSRR, mask_b);
        write_u32(GPIOF_BASE + GPIO_BSRR, mask_f);
        write_u32(GPIOG_BASE + GPIO_BSRR, mask_g);
    } else {
        write_u32(GPIOB_BASE + GPIO_BRR, mask_b);
        write_u32(GPIOF_BASE + GPIO_BRR, mask_f);
        write_u32(GPIOG_BASE + GPIO_BRR, mask_g);
    }
}

fn sample_button_pc13() -> u32 {
    let btn13 = (read_u32(GPIOC_BASE + GPIO_IDR) >> 13) & 1;
    if btn13 == 0 {
        0
    } else {
        1
    }
}

fn report_io_state(led_on: bool) {
    let led = if led_on { 1 } else { 0 };
    let btn13 = sample_button_pc13();

    uart_write_str("PB0=");
    uart_write_bit(led);
    uart_write_str(" PF4=");
    uart_write_bit(led);
    uart_write_str(" PG4=");
    uart_write_bit(led);
    uart_write_str(" BTN13=");
    uart_write_bit(btn13);
    uart_write_str("\n");
}

fn delay(mut ticks: u32) {
    while ticks != 0 {
        unsafe {
            core::arch::asm!("nop");
        }
        ticks -= 1;
    }
}

fn main() -> ! {
    uart_write_str("H563-IO\n");

    gpio_config_pin_output(GPIOB_BASE, 0);
    gpio_config_pin_output(GPIOF_BASE, 4);
    gpio_config_pin_output(GPIOG_BASE, 4);

    set_led_state(true);
    report_io_state(true);

    loop {
        set_led_state(true);
        report_io_state(true);
        delay(100);

        set_led_state(false);
        report_io_state(false);
        delay(100);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
