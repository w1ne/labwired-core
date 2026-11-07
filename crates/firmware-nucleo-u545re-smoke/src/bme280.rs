#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const RCC_AHB2ENR1: *mut u32 = 0x4602_0C8C as *mut u32;
const RCC_APB1ENR1: *mut u32 = 0x4602_0C9C as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4602_0CA4 as *mut u32;

const GPIOA: usize = 0x4202_0000;
const GPIOB: usize = 0x4202_0400;
const GPIOA_MODER: *mut u32 = GPIOA as *mut u32;
const GPIOA_AFRH: *mut u32 = (GPIOA + 0x24) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA + 0x18) as *mut u32;
const GPIOB_MODER: *mut u32 = GPIOB as *mut u32;
const GPIOB_OTYPER: *mut u32 = (GPIOB + 0x04) as *mut u32;
const GPIOB_PUPDR: *mut u32 = (GPIOB + 0x0C) as *mut u32;
const GPIOB_AFRL: *mut u32 = (GPIOB + 0x20) as *mut u32;

const USART1_CR1: *mut u32 = 0x4001_3800 as *mut u32;
const USART1_BRR: *mut u32 = 0x4001_380C as *mut u32;
const USART1_ISR: *const u32 = 0x4001_381C as *const u32;
const USART1_TDR: *mut u32 = 0x4001_3828 as *mut u32;

const I2C1_CR1: *mut u32 = 0x4000_5400 as *mut u32;
const I2C1_CR2: *mut u32 = 0x4000_5404 as *mut u32;
const I2C1_TIMINGR: *mut u32 = 0x4000_5410 as *mut u32;
const I2C1_ISR: *const u32 = 0x4000_5418 as *const u32;
const I2C1_ICR: *mut u32 = 0x4000_541C as *mut u32;
const I2C1_RXDR: *const u32 = 0x4000_5424 as *const u32;
const I2C1_TXDR: *mut u32 = 0x4000_5428 as *mut u32;

const TXE: u32 = 1 << 7;
const LED_PA5: u32 = 1 << 5;
const ISR_TXIS: u32 = 1 << 1;
const ISR_RXNE: u32 = 1 << 2;
const ISR_NACKF: u32 = 1 << 4;
const ISR_STOPF: u32 = 1 << 5;
const ISR_TC: u32 = 1 << 6;
const ICR_NACKCF: u32 = 1 << 4;
const ICR_STOPCF: u32 = 1 << 5;
const I2C_ADDRESS: u32 = 0x77;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(USART1_ISR) & TXE == 0 {}
        core::ptr::write_volatile(USART1_TDR, byte as u32);
    }
}

fn puts(bytes: &[u8]) {
    for byte in bytes {
        putc(*byte);
    }
}

fn put_hex_byte(byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    putc(HEX[(byte >> 4) as usize]);
    putc(HEX[(byte & 0x0F) as usize]);
}

fn wait_for(mask: u32) -> bool {
    for _ in 0..500_000 {
        let status = unsafe { core::ptr::read_volatile(I2C1_ISR) };
        if status & mask != 0 {
            return true;
        }
        if status & ISR_NACKF != 0 {
            unsafe { core::ptr::write_volatile(I2C1_ICR, ICR_NACKCF | ICR_STOPCF) };
            return false;
        }
    }
    false
}

fn read_bme280_id() -> Result<u8, ()> {
    unsafe {
        // Write register 0xD0, then issue a repeated START for the one-byte read.
        let address = I2C_ADDRESS << 1;
        core::ptr::write_volatile(I2C1_CR2, address | (1 << 16) | (1 << 13));
        if !wait_for(ISR_TXIS) {
            return Err(());
        }
        core::ptr::write_volatile(I2C1_TXDR, 0xD0);
        if !wait_for(ISR_TC) {
            return Err(());
        }

        core::ptr::write_volatile(
            I2C1_CR2,
            address | (1 << 10) | (1 << 16) | (1 << 25) | (1 << 13),
        );
        if !wait_for(ISR_RXNE) {
            return Err(());
        }
        let value = core::ptr::read_volatile(I2C1_RXDR) as u8;
        if !wait_for(ISR_STOPF) {
            return Err(());
        }
        core::ptr::write_volatile(I2C1_ICR, ICR_STOPCF);
        Ok(value)
    }
}

fn main() -> ! {
    unsafe {
        // Enable GPIOA/B, I2C1, and USART1 clocks. The board's reset MSI is 4 MHz.
        core::ptr::write_volatile(RCC_AHB2ENR1, (1 << 0) | (1 << 1));
        core::ptr::write_volatile(RCC_APB1ENR1, 1 << 21);
        core::ptr::write_volatile(RCC_APB2ENR, 1 << 14);

        // LD2 is active-high on PA5; USART1 TX on PA9 uses AF7.
        let moder = core::ptr::read_volatile(GPIOA_MODER);
        let moder = (moder & !((0b11 << 10) | (0b11 << 18))) | (0b01 << 10) | (0b10 << 18);
        core::ptr::write_volatile(GPIOA_MODER, moder);
        let afrh = core::ptr::read_volatile(GPIOA_AFRH);
        core::ptr::write_volatile(GPIOA_AFRH, (afrh & !(0xF << 4)) | (7 << 4));
        core::ptr::write_volatile(GPIOA_BSRR, LED_PA5);

        // Arduino D15/D14 are PB6/SCL and PB7/SDA. I2C1 uses AF4, open-drain.
        let moder = core::ptr::read_volatile(GPIOB_MODER);
        core::ptr::write_volatile(
            GPIOB_MODER,
            (moder & !((0b11 << 12) | (0b11 << 14))) | (0b10 << 12) | (0b10 << 14),
        );
        let otyper = core::ptr::read_volatile(GPIOB_OTYPER);
        core::ptr::write_volatile(GPIOB_OTYPER, otyper | (1 << 6) | (1 << 7));
        let pupdr = core::ptr::read_volatile(GPIOB_PUPDR);
        core::ptr::write_volatile(
            GPIOB_PUPDR,
            (pupdr & !((0b11 << 12) | (0b11 << 14))) | (0b01 << 12) | (0b01 << 14),
        );
        let afrl = core::ptr::read_volatile(GPIOB_AFRL);
        core::ptr::write_volatile(
            GPIOB_AFRL,
            (afrl & !((0xF << 24) | (0xF << 28))) | (4 << 24) | (4 << 28),
        );

        core::ptr::write_volatile(USART1_BRR, 35);
        core::ptr::write_volatile(USART1_CR1, (1 << 0) | (1 << 3)); // UE | TE

        // 100 kHz I2C timing at the reset 4 MHz peripheral clock.
        core::ptr::write_volatile(I2C1_TIMINGR, 0x0020_0F17);
        core::ptr::write_volatile(I2C1_CR1, 1); // PE
    }

    match read_bme280_id() {
        Ok(id) => {
            puts(b"NUCLEO_U545RE_BME280_OK=0x");
            put_hex_byte(id);
            puts(b"\n");
        }
        Err(()) => puts(b"NUCLEO_U545RE_BME280_I2C_ERROR\n"),
    }

    let mut iterations = 0u32;
    let mut led_high = true;
    loop {
        iterations += 1;
        if iterations == 64 {
            iterations = 0;
            led_high = !led_high;
            unsafe {
                core::ptr::write_volatile(
                    GPIOA_BSRR,
                    if led_high { LED_PA5 } else { LED_PA5 << 16 },
                );
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
