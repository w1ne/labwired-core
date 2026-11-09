#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB1ENR1: *mut u32 = 0x4002_1058 as *mut u32;
const RCC_APB1ENR2: *mut u32 = 0x4002_105C as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4800_0020 as *mut u32;
const GPIOA_AFRH: *mut u32 = 0x4800_0024 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4800_0018 as *mut u32;
const LPUART1_CR1: *mut u32 = 0x4000_8000 as *mut u32;
const LPUART1_BRR: *mut u32 = 0x4000_800C as *mut u32;
const LPUART1_ISR: *const u32 = 0x4000_801C as *const u32;
const LPUART1_TDR: *mut u32 = 0x4000_8028 as *mut u32;
const FDCAN1_BASE: usize = 0x4000_A400;
const FDCAN1_SRAM: usize = FDCAN1_BASE + 0x800;

const TXE: u32 = 1 << 7;
const LED_PA5: u32 = 1 << 5;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(LPUART1_ISR) & TXE == 0 {}
        core::ptr::write_volatile(LPUART1_TDR, byte as u32);
    }
}

fn puts(bytes: &[u8]) {
    for &byte in bytes {
        putc(byte);
    }
}

/// Send one standard frame through FDCAN1's internal loopback and check the
/// received identifier and payload in RX FIFO0. This needs no external CAN
/// transceiver; it does not test PA11/PA12 pad propagation or bus timing.
fn check_fdcan1_loopback() -> bool {
    const CCCR: *mut u32 = (FDCAN1_BASE + 0x018) as *mut u32;
    const TEST: *mut u32 = (FDCAN1_BASE + 0x010) as *mut u32;
    const RXF0S: *const u32 = (FDCAN1_BASE + 0x090) as *const u32;
    const RXF0A: *mut u32 = (FDCAN1_BASE + 0x094) as *mut u32;
    const TXBAR: *mut u32 = (FDCAN1_BASE + 0x0CC) as *mut u32;
    const TXBTO: *const u32 = (FDCAN1_BASE + 0x0D4) as *const u32;
    const IR: *mut u32 = (FDCAN1_BASE + 0x050) as *mut u32;
    const TX_BUFFER0: usize = FDCAN1_SRAM + 0x278;
    const RX_FIFO0_ELEMENT0: usize = FDCAN1_SRAM + 0x0B0;

    unsafe {
        // Prove the FDCAN bus clock gate before enabling it (ENDN is zero
        // while the peripheral is clock-gated).
        if core::ptr::read_volatile((FDCAN1_BASE + 0x004) as *const u32) != 0 {
            return false;
        }
        let apb1enr1 = core::ptr::read_volatile(RCC_APB1ENR1);
        core::ptr::write_volatile(RCC_APB1ENR1, apb1enr1 | (1 << 25));
        if core::ptr::read_volatile((FDCAN1_BASE + 0x004) as *const u32) != 0x8765_4321 {
            return false;
        }

        // Set PA11/PA12 to AF9 as the Nucleo's exposed FDCAN1 RX/TX route.
        // Internal loopback below does not depend on pad-level CAN signaling.
        let moder = core::ptr::read_volatile(GPIOA_MODER);
        core::ptr::write_volatile(
            GPIOA_MODER,
            (moder & !((0b11 << 22) | (0b11 << 24))) | (0b10 << 22) | (0b10 << 24),
        );
        let afrh_value = core::ptr::read_volatile(GPIOA_AFRH);
        core::ptr::write_volatile(
            GPIOA_AFRH,
            (afrh_value & !((0xF << 12) | (0xF << 16))) | (9 << 12) | (9 << 16),
        );

        // Enter configuration and test modes, then select internal loopback.
        core::ptr::write_volatile(CCCR, 0x3); // INIT | CCE
        if core::ptr::read_volatile(CCCR) & 0x3 != 0x3 {
            return false;
        }
        core::ptr::write_volatile(CCCR, 0xA3); // INIT | CCE | TEST | MON
        core::ptr::write_volatile(TEST, 1 << 4); // LBCK
        if core::ptr::read_volatile(TEST) & (1 << 4) == 0 {
            return false;
        }

        // TX buffer 0: standard ID 0x321, DLC 8, payload 0x12345678/0x9ABCDEF0.
        core::ptr::write_volatile(TX_BUFFER0 as *mut u32, 0x321 << 18);
        core::ptr::write_volatile((TX_BUFFER0 + 4) as *mut u32, 8 << 16);
        core::ptr::write_volatile((TX_BUFFER0 + 8) as *mut u32, 0x1234_5678);
        core::ptr::write_volatile((TX_BUFFER0 + 12) as *mut u32, 0x9ABC_DEF0);
        core::ptr::write_volatile(RX_FIFO0_ELEMENT0 as *mut u32, 0);
        core::ptr::write_volatile((RX_FIFO0_ELEMENT0 + 4) as *mut u32, 0);
        core::ptr::write_volatile((RX_FIFO0_ELEMENT0 + 8) as *mut u32, 0);
        core::ptr::write_volatile((RX_FIFO0_ELEMENT0 + 12) as *mut u32, 0);

        // Leave INIT, submit buffer 0, and wait for its loopback receive.
        core::ptr::write_volatile(CCCR, 0xA2);
        let mut ready = false;
        for _ in 0..20_000 {
            if core::ptr::read_volatile(CCCR) & 1 == 0 {
                ready = true;
                break;
            }
        }
        if !ready {
            return false;
        }
        core::ptr::write_volatile(TXBAR, 1);
        let mut received = false;
        for _ in 0..20_000 {
            if core::ptr::read_volatile(RXF0S) & 0x7F != 0 {
                received = true;
                break;
            }
        }
        if !received || core::ptr::read_volatile(TXBTO) & 1 == 0 {
            return false;
        }

        let id = (core::ptr::read_volatile(RX_FIFO0_ELEMENT0 as *const u32) >> 18) & 0x7FF;
        let dlc = (core::ptr::read_volatile((RX_FIFO0_ELEMENT0 + 4) as *const u32) >> 16) & 0xF;
        let data0 = core::ptr::read_volatile((RX_FIFO0_ELEMENT0 + 8) as *const u32);
        let data1 = core::ptr::read_volatile((RX_FIFO0_ELEMENT0 + 12) as *const u32);
        if id != 0x321 || dlc != 8 || data0 != 0x1234_5678 || data1 != 0x9ABC_DEF0 {
            return false;
        }

        // Clear the receive interrupt and acknowledge FIFO0 element zero.
        let ir = core::ptr::read_volatile(IR);
        core::ptr::write_volatile(IR, ir);
        core::ptr::write_volatile(RXF0A, 0);
        core::ptr::read_volatile(RXF0S) & 0x7F == 0
    }
}

