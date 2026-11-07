#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

// STM32G474RE, RM0440. The NUCLEO-G474RE's default STLINK-V3E VCP is
// LPUART1 on PA2/PA3. PA5 drives the active-high LD2 user LED.
const RCC_BASE: usize = 0x4002_1000;
const RCC_CR: *mut u32 = RCC_BASE as *mut u32;
const RCC_AHB2ENR: *mut u32 = (RCC_BASE + 0x4c) as *mut u32;
const RCC_APB1ENR2: *mut u32 = (RCC_BASE + 0x5c) as *mut u32;
const GPIOA_BASE: usize = 0x4800_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;
const LPUART1_BASE: usize = 0x4000_8000;
const LPUART1_CR1: *mut u32 = LPUART1_BASE as *mut u32;
const LPUART1_BRR: *mut u32 = (LPUART1_BASE + 0x0c) as *mut u32;
const LPUART1_ISR: *const u32 = (LPUART1_BASE + 0x1c) as *const u32;
const LPUART1_TDR: *mut u32 = (LPUART1_BASE + 0x28) as *mut u32;

const GPIOAEN: u32 = 1 << 0;
const LPUART1EN: u32 = 1 << 0;
const HSI16ON: u32 = 1 << 8;
const HSIRDY: u32 = 1 << 10;
const USART_UE: u32 = 1 << 0;
const USART_TE: u32 = 1 << 3;
const USART_TXE: u32 = 1 << 7;
const LED_PIN: u32 = 5;
const LPUART_AF: u32 = 12;
// LPUART BRR = 256 * f_kernel / baud. At reset, PCLK1 is HSI16 (16 MHz).
const LPUART_BRR_115200: u32 = (256 * 16_000_000 + 57_600) / 115_200;
const SPIN_LIMIT: u32 = 20_000;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        // Use the reset HSI16 clock for PCLK1 and its default LPUART1 kernel
        // source. Enable GPIOA and LPUART1 before accessing their registers.
        core::ptr::write_volatile(RCC_CR, core::ptr::read_volatile(RCC_CR) | HSI16ON);
        spin_until(|| core::ptr::read_volatile(RCC_CR) & HSIRDY != 0);
        core::ptr::write_volatile(
            RCC_AHB2ENR,
            core::ptr::read_volatile(RCC_AHB2ENR) | GPIOAEN,
        );
        core::ptr::write_volatile(
            RCC_APB1ENR2,
            core::ptr::read_volatile(RCC_APB1ENR2) | LPUART1EN,
        );

        // PA2/PA3 use alternate function 12 for LPUART1. PA5 is the board's
        // user LED (LD2), active high.
        let mut moder = core::ptr::read_volatile(GPIOA_MODER);
        moder &= !((0b11 << (2 * 2)) | (0b11 << (3 * 2)) | (0b11 << (LED_PIN * 2)));
        moder |= (0b10 << (2 * 2)) | (0b10 << (3 * 2)) | (0b01 << (LED_PIN * 2));
        core::ptr::write_volatile(GPIOA_MODER, moder);
        let mut afrl = core::ptr::read_volatile(GPIOA_AFRL);
        afrl &= !((0xf << (2 * 4)) | (0xf << (3 * 4)));
        afrl |= (LPUART_AF << (2 * 4)) | (LPUART_AF << (3 * 4));
        core::ptr::write_volatile(GPIOA_AFRL, afrl);

        // Configure 115200 8N1. LPUART uses BRR = 256*f_kernel/baud.
        core::ptr::write_volatile(LPUART1_BRR, LPUART_BRR_115200);
        core::ptr::write_volatile(LPUART1_CR1, USART_UE | USART_TE);
    }

    print_str("NUCLEO_G474RE_OK\n");

    // The board smoke checks LD2 on PA5, not just the startup UART marker.
    for _ in 0..4u32 {
        led_set(true);
        delay(20_000);
        led_set(false);
        delay(20_000);
    }

    loop {}
}

unsafe fn spin_until(mut condition: impl FnMut() -> bool) {
    let mut guard = 0;
    while !condition() && guard < SPIN_LIMIT {
        guard += 1;
    }
}

fn print_str(text: &str) {
    for byte in text.bytes() {
        unsafe {
            let mut guard = 0;
            while core::ptr::read_volatile(LPUART1_ISR) & USART_TXE == 0 && guard < SPIN_LIMIT {
                guard += 1;
            }
            core::ptr::write_volatile(LPUART1_TDR, byte as u32);
        }
    }
}

fn led_set(on: bool) {
    let value = if on { 1 << LED_PIN } else { 1 << (LED_PIN + 16) };
    unsafe { core::ptr::write_volatile(GPIOA_BSRR, value) };
}

fn delay(iterations: u32) {
    for _ in 0..iterations {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
