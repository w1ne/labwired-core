#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const USART2_TDR_PTR: *mut u8 = (0x40004400 + 0x28) as *mut u8;
const RCC_AHB2ENR: *mut u32 = (0x40021000 + 0x4c) as *mut u32;
const RCC_APB2ENR: *mut u32 = (0x40021000 + 0x60) as *mut u32;
const GPIOA_BASE: usize = 0x48000000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRH: *mut u32 = (GPIOA_BASE + 0x24) as *mut u32;
const TIM1_BASE: usize = 0x40012c00;
const TIM1_CR1: *mut u32 = TIM1_BASE as *mut u32;
const TIM1_EGR: *mut u32 = (TIM1_BASE + 0x14) as *mut u32;
const TIM1_CCMR1: *mut u32 = (TIM1_BASE + 0x18) as *mut u32;
const TIM1_CCER: *mut u32 = (TIM1_BASE + 0x20) as *mut u32;
const TIM1_PSC: *mut u32 = (TIM1_BASE + 0x28) as *mut u32;
const TIM1_ARR: *mut u32 = (TIM1_BASE + 0x2c) as *mut u32;
const TIM1_CCR1: *mut u32 = (TIM1_BASE + 0x34) as *mut u32;
const TIM1_BDTR: *mut u32 = (TIM1_BASE + 0x44) as *mut u32;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(USART2_TDR_PTR, b'O');
        core::ptr::write_volatile(USART2_TDR_PTR, b'K');
        core::ptr::write_volatile(USART2_TDR_PTR, b'\n');

        // PA8/TIM1_CH1 is AF6 on STM32G474 (DS12288 Table 13). Enable GPIOA
        // and TIM1, then generate 1 kHz, 50% PWM from the reset HSI16 clock.
        core::ptr::write_volatile(RCC_AHB2ENR, core::ptr::read_volatile(RCC_AHB2ENR) | 1);
        core::ptr::write_volatile(
            RCC_APB2ENR,
            core::ptr::read_volatile(RCC_APB2ENR) | (1 << 11),
        );
        let moder = core::ptr::read_volatile(GPIOA_MODER);
        core::ptr::write_volatile(GPIOA_MODER, (moder & !(0b11 << 16)) | (0b10 << 16));
        let afrh = core::ptr::read_volatile(GPIOA_AFRH);
        core::ptr::write_volatile(GPIOA_AFRH, (afrh & !0xf) | 6);

        core::ptr::write_volatile(TIM1_PSC, 0);
        core::ptr::write_volatile(TIM1_ARR, 15_999);
        core::ptr::write_volatile(TIM1_CCR1, 8_000);
        core::ptr::write_volatile(TIM1_CCMR1, (0b110 << 4) | (1 << 3));
        core::ptr::write_volatile(TIM1_CCER, 1);
        core::ptr::write_volatile(TIM1_EGR, 1);
        core::ptr::write_volatile(TIM1_BDTR, 1 << 15);
        core::ptr::write_volatile(TIM1_CR1, (1 << 7) | 1);
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