fn main() -> ! {
    unsafe {
        // Enable GPIOA and the board's default LPUART1 virtual COM port.
        let ahb2enr = core::ptr::read_volatile(RCC_AHB2ENR);
        core::ptr::write_volatile(RCC_AHB2ENR, ahb2enr | 1);
        let apb1enr2 = core::ptr::read_volatile(RCC_APB1ENR2);
        core::ptr::write_volatile(RCC_APB1ENR2, apb1enr2 | 1);

        // LD2 is active-high PA5. PA2/PA3 connect LPUART1 TX/RX (AF12).
        let moder = core::ptr::read_volatile(GPIOA_MODER);
        let moder = (moder & !((0b11 << 4) | (0b11 << 6) | (0b11 << 10)))
            | (0b10 << 4)
            | (0b10 << 6)
            | (0b01 << 10);
        core::ptr::write_volatile(GPIOA_MODER, moder);

        let afrl = core::ptr::read_volatile(GPIOA_AFRL);
        let afrl = (afrl & !((0xF << 8) | (0xF << 12))) | (12 << 8) | (12 << 12);
        core::ptr::write_volatile(GPIOA_AFRL, afrl);

        // The reset clock is HSI16; LPUART BRR uses the 256x baud ratio.
        core::ptr::write_volatile(LPUART1_BRR, 35_556);
        core::ptr::write_volatile(LPUART1_CR1, (1 << 0) | (1 << 3)); // UE | TE

        puts(b"NUCLEO_G491RE_UART_OK\n");
        if !check_fdcan1_loopback() {
            puts(b"NUCLEO_G491RE_FDCAN_FAIL\n");
            loop {}
        }
        puts(b"NUCLEO_G491RE_FDCAN_LOOPBACK_OK\n");
        puts(b"NUCLEO_G491RE_OK\n");
    }

    let mut led_high = false;
    loop {
        led_high = !led_high;
        unsafe {
            core::ptr::write_volatile(GPIOA_BSRR, if led_high { LED_PA5 } else { LED_PA5 << 16 });
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
