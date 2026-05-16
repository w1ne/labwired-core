//! LabWired - STM32F103 I²C silicon-validation trace.
//!
//! Runs a no-slave I²C master transaction against I2C1 and emits a
//! human-readable register fingerprint on USART2 at 115200/8N1.  The
//! same firmware ELF runs:
//!
//! * In the LabWired simulator, against the modelled STM32F1 I²C
//!   peripheral with `external_devices: []`.
//! * On a Nucleo-F103RB attached via ST-Link/V2.1, with USART2 wired
//!   to the onboard VCP exposed as `/dev/ttyACM1` on the host.
//!
//! Both runs must produce byte-for-byte identical traces.  Any
//! divergence is a simulator bug or a chip-yaml mistake.
//!
//! # STM32F103 register map (subset used here)
//!
//! RCC  @ 0x40021000 — APB2ENR.IOPAEN bit 2, AFIOEN bit 0;
//!                    APB1ENR.USART2EN bit 17, I2C1EN bit 21.
//! GPIOA@ 0x40010800 — CRL: PA2 (USART2_TX) = alternate-function push-pull
//!                    50 MHz output = nibble 0b1011 in bits[11:8].
//! USART2@0x40004400 — SR/DR/BRR/CR1; BRR=0x45 → 115200 @ 8 MHz HSI.
//! I2C1 @ 0x40005400 — CR1/CR2/OAR1/DR/SR1/SR2/CCR/TRISE.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};

// ── Register addresses ────────────────────────────────────────────────────────

const RCC_BASE:   u32 = 0x4002_1000;
const RCC_APB2ENR: u32 = RCC_BASE + 0x18;
const RCC_APB1ENR: u32 = RCC_BASE + 0x1C;

const GPIOA_BASE: u32 = 0x4001_0800;
const GPIOA_CRL:  u32 = GPIOA_BASE + 0x00;

const GPIOB_BASE: u32 = 0x4001_0C00;
const GPIOB_CRL:  u32 = GPIOB_BASE + 0x00; /* PB6 (SCL) and PB7 (SDA) live here */

const USART2_BASE: u32 = 0x4000_4400;
const USART2_SR:   u32 = USART2_BASE + 0x00;
const USART2_DR:   u32 = USART2_BASE + 0x04;
const USART2_BRR:  u32 = USART2_BASE + 0x08;
const USART2_CR1:  u32 = USART2_BASE + 0x0C;

const I2C1_BASE:  u32 = 0x4000_5400;
const I2C1_CR1:   u32 = I2C1_BASE + 0x00;
const I2C1_CR2:   u32 = I2C1_BASE + 0x04;
const I2C1_OAR1:  u32 = I2C1_BASE + 0x08;
const I2C1_DR:    u32 = I2C1_BASE + 0x10;
const I2C1_SR1:   u32 = I2C1_BASE + 0x14;
const I2C1_SR2:   u32 = I2C1_BASE + 0x18;
const I2C1_CCR:   u32 = I2C1_BASE + 0x1C;
const I2C1_TRISE: u32 = I2C1_BASE + 0x20;

// ── Entry ─────────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        rcc_init();
        usart2_init();
    }
    putline(b"F103 I2C");
    unsafe {
        i2c1_init();
    }
    print_state(b"INIT");

    // Start the transaction.
    unsafe { rmw32(I2C1_CR1, 1 << 8) }; // CR1.START
    wait_sr1(0x0001); // SB
    print_state(b"START");

    // Send address byte (write) for a non-existent slave (0xA0/2 = 0x50).
    unsafe { write_volatile(I2C1_DR as *mut u32, 0xA0) };
    // Wait for either ADDR (slave ACKed) or AF (no ACK).  On the bare bus
    // we expect AF.  Cap the polling at 16k iterations.
    for _ in 0..16_000 {
        let sr1 = unsafe { read_volatile(I2C1_SR1 as *const u32) };
        if sr1 & 0x0402 != 0 {
            break;
        }
    }
    print_state(b"ADDR");

    // Generate STOP and wait for the bus to settle.
    unsafe { rmw32(I2C1_CR1, 1 << 9) }; // CR1.STOP
    for _ in 0..16_000 {
        let sr2 = unsafe { read_volatile(I2C1_SR2 as *const u32) };
        if sr2 & 0x0002 == 0 {
            break;
        }
    }
    print_state(b"STOP");

    putline(b"DONE");

    loop {}
}

// ── Init helpers ──────────────────────────────────────────────────────────────

