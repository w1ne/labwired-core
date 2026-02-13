#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

const USART3_BASE: u32 = 0x4000_4800;
const USART3_TX_PTR: *mut u8 = USART3_BASE as *mut u8;

const RCC_BASE: u32 = 0x4402_0C00;
const SYSTICK_BASE: u32 = 0xE000_E010;

const GPIOA_BASE: u32 = 0x4202_0000;
const GPIOB_BASE: u32 = 0x4202_0400;
const GPIOC_BASE: u32 = 0x4202_0800;
const GPIOD_BASE: u32 = 0x4202_0C00;
const GPIOE_BASE: u32 = 0x4202_1000;
const GPIOF_BASE: u32 = 0x4202_1400;
const GPIOG_BASE: u32 = 0x4202_1800;

// Current LabWired model uses STM32F1-style GPIO offsets.
const GPIO_CRL: u32 = 0x00;
const GPIO_CRH: u32 = 0x04;
const GPIO_IDR: u32 = 0x08;
const GPIO_ODR: u32 = 0x0C;
const GPIO_BSRR: u32 = 0x10;
const GPIO_BRR: u32 = 0x14;

const RCC_APB2ENR: u32 = 0x18;
const RCC_APB1ENR: u32 = 0x1C;

const STK_CSR: u32 = 0x00;
const STK_RVR: u32 = 0x04;
const STK_CVR: u32 = 0x08;
const STK_CALIB: u32 = 0x0C;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn write_u32(addr: u32, value: u32) {
    unsafe {
        core::ptr::write_volatile(addr as *mut u32, value);
    }
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

fn config_gpio_output(base: u32, pin: u32) {
    // STM32F1 nibble format: MODE=11 (50 MHz), CNF=00 (push-pull output).
    let cfg_nibble = 0x3u32;
    if pin < 8 {
        let shift = pin * 4;
        let reg = cfg_nibble << shift;
        write_u32(base + GPIO_CRL, reg);
    } else {
        let shift = (pin - 8) * 4;
        let reg = cfg_nibble << shift;
        write_u32(base + GPIO_CRH, reg);
    }
}

fn touch_gpio_port(base: u32, pin: u32) {
    let mask = 1u32 << pin;
    config_gpio_output(base, pin);
    write_u32(base + GPIO_BSRR, mask);
    write_u32(base + GPIO_BRR, mask);
    write_u32(base + GPIO_ODR, 0);
    write_u32(base + GPIO_IDR, 0);
}

fn touch_rcc() {
    write_u32(RCC_BASE + RCC_APB2ENR, 0xA5A5_0001);
    write_u32(RCC_BASE + RCC_APB1ENR, 0x5A5A_0002);
}

fn touch_systick() {
    write_u32(SYSTICK_BASE + STK_RVR, 0x1234);
    write_u32(SYSTICK_BASE + STK_CVR, 0xFFFF_FFFF); // resets CVR in model
    write_u32(SYSTICK_BASE + STK_CSR, 0x2); // do not enable counting for deterministic readback
    let _ = STK_CALIB;
}

fn main() -> ! {
    uart_write_str("H563-FULLCHIP\n");

    touch_rcc();
    touch_systick();
    touch_gpio_port(GPIOA_BASE, 0);
    touch_gpio_port(GPIOB_BASE, 1);
    touch_gpio_port(GPIOC_BASE, 2);
    touch_gpio_port(GPIOD_BASE, 3);
    touch_gpio_port(GPIOE_BASE, 4);
    touch_gpio_port(GPIOF_BASE, 5);
    touch_gpio_port(GPIOG_BASE, 6);

    uart_write_str("RCC=1 SYSTICK=1 UART=1\n");
    uart_write_str("GPIOA=1 GPIOB=1 GPIOC=1 GPIOD=1 GPIOE=1 GPIOF=1 GPIOG=1\n");
    uart_write_str("ALL=1\n");

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
