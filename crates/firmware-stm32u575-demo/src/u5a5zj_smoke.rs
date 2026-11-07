#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

// NUCLEO-U5A5ZJ-Q / MB1549 smoke: USART1 VCP and its three active-high user
// LEDs (LD1 PC7, LD2 PB7, LD3 PG2). The U5A5 and U575 share these board pins
// and the RCC/GPIO/USART register layout used here.

const RCC_BASE: usize = 0x4602_0C00;
const RCC_AHB2ENR1: *mut u32 = (RCC_BASE + 0x8C) as *mut u32;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0xA4) as *mut u32;
const USART1_TDR: *mut u8 = (0x4001_3800 + 0x28) as *mut u8;
const USART1_CR1: *mut u32 = 0x4001_3800 as *mut u32;
const USART1_BRR: *mut u32 = (0x4001_3800 + 0x0C) as *mut u32;

const GPIOA_BASE: usize = 0x4202_0000;
const GPIOB_BASE: usize = 0x4202_0400;
const GPIOC_BASE: usize = 0x4202_0800;
const GPIOG_BASE: usize = 0x4202_1800;
const GPIO_MODER: usize = 0x00;
const GPIO_AFRH: usize = 0x24;
const GPIO_BSRR: usize = 0x18;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn configure_output(port: usize, pin: u32) {
    unsafe {
        let moder = (port + GPIO_MODER) as *mut u32;
        let shift = pin * 2;
        let value = core::ptr::read_volatile(moder);
        core::ptr::write_volatile(moder, (value & !(0b11 << shift)) | (0b01 << shift));
    }
}

fn configure_alternate(port: usize, pin: u32, af: u32) {
    unsafe {
        let moder = (port + GPIO_MODER) as *mut u32;
        let mode_shift = pin * 2;
        let mode = core::ptr::read_volatile(moder);
        core::ptr::write_volatile(moder, (mode & !(0b11 << mode_shift)) | (0b10 << mode_shift));

        let afr = (port + GPIO_AFRH) as *mut u32;
        let af_shift = (pin - 8) * 4;
        let af_value = core::ptr::read_volatile(afr);
        core::ptr::write_volatile(afr, (af_value & !(0b1111 << af_shift)) | (af << af_shift));
    }
}

fn configure_usart1() {
    // NUCLEO-U5A5ZJ-Q VCP: PA9/PA10 AF7, 115200 8N1 from the reset 4 MHz
    // MSI clock. Program BRR before enabling USART and its transmitter.
    configure_alternate(GPIOA_BASE, 9, 7);
    configure_alternate(GPIOA_BASE, 10, 7);
    unsafe {
        core::ptr::write_volatile(USART1_CR1, 0);
        core::ptr::write_volatile(USART1_BRR, 35);
        core::ptr::write_volatile(USART1_CR1, (1 << 0) | (1 << 3));
    }
}

fn write_led(port: usize, pin: u32, on: bool) {
    let bit = if on { pin } else { pin + 16 };
    unsafe {
        core::ptr::write_volatile((port + GPIO_BSRR) as *mut u32, 1 << bit);
    }
}

fn main() -> ! {
    unsafe {
        // GPIOA/B/C/G on AHB2ENR1; USART1 on APB2ENR.
        core::ptr::write_volatile(RCC_AHB2ENR1, (1 << 0) | (1 << 1) | (1 << 2) | (1 << 6));
        core::ptr::write_volatile(RCC_APB2ENR, 1 << 14);
    }

    configure_usart1();
    configure_output(GPIOC_BASE, 7);
    configure_output(GPIOB_BASE, 7);
    configure_output(GPIOG_BASE, 2);

    for byte in b"U5A5ZJ_OK\n" {
        unsafe { core::ptr::write_volatile(USART1_TDR, *byte) };
    }

    let mut on = false;
    loop {
        write_led(GPIOC_BASE, 7, on);
        write_led(GPIOB_BASE, 7, on);
        write_led(GPIOG_BASE, 2, on);
        for _ in 0..2_000 {
            unsafe { core::arch::asm!("nop") };
        }
        on = !on;
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
