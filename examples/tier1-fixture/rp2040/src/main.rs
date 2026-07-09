//! RP2040 Tier-1 fixture firmware.
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses and reports one line per class over UART0 using the
//! TIER1 protocol:
//!
//! ```text
//! TIER1 <class> PASS
//! TIER1 <class> FAIL code=<reason>
//! TIER1 done
//! ```
//!
//! The `uart` class is implicit: receiving `TIER1 done` over UART0 is itself
//! the proof of a working UART path.
//!
//! The RP2040 chip YAML declares behavioural models for the clocks/resets
//! subsystem (`clk_rst`), the 64-bit timer, the SIO GPIO block, the PL022 SPI
//! (SPI0) and the DW_apb_i2c (I2C0). Each is exercised below with raw register
//! round-trips. Classes the fixture does not attempt (dma, irq, adc, pwm, wdt,
//! rtc) resolve to `unrecorded`/`na`.
//!
//! Register offsets follow the RP2040 datasheet: §2.14 (clocks/resets), §4.6
//! (timer), §2.3.1 (SIO GPIO), §4.3 (I2C), §4.4 (SPI), §4.2 (UART, a PL011).

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// ── UART0 (RP2040 datasheet §4.2, base 0x40034000) ────────────────────────
//
// The simulator wires uart0 with profile "pl011" (ARM PrimeCell PL011, the
// RP2040's actual UART IP). In that layout the data register (UARTDR) sits at
// offset 0x00 — writing a byte here enqueues it for transmission.
const UART0_BASE: u32 = 0x4003_4000;
const UART0_TDR: u32 = UART0_BASE;

// ── CLOCKS / RESETS (rp2040_clkrst, datasheet §2.14) ──────────────────────
//
// RESETS holds peripherals in reset out of power-on; clearing a peripheral's
// RESET bit makes the matching RESET_DONE bit assert. The block is in the
// APB/AHB window, so the RP2040 atomic CLR alias (+0x3000) is honoured by the
// bus. The PLLs report LOCK and the crystal oscillator reports STABLE.
const RESETS_BASE: u32 = 0x4000_c000;
const RESETS_RESET: u32 = RESETS_BASE;
const RESETS_RESET_CLR: u32 = RESETS_BASE + 0x3000; // atomic clear alias
const RESETS_RESET_DONE: u32 = RESETS_BASE + 0x8;
const RESETS_IO_BANK0: u32 = 1 << 5; // a representative peripheral reset bit
const PLL_SYS_CS: u32 = 0x4002_8000; // bit31 = LOCK
const XOSC_STATUS: u32 = 0x4002_4000 + 0x4; // bit31 = STABLE
const LOCK_OR_STABLE: u32 = 1 << 31;

// ── TIMER (rp2040_timer, datasheet §4.6, base 0x40054000) ─────────────────
const TIMER_BASE: u32 = 0x4005_4000;
const TIMER_TIMERAWL: u32 = TIMER_BASE + 0x28;
const TIMER_TIMERAWH: u32 = TIMER_BASE + 0x24;

// ── SIO GPIO (rp2040_sio, datasheet §2.3.1, base 0xD0000000) ──────────────
const SIO_BASE: u32 = 0xD000_0000;
const SIO_GPIO_IN: u32 = SIO_BASE + 0x04;
const SIO_GPIO_OUT: u32 = SIO_BASE + 0x10;
const SIO_GPIO_OUT_SET: u32 = SIO_BASE + 0x14;
const SIO_GPIO_OUT_CLR: u32 = SIO_BASE + 0x18;
const SIO_GPIO_OE_SET: u32 = SIO_BASE + 0x24;
const GPIO_PIN25: u32 = 1 << 25; // Pico on-board LED, safe to toggle

// ── SPI0 (rp2040_spi, PL022, datasheet §4.4, base 0x4003c000) ─────────────
const SPI0_BASE: u32 = 0x4003_c000;
const SSPCR1: u32 = SPI0_BASE + 0x04;
const SSPDR: u32 = SPI0_BASE + 0x08;
const SSPSR: u32 = SPI0_BASE + 0x0c;
const CR1_LBM: u32 = 1 << 0; // loopback
const CR1_SSE: u32 = 1 << 1; // enable
const SR_RNE: u32 = 1 << 2; // RX FIFO not empty

// ── I2C0 (rp2040_i2c, DW_apb_i2c, datasheet §4.3, base 0x40044000) ────────
const I2C0_BASE: u32 = 0x4004_4000;
const IC_DATA_CMD: u32 = I2C0_BASE + 0x10;
const IC_RAW_INTR_STAT: u32 = I2C0_BASE + 0x34;
const IC_ENABLE: u32 = I2C0_BASE + 0x6c;
const IC_TX_ABRT_SOURCE: u32 = I2C0_BASE + 0x80;
const INTR_TX_ABRT: u32 = 1 << 6;
const ABRT_7B_ADDR_NOACK: u32 = 1 << 0;

