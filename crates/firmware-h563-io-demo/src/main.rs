#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

// NUCLEO-H563ZI virtual COM (COM1) maps to USART3 in BSP.
const USART3_TX_PTR: *mut u8 = 0x4000_4800 as *mut u8;

// GPIO base addresses from `stm32h563xx.h` (non-secure aliases used by our config).
const GPIOB_BASE: u32 = 0x4202_0400;
const GPIOC_BASE: u32 = 0x4202_0800;
const GPIOF_BASE: u32 = 0x4202_1400;
const GPIOG_BASE: u32 = 0x4202_1800;

// Current LabWired GPIO model uses STM32F1-style register offsets.
const GPIO_CRL: u32 = 0x00;
const GPIO_CRH: u32 = 0x04;
const GPIO_IDR: u32 = 0x08;
const GPIO_ODR: u32 = 0x0C;

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
        core::ptr::write_volatile(USART3_TX_PTR, ch);
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

fn gpio_config_pin_output_pushpull_50mhz(base: u32, pin: u32) {
    // STM32F1 nibble format: MODE[1:0]=11 (50 MHz), CNF[1:0]=00 (PP output).
    let cfg: u32 = 0x3;
    if pin < 8 {
        let shift = pin * 4;
        let mut reg = read_u32(base + GPIO_CRL);
        reg &= !(0xF << shift);
        reg |= cfg << shift;
        write_u32(base + GPIO_CRL, reg);
    } else {
        let shift = (pin - 8) * 4;
        let mut reg = read_u32(base + GPIO_CRH);
        reg &= !(0xF << shift);
        reg |= cfg << shift;
        write_u32(base + GPIO_CRH, reg);
    }
}

fn set_led_state(on: bool) {
    // NUCLEO-H563ZI LEDs from BSP:
    // LED1: PB0, LED2: PF4, LED3: PG4
    let (mask_b, mask_f, mask_g) = (1u32 << 0, 1u32 << 4, 1u32 << 4);

    let mut odr_b = read_u32(GPIOB_BASE + GPIO_ODR);
    let mut odr_f = read_u32(GPIOF_BASE + GPIO_ODR);
    let mut odr_g = read_u32(GPIOG_BASE + GPIO_ODR);

    if on {
        odr_b |= mask_b;
        odr_f |= mask_f;
        odr_g |= mask_g;
    } else {
        odr_b &= !mask_b;
        odr_f &= !mask_f;
        odr_g &= !mask_g;
    }

    write_u32(GPIOB_BASE + GPIO_ODR, odr_b);
    write_u32(GPIOF_BASE + GPIO_ODR, odr_f);
    write_u32(GPIOG_BASE + GPIO_ODR, odr_g);
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

fn main() -> ! {
    uart_write_str("H563-IO\n");

    gpio_config_pin_output_pushpull_50mhz(GPIOB_BASE, 0);
    gpio_config_pin_output_pushpull_50mhz(GPIOF_BASE, 4);
    gpio_config_pin_output_pushpull_50mhz(GPIOG_BASE, 4);

    set_led_state(true);
    report_io_state(true);

    set_led_state(false);
    report_io_state(false);

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
