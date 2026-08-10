#![no_std]
#![no_main]
#![allow(clippy::identity_op)]

use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x18) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x1C) as *mut u32;

/// GPIOA CRL/CRH — the F1 pad mux for PA0..PA7 and PA8..PA15. Four bits per
/// pin, MODE[1:0] then CNF[1:0]. This family has no `MODER` and no `AFR`, so
/// there is no AF number to write: a pin's alternate function is fixed.
const GPIOA_CRL: *mut u32 = (0x4001_0800 + 0x00) as *mut u32;
const GPIOA_CRH: *mut u32 = (0x4001_0800 + 0x04) as *mut u32;

/// Enable AFIO (APB2 bit 0), GPIOA (bit 2), USART1 (bit 14), USART2 (APB1
/// bit 17) and I2C1 (APB1 bit 21). Required now that stm32f103.yaml clocks
/// those peripherals — unclocked MMIO is dropped.
///
/// AFIOEN, IOPAEN and USART2EN were all missing. Without IOPAEN the GPIO port
/// is held in reset, so `serial_init`'s CRL/CRH writes would be swallowed and
/// PA2/PA9 would stay the floating inputs they are after reset.
fn enable_peripheral_clocks() {
    unsafe {
        let apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        // AFIOEN | IOPAEN | USART1EN
        core::ptr::write_volatile(RCC_APB2ENR, apb2 | (1 << 0) | (1 << 2) | (1 << 14));
        let apb1 = core::ptr::read_volatile(RCC_APB1ENR);
        // USART2EN | I2C1EN
        core::ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 17) | (1 << 21));
    }
}

/// Mux the serial pads and give both transmitters a baud divisor.
///
/// Clocking the USARTs and writing `DR` is all this lab used to do, and it is
/// enough for LabWired's permissive USART model. It is not enough for silicon
/// and it is not enough for a probe: PA2 and PA9 stay floating inputs until
/// CRL/CRH select an alternate-function output, so the pad routes never go live
/// and a logic analyzer on either pin reads the GPIO latch — a flat line —
/// while the transaction-level bus monitor decodes the same traffic fine. A
/// zero `BRR` is the other half: with no divisor there is no bit period, so
/// there is nothing to narrate even once a route exists.
///
/// Pads, from the **Default** alternate-function column of DS5319 Rev 20,
/// Table 5 — no AFIO remap is involved for any of them:
///
/// * PA9  = `USART1_TX` (p.31) → CRH bits [7:4]   = `0xB`
/// * PA10 = `USART1_RX` (p.31) → CRH bits [11:8]  = `0x4`
/// * PA2  = `USART2_TX` (p.29) → CRL bits [11:8]  = `0xB`
///
/// `0xB` is MODE `0b11` (output, 50 MHz) + CNF `0b10` (alternate-function
/// push-pull); `0x4` is MODE `0b00` (input) + CNF `0b01` (floating), which is
/// what a receive pin needs.
///
/// `BRR` = f_PCLK / baud at the default 16× oversampling. This firmware never
/// touches the PLL, so the part runs on the 8 MHz HSI it selects at reset
/// (DS5319 Rev 20 §2.3.7, p.15) and both APB prescalers are 1, so PCLK1 =
/// PCLK2 = 8 MHz.
///
/// * USART1 (the GPS link) at 9600, the NEO-6 factory default for serial
///   port 1 in and out (u-blox NEO-6 datasheet GPS.G6-HW-09005-E, Table 15
///   “Default settings”, p.22): 8_000_000 / 9600 = 833.33 → 833 = 0x341.
/// * USART2 (console) at 115 200: 8_000_000 / 115_200 = 69.44 → 69 = 0x45.
fn serial_init() {
    unsafe {
        let crl = core::ptr::read_volatile(GPIOA_CRL);
        core::ptr::write_volatile(GPIOA_CRL, (crl & !(0xF << 8)) | (0xB << 8));
        let crh = core::ptr::read_volatile(GPIOA_CRH);
        let crh = (crh & !(0xFF << 4)) | (0xB << 4) | (0x4 << 8);
        core::ptr::write_volatile(GPIOA_CRH, crh);

        core::ptr::write_volatile(UART1_BRR, 0x341);
        // UE (bit 13) | TE (bit 3) | RE (bit 2) — this link is bidirectional.
        core::ptr::write_volatile(UART1_CR1, (1 << 13) | (1 << 3) | (1 << 2));

        core::ptr::write_volatile(UART2_BRR, 0x45);
        // UE | TE — the console is transmit only.
        core::ptr::write_volatile(UART2_CR1, (1 << 13) | (1 << 3));
    }
}

