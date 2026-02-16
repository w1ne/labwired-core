#![no_std]
#![no_main]

// NUCLEO-H563ZI virtual COM (COM1) maps to USART3 in BSP.
const USART3_TDR_PTR: *mut u8 = (0x4000_4800 + 0x28) as *mut u8;

const RCC_BASE: u32 = 0x4402_0C00;
const RCC_AHB2ENR: u32 = 0x08C;

const GPIOB_BASE: u32 = 0x4202_0400;
const GPIOC_BASE: u32 = 0x4202_0800;
const GPIOF_BASE: u32 = 0x4202_1400;
const GPIOG_BASE: u32 = 0x4202_1800;

const GPIO_MODER: u32 = 0x00;
const GPIO_IDR: u32 = 0x10;
const GPIO_ODR: u32 = 0x14;
const GPIO_BSRR: u32 = 0x18;
const GPIO_BRR: u32 = 0x28;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn read_u32(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn write_u32(addr: u32, value: u32) {
    unsafe {
        core::ptr::write_volatile(addr as *mut u32, value);
    }
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
    moder |= 0x1 << shift;
    write_u32(base + GPIO_MODER, moder);
}

fn set_led_state(on: bool) {
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

fn report_io_state() {
    let odr_b = read_u32(GPIOB_BASE + 0x14);
    let odr_f = read_u32(GPIOF_BASE + 0x14);
    let odr_g = read_u32(GPIOG_BASE + 0x14);
    let btn13 = (read_u32(GPIOC_BASE + GPIO_IDR) >> 13) & 1;

    uart_write_str("PB0=");
    uart_write_bit(odr_b & 1);
    uart_write_str(" PF4=");
    uart_write_bit((odr_f >> 4) & 1);
    uart_write_str(" PG4=");
    uart_write_bit((odr_g >> 4) & 1);
    uart_write_str(" BTN13=");
    uart_write_bit(btn13);
    uart_write_str("\n");
}

fn main() -> ! {
    let ahb2enr = RCC_BASE + RCC_AHB2ENR;
    let mut val = read_u32(ahb2enr);
    val |= (1 << 1) | (1 << 2) | (1 << 5) | (1 << 6);
    write_u32(ahb2enr, val);

    uart_write_str("H563-IO\n");

    gpio_config_pin_output(GPIOB_BASE, 0);
    gpio_config_pin_output(GPIOF_BASE, 4);
    gpio_config_pin_output(GPIOG_BASE, 4);

    loop {
        set_led_state(true);
        uart_write_str("S1-ON\n");
        report_io_state();
        for _ in 0..100_000 { unsafe { core::arch::asm!("nop"); } }

        set_led_state(false);
        uart_write_str("S2-OFF\n");
        report_io_state();
        for _ in 0..100_000 { unsafe { core::arch::asm!("nop"); } }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
