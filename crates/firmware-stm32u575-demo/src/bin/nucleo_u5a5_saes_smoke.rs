#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;

// NUCLEO-U5A5ZJ-Q VCP: USART1 on PA9/PA10.
const RCC_APB2ENR: *mut u32 = (0x4602_0c00 + 0xA4) as *mut u32;
const RCC_AHB2ENR1: *mut u32 = (0x4602_0c00 + 0x8C) as *mut u32;
const USART1_CR1: *mut u32 = 0x4001_3800 as *mut u32;
const USART1_TDR: *mut u8 = (0x4001_3800 + 0x28) as *mut u8;

// STM32U5A5 SAES register offsets from RM0456; this instance is absent on U575.
const SAES: usize = 0x420C_0C00;
const SAES_CR: *mut u32 = SAES as *mut u32;
const SAES_SR: *const u32 = (SAES + 0x04) as *const u32;
const SAES_DINR: *mut u32 = (SAES + 0x08) as *mut u32;
const SAES_DOUTR: *const u32 = (SAES + 0x0C) as *const u32;
const SAES_KEYR: *mut u32 = (SAES + 0x10) as *mut u32;

const KEY: [u32; 4] = [0x1615_7e2b, 0xa6d2_ae28, 0x8815_f7ab, 0x3c4f_cf09];
const PLAINTEXT: [u32; 4] = [0xe2be_c16b, 0x969f_402e, 0x117e_3de9, 0x2a17_9373];
const EXPECTED: [u32; 4] = [0xb47b_d73a, 0x6036_7a0d, 0xf3ca_9ea8, 0x97ef_6624];

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn uart_write(bytes: &[u8]) {
    unsafe {
        for &byte in bytes {
            core::ptr::write_volatile(USART1_TDR, byte);
        }
    }
}

fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(RCC_APB2ENR, 1 << 14); // USART1EN
        core::ptr::write_volatile(RCC_AHB2ENR1, 1 << 20); // SAESEN
        core::ptr::write_volatile(USART1_CR1, (1 << 0) | (1 << 3)); // UE | TE

        // AES-128 ECB, 32-bit data type. The key and block words are written
        // least-significant byte first, matching the RM0456 DATAWIDTH path.
        core::ptr::write_volatile(SAES_CR, 1 | (2 << 1));
        for (index, word) in KEY.iter().enumerate() {
            core::ptr::write_volatile(SAES_KEYR.add(index), *word);
        }
        for word in PLAINTEXT {
            core::ptr::write_volatile(SAES_DINR, word);
        }

        let mut ready = false;
        for _ in 0..1000 {
            if core::ptr::read_volatile(SAES_SR) & 1 != 0 {
                ready = true;
                break;
            }
        }

        let mut matched = ready;
        if ready {
            for expected in EXPECTED {
                if core::ptr::read_volatile(SAES_DOUTR) != expected {
                    matched = false;
                }
            }
        }

        if matched {
            uart_write(b"U5A5_SAES_AES128_ECB_OK\n");
        } else {
            uart_write(b"U5A5_SAES_AES128_ECB_FAIL\n");
        }
    }

    loop {}
}