// USART1 on STM32F103: base 0x4001_3800 (same as uart1 in chip config)
// SR offset 0x00, DR offset 0x04, CR1 offset 0x0C
const UART1_BASE: u32 = 0x4001_3800;
const UART1_SR: *const u32 = (UART1_BASE + 0x00) as *const u32;
const UART1_DR: *mut u32 = (UART1_BASE + 0x04) as *mut u32;
const UART1_BRR: *mut u32 = (UART1_BASE + 0x08) as *mut u32;
const UART1_CR1: *mut u32 = (UART1_BASE + 0x0C) as *mut u32;

// USART2: base 0x4000_4400 — used as debug output
const UART2_BASE: u32 = 0x4000_4400;
const UART2_SR: *const u32 = (UART2_BASE + 0x00) as *const u32;
const UART2_DR: *mut u32 = (UART2_BASE + 0x04) as *mut u32;
const UART2_BRR: *mut u32 = (UART2_BASE + 0x08) as *mut u32;
const UART2_CR1: *mut u32 = (UART2_BASE + 0x0C) as *mut u32;

// SR bits
const SR_RXNE: u32 = 1 << 5; // RX Not Empty
const SR_TXE: u32 = 1 << 7; // TX Empty (ready)

fn uart2_byte(byte: u8) {
    unsafe {
        // Wait until TX is ready
        for _ in 0..256 {
            if core::ptr::read_volatile(UART2_SR) & SR_TXE != 0 {
                break;
            }
        }
        core::ptr::write_volatile(UART2_DR, byte as u32);
    }
}

fn uart2_str(s: &str) {
    for b in s.bytes() {
        uart2_byte(b);
    }
}

fn uart1_has_data() -> bool {
    unsafe { core::ptr::read_volatile(UART1_SR) & SR_RXNE != 0 }
}

fn uart1_read_byte() -> u8 {
    unsafe { (core::ptr::read_volatile(UART1_DR) & 0xFF) as u8 }
}

/// Read one complete NMEA sentence from UART1 into `buf`.
/// Returns the length of the sentence (including the trailing \n), or 0 on overflow.
fn read_nmea_sentence(buf: &mut [u8]) -> usize {
    let mut len = 0;
    loop {
        // Poll for next byte (busy-wait with iteration limit to avoid infinite hang)
        let byte = loop {
            let mut attempts = 0u32;
            if uart1_has_data() {
                break uart1_read_byte();
            }
            attempts += 1;
            if attempts > 2_000_000 {
                return 0; // timeout
            }
        };

        if len >= buf.len() {
            return 0; // buffer overflow — discard
        }
        buf[len] = byte;
        len += 1;

        // NMEA sentences end with \n (preceded by \r)
        if byte == b'\n' {
            return len;
        }
    }
}

#[entry]
fn main() -> ! {
    enable_peripheral_clocks();
    serial_init();
    uart2_str("NEO-6M GPS Lab\r\n");
    uart2_str("Reading NMEA stream from UART1...\r\n");

    let mut sentence_buf = [0u8; 128];

    loop {
        let len = read_nmea_sentence(&mut sentence_buf);
        if len == 0 {
            continue;
        }

        // Echo the raw NMEA sentence to UART2 with a prefix
        uart2_str("[GPS] ");
        for &b in &sentence_buf[..len] {
            uart2_byte(b);
        }
    }
}