#[inline(always)]
fn reg_read(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
fn reg_write(addr: u32, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

// ── UART0 output (raw register writes) ───────────────────────────────────
fn uart_write_byte(byte: u8) {
    reg_write(UART0_TDR, byte as u32);
}

fn uart_write_str(s: &str) {
    for b in s.as_bytes() {
        uart_write_byte(*b);
    }
}

fn uart_write_line(s: &str) {
    uart_write_str(s);
    uart_write_str("\r\n");
}

fn report(class: &str, result: Result<(), &'static str>) {
    uart_write_str("TIER1 ");
    uart_write_str(class);
    match result {
        Ok(()) => uart_write_line(" PASS"),
        Err(code) => {
            uart_write_str(" FAIL code=");
            uart_write_line(code);
        }
    }
}

// ── clock: clear a RESET bit → RESET_DONE asserts; PLL LOCK + XOSC STABLE ──
fn check_clock() -> Result<(), &'static str> {
    // Out of reset IO_BANK0 is held in reset, so its RESET_DONE bit is 0.
    if reg_read(RESETS_RESET) & RESETS_IO_BANK0 == 0 {
        return Err("reset-not-asserted");
    }
    // Release it via the atomic CLR alias; RESET_DONE must then reflect it.
    reg_write(RESETS_RESET_CLR, RESETS_IO_BANK0);
    if reg_read(RESETS_RESET) & RESETS_IO_BANK0 != 0 {
        return Err("reset-not-cleared");
    }
    if reg_read(RESETS_RESET_DONE) & RESETS_IO_BANK0 == 0 {
        return Err("reset-done");
    }
    // PLL_SYS reports LOCK and the crystal oscillator reports STABLE.
    if reg_read(PLL_SYS_CS) & LOCK_OR_STABLE == 0 {
        return Err("pll-lock");
    }
    if reg_read(XOSC_STATUS) & LOCK_OR_STABLE == 0 {
        return Err("xosc-stable");
    }
    Ok(())
}

// ── timer: the free-running 64-bit counter advances ────────────────────────
fn check_timer() -> Result<(), &'static str> {
    let a = reg_read(TIMER_TIMERAWL);
    for _ in 0..256 {
        core::hint::spin_loop();
    }
    let b = reg_read(TIMER_TIMERAWL);
    if b == a {
        return Err("timer-not-advancing");
    }
    // The high word must be a sane (small) value, proving the 64-bit split is
    // wired rather than aliasing the low word.
    if reg_read(TIMER_TIMERAWH) > 1 {
        return Err("timer-high-word");
    }
    Ok(())
}

// ── gpio: SIO drive + readback round-trip on GPIO25 ───────────────────────
fn check_gpio() -> Result<(), &'static str> {
    // Enable the output driver, then set the pin high and read it back.
    reg_write(SIO_GPIO_OE_SET, GPIO_PIN25);
    reg_write(SIO_GPIO_OUT_SET, GPIO_PIN25);
    if reg_read(SIO_GPIO_OUT) & GPIO_PIN25 == 0 {
        return Err("gpio-out-set");
    }
    if reg_read(SIO_GPIO_IN) & GPIO_PIN25 == 0 {
        return Err("gpio-in-high");
    }
    // Clear it; the input must follow.
    reg_write(SIO_GPIO_OUT_CLR, GPIO_PIN25);
    if reg_read(SIO_GPIO_IN) & GPIO_PIN25 != 0 {
        return Err("gpio-in-low");
    }
    Ok(())
}

// ── spi: PL022 internal-loopback transfer round-trips a byte ──────────────
fn check_spi() -> Result<(), &'static str> {
    // Enable the SSP in internal-loopback mode (MOSI wired to MISO).
    reg_write(SSPCR1, CR1_SSE | CR1_LBM);
    reg_write(SSPDR, 0xA5);
    // The byte clocks straight into the RX FIFO; RNE must assert.
    let mut spins = 0u32;
    while reg_read(SSPSR) & SR_RNE == 0 {
        spins += 1;
        if spins > 100_000 {
            return Err("spi-no-rx");
        }
    }
    if reg_read(SSPDR) != 0xA5 {
        return Err("spi-data");
    }
    Ok(())
}

// ── i2c: master transfer to an unconnected target → address-NACK abort ────
fn check_i2c() -> Result<(), &'static str> {
    reg_write(IC_ENABLE, 1);
    reg_write(IC_DATA_CMD, 0xDE); // write one byte to a 7-bit target
    if reg_read(IC_RAW_INTR_STAT) & INTR_TX_ABRT == 0 {
        return Err("i2c-no-abort");
    }
    if reg_read(IC_TX_ABRT_SOURCE) & ABRT_7B_ADDR_NOACK == 0 {
        return Err("i2c-abort-source");
    }
    Ok(())
}

#[entry]
fn main() -> ! {
    // Behavioural peripheral round-trips against the modeled RP2040.
    report("clock", check_clock());
    report("timer", check_timer());
    report("gpio", check_gpio());
    report("spi", check_spi());
    report("i2c", check_i2c());

    // uart: implicit via TIER1 done — no explicit line needed.
    uart_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
