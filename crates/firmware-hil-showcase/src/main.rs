#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;

// Register Addresses (STM32H563)
const RCC_AHB1ENR: *mut u32 = 0x4402_0C1C as *mut u32;
const RCC_AHB2ENR: *mut u32 = 0x4402_0C20 as *mut u32;
const RCC_APB1LENR: *mut u32 = 0x4402_0C28 as *mut u32;

const GPIOB_BSRR: *mut u32 = 0x4202_0418 as *mut u32;
const GPIOD_MODER: *mut u32 = 0x4202_0C00 as *mut u32;
const GPIOD_AFRH: *mut u32 = 0x4202_0C24 as *mut u32;

const USART3_TDR: *mut u32 = 0x4000_4828 as *mut u32;
const USART3_ISR: *mut u32 = 0x4000_481C as *mut u32;
const USART3_CR1: *mut u32 = 0x4000_4800 as *mut u32;
const USART3_CR3: *mut u32 = 0x4000_4808 as *mut u32;
const USART3_BRR: *mut u32 = 0x4000_480C as *mut u32;

const DMA1_ISR: *mut u32 = 0x4002_0000 as *mut u32;
const DMA1_CCR1: *mut u32 = 0x4002_0008 as *mut u32;
const DMA1_CPAR1: *mut u32 = 0x4002_0010 as *mut u32;
const DMA1_CMAR1: *mut u32 = 0x4002_0014 as *mut u32;
const DMA1_CNDTR1: *mut u32 = 0x4002_000C as *mut u32;

const STRESS_BUFFER_SIZE: usize = 256;
static mut STRESS_BUFFER: [u8; STRESS_BUFFER_SIZE] = [0; STRESS_BUFFER_SIZE];

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn uart3_init() {
    unsafe {
        // AHB2ENR: Enable GPIOD
        core::ptr::write_volatile(
            RCC_AHB2ENR,
            core::ptr::read_volatile(RCC_AHB2ENR) | (1 << 3),
        );
        // APB1LENR: Enable USART3
        core::ptr::write_volatile(
            RCC_APB1LENR,
            core::ptr::read_volatile(RCC_APB1LENR) | (1 << 18),
        );

        // GPIOD MODER: PD8 as AF (10)
        let mut moder = core::ptr::read_volatile(GPIOD_MODER);
        moder &= !(0b11 << 16);
        moder |= 0b10 << 16;
        core::ptr::write_volatile(GPIOD_MODER, moder);

        // GPIOD AFRH: PD8 AF7 (0111)
        let mut afrh = core::ptr::read_volatile(GPIOD_AFRH);
        afrh &= !0xF;
        afrh |= 0x7;
        core::ptr::write_volatile(GPIOD_AFRH, afrh);

        core::ptr::write_volatile(USART3_BRR, 556); // 115200 at 64MHz
        core::ptr::write_volatile(USART3_CR3, 1 << 7); // DMAT
        core::ptr::write_volatile(USART3_CR1, (1 << 3) | (1 << 0)); // TE | UE
    }
}

fn uart3_write_str(s: &str) {
    for b in s.as_bytes() {
        unsafe {
            while (core::ptr::read_volatile(USART3_ISR) & (1 << 7)) == 0 {}
            core::ptr::write_volatile(USART3_TDR, *b as u32);
        }
    }
}

fn dma1_init() {
    unsafe {
        // AHB1ENR: Enable DMA1
        core::ptr::write_volatile(
            RCC_AHB1ENR,
            core::ptr::read_volatile(RCC_AHB1ENR) | (1 << 0),
        );

        // Program DMA1 Channel 1
        core::ptr::write_volatile(DMA1_CPAR1, USART3_TDR as u32);
        core::ptr::write_volatile(DMA1_CMAR1, STRESS_BUFFER.as_ptr() as u32);
        core::ptr::write_volatile(DMA1_CNDTR1, STRESS_BUFFER_SIZE as u32);
        // CCR: MINC(1<<7), DIR(1<<4), TCIE(1<<1), EN(1<<0)
        core::ptr::write_volatile(DMA1_CCR1, (1 << 7) | (1 << 4) | (1 << 1) | (1 << 0));
    }
}

fn main() -> ! {
    unsafe {
        for (i, val) in STRESS_BUFFER.iter_mut().enumerate() {
            *val = (i & 0xFF) as u8;
        }

        uart3_init();
        dma1_init();

        // Signal start
        core::ptr::write_volatile(GPIOB_BSRR, 1 << 0); // PB0 ON
        uart3_write_str("HIL Stress Test Started\r\n");

        // Wait for TCIF1 (Transfer Complete Interrupt Flag) in ISR
        // ISR bit index for Channel 1 TCIF is 1 (if following standard STM32 DMA layout)
        while (core::ptr::read_volatile(DMA1_ISR) & (1 << 1)) == 0 {}

        uart3_write_str("HIL Stress Test Passed\r\n");
        core::ptr::write_volatile(GPIOB_BSRR, 1 << 16); // PB0 OFF

        // Final Halt
        core::arch::asm!("bkpt #0");
    }

    loop {}
}
