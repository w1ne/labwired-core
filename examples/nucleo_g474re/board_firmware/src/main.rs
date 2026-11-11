#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const USART2_TDR_PTR: *mut u8 = (0x40004400 + 0x28) as *mut u8;

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4002_1060 as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRH: *mut u32 = (0x4800_0000 + 0x24) as *mut u32;
const TIM1_BASE: u32 = 0x4001_2C00;

unsafe fn start_tim1_ch1_pwm_on_pa8() {
    let ahb2enr = core::ptr::read_volatile(RCC_AHB2ENR);
    core::ptr::write_volatile(RCC_AHB2ENR, ahb2enr | 1); // GPIOAEN
    let apb2enr = core::ptr::read_volatile(RCC_APB2ENR);
    core::ptr::write_volatile(RCC_APB2ENR, apb2enr | (1 << 11)); // TIM1EN

    // PA8 is NUCLEO-G474RE's TIM1_CH1 output (AF6), exposed on CN10 / Arduino D7.
    let moder = core::ptr::read_volatile(GPIOA_MODER);
    core::ptr::write_volatile(GPIOA_MODER, (moder & !(0b11 << 16)) | (0b10 << 16));
    let afrh = core::ptr::read_volatile(GPIOA_AFRH);
    core::ptr::write_volatile(GPIOA_AFRH, (afrh & !0xF) | 6);

    // Edge-aligned 50% PWM. The simulator's default HSI clock is enough for
    // this smoke; it does not claim the Nucleo's PLL-configured board rate.
    core::ptr::write_volatile((TIM1_BASE + 0x28) as *mut u32, 0); // PSC
    core::ptr::write_volatile((TIM1_BASE + 0x2C) as *mut u32, 99); // ARR
    core::ptr::write_volatile((TIM1_BASE + 0x34) as *mut u32, 50); // CCR1
    core::ptr::write_volatile((TIM1_BASE + 0x18) as *mut u32, 0x0068); // PWM1 + OC1 preload
    core::ptr::write_volatile((TIM1_BASE + 0x20) as *mut u32, 1); // CC1E
    core::ptr::write_volatile((TIM1_BASE + 0x44) as *mut u32, 1 << 15); // BDTR.MOE
    core::ptr::write_volatile((TIM1_BASE + 0x14) as *mut u32, 1); // EGR.UG
    core::ptr::write_volatile((TIM1_BASE + 0x10) as *mut u32, 0); // clear SR
    core::ptr::write_volatile(TIM1_BASE as *mut u32, 1); // CR1.CEN
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        start_tim1_ch1_pwm_on_pa8();
        core::ptr::write_volatile(USART2_TDR_PTR, b'O');
        core::ptr::write_volatile(USART2_TDR_PTR, b'K');
        core::ptr::write_volatile(USART2_TDR_PTR, b'\n');
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
