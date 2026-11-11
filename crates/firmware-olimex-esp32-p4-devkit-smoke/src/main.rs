#![no_std]
#![no_main]

const GPIO_OUT_W1TS: *mut u32 = 0x500E_0008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x500E_000C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x500E_0024 as *mut u32;
const GPIO2_OUT_SEL: *mut u32 = 0x500E_0560 as *mut u32;
const IO_MUX_GPIO2: *mut u32 = 0x500E_100C as *mut u32;
const GPIO2_MASK: u32 = 1 << 2;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        // GPIO2 function 1 is the GPIO pad function. Select the GPIO output
        // latch (256) and GPIO_ENABLE as the pad output-enable source.
        let iomux = core::ptr::read_volatile(IO_MUX_GPIO2);
        core::ptr::write_volatile(IO_MUX_GPIO2, (iomux & !(0x7 << 12)) | (1 << 12));
        core::ptr::write_volatile(GPIO2_OUT_SEL, 256 | (1 << 10));
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO2_MASK);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO2_MASK);

        // USER_LED1 is wired active-low. Alternate the GPIO2 pad state long
        // enough for the deterministic GPIO-edge assertion to observe it.
        loop {
            core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO2_MASK);
            for _ in 0..1_000 {
                core::arch::asm!("nop");
            }
            core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO2_MASK);
            for _ in 0..1_000 {
                core::arch::asm!("nop");
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