unsafe fn rcc_init() {
    // Enable GPIOA + GPIOB + AFIO on APB2 (bits 2, 3, 0).  GPIOB is
    // required because I2C1's default pin mapping is PB6 (SCL) / PB7
    // (SDA); without IOPBEN the GPIOB CRL writes go to a dead clock
    // domain and the I²C state machine never gets the bus pins it
    // needs to drive — which surfaces on real silicon as a stalled
    // START condition (CR1.START stays set, SB never fires).  The
    // simulator's I²C model does NOT check GPIO config so the same
    // firmware "works" there even without IOPBEN; matching silicon
    // is mandatory.
    rmw32(RCC_APB2ENR, (1 << 0) | (1 << 2) | (1 << 3));
    // Enable USART2 + I2C1 on APB1 (bits 17 and 21).
    rmw32(RCC_APB1ENR, (1 << 17) | (1 << 21));
}

unsafe fn usart2_init() {
    // PA2 = AF push-pull, 50 MHz output → CRL bits[11:8] = 0b1011.
    let crl = read_volatile(GPIOA_CRL as *const u32);
    let crl = (crl & !(0xF << 8)) | (0xB << 8);
    write_volatile(GPIOA_CRL as *mut u32, crl);

    // BRR = PCLK1 / baud.  PCLK1 = 8 MHz (HSI default, no PLL),
    // baud = 115200 → BRR = 0x45 (DIV_M=4, DIV_F=5).
    write_volatile(USART2_BRR as *mut u32, 0x45);
    // CR1: UE (13) | TE (3).
    write_volatile(USART2_CR1 as *mut u32, (1 << 13) | (1 << 3));
}

unsafe fn i2c1_init() {
    // PB6 (SCL) and PB7 (SDA): AF open-drain 50 MHz → nibble 0xF in
    // GPIOB.CRL bits[27:24] and bits[31:28].  Required on real silicon;
    // ignored by the simulator's I²C model but matching it keeps the
    // sim/HW traces identical.
    let crl = read_volatile(GPIOB_CRL as *const u32);
    let crl = (crl & !(0xFF << 24)) | (0xFF << 24);
    write_volatile(GPIOB_CRL as *mut u32, crl);

    // CR2.FREQ = 8 (8 MHz APB1 clock).
    write_volatile(I2C1_CR2 as *mut u32, 8);
    // CCR for standard-mode 100 kHz: CCR = PCLK1 / (2 * SCL) = 40.
    write_volatile(I2C1_CCR as *mut u32, 40);
    // TRISE = PCLK1_MHz + 1 = 9.
    write_volatile(I2C1_TRISE as *mut u32, 9);
    // OAR1: set the reserved bit 14 per RM0008 §26.6.3.
    write_volatile(I2C1_OAR1 as *mut u32, 1 << 14);
    // PE.
    write_volatile(I2C1_CR1 as *mut u32, 1);
}

// ── State printer ─────────────────────────────────────────────────────────────

fn print_state(tag: &[u8]) {
    putline(tag);
    print_reg(b"CR1=",   unsafe { read_volatile(I2C1_CR1   as *const u32) });
    print_reg(b"CR2=",   unsafe { read_volatile(I2C1_CR2   as *const u32) });
    print_reg(b"CCR=",   unsafe { read_volatile(I2C1_CCR   as *const u32) });
    print_reg(b"TRISE=", unsafe { read_volatile(I2C1_TRISE as *const u32) });
    print_reg(b"OAR1=",  unsafe { read_volatile(I2C1_OAR1  as *const u32) });
    print_reg(b"SR1=",   unsafe { read_volatile(I2C1_SR1   as *const u32) });
    print_reg(b"SR2=",   unsafe { read_volatile(I2C1_SR2   as *const u32) });
}

fn print_reg(label: &[u8], v: u32) {
    putbytes(label);
    let mut buf = [0u8; 8];
    for i in 0..8 {
        let nib = ((v >> ((7 - i) * 4)) & 0xF) as u8;
        buf[i] = if nib < 10 { b'0' + nib } else { b'A' + nib - 10 };
    }
    putbytes(&buf);
    putbyte(b'\n');
}

fn putline(s: &[u8]) {
    putbytes(s);
    putbyte(b'\n');
}

fn putbytes(s: &[u8]) {
    for &b in s {
        putbyte(b);
    }
}

fn putbyte(b: u8) {
    unsafe {
        // Wait for USART2.SR.TXE.
        for _ in 0..1_000_000 {
            let sr = read_volatile(USART2_SR as *const u32);
            if sr & (1 << 7) != 0 {
                break;
            }
        }
        write_volatile(USART2_DR as *mut u32, b as u32);
    }
}

// ── Polling helpers ───────────────────────────────────────────────────────────

fn wait_sr1(mask: u32) -> bool {
    for _ in 0..16_000 {
        let sr1 = unsafe { read_volatile(I2C1_SR1 as *const u32) };
        if sr1 & mask == mask {
            return true;
        }
    }
    false
}

unsafe fn rmw32(addr: u32, set_bits: u32) {
    let v = read_volatile(addr as *const u32);
    write_volatile(addr as *mut u32, v | set_bits);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
