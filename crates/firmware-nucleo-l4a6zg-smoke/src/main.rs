#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const GPIOB_MODER: *mut u32 = 0x4800_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4800_0418 as *mut u32;

const LED_LD3_PB14: u32 = 1 << 14;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    unsafe {
        // Enable GPIOB on the AHB2 bus before configuring PB14.
        let ahb2enr = core::ptr::read_volatile(RCC_AHB2ENR);
        core::ptr::write_volatile(RCC_AHB2ENR, ahb2enr | (1 << 1));

        // Configure PB14 as a push-pull output for the active-high LD3 LED.
        let moder = core::ptr::read_volatile(GPIOB_MODER);
        core::ptr::write_volatile(GPIOB_MODER, (moder & !(0b11 << 28)) | (0b01 << 28));
    }

    let mut led_high = false;
    loop {
        led_high = !led_high;
        unsafe {
            core::ptr::write_volatile(
                GPIOB_BSRR,
                if led_high {
                    LED_LD3_PB14
                } else {
                    LED_LD3_PB14 << 16
                },
            );
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
