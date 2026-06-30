#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;

// Register Addresses (STM32H563)
const RCC_AHB1ENR: *mut u32 = 0x4402_0C88 as *mut u32;
const RCC_AHB2ENR: *mut u32 = 0x4402_0C8C as *mut u32;
const RCC_APB1LENR: *mut u32 = 0x4402_0C9C as *mut u32;

const GPIOB_BSRR: *mut u32 = 0x4202_0418 as *mut u32;
const GPIOD_MODER: *mut u32 = 0x4202_0C00 as *mut u32;
const GPIOD_AFRH: *mut u32 = 0x4202_0C24 as *mut u32;

const USART3_TDR: *mut u32 = 0x4000_4828 as *mut u32;
const USART3_ISR: *mut u32 = 0x4000_481C as *mut u32;
const USART3_CR1: *mut u32 = 0x4000_4800 as *mut u32;
const USART3_CR3: *mut u32 = 0x4000_4808 as *mut u32;
const USART3_BRR: *mut u32 = 0x4000_480C as *mut u32;

// GPDMA1 channel 0 (RM0481 — the real H5 DMA engine; the generic DMA1
// layout this firmware used previously was a documented sim-only stand-in).
const GPDMA_C0SR: *mut u32 = 0x4002_0060 as *mut u32;
const GPDMA_C0CR: *mut u32 = 0x4002_0064 as *mut u32;
const GPDMA_C0TR1: *mut u32 = 0x4002_0090 as *mut u32;
const GPDMA_C0TR2: *mut u32 = 0x4002_0094 as *mut u32;
const GPDMA_C0BR1: *mut u32 = 0x4002_0098 as *mut u32;
const GPDMA_C0SAR: *mut u32 = 0x4002_009C as *mut u32;
const GPDMA_C0DAR: *mut u32 = 0x4002_00A0 as *mut u32;

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

fn gpdma_init() {
    unsafe {
        // AHB1ENR: Enable GPDMA1
        core::ptr::write_volatile(
            RCC_AHB1ENR,
            core::ptr::read_volatile(RCC_AHB1ENR) | (1 << 0),
        );

        // GPDMA1 channel 0: memory -> USART3 TDR, byte elements, source
        // increments, destination fixed. Software request (CTR2.SWREQ)
        // streams the block without TXE pacing — fine for the stress sim;
        // real-silicon UART TX would select the usart3_tx hardware request
        // instead (peripheral-request mode).
        core::ptr::write_volatile(GPDMA_C0TR1, 1 << 3); // SINC, DINC=0
        core::ptr::write_volatile(GPDMA_C0TR2, 1 << 9); // SWREQ
        core::ptr::write_volatile(GPDMA_C0BR1, STRESS_BUFFER_SIZE as u32);
        core::ptr::write_volatile(GPDMA_C0SAR, core::ptr::addr_of_mut!(STRESS_BUFFER) as u32);
        core::ptr::write_volatile(GPDMA_C0DAR, USART3_TDR as u32);
        core::ptr::write_volatile(GPDMA_C0CR, 1 << 0); // EN
    }
}

fn main() -> ! {
    unsafe {
        let buffer = &mut *core::ptr::addr_of_mut!(STRESS_BUFFER);
        for (i, val) in buffer.iter_mut().enumerate() {
            *val = (i & 0xFF) as u8;
        }

        uart3_init();
        gpdma_init();

        // Signal start
        core::ptr::write_volatile(GPIOB_BSRR, 1 << 0); // PB0 ON
        uart3_write_str("HIL Stress Test Started\r\n");

        // Wait for transfer complete: GPDMA C0SR.TCF (bit 8).
        while (core::ptr::read_volatile(GPDMA_C0SR) & (1 << 8)) == 0 {}

        uart3_write_str("HIL Stress Test Passed\r\n");
        core::ptr::write_volatile(GPIOB_BSRR, 1 << 16); // PB0 OFF

        // Final Halt
        core::arch::asm!("bkpt #0");
    }

    loop {}
}
